use crate::games::dealt_hand::DealtHandGameState;
use crate::games::{GameEvaluation, GameState, GameWinner, WinnerTracker};
use crate::ranking::deuce_to_seven::{DeuceToSevenRanker, DeuceToSevenRanks};

/// Game state for Single Draw Deuce-to-Seven Lowball (Kansas City Lowball).
/// Each player has 5 cards. Aces are high, straights and flushes count
/// against you. Best hand is 2-3-4-5-7 offsuit.
pub struct SingleDraw27GameState {
    pub(crate) state: DealtHandGameState,
}

impl GameState for SingleDraw27GameState {}

impl Default for SingleDraw27GameState {
    fn default() -> Self {
        Self {
            state: DealtHandGameState::new(5),
        }
    }
}

impl SingleDraw27GameState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_player(&mut self, cards: crate::deck::Deck) -> Result<(), crate::error::DucyError> {
        self.state.add_player(cards)
    }
}

pub struct SingleDraw27GameEvaluation;

impl GameEvaluation<SingleDraw27GameState, DeuceToSevenRanks> for SingleDraw27GameEvaluation {
    fn evaluate_winners(
        &self,
        game_state: &SingleDraw27GameState,
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
        let mut state = SingleDraw27GameState::new();
        state
            .add_player(Deck::parse("2c 3d 4h 5s 7c").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("2d 3h 4s 5c 8d").unwrap())
            .unwrap();

        let eval = SingleDraw27GameEvaluation;
        let winners = eval.evaluate_winners(&state);
        assert_eq!(winners.len(), 1);
        assert_eq!(winners[0].player_index(), 0);
        assert_eq!(winners[0].pot_amount(), dec!(1));
    }

    #[test]
    fn test_straight_hurts() {
        let mut state = SingleDraw27GameState::new();
        // 6-high straight — bad in 2-7
        state
            .add_player(Deck::parse("2c 3d 4h 5s 6c").unwrap())
            .unwrap();
        // 8-high no straight
        state
            .add_player(Deck::parse("2d 3h 4s 5c 8d").unwrap())
            .unwrap();

        let eval = SingleDraw27GameEvaluation;
        let winners = eval.evaluate_winners(&state);
        assert_eq!(winners.len(), 1);
        assert_eq!(winners[0].player_index(), 1);
    }

    #[test]
    fn test_ace_is_high() {
        let mut state = SingleDraw27GameState::new();
        // Ace makes this worse (ace is high in 2-7)
        state
            .add_player(Deck::parse("Ac 2d 3h 4s 7c").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("2s 3c 4d 5h 8s").unwrap())
            .unwrap();

        let eval = SingleDraw27GameEvaluation;
        let winners = eval.evaluate_winners(&state);
        assert_eq!(winners.len(), 1);
        assert_eq!(winners[0].player_index(), 1);
    }
}
