//! Following a real hand on the blueprint's betting tree.
//!
//! The real game allows bets of any size, the blueprint knows only its menu.
//! [`Follower`] takes a hand's events one at a time and keeps the tree node
//! the hand is at:
//!
//! - **A player's own choices** can be given in advance
//!   ([`Follower::pending`]), and are taken as they are.
//! - **Other bets** are mapped to the nearest menu sizes with the randomized
//!   *pseudo-harmonic mapping* (Ganzfried and Sandholm 2013). A bet of `x`
//!   pot between menu sizes `a < x < b` maps to `a` with probability
//!   `(b - x)(1 + a) / ((b - a)(1 + x))` and to `b` otherwise. Each
//!   translation is drawn once and remembered, so later decisions see a
//!   consistent history, and the [`Step`] records both sizes and their
//!   probabilities.
//! - **A real round longer than the tree's** (a raise mapped to a call) is
//!   skipped to the next street, and a tree round still open when the real
//!   one ends is closed with checks and calls.
//!
//! `GtoBot` follows its hands this way, and the hand review replays finished
//! hands with it.

use ducy_play::{Event, Street};

use super::hunl::{BettingTree, HunlAction, pot_fraction};
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

/// The real hand's chips, tracked from its events (player 0 the button).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RealBetting {
    pub street_bet: [u64; 2],
    pub contributed: [u64; 2],
    pub current_bet: u64,
}

impl RealBetting {
    pub fn pot(&self) -> u64 {
        self.contributed[0] + self.contributed[1]
    }
}

/// One action taken on the tree.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub node: u32,
    /// The action's index at `node`.
    pub action: usize,
    /// For a bet size between two menu sizes: both menu actions' indexes and
    /// the probability of each under the mapping (summing to 1). `action` is
    /// the one drawn.
    pub mapped: Option<[(usize, f64); 2]>,
}

/// Where a real hand is on the betting tree.
#[derive(Clone, Debug)]
pub struct Follower {
    /// The current node, or `None` once the hand can't be followed.
    pub node: Option<u32>,
    pub real: RealBetting,
    /// The real street: 0 preflop to 3 river.
    pub street: usize,
    /// Every step taken on the tree.
    pub steps: Vec<Step>,
    /// The tree node the turn starts at.
    pub turn_root: Option<u32>,
    /// The real turn actions so far, in chips.
    pub turn_path: Vec<HunlAction>,
    /// The tree node the river starts at.
    pub river_root: Option<u32>,
    /// The real river actions so far, in chips.
    pub river_path: Vec<HunlAction>,
    /// A player's next action, as the tree action they chose: `(player,
    /// action index)`. Applied when their next action arrives, instead of
    /// translating it back.
    pub pending: Option<(usize, usize)>,
}

impl Default for Follower {
    fn default() -> Self {
        Self::new()
    }
}

impl Follower {
    /// At the root, before the blinds.
    pub fn new() -> Self {
        Self {
            node: Some(0),
            real: RealBetting::default(),
            street: 0,
            steps: Vec::new(),
            turn_root: None,
            turn_path: Vec::new(),
            river_root: None,
            river_path: Vec::new(),
            pending: None,
        }
    }

    /// Applies one event. `side` turns a seat number into a tree player (the
    /// button is 0); `rng` draws the mapping of off-menu sizes.
    pub fn apply(
        &mut self,
        tree: &BettingTree,
        e: &Event,
        side: impl Fn(usize) -> usize,
        rng: &mut Rng,
    ) {
        match *e {
            Event::Ante { seat, amount } => self.real.contributed[side(seat)] += amount,
            // Heads-up GTO tables have no sitting out, but follow it anyway:
            // the dead part is like an ante, the live part like a blind.
            Event::Post { seat, dead, live } => {
                let p = side(seat);
                self.real.contributed[p] += dead + live;
                self.real.street_bet[p] += live;
                self.real.current_bet = self.real.current_bet.max(self.real.street_bet[p]);
            }
            Event::SmallBlind { seat, amount } | Event::BigBlind { seat, amount } => {
                let p = side(seat);
                self.real.street_bet[p] += amount;
                self.real.contributed[p] += amount;
                self.real.current_bet = self.real.current_bet.max(self.real.street_bet[p]);
            }
            Event::Board { street, .. } => {
                self.street = street_index(street);
                self.real.street_bet = [0, 0];
                self.real.current_bet = 0;
                // The tree may still be on an earlier street if a real bet was
                // mapped to a smaller one: close the round with calls/checks.
                self.catch_up(tree);
                match street {
                    Street::Turn => self.turn_root = self.node,
                    Street::River => self.river_root = self.node,
                    _ => {}
                }
            }
            Event::Fold { seat } => {
                self.record(HunlAction::Fold);
                self.step(tree, side(seat), Real::Fold, rng)
            }
            Event::Check { seat } => {
                self.record(HunlAction::Check);
                self.step(tree, side(seat), Real::Check, rng)
            }
            Event::Call { seat, amount, .. } => {
                let p = side(seat);
                self.record(HunlAction::Call);
                self.step(tree, p, Real::Call, rng);
                self.real.street_bet[p] += amount;
                self.real.contributed[p] += amount;
            }
            Event::Bet { seat, to, all_in } | Event::Raise { seat, to, all_in } => {
                let p = side(seat);
                let r = self.real;
                let to_call = r.current_bet.saturating_sub(r.street_bet[p]);
                let frac = pot_fraction(to, r.current_bet, to_call, r.pot());
                self.record(if r.current_bet == 0 {
                    HunlAction::Bet(to)
                } else {
                    HunlAction::Raise(to)
                });
                self.step(tree, p, Real::Raise { frac, all_in }, rng);
                let added = to - self.real.street_bet[p];
                self.real.street_bet[p] = to;
                self.real.contributed[p] += added;
                self.real.current_bet = to;
            }
            Event::Award { .. }
            | Event::Reveal { .. }
            | Event::Forfeit { .. }
            | Event::Runs { .. }
            | Event::SecondBoard { .. } => {}
        }
    }

    /// The steps before the river: what the ranges at the river depend on.
    pub fn steps_before_river(&self, tree: &BettingTree) -> Vec<(u32, usize)> {
        self.steps_before(tree, 3)
    }

    /// The steps before `street` (0 preflop to 3 river).
    pub fn steps_before(&self, tree: &BettingTree, street: usize) -> Vec<(u32, usize)> {
        self.steps
            .iter()
            .filter(|s| tree.nodes[s.node as usize].betting.street < street)
            .map(|s| (s.node, s.action))
            .collect()
    }

    /// Records a real action on the turn or river.
    fn record(&mut self, a: HunlAction) {
        match self.street {
            2 => self.turn_path.push(a),
            3 => self.river_path.push(a),
            _ => {}
        }
    }

    /// Plays checks and calls on the tree until it reaches the real street.
    fn catch_up(&mut self, tree: &BettingTree) {
        while let Some(node) = self.node {
            let n = &tree.nodes[node as usize];
            if n.actions.is_empty() || n.betting.street >= self.street {
                return;
            }
            let i = n
                .actions
                .iter()
                .position(|a| matches!(a, HunlAction::Check | HunlAction::Call));
            self.take(tree, node, i, None);
        }
    }

    fn take(
        &mut self,
        tree: &BettingTree,
        node: u32,
        i: Option<usize>,
        mapped: Option<[(usize, f64); 2]>,
    ) {
        match i {
            Some(i) => {
                self.steps.push(Step {
                    node,
                    action: i,
                    mapped,
                });
                self.node = Some(tree.nodes[node as usize].children[i]);
            }
            None => self.node = None,
        }
    }

    /// Moves the node along the real action by player `p`.
    fn step(&mut self, tree: &BettingTree, p: usize, real: Real, rng: &mut Rng) {
        let Some(node) = self.node else { return };
        let n = &tree.nodes[node as usize];
        if n.actions.is_empty() || n.betting.street != self.street {
            // The tree's round closed before the real one did: a real raise
            // was mapped to a call. Skip the rest of this real round.
            return;
        }
        if n.betting.to_act != p {
            self.node = None;
            return;
        }
        if let Some((q, i)) = self.pending
            && q == p
        {
            self.pending = None;
            self.take(tree, node, Some(i), None);
            return;
        }
        let pick = |pred: &dyn Fn(&HunlAction) -> bool| n.actions.iter().position(pred);
        let mut mapped = None;
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
                        HunlAction::Bet(to) | HunlAction::Raise(to) => {
                            Some((i, b.pot_fraction(to), to == b.all_in_to()))
                        }
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
                            mapped = Some([(a.0, p_small), (b.0, 1.0 - p_small)]);
                            Some(if rng.next_f64() < p_small { a.0 } else { b.0 })
                        }
                        (Some(a), _) => Some(a.0),
                        (None, Some(b)) => Some(b.0),
                        (None, None) => None,
                    }
                }
            }
        };
        self.take(tree, node, i, mapped);
    }
}

#[derive(Clone, Copy, Debug)]
enum Real {
    Fold,
    Check,
    Call,
    Raise { frac: f64, all_in: bool },
}

pub fn street_index(s: Street) -> usize {
    match s {
        Street::Preflop => 0,
        Street::Flop => 1,
        Street::Turn => 2,
        Street::River => 3,
    }
}
