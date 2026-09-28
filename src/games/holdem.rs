use std::cmp::Ordering;

use regex::regex;
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
            if num_winners > 0 {
                for winner in &winners {
                    win_shares[winner.player_index] += EQUITY_SCALE / num_winners;
                }
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

/// A Hold'em hand range supporting offsuit and suited range notation (e.g. "AQo+", "AJs+").
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

    fn add_offsuit_range(
        &mut self,
        first_rank: Rank,
        second_rank: Rank,
        weight: Decimal,
    ) -> Result<(), DucyError> {
        if RankOrder::AceIsHigh.cmp(first_rank, second_rank) != Ordering::Greater {
            return Err(DucyError::InvalidRange);
        }

        for i in RankOrder::AceIsHigh.get_ranks_between(&first_rank, None) {
            for j in RankOrder::AceIsHigh.get_ranks_between(&second_rank, Some(&first_rank)) {
                for suit_1 in Suit::iter() {
                    for suit_2 in Suit::iter() {
                        if suit_1 != suit_2 {
                            let mut deck = Deck::empty();
                            let cards = [Card::new(i, suit_1), Card::new(j, suit_2)];
                            deck.insert_cards(cards.iter());

                            self.range_base.add_deck_weight(deck, weight);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn add_suited_range(
        &mut self,
        first_rank: Rank,
        second_rank: Rank,
        weight: Decimal,
    ) -> Result<(), DucyError> {
        if RankOrder::AceIsHigh.cmp(first_rank, second_rank) != Ordering::Greater {
            return Err(DucyError::InvalidRange);
        }

        for i in RankOrder::AceIsHigh.get_ranks_between(&first_rank, None) {
            for j in RankOrder::AceIsHigh.get_ranks_between(&second_rank, Some(&first_rank)) {
                for suit_1 in Suit::iter() {
                    let mut deck = Deck::empty();
                    let cards = [Card::new(i, suit_1), Card::new(j, suit_1)];
                    deck.insert_cards(cards.iter());

                    self.range_base.add_deck_weight(deck, weight);
                }
            }
        }
        Ok(())
    }

    /// Adds hands matching a range string (e.g. "AQo+", "AJs+") with the given weight.
    pub fn add(&mut self, range: &str, weight: Decimal) -> Result<(), DucyError> {
        let offsuit_range = regex!(r"([23456789TtJjQqKkAa])([234567789TtJjQqKkAa])o\+");
        if let Some(x) = offsuit_range.captures(range).into_iter().next() {
            let first_card = Rank::try_from_char(&x[1].chars().next().unwrap())?;
            let second_card = Rank::try_from_char(&x[2].chars().next().unwrap())?;

            return self.add_offsuit_range(first_card, second_card, weight);
        }

        let suited_range = regex!(r"([23456789TtJjQqKkAa])([234567789TtJjQqKkAa])s\+");
        if let Some(x) = suited_range.captures(range).into_iter().next() {
            let first_card = Rank::try_from_char(&x[1].chars().next().unwrap())?;
            let second_card = Rank::try_from_char(&x[2].chars().next().unwrap())?;

            return self.add_suited_range(first_card, second_card, weight);
        }

        Err(DucyError::InvalidRange)
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
