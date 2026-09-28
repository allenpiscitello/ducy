use crate::games::dealt_hand::DealtHandGameState;
use crate::games::{GameEvaluation, GameState, GameWinner, WinnerTracker};
use crate::ranking::deuce_to_seven::{DeuceToSevenRanker, DeuceToSevenRanks};

/// Game state for 2-7 Triple Draw Lowball (5 cards per player).
pub struct DeuceToSevenGameState {
    pub(crate) state: DealtHandGameState,
}

impl GameState for DeuceToSevenGameState {}

impl Default for DeuceToSevenGameState {
    fn default() -> Self {
        Self {
            state: DealtHandGameState::new(5),
        }
    }
}

impl DeuceToSevenGameState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_player(&mut self, cards: crate::deck::Deck) -> Result<(), crate::error::DucyError> {
        self.state.add_player(cards)
    }
}

pub struct DeuceToSevenGameEvaluation;

impl GameEvaluation<DeuceToSevenGameState, DeuceToSevenRanks> for DeuceToSevenGameEvaluation {
    fn evaluate_winners(
        &self,
        game_state: &DeuceToSevenGameState,
    ) -> Vec<GameWinner<DeuceToSevenRanks>> {
        let mut tracker = WinnerTracker::new();
        for (i, hand) in game_state.state.hole_cards().iter().enumerate() {
            let rank = DeuceToSevenRanker::get_rank(hand);
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
    fn test_nut_low_wins() {
        let mut state = DeuceToSevenGameState::new();
        state
            .add_player(Deck::parse("2c 3d 4h 5s 7c").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("2d 3h 4s 5c 8d").unwrap())
            .unwrap();

        let eval = DeuceToSevenGameEvaluation;
        let winners = eval.evaluate_winners(&state);
        assert_eq!(winners.len(), 1);
        assert_eq!(winners[0].player_index(), 0);
        assert_eq!(winners[0].pot_amount(), dec!(1));
    }

    #[test]
    fn test_straight_loses_to_high_card() {
        let mut state = DeuceToSevenGameState::new();
        state
            .add_player(Deck::parse("2c 3d 4h 5s 6c").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("2d 3h 4s 5c 8d").unwrap())
            .unwrap();

        let eval = DeuceToSevenGameEvaluation;
        let winners = eval.evaluate_winners(&state);
        assert_eq!(winners.len(), 1);
        assert_eq!(winners[0].player_index(), 1);
    }

    #[test]
    fn test_tie_splits_pot() {
        let mut state = DeuceToSevenGameState::new();
        state
            .add_player(Deck::parse("2c 3d 4h 5s 7c").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("2d 3h 4s 5c 7d").unwrap())
            .unwrap();

        let eval = DeuceToSevenGameEvaluation;
        let winners = eval.evaluate_winners(&state);
        assert_eq!(winners.len(), 2);
        assert_eq!(winners[0].pot_amount(), dec!(0.5));
    }
}
