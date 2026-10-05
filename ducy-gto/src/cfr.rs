//! Counterfactual regret minimization over the full game tree.
//!
//! Each iteration walks the whole tree. At every information set the solver
//! plays in proportion to its positive regrets (regret matching), then adds
//! how much better each action would have done than its current mix,
//! weighted by how likely chance and the opponent are to reach that point.
//! The *average* strategy over all iterations converges to a Nash
//! equilibrium; the current strategy alone needn't.
//!
//! [`Variant::Vanilla`] is the original algorithm (Zinkevich et al. 2007).
//! [`Variant::Plus`] is CFR+ (Tammelin 2014): regrets are floored at zero,
//! players update in turn, and later iterations weigh more in the average.
//! It converges far faster in practice.
//!
//! This walks every node each iteration, so it's for games small enough to
//! enumerate (Kuhn, Leduc). Bigger games need sampling.

use std::collections::HashMap;

use crate::{
    game::{Game, Turn},
    profile::Profile,
};

/// Which CFR to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    /// Simultaneous updates, uniform averaging.
    Vanilla,
    /// Regrets floored at zero, alternating updates, linear averaging.
    Plus,
}

struct Node {
    /// Cumulative regret per action.
    regret: Vec<f64>,
    /// This iteration's regret, added at the end of the pass so the strategy
    /// stays fixed while the tree is walked.
    pending: Vec<f64>,
    /// Sum of the strategy played, weighted by the player's own reach.
    strategy_sum: Vec<f64>,
    /// The strategy for this iteration, from regret matching.
    current: Vec<f64>,
}

impl Node {
    fn new(n: usize) -> Self {
        Self {
            regret: vec![0.0; n],
            pending: vec![0.0; n],
            strategy_sum: vec![0.0; n],
            current: vec![1.0 / n as f64; n],
        }
    }

    fn update_current(&mut self) {
        let positive: f64 = self.regret.iter().map(|r| r.max(0.0)).sum();
        let n = self.regret.len();
        for (c, r) in self.current.iter_mut().zip(&self.regret) {
            *c = if positive > 0.0 {
                r.max(0.0) / positive
            } else {
                1.0 / n as f64
            };
        }
    }
}

/// Solves a small game with CFR or CFR+.
pub struct Cfr<'a, G: Game> {
    game: &'a G,
    variant: Variant,
    nodes: HashMap<G::Info, Node>,
    iterations: u64,
}

impl<'a, G: Game> Cfr<'a, G> {
    pub fn new(game: &'a G, variant: Variant) -> Self {
        Self {
            game,
            variant,
            nodes: HashMap::new(),
            iterations: 0,
        }
    }

    /// Iterations run so far.
    pub fn iterations(&self) -> u64 {
        self.iterations
    }

    /// Information sets seen so far (all of them after one iteration).
    pub fn num_infosets(&self) -> usize {
        self.nodes.len()
    }

    /// Runs `n` more iterations.
    pub fn run(&mut self, n: u64) {
        for _ in 0..n {
            self.iterations += 1;
            match self.variant {
                Variant::Vanilla => self.pass(None),
                Variant::Plus => {
                    self.pass(Some(0));
                    self.pass(Some(1));
                }
            }
        }
    }

    /// One walk of the tree, updating `player` (or both), then applying the
    /// regrets it found.
    fn pass(&mut self, player: Option<usize>) {
        let root = self.game.root();
        self.walk(&root, [1.0, 1.0], 1.0, player);
        let plus = self.variant == Variant::Plus;
        for node in self.nodes.values_mut() {
            for (r, p) in node.regret.iter_mut().zip(node.pending.iter_mut()) {
                *r += *p;
                if plus {
                    *r = r.max(0.0);
                }
                *p = 0.0;
            }
            node.update_current();
        }
    }

    /// Returns player 0's expected payoff from `s` under the current
    /// strategies. `reach` is each player's probability of playing to `s`,
    /// `chance` chance's.
    fn walk(&mut self, s: &G::State, reach: [f64; 2], chance: f64, update: Option<usize>) -> f64 {
        match self.game.turn(s) {
            Turn::Terminal => self.game.utility(s),
            Turn::Chance => self
                .game
                .chance_outcomes(s)
                .iter()
                .map(|(next, p)| p * self.walk(next, reach, chance * p, update))
                .sum(),
            Turn::Player(p) => {
                let n = self.game.num_actions(s);
                let info = self.game.info(s);
                let sigma = self
                    .nodes
                    .entry(info.clone())
                    .or_insert_with(|| Node::new(n))
                    .current
                    .clone();
                let mut values = vec![0.0; n];
                let mut v = 0.0;
                for a in 0..n {
                    let mut r = reach;
                    r[p] *= sigma[a];
                    values[a] = self.walk(&self.game.apply(s, a), r, chance, update);
                    v += sigma[a] * values[a];
                }
                if update.is_none_or(|u| u == p) {
                    let sign = if p == 0 { 1.0 } else { -1.0 };
                    let others = reach[1 - p] * chance;
                    let weight = match self.variant {
                        Variant::Vanilla => 1.0,
                        Variant::Plus => self.iterations as f64,
                    };
                    let node = self.nodes.get_mut(&info).expect("inserted above");
                    for a in 0..n {
                        node.pending[a] += others * sign * (values[a] - v);
                        node.strategy_sum[a] += weight * reach[p] * sigma[a];
                    }
                }
                v
            }
        }
    }

    /// The average strategy so far, which converges to an equilibrium.
    pub fn average(&self) -> Profile<G::Info> {
        let mut profile = Profile::new();
        for (info, node) in &self.nodes {
            let total: f64 = node.strategy_sum.iter().sum();
            let n = node.strategy_sum.len();
            let probs = if total > 0.0 {
                node.strategy_sum.iter().map(|x| x / total).collect()
            } else {
                vec![1.0 / n as f64; n]
            };
            profile.set(info.clone(), probs);
        }
        profile
    }

    /// The strategy the next iteration would play.
    pub fn current(&self) -> Profile<G::Info> {
        let mut profile = Profile::new();
        for (info, node) in &self.nodes {
            profile.set(info.clone(), node.current.clone());
        }
        profile
    }
}
