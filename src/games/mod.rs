use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use rand::{RngExt, SeedableRng, rngs::StdRng};

use crate::deck::Deck;
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
/// Omaha starting-hand analysis: features, tags, nut-flush blocking and a
/// percentile ranking.
pub mod omaha_analysis;
/// Omaha bomb pot with multiple boards.
pub mod omaha_bomb_pot;
/// Omaha Hi-Lo 8-or-Better game state and split-pot evaluation.
pub mod omaha_hilo;
/// Omaha starting-hand ranges (PPT-style syntax) and preflop coverage.
/// See [`omaha_range::OmahaRange`] for the full syntax.
pub mod omaha_range;
/// Monte Carlo equity for Omaha and Omaha Hi-Lo players holding shorthand ranges.
pub mod omaha_range_equity;
/// The fitted Omaha starting-hand ranking model (generated).
pub mod omaha_rank_model;
/// Razz (Seven-Card Stud Low) game state and evaluation.
pub mod razz;
/// Single Draw Deuce-to-Seven Lowball (Kansas City Lowball) game state and evaluation.
pub mod single_draw_27;
/// Single Draw Ace-to-Five Lowball (California Lowball) game state and evaluation.
pub mod single_draw_a5;
/// Seven-Card Stud (high only) game state and evaluation.
pub mod stud;
/// Seven-Card Stud Hi-Lo 8-or-Better game state and evaluation.
pub mod stud_hilo;

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

    /// The winner's index, in the order players were added.
    pub fn player_index(&self) -> usize {
        self.player_index
    }

    /// The share of the pot this winner takes (1 split by the number of
    /// winners, or by the share of runouts won in an equity calculation).
    pub fn pot_amount(&self) -> Decimal {
        self.pot_amount
    }

    /// The hand that won.
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
                std::cmp::Ordering::Equal => {
                    if !self.winners.contains(&player_index) {
                        self.winners.push(player_index);
                    }
                }
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

pub(crate) struct FastWinnerTracker {
    best_score: u32,
    winners: Vec<usize>,
}

impl FastWinnerTracker {
    pub fn new() -> Self {
        Self {
            best_score: 0,
            winners: Vec::with_capacity(4),
        }
    }

    #[inline(always)]
    pub fn best_score(&self) -> u32 {
        self.best_score
    }

    #[inline(always)]
    pub fn consider(&mut self, player_index: usize, score: u32) {
        if score > self.best_score {
            self.best_score = score;
            self.winners.clear();
            self.winners.push(player_index);
        } else if score == self.best_score && !self.winners.contains(&player_index) {
            self.winners.push(player_index);
        }
    }

    pub fn distribute(&self, shares: &mut [u64]) {
        if let Some(share) = EQUITY_SCALE.checked_div(self.winners.len() as u64) {
            for &w in &self.winners {
                shares[w] += share;
            }
        }
    }

    pub fn winners(&self) -> &[usize] {
        &self.winners
    }

    pub fn distribute_scaled(&self, shares: &mut [u64], scale: u64) {
        if let Some(share) = scale.checked_div(self.winners.len() as u64) {
            for &w in &self.winners {
                shares[w] += share;
            }
        }
    }
}

/// Deals random cards from a fixed set using a partial Fisher-Yates shuffle.
/// Owns its random generator so sampling can be made reproducible.
pub(crate) struct CardDealer {
    cards: Vec<Deck>,
    dealt: usize,
    rng: StdRng,
}

impl CardDealer {
    /// A dealer seeded from the system's random source.
    pub fn new(deck: Deck) -> Self {
        Self::with_rng(deck, rand::make_rng())
    }

    /// A dealer whose deals are fully determined by `seed`.
    pub fn seeded(deck: Deck, seed: u64) -> Self {
        Self::with_rng(deck, StdRng::seed_from_u64(seed))
    }

    /// `seeded` when a seed is given, otherwise `new`.
    pub fn maybe_seeded(deck: Deck, seed: Option<u64>) -> Self {
        seed.map_or_else(|| Self::new(deck), |s| Self::seeded(deck, s))
    }

    fn with_rng(deck: Deck, rng: StdRng) -> Self {
        Self {
            cards: deck.enumerate_combinations(1).collect(),
            dealt: 0,
            rng,
        }
    }

    /// Uniform value in `0.0..below`.
    pub fn random_f64(&mut self, below: f64) -> f64 {
        self.rng.random_range(0.0..below)
    }

    pub fn available(&self) -> usize {
        self.cards.len()
    }

    /// Returns every card to the deck.
    pub fn reset(&mut self) {
        self.dealt = 0;
    }

    /// Deals `n` cards not already dealt since the last reset. Panics if too few remain.
    pub fn deal(&mut self, n: usize) -> Deck {
        let mut hand = Deck::empty();
        for _ in 0..n {
            let j = self.rng.random_range(self.dealt..self.cards.len());
            self.cards.swap(self.dealt, j);
            hand |= self.cards[self.dealt];
            self.dealt += 1;
        }
        hand
    }

    /// Like `deal`, but never deals a card in `exclude`. Panics if too few
    /// non-excluded cards remain.
    pub fn deal_excluding(&mut self, n: usize, exclude: Deck) -> Deck {
        let exclude = u64::from(exclude);
        let mut hand = Deck::empty();
        let mut dealt_to_hand = 0;
        while dealt_to_hand < n {
            let j = self.rng.random_range(self.dealt..self.cards.len());
            self.cards.swap(self.dealt, j);
            let card = self.cards[self.dealt];
            self.dealt += 1;
            if u64::from(card) & exclude == 0 {
                hand |= card;
                dealt_to_hand += 1;
            }
        }
        hand
    }
}

// LCM(1..10) — allows exact integer division for any split up to 10 winners
pub(crate) const EQUITY_SCALE: u64 = 2520;

#[cfg(feature = "parallel")]
const PARALLEL_THRESHOLD: usize = 100;

/// Unnormalized equity totals over a set of runouts. Results for separate
/// chunks of runouts (e.g. computed on different threads or Web Workers)
/// combine with [`EquityShares::merge`]; [`EquityShares::equity`] finishes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EquityShares {
    /// Number of runouts evaluated.
    pub runouts: u64,
    /// Per player, pot shares won in units of [`EquityShares::SCALE`] per runout.
    pub shares: Vec<u64>,
}

impl EquityShares {
    /// Units per whole pot: every split among up to 10 winners divides evenly.
    pub const SCALE: u64 = EQUITY_SCALE;

    /// Adds another chunk's totals into this one.
    pub fn merge(&mut self, other: &EquityShares) {
        if self.shares.len() < other.shares.len() {
            self.shares.resize(other.shares.len(), 0);
        }
        for (s, o) in self.shares.iter_mut().zip(&other.shares) {
            *s += o;
        }
        self.runouts += other.runouts;
    }

    /// Each player's equity; all zeros when no runouts were evaluated.
    pub fn equity(&self) -> Vec<Decimal> {
        if self.runouts == 0 {
            return vec![Decimal::ZERO; self.shares.len()];
        }
        let divisor = Decimal::from(Self::SCALE) * Decimal::from(self.runouts);
        self.shares
            .iter()
            .map(|&s| Decimal::from(s) / divisor)
            .collect()
    }

    /// Each player's summed equity over the runouts (shares / SCALE).
    pub fn equity_sum(&self) -> Vec<f64> {
        self.shares
            .iter()
            .map(|&s| s as f64 / Self::SCALE as f64)
            .collect()
    }
}

pub(crate) fn accumulate_equity<F>(
    runouts: Vec<crate::deck::Deck>,
    num_players: usize,
    eval: F,
) -> Vec<Decimal>
where
    F: Fn(&crate::deck::Deck, &mut [u64]) + Send + Sync,
{
    accumulate_shares(runouts, num_players, eval).equity()
}

pub(crate) fn accumulate_shares<F>(
    runouts: Vec<crate::deck::Deck>,
    num_players: usize,
    eval: F,
) -> EquityShares
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

    EquityShares {
        runouts: hand_count,
        shares: win_shares,
    }
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
