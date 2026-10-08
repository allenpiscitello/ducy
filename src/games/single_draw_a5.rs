use crate::games::dealt_hand::DealtHandGameState;
use crate::games::{GameEvaluation, GameState, GameWinner, WinnerTracker};
use crate::ranking::razz::{RazzRanker, RazzRanks};

/// Game state for Single Draw Ace-to-Five Lowball (California Lowball).
/// Each player has 5 cards. Aces are low, straights and flushes don't
/// count against you. Best hand is A-2-3-4-5 (the wheel).
pub struct SingleDrawA5GameState {
    pub(crate) state: DealtHandGameState,
}

impl GameState for SingleDrawA5GameState {}

impl Default for SingleDrawA5GameState {
    fn default() -> Self {
        Self {
            state: DealtHandGameState::new(5),
        }
    }
}

impl SingleDrawA5GameState {
    /// An empty game: add each player's cards with [`add_player`](Self::add_player).
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the next player's cards. Fails with
    /// [`IncorrectCardCount`](crate::error::DucyError::IncorrectCardCount) if it
    /// isn't this game's number of cards,
    /// [`CardsNotAvailable`](crate::error::DucyError::CardsNotAvailable) if a card
    /// was already dealt, or
    /// [`TooManyPlayers`](crate::error::DucyError::TooManyPlayers) past 10 players.
    pub fn add_player(&mut self, cards: crate::deck::Deck) -> Result<(), crate::error::DucyError> {
        self.state.add_player(cards)
    }
}

/// Finds the winners of a [`SingleDrawA5GameState`]: the best hand, ties sharing.
pub struct SingleDrawA5GameEvaluation;

impl GameEvaluation<SingleDrawA5GameState, RazzRanks> for SingleDrawA5GameEvaluation {
    fn evaluate_winners(&self, game_state: &SingleDrawA5GameState) -> Vec<GameWinner<RazzRanks>> {
        let mut tracker = WinnerTracker::new();
        for (i, hand) in game_state.state.hole_cards().iter().enumerate() {
            let rank = RazzRanker::get_rank(hand);
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
        let mut state = SingleDrawA5GameState::new();
        state
            .add_player(Deck::parse("Ac 2d 3h 4s 5c").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("Ad 2h 3s 4c 6d").unwrap())
            .unwrap();

        let eval = SingleDrawA5GameEvaluation;
        let winners = eval.evaluate_winners(&state);
        assert_eq!(winners.len(), 1);
        assert_eq!(winners[0].player_index(), 0);
        assert_eq!(winners[0].pot_amount(), dec!(1));
    }

    #[test]
    fn test_flush_doesnt_hurt() {
        let mut state = SingleDrawA5GameState::new();
        // All clubs but still the wheel
        state
            .add_player(Deck::parse("Ac 2c 3c 4c 5c").unwrap())
            .unwrap();
        // 6-low offsuit
        state
            .add_player(Deck::parse("Ad 2h 3s 4d 6d").unwrap())
            .unwrap();

        let eval = SingleDrawA5GameEvaluation;
        let winners = eval.evaluate_winners(&state);
        assert_eq!(winners.len(), 1);
        assert_eq!(winners[0].player_index(), 0);
    }

    #[test]
    fn test_pair_loses() {
        let mut state = SingleDrawA5GameState::new();
        state
            .add_player(Deck::parse("Ac Ad 2h 3s 4c").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("2d 3h 4s 5c Kd").unwrap())
            .unwrap();

        let eval = SingleDrawA5GameEvaluation;
        let winners = eval.evaluate_winners(&state);
        assert_eq!(winners.len(), 1);
        assert_eq!(winners[0].player_index(), 1);
    }
}
