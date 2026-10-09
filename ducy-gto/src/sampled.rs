//! A sampled best response, for games too big to evaluate exactly.
//!
//! [`best_response_value`](crate::best_response_value) walks every chance
//! outcome, which is out of reach for Hold'em or Omaha. Here the best
//! responder plays the same abstract game (the same information sets: in
//! the poker games, the betting and its own bucket), against a fixed
//! strategy, and chance is sampled:
//!
//! 1. **Fit:** deal `fit` hands and walk the whole betting tree for all of
//!    them at once. At each of the responder's information sets, pick the
//!    action with the highest total value over the deals that reach it.
//! 2. **Evaluate:** play that policy against the strategy on `eval` fresh
//!    deals, exactly in expectation over the strategy's mixed actions.
//!
//! Evaluating on fresh deals keeps the estimate honest: it can't profit from
//! noise in the deals it was fitted on. Its value is a *lower* bound on the
//! abstract game's best response (the fitted policy is only near-best,
//! especially where few deals reach an information set), with sampling
//! error on top, reported as a standard error. It's the "bucketed best
//! response" the PLO bot uses in place of exact exploitability.
//!
//! It needs games where the deals at one point in the betting reach the
//! same kind of node (all chance, all one player, or all over), as the poker
//! games here and Kuhn and Leduc are.

use std::collections::HashMap;

use crate::{
    game::{Game, Turn},
    rng::Rng,
};

/// A sampled best response's result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SampledResponse {
    /// The responder's mean payoff per deal on the evaluation deals.
    pub value: f64,
    /// Its standard error.
    pub std_error: f64,
}

/// How the walks draw chance outcomes after the root: each deal has its own
/// stream, so the result doesn't depend on parallelism.
fn chance<G: Game>(game: &G, states: &[G::State], rngs: &mut [Rng]) -> Vec<G::State> {
    states
        .iter()
        .zip(rngs.iter_mut())
        .map(|(s, r)| game.sample_chance(s, r))
        .collect()
}

struct Walk<'a, G: Game, S> {
    game: &'a G,
    strategy: &'a S,
    br: usize,
}

impl<G, S> Walk<'_, G, S>
where
    G: Game,
    S: Fn(&G::Info, usize) -> Vec<f64>,
{
    /// The responder's value for each deal from `states` on, choosing its
    /// actions to maximise the reach-weighted total per information set;
    /// the choices go into `policy`.
    fn fit(
        &self,
        states: Vec<G::State>,
        reach: Vec<f64>,
        mut rngs: Vec<Rng>,
        policy: &mut HashMap<G::Info, usize>,
    ) -> Vec<f64> {
        let g = self.game;
        match g.turn(&states[0]) {
            Turn::Terminal => states.iter().map(|s| self.payoff(s)).collect(),
            Turn::Chance => {
                let next = chance(g, &states, &mut rngs);
                self.fit(next, reach, rngs, policy)
            }
            Turn::Player(p) => {
                let n = g.num_actions(&states[0]);
                let infos: Vec<G::Info> = states.iter().map(|s| g.info(s)).collect();
                if p == self.br {
                    let children: Vec<Vec<f64>> = (0..n)
                        .map(|a| {
                            let next = states.iter().map(|s| g.apply(s, a)).collect();
                            self.fit(next, reach.clone(), rngs.clone(), policy)
                        })
                        .collect();
                    let mut totals: HashMap<&G::Info, Vec<f64>> = HashMap::new();
                    for (d, info) in infos.iter().enumerate() {
                        let t = totals.entry(info).or_insert_with(|| vec![0.0; n]);
                        for a in 0..n {
                            t[a] += reach[d] * children[a][d];
                        }
                    }
                    let mut choice: HashMap<&G::Info, usize> = HashMap::new();
                    for (info, t) in totals {
                        let best = (0..n).max_by(|&x, &y| t[x].total_cmp(&t[y])).unwrap_or(0);
                        policy.insert(info.clone(), best);
                        choice.insert(info, best);
                    }
                    infos
                        .iter()
                        .enumerate()
                        .map(|(d, info)| children[choice[info]][d])
                        .collect()
                } else {
                    let probs: Vec<Vec<f64>> =
                        infos.iter().map(|i| (self.strategy)(i, n)).collect();
                    let mut out = vec![0.0; states.len()];
                    for a in 0..n {
                        if probs.iter().all(|p| p[a] == 0.0) {
                            continue;
                        }
                        let next = states.iter().map(|s| g.apply(s, a)).collect();
                        let r = reach.iter().zip(&probs).map(|(r, p)| r * p[a]).collect();
                        let v = self.fit(next, r, rngs.clone(), policy);
                        for d in 0..out.len() {
                            out[d] += probs[d][a] * v[d];
                        }
                    }
                    out
                }
            }
        }
    }

    /// The responder's value for each deal playing `policy` (action `1`, or
    /// `0` if there's only one, where it has no choice recorded).
    fn eval(
        &self,
        states: Vec<G::State>,
        rngs: &mut [Rng],
        policy: &HashMap<G::Info, usize>,
    ) -> Vec<f64> {
        let g = self.game;
        match g.turn(&states[0]) {
            Turn::Terminal => states.iter().map(|s| self.payoff(s)).collect(),
            Turn::Chance => {
                let next = chance(g, &states, rngs);
                self.eval(next, rngs, policy)
            }
            Turn::Player(p) => {
                let n = g.num_actions(&states[0]);
                let infos: Vec<G::Info> = states.iter().map(|s| g.info(s)).collect();
                let mut out = vec![0.0; states.len()];
                for a in 0..n {
                    let weights: Vec<f64> = if p == self.br {
                        infos
                            .iter()
                            .map(|i| {
                                let pick = policy.get(i).copied().unwrap_or(1.min(n - 1));
                                if pick == a { 1.0 } else { 0.0 }
                            })
                            .collect()
                    } else {
                        infos.iter().map(|i| (self.strategy)(i, n)[a]).collect()
                    };
                    let live: Vec<usize> =
                        (0..states.len()).filter(|&d| weights[d] > 0.0).collect();
                    if live.is_empty() {
                        continue;
                    }
                    let next = live.iter().map(|&d| g.apply(&states[d], a)).collect();
                    let mut sub: Vec<Rng> = live.iter().map(|&d| rngs[d].clone()).collect();
                    let v = self.eval(next, &mut sub, policy);
                    for (k, &d) in live.iter().enumerate() {
                        out[d] += weights[d] * v[k];
                    }
                }
                out
            }
        }
    }

    fn payoff(&self, s: &G::State) -> f64 {
        let u = self.game.utility(s);
        if self.br == 0 { u } else { -u }
    }
}

/// The value player `br` gets by best-responding to `strategy` (each
/// information set's action probabilities), fitted on `fit` sampled deals
/// and measured on `eval` fresh ones. See the module docs.
pub fn sampled_best_response<G, S>(
    game: &G,
    strategy: &S,
    br: usize,
    fit: usize,
    eval: usize,
    seed: u64,
) -> SampledResponse
where
    G: Game + Sync,
    G::State: Send + Sync,
    G::Info: Send + Sync,
    S: Fn(&G::Info, usize) -> Vec<f64> + Sync,
{
    summarize(&best_response_values(game, strategy, br, fit, eval, seed))
}

/// Sampled exploitability: the mean of both players' sampled best
/// responses to `strategy`, measured on the *same* evaluation deals, so the
/// luck of the cards largely cancels between the two seats (as in a
/// duplicate match) and the standard error is far smaller than either
/// response's alone.
pub fn sampled_exploitability<G, S>(
    game: &G,
    strategy: &S,
    fit: usize,
    eval: usize,
    seed: u64,
) -> SampledResponse
where
    G: Game + Sync,
    G::State: Send + Sync,
    G::Info: Send + Sync,
    S: Fn(&G::Info, usize) -> Vec<f64> + Sync,
{
    let values = |br: usize| best_response_values(game, strategy, br, fit, eval, seed);
    #[cfg(feature = "parallel")]
    let (a, b) = rayon::join(|| values(0), || values(1));
    #[cfg(not(feature = "parallel"))]
    let (a, b) = (values(0), values(1));
    let both: Vec<f64> = a.iter().zip(&b).map(|(x, y)| (x + y) / 2.0).collect();
    summarize(&both)
}

fn summarize(values: &[f64]) -> SampledResponse {
    let n = values.len().max(1) as f64;
    let mean = values.iter().sum::<f64>() / n;
    let var = values.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / (n - 1.0).max(1.0);
    SampledResponse {
        value: mean,
        std_error: (var / n).sqrt(),
    }
}

/// The responder's value on each evaluation deal.
fn best_response_values<G, S>(
    game: &G,
    strategy: &S,
    br: usize,
    fit: usize,
    eval: usize,
    seed: u64,
) -> Vec<f64>
where
    G: Game + Sync,
    G::State: Send + Sync,
    G::Info: Send + Sync,
    S: Fn(&G::Info, usize) -> Vec<f64> + Sync,
{
    let walk = Walk { game, strategy, br };
    let deal = |n: usize, salt: u64| -> (Vec<G::State>, Vec<Rng>) {
        let root = game.root();
        let mut states = Vec::with_capacity(n);
        let mut rngs = Vec::with_capacity(n);
        for i in 0..n {
            let mut rng = Rng::new(seed ^ salt ^ (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
            let mut s = root.clone();
            while game.turn(&s) == Turn::Chance {
                s = game.sample_chance(&s, &mut rng);
            }
            states.push(s);
            rngs.push(rng);
        }
        (states, rngs)
    };
    let (states, rngs) = deal(fit, 0x5eed_0001);
    let mut policy = HashMap::new();
    walk.fit(states, vec![1.0; fit], rngs, &mut policy);
    let (states, rngs) = deal(eval, 0x5eed_0002);
    // Each deal's value depends only on its own cards and stream, so the
    // evaluation splits into chunks.
    let chunk = eval.div_ceil(64).max(1);
    let pieces: Vec<(Vec<G::State>, Vec<Rng>)> = states
        .chunks(chunk)
        .zip(rngs.chunks(chunk))
        .map(|(s, r)| (s.to_vec(), r.to_vec()))
        .collect();
    let run = |(s, mut r): (Vec<G::State>, Vec<Rng>)| walk.eval(s, &mut r, &policy);
    #[cfg(feature = "parallel")]
    let parts: Vec<Vec<f64>> = {
        use rayon::prelude::*;
        pieces.into_par_iter().map(run).collect()
    };
    #[cfg(not(feature = "parallel"))]
    let parts: Vec<Vec<f64>> = pieces.into_iter().map(run).collect();
    parts.concat()
}
