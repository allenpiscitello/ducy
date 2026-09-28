use std::cmp::Ordering;

use rust_decimal::Decimal;
use strum::IntoEnumIterator;

use crate::{
    deck::{
        Card, Deck, Rank, Suit,
        range::{Range, RangeBase},
    },
    error::DucyError,
    games::{
        EQUITY_SCALE, GameEquityEvaluation, GameEvaluation, GameState, GameWinner, WinnerTracker,
        flop_game::{FlopGame, FlopGameState},
    },
    ranking::{
        hand_rank::{StandardHandRanker, StandardHandRanks},
        standard_hand_ranker::RankOrder,
    },
};

/// Texas Hold'em game state (2 hole cards per player).
pub struct HoldemGameState {
    pub(crate) flop_game_state: FlopGameState,
}

impl HoldemGameState {
    /// Creates a new Hold'em game state.
    pub fn new() -> Self {
        Self {
            flop_game_state: FlopGameState::new(2),
        }
    }
}

impl Default for HoldemGameState {
    fn default() -> Self {
        Self::new()
    }
}

impl FlopGame for HoldemGameState {
    fn get_community_cards(&self) -> Deck {
        self.flop_game_state.get_community_cards()
    }

    fn add_player(&mut self, cards: Deck) -> Result<(), DucyError> {
        self.flop_game_state.add_player(cards)
    }

    fn set_flop(&mut self, cards: Deck) -> Result<(), DucyError> {
        self.flop_game_state.set_flop(cards)
    }

    fn set_turn(&mut self, card: Card) -> Result<(), DucyError> {
        self.flop_game_state.set_turn(card)
    }

    fn set_river(&mut self, card: Card) -> Result<(), DucyError> {
        self.flop_game_state.set_river(card)
    }

    fn get_player_hole_cards(&self) -> impl Iterator<Item = &Deck> {
        self.flop_game_state.get_player_hole_cards()
    }

    fn get_final_states<'a>(&'a self) -> impl Iterator<Item = Self> + 'a {
        self.flop_game_state
            .get_final_states()
            .map(|x| Self { flop_game_state: x })
    }
}

impl GameState for HoldemGameState {}

/// Evaluates Hold'em hands to determine winners.
pub struct HoldemGameEvaluation {}

impl GameEvaluation<HoldemGameState, StandardHandRanks> for HoldemGameEvaluation {
    fn evaluate_winners(&self, game_state: &HoldemGameState) -> Vec<GameWinner<StandardHandRanks>> {
        let mut tracker = WinnerTracker::new();
        for (i, player) in game_state.get_player_hole_cards().enumerate() {
            let combined_deck = *player | game_state.get_community_cards();
            if let Some(rank) =
                StandardHandRanker::get_rank_at_least(&combined_deck, tracker.best_hand())
            {
                tracker.consider(i, rank);
            }
        }
        tracker.into_results()
    }
}

impl GameEquityEvaluation<HoldemGameState, StandardHandRanks, HoldemGameEvaluation>
    for HoldemGameEvaluation
{
    fn evaluate_equity(&self, game_state: &HoldemGameState) -> Vec<Decimal> {
        let num_players = game_state.get_player_hole_cards().count();
        let mut win_shares: Vec<u64> = vec![0; num_players];
        let mut hand_count: u64 = 0;
        for runout in game_state.get_final_states() {
            let winners = HoldemGameEvaluation {}.evaluate_winners(&runout);
            let num_winners = winners.len() as u64;
            for winner in &winners {
                win_shares[winner.player_index] += EQUITY_SCALE / num_winners;
            }
            hand_count += 1;
        }
        let divisor = Decimal::from(EQUITY_SCALE) * Decimal::from(hand_count);
        win_shares
            .iter()
            .map(|&s| Decimal::from(s) / divisor)
            .collect()
    }
}

/// A Hold'em hand range supporting standard poker range notation.
///
/// Supported patterns:
/// - Pair: `TT`, `TT+`, `77-TT`
/// - Offsuit: `AKo`, `AQo+`, `A8o-ATo`
/// - Suited: `AKs`, `AJs+`, `A8s-ATs`
/// - Mixed (both suited and offsuit): `AK`, `AQ+`
/// - Specific combo: `AhKs`
pub struct HoldemRange {
    range_base: RangeBase,
}

impl HoldemRange {
    /// Creates an empty range.
    pub fn new() -> Self {
        Self {
            range_base: RangeBase::new(),
        }
    }

    fn add_pair_combos(&mut self, rank: Rank, weight: Decimal) {
        let suits: Vec<Suit> = Suit::iter().collect();
        for i in 0..suits.len() {
            for j in (i + 1)..suits.len() {
                let mut deck = Deck::empty();
                deck.insert_cards([Card::new(rank, suits[i]), Card::new(rank, suits[j])].iter());
                self.range_base.add_deck_weight(deck, weight);
            }
        }
    }

    fn add_pair_range(
        &mut self,
        low_rank: Rank,
        high_rank: Rank,
        weight: Decimal,
    ) -> Result<(), DucyError> {
        let (low, high) = if RankOrder::AceIsHigh.cmp(low_rank, high_rank) == Ordering::Greater {
            (high_rank, low_rank)
        } else {
            (low_rank, high_rank)
        };
        for rank in RankOrder::AceIsHigh.get_ranks_between(&low, None) {
            self.add_pair_combos(rank, weight);
            if rank == high {
                break;
            }
        }
        Ok(())
    }

    fn add_offsuit_combo(&mut self, first_rank: Rank, second_rank: Rank, weight: Decimal) {
        for s1 in Suit::iter() {
            for s2 in Suit::iter() {
                if s1 != s2 {
                    let mut deck = Deck::empty();
                    deck.insert_cards(
                        [Card::new(first_rank, s1), Card::new(second_rank, s2)].iter(),
                    );
                    self.range_base.add_deck_weight(deck, weight);
                }
            }
        }
    }

    fn add_suited_combo(&mut self, first_rank: Rank, second_rank: Rank, weight: Decimal) {
        for s in Suit::iter() {
            let mut deck = Deck::empty();
            deck.insert_cards([Card::new(first_rank, s), Card::new(second_rank, s)].iter());
            self.range_base.add_deck_weight(deck, weight);
        }
    }

    fn add_offsuit_range(
        &mut self,
        high_rank: Rank,
        low_rank: Rank,
        weight: Decimal,
    ) -> Result<(), DucyError> {
        if RankOrder::AceIsHigh.cmp(high_rank, low_rank) != Ordering::Greater {
            return Err(DucyError::InvalidRange);
        }
        for rank in RankOrder::AceIsHigh.get_ranks_between(&low_rank, Some(&high_rank)) {
            self.add_offsuit_combo(high_rank, rank, weight);
        }
        Ok(())
    }

    fn add_suited_range(
        &mut self,
        high_rank: Rank,
        low_rank: Rank,
        weight: Decimal,
    ) -> Result<(), DucyError> {
        if RankOrder::AceIsHigh.cmp(high_rank, low_rank) != Ordering::Greater {
            return Err(DucyError::InvalidRange);
        }
        for rank in RankOrder::AceIsHigh.get_ranks_between(&low_rank, Some(&high_rank)) {
            self.add_suited_combo(high_rank, rank, weight);
        }
        Ok(())
    }

    fn parse_dash_range(
        &mut self,
        left: &str,
        right: &str,
        weight: Decimal,
    ) -> Result<(), DucyError> {
        let left_chars: Vec<char> = left.chars().collect();
        let right_chars: Vec<char> = right.chars().collect();

        match (left_chars.len(), right_chars.len()) {
            (2, 2) => {
                let lr1 = Rank::try_from_char(&left_chars[0])?;
                let lr2 = Rank::try_from_char(&left_chars[1])?;
                let rr1 = Rank::try_from_char(&right_chars[0])?;
                let rr2 = Rank::try_from_char(&right_chars[1])?;
                if lr1 != lr2 || rr1 != rr2 {
                    return Err(DucyError::InvalidRange);
                }
                self.add_pair_range(lr1, rr1, weight)
            }
            (3, 3) => {
                let lr1 = Rank::try_from_char(&left_chars[0])?;
                let lr2 = Rank::try_from_char(&left_chars[1])?;
                let rr1 = Rank::try_from_char(&right_chars[0])?;
                let rr2 = Rank::try_from_char(&right_chars[1])?;
                let left_suffix = left_chars[2].to_ascii_lowercase();
                let right_suffix = right_chars[2].to_ascii_lowercase();

                if left_suffix != right_suffix || (left_suffix != 'o' && left_suffix != 's') {
                    return Err(DucyError::InvalidRange);
                }
                if lr1 != rr1 {
                    return Err(DucyError::InvalidRange);
                }

                let high = lr1;
                let (low_start, low_end) = if RankOrder::AceIsHigh.cmp(lr2, rr2) == Ordering::Less {
                    (lr2, rr2)
                } else {
                    (rr2, lr2)
                };

                for rank in RankOrder::AceIsHigh.get_ranks_between(&low_start, None) {
                    if RankOrder::AceIsHigh.cmp(rank, high) != Ordering::Less {
                        break;
                    }
                    if left_suffix == 'o' {
                        self.add_offsuit_combo(high, rank, weight);
                    } else {
                        self.add_suited_combo(high, rank, weight);
                    }
                    if rank == low_end {
                        break;
                    }
                }
                Ok(())
            }
            _ => Err(DucyError::InvalidRange),
        }
    }

    /// Adds hands matching a range string with the given weight.
    pub fn add(&mut self, range: &str, weight: Decimal) -> Result<(), DucyError> {
        let range = range.trim();

        // Specific combo: "AhKs" (rank-suit-rank-suit)
        if range.len() == 4 && !range.ends_with('+') {
            let chars: Vec<char> = range.chars().collect();
            if let (Ok(r1), Ok(s1), Ok(r2), Ok(s2)) = (
                Rank::try_from_char(&chars[0]),
                Suit::try_from_char(&chars[1]),
                Rank::try_from_char(&chars[2]),
                Suit::try_from_char(&chars[3]),
            ) {
                let card1 = Card::new(r1, s1);
                let card2 = Card::new(r2, s2);
                if card1 == card2 {
                    return Err(DucyError::InvalidRange);
                }
                let mut deck = Deck::empty();
                deck.insert_cards([card1, card2].iter());
                self.range_base.add_deck_weight(deck, weight);
                return Ok(());
            }
        }

        // Dash ranges: "77-TT", "A8s-ATs"
        if let Some(dash_pos) = range.find('-') {
            return self.parse_dash_range(&range[..dash_pos], &range[dash_pos + 1..], weight);
        }

        let has_plus = range.ends_with('+');
        let base = if has_plus {
            &range[..range.len() - 1]
        } else {
            range
        };
        let base_chars: Vec<char> = base.chars().collect();

        match base_chars.len() {
            2 => {
                let r1 = Rank::try_from_char(&base_chars[0])?;
                let r2 = Rank::try_from_char(&base_chars[1])?;

                if r1 == r2 {
                    if has_plus {
                        self.add_pair_range(r1, Rank::Ace, weight)
                    } else {
                        self.add_pair_combos(r1, weight);
                        Ok(())
                    }
                } else {
                    let (high, low) = if RankOrder::AceIsHigh.cmp(r1, r2) == Ordering::Greater {
                        (r1, r2)
                    } else {
                        (r2, r1)
                    };
                    if has_plus {
                        self.add_offsuit_range(high, low, weight)?;
                        self.add_suited_range(high, low, weight)
                    } else {
                        self.add_offsuit_combo(high, low, weight);
                        self.add_suited_combo(high, low, weight);
                        Ok(())
                    }
                }
            }
            3 => {
                let r1 = Rank::try_from_char(&base_chars[0])?;
                let r2 = Rank::try_from_char(&base_chars[1])?;
                let suffix = base_chars[2].to_ascii_lowercase();

                if r1 == r2 {
                    return Err(DucyError::InvalidRange);
                }

                let (high, low) = if RankOrder::AceIsHigh.cmp(r1, r2) == Ordering::Greater {
                    (r1, r2)
                } else {
                    (r2, r1)
                };

                match suffix {
                    'o' => {
                        if has_plus {
                            self.add_offsuit_range(high, low, weight)
                        } else {
                            self.add_offsuit_combo(high, low, weight);
                            Ok(())
                        }
                    }
                    's' => {
                        if has_plus {
                            self.add_suited_range(high, low, weight)
                        } else {
                            self.add_suited_combo(high, low, weight);
                            Ok(())
                        }
                    }
                    _ => Err(DucyError::InvalidRange),
                }
            }
            _ => Err(DucyError::InvalidRange),
        }
    }
}

impl Default for HoldemRange {
    fn default() -> Self {
        Self::new()
    }
}

impl Range for HoldemRange {
    fn iter(&self) -> impl Iterator<Item = crate::deck::range::RangeItem> {
        self.range_base.iter()
    }
}

#[cfg(test)]
mod test {
    use crate::{
        deck::{Card, Deck, Rank, range::Range},
        games::{
            GameEquityEvaluation, GameEvaluation, GameWinner,
            flop_game::FlopGame,
            holdem::{HoldemGameEvaluation, HoldemGameState, HoldemRange},
        },
        ranking::hand_rank::StandardHandRanks,
    };
    use rust_decimal_macros::dec;

    #[test]
    pub fn test_holdem_hand() {
        let mut holdem_hand = HoldemGameState::new();
        holdem_hand
            .add_player(Deck::parse("As Ac").unwrap())
            .unwrap();
        holdem_hand
            .add_player(Deck::parse("ks kd").unwrap())
            .unwrap();

        let hand_evaluation = HoldemGameEvaluation {};

        holdem_hand
            .set_flop(Deck::parse("kc qd js").unwrap())
            .unwrap();

        let winners = hand_evaluation.evaluate_winners(&holdem_hand);

        assert_eq!(winners.len(), 1);
        assert_eq!(
            winners[0],
            GameWinner {
                player_index: 1,
                pot_amount: dec!(1),
                winning_hand: StandardHandRanks::ThreeOfAKind {
                    t: Rank::King,
                    c1: Rank::Queen,
                    c2: Rank::Jack
                }
            }
        );

        let equities = hand_evaluation.evaluate_equity(&holdem_hand);

        assert_eq!(equities.len(), 2);
        assert_eq!(equities[0], dec!(418) / dec!(1980));
        assert_eq!(equities[1], dec!(1562) / dec!(1980));

        holdem_hand.set_turn(Card::parse("Tc").unwrap()).unwrap();

        let winners = hand_evaluation.evaluate_winners(&holdem_hand);

        assert_eq!(winners.len(), 1);
        assert_eq!(
            winners[0],
            GameWinner {
                player_index: 0,
                pot_amount: dec!(1),
                winning_hand: StandardHandRanks::Straight { s: Rank::Ace }
            }
        );

        let equities = hand_evaluation.evaluate_equity(&holdem_hand);

        assert_eq!(equities.len(), 2);
        assert_eq!(equities[0], dec!(33) / dec!(44));
        assert_eq!(equities[1], dec!(11) / dec!(44));

        holdem_hand.set_river(Card::parse("Ad").unwrap()).unwrap();

        let winners = hand_evaluation.evaluate_winners(&holdem_hand);

        assert_eq!(winners.len(), 2);
        assert_eq!(
            winners[0],
            GameWinner {
                player_index: 0,
                pot_amount: dec!(0.5),
                winning_hand: StandardHandRanks::Straight { s: Rank::Ace }
            }
        );
        assert_eq!(
            winners[1],
            GameWinner {
                player_index: 1,
                pot_amount: dec!(0.5),
                winning_hand: StandardHandRanks::Straight { s: Rank::Ace }
            }
        );

        let equities = hand_evaluation.evaluate_equity(&holdem_hand);

        assert_eq!(equities.len(), 2);
        assert_eq!(equities[0], dec!(1) / dec!(2));
        assert_eq!(equities[1], dec!(1) / dec!(2));
    }

    #[test]
    pub fn range_tests() {
        let mut range = HoldemRange::new();
        range.add("AQo+", dec!(1)).unwrap();
        let mut range_1 = vec![
            "As Kc", "As Kd", "As Kh", "Ac Ks", "Ac Kh", "Ac Kd", "Ad Ks", "Ad Kc", "Ad Kh",
            "Ah Ks", "Ah Kd", "Ah Kc", "As Qc", "As Qd", "As Qh", "Ac Qs", "Ac Qh", "Ac Qd",
            "Ad Qs", "Ad Qc", "Ad Qh", "Ah Qs", "Ah Qd", "Ah Qc",
        ];

        range_1.sort();

        let mut actual_range_items: Vec<String> =
            range.iter().map(|x| x.get_deck().to_string()).collect();
        actual_range_items.sort();

        assert_eq!(actual_range_items, range_1);

        range.add("AJs+", dec!(1)).unwrap();

        let mut range_2 = vec![
            "As Ks", "Ac Kc", "Ad Kd", "Ah Kh", "As Qs", "Ah Qh", "Ac Qc", "Ad Qd", "Ac Jc",
            "Ad Jd", "Ah Jh", "As Js",
        ];

        range_1.append(&mut range_2);
        range_1.sort();

        let mut actual_range_items: Vec<String> =
            range.iter().map(|x| x.get_deck().to_string()).collect();
        actual_range_items.sort();

        assert_eq!(actual_range_items, range_1);
    }

    #[test]
    pub fn test_pair_range() {
        let mut range = HoldemRange::new();
        range.add("TT", dec!(1)).unwrap();
        assert_eq!(range.iter().count(), 6); // C(4,2) = 6 combos for one pair
    }

    #[test]
    pub fn test_pair_plus_range() {
        let mut range = HoldemRange::new();
        range.add("QQ+", dec!(1)).unwrap();
        // QQ, KK, AA = 3 pairs * 6 combos = 18
        assert_eq!(range.iter().count(), 18);
    }

    #[test]
    pub fn test_pair_dash_range() {
        let mut range = HoldemRange::new();
        range.add("77-TT", dec!(1)).unwrap();
        // 77, 88, 99, TT = 4 pairs * 6 combos = 24
        assert_eq!(range.iter().count(), 24);
    }

    #[test]
    pub fn test_offsuit_exact() {
        let mut range = HoldemRange::new();
        range.add("AKo", dec!(1)).unwrap();
        assert_eq!(range.iter().count(), 12); // 4*3 = 12 offsuit combos
    }

    #[test]
    pub fn test_suited_exact() {
        let mut range = HoldemRange::new();
        range.add("AKs", dec!(1)).unwrap();
        assert_eq!(range.iter().count(), 4); // 4 suited combos
    }

    #[test]
    pub fn test_mixed_exact() {
        let mut range = HoldemRange::new();
        range.add("AK", dec!(1)).unwrap();
        assert_eq!(range.iter().count(), 16); // 12 offsuit + 4 suited
    }

    #[test]
    pub fn test_mixed_plus() {
        let mut range = HoldemRange::new();
        range.add("AQ+", dec!(1)).unwrap();
        // AQ (16) + AK (16) = 32
        assert_eq!(range.iter().count(), 32);
    }

    #[test]
    pub fn test_specific_combo() {
        let mut range = HoldemRange::new();
        range.add("AhKs", dec!(1)).unwrap();
        assert_eq!(range.iter().count(), 1);
        let item = range.iter().next().unwrap();
        assert_eq!(item.get_deck().to_string(), "Ah Ks");
    }

    #[test]
    pub fn test_suited_dash_range() {
        let mut range = HoldemRange::new();
        range.add("A8s-ATs", dec!(1)).unwrap();
        // A8s, A9s, ATs = 3 * 4 = 12
        assert_eq!(range.iter().count(), 12);
    }

    #[test]
    pub fn test_offsuit_dash_range() {
        let mut range = HoldemRange::new();
        range.add("K9o-KJo", dec!(1)).unwrap();
        // K9o, KTo, KJo = 3 * 12 = 36
        assert_eq!(range.iter().count(), 36);
    }

    #[test]
    pub fn test_offsuit_range_plus_fixed_high_card() {
        let mut range = HoldemRange::new();
        range.add("KTo+", dec!(1)).unwrap();
        // KT, KJ, KQ = 3 combos * 12 = 36
        assert_eq!(range.iter().count(), 36);
    }

    #[test]
    pub fn test_invalid_range_same_card() {
        let mut range = HoldemRange::new();
        assert!(range.add("AsAs", dec!(1)).is_err());
    }

    #[test]
    pub fn test_invalid_range_garbage() {
        let mut range = HoldemRange::new();
        assert!(range.add("xyz", dec!(1)).is_err());
    }

    #[test]
    pub fn test_invalid_pair_with_suit_suffix() {
        let mut range = HoldemRange::new();
        assert!(range.add("TTs", dec!(1)).is_err());
    }

    #[test]
    pub fn test_add_player_wrong_card_count() {
        let mut game = HoldemGameState::new();
        assert!(game.add_player(Deck::parse("As Ks Qs").unwrap()).is_err());
    }

    #[test]
    pub fn test_add_player_duplicate_cards() {
        let mut game = HoldemGameState::new();
        game.add_player(Deck::parse("As Ks").unwrap()).unwrap();
        assert!(game.add_player(Deck::parse("As Qd").unwrap()).is_err());
    }

    #[test]
    pub fn test_set_flop_wrong_card_count() {
        let mut game = HoldemGameState::new();
        game.add_player(Deck::parse("As Ks").unwrap()).unwrap();
        assert!(game.set_flop(Deck::parse("Qd Jd").unwrap()).is_err());
    }

    #[test]
    pub fn test_set_turn_before_flop() {
        let mut game = HoldemGameState::new();
        game.add_player(Deck::parse("As Ks").unwrap()).unwrap();
        assert!(game.set_turn(Card::parse("2c").unwrap()).is_err());
    }

    #[test]
    pub fn test_set_river_before_turn() {
        let mut game = HoldemGameState::new();
        game.add_player(Deck::parse("As Ks").unwrap()).unwrap();
        game.set_flop(Deck::parse("Qd Jd Td").unwrap()).unwrap();
        assert!(game.set_river(Card::parse("2c").unwrap()).is_err());
    }

    #[test]
    pub fn test_set_turn_card_not_in_deck() {
        let mut game = HoldemGameState::new();
        game.add_player(Deck::parse("As Ks").unwrap()).unwrap();
        game.set_flop(Deck::parse("Qd Jd Td").unwrap()).unwrap();
        assert!(game.set_turn(Card::parse("As").unwrap()).is_err());
    }
}
