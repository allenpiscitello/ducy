//! The solver as it was before the speed work (#352): eager discounting
//! after every batch, SipHash maps, a fresh `Vec` per node, a serial merge.
//! Kept only to check that the fast solver gives exactly the same results.

use std::collections::HashMap;

use super::{CHUNK, Config, Discount, MAGIC};
use crate::{
    game::{Game, Turn},
    key::Key,
    rng::Rng,
};

struct Slot {
    start: u32,
    n: u8,
}

struct Table<I> {
    index: HashMap<I, u32>,
    slots: Vec<Slot>,
    regret: Vec<f32>,
    sum: Vec<f32>,
}

impl<I: Key> Table<I> {
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

type Pair = (Vec<f64>, Vec<f64>);

struct Delta<I> {
    known: HashMap<u32, Pair>,
    new_index: HashMap<I, usize>,
    new: Vec<(I, Vec<f64>, Vec<f64>)>,
}

impl<I: Key> Delta<I> {
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

pub(super) struct Reference<'a, G: Game> {
    game: &'a G,
    config: Config,
    table: Table<G::Info>,
    iterations: u64,
    batches: u64,
}

impl<'a, G> Reference<'a, G>
where
    G: Game,
    G::Info: Key,
{
    pub(super) fn new(game: &'a G, config: Config) -> Self {
        Self {
            game,
            config,
            table: Table {
                index: HashMap::new(),
                slots: Vec::new(),
                regret: Vec::new(),
                sum: Vec::new(),
            },
            iterations: 0,
            batches: 0,
        }
    }

    pub(super) fn run(&mut self, n: u64) {
        let target = self.iterations + n;
        while self.iterations < target {
            let first = self.iterations;
            let last = first + self.config.batch.max(1);
            let deltas: Vec<_> = (first..last)
                .step_by(CHUNK as usize)
                .map(|s| self.chunk(s, (s + CHUNK).min(last)))
                .collect();
            for d in deltas {
                self.apply(d);
            }
            self.iterations = last;
            self.batches += 1;
            self.discount();
        }
    }

    fn chunk(&self, first: u64, last: u64) -> Delta<G::Info> {
        let mut delta = Delta {
            known: HashMap::new(),
            new_index: HashMap::new(),
            new: Vec::new(),
        };
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

    pub(super) fn save(&self) -> Vec<u8> {
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
}
