//! Monte Carlo CFR for more than two players (#249), on the six-handed
//! preflop game: external sampling, one traversal per player per iteration,
//! regrets floored at zero (as in CFR+) and the average strategy weighted by
//! iteration (as in linear CFR).
//!
//! With three or more players CFR has no equilibrium guarantee, but in
//! poker it finds strong strategies in practice (Pluribus, Brown and
//! Sandholm 2019). The tables are shared by worker threads without locks:
//! each slot is an atomic `f32`, and the rare lost update only adds a little
//! noise, as in Hogwild!-style training.

use std::{
    sync::atomic::{AtomicU32, AtomicU64, Ordering::Relaxed},
    thread,
};

use super::{
    bot::PreflopStrategy,
    equity::PreflopEquity,
    preflop::{Node, PreflopGame, SEATS},
};
use crate::{
    Rng,
    holdem::{cards::Card, iso::NUM_PREFLOP_CLASSES, iso::preflop_class},
};

const C: usize = NUM_PREFLOP_CLASSES;
const MAGIC: &[u8; 8] = b"DUCY6PF\0";
const VERSION: u32 = 1;

fn load(a: &AtomicU32) -> f32 {
    f32::from_bits(a.load(Relaxed))
}

fn add(a: &AtomicU32, x: f32) {
    a.store((load(a) + x).to_bits(), Relaxed);
}

pub struct PreflopSolver<'a> {
    game: &'a PreflopGame,
    eq: &'a PreflopEquity,
    /// Per decision id: where its slots start, and its number of actions.
    offsets: Vec<(usize, usize)>,
    regret: Vec<AtomicU32>,
    average: Vec<AtomicU32>,
    iterations: AtomicU64,
    seed: u64,
    /// When set, every seat but the one being trained plays this fixed
    /// strategy (`against_field`).
    field: Option<&'a PreflopStrategy>,
}

impl<'a> PreflopSolver<'a> {
    /// A solver where the field plays `field` (a recorded field's strategy,
    /// `FieldCounts::strategy`) and only the seat being traversed learns: each
    /// position's strategy becomes a best response to five players who play
    /// like the field, rather than an equilibrium.
    pub fn against_field(
        game: &'a PreflopGame,
        eq: &'a PreflopEquity,
        field: &'a PreflopStrategy,
        seed: u64,
    ) -> Self {
        assert_eq!(field.game().decisions, game.decisions, "same tree");
        Self {
            field: Some(field),
            ..Self::new(game, eq, seed)
        }
    }

    pub fn new(game: &'a PreflopGame, eq: &'a PreflopEquity, seed: u64) -> Self {
        let mut offsets = vec![(0, 0); game.decisions as usize];
        let mut total = 0;
        for node in &game.nodes {
            if let Node::Decision { id, actions, .. } = node {
                offsets[*id as usize] = (total, actions.len());
                total += actions.len() * C;
            }
        }
        let zeros = |n: usize| (0..n).map(|_| AtomicU32::new(0)).collect();
        Self {
            game,
            eq,
            offsets,
            regret: zeros(total),
            average: zeros(total),
            iterations: AtomicU64::new(0),
            seed,
            field: None,
        }
    }

    pub fn iterations(&self) -> u64 {
        self.iterations.load(Relaxed)
    }

    fn slots(&self, id: u32, class: usize) -> (usize, usize) {
        let (start, n) = self.offsets[id as usize];
        (start + class * n, n)
    }

    /// The current strategy by regret matching.
    fn current(&self, id: u32, class: usize, out: &mut [f64]) {
        let (base, n) = self.slots(id, class);
        let mut sum = 0.0;
        for (x, slot) in out.iter_mut().zip(&self.regret[base..base + n]) {
            *x = load(slot).max(0.0) as f64;
            sum += *x;
        }
        for x in out.iter_mut().take(n) {
            *x = if sum > 0.0 { *x / sum } else { 1.0 / n as f64 };
        }
    }

    /// The average strategy at a decision for a class.
    pub fn strategy(&self, id: u32, class: usize) -> Vec<f64> {
        let (base, n) = self.slots(id, class);
        let w: Vec<f64> = (0..n)
            .map(|a| load(&self.average[base + a]) as f64)
            .collect();
        let sum: f64 = w.iter().sum();
        if sum > 0.0 {
            w.iter().map(|x| x / sum).collect()
        } else {
            vec![1.0 / n as f64; n]
        }
    }

    fn deal(rng: &mut Rng) -> [usize; SEATS] {
        let mut used = 0u64;
        let mut draw = || loop {
            let c = (rng.next_u64() % 52) as Card;
            if used & 1 << c == 0 {
                used |= 1 << c;
                return c;
            }
        };
        let mut classes = [0; SEATS];
        for c in &mut classes {
            let a = draw();
            let b = draw();
            *c = preflop_class(a, b);
        }
        classes
    }

    /// `reach` is the traversed player's own chance of playing to this node,
    /// used for its average strategy when the field is fixed.
    fn traverse(
        &self,
        node: usize,
        player: usize,
        classes: &[usize; SEATS],
        weight: f32,
        reach: f32,
        rng: &mut Rng,
    ) -> f64 {
        match &self.game.nodes[node] {
            Node::End { .. } => self.game.payoffs(node, classes, self.eq)[player],
            Node::Decision { seat, id, actions } => {
                let n = actions.len();
                let mut sigma = [0f64; 4];
                let (base, _) = self.slots(*id, classes[*seat]);
                if *seat == player {
                    self.current(*id, classes[*seat], &mut sigma);
                    if self.field.is_some() {
                        // The field never updates the average here: the
                        // player's own, weighted by its reach.
                        for (slot, p) in self.average[base..base + n].iter().zip(sigma) {
                            add(slot, weight * reach * p as f32);
                        }
                    }
                    let mut values = [0f64; 4];
                    let mut value = 0.0;
                    for (a, &(_, child)) in actions.iter().enumerate() {
                        let r = reach * sigma[a] as f32;
                        values[a] = self.traverse(child as usize, player, classes, weight, r, rng);
                        value += sigma[a] * values[a];
                    }
                    for (slot, v) in self.regret[base..base + n].iter().zip(values) {
                        let r = (load(slot) + (v - value) as f32).max(0.0);
                        slot.store(r.to_bits(), Relaxed);
                    }
                    value
                } else if let Some(field) = self.field {
                    for (s, &p) in sigma.iter_mut().zip(field.probs(*id, classes[*seat])) {
                        *s = p as f64;
                    }
                    let a = rng.sample(&sigma[..n]);
                    self.traverse(actions[a].1 as usize, player, classes, weight, reach, rng)
                } else {
                    self.current(*id, classes[*seat], &mut sigma);
                    for (slot, p) in self.average[base..base + n].iter().zip(sigma) {
                        add(slot, weight * p as f32);
                    }
                    let a = rng.sample(&sigma[..n]);
                    self.traverse(actions[a].1 as usize, player, classes, weight, reach, rng)
                }
            }
        }
    }

    /// Runs `iterations` more iterations on `threads` threads. Each
    /// iteration deals six hands and traverses once for every seat.
    pub fn run(&self, iterations: u64, threads: usize) {
        let start = self.iterations();
        let threads = threads.max(1) as u64;
        thread::scope(|scope| {
            for t in 0..threads {
                scope.spawn(move || {
                    let mut i = start + t;
                    while i < start + iterations {
                        let mut rng = Rng::for_iteration(self.seed, i);
                        let classes = Self::deal(&mut rng);
                        let weight = (i + 1) as f32;
                        for player in 0..SEATS {
                            self.traverse(
                                self.game.root(),
                                player,
                                &classes,
                                weight,
                                1.0,
                                &mut rng,
                            );
                        }
                        i += threads;
                    }
                });
            }
        });
        self.iterations.store(start + iterations, Relaxed);
    }

    /// The tables, to resume training or read the strategy later.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(32 + 8 * self.regret.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&self.game.stack.to_le_bytes());
        out.extend_from_slice(&(self.regret.len() as u64).to_le_bytes());
        out.extend_from_slice(&self.iterations().to_le_bytes());
        for x in self.regret.iter().chain(&self.average) {
            out.extend_from_slice(&x.load(Relaxed).to_le_bytes());
        }
        out
    }

    /// Loads tables saved by [`Self::to_bytes`] for the same game.
    pub fn load(&self, bytes: &[u8]) -> Result<(), String> {
        let n = self.regret.len();
        if bytes.len() != 32 + 8 * n || &bytes[..8] != MAGIC {
            return Err("not a six-handed preflop checkpoint for this game".into());
        }
        let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().expect("4"));
        let u64_at = |i: usize| u64::from_le_bytes(bytes[i..i + 8].try_into().expect("8"));
        if u32_at(8) != VERSION || u32_at(12) != self.game.stack || u64_at(16) as usize != n {
            return Err("checkpoint for another version or depth".into());
        }
        self.iterations.store(u64_at(24), Relaxed);
        for (i, slot) in self.regret.iter().chain(&self.average).enumerate() {
            slot.store(u32_at(32 + 4 * i), Relaxed);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{holdem::chart::class_name, sixmax::preflop::Act};

    fn class(name: &str) -> usize {
        (0..C).find(|&c| class_name(c) == name).expect("class")
    }

    #[test]
    fn learns_to_open_aces_and_fold_trash() {
        let eq = PreflopEquity::compute(200, 3, 4);
        let game = PreflopGame::new(100);
        let solver = PreflopSolver::new(&game, &eq, 5);
        solver.run(60_000, 4);
        // UTG, first in: the root decision.
        let Node::Decision { id, actions, .. } = &game.nodes[game.root()] else {
            panic!("root");
        };
        let fold = actions
            .iter()
            .position(|(a, _)| *a == Act::Fold)
            .expect("fold");
        let aa = solver.strategy(*id, class("AA"));
        let trash = solver.strategy(*id, class("72o"));
        assert!(aa[fold] < 0.05, "AA folds {:?}", aa);
        assert!(trash[fold] > 0.9, "72o plays {:?}", trash);
        let again = PreflopSolver::new(&game, &eq, 5);
        again.load(&solver.to_bytes()).expect("load");
        assert_eq!(again.strategy(*id, class("AA")), aa);
        assert_eq!(again.iterations(), 60_000);
    }

    #[test]
    fn against_a_field_that_always_folds_it_plays_any_hand() {
        let eq = PreflopEquity::compute(200, 3, 4);
        let game = PreflopGame::new(100);
        // The field folds whenever it can, and checks otherwise.
        let mut by_id = vec![None; game.decisions as usize];
        for node in &game.nodes {
            if let Node::Decision { id, actions, .. } = node {
                by_id[*id as usize] = Some(actions);
            }
        }
        let field = PreflopStrategy::from_fn(&game, |id, _| {
            let actions = by_id[id as usize].expect("decision");
            let fold = actions.iter().position(|(a, _)| *a == Act::Fold);
            let call = actions.iter().position(|(a, _)| *a == Act::Call);
            let pick = fold.or(call).expect("fold or check");
            (0..actions.len())
                .map(|a| if a == pick { 1.0 } else { 0.0 })
                .collect()
        });
        let solver = PreflopSolver::against_field(&game, &eq, &field, 5);
        solver.run(20_000, 4);
        let Node::Decision { id, actions, .. } = &game.nodes[game.root()] else {
            panic!("root");
        };
        let fold = actions
            .iter()
            .position(|(a, _)| *a == Act::Fold)
            .expect("fold");
        // Everyone folds behind: even 72o wins the blinds by playing.
        let trash = solver.strategy(*id, class("72o"));
        assert!(trash[fold] < 0.1, "72o folds {:?}", trash);
    }
}
