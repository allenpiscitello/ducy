//! A card abstraction for heads-up Omaha, computed from features (#128).
//!
//! Hold'em's abstraction looks every flop and turn hand up in a table. Omaha
//! has about 10⁸ flop hands and 10¹⁰ turn hands, and sampling equity during
//! training is far too slow (see `plo_bench`), so buckets come from cheap,
//! exact features of the hand on the board:
//!
//! - **Preflop:** hands are grouped by suit isomorphism (16,432 classes for
//!   four cards). Each class's equity against a random hand and ducy's
//!   playability percentile (`omaha_analysis`) are clustered with k-means;
//!   `preflop: 0` keeps every class as its own bucket instead.
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
//! few kilobytes plus a 33 KB preflop table: small enough to ship.

use super::showdown::{draw, equity_vs_random};
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
    /// Hole cards (4 for PLO).
    pub hole_cards: usize,
    /// Preflop buckets, or 0 for one per suit-isomorphism class.
    pub preflop: usize,
    pub flop: usize,
    pub turn: usize,
    pub river: usize,
    /// Sampled hands per street for fitting.
    pub fit_hands: usize,
    /// Equity samples per fitted hand (and per preflop class).
    pub equity_samples: usize,
    pub seed: u64,
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
        }
    }
}

/// Features per hand for the equity model.
pub const NUM_FEATURES: usize = 14;

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
/// error) for a hand near 50%.
pub const RIVER_OPPONENTS: usize = 512;

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
    /// The view of a 3-, 4- or 5-card board.
    pub fn new(board: &[Card]) -> Self {
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
        if n == 5 {
            let mut rng = Rng::new(used.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
            let mut hand = [0 as Card; 4];
            view.opponents = (0..RIVER_OPPONENTS)
                .map(|_| {
                    let mut taken = used;
                    for c in hand.iter_mut() {
                        *c = draw(&mut taken, &mut rng);
                    }
                    (view.best(&hand), mask(&hand))
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

/// A flop or turn model: equity weights and the cluster centres over
/// (predicted equity, `STRENGTH_WEIGHT` × made strength).
#[derive(Clone, Debug, PartialEq)]
pub struct StreetModel {
    pub weights: Vec<f64>,
    pub centres: Vec<f32>,
}

/// How much made strength counts against predicted equity in clustering.
const STRENGTH_WEIGHT: f64 = 0.5;

fn dot(w: &[f64], x: &[f64; NUM_FEATURES]) -> f64 {
    w.iter().zip(x).map(|(a, b)| a * b).sum()
}

impl StreetModel {
    pub fn predict(&self, x: &[f64; NUM_FEATURES]) -> f64 {
        dot(&self.weights, x).clamp(0.0, 1.0)
    }

    fn point(&self, x: &[f64; NUM_FEATURES]) -> [f32; 2] {
        [self.predict(x) as f32, (STRENGTH_WEIGHT * x[1]) as f32]
    }

    pub fn bucket(&self, x: &[f64; NUM_FEATURES]) -> u16 {
        nearest(&self.point(x), &self.centres, 2, Distance::L2) as u16
    }
}

/// The PLO card abstraction. See the module docs.
#[derive(Clone, Debug)]
pub struct PloAbstraction {
    pub config: PloAbstractionConfig,
    classes: PreflopClasses,
    /// Bucket of each preflop class.
    preflop: Vec<u16>,
    flop: StreetModel,
    turn: StreetModel,
    /// River-equity cut-offs between river buckets (`river - 1` of them).
    river_cuts: Vec<f64>,
}

/// One sampled flop or turn hand for fitting: its features and equity.
struct FitHand {
    x: [f64; NUM_FEATURES],
    equity: f64,
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
fn least_squares(xs: &[[f64; NUM_FEATURES]], ys: &[f64]) -> Vec<f64> {
    let n = NUM_FEATURES;
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
        let classes = PreflopClasses::new(k);
        progress(&format!("{} preflop classes", classes.len()));

        // Preflop: equity and ducy's playability percentile per class.
        let samples = config.equity_samples;
        let seed = config.seed;
        let preflop: Vec<u16> = if config.preflop == 0 {
            (0..classes.len() as u16).collect()
        } else {
            let points: Vec<[f32; 2]> = map_parallel((0..classes.len()).collect(), |i| {
                let hand = classes.representative(i);
                let mut rng = Rng::new(seed ^ (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
                let e = equity_vs_random(hand, &[], samples, &mut rng).mean;
                [e as f32, (STRENGTH_WEIGHT * playability(hand)) as f32]
            });
            let flat: Vec<f32> = points.iter().flatten().copied().collect();
            let centres = kmeans(&flat, 2, config.preflop, 50, seed, Distance::L2);
            points
                .iter()
                .map(|p| nearest(p, &centres, 2, Distance::L2) as u16)
                .collect()
        };
        progress("preflop buckets done");

        // Sampled hands with their features and equity, and a fitted model.
        let sample = |board_len: usize| -> (Vec<[f64; NUM_FEATURES]>, Vec<f64>, Vec<f64>) {
            let hands: Vec<FitHand> = map_parallel((0..config.fit_hands).collect(), |i| {
                let mut rng = Rng::new(
                    seed.wrapping_add(board_len as u64 * 1_000_003)
                        ^ (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15),
                );
                let (hole, board) = random_spot(k, board_len, &mut rng);
                let view = BoardView::new(&board);
                FitHand {
                    x: view.features(&hole),
                    equity: equity_vs_random(&hole, &board, samples, &mut rng).mean,
                }
            });
            let xs: Vec<[f64; NUM_FEATURES]> = hands.iter().map(|h| h.x).collect();
            let ys: Vec<f64> = hands.iter().map(|h| h.equity).collect();
            let weights = least_squares(&xs, &ys);
            (xs, ys, weights)
        };
        let fit = |board_len: usize, buckets: usize| -> StreetModel {
            let (xs, _, weights) = sample(board_len);
            let mut model = StreetModel {
                weights,
                centres: Vec::new(),
            };
            let flat: Vec<f32> = xs.iter().flat_map(|x| model.point(x)).collect();
            model.centres = kmeans(&flat, 2, buckets, 50, seed + board_len as u64, Distance::L2);
            model
        };
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
            BoardView::new(&board).river_equity(&hole)
        });
        equities.sort_by(f64::total_cmp);
        let river_cuts: Vec<f64> = (1..config.river)
            .map(|b| equities[b * equities.len() / config.river])
            .collect();
        progress("river cut-offs done");

        Self {
            config,
            classes,
            preflop,
            flop,
            turn,
            river_cuts,
        }
    }

    /// The preflop bucket of `hole`.
    pub fn preflop_bucket(&self, hole: &[Card]) -> u16 {
        self.preflop[self.classes.class(hole)]
    }

    /// The bucket of `hole` on a board that `view` describes.
    pub fn bucket_on(&self, hole: &[Card], view: &BoardView) -> u16 {
        match view.board.len() {
            3 => self.flop.bucket(&view.features(hole)),
            4 => self.turn.bucket(&view.features(hole)),
            _ => {
                let e = view.river_equity(hole);
                self.river_cuts.partition_point(|&c| c <= e) as u16
            }
        }
    }

    /// The equity the buckets use: predicted on the flop and turn, against
    /// the sampled opponents on the river.
    pub fn predicted_equity(&self, hole: &[Card], view: &BoardView) -> f64 {
        match view.board.len() {
            3 => self.flop.predict(&view.features(hole)),
            4 => self.turn.predict(&view.features(hole)),
            _ => view.river_equity(hole),
        }
    }

    pub fn street_model(&self, board_len: usize) -> Option<&StreetModel> {
        match board_len {
            3 => Some(&self.flop),
            4 => Some(&self.turn),
            _ => None,
        }
    }

    pub fn save(&self) -> Vec<u8> {
        let c = &self.config;
        let mut out = MAGIC.to_vec();
        for v in [
            c.hole_cards,
            c.preflop,
            c.flop,
            c.turn,
            c.river,
            c.fit_hands,
            c.equity_samples,
            c.seed as usize,
            self.preflop.len(),
        ] {
            out.extend((v as u64).to_le_bytes());
        }
        for &b in &self.preflop {
            out.extend(b.to_le_bytes());
        }
        for m in [&self.flop, &self.turn] {
            out.extend((m.weights.len() as u64).to_le_bytes());
            for w in &m.weights {
                out.extend(w.to_le_bytes());
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
        if take(input, MAGIC.len())? != MAGIC {
            return None;
        }
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
        };
        let n = num()?;
        let classes = PreflopClasses::new(config.hole_cards);
        if n != classes.len() {
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
        let b = take(input, n * 2)?;
        let preflop = b
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let mut models = Vec::new();
        for _ in 0..2 {
            let w = read_u64(input)? as usize;
            let weights = f64s(input, w)?;
            let c = read_u64(input)? as usize;
            let b = take(input, c * 4)?;
            let centres = b
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                .collect();
            models.push(StreetModel { weights, centres });
        }
        let r = read_u64(input)? as usize;
        let river_cuts = f64s(input, r)?;
        if !input.is_empty() || flop_turn_bad(&models) {
            return None;
        }
        let turn = models.pop()?;
        let flop = models.pop()?;
        Some(Self {
            config,
            classes,
            preflop,
            flop,
            turn,
            river_cuts,
        })
    }
}

const MAGIC: &[u8] = b"DUCYPLOA\x01";

/// A saved flop or turn model with the wrong number of weights, or no
/// centres (a file from a different feature set).
fn flop_turn_bad(models: &[StreetModel]) -> bool {
    models
        .iter()
        .any(|m| m.weights.len() != NUM_FEATURES || m.centres.is_empty())
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
            self.bucket_on(hole, &BoardView::new(board))
        }
    }

    fn bucket_count(&self, board_len: usize) -> usize {
        match board_len {
            0 => {
                if self.config.preflop == 0 {
                    self.classes.len()
                } else {
                    self.config.preflop
                }
            }
            3 => self.flop.centres.len() / 2,
            4 => self.turn.centres.len() / 2,
            _ => self.river_cuts.len() + 1,
        }
    }

    fn settings(&self) -> String {
        format!("{:?}|plo-features-v1", self.config)
    }

    fn deal_buckets(&self, hole: [&[Card]; 2], board: &[Card; 5]) -> [[u16; 4]; 2] {
        let mut out = [[0u16; 4]; 2];
        for (p, h) in hole.iter().enumerate() {
            out[p][0] = self.preflop_bucket(h);
        }
        for (street, n) in [(1, 3), (2, 4), (3, 5)] {
            let view = BoardView::new(&board[..n]);
            for (p, h) in hole.iter().enumerate() {
                out[p][street] = self.bucket_on(h, &view);
            }
        }
        out
    }
}
