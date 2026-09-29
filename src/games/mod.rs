use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::ranking::hand_rank::HandRanking;

/// Badugi game state and evaluation.
pub mod badugi;
/// Dealt-hand game state (shared by draw and stud variants).
pub mod dealt_hand;
/// 2-7 Triple Draw Lowball game state and evaluation.
pub mod deuce_to_seven;
/// Flop-based game state management (shared by Hold'em and Omaha).
pub mod flop_game;
/// Texas Hold'em game state, evaluation, and ranges.
pub mod holdem;
/// Omaha game state, evaluation, and board analysis.
pub mod omaha;
/// Omaha bomb pot with multiple boards.
pub mod omaha_bomb_pot;
/// Omaha Hi-Lo 8-or-Better game state and split-pot evaluation.
pub mod omaha_hilo;
/// Razz (Seven-Card Stud Low) game state and evaluation.
pub mod razz;

/// Marker trait for game state types.
pub trait GameState {}

/// A winning player's result: their index, pot share, and best hand.
#[derive(Eq, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GameWinner<H: HandRanking> {
    pub(crate) player_index: usize,
    pub(crate) pot_amount: Decimal,
    pub(crate) winning_hand: H,
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

#[cfg(feature = "parallel")]
const PARALLEL_THRESHOLD: usize = 100;

pub(crate) fn accumulate_equity<F>(
    runouts: Vec<crate::deck::Deck>,
    num_players: usize,
    eval: F,
) -> Vec<Decimal>
where
    F: Fn(&crate::deck::Deck, &mut [u64]) + Send + Sync,
{
    let hand_count = runouts.len() as u64;
    let win_shares = {
        #[cfg(feature = "parallel")]
        {
            if runouts.len() >= PARALLEL_THRESHOLD {
                use rayon::prelude::*;
                runouts
                    .par_iter()
                    .fold(
                        || vec![0u64; num_players],
                        |mut shares, community| {
                            eval(community, &mut shares);
                            shares
                        },
                    )
                    .reduce(
                        || vec![0u64; num_players],
                        |mut a, b| {
                            for (i, &v) in b.iter().enumerate() {
                                a[i] += v;
                            }
                            a
                        },
                    )
            } else {
                let mut shares = vec![0u64; num_players];
                for community in &runouts {
                    eval(community, &mut shares);
                }
                shares
            }
        }
        #[cfg(not(feature = "parallel"))]
        {
            let mut shares = vec![0u64; num_players];
            for community in &runouts {
                eval(community, &mut shares);
            }
            shares
        }
    };

    let divisor = Decimal::from(EQUITY_SCALE) * Decimal::from(hand_count);
    win_shares
        .iter()
        .map(|&s| Decimal::from(s) / divisor)
        .collect()
}

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
