//! Monte Carlo CFR with external sampling (Lanctot et al. 2009), for games
//! too big to walk in full.
//!
//! Each iteration traverses the tree once per player. The traverser explores
//! all of their own actions, while chance and the opponent each play one
//! sampled action. That gives unbiased regret estimates at a tiny fraction of
//! a full walk's cost.
//!
//! On top of that:
//! - **Discounting** ([`Discount`]): early iterations, played with a poor
//!   strategy, count for less. Linear CFR and DCFR (Brown and Sandholm 2019)
//!   converge much faster than plain averaging.
//! - **Regret-based pruning** ([`Prune`]): actions with very negative regret
//!   are skipped on most traversals. Periodic full traversals let them
//!   recover if they turn out to be good.
//! - **Parallel batches:** iterations run in batches with the strategy fixed
//!   within a batch, spread over all cores. Each iteration has its own random
//!   stream, and results are merged in a fixed order, so a run is reproducible
//!   for a given seed and batch size whatever the thread count.
//! - **Compact storage:** regrets and strategy sums are `f32`s in flat arrays,
//!   indexed through one map from information set to slot.
//! - **Checkpoints:** [`Mccfr::save`] and [`Mccfr::load`]. A resumed run
//!   matches one that never stopped.

use std::collections::HashMap;

use crate::{
    game::{Game, Turn},
    key::{Key, read_u64, take},
    profile::Profile,
    rng::Rng,
};

/// How earlier iterations are discounted. With `t` the number of batches so
/// far, after each batch:
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Discount {
    /// No discounting: every iteration counts equally.
    None,
    /// Linear CFR: regrets and strategy sums are scaled by `t / (t + 1)`, so
    /// batch `t` weighs in proportion to `t`. Stops after `until` iterations,
    /// when the strategy is good enough that plain averaging is fine.
    Linear { until: u64 },
    /// Discounted CFR: positive regrets are scaled by `t^α / (t^α + 1)`,
    /// negative ones by `t^β / (t^β + 1)`, and strategy sums by
    /// `(t / (t + 1))^γ`. The authors recommend α = 1.5, β = 0, γ = 2.
    Dcfr { alpha: f64, beta: f64, gamma: f64 },
}

impl Discount {
    /// DCFR with the recommended α = 1.5, β = 0, γ = 2.
    pub const DCFR: Discount = Discount::Dcfr {
        alpha: 1.5,
        beta: 0.0,
        gamma: 2.0,
    };
}

/// Regret-based pruning: once `after` iterations have run, actions whose
/// regret is below `threshold` are skipped, except on every `full_every`-th
/// iteration, which explores everything.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prune {
    pub after: u64,
    pub threshold: f64,
    pub full_every: u64,
}

/// How to train.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Config {
    /// Seed for the sampling; the same seed and batch size give the same run.
    pub seed: u64,
    /// Iterations per batch. The strategy stays fixed within a batch, and the
    /// batch is what runs in parallel, so larger batches use more cores but
    /// update the strategy less often.
    pub batch: u64,
    pub discount: Discount,
    pub prune: Option<Prune>,
}

impl Default for Config {
    /// Seed 0, batches of 64, DCFR, no pruning.
    fn default() -> Self {
        Self {
            seed: 0,
            batch: 64,
            discount: Discount::DCFR,
            prune: None,
        }
    }
}

/// Iterations each parallel task runs in order. Fixed, so the merge order
/// doesn't depend on the number of threads.
const CHUNK: u64 = 16;

struct Slot {
    start: u32,
    n: u8,
}

/// Regrets and strategy sums for every information set seen so far.
struct Table<I> {
    index: HashMap<I, u32>,
    slots: Vec<Slot>,
    regret: Vec<f32>,
    sum: Vec<f32>,
}

impl<I: Key> Table<I> {
    fn new() -> Self {
        Self {
            index: HashMap::new(),
            slots: Vec::new(),
            regret: Vec::new(),
            sum: Vec::new(),
        }
    }

    fn insert(&mut self, info: I, n: usize) -> u32 {
        let id = self.slots.len() as u32;
        self.slots.push(Slot {
            start: self.regret.len() as u32,
            n: n as u8,
        });
        self.regret.extend(std::iter::repeat_n(0.0, n));
        self.sum.extend(std::iter::repeat_n(0.0, n));
        self.index.insert(info, id);
        id
    }

    fn range(&self, id: u32) -> std::ops::Range<usize> {
        let s = &self.slots[id as usize];
        s.start as usize..s.start as usize + s.n as usize
    }

    /// Regret matching: play in proportion to positive regret.
    fn strategy(&self, id: Option<u32>, n: usize, out: &mut [f64]) {
        let Some(id) = id else {
            out.fill(1.0 / n as f64);
            return;
        };
        let r = &self.regret[self.range(id)];
        let positive: f64 = r.iter().map(|&x| (x as f64).max(0.0)).sum();
        for (o, &x) in out.iter_mut().zip(r) {
            *o = if positive > 0.0 {
                (x as f64).max(0.0) / positive
            } else {
                1.0 / n as f64
            };
        }
    }
}

/// Changes found by one chunk of iterations, applied after the batch.
struct Delta<I> {
    known: HashMap<u32, (Vec<f64>, Vec<f64>)>,
    new_index: HashMap<I, usize>,
    /// Information sets not yet in the table, in the order first seen.
    new: Vec<(I, Vec<f64>, Vec<f64>)>,
}

impl<I: Key> Delta<I> {
    fn new() -> Self {
        Self {
            known: HashMap::new(),
            new_index: HashMap::new(),
            new: Vec::new(),
        }
    }

    fn entry(&mut self, id: Option<u32>, info: &I, n: usize) -> (&mut Vec<f64>, &mut Vec<f64>) {
        match id {
            Some(id) => {
                let e = self
                    .known
                    .entry(id)
                    .or_insert_with(|| (vec![0.0; n], vec![0.0; n]));
                (&mut e.0, &mut e.1)
            }
            None => {
                let i = match self.new_index.get(info) {
                    Some(&i) => i,
                    None => {
                        self.new.push((info.clone(), vec![0.0; n], vec![0.0; n]));
                        self.new_index.insert(info.clone(), self.new.len() - 1);
                        self.new.len() - 1
                    }
                };
                let e = &mut self.new[i];
                (&mut e.1, &mut e.2)
            }
        }
    }
}

/// Monte Carlo CFR with external sampling.
pub struct Mccfr<'a, G: Game> {
    game: &'a G,
    config: Config,
    table: Table<G::Info>,
    iterations: u64,
    batches: u64,
}

impl<'a, G> Mccfr<'a, G>
where
    G: Game + Sync,
    G::State: Send,
    G::Info: Key,
{
    pub fn new(game: &'a G, config: Config) -> Self {
        Self {
            game,
            config,
            table: Table::new(),
            iterations: 0,
            batches: 0,
        }
    }

    pub fn iterations(&self) -> u64 {
        self.iterations
    }

    pub fn num_infosets(&self) -> usize {
        self.table.slots.len()
    }

    /// Bytes of regret and strategy storage (not counting the index).
    pub fn table_bytes(&self) -> usize {
        (self.table.regret.len() + self.table.sum.len()) * std::mem::size_of::<f32>()
    }

    /// Runs `n` more iterations (rounded up to whole batches).
    pub fn run(&mut self, n: u64) {
        let target = self.iterations + n;
        while self.iterations < target {
            self.batch();
        }
    }

    fn batch(&mut self) {
        let first = self.iterations;
        let last = first + self.config.batch.max(1);
        let chunks: Vec<(u64, u64)> = (first..last)
            .step_by(CHUNK as usize)
            .map(|s| (s, (s + CHUNK).min(last)))
            .collect();
        let deltas = self.map_chunks(&chunks);
        for d in deltas {
            self.apply(d);
        }
        self.iterations = last;
        self.batches += 1;
        self.discount();
    }

    #[cfg(feature = "parallel")]
    fn map_chunks(&self, chunks: &[(u64, u64)]) -> Vec<Delta<G::Info>> {
        use rayon::prelude::*;
        chunks.par_iter().map(|&(a, b)| self.chunk(a, b)).collect()
    }

    #[cfg(not(feature = "parallel"))]
    fn map_chunks(&self, chunks: &[(u64, u64)]) -> Vec<Delta<G::Info>> {
        chunks.iter().map(|&(a, b)| self.chunk(a, b)).collect()
    }

    fn chunk(&self, first: u64, last: u64) -> Delta<G::Info> {
        let mut delta = Delta::new();
        let root = self.game.root();
        for i in first..last {
            let mut rng = Rng::for_iteration(self.config.seed, i);
            let prune = self
                .config
                .prune
                .is_some_and(|p| i >= p.after && (p.full_every == 0 || i % p.full_every != 0));
            for player in 0..2 {
                self.traverse(&root, player, prune, &mut rng, &mut delta);
            }
        }
        delta
    }

    /// The traverser's sampled value of `s`.
    fn traverse(
        &self,
        s: &G::State,
        player: usize,
        prune: bool,
        rng: &mut Rng,
        delta: &mut Delta<G::Info>,
    ) -> f64 {
        match self.game.turn(s) {
            Turn::Terminal => {
                let u = self.game.utility(s);
                if player == 0 { u } else { -u }
            }
            Turn::Chance => {
                let outcomes = self.game.chance_outcomes(s);
                let probs: Vec<f64> = outcomes.iter().map(|(_, p)| *p).collect();
                let i = rng.sample(&probs);
                self.traverse(&outcomes[i].0, player, prune, rng, delta)
            }
            Turn::Player(p) => {
                let n = self.game.num_actions(s);
                let info = self.game.info(s);
                let id = self.table.index.get(&info).copied();
                let mut sigma = vec![0.0; n];
                self.table.strategy(id, n, &mut sigma);
                if p == player {
                    let threshold = self.config.prune.map_or(f64::NEG_INFINITY, |p| p.threshold);
                    let mut values = vec![0.0; n];
                    let mut explored = vec![true; n];
                    let mut v = 0.0;
                    for a in 0..n {
                        if prune
                            && let Some(id) = id
                            && (self.table.regret[self.table.range(id)][a] as f64) < threshold
                        {
                            explored[a] = false;
                            continue;
                        }
                        values[a] =
                            self.traverse(&self.game.apply(s, a), player, prune, rng, delta);
                        v += sigma[a] * values[a];
                    }
                    let (regret, _) = delta.entry(id, &info, n);
                    for a in 0..n {
                        if explored[a] {
                            regret[a] += values[a] - v;
                        }
                    }
                    v
                } else {
                    let (_, sum) = delta.entry(id, &info, n);
                    for a in 0..n {
                        sum[a] += sigma[a];
                    }
                    let a = rng.sample(&sigma);
                    self.traverse(&self.game.apply(s, a), player, prune, rng, delta)
                }
            }
        }
    }

    fn apply(&mut self, d: Delta<G::Info>) {
        for (id, (regret, sum)) in d.known {
            let r = self.table.range(id);
            for (k, i) in r.enumerate() {
                self.table.regret[i] += regret[k] as f32;
                self.table.sum[i] += sum[k] as f32;
            }
        }
        for (info, regret, sum) in d.new {
            // Seen by an earlier chunk of this batch: now a known slot.
            let id = match self.table.index.get(&info) {
                Some(&id) => id,
                None => self.table.insert(info, regret.len()),
            };
            let r = self.table.range(id);
            for (k, i) in r.enumerate() {
                self.table.regret[i] += regret[k] as f32;
                self.table.sum[i] += sum[k] as f32;
            }
        }
    }

    fn discount(&mut self) {
        let t = self.batches as f64;
        let (pos, neg, avg) = match self.config.discount {
            Discount::None => return,
            Discount::Linear { until } => {
                if self.iterations > until {
                    return;
                }
                let d = t / (t + 1.0);
                (d, d, d)
            }
            Discount::Dcfr { alpha, beta, gamma } => (
                t.powf(alpha) / (t.powf(alpha) + 1.0),
                t.powf(beta) / (t.powf(beta) + 1.0),
                (t / (t + 1.0)).powf(gamma),
            ),
        };
        for r in &mut self.table.regret {
            *r *= if *r > 0.0 { pos as f32 } else { neg as f32 };
        }
        for s in &mut self.table.sum {
            *s *= avg as f32;
        }
    }

    /// The average strategy, which converges to an equilibrium.
    pub fn average(&self) -> Profile<G::Info> {
        let mut profile = Profile::new();
        for (info, &id) in &self.table.index {
            let s = &self.table.sum[self.table.range(id)];
            let total: f64 = s.iter().map(|&x| x as f64).sum();
            let n = s.len();
            let probs = if total > 0.0 {
                s.iter().map(|&x| x as f64 / total).collect()
            } else {
                vec![1.0 / n as f64; n]
            };
            profile.set(info.clone(), probs);
        }
        profile
    }

    /// Everything needed to resume: the counters, then every information set
    /// in slot order with its regrets and strategy sums.
    pub fn save(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(MAGIC);
        out.extend(self.iterations.to_le_bytes());
        out.extend(self.batches.to_le_bytes());
        out.extend(self.config.seed.to_le_bytes());
        out.extend((self.table.slots.len() as u64).to_le_bytes());
        let mut keys: Vec<(&G::Info, u32)> =
            self.table.index.iter().map(|(k, &v)| (k, v)).collect();
        keys.sort_by_key(|&(_, id)| id);
        for (info, id) in keys {
            info.write(&mut out);
            let r = self.table.range(id);
            out.push(r.len() as u8);
            for i in r {
                out.extend(self.table.regret[i].to_le_bytes());
                out.extend(self.table.sum[i].to_le_bytes());
            }
        }
        out
    }

    /// Resumes from [`Self::save`]'s bytes. `config` should match the saved
    /// run's (its seed must) for the run to continue exactly as if it never
    /// stopped. Returns `None` for bytes that aren't a checkpoint or don't
    /// match the seed.
    pub fn load(game: &'a G, config: Config, mut bytes: &[u8]) -> Option<Self> {
        let input = &mut bytes;
        if take(input, MAGIC.len())? != MAGIC {
            return None;
        }
        let iterations = read_u64(input)?;
        let batches = read_u64(input)?;
        if read_u64(input)? != config.seed {
            return None;
        }
        let count = read_u64(input)?;
        let mut table = Table::new();
        for _ in 0..count {
            let info = G::Info::read(input)?;
            let n = *take(input, 1)?.first()? as usize;
            let id = table.insert(info, n);
            let r = table.range(id);
            for i in r {
                let b = take(input, 8)?;
                table.regret[i] = f32::from_le_bytes(b[..4].try_into().ok()?);
                table.sum[i] = f32::from_le_bytes(b[4..].try_into().ok()?);
            }
        }
        input.is_empty().then_some(Self {
            game,
            config,
            table,
            iterations,
            batches,
        })
    }
}

const MAGIC: &[u8] = b"DUCYMCCF\x01";
