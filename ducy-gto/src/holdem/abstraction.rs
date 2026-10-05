//! Card abstraction for heads-up Hold'em: maps hole cards and a board to a
//! small bucket number per street, so a strategy can be stored per bucket
//! rather than per hand.
//!
//! - **Preflop:** lossless, the 169 distinct starting hands.
//! - **Flop and turn:** each canonical hand (up to suit isomorphism) gets a
//!   histogram of its equity on the river, over every runout. Hands are
//!   clustered with k-means under earth mover's distance, which groups hands
//!   by how their strength can develop, not just their average strength. So
//!   a flush draw and a weak made hand with the same equity end up apart.
//! - **River:** equity against a random hand, split into equal-population
//!   buckets.
//!
//! Building runs one pass over every runout of every distinct flop (1,755
//! flops, 2.06 million river boards) to sample features and fit the
//! clusters, then a second pass to assign every canonical flop and turn hand
//! (1,286,792 and 55,190,538 of them) to a bucket. River buckets are computed
//! on demand from the hand's equity. With the default config this takes
//! about 4.6 minutes on 4 cores and saves to about 565 MB.

use std::collections::{BTreeMap, HashSet};

use super::{
    cards::{Card, NUM_CARDS, NUM_HOLES, bit, hole_cards, hole_index, mask, score},
    equity::{river_equities, river_equity, strengths},
    iso::{NUM_PREFLOP_CLASSES, canonical, canonical_board, preflop_class},
    kmeans::{Distance, kmeans, nearest},
};
use crate::rng::Rng;

/// How finely to abstract each street.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbstractionConfig {
    pub flop_buckets: usize,
    pub turn_buckets: usize,
    pub river_buckets: usize,
    /// Histogram bins for flop and turn equity distributions.
    pub flop_bins: usize,
    pub turn_bins: usize,
    /// Roughly how many hands per street to fit the clusters on.
    pub sample: usize,
    pub kmeans_iters: usize,
    pub seed: u64,
}

impl Default for AbstractionConfig {
    /// 169 / 200 / 200 / 200 buckets.
    fn default() -> Self {
        Self {
            flop_buckets: 200,
            turn_buckets: 200,
            river_buckets: 200,
            flop_bins: 50,
            turn_bins: 30,
            sample: 200_000,
            kmeans_iters: 40,
            seed: 1,
        }
    }
}

/// Canonical hand keys (sorted) and their buckets.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct KeyTable {
    keys: Vec<u64>,
    buckets: Vec<u16>,
}

impl KeyTable {
    fn from_pairs(mut pairs: Vec<(u64, u16)>) -> Self {
        pairs.sort_unstable_by_key(|p| p.0);
        pairs.dedup_by_key(|p| p.0);
        Self {
            keys: pairs.iter().map(|p| p.0).collect(),
            buckets: pairs.iter().map(|p| p.1).collect(),
        }
    }

    fn get(&self, key: u64) -> Option<u16> {
        self.keys.binary_search(&key).ok().map(|i| self.buckets[i])
    }
}

/// A built card abstraction.
#[derive(Clone, Debug, PartialEq)]
pub struct CardAbstraction {
    pub config: AbstractionConfig,
    flop: KeyTable,
    turn: KeyTable,
    /// Upper equity bound of each river bucket but the last.
    river_edges: Vec<f32>,
    /// Cluster centres (equity histograms), so buckets can be computed
    /// without the tables. Empty for abstractions saved before they were
    /// kept.
    flop_centroids: Vec<f32>,
    turn_centroids: Vec<f32>,
}

/// One distinct flop and how many of the 22,100 flops it stands for.
fn distinct_flops() -> Vec<([Card; 3], u32)> {
    let mut seen: BTreeMap<u64, ([Card; 3], u32)> = BTreeMap::new();
    for a in 0..NUM_CARDS as Card {
        for b in a + 1..NUM_CARDS as Card {
            for c in b + 1..NUM_CARDS as Card {
                let (k, _) = canonical_board(&[a, b, c]);
                seen.entry(k).or_insert(([a, b, c], 0)).1 += 1;
            }
        }
    }
    seen.into_values().collect()
}

/// Equity histograms for every hand on one flop: on the flop itself (over all
/// turn and river cards) and after each turn card (over the rivers).
struct FlopPass {
    flop_bins: usize,
    turn_bins: usize,
    /// `[hole][bin]`
    flop_hist: Vec<f32>,
    /// `[turn card][hole][bin]`
    turn_hist: Vec<f32>,
    /// A sample of river equities, for the river bucket edges.
    river: Vec<f32>,
}

impl FlopPass {
    fn run(flop: [Card; 3], flop_bins: usize, turn_bins: usize, river_stride: usize) -> Self {
        let mut p = Self {
            flop_bins,
            turn_bins,
            flop_hist: vec![0.0; NUM_HOLES * flop_bins],
            turn_hist: vec![0.0; NUM_CARDS * NUM_HOLES * turn_bins],
            river: Vec::new(),
        };
        let fmask = mask(&flop);
        let rest: Vec<Card> = (0..NUM_CARDS as Card)
            .filter(|&c| fmask & bit(c) == 0)
            .collect();
        let mut eq = [0.0f32; NUM_HOLES];
        let mut n = 0usize;
        for (i, &t) in rest.iter().enumerate() {
            for &r in &rest[i + 1..] {
                river_equities(&[flop[0], flop[1], flop[2], t, r], &mut eq);
                for (h, &e) in eq.iter().enumerate() {
                    if e < 0.0 {
                        continue;
                    }
                    let fb = bin(e, flop_bins);
                    p.flop_hist[h * flop_bins + fb] += 1.0;
                    let tb = bin(e, turn_bins);
                    // River r after turn t, and river t after turn r.
                    p.turn_hist[(t as usize * NUM_HOLES + h) * turn_bins + tb] += 1.0;
                    p.turn_hist[(r as usize * NUM_HOLES + h) * turn_bins + tb] += 1.0;
                    n += 1;
                    if n % river_stride == 0 {
                        p.river.push(e);
                    }
                }
            }
        }
        p
    }

    /// Calls `f(key, histogram)` once per canonical flop hand on this flop,
    /// then `g` once per canonical turn hand, with normalized histograms.
    fn emit(
        &self,
        flop: [Card; 3],
        mut f: impl FnMut(u64, &[f32]),
        mut g: impl FnMut(u64, &[f32]),
    ) {
        let fmask = mask(&flop);
        let mut seen = HashSet::new();
        let mut buf = vec![0.0f32; self.flop_bins.max(self.turn_bins)];
        for h in 0..NUM_HOLES {
            let (a, b) = hole_cards(h);
            if fmask & (bit(a) | bit(b)) != 0 {
                continue;
            }
            let k = canonical(&[a, b, flop[0], flop[1], flop[2]]);
            if seen.insert(k) {
                normalize(
                    &self.flop_hist[h * self.flop_bins..(h + 1) * self.flop_bins],
                    &mut buf[..self.flop_bins],
                );
                f(k, &buf[..self.flop_bins]);
            }
        }
        seen.clear();
        for t in 0..NUM_CARDS as Card {
            if fmask & bit(t) != 0 {
                continue;
            }
            for h in 0..NUM_HOLES {
                let (a, b) = hole_cards(h);
                if (fmask | bit(t)) & (bit(a) | bit(b)) != 0 {
                    continue;
                }
                let k = canonical(&[a, b, flop[0], flop[1], flop[2], t]);
                if seen.insert(k) {
                    let at = (t as usize * NUM_HOLES + h) * self.turn_bins;
                    normalize(
                        &self.turn_hist[at..at + self.turn_bins],
                        &mut buf[..self.turn_bins],
                    );
                    g(k, &buf[..self.turn_bins]);
                }
            }
        }
    }
}

fn bin(e: f32, bins: usize) -> usize {
    ((e * bins as f32) as usize).min(bins - 1)
}

fn normalize(h: &[f32], out: &mut [f32]) {
    let total: f32 = h.iter().sum();
    for (o, &x) in out.iter_mut().zip(h) {
        *o = if total > 0.0 { x / total } else { 0.0 };
    }
}

/// Keeps a deterministic pseudo-random share of keys.
fn keep(key: u64, seed: u64, share: f64) -> bool {
    Rng::for_iteration(seed, key).next_f64() < share
}

#[cfg(feature = "parallel")]
fn map_flops<T: Send>(
    flops: &[([Card; 3], u32)],
    f: impl Fn(&([Card; 3], u32)) -> T + Sync + Send,
) -> Vec<T> {
    use rayon::prelude::*;
    flops.par_iter().map(f).collect()
}

#[cfg(not(feature = "parallel"))]
fn map_flops<T>(flops: &[([Card; 3], u32)], f: impl Fn(&([Card; 3], u32)) -> T) -> Vec<T> {
    flops.iter().map(f).collect()
}

/// Canonical hands per street, for sizing the sample.
const FLOP_HANDS: f64 = 1_286_792.0;
const TURN_HANDS: f64 = 55_190_538.0;

impl CardAbstraction {
    /// Builds the abstraction (about 4.6 minutes on 4 cores with the default
    /// config). `progress` is called with a short message as each stage
    /// starts.
    pub fn build(config: AbstractionConfig, mut progress: impl FnMut(&str)) -> Self {
        let flops = distinct_flops();
        let (fb, tb) = (config.flop_bins, config.turn_bins);
        let flop_share = (config.sample as f64 / FLOP_HANDS).min(1.0);
        let turn_share = (config.sample as f64 / TURN_HANDS).min(1.0);
        let stride = ((2_062_800.0 * 1081.0) / (config.sample.max(1) as f64)).max(1.0) as usize;

        progress("sampling equity histograms");
        let samples = map_flops(&flops, |&(flop, _)| {
            let pass = FlopPass::run(flop, fb, tb, stride);
            let (mut f, mut t) = (Vec::new(), Vec::new());
            pass.emit(
                flop,
                |k, h| {
                    if keep(k, config.seed, flop_share) {
                        f.extend_from_slice(h);
                    }
                },
                |k, h| {
                    if keep(k, config.seed ^ 1, turn_share) {
                        t.extend_from_slice(h);
                    }
                },
            );
            (f, t, pass.river)
        });
        let mut flop_points = Vec::new();
        let mut turn_points = Vec::new();
        let mut river = Vec::new();
        for (f, t, r) in samples {
            flop_points.extend(f);
            turn_points.extend(t);
            river.extend(r);
        }

        progress("clustering flop hands");
        let flop_centroids = kmeans(
            &flop_points,
            fb,
            config.flop_buckets,
            config.kmeans_iters,
            config.seed,
            Distance::Emd,
        );
        progress("clustering turn hands");
        let turn_centroids = kmeans(
            &turn_points,
            tb,
            config.turn_buckets,
            config.kmeans_iters,
            config.seed + 1,
            Distance::Emd,
        );
        river.sort_unstable_by(f32::total_cmp);
        let river_edges = (1..config.river_buckets)
            .map(|i| river[i * river.len() / config.river_buckets])
            .collect();

        progress("assigning every flop and turn hand");
        let assigned = map_flops(&flops, |&(flop, _)| {
            let pass = FlopPass::run(flop, fb, tb, usize::MAX);
            let (mut f, mut t) = (Vec::new(), Vec::new());
            pass.emit(
                flop,
                |k, h| f.push((k, nearest(h, &flop_centroids, fb, Distance::Emd) as u16)),
                |k, h| t.push((k, nearest(h, &turn_centroids, tb, Distance::Emd) as u16)),
            );
            (f, t)
        });
        let mut flop = Vec::new();
        let mut turn = Vec::new();
        for (f, t) in assigned {
            flop.extend(f);
            turn.extend(t);
        }
        progress("sorting");
        Self {
            config,
            flop: KeyTable::from_pairs(flop),
            turn: KeyTable::from_pairs(turn),
            river_edges,
            flop_centroids,
            turn_centroids,
        }
    }

    /// Number of buckets on a street with `board_len` board cards.
    pub fn num_buckets(&self, board_len: usize) -> usize {
        match board_len {
            0 => NUM_PREFLOP_CLASSES,
            3 => self.config.flop_buckets,
            4 => self.config.turn_buckets,
            _ => self.config.river_buckets,
        }
    }

    /// The bucket of `hole` with `board` (0, 3, 4 or 5 cards in deal order).
    pub fn bucket(&self, hole: [Card; 2], board: &[Card]) -> u16 {
        match board.len() {
            0 => preflop_class(hole[0], hole[1]) as u16,
            3 | 4 => {
                let mut cards = vec![hole[0], hole[1]];
                cards.extend_from_slice(board);
                let table = if board.len() == 3 {
                    &self.flop
                } else {
                    &self.turn
                };
                if table.keys.is_empty() {
                    if self.flop_centroids.is_empty() {
                        // A quick abstraction: hand strength right now.
                        return bucket_of(
                            hand_strength(hole, board),
                            self.num_buckets(board.len()),
                        );
                    }
                    return self.bucket_on_the_fly(hole, board);
                }
                table
                    .get(canonical(&cards))
                    .expect("every canonical hand is in the table")
            }
            5 => {
                let b: [Card; 5] = board.try_into().expect("five cards");
                self.river_bucket(river_equity(hole, &b))
            }
            n => panic!("a board has 0, 3, 4 or 5 cards, not {n}"),
        }
    }

    /// Every hand's bucket with `board`, indexed by
    /// [`hole_index`](super::cards::hole_index); `u16::MAX` for hands that
    /// use a board card. The same buckets as [`bucket`](Self::bucket), much
    /// faster for all 1,326 hands: on the river and with the quick
    /// abstraction every hand is scored once and the work shared, and a
    /// compact abstraction builds every hand's histogram from one pass over
    /// the runouts (about 0.2 s on the flop, 10 ms on the turn).
    pub fn buckets(&self, board: &[Card]) -> Vec<u16> {
        let bm = mask(board);
        let quick = board.len() < 5 && self.flop_centroids.is_empty() && {
            let table = if board.len() == 3 {
                &self.flop
            } else {
                &self.turn
            };
            table.keys.is_empty()
        };
        if board.len() == 5 || (board.len() >= 3 && quick) {
            let mut eq = [0f32; NUM_HOLES];
            strengths(board, &mut eq);
            return eq
                .iter()
                .map(|&e| match e < 0.0 {
                    true => u16::MAX,
                    false if board.len() == 5 => self.river_bucket(e),
                    false => bucket_of(e, self.num_buckets(board.len())),
                })
                .collect();
        }
        if matches!(board.len(), 3 | 4) && !self.has_tables() {
            return self.buckets_on_the_fly(board);
        }
        (0..NUM_HOLES)
            .map(|h| {
                let (a, b) = hole_cards(h);
                if bm & (bit(a) | bit(b)) != 0 {
                    u16::MAX
                } else {
                    self.bucket([a, b], board)
                }
            })
            .collect()
    }

    /// [`bucket_on_the_fly`](Self::bucket_on_the_fly) for every hand at
    /// once: each runout scores all hands in one [`river_equities`] call, so
    /// a whole flop takes about 0.2 s on one core instead of 1,081 times
    /// 13 ms (spread over the cores with the `parallel` feature).
    fn buckets_on_the_fly(&self, board: &[Card]) -> Vec<u16> {
        let flop = board.len() == 3;
        let (bins, centroids) = if flop {
            (self.config.flop_bins, &self.flop_centroids)
        } else {
            (self.config.turn_bins, &self.turn_centroids)
        };
        let bm = mask(board);
        let open: Vec<usize> = (0..NUM_HOLES)
            .filter(|&h| {
                let (a, b) = hole_cards(h);
                bm & (bit(a) | bit(b)) == 0
            })
            .collect();
        let free: Vec<Card> = (0..NUM_CARDS as Card)
            .filter(|&c| bm & bit(c) == 0)
            .collect();
        // Histograms over the rivers after each turn card (after the one turn
        // on the turn), summed: integer counts, so in any order the same.
        let turns: Vec<Option<Card>> = if flop {
            free.iter().map(|&t| Some(t)).collect()
        } else {
            vec![None]
        };
        let per_turn = |turn: &Option<Card>| {
            let mut hist = vec![0f32; NUM_HOLES * bins];
            let mut full = [0u8; 5];
            full[..board.len()].copy_from_slice(board);
            let mut eq = [0f32; NUM_HOLES];
            // Each pair of runout cards once: rivers above the turn card.
            let (at, rivers) = match *turn {
                Some(t) => (
                    3,
                    &free[free.iter().position(|&c| c == t).expect("free") + 1..],
                ),
                None => (4, &free[..]),
            };
            if let Some(t) = *turn {
                full[at] = t;
            }
            for &r in rivers {
                full[4] = r;
                river_equities(&full, &mut eq);
                for &h in &open {
                    // -1 marks hands that hold a runout card.
                    if eq[h] >= 0.0 {
                        hist[h * bins + bin(eq[h], bins)] += 1.0;
                    }
                }
            }
            hist
        };
        let add = |mut a: Vec<f32>, b: Vec<f32>| {
            a.iter_mut().zip(b).for_each(|(x, y)| *x += y);
            a
        };
        #[cfg(feature = "parallel")]
        let hist = {
            use rayon::prelude::*;
            turns
                .par_iter()
                .map(per_turn)
                .reduce(|| vec![0f32; NUM_HOLES * bins], add)
        };
        #[cfg(not(feature = "parallel"))]
        let hist = turns
            .iter()
            .map(per_turn)
            .fold(vec![0f32; NUM_HOLES * bins], add);
        let mut out = vec![u16::MAX; NUM_HOLES];
        let mut h = vec![0f32; bins];
        for &i in &open {
            normalize(&hist[i * bins..(i + 1) * bins], &mut h);
            out[i] = nearest(&h, centroids, bins, Distance::Emd) as u16;
        }
        out
    }

    /// A cheap abstraction for tests and quick experiments, built instantly:
    /// preflop classes as usual, then `buckets` equal-width buckets per
    /// street by the hand's current strength against a random hand (no
    /// lookahead for draws). Not for real training.
    pub fn quick(buckets: usize) -> Self {
        let config = AbstractionConfig {
            flop_buckets: buckets,
            turn_buckets: buckets,
            river_buckets: buckets,
            flop_bins: 0,
            turn_bins: 0,
            sample: 0,
            kmeans_iters: 0,
            seed: 0,
        };
        Self {
            config,
            flop: KeyTable::default(),
            turn: KeyTable::default(),
            river_edges: (1..buckets).map(|i| i as f32 / buckets as f32).collect(),
            flop_centroids: Vec::new(),
            turn_centroids: Vec::new(),
        }
    }

    /// The same abstraction without the flop and turn tables: about 100 KB
    /// instead of hundreds of MB. Buckets come out the same, computed from
    /// the hand's equity histogram and the cluster centres when needed
    /// (about 20 ms on the flop, under 1 ms on the turn). `None` if this
    /// abstraction didn't keep its centres.
    pub fn compact(&self) -> Option<Self> {
        if self.flop_centroids.is_empty() || self.turn_centroids.is_empty() {
            return None;
        }
        Some(Self {
            config: self.config,
            flop: KeyTable::default(),
            turn: KeyTable::default(),
            river_edges: self.river_edges.clone(),
            flop_centroids: self.flop_centroids.clone(),
            turn_centroids: self.turn_centroids.clone(),
        })
    }

    /// Whether `other` assigns every hand the same bucket: the same settings,
    /// river edges and flop and turn tables (centres aside).
    pub fn same_buckets(&self, other: &Self) -> bool {
        self.config == other.config
            && self.river_edges == other.river_edges
            && self.flop == other.flop
            && self.turn == other.turn
    }

    /// Whether flop and turn buckets come from tables (fast) rather than
    /// being computed.
    pub fn has_tables(&self) -> bool {
        !self.flop.keys.is_empty()
    }

    /// A flop or turn bucket computed the way the build assigns it: the
    /// hand's river-equity histogram over every runout, then the nearest
    /// cluster centre.
    fn bucket_on_the_fly(&self, hole: [Card; 2], board: &[Card]) -> u16 {
        let flop = board.len() == 3;
        let (bins, centroids) = if flop {
            (self.config.flop_bins, &self.flop_centroids)
        } else {
            (self.config.turn_bins, &self.turn_centroids)
        };
        let used = mask(board) | bit(hole[0]) | bit(hole[1]);
        let free: Vec<Card> = (0..NUM_CARDS as Card)
            .filter(|&c| used & bit(c) == 0)
            .collect();
        let mut hist = vec![0.0f32; bins];
        let mut full = [0u8; 5];
        full[..board.len()].copy_from_slice(board);
        if flop {
            for (i, &t) in free.iter().enumerate() {
                for &r in &free[i + 1..] {
                    full[3] = t;
                    full[4] = r;
                    hist[bin(river_equity(hole, &full), bins)] += 1.0;
                }
            }
        } else {
            for &r in &free {
                full[4] = r;
                hist[bin(river_equity(hole, &full), bins)] += 1.0;
            }
        }
        let mut h = vec![0.0f32; bins];
        normalize(&hist, &mut h);
        nearest(&h, centroids, bins, Distance::Emd) as u16
    }

    /// The river bucket for an equity against a random hand.
    pub fn river_bucket(&self, equity: f32) -> u16 {
        self.river_edges.partition_point(|&e| e <= equity) as u16
    }

    /// Canonical hands stored for the flop and turn.
    pub fn table_sizes(&self) -> (usize, usize) {
        (self.flop.keys.len(), self.turn.keys.len())
    }

    /// The abstraction as bytes: config, river edges, then the flop and turn
    /// tables.
    pub fn save(&self) -> Vec<u8> {
        let c = &self.config;
        let mut out = MAGIC2.to_vec();
        for v in [
            c.flop_buckets,
            c.turn_buckets,
            c.river_buckets,
            c.flop_bins,
            c.turn_bins,
            c.sample,
            c.kmeans_iters,
        ] {
            out.extend((v as u64).to_le_bytes());
        }
        out.extend(c.seed.to_le_bytes());
        out.extend((self.river_edges.len() as u64).to_le_bytes());
        for e in &self.river_edges {
            out.extend(e.to_le_bytes());
        }
        for c in [&self.flop_centroids, &self.turn_centroids] {
            out.extend((c.len() as u64).to_le_bytes());
            for x in c {
                out.extend(x.to_le_bytes());
            }
        }
        for t in [&self.flop, &self.turn] {
            out.extend((t.keys.len() as u64).to_le_bytes());
            for k in &t.keys {
                out.extend(k.to_le_bytes());
            }
            for b in &t.buckets {
                out.extend(b.to_le_bytes());
            }
        }
        out
    }

    pub fn load(mut bytes: &[u8]) -> Option<Self> {
        use crate::key::{read_u64, take};
        let input = &mut bytes;
        let version = match take(input, MAGIC.len())? {
            m if m == MAGIC => 1,
            m if m == MAGIC2 => 2,
            _ => return None,
        };
        let mut v = [0usize; 7];
        for x in &mut v {
            *x = read_u64(input)? as usize;
        }
        let config = AbstractionConfig {
            flop_buckets: v[0],
            turn_buckets: v[1],
            river_buckets: v[2],
            flop_bins: v[3],
            turn_bins: v[4],
            sample: v[5],
            kmeans_iters: v[6],
            seed: read_u64(input)?,
        };
        let n = read_u64(input)? as usize;
        let floats = |input: &mut &[u8], n: usize| {
            (0..n)
                .map(|_| take(input, 4).map(|b| f32::from_le_bytes(b.try_into().expect("4 bytes"))))
                .collect::<Option<Vec<_>>>()
        };
        let river_edges = floats(input, n)?;
        let mut centroids = vec![Vec::new(), Vec::new()];
        if version >= 2 {
            for c in &mut centroids {
                let n = read_u64(input)? as usize;
                *c = floats(input, n)?;
            }
        }
        let turn_centroids = centroids.pop()?;
        let flop_centroids = centroids.pop()?;
        let mut tables = Vec::new();
        for _ in 0..2 {
            let n = read_u64(input)? as usize;
            let keys = (0..n)
                .map(|_| read_u64(input))
                .collect::<Option<Vec<_>>>()?;
            let buckets = (0..n)
                .map(|_| take(input, 2).map(|b| u16::from_le_bytes(b.try_into().expect("2 bytes"))))
                .collect::<Option<Vec<_>>>()?;
            tables.push(KeyTable { keys, buckets });
        }
        let turn = tables.pop()?;
        let flop = tables.pop()?;
        input.is_empty().then_some(Self {
            config,
            flop,
            turn,
            river_edges,
            flop_centroids,
            turn_centroids,
        })
    }
}

const MAGIC: &[u8] = b"DUCYCABS\x01";
/// Version 2 adds the cluster centres.
const MAGIC2: &[u8] = b"DUCYCABS\x02";

fn bucket_of(strength: f32, buckets: usize) -> u16 {
    ((strength * buckets as f32) as usize).min(buckets - 1) as u16
}

/// Share of opponent hands `hole` beats (ties count half) with the board as
/// it is now, ignoring cards to come.
pub fn hand_strength(hole: [Card; 2], board: &[Card]) -> f32 {
    let bm = mask(board);
    let used = bm | bit(hole[0]) | bit(hole[1]);
    let me = score(used);
    let free: Vec<Card> = (0..NUM_CARDS as Card)
        .filter(|&c| used & bit(c) == 0)
        .collect();
    let (mut won, mut n) = (0.0f32, 0.0f32);
    for (i, &a) in free.iter().enumerate() {
        for &b in &free[i + 1..] {
            let them = score(bm | bit(a) | bit(b));
            won += match me.cmp(&them) {
                std::cmp::Ordering::Greater => 1.0,
                std::cmp::Ordering::Equal => 0.5,
                std::cmp::Ordering::Less => 0.0,
            };
            n += 1.0;
        }
    }
    won / n
}

/// The index of `hole` among all 1,326 two-card hands (re-exported for
/// callers building per-hand tables).
pub fn hand_index(hole: [Card; 2]) -> usize {
    hole_index(hole[0], hole[1])
}
