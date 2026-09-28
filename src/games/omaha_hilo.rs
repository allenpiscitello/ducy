use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::{
    deck::{Card, Deck},
    error::DucyError,
    games::{
        EQUITY_SCALE, GameState, GameWinner, WinnerTracker,
        flop_game::{FlopGame, FlopGameState},
    },
    ranking::{
        hand_rank::{StandardHandRanker, StandardHandRanks},
        low_hand_rank::{LowHandRanker, LowHandRanks},
    },
};

/// Omaha Hi-Lo 8-or-Better game state (4 hole cards per player, split pot).
pub struct OmahaHiLoGameState {
    pub(crate) flop_game_state: FlopGameState,
}

impl OmahaHiLoGameState {
    /// Creates a new Omaha Hi-Lo game state.
    pub fn new() -> Self {
        Self {
            flop_game_state: FlopGameState::new(4),
        }
    }
}

impl Default for OmahaHiLoGameState {
    fn default() -> Self {
        Self::new()
    }
}

impl FlopGame for OmahaHiLoGameState {
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

impl GameState for OmahaHiLoGameState {}

/// Result of a Hi-Lo hand evaluation: high winners and optional low winners.
pub struct HiLoResult {
    pub high_winners: Vec<GameWinner<StandardHandRanks>>,
    pub low_winners: Vec<GameWinner<LowHandRanks>>,
}

/// Evaluates Omaha Hi-Lo hands (high and low).
pub struct OmahaHiLoGameEvaluation {}

impl OmahaHiLoGameEvaluation {
    /// Evaluates a single board state, returning high and low winners with
    /// correct pot shares. If no low qualifies, high scoops the full pot.
    pub fn evaluate_winners(&self, game_state: &OmahaHiLoGameState) -> HiLoResult {
        let community = game_state.get_community_cards();

        let mut high_tracker = WinnerTracker::new();
        let mut low_tracker: WinnerTracker<LowHandRanks> = WinnerTracker::new();

        for (i, player) in game_state.get_player_hole_cards().enumerate() {
            for community_cards_of_3 in community.enumerate_combinations(3) {
                for player_cards_of_2 in player.enumerate_combinations(2) {
                    let combined = community_cards_of_3 | player_cards_of_2;
                    if let Some(rank) =
                        StandardHandRanker::get_rank_at_least(&combined, high_tracker.best_hand())
                    {
                        high_tracker.consider(i, rank);
                    }
                    if let Some(rank) =
                        LowHandRanker::get_rank_at_least(&combined, low_tracker.best_hand())
                    {
                        low_tracker.consider(i, rank);
                    }
                }
            }
        }

        let has_low = low_tracker.best_hand().is_some()
            && low_tracker.best_hand() != Some(LowHandRanks::NoLow);

        if has_low {
            let mut high_winners = high_tracker.into_results();
            let mut low_winners = low_tracker.into_results();
            for w in &mut high_winners {
                w.pot_amount *= dec!(0.5);
            }
            for w in &mut low_winners {
                w.pot_amount *= dec!(0.5);
            }
            HiLoResult {
                high_winners,
                low_winners,
            }
        } else {
            HiLoResult {
                high_winners: high_tracker.into_results(),
                low_winners: vec![],
            }
        }
    }

    /// Evaluates equity across all possible runouts, returning each player's
    /// expected pot share accounting for high/low splits and scooping.
    pub fn evaluate_equity(&self, game_state: &OmahaHiLoGameState) -> Vec<Decimal> {
        let num_players = game_state.get_player_hole_cards().count();
        let mut win_shares: Vec<u64> = vec![0; num_players];
        let mut hand_count: u64 = 0;

        for runout in game_state.get_final_states() {
            let community = runout.get_community_cards();
            let mut high_tracker = WinnerTracker::new();
            let mut low_tracker: WinnerTracker<LowHandRanks> = WinnerTracker::new();

            for (i, player) in runout.get_player_hole_cards().enumerate() {
                for community_cards_of_3 in community.enumerate_combinations(3) {
                    for player_cards_of_2 in player.enumerate_combinations(2) {
                        let combined = community_cards_of_3 | player_cards_of_2;
                        if let Some(rank) = StandardHandRanker::get_rank_at_least(
                            &combined,
                            high_tracker.best_hand(),
                        ) {
                            high_tracker.consider(i, rank);
                        }
                        if let Some(rank) =
                            LowHandRanker::get_rank_at_least(&combined, low_tracker.best_hand())
                        {
                            low_tracker.consider(i, rank);
                        }
                    }
                }
            }

            let has_low = low_tracker.best_hand().is_some()
                && low_tracker.best_hand() != Some(LowHandRanks::NoLow);

            let high_winners = high_tracker.into_results();
            let high_count = high_winners.len() as u64;

            if has_low {
                let low_winners = low_tracker.into_results();
                let low_count = low_winners.len() as u64;
                let half_scale = EQUITY_SCALE / 2;
                for w in &high_winners {
                    win_shares[w.player_index] += half_scale / high_count;
                }
                for w in &low_winners {
                    win_shares[w.player_index] += half_scale / low_count;
                }
            } else {
                for w in &high_winners {
                    win_shares[w.player_index] += EQUITY_SCALE / high_count;
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

#[cfg(test)]
mod test {
    use rust_decimal_macros::dec;

    use crate::{
        deck::{Card, Deck, Rank},
        games::{
            GameWinner,
            flop_game::FlopGame,
            omaha_hilo::{OmahaHiLoGameEvaluation, OmahaHiLoGameState},
        },
        ranking::{hand_rank::StandardHandRanks, low_hand_rank::LowHandRanks},
    };

    #[test]
    fn test_hilo_high_scoops_no_low() {
        let mut state = OmahaHiLoGameState::new();
        state
            .add_player(Deck::parse("Ks Kd Qc Qd").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("Js Jd Tc Td").unwrap())
            .unwrap();
        state.set_flop(Deck::parse("Kc 9h 9d").unwrap()).unwrap();
        state.set_turn(Card::parse("Jh").unwrap()).unwrap();
        state.set_river(Card::parse("Th").unwrap()).unwrap();

        let evaluator = OmahaHiLoGameEvaluation {};
        let result = evaluator.evaluate_winners(&state);

        assert_eq!(result.high_winners.len(), 1);
        assert_eq!(result.high_winners[0].player_index(), 0);
        assert_eq!(result.high_winners[0].pot_amount(), dec!(1));
        assert!(result.low_winners.is_empty());
    }

    #[test]
    fn test_hilo_split_pot() {
        let mut state = OmahaHiLoGameState::new();
        // Player 0: strong high hand
        state
            .add_player(Deck::parse("Ks Kd Qc Qd").unwrap())
            .unwrap();
        // Player 1: has a low draw
        state
            .add_player(Deck::parse("As 2d 3c 4d").unwrap())
            .unwrap();
        state.set_flop(Deck::parse("5h 7c Kc").unwrap()).unwrap();
        state.set_turn(Card::parse("8h").unwrap()).unwrap();
        state.set_river(Card::parse("Jd").unwrap()).unwrap();

        let evaluator = OmahaHiLoGameEvaluation {};
        let result = evaluator.evaluate_winners(&state);

        // Player 0 should win high (trips kings)
        assert_eq!(result.high_winners.len(), 1);
        assert_eq!(result.high_winners[0].player_index(), 0);
        assert_eq!(result.high_winners[0].pot_amount(), dec!(0.5));

        // Player 1 should win low (A-2-3-5-7 or similar)
        assert_eq!(result.low_winners.len(), 1);
        assert_eq!(result.low_winners[0].player_index(), 1);
        assert_eq!(result.low_winners[0].pot_amount(), dec!(0.5));
    }

    #[test]
    fn test_hilo_scoop_both_high_and_low() {
        let mut state = OmahaHiLoGameState::new();
        // Player 0: A-2 with strong high potential
        state
            .add_player(Deck::parse("As 2d Kc Kd").unwrap())
            .unwrap();
        // Player 1: no low cards
        state
            .add_player(Deck::parse("Qs Qd Jc Jd").unwrap())
            .unwrap();
        state.set_flop(Deck::parse("3h 4c 5h").unwrap()).unwrap();
        state.set_turn(Card::parse("Kh").unwrap()).unwrap();
        state.set_river(Card::parse("9d").unwrap()).unwrap();

        let evaluator = OmahaHiLoGameEvaluation {};
        let result = evaluator.evaluate_winners(&state);

        // Player 0 wins high (trips kings) and low (A-2-3-4-5)
        assert_eq!(result.high_winners.len(), 1);
        assert_eq!(result.high_winners[0].player_index(), 0);
        assert_eq!(result.high_winners[0].pot_amount(), dec!(0.5));

        assert_eq!(result.low_winners.len(), 1);
        assert_eq!(result.low_winners[0].player_index(), 0);
        assert_eq!(result.low_winners[0].pot_amount(), dec!(0.5));
    }

    #[test]
    fn test_hilo_equity_no_low_possible() {
        let mut state = OmahaHiLoGameState::new();
        state
            .add_player(Deck::parse("Ks Kd Qc Qd").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("Js Jd Tc Td").unwrap())
            .unwrap();
        state.set_flop(Deck::parse("Kc 9h 9d").unwrap()).unwrap();
        state.set_turn(Card::parse("Jh").unwrap()).unwrap();
        state.set_river(Card::parse("Th").unwrap()).unwrap();

        let evaluator = OmahaHiLoGameEvaluation {};
        let equities = evaluator.evaluate_equity(&state);

        assert_eq!(equities.len(), 2);
        assert_eq!(equities[0] + equities[1], dec!(1));
        assert_eq!(equities[0], dec!(1));
    }
}
