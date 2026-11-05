//! Monte Carlo CFR with external sampling (Lanctot et al. 2009), for games
//! too big to walk in full.
//!
//! Each iteration traverses the tree once per player. The traverser explores
//! all of their own actions, while chance and the opponents each play one
//! sampled action. That gives unbiased regret estimates at a tiny fraction of
//! a full walk's cost.
//!
//! It works for any number of players ([`Game::num_players`]), each with
//! their own payoff ([`Game::utility_for`]), as in Pluribus (Brown and
//! Sandholm 2019): nothing assumes the game is zero-sum. With three or more
//! players CFR has no equilibrium guarantee, but in poker it finds strong
//! strategies in practice.
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

/// One batch's discount factors: for positive regrets, for the others, and
/// for strategy sums.
type Factors = [f32; 3];

/// Scales a regret by each batch's factor in turn, as eager discounting
/// after every batch would have.
fn discount_regret(mut x: f32, pending: &[Factors]) -> f32 {
    for f in pending {
        x *= if x > 0.0 { f[0] } else { f[1] };
    }
    x
}

/// Scales a strategy sum by each batch's factor in turn.
fn discount_sum(mut x: f32, pending: &[Factors]) -> f32 {
    for f in pending {
        x *= f[2];
    }
    x
}

/// Regrets and strategy sums for every information set seen so far.
///
/// Discounting is lazy: rather than scaling the whole table after every
/// batch, each batch's factors are kept, and an information set's values
/// are scaled when it's next touched, or read for an average or a
/// checkpoint. The same multiplications happen in the same order as when
/// scaling everything after each batch, so the results are identical, but
/// a batch only costs the information sets it visits.
struct Table<I> {
    index: HashMap<I, u32>,
    slots: Vec<Slot>,
    regret: Vec<f32>,
    sum: Vec<f32>,
    /// Per information set, the batch through which its values are
    /// discounted.
    discounted: Vec<u32>,
    /// The factors of batches `base + 1`, `base + 2`, … up to the last one
    /// that discounted (discounting only ever stops, never restarts).
    factors: Vec<Factors>,
    base: u64,
}

impl<I: Key> Table<I> {
    fn new(base: u64) -> Self {
        Self {
            index: HashMap::new(),
            slots: Vec::new(),
            regret: Vec::new(),
            sum: Vec::new(),
            discounted: Vec::new(),
            factors: Vec::new(),
            base,
        }
    }

    /// Adds an information set, discounted through batch `through`.
    fn insert(&mut self, info: I, n: usize, through: u64) -> u32 {
        let id = self.slots.len() as u32;
        self.slots.push(Slot {
            start: self.regret.len() as u32,
            n: n as u8,
        });
        self.regret.extend(std::iter::repeat_n(0.0, n));
        self.sum.extend(std::iter::repeat_n(0.0, n));
        self.discounted.push(through as u32);
        self.index.insert(info, id);
        id
    }

    fn range(&self, id: u32) -> std::ops::Range<usize> {
        let s = &self.slots[id as usize];
        s.start as usize..s.start as usize + s.n as usize
    }

    /// The factors not yet applied to `id`'s values.
    fn pending(&self, id: u32) -> &[Factors] {
        pending(&self.factors, self.base, self.discounted[id as usize])
    }

    /// `id`'s regret for action `k`, discounted through the last batch.
    fn regret_at(&self, id: u32, k: usize) -> f32 {
        discount_regret(self.regret[self.range(id)][k], self.pending(id))
    }

    /// `id`'s strategy sums, discounted through the last batch.
    fn sums(&self, id: u32, out: &mut Vec<f32>) {
        let pending = self.pending(id);
        out.clear();
        out.extend(
            self.sum[self.range(id)]
                .iter()
                .map(|&x| discount_sum(x, pending)),
        );
    }

    /// Regret matching: play in proportion to positive regret.
    fn strategy(&self, id: Option<u32>, n: usize, out: &mut [f64]) {
        let Some(id) = id else {
            out.fill(1.0 / n as f64);
            return;
        };
        let pending = self.pending(id);
        for (o, &x) in out.iter_mut().zip(&self.regret[self.range(id)]) {
            *o = discount_regret(x, pending) as f64;
        }
        let positive: f64 = out.iter().map(|&x| x.max(0.0)).sum();
        for o in out.iter_mut() {
            *o = if positive > 0.0 {
                o.max(0.0) / positive
            } else {
                1.0 / n as f64
            };
        }
    }

    /// Brings `id`'s values up to date (all factors applied).
    fn settle(&mut self, id: u32) {
        let through = self.base + self.factors.len() as u64;
        let pending = pending(&self.factors, self.base, self.discounted[id as usize]);
        if pending.is_empty() {
            return;
        }
        let r = self.range(id);
        for x in &mut self.regret[r.clone()] {
            *x = discount_regret(*x, pending);
        }
        for x in &mut self.sum[r] {
            *x = discount_sum(*x, pending);
        }
        self.discounted[id as usize] = through as u32;
    }
}

/// The factors after batch `discounted`.
fn pending(factors: &[Factors], base: u64, discounted: u32) -> &[Factors] {
    factors
        .get((discounted as u64 - base) as usize..)
        .unwrap_or(&[])
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
            table: Table::new(0),
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
        assert!(self.batches < u32::MAX as u64, "too many batches");
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
            let dealt;
            let start = if self.game.shared_deal() && self.game.turn(&root) == Turn::Chance {
                dealt = self.game.sample_chance(&root, &mut rng);
                &dealt
            } else {
                &root
            };
            for player in 0..self.game.num_players() {
                self.traverse(start, player, prune, &mut rng, &mut delta);
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
            Turn::Terminal => self.game.utility_for(s, player),
            Turn::Chance => {
                let next = self.game.sample_chance(s, rng);
                self.traverse(&next, player, prune, rng, delta)
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
                            && (self.table.regret_at(id, a) as f64) < threshold
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

    /// Adds one chunk's changes, before this batch's discount.
    fn apply(&mut self, d: Delta<G::Info>) {
        for (id, (regret, sum)) in d.known {
            self.table.settle(id);
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
                None => self.table.insert(info, regret.len(), self.batches),
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
        // Applied lazily (see `Table`). Every batch so far has discounted.
        debug_assert_eq!(
            self.table.base + self.table.factors.len() as u64 + 1,
            self.batches
        );
        self.table
            .factors
            .push([pos as f32, neg as f32, avg as f32]);
    }

    /// The average strategy at one information set, if it has been seen.
    pub fn average_at(&self, info: &G::Info) -> Option<Vec<f64>> {
        let &id = self.table.index.get(info)?;
        let mut s = Vec::new();
        self.table.sums(id, &mut s);
        let total: f64 = s.iter().map(|&x| x as f64).sum();
        Some(if total > 0.0 {
            s.iter().map(|&x| x as f64 / total).collect()
        } else {
            vec![1.0 / s.len() as f64; s.len()]
        })
    }

    /// Calls `f` with every information set seen and its average strategy,
    /// in the order they were first seen, without building a [`Profile`]:
    /// for tables too big to copy.
    pub fn average_each(&self, mut f: impl FnMut(&G::Info, &[f64])) {
        let mut keys: Vec<(&G::Info, u32)> =
            self.table.index.iter().map(|(k, &v)| (k, v)).collect();
        keys.sort_by_key(|&(_, id)| id);
        let mut probs = Vec::new();
        let mut s = Vec::new();
        for (info, id) in keys {
            self.table.sums(id, &mut s);
            let total: f64 = s.iter().map(|&x| x as f64).sum();
            probs.clear();
            if total > 0.0 {
                probs.extend(s.iter().map(|&x| x as f64 / total));
            } else {
                probs.extend(std::iter::repeat_n(1.0 / s.len() as f64, s.len()));
            }
            f(info, &probs);
        }
    }

    /// The average strategy, which converges to an equilibrium.
    pub fn average(&self) -> Profile<G::Info> {
        let mut profile = Profile::new();
        let mut s = Vec::new();
        for (info, &id) in &self.table.index {
            self.table.sums(id, &mut s);
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
        self.save_to(&mut out).expect("writing to memory");
        out
    }

    /// [`Self::save`]'s bytes, written to `w` as they're made, so a big
    /// table needn't be copied whole into memory first. Wrap files in a
    /// `BufWriter`.
    pub fn save_to(&self, w: &mut impl std::io::Write) -> std::io::Result<()> {
        let mut out = Vec::with_capacity(1 << 16);
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
            // As eager discounting would have left them.
            let pending = self.table.pending(id);
            for i in r {
                let regret = discount_regret(self.table.regret[i], pending);
                out.extend(regret.to_le_bytes());
                out.extend(discount_sum(self.table.sum[i], pending).to_le_bytes());
            }
            if out.len() >= 1 << 16 {
                w.write_all(&out)?;
                out.clear();
            }
        }
        w.write_all(&out)
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
        if batches >= u32::MAX as u64 {
            return None;
        }
        let mut table = Table::new(batches);
        for _ in 0..count {
            let info = G::Info::read(input)?;
            let n = *take(input, 1)?.first()? as usize;
            let id = table.insert(info, n, batches);
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

#[cfg(test)]
mod reference;

#[cfg(test)]
mod tests {
    use super::{reference::Reference, *};
    use crate::{
        games::{kuhn3::Kuhn3, leduc::Leduc},
        holdem::abstraction::CardAbstraction,
        sixmax::multiway::{MultiwayConfig, MultiwayGame},
    };

    /// Trains `game` with this solver and the pre-#352 one side by side,
    /// `steps` iterations at a time, and checks that every checkpoint is
    /// byte for byte the same, then that a run resumed from the reference's
    /// checkpoint stays the same.
    fn matches_reference<G>(game: &G, config: Config, steps: &[u64])
    where
        G: Game + Sync,
        G::State: Send,
        G::Info: Key,
    {
        let mut fast = Mccfr::new(game, config);
        let mut slow = Reference::new(game, config);
        for &n in steps {
            fast.run(n);
            slow.run(n);
            let bytes = slow.save();
            assert!(
                fast.save() == bytes,
                "{config:?}: differs at {}",
                fast.iterations()
            );
            let mut streamed = Vec::new();
            fast.save_to(&mut streamed).expect("write");
            assert!(streamed == bytes);
        }
        let mut resumed = Mccfr::load(game, config, &slow.save()).expect("a checkpoint");
        resumed.run(steps[0]);
        slow.run(steps[0]);
        assert!(
            resumed.save() == slow.save(),
            "{config:?}: differs after resuming"
        );
    }

    fn config(discount: Discount, batch: u64) -> Config {
        Config {
            seed: 5,
            batch,
            discount,
            prune: None,
        }
    }

    #[test]
    fn leduc_matches_the_reference_solver() {
        for d in [
            Discount::None,
            Discount::Linear { until: 3_000 },
            Discount::DCFR,
        ] {
            matches_reference(&Leduc, config(d, 64), &[1_000, 2_000, 5_000]);
        }
        let prune = Some(Prune {
            after: 1_000,
            threshold: -5.0,
            full_every: 20,
        });
        matches_reference(
            &Leduc,
            Config {
                prune,
                ..config(Discount::DCFR, 128)
            },
            &[1_500, 6_000],
        );
    }

    #[test]
    fn three_player_kuhn_matches_the_reference_solver() {
        matches_reference(&Kuhn3, config(Discount::DCFR, 32), &[500, 3_000]);
    }

    #[test]
    fn six_handed_holdem_matches_the_reference_solver() {
        let cards = CardAbstraction::quick(4);
        let game = MultiwayGame::<_, 2>::new(
            MultiwayConfig {
                stack: 20,
                ..MultiwayConfig::holdem(10)
            },
            Some(&cards),
        );
        matches_reference(&game, config(Discount::DCFR, 64), &[256, 512]);
    }
}
