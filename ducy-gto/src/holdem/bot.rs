//! `GtoBot`: plays a trained blueprint in a real ducy-play hand.
//!
//! The real game allows bets of any size, the blueprint knows only its menu.
//! At each decision the bot replays the hand's betting so far on the
//! blueprint's betting tree:
//!
//! - **Its own actions** are on the tree already.
//! - **The opponent's actions** are mapped to the nearest menu sizes with the
//!   randomized *pseudo-harmonic mapping* (see [`follow`](super::follow)),
//!   drawn once per hand and remembered, so later decisions see a consistent
//!   history.
//!
//! It then looks up its bucket for the current street, **samples** an action
//! from the blueprint's mixed strategy, and converts that action's size back
//! to chips as the same fraction of the real pot, clamped to what's legal.
//!
//! With [`GtoBot::with_river_solving`], river decisions come from solving
//! the river in real time instead (see [`river`](super::river)): both
//! players' ranges are tracked through the hand, the river is solved from
//! the real pot and stacks with the resolving gadget, and the solution is
//! kept for the rest of the hand, solved again when the opponent bets a
//! size it doesn't have.
//!
//! With [`GtoBot::with_turn_solving`], turn decisions likewise come from a
//! depth-limited solve of the turn (see [`turn`](super::turn)): the turn's
//! betting is solved from the real chips, with the rivers played out by the
//! blueprint (or, for the opponent, the blueprint biased one of several
//! ways). With river solving on as well, the river's ranges then follow the
//! turn solution rather than the blueprint.
//!
//! The blueprint is for heads-up play. At a table with more players, or if
//! the hand ever can't be followed on the tree, the bot falls back to a
//! simple pot-odds rule rather than ever returning an illegal action.

use std::{sync::Arc, time::Duration};

use ducy_play::{Action, Bot, HandSummary, LegalActions, Observation, Street};
use rand::{SeedableRng, rngs::StdRng};

use super::{
    abstraction::CardAbstraction,
    blueprint::{Blueprint, BlueprintError},
    cards::{Card, from_ducy, hole_index},
    follow::{Follower, street_index},
    hunl::{Betting, BettingTree, Hunl, HunlAction, HunlConfig, pot_fraction},
    range::{BucketCache, Range, replay},
    river::RiverSolver,
    turn::{Bias, TurnSolver},
};
use crate::rng::Rng;

pub use super::follow::pseudo_harmonic;

/// How a hand is being followed on the tree.
#[derive(Clone, Debug, Default)]
struct Track {
    /// Identifies the hand: our hole cards and the button.
    hand: Option<(u64, usize)>,
    /// Real events consumed so far.
    events: usize,
    /// Our side: 0 is the button.
    me: usize,
    follow: Follower,
    /// The turn solution for this hand.
    turn_solved: Option<Box<TurnSolver>>,
    /// The river solution for this hand.
    solved: Option<Box<RiverSolver>>,
}

/// How hard to solve the river.
#[derive(Clone, Debug, PartialEq)]
pub struct RiverSolving {
    /// Iterations per solve.
    pub iterations: usize,
    /// Stop early after this long (not on WebAssembly, which has no clock).
    pub time: Option<Duration>,
    /// The river bet menu (`menu.postflop`); blinds come from the table.
    pub config: HunlConfig,
}

impl RiverSolving {
    /// `iterations` per solve, the blueprint's own postflop menu.
    pub fn new(iterations: usize) -> Self {
        Self {
            iterations,
            time: None,
            config: HunlConfig::default(),
        }
    }
}

/// How hard to solve the turn.
#[derive(Clone, Debug, PartialEq)]
pub struct TurnSolving {
    /// Iterations per solve.
    pub iterations: usize,
    /// Stop early after this long (not on WebAssembly, which has no clock).
    pub time: Option<Duration>,
    /// The turn and river bet menu (`menu.postflop`); blinds come from the
    /// table.
    pub config: HunlConfig,
    /// The river continuations the opponent may pick from at each leaf;
    /// empty to have both play the blueprint's.
    pub biases: Vec<Bias>,
    /// River cards dealt per iteration when valuing leaves (0 for all 48;
    /// see [`TurnSolver::set_river_samples`]).
    pub river_samples: usize,
}

impl TurnSolving {
    /// `iterations` per solve, the blueprint's own postflop menu, the
    /// opponent choosing among all four continuations, and 8 rivers dealt
    /// per iteration.
    pub fn new(iterations: usize) -> Self {
        Self {
            iterations,
            time: None,
            config: HunlConfig::default(),
            biases: Bias::ALL.to_vec(),
            river_samples: 8,
        }
    }
}

impl Default for RiverSolving {
    /// 200 iterations, as measured (see the crate README).
    fn default() -> Self {
        Self::new(200)
    }
}

impl Default for TurnSolving {
    /// 50 iterations: with 200 on the river, LBR wins 197 ± 97 mbb/hand less
    /// against it than against river solving alone (70,000 paired hands).
    /// About 3–4 s a turn decision on 12 cores.
    fn default() -> Self {
        Self::new(50)
    }
}

/// A bot that plays a heads-up blueprint.
pub struct GtoBot {
    cards: Arc<CardAbstraction>,
    blueprint: Arc<Blueprint>,
    tree: Arc<BettingTree>,
    rng: Rng,
    track: Track,
    /// Decisions where it had to use the fallback rule.
    pub off_tree: u32,
    /// River solves run (re-solves included).
    pub river_solves: u32,
    /// Turn solves run (re-solves included).
    pub turn_solves: u32,
    fallback_rng: StdRng,
    river: Option<RiverSolving>,
    turn: Option<TurnSolving>,
    /// Every hand's buckets on recent boards, for tracking ranges.
    buckets: BucketCache,
}

impl GtoBot {
    /// A bot playing `blueprint`, trained for `config` with `cards`. Loading
    /// checks they match.
    pub fn new(
        config: HunlConfig,
        cards: Arc<CardAbstraction>,
        blueprint_bytes: &[u8],
        seed: u64,
    ) -> Result<Self, BlueprintError> {
        let game = Hunl::new(config.clone(), Some(&cards));
        let blueprint = Blueprint::load(blueprint_bytes, &game, &cards)?;
        let tree = Arc::new(game.tree);
        Ok(Self::from_parts(cards, Arc::new(blueprint), tree, seed))
    }

    /// A bot sharing an already-loaded blueprint and tree (cheap to make
    /// many).
    pub fn from_parts(
        cards: Arc<CardAbstraction>,
        blueprint: Arc<Blueprint>,
        tree: Arc<BettingTree>,
        seed: u64,
    ) -> Self {
        Self {
            cards,
            blueprint,
            tree,
            rng: Rng::new(seed),
            track: Track::default(),
            off_tree: 0,
            river_solves: 0,
            turn_solves: 0,
            fallback_rng: StdRng::seed_from_u64(seed ^ 0x5eed),
            river: None,
            turn: None,
            buckets: BucketCache::default(),
        }
    }

    /// Solves the river in real time instead of playing the blueprint there
    /// (with zero iterations, it keeps playing the blueprint). Each solve
    /// first works out both ranges, which takes every hand's bucket on each
    /// street: instant with the full tables, about 0.2 s with a compact
    /// abstraction (see [`CardAbstraction::buckets`]).
    pub fn with_river_solving(mut self, solving: RiverSolving) -> Self {
        self.river = (solving.iterations > 0).then_some(solving);
        self
    }

    /// Solves the turn in real time, depth-limited, instead of playing the
    /// blueprint there (with zero iterations, it keeps playing the
    /// blueprint). Each solve plays out all 48 rivers at every leaf of the
    /// turn's betting, for each continuation the opponent may pick, so it
    /// costs far more per iteration than a river solve.
    pub fn with_turn_solving(mut self, solving: TurnSolving) -> Self {
        self.turn = (solving.iterations > 0).then_some(solving);
        self
    }

    /// Turn and river solving at their defaults: the strongest setup
    /// measured, and how the bot should normally play.
    pub fn with_solving(self) -> Self {
        self.with_turn_solving(TurnSolving::default())
            .with_river_solving(RiverSolving::default())
    }

    /// The blueprint's action probabilities at this decision, with the
    /// actions they belong to, or `None` when the hand can't be followed on
    /// the tree.
    pub fn strategy(&mut self, obs: &Observation) -> Option<(Vec<HunlAction>, Vec<f64>)> {
        if obs.seats.len() != 2 {
            return None;
        }
        self.follow(obs);
        let node = self.track.follow.node?;
        let n = &self.tree.nodes[node as usize];
        let street = street_index(obs.street);
        if n.actions.is_empty() || n.betting.street != street || n.betting.to_act != self.track.me {
            return None;
        }
        let hole: Vec<Card> = obs.hole_cards.iter(false).map(from_ducy).collect();
        let board: Vec<Card> = obs.board.iter().copied().map(from_ducy).collect();
        let bucket = self.cards.bucket([hole[0], hole[1]], &board);
        Some((n.actions.clone(), self.blueprint.probs(node, bucket)))
    }

    /// Whether the tree has both players all-in while the real hand still
    /// has chips behind (a large real bet was mapped to all-in). The tree's
    /// plan is then to get every chip in, so the bot does.
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
    fn follow(&mut self, obs: &Observation) {
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
        // Seat numbers to tree players: the button is player 0.
        let side = |seat: usize| if seat == button { 0 } else { 1 };
        while self.track.events < obs.history.len() {
            let e = &obs.history[self.track.events];
            self.track.events += 1;
            self.track.follow.apply(&self.tree, e, side, &mut self.rng);
        }
    }

    /// A river decision from the solved subgame: solved now if there's no
    /// solution yet or the real river left its tree. `None` to fall back to
    /// the blueprint.
    fn river_action(&mut self, obs: &Observation) -> Option<Action> {
        if obs.seats.len() != 2 {
            return None;
        }
        self.follow(obs);
        let root = street_root(obs, 3)?;
        if !replays_to_me(&root, &self.track.follow.river_path, self.track.me) {
            return None;
        }
        let path = self.track.follow.river_path.clone();
        let on_tree = |s: &RiverSolver| s.tree.follow(&path).is_some();
        if !self.track.solved.as_deref().is_some_and(on_tree) {
            let solver = self.solve_river(obs, &root, &path)?;
            self.track.solved = Some(Box::new(solver));
            self.river_solves += 1;
        }
        let solver = self.track.solved.as_deref()?;
        let node = solver.tree.follow(&path)?;
        let hole: Vec<Card> = obs.hole_cards.iter(false).map(from_ducy).collect();
        let probs = solver.probs(node, hole_index(hole[0], hole[1]))?;
        let a = solver.tree.nodes[node as usize].actions[self.rng.sample(&probs)];
        self.track.follow.pending = None;
        Some(real_action(a, obs))
    }

    /// Solves the river from `root` with the real actions `path` in the
    /// tree, the bot's own earlier river actions frozen at the previous
    /// solution, and the gadget against the blueprint's river values.
    fn solve_river(
        &mut self,
        obs: &Observation,
        root: &Betting,
        path: &[HunlAction],
    ) -> Option<RiverSolver> {
        let settings = self.river.clone()?;
        let bp_root = self.track.follow.river_root?;
        let bp_node = &self.tree.nodes[bp_root as usize];
        if bp_node.actions.is_empty() || bp_node.betting.street != 3 {
            return None;
        }
        let board: Vec<Card> = obs.board.iter().copied().map(from_ducy).collect();
        let board: [Card; 5] = board.try_into().ok()?;
        let ranges = self.river_ranges(&board);
        if ranges.iter().any(|r| r.total() <= 0.0) {
            return None;
        }
        let mut config = settings.config.clone();
        config.big_blind = obs.rules.big_blind;
        let mut solver = RiverSolver::new(
            board,
            root,
            &config,
            path,
            [&ranges[0].weight, &ranges[1].weight],
        );
        let me = self.track.me;
        let buckets = self.buckets.get(&self.cards, &board).to_vec();
        let reference = solver.blueprint_strategy(&self.blueprint, &self.tree, bp_root, &buckets);
        let target = solver.best_response(1 - me, &reference);
        solver.set_gadget(1 - me, target);
        if let Some(prev) = self.track.solved.as_deref() {
            for (k, (node, _)) in solver.tree.along(path)?.into_iter().enumerate() {
                let n = &solver.tree.nodes[node as usize];
                if n.betting.to_act != me {
                    continue;
                }
                if let Some(old) = prev.tree.follow(&path[..k])
                    && prev.tree.nodes[old as usize].actions == n.actions
                {
                    solver.freeze(node, prev.average_at(old));
                }
            }
        }
        run_budget(settings.iterations, settings.time, || solver.iterate());
        Some(solver)
    }

    /// The blueprint steps before `street`, for replaying ranges.
    fn steps_before(&self, street: usize) -> Vec<(u32, usize)> {
        self.track.follow.steps_before(&self.tree, street)
    }

    /// Both ranges at the start of the river: as the blueprint played up to
    /// the turn, then as the turn solution played the turn if there is one
    /// that has the real turn's actions, else as the blueprint did.
    fn river_ranges(&mut self, board: &[Card; 5]) -> [Range; 2] {
        let solved = self.track.turn_solved.as_deref();
        let along = solved.and_then(|s| s.tree.along(&self.track.follow.turn_path));
        let steps = self.steps_before(if along.is_some() { 2 } else { 3 });
        let mut ranges = replay(
            &self.cards,
            &self.blueprint,
            &self.tree,
            &steps,
            board,
            &mut self.buckets,
        );
        let turn = self.track.turn_solved.as_deref().zip(along);
        if let Some((s, along)) = turn {
            for (node, a) in along {
                let p = s.tree.nodes[node as usize].betting.to_act;
                ranges[p].update_by(|h| s.probs(node, h).map_or(0.0, |x| x[a]));
            }
        }
        ranges
    }

    /// A turn decision from the depth-limited solve: solved now if there's
    /// no solution yet or the real turn left its tree. `None` to fall back
    /// to the blueprint.
    fn turn_action(&mut self, obs: &Observation) -> Option<Action> {
        if obs.seats.len() != 2 {
            return None;
        }
        self.follow(obs);
        let root = street_root(obs, 2)?;
        if !replays_to_me(&root, &self.track.follow.turn_path, self.track.me) {
            return None;
        }
        let path = self.track.follow.turn_path.clone();
        let on_tree = |s: &TurnSolver| s.tree.follow(&path).is_some();
        if !self.track.turn_solved.as_deref().is_some_and(on_tree) {
            let solver = self.solve_turn(obs, &root, &path)?;
            self.track.turn_solved = Some(Box::new(solver));
            self.turn_solves += 1;
        }
        let solver = self.track.turn_solved.as_deref()?;
        let node = solver.tree.follow(&path)?;
        let hole: Vec<Card> = obs.hole_cards.iter(false).map(from_ducy).collect();
        let probs = solver.probs(node, hole_index(hole[0], hole[1]))?;
        let a = solver.tree.nodes[node as usize].actions[self.rng.sample(&probs)];
        self.track.follow.pending = None;
        Some(real_action(a, obs))
    }

    /// Solves the turn from `root` with the real actions `path` in the
    /// tree, the bot's own earlier turn actions frozen at the previous
    /// solution, the opponent picking river continuations, and the gadget
    /// against the blueprint's turn values.
    fn solve_turn(
        &mut self,
        obs: &Observation,
        root: &Betting,
        path: &[HunlAction],
    ) -> Option<TurnSolver> {
        let settings = self.turn.clone()?;
        let bp_root = self.track.follow.turn_root?;
        let bp_node = &self.tree.nodes[bp_root as usize];
        if bp_node.actions.is_empty() || bp_node.betting.street != 2 {
            return None;
        }
        let board: Vec<Card> = obs.board.iter().copied().map(from_ducy).collect();
        let board: [Card; 4] = board.try_into().ok()?;
        let ranges = replay(
            &self.cards,
            &self.blueprint,
            &self.tree,
            &self.steps_before(2),
            &board,
            &mut self.buckets,
        );
        if ranges.iter().any(|r| r.total() <= 0.0) {
            return None;
        }
        let mut config = settings.config.clone();
        config.big_blind = obs.rules.big_blind;
        let mut solver = TurnSolver::from_blueprint(
            board,
            root,
            &config,
            path,
            [&ranges[0].weight, &ranges[1].weight],
            &self.cards,
            &self.blueprint,
            &self.tree,
            bp_root,
        );
        let me = self.track.me;
        if !settings.biases.is_empty() {
            solver.set_chooser(1 - me, &settings.biases);
        }
        solver.set_river_samples(settings.river_samples, self.rng.next_u64());
        let buckets = self.buckets.get(&self.cards, &board).to_vec();
        let reference = solver.blueprint_strategy(&self.blueprint, &self.tree, bp_root, &buckets);
        let target = solver.best_response(1 - me, &reference);
        solver.set_gadget(1 - me, target);
        if let Some(prev) = self.track.turn_solved.as_deref() {
            for (k, (node, _)) in solver.tree.along(path)?.into_iter().enumerate() {
                let n = &solver.tree.nodes[node as usize];
                if n.betting.to_act != me {
                    continue;
                }
                if let Some(old) = prev.tree.follow(&path[..k])
                    && prev.tree.nodes[old as usize].actions == n.actions
                {
                    solver.freeze(node, prev.average_at(old));
                }
            }
        }
        run_budget(settings.iterations, settings.time, || solver.iterate());
        Some(solver)
    }

    /// Converts an abstract action at `node` to a legal real one.
    fn realize(&self, node: u32, a: HunlAction, obs: &Observation) -> Action {
        let legal = &obs.legal;
        let check_or_call = if legal.can_check {
            Action::Check
        } else {
            Action::Call
        };
        match a {
            HunlAction::Fold if legal.can_fold => Action::Fold,
            HunlAction::Fold | HunlAction::Check | HunlAction::Call => check_or_call,
            HunlAction::Bet(to) | HunlAction::Raise(to) => {
                let Some(range) = legal.bet.or(legal.raise) else {
                    return check_or_call;
                };
                let b = &self.tree.nodes[node as usize].betting;
                if to >= b.all_in_to() {
                    return if legal.bet.is_some() {
                        Action::Bet(range.max_to)
                    } else {
                        Action::Raise(range.max_to)
                    };
                }
                let frac = pot_fraction(to, b.current_bet, b.to_call(), b.pot());
                let me = &obs.seats[obs.seat];
                let to_call = obs.current_bet.saturating_sub(me.street_bet);
                let real = obs.current_bet + (frac * (obs.pot + to_call) as f64).round() as u64;
                let real = real.clamp(range.min_to, range.max_to);
                if legal.bet.is_some() {
                    Action::Bet(real)
                } else {
                    Action::Raise(real)
                }
            }
        }
    }

    /// Off the tree: check when free, otherwise call with enough equity
    /// against a random hand for the price.
    fn fallback(&mut self, obs: &Observation) -> Action {
        self.off_tree += 1;
        let legal: &LegalActions = &obs.legal;
        if legal.can_check {
            return Action::Check;
        }
        let call = legal.call.unwrap_or(0);
        let equity = ducy_play::strength::observation_equity(obs, 200, &mut self.fallback_rng);
        if equity * (obs.pot + call) as f64 >= call as f64 {
            Action::Call
        } else {
            Action::Fold
        }
    }
}

/// Runs `iterate` for the iterations allowed, stopping early once `time`
/// has passed.
fn run_budget(iterations: usize, time: Option<Duration>, mut iterate: impl FnMut()) {
    let start = time.map(|_| std::time::Instant::now());
    for _ in 0..iterations {
        iterate();
        if let (Some(s), Some(limit)) = (start, time)
            && s.elapsed() >= limit
        {
            break;
        }
    }
}

/// Whether the real actions `path` replay legally from `root` to a decision
/// of player `me`.
fn replays_to_me(root: &Betting, path: &[HunlAction], me: usize) -> bool {
    let mut b = root.clone();
    for &a in path {
        if b.is_over() || b.street != root.street {
            return false;
        }
        b = b.play(a);
    }
    !b.is_over() && b.street == root.street && b.to_act == me
}

/// The real hand's betting at the start of `street` (2 the turn, 3 the
/// river; player 0 the button).
fn street_root(obs: &Observation, street: usize) -> Option<Betting> {
    let seat = |p: usize| if p == 0 { obs.button } else { 1 - obs.button };
    let mut stack = [0; 2];
    let mut contributed = [0; 2];
    for p in 0..2 {
        let s = obs.seats.get(seat(p))?;
        stack[p] = s.stack + s.street_bet;
        contributed[p] = s.contributed - s.street_bet;
    }
    Some(Betting::street_start(
        street,
        stack,
        contributed,
        obs.rules.big_blind,
    ))
}

/// A subgame action in real chips, clamped to what's legal.
fn real_action(a: HunlAction, obs: &Observation) -> Action {
    let legal = &obs.legal;
    let check_or_call = if legal.can_check {
        Action::Check
    } else {
        Action::Call
    };
    match a {
        HunlAction::Fold if legal.can_fold => Action::Fold,
        HunlAction::Fold | HunlAction::Check | HunlAction::Call => check_or_call,
        HunlAction::Bet(to) | HunlAction::Raise(to) => match (legal.bet, legal.raise) {
            (Some(r), _) => Action::Bet(to.clamp(r.min_to, r.max_to)),
            (None, Some(r)) => Action::Raise(to.clamp(r.min_to, r.max_to)),
            (None, None) => check_or_call,
        },
    }
}

impl Bot for GtoBot {
    fn act(&mut self, obs: &Observation) -> Option<Action> {
        if self.committed(obs) {
            return Some(match obs.legal.bet.or(obs.legal.raise) {
                Some(r) if obs.legal.bet.is_some() => Action::Bet(r.max_to),
                Some(r) => Action::Raise(r.max_to),
                None if obs.legal.can_check => Action::Check,
                None => Action::Call,
            });
        }
        if self.turn.is_some()
            && obs.street == Street::Turn
            && let Some(action) = self.turn_action(obs)
        {
            return Some(action);
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
                let action = self.realize(node, a, obs);
                // When the real action is the same kind as the tree's, follow
                // the tree's choice exactly once its event arrives.
                let same = matches!(
                    (a, action),
                    (HunlAction::Fold, Action::Fold)
                        | (
                            HunlAction::Check | HunlAction::Call,
                            Action::Check | Action::Call
                        )
                        | (
                            HunlAction::Bet(_) | HunlAction::Raise(_),
                            Action::Bet(_) | Action::Raise(_)
                        )
                );
                let me = self.track.me;
                self.track.follow.pending = same.then_some((me, i));
                Some(action)
            }
            None => Some(self.fallback(obs)),
        }
    }

    fn hand_over(&mut self, _summary: &HandSummary) {
        self.track = Track::default();
    }
}
