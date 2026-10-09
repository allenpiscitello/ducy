//! Omaha showdowns and equity, fast enough for training.
//!
//! An Omaha hand plays exactly two hole cards and three board cards, so its
//! strength on a river is the best of 6 × 10 = 60 five-card hands (for four
//! hole cards). [`RiverBoard`] keeps the board's ten triples, so scoring a
//! hand is 60 calls to ducy's bit-twiddling scorer and nothing else.
//!
//! Exact equity against every opponent hand (123,410 of them once four cards
//! are out) is too slow to use in training, so [`equity_vs_random`] samples:
//! a random runout and a random opponent hand per sample, with a standard
//! error to say how far off it can be. For many hands on one river,
//! [`PairTable`] scores every two-card pair once, after which any hand's
//! strength is a handful of table lookups.

use crate::{
    holdem::cards::{Card, NUM_CARDS, bit, hole_index, mask, score},
    rng::Rng,
};

/// A complete board's ten three-card subsets, as card masks.
#[derive(Clone, Debug)]
pub struct RiverBoard {
    triples: [u64; 10],
}

impl RiverBoard {
    pub fn new(board: &[Card; 5]) -> Self {
        let mut triples = [0u64; 10];
        let mut i = 0;
        for a in 0..5 {
            for b in a + 1..5 {
                for c in b + 1..5 {
                    triples[i] = bit(board[a]) | bit(board[b]) | bit(board[c]);
                    i += 1;
                }
            }
        }
        Self { triples }
    }

    /// The best five-card hand from these two hole cards and three of the
    /// board's.
    #[inline]
    pub fn pair_score(&self, a: Card, b: Card) -> u32 {
        let two = bit(a) | bit(b);
        let mut best = 0;
        for &t in &self.triples {
            best = best.max(score(two | t));
        }
        best
    }

    /// A hand's showdown strength (higher is better): exactly two of `hole`
    /// and three of the board.
    pub fn score(&self, hole: &[Card]) -> u32 {
        let mut best = 0;
        for i in 0..hole.len() {
            for j in i + 1..hole.len() {
                best = best.max(self.pair_score(hole[i], hole[j]));
            }
        }
        best
    }
}

/// The showdown between two hands on a full board: 1 if `a` wins, -1 if it
/// loses, 0 for a split.
pub fn showdown(a: &[Card], b: &[Card], board: &[Card; 5]) -> i8 {
    let river = RiverBoard::new(board);
    river.score(a).cmp(&river.score(b)) as i8
}

/// Every two-card pair's best hand on one river, so a hand's strength is
/// the best of its pairs' entries (6 lookups for four hole cards).
#[derive(Clone, Debug)]
pub struct PairTable {
    scores: Vec<u32>,
}

impl PairTable {
    /// Scores the 1,081 pairs the board leaves (pairs using a board card
    /// score 0).
    pub fn new(board: &[Card; 5]) -> Self {
        let river = RiverBoard::new(board);
        let used = mask(board);
        let mut scores = vec![0u32; NUM_CARDS * (NUM_CARDS - 1) / 2];
        for hi in 1..NUM_CARDS as Card {
            if used & bit(hi) != 0 {
                continue;
            }
            for lo in 0..hi {
                if used & bit(lo) == 0 {
                    scores[hole_index(hi, lo)] = river.pair_score(hi, lo);
                }
            }
        }
        Self { scores }
    }

    #[inline]
    pub fn score(&self, hole: &[Card]) -> u32 {
        let mut best = 0;
        for i in 0..hole.len() {
            for j in i + 1..hole.len() {
                best = best.max(self.scores[hole_index(hole[i], hole[j])]);
            }
        }
        best
    }
}

/// A sampled estimate: the mean and its standard error.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Estimate {
    pub mean: f64,
    pub std_error: f64,
}

/// Draws a card not in `used` and adds it.
#[inline]
pub fn draw(used: &mut u64, rng: &mut Rng) -> Card {
    loop {
        let c = (rng.next_u64() % NUM_CARDS as u64) as Card;
        if *used & bit(c) == 0 {
            *used |= bit(c);
            return c;
        }
    }
}

/// `hole`'s equity against a random hand of the same size, on `board` (0,
/// 3, 4 or 5 cards): the share of `samples` random runouts and opponent hands
/// it wins, ties counting half.
///
/// The standard error is at most 0.5/√samples: ±1.0% (one standard error)
/// at 2,500 samples, ±0.5% at 10,000.
pub fn equity_vs_random(hole: &[Card], board: &[Card], samples: usize, rng: &mut Rng) -> Estimate {
    let k = hole.len();
    let base = mask(hole) | mask(board);
    let mut full = [0 as Card; 5];
    full[..board.len()].copy_from_slice(board);
    // On the river every sample shares the board: score pairs once.
    let table = (board.len() == 5).then(|| PairTable::new(&full));
    let mine_on_river = table.as_ref().map(|t| t.score(hole));
    let mut opp = [0 as Card; 6];
    let mut total = 0.0;
    let mut squares = 0.0;
    for _ in 0..samples {
        let mut used = base;
        for c in full.iter_mut().skip(board.len()) {
            *c = draw(&mut used, rng);
        }
        for c in opp.iter_mut().take(k) {
            *c = draw(&mut used, rng);
        }
        let (mine, theirs) = match (&table, mine_on_river) {
            (Some(t), Some(m)) => (m, t.score(&opp[..k])),
            _ => {
                let river = RiverBoard::new(&full);
                (river.score(hole), river.score(&opp[..k]))
            }
        };
        let r = match mine.cmp(&theirs) {
            std::cmp::Ordering::Greater => 1.0,
            std::cmp::Ordering::Equal => 0.5,
            std::cmp::Ordering::Less => 0.0,
        };
        total += r;
        squares += r * r;
    }
    let n = samples.max(1) as f64;
    let mean = total / n;
    let var = (squares / n - mean * mean).max(0.0);
    Estimate {
        mean,
        std_error: (var / n).sqrt(),
    }
}
