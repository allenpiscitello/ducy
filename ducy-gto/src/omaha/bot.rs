//! `PloGtoBot`: plays a trained heads-up PLO blueprint in a real ducy-play
//! hand (#131).
//!
//! It works like Hold'em's [`GtoBot`](crate::holdem::bot::GtoBot):
//!
//! - **Following the hand:** it replays the betting on the blueprint's
//!   tree. The opponent's bets go to the neighbouring menu sizes with the
//!   randomized pseudo-harmonic mapping, drawn once per hand
//!   ([`follow`](crate::holdem::follow)).
//! - **Choosing an action:** it looks up its bucket in the
//!   [`PloAbstraction`], samples the blueprint's mixed strategy, and turns
//!   the chosen size into chips as the same fraction of the real pot,
//!   clamped to what's legal. Under pot-limit, the tree's largest bet is
//!   the real pot-sized bet.
//! - **When the tree is all-in early:** it gets every chip in.
//! - **Anything it can't follow:** more than two players, or a hand off the
//!   tree, gets a pot-odds fallback that never acts illegally (counted in
//!   `off_tree`).

use std::sync::Arc;

use ducy_play::{Action, Bot, HandSummary, Observation};
use rand::{SeedableRng, rngs::StdRng};

use super::abstraction::PloAbstraction;
use crate::{
    holdem::{
        blueprint::{Blueprint, BlueprintError},
        bot::{fallback, realize, same_kind, shove},
        cards::{Card, from_ducy},
        follow::{Follower, street_index},
        hunl::{BettingTree, Buckets, HuPlo, HunlAction, HunlConfig},
    },
    rng::Rng,
};

/// How a hand is being followed on the tree.
#[derive(Clone, Debug, Default)]
pub(crate) struct Track {
    /// Identifies the hand: our hole cards and the button.
    pub(crate) hand: Option<(u64, usize)>,
    /// Real events consumed so far.
    pub(crate) events: usize,
    /// Our side: 0 is the button.
    pub(crate) me: usize,
    pub(crate) follow: Follower,
}

/// A bot that plays a heads-up PLO blueprint.
pub struct PloGtoBot {
    pub(crate) cards: Arc<PloAbstraction>,
    pub(crate) blueprint: Arc<Blueprint>,
    pub(crate) tree: Arc<BettingTree>,
    pub(crate) config: HunlConfig,
    pub(crate) rng: Rng,
    pub(crate) track: Track,
    /// Decisions where it had to use the fallback rule.
    pub off_tree: u32,
    fallback_rng: StdRng,
}

impl PloGtoBot {
    /// A bot playing `blueprint`, trained for `config` with `cards`. Loading
    /// checks they match.
    pub fn new(
        config: HunlConfig,
        cards: Arc<PloAbstraction>,
        blueprint_bytes: &[u8],
        seed: u64,
    ) -> Result<Self, BlueprintError> {
        let game = HuPlo::with_cards(config.clone(), Some(&*cards));
        let blueprint = Blueprint::load(blueprint_bytes, &game, &*cards)?;
        let tree = Arc::new(game.tree);
        Ok(Self::from_parts(
            config,
            cards,
            Arc::new(blueprint),
            tree,
            seed,
        ))
    }

    /// A bot sharing an already-loaded blueprint and tree (cheap to make
    /// many).
    pub fn from_parts(
        config: HunlConfig,
        cards: Arc<PloAbstraction>,
        blueprint: Arc<Blueprint>,
        tree: Arc<BettingTree>,
        seed: u64,
    ) -> Self {
        Self {
            cards,
            blueprint,
            tree,
            config,
            rng: Rng::new(seed),
            track: Track::default(),
            off_tree: 0,
            fallback_rng: StdRng::seed_from_u64(seed ^ 0x5eed),
        }
    }

    /// The game the blueprint was trained for.
    pub fn config(&self) -> &HunlConfig {
        &self.config
    }

    /// The blueprint's action probabilities at this decision, with the
    /// actions they belong to, or `None` when the hand can't be followed on
    /// the tree.
    pub fn strategy(&mut self, obs: &Observation) -> Option<(Vec<HunlAction>, Vec<f64>)> {
        if obs.seats.len() != 2 || obs.hole_cards.num_cards() != 4 {
            return None;
        }
        self.follow(obs);
        let node = self.track.follow.node?;
        let n = &self.tree.nodes[node as usize];
        if n.actions.is_empty()
            || n.betting.street != street_index(obs.street)
            || n.betting.to_act != self.track.me
        {
            return None;
        }
        let hole: Vec<Card> = obs.hole_cards.iter(false).map(from_ducy).collect();
        let board: Vec<Card> = obs.board.iter().copied().map(from_ducy).collect();
        let bucket = self.cards.hand_bucket(&hole, &board);
        Some((n.actions.clone(), self.blueprint.probs(node, bucket)))
    }

    /// Whether the tree has both players all-in while the real hand still
    /// has chips behind.
    fn committed(&mut self, obs: &Observation) -> bool {
        if obs.seats.len() != 2 {
            return false;
        }
        self.follow(obs);
        self.track.follow.node.is_some_and(|node| {
            let b = &self.tree.nodes[node as usize].betting;
            b.is_over() && b.folded.is_none() && b.stack == [0, 0]
        })
    }

    /// Brings the tracked node up to date with the real history.
    pub(crate) fn follow(&mut self, obs: &Observation) {
        let hole = u64::from(obs.hole_cards);
        let key = (hole, obs.button);
        if self.track.hand != Some(key) || obs.history.len() < self.track.events {
            self.track = Track {
                hand: Some(key),
                me: if obs.seat == obs.button { 0 } else { 1 },
                ..Track::default()
            };
        }
        let button = obs.button;
        let side = |seat: usize| if seat == button { 0 } else { 1 };
        while self.track.events < obs.history.len() {
            let e = &obs.history[self.track.events];
            self.track.events += 1;
            self.track.follow.apply(&self.tree, e, side, &mut self.rng);
        }
    }
}

impl Bot for PloGtoBot {
    fn act(&mut self, obs: &Observation) -> Option<Action> {
        if self.committed(obs) {
            return Some(shove(obs));
        }
        match self.strategy(obs) {
            Some((actions, probs)) => {
                let node = self.track.follow.node.expect("strategy found a node");
                let i = self.rng.sample(&probs);
                let a = actions[i];
                let action = realize(&self.tree, node, a, obs);
                let me = self.track.me;
                self.track.follow.pending = same_kind(a, action).then_some((me, i));
                Some(action)
            }
            None => {
                self.off_tree += 1;
                Some(fallback(obs, &mut self.fallback_rng))
            }
        }
    }

    fn hand_over(&mut self, _summary: &HandSummary) {
        self.track = Track::default();
    }
}
