//! Strategy profiles and what they're worth: expected value, best responses
//! and exploitability, computed exactly by walking the whole game tree.

use std::collections::HashMap;

use crate::game::{Game, Turn};

/// A mixed strategy for both players: for each information set, the
/// probability of each action. Information sets that are missing play
/// uniformly at random.
#[derive(Clone, Debug)]
pub struct Profile<I> {
    probs: HashMap<I, Vec<f64>>,
}

impl<I: Clone + Eq + std::hash::Hash> Profile<I> {
    pub fn new() -> Self {
        Self {
            probs: HashMap::new(),
        }
    }

    pub fn from_map(probs: HashMap<I, Vec<f64>>) -> Self {
        Self { probs }
    }

    /// Action probabilities at `info`, which has `n` actions.
    pub fn probs(&self, info: &I, n: usize) -> Vec<f64> {
        match self.probs.get(info) {
            Some(p) if p.len() == n => p.clone(),
            _ => vec![1.0 / n as f64; n],
        }
    }

    pub fn get(&self, info: &I) -> Option<&[f64]> {
        self.probs.get(info).map(Vec::as_slice)
    }

    pub fn set(&mut self, info: I, probs: Vec<f64>) {
        self.probs.insert(info, probs);
    }

    pub fn len(&self) -> usize {
        self.probs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.probs.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&I, &Vec<f64>)> {
        self.probs.iter()
    }
}

impl<I: Clone + Eq + std::hash::Hash> Default for Profile<I> {
    fn default() -> Self {
        Self::new()
    }
}

/// Player 0's expected payoff when both players follow `profile`.
pub fn expected_value<G: Game>(game: &G, profile: &Profile<G::Info>) -> f64 {
    fn ev<G: Game>(game: &G, profile: &Profile<G::Info>, s: &G::State) -> f64 {
        match game.turn(s) {
            Turn::Terminal => game.utility(s),
            Turn::Chance => game
                .chance_outcomes(s)
                .iter()
                .map(|(next, p)| p * ev(game, profile, next))
                .sum(),
            Turn::Player(_) => {
                let n = game.num_actions(s);
                let probs = profile.probs(&game.info(s), n);
                (0..n)
                    .filter(|&a| probs[a] > 0.0)
                    .map(|a| probs[a] * ev(game, profile, &game.apply(s, a)))
                    .sum()
            }
        }
    }
    ev(game, profile, &game.root())
}

/// What `player` wins on average with a best response to the other player's
/// strategy in `profile`, from `player`'s point of view.
///
/// A best response picks one action per information set (not per state), so
/// it can only use what that player knows. It's computed exactly: every state
/// of each of `player`'s information sets is weighted by how likely chance
/// and the opponent are to reach it.
pub fn best_response_value<G: Game>(game: &G, profile: &Profile<G::Info>, player: usize) -> f64 {
    let mut br = BestResponse {
        game,
        profile,
        player,
        members: HashMap::new(),
        choice: HashMap::new(),
    };
    br.collect(&game.root(), 1.0);
    let v = br.value(&game.root());
    if player == 0 { v } else { -v }
}

/// How much a best-responding opponent wins against `profile` on average,
/// over both seats: `(br_0 + br_1) / 2`. Zero exactly at a Nash equilibrium.
pub fn exploitability<G: Game>(game: &G, profile: &Profile<G::Info>) -> f64 {
    (best_response_value(game, profile, 0) + best_response_value(game, profile, 1)) / 2.0
}

struct BestResponse<'a, G: Game> {
    game: &'a G,
    profile: &'a Profile<G::Info>,
    player: usize,
    /// For each of `player`'s information sets: its states and how likely
    /// chance and the opponent are to reach each.
    members: HashMap<G::Info, Vec<(G::State, f64)>>,
    /// The best action at each information set, once worked out.
    choice: HashMap<G::Info, usize>,
}

impl<G: Game> BestResponse<'_, G> {
    fn collect(&mut self, s: &G::State, reach: f64) {
        match self.game.turn(s) {
            Turn::Terminal => {}
            Turn::Chance => {
                for (next, p) in self.game.chance_outcomes(s) {
                    self.collect(&next, reach * p);
                }
            }
            Turn::Player(p) => {
                let n = self.game.num_actions(s);
                if p == self.player {
                    self.members
                        .entry(self.game.info(s))
                        .or_default()
                        .push((s.clone(), reach));
                    for a in 0..n {
                        self.collect(&self.game.apply(s, a), reach);
                    }
                } else {
                    let probs = self.profile.probs(&self.game.info(s), n);
                    for (a, &q) in probs.iter().enumerate() {
                        if q > 0.0 {
                            self.collect(&self.game.apply(s, a), reach * q);
                        }
                    }
                }
            }
        }
    }

    /// Player 0's payoff from `s` with the best responder playing its choices
    /// and the opponent following the profile.
    fn value(&mut self, s: &G::State) -> f64 {
        match self.game.turn(s) {
            Turn::Terminal => self.game.utility(s),
            Turn::Chance => self
                .game
                .chance_outcomes(s)
                .iter()
                .map(|(next, p)| p * self.value(next))
                .sum(),
            Turn::Player(p) if p == self.player => {
                let a = self.best_action(&self.game.info(s));
                self.value(&self.game.apply(s, a))
            }
            Turn::Player(_) => {
                let n = self.game.num_actions(s);
                let probs = self.profile.probs(&self.game.info(s), n);
                (0..n)
                    .filter(|&a| probs[a] > 0.0)
                    .map(|a| probs[a] * self.value(&self.game.apply(s, a)))
                    .sum()
            }
        }
    }

    fn best_action(&mut self, info: &G::Info) -> usize {
        if let Some(&a) = self.choice.get(info) {
            return a;
        }
        let members = self.members.get(info).cloned().unwrap_or_default();
        let n = members.first().map_or(1, |(s, _)| self.game.num_actions(s));
        let sign = if self.player == 0 { 1.0 } else { -1.0 };
        let mut best = (0, f64::NEG_INFINITY);
        for a in 0..n {
            let v: f64 = members
                .iter()
                .map(|(s, w)| w * sign * self.value(&self.game.apply(s, a)))
                .sum();
            if v > best.1 {
                best = (a, v);
            }
        }
        self.choice.insert(info.clone(), best.0);
        best.0
    }
}
