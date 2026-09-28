use crate::games::dealt_hand::DealtHandGameState;
use crate::games::{GameEvaluation, GameState, GameWinner, WinnerTracker};
use crate::ranking::hand_rank::{StandardHandRanker, StandardHandRanks};

/// Game state for Seven-Card Stud. Each player receives 7 cards
/// and makes the best 5-card high hand.
pub struct StudGameState {
    pub(crate) state: DealtHandGameState,
}

impl GameState for StudGameState {}

impl Default for StudGameState {
    fn default() -> Self {
        Self {
            state: DealtHandGameState::new(7),
        }
    }
}

impl StudGameState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_player(&mut self, cards: crate::deck::Deck) -> Result<(), crate::error::DucyError> {
        self.state.add_player(cards)
    }
}

pub struct StudGameEvaluation;

impl GameEvaluation<StudGameState, StandardHandRanks> for StudGameEvaluation {
    fn evaluate_winners(&self, game_state: &StudGameState) -> Vec<GameWinner<StandardHandRanks>> {
        let mut tracker = WinnerTracker::new();
        for (i, hand) in game_state.state.hole_cards().iter().enumerate() {
            let rank = StandardHandRanker::get_rank(hand);
            tracker.consider(i, rank);
        }
        tracker.into_results()
    }
}

#[cfg(test)]
mod test {
    use rust_decimal_macros::dec;

    use super::*;
    use crate::deck::{Deck, Rank};
    use crate::games::GameEvaluation;
    use crate::ranking::hand_rank::StandardHandRanks;

    #[test]
    fn test_best_five_from_seven() {
        let mut state = StudGameState::new();
        // Player 0: flush among 7 cards
        state
            .add_player(Deck::parse("As Ks Qs Js 2s 3d 4h").unwrap())
            .unwrap();
        // Player 1: two pair
        state
            .add_player(Deck::parse("Ac Ad Kc Kd 5h 6h 7h").unwrap())
            .unwrap();

        let eval = StudGameEvaluation;
        let winners = eval.evaluate_winners(&state);
        assert_eq!(winners.len(), 1);
        assert_eq!(winners[0].player_index(), 0);
        assert!(matches!(
            winners[0].winning_hand(),
            StandardHandRanks::Flush { .. }
        ));
    }

    #[test]
    fn test_full_house_beats_flush() {
        let mut state = StudGameState::new();
        // Player 0: flush
        state
            .add_player(Deck::parse("2s 4s 6s 8s Ts 3d 5h").unwrap())
            .unwrap();
        // Player 1: full house
        state
            .add_player(Deck::parse("Ac Ad Ah Kc Kd 2h 3c").unwrap())
            .unwrap();

        let eval = StudGameEvaluation;
        let winners = eval.evaluate_winners(&state);
        assert_eq!(winners.len(), 1);
        assert_eq!(winners[0].player_index(), 1);
        assert_eq!(
            *winners[0].winning_hand(),
            StandardHandRanks::FullHouse {
                t: Rank::Ace,
                p: Rank::King
            }
        );
    }

    #[test]
    fn test_tie_splits() {
        let mut state = StudGameState::new();
        state
            .add_player(Deck::parse("Ac Kd Qh Js Tc 2d 3h").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("Ad Kh Qs Jc Td 4s 5c").unwrap())
            .unwrap();

        let eval = StudGameEvaluation;
        let winners = eval.evaluate_winners(&state);
        assert_eq!(winners.len(), 2);
        assert_eq!(winners[0].pot_amount(), dec!(0.5));
    }
}
