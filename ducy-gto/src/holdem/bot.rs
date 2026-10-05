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
//! The blueprint is for heads-up play. At a table with more players, or if
//! the hand ever can't be followed on the tree, the bot falls back to a
//! simple pot-odds rule rather than ever returning an illegal action.

use std::sync::Arc;

use ducy_play::{Action, Bot, Event, HandSummary, LegalActions, Observation, Street};
use rand::{SeedableRng, rngs::StdRng};

use super::{
    abstraction::CardAbstraction,
    blueprint::{Blueprint, BlueprintError},
    cards::{Card, from_ducy},
    hunl::{BettingTree, Hunl, HunlAction, HunlConfig},
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

/// A bet or raise to `to` as a fraction of the pot after calling, given the
/// current bet, what the bettor has to call, and the pot before the action.
fn pot_fraction(to: u64, current_bet: u64, to_call: u64, pot: u64) -> f64 {
    (to.saturating_sub(current_bet)) as f64 / (pot + to_call).max(1) as f64
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
    /// The node our own last action leads to, applied when its event shows
    /// up instead of translating our own bet back.
    pending: Option<u32>,
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
    fallback_rng: StdRng,
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
            fallback_rng: StdRng::seed_from_u64(seed ^ 0x5eed),
        }
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
            }
            Event::Fold { seat } => self.step(side(seat), Real::Fold),
            Event::Check { seat } => self.step(side(seat), Real::Check),
            Event::Call { seat, amount, .. } => {
                let p = side(seat);
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
            && let Some(next) = self.track.pending.take()
        {
            self.track.node = Some(next);
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
        self.track.node = i.map(|i| n.children[i]);
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
                self.track.pending = same.then(|| self.tree.nodes[node as usize].children[i]);
                Some(action)
            }
            None => Some(self.fallback(obs)),
        }
    }

    fn hand_over(&mut self, _summary: &HandSummary) {
        self.track = Track::default();
    }
}
