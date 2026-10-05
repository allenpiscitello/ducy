//! `GtoBot`: plays a trained blueprint in a real ducy-play hand.
//!
//! The real game allows bets of any size, the blueprint knows only its menu.
//! At each decision the bot replays the hand's betting so far on the
//! blueprint's betting tree:
//!
//! - **Its own actions** are on the tree already.
//! - **The opponent's actions** are mapped to the nearest menu sizes with the
//!   randomized *pseudo-harmonic mapping* (Ganzfried and Sandholm 2013). A
//!   bet of `x` pot between menu sizes `a < x < b` maps to `a` with
//!   probability `(b - x)(1 + a) / ((b - a)(1 + x))` and to `b` otherwise.
//!   Each translation is drawn once per hand and remembered, so later
//!   decisions see a consistent history.
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
//! The blueprint is for heads-up play. At a table with more players, or if
//! the hand ever can't be followed on the tree, the bot falls back to a
//! simple pot-odds rule rather than ever returning an illegal action.

use std::{sync::Arc, time::Duration};

use ducy_play::{Action, Bot, Event, HandSummary, LegalActions, Observation, Street};
use rand::{SeedableRng, rngs::StdRng};

use super::{
    abstraction::CardAbstraction,
    blueprint::{Blueprint, BlueprintError},
    cards::{Card, from_ducy, hole_index},
    hunl::{Betting, BettingTree, Hunl, HunlAction, HunlConfig, pot_fraction},
    range::replay,
    river::RiverSolver,
};
use crate::rng::Rng;

/// Probability of mapping a bet of `x` (as a fraction of the pot) to the
/// smaller of two menu sizes `a < x < b`.
pub fn pseudo_harmonic(a: f64, b: f64, x: f64) -> f64 {
    if x <= a {
        return 1.0;
    }
    if x >= b {
        return 0.0;
    }
    ((b - x) * (1.0 + a)) / ((b - a) * (1.0 + x))
}

/// The real hand's chips, tracked from its events.
#[derive(Clone, Copy, Debug, Default)]
struct RealBetting {
    street_bet: [u64; 2],
    contributed: [u64; 2],
    current_bet: u64,
}

impl RealBetting {
    fn pot(&self) -> u64 {
        self.contributed[0] + self.contributed[1]
    }
}

/// How a hand is being followed on the tree.
#[derive(Clone, Debug, Default)]
struct Track {
    /// Identifies the hand: our hole cards and the button.
    hand: Option<(u64, usize)>,
    /// Real events consumed so far.
    events: usize,
    /// Current node in the betting tree, or `None` once lost.
    node: Option<u32>,
    real: RealBetting,
    street: usize,
    /// Our side: 0 is the button.
    me: usize,
    /// The action index of our own last action, applied when its event
    /// shows up instead of translating our own bet back.
    pending: Option<usize>,
    /// Every step taken on the tree: the node and the action's index.
    steps: Vec<(u32, usize)>,
    /// The tree node the river starts at.
    river_root: Option<u32>,
    /// The real river actions so far, in chips.
    river_path: Vec<HunlAction>,
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
    fallback_rng: StdRng,
    river: Option<RiverSolving>,
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
            fallback_rng: StdRng::seed_from_u64(seed ^ 0x5eed),
            river: None,
        }
    }

    /// Solves the river in real time instead of playing the blueprint there.
    /// Needs a card abstraction whose buckets are quick to compute for every
    /// hand (see [`CardAbstraction::fast_buckets`]); otherwise, or with zero
    /// iterations, the bot plays the blueprint on the river too.
    pub fn with_river_solving(mut self, solving: RiverSolving) -> Self {
        self.river = (solving.iterations > 0 && self.cards.fast_buckets()).then_some(solving);
        self
    }

    /// The blueprint's action probabilities at this decision, with the
    /// actions they belong to, or `None` when the hand can't be followed on
    /// the tree.
    pub fn strategy(&mut self, obs: &Observation) -> Option<(Vec<HunlAction>, Vec<f64>)> {
        if obs.seats.len() != 2 {
            return None;
        }
        self.follow(obs);
        let node = self.track.node?;
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
        self.track.node.is_some_and(|node| {
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
                node: Some(0),
                me: if obs.seat == obs.button { 0 } else { 1 },
                ..Track::default()
            };
        }
        let button = obs.button;
        // Seat numbers to tree players: the button is player 0.
        let side = |seat: usize| if seat == button { 0 } else { 1 };
        while self.track.events < obs.history.len() {
            let e = obs.history[self.track.events].clone();
            self.track.events += 1;
            self.apply_event(&e, side);
        }
    }

    fn apply_event(&mut self, e: &Event, side: impl Fn(usize) -> usize) {
        let t = &mut self.track;
        match *e {
            Event::Ante { seat, amount } => t.real.contributed[side(seat)] += amount,
            Event::SmallBlind { seat, amount } | Event::BigBlind { seat, amount } => {
                let p = side(seat);
                t.real.street_bet[p] += amount;
                t.real.contributed[p] += amount;
                t.real.current_bet = t.real.current_bet.max(t.real.street_bet[p]);
            }
            Event::Board { street, .. } => {
                t.street = street_index(street);
                t.real.street_bet = [0, 0];
                t.real.current_bet = 0;
                // The tree may still be on an earlier street if a real bet was
                // mapped to a smaller one: close the round with calls/checks.
                self.catch_up();
                if street == Street::River {
                    self.track.river_root = self.track.node;
                }
            }
            Event::Fold { seat } => {
                self.on_river(HunlAction::Fold);
                self.step(side(seat), Real::Fold)
            }
            Event::Check { seat } => {
                self.on_river(HunlAction::Check);
                self.step(side(seat), Real::Check)
            }
            Event::Call { seat, amount, .. } => {
                let p = side(seat);
                self.on_river(HunlAction::Call);
                self.step(p, Real::Call);
                let t = &mut self.track;
                t.real.street_bet[p] += amount;
                t.real.contributed[p] += amount;
            }
            Event::Bet { seat, to, all_in } | Event::Raise { seat, to, all_in } => {
                let p = side(seat);
                let r = self.track.real;
                let to_call = r.current_bet.saturating_sub(r.street_bet[p]);
                let frac = pot_fraction(to, r.current_bet, to_call, r.pot());
                self.on_river(if r.current_bet == 0 {
                    HunlAction::Bet(to)
                } else {
                    HunlAction::Raise(to)
                });
                self.step(p, Real::Raise { frac, all_in });
                let t = &mut self.track;
                let added = to - t.real.street_bet[p];
                t.real.street_bet[p] = to;
                t.real.contributed[p] += added;
                t.real.current_bet = to;
            }
            Event::Award { .. } => {}
        }
    }

    /// Records a real action if the hand is on the river.
    fn on_river(&mut self, a: HunlAction) {
        if self.track.street == 3 {
            self.track.river_path.push(a);
        }
    }

    /// Plays checks and calls on the tree until it reaches the real street.
    fn catch_up(&mut self) {
        while let Some(node) = self.track.node {
            let n = &self.tree.nodes[node as usize];
            if n.actions.is_empty() || n.betting.street >= self.track.street {
                return;
            }
            let i = n
                .actions
                .iter()
                .position(|a| matches!(a, HunlAction::Check | HunlAction::Call));
            if let Some(i) = i {
                self.track.steps.push((node, i));
            }
            self.track.node = i.map(|i| n.children[i]);
        }
    }

    /// Moves the tracked node along the real action by player `p`.
    fn step(&mut self, p: usize, real: Real) {
        let Some(node) = self.track.node else { return };
        let n = &self.tree.nodes[node as usize];
        if n.actions.is_empty() || n.betting.street != self.track.street {
            // The tree's round closed before the real one did: a real raise
            // was mapped to a call. Skip the rest of this real round.
            return;
        }
        if n.betting.to_act != p {
            self.track.node = None;
            return;
        }
        if p == self.track.me
            && let Some(i) = self.track.pending.take()
        {
            self.track.steps.push((node, i));
            self.track.node = Some(n.children[i]);
            return;
        }
        let pick = |pred: &dyn Fn(&HunlAction) -> bool| n.actions.iter().position(pred);
        let i = match real {
            Real::Fold => pick(&|a| *a == HunlAction::Fold),
            Real::Check | Real::Call => {
                pick(&|a| matches!(a, HunlAction::Check | HunlAction::Call))
            }
            Real::Raise { frac, all_in } => {
                let b = &n.betting;
                let sizes: Vec<(usize, f64, bool)> = n
                    .actions
                    .iter()
                    .enumerate()
                    .filter_map(|(i, a)| match *a {
                        HunlAction::Bet(to) | HunlAction::Raise(to) => Some((
                            i,
                            pot_fraction(to, b.current_bet, b.to_call(), b.pot()),
                            to == b.all_in_to(),
                        )),
                        _ => None,
                    })
                    .collect();
                if sizes.is_empty() {
                    // No raising left on the tree: the closest is a call.
                    pick(&|a| matches!(a, HunlAction::Check | HunlAction::Call))
                } else if all_in {
                    sizes.iter().find(|s| s.2).or(sizes.last()).map(|s| s.0)
                } else {
                    let below = sizes.iter().rev().find(|s| s.1 <= frac);
                    let above = sizes.iter().find(|s| s.1 >= frac);
                    match (below, above) {
                        (Some(a), Some(b)) if a.0 != b.0 => {
                            let p_small = pseudo_harmonic(a.1, b.1, frac);
                            Some(if self.rng.next_f64() < p_small {
                                a.0
                            } else {
                                b.0
                            })
                        }
                        (Some(a), _) => Some(a.0),
                        (None, Some(b)) => Some(b.0),
                        (None, None) => None,
                    }
                }
            }
        };
        if let Some(i) = i {
            self.track.steps.push((node, i));
        }
        self.track.node = i.map(|i| n.children[i]);
    }

    /// A river decision from the solved subgame: solved now if there's no
    /// solution yet or the real river left its tree. `None` to fall back to
    /// the blueprint.
    fn river_action(&mut self, obs: &Observation) -> Option<Action> {
        if obs.seats.len() != 2 {
            return None;
        }
        self.follow(obs);
        let me = self.track.me;
        let root = river_root(obs)?;
        // The real river so far must replay legally to our turn.
        let mut b = root.clone();
        for &a in &self.track.river_path {
            if b.is_over() {
                return None;
            }
            b = b.play(a);
        }
        if b.is_over() || b.to_act != me {
            return None;
        }
        let path = self.track.river_path.clone();
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
        self.track.pending = None;
        Some(real_action(a, obs))
    }

    /// Solves the river from `root` with the real actions `path` in the
    /// tree, the bot's own earlier river actions frozen at the previous
    /// solution, and the gadget against the blueprint's river values.
    fn solve_river(
        &self,
        obs: &Observation,
        root: &Betting,
        path: &[HunlAction],
    ) -> Option<RiverSolver> {
        let settings = self.river.as_ref()?;
        let bp_root = self.track.river_root?;
        let bp_node = &self.tree.nodes[bp_root as usize];
        if bp_node.actions.is_empty() || bp_node.betting.street != 3 {
            return None;
        }
        let board: Vec<Card> = obs.board.iter().copied().map(from_ducy).collect();
        let board: [Card; 5] = board.try_into().ok()?;
        // Both ranges at the start of the river, from the tree steps before it.
        let before: Vec<(u32, usize)> = self
            .track
            .steps
            .iter()
            .copied()
            .filter(|&(n, _)| self.tree.nodes[n as usize].betting.street < 3)
            .collect();
        let ranges = replay(&self.cards, &self.blueprint, &self.tree, &before, &board);
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
        let buckets = self.cards.buckets(&board);
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
        run_budget(&mut solver, settings);
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

#[derive(Clone, Copy, Debug)]
enum Real {
    Fold,
    Check,
    Call,
    Raise { frac: f64, all_in: bool },
}

/// Runs a solver for the iterations (and time) allowed.
fn run_budget(solver: &mut RiverSolver, settings: &RiverSolving) {
    match settings.time {
        None => solver.run(settings.iterations),
        Some(limit) => {
            let start = std::time::Instant::now();
            for _ in 0..settings.iterations {
                solver.iterate();
                if start.elapsed() >= limit {
                    break;
                }
            }
        }
    }
}

/// The real hand's betting at the start of the river (player 0 the button).
fn river_root(obs: &Observation) -> Option<Betting> {
    let seat = |p: usize| if p == 0 { obs.button } else { 1 - obs.button };
    let mut stack = [0; 2];
    let mut contributed = [0; 2];
    for p in 0..2 {
        let s = obs.seats.get(seat(p))?;
        stack[p] = s.stack + s.street_bet;
        contributed[p] = s.contributed - s.street_bet;
    }
    Some(Betting::street_start(
        3,
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

fn street_index(s: Street) -> usize {
    match s {
        Street::Preflop => 0,
        Street::Flop => 1,
        Street::Turn => 2,
        Street::River => 3,
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
        if self.river.is_some()
            && obs.street == Street::River
            && let Some(action) = self.river_action(obs)
        {
            return Some(action);
        }
        match self.strategy(obs) {
            Some((actions, probs)) => {
                let node = self.track.node.expect("strategy found a node");
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
                self.track.pending = same.then_some(i);
                Some(action)
            }
            None => Some(self.fallback(obs)),
        }
    }

    fn hand_over(&mut self, _summary: &HandSummary) {
        self.track = Track::default();
    }
}
