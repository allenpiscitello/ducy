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
//!
//! With [`PloGtoBot::with_river_solving`], river decisions come from a
//! real-time solve over sampled ranges instead ([`river`](super::river)),
//! solved again when the opponent bets a size the solution doesn't have.
//!
//! It plays as many hole cards as its abstraction was built for: four for
//! PLO, six for PLO6 (#200). River solving is four-card only; with more
//! cards it plays the blueprint's river.

use std::sync::Arc;

use ducy_play::{Action, Bot, HandSummary, Observation, Street};
use rand::{SeedableRng, rngs::StdRng};

use super::{
    abstraction::PloAbstraction,
    river::{PloRiverSolver, PloRiverSolving, sample_range},
};
use crate::{
    holdem::{
        blueprint::{Blueprint, BlueprintError},
        bot::{fallback, real_action, realize, replays_to_me, same_kind, shove, street_root},
        cards::{Card, from_ducy, mask},
        follow::{Follower, street_index},
        hunl::{Betting, BettingTree, Buckets, Hunl, HunlAction, HunlConfig},
    },
    rng::Rng,
};

/// Loads a PLO blueprint trained for `config` with `cards`, for the number
/// of hole cards `cards` was built for, with the betting tree it was
/// trained on.
pub fn load_blueprint(
    config: &HunlConfig,
    cards: &PloAbstraction,
    bytes: &[u8],
) -> Result<(Blueprint, BettingTree), BlueprintError> {
    fn load<const H: usize>(
        config: &HunlConfig,
        cards: &PloAbstraction,
        bytes: &[u8],
    ) -> Result<(Blueprint, BettingTree), BlueprintError> {
        let game = Hunl::<_, H>::with_cards(config.clone(), Some(cards));
        let blueprint = Blueprint::load(bytes, &game, cards)?;
        Ok((blueprint, game.tree))
    }
    match cards.config.hole_cards {
        4 => load::<4>(config, cards, bytes),
        5 => load::<5>(config, cards, bytes),
        _ => load::<6>(config, cards, bytes),
    }
}

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
    /// The river solution for this hand.
    pub(crate) solved: Option<Box<PloRiverSolver>>,
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
    /// River solves run (re-solves included).
    pub river_solves: u32,
    fallback_rng: StdRng,
    river: Option<PloRiverSolving>,
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
        let (blueprint, tree) = load_blueprint(&config, &cards, blueprint_bytes)?;
        Ok(Self::from_parts(
            config,
            cards,
            Arc::new(blueprint),
            Arc::new(tree),
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
            river_solves: 0,
            fallback_rng: StdRng::seed_from_u64(seed ^ 0x5eed),
            river: None,
        }
    }

    /// Solves the river in real time instead of playing the blueprint there
    /// (with zero iterations, it keeps playing the blueprint).
    pub fn with_river_solving(mut self, solving: PloRiverSolving) -> Self {
        self.river = (solving.iterations > 0).then_some(solving);
        self
    }

    /// A river decision from the solved subgame: solved now if there's no
    /// solution yet or the real river left its tree. `None` to play the
    /// blueprint instead.
    fn river_action(&mut self, obs: &Observation) -> Option<Action> {
        // River solving samples four-card ranges.
        if obs.seats.len() != 2
            || obs.hole_cards.num_cards() != 4
            || self.cards.config.hole_cards != 4
        {
            return None;
        }
        self.follow(obs);
        let root = street_root(obs, 3)?;
        let path = self.track.follow.river_path.clone();
        if !replays_to_me(&root, &path, self.track.me) {
            return None;
        }
        let on_tree = |s: &PloRiverSolver| s.tree.follow(&path).is_some();
        if !self.track.solved.as_deref().is_some_and(on_tree) {
            let solver = self.solve_river(obs, &root, &path)?;
            self.track.solved = Some(Box::new(solver));
            self.river_solves += 1;
        }
        let solver = self.track.solved.as_deref()?;
        let node = solver.tree.follow(&path)?;
        // Our own hand is hand 0 of our sampled range.
        let probs = solver.probs(node, 0);
        let a = solver.tree.nodes[node as usize].actions[self.rng.sample(&probs)];
        self.track.follow.pending = None;
        Some(real_action(a, obs))
    }

    /// Solves the river from `root` with the real actions `path` forced into
    /// the tree, over both players' sampled ranges, with the gadget against
    /// what the opponent's hands get from the blueprint's river.
    fn solve_river(
        &mut self,
        obs: &Observation,
        root: &Betting,
        path: &[HunlAction],
    ) -> Option<PloRiverSolver> {
        let settings = self.river.clone()?;
        let bp_root = self.track.follow.river_root?;
        let bp_node = &self.tree.nodes[bp_root as usize];
        if bp_node.actions.is_empty() || bp_node.betting.street != 3 {
            return None;
        }
        let board: Vec<Card> = obs.board.iter().copied().map(from_ducy).collect();
        let board: [Card; 5] = board.try_into().ok()?;
        let hole: Vec<Card> = obs.hole_cards.iter(false).map(from_ducy).collect();
        let hole: [Card; 4] = hole.try_into().ok()?;
        let me = self.track.me;
        let steps = self.track.follow.steps_before_river(&self.tree);
        let mut rng = Rng::new(self.rng.next_u64());
        let mut range = |player: usize, blocked: u64, first: Option<[Card; 4]>| {
            sample_range(
                &self.cards,
                &self.blueprint,
                &self.tree,
                &steps,
                player,
                &board,
                blocked,
                settings.hands,
                first,
                &mut rng,
            )
        };
        // Ours is public (only the board is known to the opponent), with our
        // real hand first; theirs can't hold our cards.
        let mine = range(me, 0, Some(hole));
        let theirs = range(1 - me, mask(&hole), None);
        let ranges = if me == 0 {
            [mine, theirs]
        } else {
            [theirs, mine]
        };
        let mut config = self.config.clone();
        config.big_blind = obs.rules.big_blind;
        let mut solver = PloRiverSolver::new(&board, root, &config, path, ranges);
        let reference =
            solver.blueprint_strategy(&self.cards, &board, &self.blueprint, &self.tree, bp_root);
        let target = solver.best_response(1 - me, &reference);
        solver.set_gadget(1 - me, target);
        solver.run(settings.iterations, settings.time);
        Some(solver)
    }

    /// The game the blueprint was trained for.
    pub fn config(&self) -> &HunlConfig {
        &self.config
    }

    /// The blueprint's action probabilities at this decision, with the
    /// actions they belong to, or `None` when the hand can't be followed on
    /// the tree.
    pub fn strategy(&mut self, obs: &Observation) -> Option<(Vec<HunlAction>, Vec<f64>)> {
        if obs.seats.len() != 2
            || obs.hole_cards.num_cards() as usize != self.cards.config.hole_cards
        {
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
        if self.river.is_some()
            && obs.street == Street::River
            && let Some(action) = self.river_action(obs)
        {
            return Some(action);
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
