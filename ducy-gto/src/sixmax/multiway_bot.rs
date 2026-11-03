//! `MultiwayBot`: plays a six-handed [`MultiwayBlueprint`] at a real
//! six-handed table (#338), Hold'em or PLO with four to six hole cards.
//!
//! It works like the heads-up [`GtoBot`](crate::holdem::bot::GtoBot):
//!
//! - **Following the hand:** at each decision it replays the betting so far
//!   on the blueprint's [`MultiwayTree`], seats turned into positions (UTG is
//!   0, the big blind 5). Its own actions are on the tree already; other
//!   players' bets go to the neighbouring menu sizes with the randomized
//!   pseudo-harmonic mapping ([`pseudo_harmonic`]), drawn once per hand. A
//!   limp where the menu has none (only the small blind limps on the lean
//!   menus) is taken as the smallest open. A real round longer than the
//!   tree's (a raise mapped to a call once the
//!   tree's raises ran out) is skipped to the next street, and a tree round
//!   still open when the real one ends is closed with checks and calls.
//! - **Choosing an action:** it buckets its hole cards and the board with the
//!   abstraction the blueprint was trained on, samples the stored strategy,
//!   and turns the chosen size into chips as the same fraction of the real
//!   pot, clamped to what's legal; the tree's largest bet is the real
//!   table's largest.
//! - **When the tree has it all-in early** (a big real bet mapped to all-in):
//!   it gets every chip in.
//! - **Anything it can't follow** (a table that isn't six-handed, a player
//!   acting out of the tree's turn, see [`OffTree`]): a pot-odds
//!   fallback that never acts illegally: check when free, call with enough
//!   equity for the price, else fold. Every such decision is counted
//!   ([`MultiwayBot::off_tree`], and [`MwCounts`] shared across bots).

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering::Relaxed},
};

use ducy_play::{Action, BettingStructure, Bot, Event, HandSummary, Observation};
use rand::{SeedableRng, rngs::StdRng};

use super::{
    multiway::{MultiwayBlueprint, MultiwayConfig, MultiwayTree, MwAction, MwBetting},
    preflop::SEATS,
};
use crate::{
    holdem::{
        abstraction::CardAbstraction,
        bot::{fallback, shove},
        cards::{Card, from_ducy},
        follow::{pseudo_harmonic, street_index},
        hunl::Buckets,
    },
    omaha::abstraction::PloAbstraction,
    rng::Rng,
};

/// A card abstraction the bot can share between threads.
pub type SharedBuckets = Arc<dyn Buckets + Send + Sync>;

/// A loaded blueprint with its betting tree and card abstraction, shared by
/// every bot that plays it.
pub struct MultiwayPlayer {
    pub config: MultiwayConfig,
    pub tree: MultiwayTree,
    pub blueprint: MultiwayBlueprint,
    pub cards: SharedBuckets,
    /// Hole cards per hand: 2 for Hold'em, 4 to 6 for PLO.
    pub hole_cards: usize,
}

impl MultiwayPlayer {
    /// A blueprint trained on `config` with `cards`, for `hole_cards` hole
    /// cards. Builds the betting tree.
    pub fn new(
        config: MultiwayConfig,
        blueprint: MultiwayBlueprint,
        cards: SharedBuckets,
        hole_cards: usize,
    ) -> Self {
        let tree = MultiwayTree::build(&config);
        Self {
            config,
            tree,
            blueprint,
            cards,
            hole_cards,
        }
    }

    /// A blueprint written by `train_sixmax` and the card abstraction file it
    /// was trained with (Hold'em or PLO, as the blueprint says). The menu,
    /// depth and hole cards come from the blueprint's settings, and the
    /// abstraction must be the one named there.
    pub fn load(blueprint: &[u8], cards: &[u8]) -> Result<Self, String> {
        let blueprint = MultiwayBlueprint::load(blueprint)?;
        let s = Settings::parse(&blueprint.settings)
            .ok_or_else(|| format!("unreadable blueprint settings: {}", blueprint.settings))?;
        let config = MultiwayConfig::preset(&s.menu, s.big_blinds)
            .ok_or_else(|| format!("no menu named {}", s.menu))?;
        let cards: SharedBuckets = if s.hole_cards == 2 {
            Arc::new(CardAbstraction::load(cards).ok_or("not a Hold'em card abstraction")?)
        } else {
            let c = PloAbstraction::load(cards).ok_or("not a PLO card abstraction")?;
            if c.config.hole_cards != s.hole_cards {
                return Err(format!(
                    "the abstraction has {} hole cards, the blueprint {}",
                    c.config.hole_cards, s.hole_cards
                ));
            }
            Arc::new(c)
        };
        if cards.settings() != s.cards {
            return Err("the blueprint was trained on another card abstraction".into());
        }
        Ok(Self::new(config, blueprint, cards, s.hole_cards))
    }
}

/// What `train_sixmax` writes in a blueprint's settings:
/// `"{menu} {bb} BB, {H} hole cards | {abstraction settings}"`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub menu: String,
    pub big_blinds: u32,
    pub hole_cards: usize,
    pub cards: String,
}

impl Settings {
    pub fn parse(text: &str) -> Option<Self> {
        let (head, cards) = text.split_once(" | ")?;
        let mut words = head.split_whitespace();
        let menu = words.next()?.to_string();
        let big_blinds = words.next()?.parse().ok()?;
        (words.next()? == "BB,").then_some(())?;
        let hole_cards = words.next()?.parse().ok()?;
        Some(Self {
            menu,
            big_blinds,
            hole_cards,
            cards: cards.to_string(),
        })
    }
}

/// Decisions counted over many bots (and threads): how often the hand could
/// be followed on the tree.
#[derive(Debug, Default)]
pub struct MwCounts {
    /// Decisions made.
    pub decisions: AtomicU64,
    /// Decisions that fell back because the hand had left the tree.
    pub off_tree: AtomicU64,
    /// Hands the bot made at least one decision in.
    pub hands: AtomicU64,
    /// Hands with at least one decision off the tree.
    pub off_hands: AtomicU64,
    /// Off-tree decisions by why the hand left the tree, indexed by
    /// [`OffTree`].
    pub by_reason: [AtomicU64; 4],
}

/// Why a hand left the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OffTree {
    /// Not a six-handed table, blinds where the tree doesn't have them, or
    /// hole cards the blueprint wasn't trained for.
    Table = 0,
    /// A real action the menu doesn't have there: a fold with nothing to
    /// call on the tree, say.
    NotOnMenu = 1,
    /// Someone acted while the tree had another player to act (the tree had
    /// them all-in or folded, say).
    OutOfTurn = 2,
    /// The tree's betting round or hand ended before the real one: a raise
    /// past the tree's cap was taken as a call.
    Closed = 3,
}

impl OffTree {
    pub const ALL: [Self; 4] = [Self::Table, Self::NotOnMenu, Self::OutOfTurn, Self::Closed];

    pub fn name(self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::NotOnMenu => "not on the menu",
            Self::OutOfTurn => "out of turn",
            Self::Closed => "tree round closed",
        }
    }
}

impl MwCounts {
    /// Off-tree decisions as a share of all decisions, and off-tree hands as
    /// a share of hands played.
    pub fn rates(&self) -> (f64, f64) {
        let share =
            |a: &AtomicU64, b: &AtomicU64| a.load(Relaxed) as f64 / b.load(Relaxed).max(1) as f64;
        (
            share(&self.off_tree, &self.decisions),
            share(&self.off_hands, &self.hands),
        )
    }
}

/// The real hand's chips, by tree position.
#[derive(Clone, Copy, Debug, Default)]
struct Real {
    street_bet: [u64; SEATS],
    contributed: [u64; SEATS],
    current_bet: u64,
}

/// How a hand is being followed on the tree.
#[derive(Clone, Debug)]
struct Track {
    /// Identifies the hand: our hole cards and the button.
    hand: (u64, usize),
    /// Real events consumed so far.
    events: usize,
    button: usize,
    /// Our position on the tree.
    me: usize,
    /// The current node, or `None` once the hand can't be followed.
    node: Option<u32>,
    /// Why `node` became `None`.
    lost: Option<OffTree>,
    /// The tree's betting at `node`.
    betting: MwBetting,
    real: Real,
    /// The real street: 0 preflop to 3 river.
    street: u8,
    /// Our next action, as the tree action we chose.
    pending: Option<usize>,
    /// Whether this hand has had a decision, and one off the tree.
    counted: bool,
    went_off: bool,
}

/// The real action kinds, with a bet's size as a fraction of the pot.
#[derive(Clone, Copy, Debug)]
enum Kind {
    Fold,
    CheckCall,
    Raise { frac: f64, all_in: bool },
}

/// A bet or raise to `to` as a fraction of the pot after calling.
fn pot_fraction(to: u64, current_bet: u64, to_call: u64, pot: u64) -> f64 {
    to.saturating_sub(current_bet) as f64 / (pot + to_call).max(1) as f64
}

/// A bot that plays a six-handed blueprint.
pub struct MultiwayBot {
    player: Arc<MultiwayPlayer>,
    rng: Rng,
    fallback_rng: StdRng,
    track: Option<Track>,
    counts: Option<Arc<MwCounts>>,
    /// Decisions made.
    pub decisions: u32,
    /// Decisions where it had to use the fallback rule.
    pub off_tree: u32,
}

impl MultiwayBot {
    pub fn new(player: Arc<MultiwayPlayer>, seed: u64) -> Self {
        Self {
            player,
            rng: Rng::new(seed),
            fallback_rng: StdRng::seed_from_u64(seed ^ 0x5eed),
            track: None,
            counts: None,
            decisions: 0,
            off_tree: 0,
        }
    }

    /// Also counts its decisions in `counts`.
    pub fn with_counts(mut self, counts: Arc<MwCounts>) -> Self {
        self.counts = Some(counts);
        self
    }

    /// The tree node the hand is at now, if it's still on the tree.
    pub fn node(&self) -> Option<u32> {
        self.track.as_ref().and_then(|t| t.node)
    }

    /// The blueprint's action probabilities at this decision, with the tree
    /// actions they belong to, or `None` when the hand can't be followed.
    pub fn strategy(&mut self, obs: &Observation) -> Option<(Vec<MwAction>, Vec<f64>)> {
        self.follow(obs);
        let p = &self.player;
        let t = self.track.as_ref()?;
        let node = t.node?;
        let n = &p.tree.nodes[node as usize];
        let b = &t.betting;
        if n.actions == 0 || b.street != t.street || b.to_act as usize != t.me {
            return None;
        }
        let hole: Vec<Card> = obs.hole_cards.iter(false).map(from_ducy).collect();
        if hole.len() != p.hole_cards {
            return None;
        }
        let board: Vec<Card> = obs.board.iter().copied().map(from_ducy).collect();
        let bucket = p.cards.hand_bucket(&hole, &board);
        let k = n.actions as usize;
        let first = n.first_child as usize;
        let actions = (0..k).map(|i| p.tree.nodes[first + i].via()).collect();
        let info = (node as u64) << 16 | bucket as u64;
        Some((actions, p.blueprint.probs(info, k)))
    }

    /// The tree's betting at this decision, when the hand is on the tree
    /// and the blueprint has a strategy for it.
    pub fn tree_betting(&mut self, obs: &Observation) -> Option<MwBetting> {
        self.strategy(obs)?;
        self.track.as_ref().map(|t| t.betting.clone())
    }

    /// Whether the tree has us all-in (and the hand over on the tree) while
    /// the real hand still asks us to act: a large real bet was mapped to
    /// all-in, and the tree's plan is to get every chip in.
    fn committed(&self) -> bool {
        self.track.as_ref().is_some_and(|t| {
            t.node.is_some()
                && t.betting.is_over()
                && t.betting.folded & 1 << t.me == 0
                && t.betting.stack[t.me] == 0
        })
    }

    /// Why this decision can't be played from the tree.
    fn off_reason(&self, obs: &Observation) -> OffTree {
        let Some(t) = &self.track else {
            return OffTree::Table;
        };
        if t.node.is_none() {
            return t.lost.unwrap_or(OffTree::NotOnMenu);
        }
        let b = &t.betting;
        if b.is_over() || b.street != t.street {
            OffTree::Closed
        } else if b.to_act as usize != t.me {
            OffTree::OutOfTurn
        } else if obs.hole_cards.iter(false).count() != self.player.hole_cards {
            OffTree::Table
        } else {
            OffTree::NotOnMenu
        }
    }

    /// Brings the tracked node up to date with the real history.
    fn follow(&mut self, obs: &Observation) {
        if obs.seats.len() != SEATS {
            self.track = None;
            return;
        }
        let key = (u64::from(obs.hole_cards), obs.button);
        let fresh = match &self.track {
            Some(t) => t.hand != key || obs.history.len() < t.events,
            None => true,
        };
        if fresh {
            self.track = Some(Track {
                hand: key,
                events: 0,
                button: obs.button,
                me: position(obs.seat, obs.button),
                node: Some(0),
                lost: None,
                betting: MwBetting::start(&self.player.config),
                real: Real::default(),
                street: 0,
                pending: None,
                counted: false,
                went_off: false,
            });
        }
        let player = self.player.clone();
        let t = self.track.as_mut().expect("just set");
        while t.events < obs.history.len() {
            let e = &obs.history[t.events];
            t.events += 1;
            t.apply(&player, e, &mut self.rng);
        }
    }

    /// Counts a decision, on the tree or off it for `why`.
    fn count(&mut self, why: Option<OffTree>) {
        let off = why.is_some();
        self.decisions += 1;
        self.off_tree += off as u32;
        let (first, first_off) = match self.track.as_mut() {
            Some(t) => {
                let first = !t.counted;
                let first_off = off && !t.went_off;
                t.counted = true;
                t.went_off |= off;
                (first, first_off)
            }
            None => (true, off),
        };
        if let Some(c) = &self.counts {
            c.decisions.fetch_add(1, Relaxed);
            c.off_tree.fetch_add(off as u64, Relaxed);
            c.hands.fetch_add(first as u64, Relaxed);
            c.off_hands.fetch_add(first_off as u64, Relaxed);
            if let Some(why) = why {
                c.by_reason[why as usize].fetch_add(1, Relaxed);
            }
        }
    }
}

/// A seat's position on the tree: 0 UTG, 1 HJ, 2 CO, 3 BTN, 4 SB, 5 BB.
fn position(seat: usize, button: usize) -> usize {
    (seat + 2 * SEATS - button + 3) % SEATS
}

impl Track {
    fn apply(&mut self, player: &MultiwayPlayer, e: &Event, rng: &mut Rng) {
        let pos = |seat: usize| position(seat, self.button);
        match *e {
            Event::Ante { seat, amount } => self.real.contributed[pos(seat)] += amount,
            Event::Post { seat, dead, live } => {
                let p = pos(seat);
                self.real.contributed[p] += dead + live;
                self.real.street_bet[p] += live;
                self.real.current_bet = self.real.current_bet.max(self.real.street_bet[p]);
            }
            Event::SmallBlind { seat, amount } | Event::BigBlind { seat, amount } => {
                let p = pos(seat);
                let want = if matches!(e, Event::SmallBlind { .. }) {
                    4
                } else {
                    5
                };
                if p != want {
                    // Blinds somewhere the tree doesn't have them.
                    self.lose(OffTree::Table);
                }
                self.real.street_bet[p] += amount;
                self.real.contributed[p] += amount;
                self.real.current_bet = self.real.current_bet.max(self.real.street_bet[p]);
            }
            Event::Board { street, .. } => {
                self.street = street_index(street) as u8;
                self.real.street_bet = [0; SEATS];
                self.real.current_bet = 0;
                self.catch_up(player);
            }
            Event::Fold { seat } => self.step(player, pos(seat), Kind::Fold, rng),
            Event::Check { seat } => self.step(player, pos(seat), Kind::CheckCall, rng),
            Event::Call { seat, amount, .. } => {
                let p = pos(seat);
                self.step(player, p, Kind::CheckCall, rng);
                self.real.street_bet[p] += amount;
                self.real.contributed[p] += amount;
            }
            Event::Bet { seat, to, all_in } | Event::Raise { seat, to, all_in } => {
                let p = pos(seat);
                let r = self.real;
                let to_call = r.current_bet.saturating_sub(r.street_bet[p]);
                let pot = r.contributed.iter().sum();
                let frac = pot_fraction(to, r.current_bet, to_call, pot);
                self.step(player, p, Kind::Raise { frac, all_in }, rng);
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

    /// Leaves the tree for `why`, unless already off it.
    fn lose(&mut self, why: OffTree) {
        if self.node.take().is_some() {
            self.lost = Some(why);
        }
    }

    /// Takes the action at index `i` of the current node.
    fn take(&mut self, player: &MultiwayPlayer, i: Option<usize>) {
        let (Some(node), Some(i)) = (self.node, i) else {
            self.lose(OffTree::NotOnMenu);
            return;
        };
        let child = player.tree.nodes[node as usize].first_child + i as u32;
        self.betting = self.betting.play(player.tree.nodes[child as usize].via());
        self.node = Some(child);
    }

    /// The index of the first action at the current node matching `want`.
    fn find(&self, player: &MultiwayPlayer, want: impl Fn(MwAction) -> bool) -> Option<usize> {
        let n = &player.tree.nodes[self.node? as usize];
        (0..n.actions as usize).find(|&i| want(player.tree.nodes[n.first_child as usize + i].via()))
    }

    /// Checks and calls on the tree until it reaches the real street.
    fn catch_up(&mut self, player: &MultiwayPlayer) {
        while self.node.is_some() && !self.betting.is_over() && self.betting.street < self.street {
            let i = self.find(player, |a| matches!(a, MwAction::Check | MwAction::Call));
            self.take(player, i);
        }
    }

    /// Moves the node along a real action by position `p`.
    fn step(&mut self, player: &MultiwayPlayer, p: usize, real: Kind, rng: &mut Rng) {
        let Some(node) = self.node else { return };
        let b = &self.betting;
        if b.is_over() || b.street != self.street {
            // The tree's round closed before the real one did (a raise was
            // mapped to a call): skip the rest of this real round.
            return;
        }
        if b.to_act as usize != p {
            self.lose(OffTree::OutOfTurn);
            return;
        }
        if p == self.me
            && let Some(i) = self.pending.take()
        {
            self.take(player, Some(i));
            return;
        }
        let i = match real {
            Kind::Fold => self.find(player, |a| a == MwAction::Fold),
            // A limp where the menu has none is closest to the smallest open.
            Kind::CheckCall => self
                .find(player, |a| matches!(a, MwAction::Check | MwAction::Call))
                .or_else(|| self.find(player, |a| matches!(a, MwAction::Raise(_)))),
            Kind::Raise { frac, all_in } => {
                let n = &player.tree.nodes[node as usize];
                let all_in_to = b.street_bet[p] + b.stack[p];
                let to_call = b.to_call();
                let pot = b.pot();
                // (index, pot fraction, all-in) of each raise on the menu.
                let sizes: Vec<(usize, f64, bool)> = (0..n.actions as usize)
                    .filter_map(
                        |i| match player.tree.nodes[n.first_child as usize + i].via() {
                            MwAction::Raise(to) => Some((
                                i,
                                pot_fraction(
                                    to as u64,
                                    b.current_bet as u64,
                                    to_call as u64,
                                    pot as u64,
                                ),
                                to == all_in_to,
                            )),
                            _ => None,
                        },
                    )
                    .collect();
                if sizes.is_empty() {
                    // No raising left on the tree: the closest is a call.
                    self.find(player, |a| matches!(a, MwAction::Check | MwAction::Call))
                } else if let Some(s) = sizes.iter().find(|s| all_in && s.2) {
                    Some(s.0)
                } else {
                    let below = sizes.iter().rev().find(|s| s.1 <= frac);
                    let above = sizes.iter().find(|s| s.1 >= frac);
                    match (below, above) {
                        (Some(a), Some(b)) if a.0 != b.0 => {
                            let p_small = pseudo_harmonic(a.1, b.1, frac);
                            Some(if rng.next_f64() < p_small { a.0 } else { b.0 })
                        }
                        (Some(a), _) => Some(a.0),
                        (None, Some(b)) => Some(b.0),
                        (None, None) => None,
                    }
                }
            }
        };
        self.take(player, i);
    }
}

/// The tree's action `a` at `b` as a legal real action: a bet or raise as the
/// same fraction of the real pot, clamped to what's legal, with the tree's
/// largest size as the real table's largest.
fn realize(a: MwAction, b: &MwBetting, pot_limit: bool, obs: &Observation) -> Action {
    let legal = &obs.legal;
    let check_or_call = if legal.can_check {
        Action::Check
    } else {
        Action::Call
    };
    match a {
        MwAction::Fold if legal.can_fold => Action::Fold,
        MwAction::Fold | MwAction::Check | MwAction::Call => check_or_call,
        MwAction::Raise(to) => {
            let Some(range) = legal.bet.or(legal.raise) else {
                return check_or_call;
            };
            let real = if to >= b.max_to(pot_limit) {
                range.max_to
            } else {
                let frac = pot_fraction(
                    to as u64,
                    b.current_bet as u64,
                    b.to_call() as u64,
                    b.pot() as u64,
                );
                let me = &obs.seats[obs.seat];
                let to_call = obs.current_bet.saturating_sub(me.street_bet);
                obs.current_bet + (frac * (obs.pot + to_call) as f64).round() as u64
            };
            let real = real.clamp(range.min_to, range.max_to);
            if legal.bet.is_some() {
                Action::Bet(real)
            } else {
                Action::Raise(real)
            }
        }
    }
}

impl Bot for MultiwayBot {
    fn act(&mut self, obs: &Observation) -> Option<Action> {
        if let Some((actions, probs)) = self.strategy(obs) {
            let t = self.track.as_ref().expect("strategy found the hand");
            let pot_limit = obs.rules.structure == BettingStructure::PotLimit;
            let i = self.rng.sample(&probs);
            let a = actions[i];
            let action = realize(a, &t.betting, pot_limit, obs);
            // When the real action is the same kind as the tree's, follow the
            // tree's choice exactly once its event arrives.
            let same = matches!(
                (a, action),
                (MwAction::Fold, Action::Fold)
                    | (
                        MwAction::Check | MwAction::Call,
                        Action::Check | Action::Call
                    )
                    | (MwAction::Raise(_), Action::Bet(_) | Action::Raise(_))
            );
            self.track.as_mut().expect("followed").pending = same.then_some(i);
            self.count(None);
            return Some(action);
        }
        if self.committed() {
            self.count(None);
            return Some(shove(obs));
        }
        self.count(Some(self.off_reason(obs)));
        Some(fallback(obs, &mut self.fallback_rng))
    }

    fn hand_over(&mut self, _summary: &HandSummary) {
        self.track = None;
    }
}
