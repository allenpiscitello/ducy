use rust_decimal_macros::dec;

use crate::games::dealt_hand::DealtHandGameState;
use crate::games::{GameState, GameWinner, WinnerTracker};
use crate::ranking::hand_rank::{StandardHandRanker, StandardHandRanks};
use crate::ranking::low_hand_rank::{LowHandRanker, LowHandRanks};

/// Game state for Seven-Card Stud Hi-Lo 8-or-Better.
/// Each player receives 7 cards. The pot is split between the best
/// high hand and the best qualifying low hand (8-or-better).
/// If no low qualifies, the high hand scoops.
pub struct StudHiLoGameState {
    pub(crate) state: DealtHandGameState,
}

impl GameState for StudHiLoGameState {}

impl Default for StudHiLoGameState {
    fn default() -> Self {
        Self {
            state: DealtHandGameState::new(7),
        }
    }
}

impl StudHiLoGameState {
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

/// The winners of each half of a hi-lo pot.
pub struct HiLoResult {
    /// The best high hand (more than one on a tie).
    pub high_winners: Vec<GameWinner<StandardHandRanks>>,
    /// The best qualifying 8-or-better low; empty if no one has one, and
    /// the high hand scoops.
    pub low_winners: Vec<GameWinner<LowHandRanks>>,
}

/// Finds the winners of a [`StudHiLoGameState`]: the best high hand, and the
/// best 8-or-better low, ties sharing.
pub struct StudHiLoGameEvaluation;

impl StudHiLoGameEvaluation {
    /// The high and low winners of `game_state`.
    pub fn evaluate_winners(&self, game_state: &StudHiLoGameState) -> HiLoResult {
        let mut high_tracker = WinnerTracker::new();
        let mut low_tracker = WinnerTracker::new();

        for (i, hand) in game_state.state.hole_cards().iter().enumerate() {
            let high_rank = StandardHandRanker::get_rank(hand);
            high_tracker.consider(i, high_rank);

            for combo in hand.enumerate_combinations(5) {
                if let Some(low_rank) =
                    LowHandRanker::get_rank_at_least(&combo, low_tracker.best_hand())
                {
                    low_tracker.consider(i, low_rank);
                }
            }
        }

        let has_low = low_tracker.best_hand().is_some()
            && low_tracker.best_hand() != Some(LowHandRanks::NoLow);

        if has_low {
            let mut high_results = high_tracker.into_results();
            let mut low_results = low_tracker.into_results();
            for w in &mut high_results {
                w.pot_amount *= dec!(0.5);
            }
            for w in &mut low_results {
                w.pot_amount *= dec!(0.5);
            }
            HiLoResult {
                high_winners: high_results,
                low_winners: low_results,
            }
        } else {
            HiLoResult {
                high_winners: high_tracker.into_results(),
                low_winners: vec![],
            }
        }
    }
}

#[cfg(test)]
mod test {
    use rust_decimal_macros::dec;

    use super::*;
    use crate::deck::Deck;

    #[test]
    fn test_high_scoops_no_low() {
        let mut state = StudHiLoGameState::new();
        state
            .add_player(Deck::parse("Ks Kd Kh Qs Qd 9c 8c").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("Jc Jd Jh Tc Td 9h 8h").unwrap())
            .unwrap();

        let eval = StudHiLoGameEvaluation;
        let result = eval.evaluate_winners(&state);
        assert_eq!(result.high_winners.len(), 1);
        assert_eq!(result.high_winners[0].player_index(), 0);
        assert_eq!(result.high_winners[0].pot_amount(), dec!(1));
        assert!(result.low_winners.is_empty());
    }

    #[test]
    fn test_split_pot() {
        let mut state = StudHiLoGameState::new();
        // Player 0: high hand (kings full), no low
        state
            .add_player(Deck::parse("Ks Kd Kh Qs Qd 9c Tc").unwrap())
            .unwrap();
        // Player 1: low hand qualifies (A-2-3-4-7)
        state
            .add_player(Deck::parse("Ac 2d 3h 4s 7c Jd Th").unwrap())
            .unwrap();

        let eval = StudHiLoGameEvaluation;
        let result = eval.evaluate_winners(&state);
        assert_eq!(result.high_winners.len(), 1);
        assert_eq!(result.high_winners[0].player_index(), 0);
        assert_eq!(result.high_winners[0].pot_amount(), dec!(0.5));
        assert_eq!(result.low_winners.len(), 1);
        assert_eq!(result.low_winners[0].player_index(), 1);
        assert_eq!(result.low_winners[0].pot_amount(), dec!(0.5));
    }

    #[test]
    fn test_scoop_high_and_low() {
        let mut state = StudHiLoGameState::new();
        // Player 0: wheel straight + best low
        state
            .add_player(Deck::parse("Ac 2d 3h 4s 5c Kd Qh").unwrap())
            .unwrap();
        // Player 1: pair of jacks, no low (no straight possible)
        state
            .add_player(Deck::parse("Jc Jd 9h Tc 6s Kh 2s").unwrap())
            .unwrap();

        let eval = StudHiLoGameEvaluation;
        let result = eval.evaluate_winners(&state);
        // Player 0 has a straight for high, and the wheel for low
        assert_eq!(result.high_winners.len(), 1);
        assert_eq!(result.high_winners[0].player_index(), 0);
        assert_eq!(result.low_winners.len(), 1);
        assert_eq!(result.low_winners[0].player_index(), 0);
    }

    #[test]
    fn test_multiple_low_qualifiers() {
        let mut state = StudHiLoGameState::new();
        // Player 0: high hand + low hand
        state
            .add_player(Deck::parse("Ac 2d 3h 4s 8c Kd Qh").unwrap())
            .unwrap();
        // Player 1: better low hand
        state
            .add_player(Deck::parse("Ad 2h 3s 4c 5d Jh Tc").unwrap())
            .unwrap();

        let eval = StudHiLoGameEvaluation;
        let result = eval.evaluate_winners(&state);
        assert_eq!(result.low_winners.len(), 1);
        assert_eq!(result.low_winners[0].player_index(), 1);
    }
}
