use crate::games::dealt_hand::DealtHandGameState;
use crate::games::{GameEvaluation, GameState, GameWinner, WinnerTracker};
use crate::ranking::razz::{RazzRanker, RazzRanks};

/// Game state for Razz (Seven-Card Stud Low). Each player receives 7 cards
/// and makes the best 5-card ace-to-five low hand.
pub struct RazzGameState {
    pub(crate) state: DealtHandGameState,
}

impl GameState for RazzGameState {}

impl Default for RazzGameState {
    fn default() -> Self {
        Self {
            state: DealtHandGameState::new(7),
        }
    }
}

impl RazzGameState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_player(&mut self, cards: crate::deck::Deck) -> Result<(), crate::error::DucyError> {
        self.state.add_player(cards)
    }
}

pub struct RazzGameEvaluation;

impl GameEvaluation<RazzGameState, RazzRanks> for RazzGameEvaluation {
    fn evaluate_winners(&self, game_state: &RazzGameState) -> Vec<GameWinner<RazzRanks>> {
        let mut tracker = WinnerTracker::new();
        for (i, hand) in game_state.state.hole_cards().iter().enumerate() {
            let rank = RazzRanker::get_best_from(hand);
            tracker.consider(i, rank);
        }
        tracker.into_results()
    }
}

#[cfg(test)]
mod test {
    use rust_decimal_macros::dec;

    use super::*;
    use crate::deck::Deck;
    use crate::games::GameEvaluation;

    #[test]
    fn test_wheel_wins() {
        let mut state = RazzGameState::new();
        // Player 0: has wheel among 7 cards
        state
            .add_player(Deck::parse("Ac 2d 3h 4s 5c Kd Qh").unwrap())
            .unwrap();
        // Player 1: 6-low
        state
            .add_player(Deck::parse("Ad 2h 3s 4c 6d Jh Th").unwrap())
            .unwrap();

        let eval = RazzGameEvaluation;
        let winners = eval.evaluate_winners(&state);
        assert_eq!(winners.len(), 1);
        assert_eq!(winners[0].player_index(), 0);
        assert_eq!(winners[0].pot_amount(), dec!(1));
    }

    #[test]
    fn test_pair_loses() {
        let mut state = RazzGameState::new();
        // Player 0: forced into a pair
        state
            .add_player(Deck::parse("Ac Ad Ah 2s 3c Kd Kh").unwrap())
            .unwrap();
        // Player 1: no pair, king-low
        state
            .add_player(Deck::parse("2d 3h 4s 5c 7d Kc Qd").unwrap())
            .unwrap();

        let eval = RazzGameEvaluation;
        let winners = eval.evaluate_winners(&state);
        assert_eq!(winners.len(), 1);
        assert_eq!(winners[0].player_index(), 1);
    }

    #[test]
    fn test_tie_splits() {
        let mut state = RazzGameState::new();
        state
            .add_player(Deck::parse("Ac 2d 3h 4s 5c Kd Qh").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("Ad 2h 3s 4c 5d Kh Qs").unwrap())
            .unwrap();

        let eval = RazzGameEvaluation;
        let winners = eval.evaluate_winners(&state);
        assert_eq!(winners.len(), 2);
        assert_eq!(winners[0].pot_amount(), dec!(0.5));
    }
}
