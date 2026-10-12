//! A card abstraction for heads-up Omaha, computed from features (#128).
//!
//! Hold'em's abstraction looks every flop and turn hand up in a table. Omaha
//! has about 10⁸ flop hands and 10¹⁰ turn hands, and sampling equity during
//! training is far too slow (see `plo_bench`), so buckets come from cheap,
//! exact features of the hand on the board:
//!
//! - **Preflop:** four-card hands are grouped by suit isomorphism (16,432
//!   classes). Each class's equity against a random hand and ducy's
//!   playability percentile (`omaha_analysis`) are clustered with k-means;
//!   `preflop: 0` keeps every class as its own bucket instead. Five and six
//!   cards have far too many hands to list (20 million six-card hands), so
//!   there a linear model predicts equity from features of the hand (its
//!   playability, pairs, suits, ranks and connectedness), and k-means over
//!   (predicted equity, playability) forms the buckets, as on the flop.
//! - **Flop and turn:** made-hand strength (where the hand's best two-plus-
//!   three ranks among every two-card holding on this board), nut and
//!   non-nut flush draws, straight outs, and the board's texture. A linear
//!   model fitted to sampled equity turns them into predicted equity, and
//!   k-means over (predicted equity, made strength) forms the buckets, so a
//!   strong draw and a medium made hand of the same equity stay apart.
//! - **River:** made-hand strength alone, in equal-mass bins. It ranks hands
//!   exactly as their equity against a random hand does.
//!
//! Most of the work is per board (scoring every pair on it), so
//! [`Buckets::deal_buckets`] does it once for both players. The model is a
//! few kilobytes, plus a 33 KB preflop table for four cards: small enough to
//! ship.
//!
//! **Nut-aware abstractions** (`nut_aware`, #204, saved as version 3).
//! Equity against a random hand misses what matters most in PLO, above all
//! with six cards: whether a hand can make the nuts. A nut flush draw and a
//! low one, or the top and bottom end of a straight, have similar equity
//! against a random hand and very different value against a range that
//! continues. So these abstractions:
//!
//! - fit a second model, of **nut potential**: how close to the nuts the
//!   hand ends on the river ([`nutness`] of its exact share of beaten
//!   two-card holdings, averaged over sampled runouts), from more features
//!   ([`BoardView::nut_features`], [`preflop_nut_features`]: closeness to
//!   the nuts now, nut straight outs, second-nut and backdoor nut flush
//!   draws, blockers, suited aces and kings, double suits, high pairs,
//!   rundowns);
//! - cluster on (predicted equity, predicted nut potential, made strength
//!   or playability) on every street before the river.
//!
//! The river is bucketed as before: equity against sampled opponents
//! already separates nut potential there about as well as anything (the
//! hand is made). Exact made strength, with buckets packed towards the
//! top, was tried and spread equity within buckets nearly three times as
//! much for no gain in nut potential. Version 1 and 2 files load and play
//! as before.

use super::showdown::{RiverBoard, draw, equity_vs_random};
use crate::{
    holdem::{
        cards::{Card, NUM_CARDS, bit, hole_index, mask, rank, score, suit, to_string},
        hunl::Buckets,
        kmeans::{Distance, kmeans, nearest},
    },
    rng::Rng,
};

/// Settings for building a [`PloAbstraction`].
#[derive(Clone, Debug, PartialEq)]
pub struct PloAbstractionConfig {
    /// Hole cards: 4 for PLO, 5 or 6 for the bigger games.
    pub hole_cards: usize,
    /// Preflop buckets, or 0 for one per suit-isomorphism class (four cards
    /// only).
    pub preflop: usize,
    pub flop: usize,
    pub turn: usize,
    pub river: usize,
    /// Sampled hands per street for fitting.
    pub fit_hands: usize,
    /// Equity samples per fitted hand (and per preflop class).
    pub equity_samples: usize,
    pub seed: u64,
    /// Nut potential as well as equity (see the module docs).
    pub nut_aware: bool,
}

impl Default for PloAbstractionConfig {
    fn default() -> Self {
        Self {
            hole_cards: 4,
            preflop: 500,
            flop: 200,
            turn: 200,
            river: 200,
            fit_hands: 40_000,
            equity_samples: 1000,
            seed: 1,
            nut_aware: true,
        }
    }
}

/// Features per hand for the equity model.
pub const NUM_FEATURES: usize = 14;

/// Features per hand on the flop and turn for nut-aware models.
pub const NUM_NUT_FEATURES: usize = 24;

/// Features per starting hand for nut-aware preflop models.
pub const NUM_PREFLOP_NUT_FEATURES: usize = 20;

/// How far from the nuts still counts: a hand beaten by this share of
/// two-card holdings or more has no [`nutness`].
pub const NUT_BAND: f64 = 0.03;

/// Runouts per fitted hand for its nut potential.
pub const NUT_RUNOUTS: usize = 50;

/// How much predicted nut potential counts in clustering (it runs from 0
/// to about 0.6, equity from 0 to 1).
const NUT_WEIGHT: f64 = 1.0;

/// How close to the nuts a hand is, from the share of two-card holdings
/// that beat it (ties half): 1 for the nuts, falling to 0 at [`NUT_BAND`].
pub fn nutness(beaten_by: f64) -> f64 {
    (1.0 - beaten_by / NUT_BAND).max(0.0)
}

/// The share of two-card holdings that beat `hole` on a full board (ties
/// half), exactly; holdings using one of `hole`'s cards are left out, so
/// blockers count.
pub fn beaten_share(hole: &[Card], board: &[Card; 5]) -> f64 {
    let river = RiverBoard::new(board);
    let s = river.score(hole);
    let used = mask(hole) | mask(board);
    let (mut above, mut ties, mut n) = (0u32, 0u32, 0u32);
    for hi in 1..NUM_CARDS as Card {
        if used & bit(hi) != 0 {
            continue;
        }
        for lo in 0..hi {
            if used & bit(lo) != 0 {
                continue;
            }
            n += 1;
            let t = river.pair_score(hi, lo);
            if t > s {
                above += 1;
            } else if t == s {
                ties += 1;
            }
        }
    }
    (above as f64 + ties as f64 / 2.0) / n.max(1) as f64
}

/// A hand's nut potential: its mean [`nutness`] on the river over `runouts`
/// random completions of `board` (exact on a full board).
pub fn nut_potential(hole: &[Card], board: &[Card], runouts: usize, rng: &mut Rng) -> f64 {
    let fixed = mask(hole) | mask(board);
    let runouts = if board.len() == 5 { 1 } else { runouts.max(1) };
    let mut total = 0.0;
    for _ in 0..runouts {
        let mut used = fixed;
        let mut full = [0 as Card; 5];
        full[..board.len()].copy_from_slice(board);
        for c in full.iter_mut().skip(board.len()) {
            *c = draw(&mut used, rng);
        }
        total += nutness(beaten_share(hole, &full));
    }
    total / runouts as f64
}

/// The board's side of the features: every two-card pair's best hand on it,
/// and its texture. Built once per board, shared by every hand on it.
#[derive(Clone, Debug)]
pub struct BoardView {
    board: Vec<Card>,
    /// Best score of each pair (by `hole_index`), 0 for pairs using a board
    /// card.
    table: Vec<u32>,
    /// The valid pairs' scores, sorted.
    sorted: Vec<u32>,
    ranks: u16,
    suits: [u8; 4],
    paired: bool,
    /// Some five-rank window holds three of the board's ranks.
    straighty: bool,
    /// On the river: [`RIVER_OPPONENTS`] random opponent hands' scores and
    /// cards. They're drawn with a seed from the board, so a hand's river
    /// equity (and bucket) depends only on the cards.
    opponents: Vec<(u32, u64)>,
}

/// Opponent hands sampled per river board for equity: ±2.2% (one standard
/// error) for a hand near 50%, with four hole cards.
pub const RIVER_OPPONENTS: usize = 512;

/// River opponents sampled for `k` hole cards. Bigger hands share a card
/// with more of them (about 58% of six-card opponents hold one of a six-card
/// hand's cards, against 31% for four), so five and six cards sample twice
/// as many.
pub fn river_opponents(k: usize) -> usize {
    if k > 4 {
        2 * RIVER_OPPONENTS
    } else {
        RIVER_OPPONENTS
    }
}

/// Rank bits for straights: bit `r + 1` for rank `r`, and bit 0 for an ace.
fn rank_bits(cards: &[Card]) -> u16 {
    cards.iter().fold(0, |m, &c| {
        let r = rank(c);
        let b = 1u16 << (r + 1);
        if r == 12 { m | b | 1 } else { m | b }
    })
}

/// Whether two of the hand's ranks and three of the board's make a
/// straight: some five-rank window whose ranks the hand doesn't hold are
/// all on the board, with at least two from the hand and three on the
/// board (then three board ranks covering the hand's gaps, and two hand
/// ranks for the rest, always exist).
fn has_straight(hand: u16, board: u16) -> bool {
    (0..10).any(|w| {
        let win = 0b11111u16 << w;
        win & !hand & !board == 0
            && (win & hand).count_ones() >= 2
            && (win & board).count_ones() >= 3
    })
}

impl BoardView {
    /// The view of a 3-, 4- or 5-card board, for four-card hands.
    pub fn new(board: &[Card]) -> Self {
        Self::for_hands(board, 4)
    }

    /// The view of a 3-, 4- or 5-card board for `k`-card hands (the size of
    /// the river's sampled opponents).
    pub fn for_hands(board: &[Card], k: usize) -> Self {
        Self::build(board, k, true)
    }

    /// The view without the river's sampled opponents, for nut-aware
    /// abstractions (their river buckets use exact made strength).
    pub fn exact(board: &[Card]) -> Self {
        Self::build(board, 4, false)
    }

    fn build(board: &[Card], k: usize, opponents: bool) -> Self {
        let n = board.len();
        let mut triples = Vec::with_capacity(10);
        for a in 0..n {
            for b in a + 1..n {
                for c in b + 1..n {
                    triples.push(bit(board[a]) | bit(board[b]) | bit(board[c]));
                }
            }
        }
        let used = mask(board);
        let mut table = vec![0u32; NUM_CARDS * (NUM_CARDS - 1) / 2];
        let mut sorted = Vec::with_capacity(1081);
        for hi in 1..NUM_CARDS as Card {
            if used & bit(hi) != 0 {
                continue;
            }
            for lo in 0..hi {
                if used & bit(lo) != 0 {
                    continue;
                }
                let two = bit(hi) | bit(lo);
                let s = triples.iter().map(|&t| score(two | t)).max().unwrap_or(0);
                table[hole_index(hi, lo)] = s;
                sorted.push(s);
            }
        }
        sorted.sort_unstable();
        let mut suits = [0u8; 4];
        let mut counts = [0u8; 13];
        for &c in board {
            suits[suit(c) as usize] += 1;
            counts[rank(c) as usize] += 1;
        }
        let ranks = rank_bits(board);
        let mut view = Self {
            board: board.to_vec(),
            table,
            sorted,
            ranks,
            suits,
            paired: counts.iter().any(|&c| c >= 2),
            straighty: (0..10).any(|w| (ranks & 0b11111 << w).count_ones() >= 3),
            opponents: Vec::new(),
        };
        if n == 5 && opponents {
            let mut rng = Rng::new(used.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
            let mut hand = [0 as Card; 6];
            let hand = &mut hand[..k];
            view.opponents = (0..river_opponents(k))
                .map(|_| {
                    let mut taken = used;
                    for c in hand.iter_mut() {
                        *c = draw(&mut taken, &mut rng);
                    }
                    (view.best(hand), mask(hand))
                })
                .collect();
        }
        view
    }

    /// On the river: the share of the board's sampled opponent hands this
    /// hand beats, ties half, leaving out opponents holding one of its
    /// cards.
    pub fn river_equity(&self, hole: &[Card]) -> f64 {
        let s = self.best(hole);
        let mine = mask(hole);
        let (mut won, mut n) = (0u32, 0u32);
        for &(theirs, cards) in &self.opponents {
            if cards & mine == 0 {
                won += match s.cmp(&theirs) {
                    std::cmp::Ordering::Greater => 2,
                    std::cmp::Ordering::Equal => 1,
                    std::cmp::Ordering::Less => 0,
                };
                n += 1;
            }
        }
        won as f64 / (2 * n.max(1)) as f64
    }

    /// The hand's best score on this board.
    pub fn best(&self, hole: &[Card]) -> u32 {
        let mut best = 0;
        for i in 0..hole.len() {
            for j in i + 1..hole.len() {
                best = best.max(self.table[hole_index(hole[i], hole[j])]);
            }
        }
        best
    }

    /// Made-hand strength: the share of two-card holdings the hand's best
    /// beats on this board (ties half), from 0 to 1.
    pub fn strength(&self, hole: &[Card]) -> f64 {
        let s = self.best(hole);
        let below = self.sorted.partition_point(|&x| x < s);
        let not_above = self.sorted.partition_point(|&x| x <= s);
        (below as f64 + (not_above - below) as f64 / 2.0) / self.sorted.len().max(1) as f64
    }

    /// The features (see [`NUM_FEATURES`]): a constant, made strength and
    /// its powers, nut and other flush draws and straight outs (none on the
    /// river), and the board's pairing, flush and straight texture with their
    /// interactions with strength.
    pub fn features(&self, hole: &[Card]) -> [f64; NUM_FEATURES] {
        let p = self.strength(hole);
        let p4 = p.powi(4);
        let seen = mask(hole) | mask(&self.board);
        let river = self.board.len() == 5;
        // Flush draws: a suit with two on the board and two or more in hand.
        let (mut nut_fd, mut fd) = (0.0, 0.0);
        for s in (0..4u8).filter(|_| !river) {
            if self.suits[s as usize] != 2 {
                continue;
            }
            let mine: Vec<u8> = hole
                .iter()
                .filter(|&&c| suit(c) == s)
                .map(|&c| rank(c))
                .collect();
            if mine.len() < 2 {
                continue;
            }
            let top = *mine.iter().max().unwrap();
            // The best rank of the suit that isn't on the board.
            let nut = (0..13u8)
                .rev()
                .find(|&r| self.board.iter().all(|&c| c != r * 4 + s))
                .unwrap_or(12);
            if top == nut {
                nut_fd = 1.0;
            } else {
                fd = 1.0;
            }
        }
        // Straight outs: unseen cards whose rank gives a straight we don't
        // have yet.
        let hand = rank_bits(hole);
        let mut outs = 0;
        let mut unseen = 0;
        if !river && !has_straight(hand, self.ranks) {
            for c in 0..NUM_CARDS as Card {
                if seen & bit(c) != 0 {
                    continue;
                }
                if has_straight(hand, self.ranks | rank_bits(&[c])) {
                    outs += 1;
                }
            }
        }
        for c in 0..NUM_CARDS as Card {
            if seen & bit(c) == 0 {
                unseen += 1;
            }
        }
        let so = outs as f64 / unseen.max(1) as f64;
        let paired = if self.paired { 1.0 } else { 0.0 };
        let flushy = if self.suits.iter().any(|&n| n >= 3) {
            1.0
        } else {
            0.0
        };
        let straighty = if self.straighty { 1.0 } else { 0.0 };
        [
            1.0,
            p,
            p4,
            p.powi(16),
            nut_fd,
            fd,
            so,
            so * (1.0 - p4),
            paired,
            paired * p4,
            flushy,
            flushy * p4,
            straighty,
            straighty * p4,
        ]
    }

    /// The best rank of suit `s` that isn't on the board, and the next.
    fn top_missing(&self, s: u8) -> (u8, u8) {
        let mut missing = (0..13u8)
            .rev()
            .filter(|&r| self.board.iter().all(|&c| c != r * 4 + s));
        let first = missing.next().unwrap_or(12);
        (first, missing.next().unwrap_or(first))
    }

    /// Whether the hand's ranks make the best straight two hole ranks can
    /// make with these board ranks (flushes and full houses aside).
    fn holds_nut_straight(hand: u16, board: u16) -> bool {
        match (0..10)
            .rev()
            .find(|&w| (board & 0b11111 << w).count_ones() >= 3)
        {
            Some(w) => {
                let win = 0b11111u16 << w;
                win & !hand & !board == 0 && (win & hand).count_ones() >= 2
            }
            None => false,
        }
    }

    /// The features of nut-aware models (see [`NUM_NUT_FEATURES`]):
    /// [`BoardView::features`], then closeness to the nuts now (its
    /// [`nutness`] and whether it is the nuts), the share of unseen cards
    /// that give it the nut straight, second-nut and backdoor nut flush
    /// draws, a nut flush draw with straight outs, nut draws on weak made
    /// hands, a lone nut-flush card on a flushed board (a blocker), pocket
    /// pairs to make sets with, and the nuts on a paired board.
    pub fn nut_features(&self, hole: &[Card]) -> [f64; NUM_NUT_FEATURES] {
        let base = self.features(hole);
        let (p, p4) = (base[1], base[2]);
        let river = self.board.len() == 5;
        let top = self.sorted.last().copied().unwrap_or(0);
        let is_nuts = if self.best(hole) >= top { 1.0 } else { 0.0 };
        let (mut second_fd, mut backdoor, mut blocker) = (0.0, 0.0, 0.0);
        for s in 0..4u8 {
            let mine: Vec<u8> = hole
                .iter()
                .filter(|&&c| suit(c) == s)
                .map(|&c| rank(c))
                .collect();
            let Some(&high) = mine.iter().max() else {
                continue;
            };
            let (nut, second) = self.top_missing(s);
            let on_board = self.suits[s as usize];
            if !river && on_board == 2 && mine.len() >= 2 && high == second && high != nut {
                second_fd = 1.0;
            }
            if self.board.len() == 3 && on_board == 1 && mine.len() >= 2 && high == nut {
                backdoor = 1.0;
            }
            if on_board >= 3 && mine.len() == 1 && high == nut {
                blocker = 1.0;
            }
        }
        let hand = rank_bits(hole);
        let seen = mask(hole) | mask(&self.board);
        let mut nut_outs = 0.0;
        if !river && !Self::holds_nut_straight(hand, self.ranks) {
            let (mut outs, mut unseen) = (0u32, 0u32);
            for c in (0..NUM_CARDS as Card).filter(|&c| seen & bit(c) == 0) {
                unseen += 1;
                if Self::holds_nut_straight(hand, self.ranks | rank_bits(&[c])) {
                    outs += 1;
                }
            }
            nut_outs = outs as f64 / unseen.max(1) as f64;
        }
        let mut counts = [0u8; 13];
        for &c in hole {
            counts[rank(c) as usize] += 1;
        }
        let set_draws = (0..13u8)
            .filter(|&r| counts[r as usize] >= 2 && self.ranks & 1 << (r + 1) == 0)
            .count() as f64
            / 3.0;
        let near = nutness(1.0 - p);
        let mut x = [0.0; NUM_NUT_FEATURES];
        x[..NUM_FEATURES].copy_from_slice(&base);
        x[NUM_FEATURES..].copy_from_slice(&[
            near,
            is_nuts,
            nut_outs,
            second_fd,
            backdoor,
            base[4] * base[6],
            (base[4] + nut_outs) * (1.0 - p4),
            blocker,
            set_draws,
            near * base[8],
        ]);
        x
    }
}

/// Per-suit rank masks, sorted: equal for hands that differ only by suits.
pub fn canonical_key(hole: &[Card]) -> u64 {
    let mut m = [0u64; 4];
    for &c in hole {
        m[suit(c) as usize] |= 1 << rank(c);
    }
    m.sort_unstable_by(|a, b| b.cmp(a));
    m[0] << 39 | m[1] << 26 | m[2] << 13 | m[3]
}

/// Every starting hand's suit-isomorphism class, with a representative.
#[derive(Clone, Debug)]
pub struct PreflopClasses {
    keys: Vec<u64>,
    reps: Vec<Vec<Card>>,
}

impl PreflopClasses {
    /// Enumerates all `k`-card hands (16,432 classes for four cards).
    pub fn new(k: usize) -> Self {
        let mut found: Vec<(u64, Vec<Card>)> = Vec::new();
        let mut hand = vec![0 as Card; k];
        fn walk(start: Card, at: usize, hand: &mut Vec<Card>, found: &mut Vec<(u64, Vec<Card>)>) {
            if at == hand.len() {
                found.push((canonical_key(hand), hand.clone()));
                return;
            }
            for c in start..NUM_CARDS as Card {
                hand[at] = c;
                walk(c + 1, at + 1, hand, found);
            }
        }
        walk(0, 0, &mut hand, &mut found);
        found.sort_by_key(|(key, _)| *key);
        found.dedup_by_key(|(key, _)| *key);
        let (keys, reps) = found.into_iter().unzip();
        Self { keys, reps }
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The class of `hole`.
    pub fn class(&self, hole: &[Card]) -> usize {
        self.keys
            .binary_search(&canonical_key(hole))
            .expect("every hand has a class")
    }

    /// A hand in class `i`.
    pub fn representative(&self, i: usize) -> &[Card] {
        &self.reps[i]
    }
}

/// A starting hand's features for the preflop model (five and six cards):
/// a constant, ducy's playability and its powers, pairs and trips, how many
/// suits it holds two or more of, a suited ace, a suit held four or more
/// times, its ranks (mean, top, distinct) and the most of its ranks in one
/// five-rank window. Index 1, playability, is what the buckets cluster on
/// besides predicted equity.
pub fn preflop_features(hole: &[Card]) -> [f64; NUM_FEATURES] {
    let k = hole.len() as f64;
    let p = playability(hole);
    let mut counts = [0u8; 13];
    let mut suits = [0u8; 4];
    for &c in hole {
        counts[rank(c) as usize] += 1;
        suits[suit(c) as usize] += 1;
    }
    let pairs = counts.iter().filter(|&&n| n >= 2).count() as f64;
    let trips = if counts.iter().any(|&n| n >= 3) {
        1.0
    } else {
        0.0
    };
    let suited = suits.iter().filter(|&&n| n >= 2).count() as f64;
    let suited_ace = if hole
        .iter()
        .any(|&c| rank(c) == 12 && suits[suit(c) as usize] >= 2)
    {
        1.0
    } else {
        0.0
    };
    let heavy_suit = if suits.iter().any(|&n| n >= 4) {
        1.0
    } else {
        0.0
    };
    let mean_rank = hole.iter().map(|&c| rank(c) as f64).sum::<f64>() / (12.0 * k);
    let top = hole.iter().map(|&c| rank(c)).max().unwrap_or(0) as f64 / 12.0;
    let distinct = counts.iter().filter(|&&n| n > 0).count() as f64 / k;
    let ranks = rank_bits(hole);
    let window = (0..10)
        .map(|w| (ranks & 0b11111 << w).count_ones())
        .max()
        .unwrap_or(0) as f64
        / 5.0;
    [
        1.0,
        p,
        p * p,
        p.powi(4),
        pairs / 3.0,
        trips,
        suited / 3.0,
        suited_ace,
        heavy_suit,
        mean_rank,
        top,
        distinct,
        window,
        p * suited / 3.0,
    ]
}

/// A starting hand's features for nut-aware preflop models:
/// [`preflop_features`], then its suited aces (nut flush draws to be), kings
/// suited without the ace (second nuts), two or more suits held twice or
/// more, its top paired rank (top set), the most of its ranks in one
/// four-rank window (a tight rundown), and its danglers (cards with no other
/// within three ranks).
pub fn preflop_nut_features(hole: &[Card]) -> [f64; NUM_PREFLOP_NUT_FEATURES] {
    let base = preflop_features(hole);
    let k = hole.len() as f64;
    let mut suits = [0u8; 4];
    let mut counts = [0u8; 13];
    for &c in hole {
        suits[suit(c) as usize] += 1;
        counts[rank(c) as usize] += 1;
    }
    let held = |r: u8, s: u8| hole.iter().any(|&c| rank(c) == r && suit(c) == s);
    let suited_aces = (0..4u8)
        .filter(|&s| suits[s as usize] >= 2 && held(12, s))
        .count() as f64;
    let suited_kings = (0..4u8)
        .filter(|&s| suits[s as usize] >= 2 && held(11, s) && !held(12, s))
        .count() as f64;
    let double_suited = if suits.iter().filter(|&&n| n >= 2).count() >= 2 {
        1.0
    } else {
        0.0
    };
    let top_pair = (0..13u8)
        .rev()
        .find(|&r| counts[r as usize] >= 2)
        .map_or(0.0, |r| (r + 1) as f64 / 13.0);
    let ranks = rank_bits(hole);
    let tight = (0..11)
        .map(|w| (ranks & 0b1111 << w).count_ones())
        .max()
        .unwrap_or(0) as f64
        / 4.0;
    let danglers = hole
        .iter()
        .filter(|&&c| {
            let r = rank(c) as i32;
            !hole.iter().any(|&o| {
                let d = (rank(o) as i32 - r).abs();
                o != c && (d <= 3 || d >= 9 && (r == 12 || rank(o) == 12))
            })
        })
        .count() as f64
        / k;
    let mut x = [0.0; NUM_PREFLOP_NUT_FEATURES];
    x[..NUM_FEATURES].copy_from_slice(&base);
    x[NUM_FEATURES..].copy_from_slice(&[
        suited_aces / 2.0,
        suited_kings / 2.0,
        double_suited,
        top_pair,
        tight,
        danglers,
    ]);
    x
}

/// How preflop hands are bucketed.
#[derive(Clone, Debug)]
enum Preflop {
    /// Four cards: the bucket of each suit-isomorphism class.
    Classes {
        classes: PreflopClasses,
        buckets: Vec<u16>,
    },
    /// Five or six cards: a model like the flop's, over
    /// [`preflop_features`].
    Model(StreetModel),
}

/// A street's model: equity weights (and nut-potential weights, for
/// nut-aware abstractions) and the cluster centres over (predicted equity,
/// [`NUT_WEIGHT`] × predicted nut potential if any, `STRENGTH_WEIGHT` ×
/// feature 1, made strength or playability).
#[derive(Clone, Debug, PartialEq)]
pub struct StreetModel {
    pub weights: Vec<f64>,
    /// Empty for abstractions that aren't nut-aware.
    pub nut_weights: Vec<f64>,
    pub centres: Vec<f32>,
}

/// How much made strength counts against predicted equity in clustering.
const STRENGTH_WEIGHT: f64 = 0.5;

fn dot(w: &[f64], x: &[f64]) -> f64 {
    w.iter().zip(x).map(|(a, b)| a * b).sum()
}

impl StreetModel {
    pub fn predict(&self, x: &[f64]) -> f64 {
        dot(&self.weights, x).clamp(0.0, 1.0)
    }

    /// Predicted nut potential (0 without a nut model).
    pub fn predict_nuts(&self, x: &[f64]) -> f64 {
        dot(&self.nut_weights, x).clamp(0.0, 1.0)
    }

    /// Values per cluster centre.
    pub fn dims(&self) -> usize {
        if self.nut_weights.is_empty() { 2 } else { 3 }
    }

    fn point(&self, x: &[f64]) -> Vec<f32> {
        let mut p = vec![self.predict(x) as f32];
        if !self.nut_weights.is_empty() {
            p.push((NUT_WEIGHT * self.predict_nuts(x)) as f32);
        }
        p.push((STRENGTH_WEIGHT * x[1]) as f32);
        p
    }

    pub fn bucket(&self, x: &[f64]) -> u16 {
        nearest(&self.point(x), &self.centres, self.dims(), Distance::L2) as u16
    }

    pub fn buckets(&self) -> usize {
        self.centres.len() / self.dims()
    }
}

/// A hand's features on a board, for a plain or nut-aware model.
enum Feats {
    Plain([f64; NUM_FEATURES]),
    Nut([f64; NUM_NUT_FEATURES]),
}

impl std::ops::Deref for Feats {
    type Target = [f64];

    fn deref(&self) -> &[f64] {
        match self {
            Feats::Plain(x) => x,
            Feats::Nut(x) => x,
        }
    }
}

/// The PLO card abstraction. See the module docs.
#[derive(Clone, Debug)]
pub struct PloAbstraction {
    pub config: PloAbstractionConfig,
    preflop: Preflop,
    flop: StreetModel,
    turn: StreetModel,
    /// River-equity cut-offs between river buckets (`river - 1` of them).
    river_cuts: Vec<f64>,
}

/// One sampled hand for fitting: its features, equity and nut potential.
struct FitHand {
    x: Vec<f64>,
    equity: f64,
    nuts: f64,
}

/// A random hand and board of `board_len` cards.
pub fn random_spot(k: usize, board_len: usize, rng: &mut Rng) -> (Vec<Card>, Vec<Card>) {
    let mut used = 0;
    let hole = (0..k).map(|_| draw(&mut used, rng)).collect();
    let board = (0..board_len).map(|_| draw(&mut used, rng)).collect();
    (hole, board)
}

fn map_parallel<T: Send, R: Send>(items: Vec<T>, f: impl Fn(T) -> R + Sync + Send) -> Vec<R> {
    #[cfg(feature = "parallel")]
    {
        use rayon::prelude::*;
        items.into_par_iter().map(f).collect()
    }
    #[cfg(not(feature = "parallel"))]
    {
        items.into_iter().map(f).collect()
    }
}

/// Ridge least squares: the weights minimising squared error plus a little
/// shrinkage, by Gaussian elimination on the normal equations.
fn least_squares(xs: &[Vec<f64>], ys: &[f64]) -> Vec<f64> {
    let n = xs.first().map_or(0, |x| x.len());
    let mut a = vec![vec![0.0; n + 1]; n];
    for (x, &y) in xs.iter().zip(ys) {
        for i in 0..n {
            for j in 0..n {
                a[i][j] += x[i] * x[j];
            }
            a[i][n] += x[i] * y;
        }
    }
    for (i, row) in a.iter_mut().enumerate() {
        row[i] += 1e-6 * xs.len() as f64;
    }
    for col in 0..n {
        let pivot = (col..n)
            .max_by(|&r, &s| a[r][col].abs().total_cmp(&a[s][col].abs()))
            .unwrap();
        a.swap(col, pivot);
        let d = a[col][col];
        if d.abs() < 1e-12 {
            continue;
        }
        for x in &mut a[col][col..] {
            *x /= d;
        }
        let pivot_row = a[col].clone();
        for (r, row) in a.iter_mut().enumerate() {
            if r != col {
                let f = row[col];
                for (x, p) in row[col..].iter_mut().zip(&pivot_row[col..]) {
                    *x -= f * p;
                }
            }
        }
    }
    a.iter().map(|row| row[n]).collect()
}

impl PloAbstraction {
    /// Builds the abstraction: preflop classes and their buckets, then a
    /// fitted model for each street. `progress` hears about each stage.
    pub fn build(config: PloAbstractionConfig, mut progress: impl FnMut(&str)) -> Self {
        let k = config.hole_cards;
        assert!((4..=6).contains(&k), "4 to 6 hole cards");
        assert!(
            k == 4 || config.preflop > 0,
            "one preflop bucket per class needs four hole cards"
        );
        let samples = config.equity_samples;
        let seed = config.seed;

        let nut_aware = config.nut_aware;
        let features = |hole: &[Card], board: &[Card]| -> Vec<f64> {
            match (board.len(), nut_aware) {
                (0, false) => preflop_features(hole).to_vec(),
                (0, true) => preflop_nut_features(hole).to_vec(),
                (_, false) => BoardView::for_hands(board, k).features(hole).to_vec(),
                (_, true) => BoardView::exact(board).nut_features(hole).to_vec(),
            }
        };

        // Sampled hands with their features, equity and nut potential, and
        // a fitted model.
        let fit = |board_len: usize, buckets: usize| -> StreetModel {
            let hands: Vec<FitHand> = map_parallel((0..config.fit_hands).collect(), |i| {
                let mut rng = Rng::new(
                    seed.wrapping_add(board_len as u64 * 1_000_003)
                        ^ (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15),
                );
                let (hole, board) = random_spot(k, board_len, &mut rng);
                FitHand {
                    x: features(&hole, &board),
                    equity: equity_vs_random(&hole, &board, samples, &mut rng).mean,
                    nuts: if nut_aware {
                        nut_potential(&hole, &board, NUT_RUNOUTS, &mut rng)
                    } else {
                        0.0
                    },
                }
            });
            let xs: Vec<Vec<f64>> = hands.iter().map(|h| h.x.clone()).collect();
            let ys: Vec<f64> = hands.iter().map(|h| h.equity).collect();
            let mut model = StreetModel {
                weights: least_squares(&xs, &ys),
                nut_weights: Vec::new(),
                centres: Vec::new(),
            };
            if nut_aware {
                let nuts: Vec<f64> = hands.iter().map(|h| h.nuts).collect();
                model.nut_weights = least_squares(&xs, &nuts);
            }
            let flat: Vec<f32> = xs.iter().flat_map(|x| model.point(x)).collect();
            let dims = model.dims();
            model.centres = kmeans(
                &flat,
                dims,
                buckets,
                50,
                seed + board_len as u64,
                Distance::L2,
            );
            model
        };

        let preflop = if k == 4 {
            // Equity (and nut potential) and ducy's playability percentile
            // per class.
            let classes = PreflopClasses::new(k);
            progress(&format!("{} preflop classes", classes.len()));
            let buckets: Vec<u16> = if config.preflop == 0 {
                (0..classes.len() as u16).collect()
            } else {
                let points: Vec<Vec<f32>> = map_parallel((0..classes.len()).collect(), |i| {
                    let hand = classes.representative(i);
                    let mut rng = Rng::new(seed ^ (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
                    let e = equity_vs_random(hand, &[], samples, &mut rng).mean;
                    let mut p = vec![e as f32];
                    if nut_aware {
                        let n = nut_potential(hand, &[], NUT_RUNOUTS, &mut rng);
                        p.push((NUT_WEIGHT * n) as f32);
                    }
                    p.push((STRENGTH_WEIGHT * playability(hand)) as f32);
                    p
                });
                let dims = points[0].len();
                let flat: Vec<f32> = points.iter().flatten().copied().collect();
                let centres = kmeans(&flat, dims, config.preflop, 50, seed, Distance::L2);
                points
                    .iter()
                    .map(|p| nearest(p, &centres, dims, Distance::L2) as u16)
                    .collect()
            };
            Preflop::Classes { classes, buckets }
        } else {
            Preflop::Model(fit(0, config.preflop))
        };
        progress("preflop buckets done");

        let flop = fit(3, config.flop);
        progress("flop model done");
        let turn = fit(4, config.turn);
        progress("turn model done");

        // River: equal-mass bins of equity against the board's sampled
        // opponents.
        let mut equities: Vec<f64> = map_parallel((0..config.fit_hands).collect(), |i| {
            let mut rng = Rng::new(
                seed.wrapping_add(5_000_015) ^ (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15),
            );
            let (hole, board) = random_spot(k, 5, &mut rng);
            BoardView::for_hands(&board, k).river_equity(&hole)
        });
        equities.sort_by(f64::total_cmp);
        let river_cuts: Vec<f64> = (1..config.river)
            .map(|b| equities[b * equities.len() / config.river])
            .collect();
        progress("river cut-offs done");

        Self {
            config,
            preflop,
            flop,
            turn,
            river_cuts,
        }
    }

    /// The preflop bucket of `hole`.
    pub fn preflop_bucket(&self, hole: &[Card]) -> u16 {
        match &self.preflop {
            Preflop::Classes { classes, buckets } => buckets[classes.class(hole)],
            Preflop::Model(m) => m.bucket(&self.preflop_features(hole)),
        }
    }

    /// A starting hand's features for this abstraction's preflop model.
    pub fn preflop_features(&self, hole: &[Card]) -> Vec<f64> {
        if self.config.nut_aware {
            preflop_nut_features(hole).to_vec()
        } else {
            preflop_features(hole).to_vec()
        }
    }

    /// The view of `board` that [`PloAbstraction::bucket_on`] needs: river
    /// opponents with this abstraction's number of hole cards.
    pub fn view(&self, board: &[Card]) -> BoardView {
        BoardView::for_hands(board, self.config.hole_cards)
    }

    fn features_on(&self, hole: &[Card], view: &BoardView) -> Feats {
        if self.config.nut_aware {
            Feats::Nut(view.nut_features(hole))
        } else {
            Feats::Plain(view.features(hole))
        }
    }

    /// The bucket of `hole` on a board that `view` describes.
    pub fn bucket_on(&self, hole: &[Card], view: &BoardView) -> u16 {
        match view.board.len() {
            3 => self.flop.bucket(&self.features_on(hole, view)),
            4 => self.turn.bucket(&self.features_on(hole, view)),
            _ => {
                let e = view.river_equity(hole);
                self.river_cuts.partition_point(|&c| c <= e) as u16
            }
        }
    }

    /// The equity the buckets use: predicted on the flop and turn, the river
    /// value on the river.
    pub fn predicted_equity(&self, hole: &[Card], view: &BoardView) -> f64 {
        match view.board.len() {
            3 => self.flop.predict(&self.features_on(hole, view)),
            4 => self.turn.predict(&self.features_on(hole, view)),
            _ => view.river_equity(hole),
        }
    }

    /// Predicted nut potential on the flop and turn (0 without a nut model),
    /// [`nutness`] of the made hand on the river.
    pub fn predicted_nuts(&self, hole: &[Card], view: &BoardView) -> f64 {
        match view.board.len() {
            3 => self.flop.predict_nuts(&self.features_on(hole, view)),
            4 => self.turn.predict_nuts(&self.features_on(hole, view)),
            _ => nutness(1.0 - view.strength(hole)),
        }
    }

    pub fn street_model(&self, board_len: usize) -> Option<&StreetModel> {
        match board_len {
            0 => match &self.preflop {
                Preflop::Model(m) => Some(m),
                Preflop::Classes { .. } => None,
            },
            3 => Some(&self.flop),
            4 => Some(&self.turn),
            _ => None,
        }
    }

    /// The saved form: version 1 (four cards) has the preflop table after
    /// the settings, version 2 (five or six cards) a third model before the
    /// flop's. Version 3 (nut-aware, any hole count) says which after the
    /// settings (1 for a table), and each model has its nut weights after
    /// its equity weights.
    pub fn save(&self) -> Vec<u8> {
        let c = &self.config;
        let mut out = match (&self.preflop, c.nut_aware) {
            (_, true) => MAGIC_NUT,
            (Preflop::Classes { .. }, false) => MAGIC,
            (Preflop::Model(_), false) => MAGIC_MODEL,
        }
        .to_vec();
        for v in [
            c.hole_cards,
            c.preflop,
            c.flop,
            c.turn,
            c.river,
            c.fit_hands,
            c.equity_samples,
            c.seed as usize,
        ] {
            out.extend((v as u64).to_le_bytes());
        }
        if c.nut_aware {
            let table = matches!(self.preflop, Preflop::Classes { .. });
            out.extend((table as u64).to_le_bytes());
        }
        let mut models = vec![&self.flop, &self.turn];
        match &self.preflop {
            Preflop::Classes { buckets, .. } => {
                out.extend((buckets.len() as u64).to_le_bytes());
                for &b in buckets {
                    out.extend(b.to_le_bytes());
                }
            }
            Preflop::Model(m) => models.insert(0, m),
        }
        for m in models {
            out.extend((m.weights.len() as u64).to_le_bytes());
            for w in &m.weights {
                out.extend(w.to_le_bytes());
            }
            if c.nut_aware {
                out.extend((m.nut_weights.len() as u64).to_le_bytes());
                for w in &m.nut_weights {
                    out.extend(w.to_le_bytes());
                }
            }
            out.extend((m.centres.len() as u64).to_le_bytes());
            for x in &m.centres {
                out.extend(x.to_le_bytes());
            }
        }
        out.extend((self.river_cuts.len() as u64).to_le_bytes());
        for x in &self.river_cuts {
            out.extend(x.to_le_bytes());
        }
        out
    }

    pub fn load(bytes: &[u8]) -> Option<Self> {
        use crate::key::{read_u64, take};
        let mut input = bytes;
        let input = &mut input;
        let magic = take(input, MAGIC.len())?;
        let nut_aware = magic == MAGIC_NUT;
        let mut with_table = if magic == MAGIC {
            true
        } else if magic == MAGIC_MODEL || nut_aware {
            false
        } else {
            return None;
        };
        let mut num = || read_u64(input).map(|v| v as usize);
        let config = PloAbstractionConfig {
            hole_cards: num()?,
            preflop: num()?,
            flop: num()?,
            turn: num()?,
            river: num()?,
            fit_hands: num()?,
            equity_samples: num()?,
            seed: num()? as u64,
            nut_aware,
        };
        if nut_aware {
            with_table = match read_u64(input)? {
                0 => false,
                1 => true,
                _ => return None,
            };
        }
        // Four cards have the table (or, nut-aware, may have the model);
        // five and six, the model.
        let table_ok = if nut_aware {
            !with_table || config.hole_cards == 4
        } else {
            with_table == (config.hole_cards == 4)
        };
        if !table_ok || !(4..=6).contains(&config.hole_cards) {
            return None;
        }
        let f64s = |input: &mut &[u8], n: usize| -> Option<Vec<f64>> {
            let b = take(input, n * 8)?;
            Some(
                b.chunks_exact(8)
                    .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
                    .collect(),
            )
        };
        let table = if with_table {
            let n = read_u64(input)? as usize;
            let classes = PreflopClasses::new(config.hole_cards);
            if n != classes.len() {
                return None;
            }
            let b = take(input, n * 2)?;
            let buckets = b
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            Some(Preflop::Classes { classes, buckets })
        } else {
            None
        };
        let mut models = Vec::new();
        for _ in 0..if with_table { 2 } else { 3 } {
            let w = read_u64(input)? as usize;
            let weights = f64s(input, w)?;
            let nut_weights = if nut_aware {
                let w = read_u64(input)? as usize;
                f64s(input, w)?
            } else {
                Vec::new()
            };
            let c = read_u64(input)? as usize;
            let b = take(input, c * 4)?;
            let centres = b
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                .collect();
            models.push(StreetModel {
                weights,
                nut_weights,
                centres,
            });
        }
        let r = read_u64(input)? as usize;
        let river_cuts = f64s(input, r)?;
        if !input.is_empty() || models_bad(&models, with_table, nut_aware) {
            return None;
        }
        let turn = models.pop()?;
        let flop = models.pop()?;
        let preflop = match table {
            Some(t) => t,
            None => Preflop::Model(models.pop()?),
        };
        Some(Self {
            config,
            preflop,
            flop,
            turn,
            river_cuts,
        })
    }
}

const MAGIC: &[u8] = b"DUCYPLOA\x01";
const MAGIC_MODEL: &[u8] = b"DUCYPLOA\x02";
const MAGIC_NUT: &[u8] = b"DUCYPLOA\x03";

/// Saved models (the preflop one first, if any, then the flop's and the
/// turn's) with the wrong number of weights or no whole set of centres (a
/// file from a different feature set).
fn models_bad(models: &[StreetModel], with_table: bool, nut_aware: bool) -> bool {
    models.iter().enumerate().any(|(i, m)| {
        let preflop = !with_table && i == 0;
        let n = match (nut_aware, preflop) {
            (false, _) => NUM_FEATURES,
            (true, true) => NUM_PREFLOP_NUT_FEATURES,
            (true, false) => NUM_NUT_FEATURES,
        };
        m.weights.len() != n
            || nut_aware && m.nut_weights.len() != n
            || m.centres.is_empty()
            || m.centres.len() % m.dims() != 0
    })
}

/// ducy's playability for an Omaha starting hand, 1 the best and 0 the
/// worst (its percentile, flipped).
pub fn playability(hole: &[Card]) -> f64 {
    let cards: Vec<ducy::deck::Card> = hole
        .iter()
        .map(|&c| ducy::deck::Card::parse(&to_string(c)).expect("a card"))
        .collect();
    ducy::games::omaha_analysis::percentile(&cards).map_or(0.5, |p| 1.0 - p / 100.0)
}

impl Buckets for PloAbstraction {
    fn hand_bucket(&self, hole: &[Card], board: &[Card]) -> u16 {
        if board.is_empty() {
            self.preflop_bucket(hole)
        } else {
            self.bucket_on(hole, &self.view(board))
        }
    }

    fn bucket_count(&self, board_len: usize) -> usize {
        match board_len {
            0 => match &self.preflop {
                Preflop::Classes { classes, .. } if self.config.preflop == 0 => classes.len(),
                Preflop::Classes { .. } => self.config.preflop,
                Preflop::Model(m) => m.buckets(),
            },
            3 => self.flop.buckets(),
            4 => self.turn.buckets(),
            _ => self.river_cuts.len() + 1,
        }
    }

    /// The settings as blueprints hash them. Abstractions that aren't
    /// nut-aware print as they did before `nut_aware` existed, so their
    /// published blueprints still load.
    fn settings(&self) -> String {
        let c = &self.config;
        let text = format!(
            "PloAbstractionConfig {{ hole_cards: {}, preflop: {}, flop: {}, turn: {}, river: {}, \
             fit_hands: {}, equity_samples: {}, seed: {} }}",
            c.hole_cards, c.preflop, c.flop, c.turn, c.river, c.fit_hands, c.equity_samples, c.seed
        );
        if c.nut_aware {
            format!("{text}|plo-features-v2")
        } else {
            format!("{text}|plo-features-v1")
        }
    }

    fn deal_buckets(&self, hole: [&[Card]; 2], board: &[Card; 5]) -> [[u16; 4]; 2] {
        let mut out = [[0u16; 4]; 2];
        for (p, h) in hole.iter().enumerate() {
            out[p][0] = self.preflop_bucket(h);
        }
        for (street, n) in [(1, 3), (2, 4), (3, 5)] {
            let view = self.view(&board[..n]);
            for (p, h) in hole.iter().enumerate() {
                out[p][street] = self.bucket_on(h, &view);
            }
        }
        out
    }
}
