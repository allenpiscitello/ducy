use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::ranking::hand_rank::HandRanking;

/// Flop-based game state management (shared by Hold'em and Omaha).
pub mod flop_game;
/// Texas Hold'em game state, evaluation, and ranges.
pub mod holdem;
/// Omaha game state, evaluation, and board analysis.
pub mod omaha;

/// Marker trait for game state types.
pub trait GameState {}

/// A winning player's result: their index, pot share, and best hand.
#[derive(Eq, PartialEq, Debug)]
pub struct GameWinner<H: HandRanking> {
    pub player_index: usize,
    pub pot_amount: Decimal,
    pub winning_hand: H,
}

impl<H: HandRanking> GameWinner<H> {
    /// Creates a new winner result.
    pub fn new(index: usize, pot_amount: Decimal, winning_hand: H) -> Self {
        Self {
            player_index: index,
            pot_amount,
            winning_hand,
        }
    }

    pub fn player_index(&self) -> usize {
        self.player_index
    }

    pub fn pot_amount(&self) -> Decimal {
        self.pot_amount
    }

    pub fn winning_hand(&self) -> &H {
        &self.winning_hand
    }
}

pub(crate) struct WinnerTracker<H: HandRanking + Ord + Copy> {
    best_hand: Option<H>,
    winners: Vec<usize>,
}

impl<H: HandRanking + Ord + Copy> WinnerTracker<H> {
    pub fn new() -> Self {
        Self {
            best_hand: None,
            winners: vec![],
        }
    }

    pub fn best_hand(&self) -> Option<H> {
        self.best_hand
    }

    pub fn consider(&mut self, player_index: usize, hand: H) {
        match &self.best_hand {
            Some(best) => match best.cmp(&hand) {
                std::cmp::Ordering::Less => {
                    self.best_hand = Some(hand);
                    self.winners = vec![player_index];
                }
                std::cmp::Ordering::Equal => self.winners.push(player_index),
                std::cmp::Ordering::Greater => {}
            },
            None => {
                self.best_hand = Some(hand);
                self.winners = vec![player_index];
            }
        }
    }

    pub fn into_results(self) -> Vec<GameWinner<H>> {
        let winner_count = self.winners.len();
        if let Some(best_hand) = self.best_hand
            && winner_count > 0
        {
            let pot_distribution = dec!(1.0) / Decimal::from(winner_count);
            self.winners
                .iter()
                .map(|x| GameWinner::new(*x, pot_distribution, best_hand))
                .collect()
        } else {
            vec![]
        }
    }
}

// LCM(1..10) — allows exact integer division for any split up to 10 winners
pub(crate) const EQUITY_SCALE: u64 = 2520;

/// Evaluates a game state to determine winners.
pub trait GameEvaluation<GS: GameState, H: HandRanking> {
    /// Returns the winners for the given game state.
    fn evaluate_winners(&self, game_state: &GS) -> Vec<GameWinner<H>>;
}

/// Evaluates equity (expected pot share) across all possible runouts.
pub trait GameEquityEvaluation<GS: GameState, H: HandRanking, GE: GameEvaluation<GS, H>> {
    /// Returns each player's equity as a fraction of the pot.
    fn evaluate_equity(&self, game_state: &GS) -> Vec<Decimal>;
}
