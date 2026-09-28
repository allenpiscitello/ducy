use crate::games::dealt_hand::DealtHandGameState;
use crate::games::{GameEvaluation, GameState, GameWinner, WinnerTracker};
use crate::ranking::badugi::{BadugiRanker, BadugiRanks};

/// Game state for Badugi (4 cards per player, three draw rounds).
/// Each player's final hand is evaluated for the best badugi
/// (cards with all different suits and ranks, lower is better).
pub struct BadugiGameState {
    pub(crate) state: DealtHandGameState,
}

impl GameState for BadugiGameState {}

impl Default for BadugiGameState {
    fn default() -> Self {
        Self {
            state: DealtHandGameState::new(4),
        }
    }
}

impl BadugiGameState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_player(&mut self, cards: crate::deck::Deck) -> Result<(), crate::error::DucyError> {
        self.state.add_player(cards)
    }
}

pub struct BadugiGameEvaluation;

impl GameEvaluation<BadugiGameState, BadugiRanks> for BadugiGameEvaluation {
    fn evaluate_winners(&self, game_state: &BadugiGameState) -> Vec<GameWinner<BadugiRanks>> {
        let mut tracker = WinnerTracker::new();
        for (i, hand) in game_state.state.hole_cards().iter().enumerate() {
            let rank = BadugiRanker::get_rank(hand);
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
    fn test_nut_badugi_wins() {
        let mut state = BadugiGameState::new();
        state
            .add_player(Deck::parse("Ac 2d 3h 4s").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("2c 3d 4h 5s").unwrap())
            .unwrap();

        let eval = BadugiGameEvaluation;
        let winners = eval.evaluate_winners(&state);
        assert_eq!(winners.len(), 1);
        assert_eq!(winners[0].player_index(), 0);
        assert_eq!(winners[0].pot_amount(), dec!(1));
    }

    #[test]
    fn test_four_card_beats_three_card() {
        let mut state = BadugiGameState::new();
        // Player 0: 4-card badugi (K-Q-J-T)
        state
            .add_player(Deck::parse("Kc Qd Jh Ts").unwrap())
            .unwrap();
        // Player 1: only 3-card badugi (duplicate rank)
        state
            .add_player(Deck::parse("Ac 2d 3h 3s").unwrap())
            .unwrap();

        let eval = BadugiGameEvaluation;
        let winners = eval.evaluate_winners(&state);
        assert_eq!(winners.len(), 1);
        assert_eq!(winners[0].player_index(), 0);
    }

    #[test]
    fn test_tie_splits() {
        let mut state = BadugiGameState::new();
        state
            .add_player(Deck::parse("Ac 2d 3h 4s").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("Ad 2h 3s 4c").unwrap())
            .unwrap();

        let eval = BadugiGameEvaluation;
        let winners = eval.evaluate_winners(&state);
        assert_eq!(winners.len(), 2);
        assert_eq!(winners[0].pot_amount(), dec!(0.5));
    }
}
