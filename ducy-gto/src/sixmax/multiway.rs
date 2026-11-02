//! The six-handed game to the river (#338): six players, blinds, a small
//! bet menu with a cap on raises per street, and the cards seen only through
//! a card abstraction, as in Pluribus (Brown and Sandholm 2019). Players who
//! fold drop out; the rest play on to the river, with side pots when someone
//! is all-in. [`Mccfr`](crate::Mccfr) trains it with one traversal per seat.
//!
//! Seats are numbered by position: 0 UTG, 1 HJ, 2 CO, 3 BTN, 4 SB, 5 BB.
//! Preflop, UTG acts first; after the flop the small blind does, and the
//! button acts last. Chips are counted as in the heads-up game: the small
//! blind is 1 and the big blind 2 by default.
//!
//! Like the heads-up game ([`Hunl`](crate::holdem::hunl::Hunl)), the betting
//! doesn't depend on the cards, so it's compiled once into a
//! [`MultiwayTree`] of numbered nodes, and an information set is a node and
//! the player's bucket on that street (`node << 16 | bucket`). A node takes
//! 8 bytes, and only the betting of the hand being played is kept in the
//! state, so trees of hundreds of millions of nodes fit.
//!
//! Simplifications: every bet or raise reopens the betting for the others
//! (a short all-in raise does too), and every seat starts with the same stack.

use super::preflop::SEATS;
use crate::{
    game::{Game, Turn},
    holdem::{
        cards::{Card, NUM_CARDS, bit, showdown_score},
        hunl::{Buckets, Size},
    },
    omaha::showdown::RiverBoard,
    rng::Rng,
};

const SB: usize = 4;
const BB: usize = 5;

/// Blinds, stacks and the bet menu.
#[derive(Clone, Debug, PartialEq)]
pub struct MultiwayConfig {
    pub small_blind: u32,
    pub big_blind: u32,
    /// Every seat's stack at the start of a hand, blinds included.
    pub stack: u32,
    /// Pot-limit (Omaha): no bet or raise bigger than the pot after calling.
    pub pot_limit: bool,
    /// Preflop raise sizes by depth: `[0]` is the open, `[1]` the 3-bet, and
    /// so on. The last list repeats for deeper raises.
    pub preflop: Vec<Vec<Size>>,
    /// Sizes after the flop with two players in the hand, by depth: `[0]` is
    /// the bet, `[1]` the raise, and so on.
    pub postflop: Vec<Vec<Size>>,
    /// Sizes after the flop with three or more players in the hand.
    pub multiway: Vec<Vec<Size>>,
    /// Raises allowed preflop, the open included (3: open, 3-bet, 4-bet).
    pub preflop_raises: u8,
    /// Bets and raises allowed per street after the flop, heads-up and with
    /// three or more players in.
    pub postflop_raises: u8,
    pub multiway_raises: u8,
    /// Who may just call the big blind preflop.
    pub limp: Limp,
}

/// Who may limp (call the big blind with no raise in front).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limp {
    /// Anyone: many multiway limped pots, a much bigger tree.
    Any,
    /// Only the small blind (completing); everyone else raises or folds.
    /// Solvers' six-handed charts rarely limp anywhere else.
    SmallBlind,
    /// Nobody.
    Never,
}

impl MultiwayConfig {
    /// No-limit Hold'em: open 2.5 BB, 3-bet 3×, 4-bet all-in, only the small
    /// blind limps. After the flop heads-up: bet ⅓, ¾ or all-in, raise the
    /// pot or all-in, two bets or raises a street. Three or more players:
    /// bet ½ or all-in, raise all-in.
    pub fn holdem(big_blinds: u32) -> Self {
        use Size::*;
        Self {
            small_blind: 1,
            big_blind: 2,
            stack: 2 * big_blinds,
            pot_limit: false,
            preflop: vec![vec![Bb(2.5)], vec![Times(3.0)], vec![AllIn]],
            postflop: vec![vec![Pot(0.33), Pot(0.75), AllIn], vec![Pot(1.0), AllIn]],
            multiway: vec![vec![Pot(0.5), AllIn], vec![AllIn]],
            preflop_raises: 3,
            postflop_raises: 2,
            multiway_raises: 2,
            limp: Limp::SmallBlind,
        }
    }

    /// Pot-limit Omaha: every preflop raise pot-sized, three of them, only
    /// the small blind limps. After the flop heads-up: bet ½ or the pot,
    /// raise the pot, two a street. Three or more: bet or raise the pot.
    pub fn pot_limit_omaha(big_blinds: u32) -> Self {
        use Size::*;
        Self {
            small_blind: 1,
            big_blind: 2,
            stack: 2 * big_blinds,
            pot_limit: true,
            preflop: vec![vec![Pot(1.0)]],
            postflop: vec![vec![Pot(0.5), Pot(1.0)], vec![Pot(1.0)]],
            multiway: vec![vec![Pot(1.0)]],
            preflop_raises: 3,
            postflop_raises: 2,
            multiway_raises: 2,
            limp: Limp::SmallBlind,
        }
    }

    /// Named menus for sizing and training, at `big_blinds` deep: `holdem`
    /// and `plo` ([`Self::holdem`], [`Self::pot_limit_omaha`]) and variants.
    /// `-lean` has fewer sizes and raises after the flop, `-wide` more,
    /// `-limp` lets anyone limp.
    pub fn presets(big_blinds: u32) -> Vec<(&'static str, Self)> {
        use Size::*;
        let holdem = Self::holdem(big_blinds);
        let holdem_lean = Self {
            postflop: vec![vec![Pot(0.5), AllIn], vec![AllIn]],
            multiway: vec![vec![Pot(0.5)], vec![AllIn]],
            multiway_raises: 1,
            ..holdem.clone()
        };
        let plo = Self::pot_limit_omaha(big_blinds);
        let plo_lean = Self {
            postflop: vec![vec![Pot(0.67)], vec![Pot(1.0)]],
            multiway_raises: 1,
            ..plo.clone()
        };
        vec![
            ("holdem", holdem.clone()),
            ("holdem-lean", holdem_lean),
            (
                // The heads-up blueprint's postflop sizes, three raises.
                "holdem-wide",
                Self {
                    preflop: vec![vec![Bb(2.5), AllIn], vec![Times(3.0), AllIn], vec![AllIn]],
                    postflop: vec![
                        vec![Pot(0.33), Pot(0.75), Pot(1.25), AllIn],
                        vec![Pot(1.0), AllIn],
                        vec![AllIn],
                    ],
                    postflop_raises: 3,
                    ..holdem.clone()
                },
            ),
            (
                "holdem-limp",
                Self {
                    limp: Limp::Any,
                    ..holdem
                },
            ),
            ("plo", plo.clone()),
            ("plo-lean", plo_lean),
            (
                "plo-wide",
                Self {
                    postflop: vec![vec![Pot(0.33), Pot(0.67), Pot(1.0)], vec![Pot(1.0)]],
                    multiway: vec![vec![Pot(0.5), Pot(1.0)], vec![Pot(1.0)]],
                    ..plo.clone()
                },
            ),
            (
                "plo-limp",
                Self {
                    limp: Limp::Any,
                    ..plo
                },
            ),
        ]
    }
}

/// An action, with amounts as the street total ("raise to").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MwAction {
    Fold,
    Check,
    Call,
    /// Bet or raise to this street total.
    Raise(u32),
}

/// The betting part of a hand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MwBetting {
    /// 0 preflop to 3 river.
    pub street: u8,
    pub to_act: u8,
    /// Seats that have folded, and that have acted since the last bet or
    /// raise, as bit masks.
    pub folded: u8,
    acted: u8,
    /// Bets and raises this street (preflop, the big blind counts as one).
    pub raises: u8,
    done: bool,
    pub stack: [u32; SEATS],
    pub street_bet: [u32; SEATS],
    pub contributed: [u32; SEATS],
    pub current_bet: u32,
    pub last_raise: u32,
    big_blind: u32,
}

impl MwBetting {
    /// Blinds posted, UTG to act.
    pub fn start(c: &MultiwayConfig) -> Self {
        let mut b = Self {
            street: 0,
            to_act: 0,
            folded: 0,
            acted: 0,
            raises: 1,
            done: false,
            stack: [c.stack; SEATS],
            street_bet: [0; SEATS],
            contributed: [0; SEATS],
            current_bet: c.big_blind,
            last_raise: c.big_blind,
            big_blind: c.big_blind,
        };
        b.put_in(SB, c.small_blind);
        b.put_in(BB, c.big_blind);
        b
    }

    pub fn is_over(&self) -> bool {
        self.done
    }

    pub fn pot(&self) -> u32 {
        self.contributed.iter().sum()
    }

    /// Players who haven't folded (all-in ones included).
    pub fn in_hand(&self) -> u32 {
        SEATS as u32 - self.folded.count_ones()
    }

    fn live(&self, p: usize) -> bool {
        self.folded & 1 << p == 0
    }

    pub fn to_call(&self) -> u32 {
        self.current_bet - self.street_bet[self.to_act as usize]
    }

    /// The largest legal bet or raise: all-in, or under pot-limit the pot
    /// after calling if that's less.
    pub fn max_to(&self, pot_limit: bool) -> u32 {
        let me = self.to_act as usize;
        let all_in = self.street_bet[me] + self.stack[me];
        if pot_limit {
            all_in.min(self.current_bet + self.pot() + self.to_call())
        } else {
            all_in
        }
    }

    /// Whether the player to act may bet or raise: someone else in the hand
    /// still has chips, and they have more than the call.
    fn can_raise(&self) -> bool {
        let me = self.to_act as usize;
        let others = (0..SEATS).any(|p| p != me && self.live(p) && self.stack[p] > 0);
        others && self.stack[me] > self.to_call()
    }

    /// The menu's actions here, in action-index order: fold (when facing a
    /// bet), check or call, then bets or raises from smallest to largest.
    pub fn actions(&self, c: &MultiwayConfig) -> Vec<MwAction> {
        let to_call = self.to_call();
        let mut out = Vec::with_capacity(5);
        let limp = self.street == 0 && self.raises == 1 && to_call > 0;
        if to_call > 0 {
            out.push(MwAction::Fold);
            let may_limp = match c.limp {
                Limp::Any => true,
                Limp::SmallBlind => self.to_act as usize == SB,
                Limp::Never => false,
            };
            if may_limp || !limp {
                out.push(MwAction::Call);
            }
        } else {
            out.push(MwAction::Check);
        }
        let (menu, cap, depth) = if self.street == 0 {
            (&c.preflop, c.preflop_raises + 1, self.raises - 1)
        } else if self.in_hand() > 2 {
            (&c.multiway, c.multiway_raises, self.raises)
        } else {
            (&c.postflop, c.postflop_raises, self.raises)
        };
        if !self.can_raise() || self.raises >= cap || menu.is_empty() {
            return out;
        }
        let max_to = self.max_to(c.pot_limit);
        let min_to = (self.current_bet + self.last_raise).min(max_to);
        let pot = self.pot();
        let sizes = &menu[(depth as usize).min(menu.len() - 1)];
        let mut tos: Vec<u32> = sizes
            .iter()
            .map(|&size| {
                let to = match size {
                    Size::Pot(f) => self.current_bet + (f * (pot + to_call) as f64).round() as u32,
                    Size::Bb(x) => (x * c.big_blind as f64).round() as u32,
                    Size::Times(x) => (x * self.current_bet as f64).round() as u32,
                    Size::AllIn => max_to,
                };
                to.clamp(min_to, max_to)
            })
            .collect();
        tos.sort_unstable();
        tos.dedup();
        out.extend(tos.into_iter().map(MwAction::Raise));
        out
    }

    fn put_in(&mut self, p: usize, amount: u32) {
        let amount = amount.min(self.stack[p]);
        self.stack[p] -= amount;
        self.street_bet[p] += amount;
        self.contributed[p] += amount;
    }

    /// Whether `p` still has to act this round.
    fn needs(&self, p: usize) -> bool {
        self.live(p)
            && self.stack[p] > 0
            && (self.street_bet[p] < self.current_bet || self.acted & 1 << p == 0)
    }

    /// The betting after `action`, which must be legal here.
    pub fn play(&self, action: MwAction) -> Self {
        let mut n = self.clone();
        let me = self.to_act as usize;
        match action {
            MwAction::Fold => n.folded |= 1 << me,
            MwAction::Check => {}
            MwAction::Call => n.put_in(me, self.to_call()),
            MwAction::Raise(to) => {
                let increase = to - self.current_bet;
                n.put_in(me, to - self.street_bet[me]);
                n.current_bet = to;
                n.last_raise = n.last_raise.max(increase);
                n.raises += 1;
                n.acted = 0;
            }
        }
        n.acted |= 1 << me;
        if n.in_hand() == 1 {
            n.done = true;
            return n;
        }
        // The next seat that still has to act, in order from `me`.
        for k in 1..=SEATS {
            let p = (me + k) % SEATS;
            if n.needs(p) {
                n.to_act = p as u8;
                return n;
            }
        }
        n.next_street();
        n
    }

    /// Ends the betting round: on to the next street where two players can
    /// still bet, or to the showdown.
    fn next_street(&mut self) {
        let with_chips = (0..SEATS)
            .filter(|&p| self.live(p) && self.stack[p] > 0)
            .count();
        if self.street == 3 || with_chips < 2 {
            self.done = true;
            return;
        }
        self.street += 1;
        self.street_bet = [0; SEATS];
        self.current_bet = 0;
        self.raises = 0;
        self.acted = 0;
        self.last_raise = self.big_blind;
        // After the flop the small blind acts first.
        for k in 0..SEATS {
            let p = (SB + k) % SEATS;
            if self.needs(p) {
                self.to_act = p as u8;
                return;
            }
        }
        unreachable!("two players with chips");
    }
}

/// One node of the compiled tree: 8 bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MwNode {
    /// The first child; the others follow it.
    pub first_child: u32,
    /// The action that led here (its street total, for a raise).
    pub amount: u16,
    /// 0 fold, 1 check, 2 call, 3 raise: the action that led here.
    pub via: u8,
    /// Actions here (0 once the hand is over).
    pub actions: u8,
}

impl MwNode {
    pub fn via(&self) -> MwAction {
        match self.via {
            0 => MwAction::Fold,
            1 => MwAction::Check,
            2 => MwAction::Call,
            _ => MwAction::Raise(self.amount as u32),
        }
    }
}

/// Counts of the betting tree, ignoring cards.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MwStats {
    /// Nodes in all, decision points and terminal ones.
    pub nodes: u64,
    /// Decision points (betting sequences where someone acts) per street.
    pub sequences: [u64; 4],
    /// Actions summed over those decision points, per street.
    pub actions: [u64; 4],
    /// Decision points per street by players in the hand: `[street][k]`
    /// for k = 2 … 6 players (indices 0 and 1 unused).
    pub by_players: [[u64; SEATS + 1]; 4],
}

impl MwStats {
    /// Information sets per street with these bucket counts per street.
    pub fn infosets(&self, buckets: [u64; 4]) -> [u64; 4] {
        std::array::from_fn(|s| self.sequences[s] * buckets[s])
    }

    /// Regret and strategy-sum entries (one per action of every information
    /// set) with these bucket counts per street.
    pub fn entries(&self, buckets: [u64; 4]) -> u64 {
        (0..4).map(|s| self.actions[s] * buckets[s]).sum()
    }
}

/// Every betting sequence the menu allows, numbered; node 0 is the start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MultiwayTree {
    pub nodes: Vec<MwNode>,
}

impl MultiwayTree {
    /// Walks the tree without storing it, for sizing: `None` if it has more
    /// than `limit` nodes.
    pub fn count(c: &MultiwayConfig, limit: u64) -> Option<MwStats> {
        fn walk(c: &MultiwayConfig, b: &MwBetting, s: &mut MwStats, limit: u64) -> bool {
            s.nodes += 1;
            if s.nodes > limit {
                return false;
            }
            if b.is_over() {
                return true;
            }
            let actions = b.actions(c);
            let st = b.street as usize;
            s.sequences[st] += 1;
            s.actions[st] += actions.len() as u64;
            s.by_players[st][b.in_hand() as usize] += 1;
            actions.iter().all(|&a| walk(c, &b.play(a), s, limit))
        }
        let mut s = MwStats::default();
        walk(c, &MwBetting::start(c), &mut s, limit).then_some(s)
    }

    /// The whole tree. Children are numbered together, so a node needs only
    /// its first child's number.
    pub fn build(c: &MultiwayConfig) -> Self {
        fn add(c: &MultiwayConfig, b: &MwBetting, id: usize, nodes: &mut Vec<MwNode>) {
            if b.is_over() {
                return;
            }
            let actions = b.actions(c);
            let first = nodes.len();
            assert!(first + actions.len() <= u32::MAX as usize, "tree too big");
            nodes[id].first_child = first as u32;
            nodes[id].actions = actions.len() as u8;
            for &a in &actions {
                let (via, amount) = match a {
                    MwAction::Fold => (0, 0),
                    MwAction::Check => (1, 0),
                    MwAction::Call => (2, 0),
                    MwAction::Raise(to) => (3, to),
                };
                nodes.push(MwNode {
                    first_child: 0,
                    amount: u16::try_from(amount).expect("amounts fit 16 bits"),
                    via,
                    actions: 0,
                });
            }
            for (k, &a) in actions.iter().enumerate() {
                add(c, &b.play(a), first + k, nodes);
            }
        }
        let mut nodes = vec![MwNode {
            first_child: 0,
            amount: 0,
            via: 1,
            actions: 0,
        }];
        add(c, &MwBetting::start(c), 0, &mut nodes);
        nodes.shrink_to_fit();
        Self { nodes }
    }
}

/// A hand in progress: the deal, every seat's buckets and showdown score,
/// the node and the betting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MultiwayState<const H: usize = 2> {
    pub hole: [[Card; H]; SEATS],
    pub board: [Card; 5],
    pub buckets: [[u16; 4]; SEATS],
    /// Showdown strength (higher wins).
    pub score: [u32; SEATS],
    pub node: u32,
    pub betting: MwBetting,
    dealt: bool,
}

/// The six-handed game: Hold'em (`H` = 2, the default) or Omaha with `H`
/// hole cards, abstracted by `A`.
pub struct MultiwayGame<'a, A, const H: usize = 2> {
    pub config: MultiwayConfig,
    /// `None` puts every hand in bucket 0: just the betting, for tests.
    pub cards: Option<&'a A>,
    pub tree: MultiwayTree,
}

impl<'a, A: Buckets, const H: usize> MultiwayGame<'a, A, H> {
    pub fn new(config: MultiwayConfig, cards: Option<&'a A>) -> Self {
        let tree = MultiwayTree::build(&config);
        Self {
            config,
            cards,
            tree,
        }
    }

    /// A hand with these cards dealt and the blinds posted.
    pub fn deal(&self, hole: [[Card; H]; SEATS], board: [Card; 5]) -> MultiwayState<H> {
        let mut buckets = [[0; 4]; SEATS];
        if let Some(cards) = self.cards {
            let hands: [&[Card]; SEATS] = std::array::from_fn(|p| &hole[p][..]);
            cards.table_buckets(&hands, &board, &mut buckets);
        }
        let score = if H == 2 {
            std::array::from_fn(|p| showdown_score(&hole[p], &board))
        } else {
            let river = RiverBoard::new(&board);
            std::array::from_fn(|p| river.score(&hole[p]))
        };
        MultiwayState {
            hole,
            board,
            buckets,
            score,
            node: 0,
            betting: MwBetting::start(&self.config),
            dealt: true,
        }
    }

    /// Every seat's payoff at the end of a hand: what it wins from the pots
    /// (side pots when someone is all-in, split pots shared) less what it
    /// put in.
    pub fn payoffs(&self, s: &MultiwayState<H>) -> [f64; SEATS] {
        std::array::from_fn(|p| self.payoff(s, p))
    }

    /// One seat's payoff at the end of a hand (see [`Self::payoffs`]).
    pub fn payoff(&self, s: &MultiwayState<H>, player: usize) -> f64 {
        let b = &s.betting;
        let put = b.contributed;
        let mine = put[player] as f64;
        if !b.live(player) {
            return -mine;
        }
        if b.in_hand() == 1 {
            return b.pot() as f64 - mine;
        }
        // Pots from the smallest all-in up: each level's slice goes to the
        // best hand among the live players who reached it.
        let mut won = 0.0;
        let mut below = 0;
        while below < put[player] {
            let level = (0..SEATS)
                .filter(|&p| b.live(p) && put[p] > below)
                .map(|p| put[p])
                .min()
                .expect("the player is live above `below`");
            let pot: u32 = put.iter().map(|&x| x.min(level) - x.min(below)).sum();
            let (mut best, mut winners) = (0, 0);
            for p in (0..SEATS).filter(|&p| b.live(p) && put[p] >= level) {
                if winners == 0 || s.score[p] > best {
                    (best, winners) = (s.score[p], 1);
                } else if s.score[p] == best {
                    winners += 1;
                }
            }
            if s.score[player] == best {
                won += pot as f64 / winners as f64;
            }
            below = level;
        }
        won - mine
    }
}

impl<A: Buckets, const H: usize> Game for MultiwayGame<'_, A, H> {
    type State = MultiwayState<H>;
    /// `node << 16 | bucket`.
    type Info = u64;

    fn root(&self) -> MultiwayState<H> {
        MultiwayState {
            hole: [[0; H]; SEATS],
            board: [0; 5],
            buckets: [[0; 4]; SEATS],
            score: [0; SEATS],
            node: 0,
            betting: MwBetting::start(&self.config),
            dealt: false,
        }
    }

    fn turn(&self, s: &MultiwayState<H>) -> Turn {
        if !s.dealt {
            Turn::Chance
        } else if s.betting.is_over() {
            Turn::Terminal
        } else {
            Turn::Player(s.betting.to_act as usize)
        }
    }

    fn num_players(&self) -> usize {
        SEATS
    }

    /// Bucketing six hands on every street is most of a traversal's cost,
    /// so the six traversals share one deal.
    fn shared_deal(&self) -> bool {
        true
    }

    fn utility(&self, s: &MultiwayState<H>) -> f64 {
        self.payoff(s, 0)
    }

    fn utility_for(&self, s: &MultiwayState<H>, player: usize) -> f64 {
        self.payoff(s, player)
    }

    /// Too many deals to list; see [`Game::sample_chance`].
    fn chance_outcomes(&self, _: &MultiwayState<H>) -> Vec<(MultiwayState<H>, f64)> {
        Vec::new()
    }

    fn sample_chance(&self, _: &MultiwayState<H>, rng: &mut Rng) -> MultiwayState<H> {
        let mut used = 0u64;
        let mut draw = || loop {
            let x = (rng.next_u64() % NUM_CARDS as u64) as Card;
            if used & bit(x) == 0 {
                used |= bit(x);
                return x;
            }
        };
        let hole: [[Card; H]; SEATS] = std::array::from_fn(|_| std::array::from_fn(|_| draw()));
        let board: [Card; 5] = std::array::from_fn(|_| draw());
        self.deal(hole, board)
    }

    fn num_actions(&self, s: &MultiwayState<H>) -> usize {
        self.tree.nodes[s.node as usize].actions as usize
    }

    fn apply(&self, s: &MultiwayState<H>, action: usize) -> MultiwayState<H> {
        let child = self.tree.nodes[s.node as usize].first_child + action as u32;
        let mut n = s.clone();
        n.node = child;
        n.betting = s.betting.play(self.tree.nodes[child as usize].via());
        n
    }

    fn info(&self, s: &MultiwayState<H>) -> u64 {
        let b = &s.betting;
        let bucket = s.buckets[b.to_act as usize][b.street as usize];
        (s.node as u64) << 16 | bucket as u64
    }
}

const BLUEPRINT_MAGIC: &[u8; 8] = b"DUCY6MW\x01";

/// A trained six-handed strategy, compact enough to load whole: for every
/// information set the solver saw, its action probabilities in 1/255ths,
/// sorted by key. Missing information sets play uniformly.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MultiwayBlueprint {
    /// Free text naming the game and abstraction it was trained on.
    pub settings: String,
    keys: Vec<u64>,
    /// Where each key's probabilities start in `probs` (one past the last
    /// is the next key's start).
    starts: Vec<u32>,
    probs: Vec<u8>,
}

impl MultiwayBlueprint {
    /// Writes a blueprint as the average strategies come, without holding
    /// it in memory: the magic, the settings, the count, then per
    /// information set its key, action count and quantized probabilities.
    /// `each` calls its argument once per information set, as
    /// [`Mccfr::average_each`](crate::Mccfr::average_each) does.
    pub fn write(
        settings: &str,
        count: usize,
        each: impl FnOnce(&mut dyn FnMut(&u64, &[f64])),
        w: &mut impl std::io::Write,
    ) -> std::io::Result<()> {
        let mut out = BLUEPRINT_MAGIC.to_vec();
        out.extend((settings.len() as u32).to_le_bytes());
        out.extend(settings.as_bytes());
        out.extend((count as u64).to_le_bytes());
        let mut result = Ok(());
        each(&mut |key, probs| {
            out.extend(key.to_le_bytes());
            out.push(probs.len() as u8);
            out.extend(quantize(probs));
            if out.len() >= 1 << 16 && result.is_ok() {
                result = w.write_all(&out);
                out.clear();
            }
        });
        result?;
        w.write_all(&out)
    }

    pub fn load(bytes: &[u8]) -> Result<Self, String> {
        let bad = || "not a six-handed blueprint".to_string();
        let mut at = 0usize;
        let mut take = |n: usize| -> Result<&[u8], String> {
            let b = bytes.get(at..at + n).ok_or_else(bad)?;
            at += n;
            Ok(b)
        };
        if take(8)? != BLUEPRINT_MAGIC {
            return Err(bad());
        }
        let len = u32::from_le_bytes(take(4)?.try_into().expect("4")) as usize;
        let settings = String::from_utf8(take(len)?.to_vec()).map_err(|_| bad())?;
        let count = u64::from_le_bytes(take(8)?.try_into().expect("8")) as usize;
        let mut rows: Vec<(u64, &[u8])> = Vec::with_capacity(count.min(bytes.len() / 9));
        for _ in 0..count {
            let key = u64::from_le_bytes(take(8)?.try_into().expect("8"));
            let n = take(1)?[0] as usize;
            rows.push((key, take(n)?));
        }
        if at != bytes.len() {
            return Err(bad());
        }
        rows.sort_unstable_by_key(|r| r.0);
        let mut b = Self {
            settings,
            keys: Vec::with_capacity(count),
            starts: Vec::with_capacity(count + 1),
            probs: Vec::new(),
        };
        for (key, p) in rows {
            b.keys.push(key);
            b.starts.push(b.probs.len() as u32);
            b.probs.extend_from_slice(p);
        }
        b.starts.push(b.probs.len() as u32);
        Ok(b)
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The probabilities at `info` (which has `n` actions), uniform if the
    /// blueprint never saw it.
    pub fn probs(&self, info: u64, n: usize) -> Vec<f64> {
        match self.keys.binary_search(&info) {
            Ok(i) if (self.starts[i + 1] - self.starts[i]) as usize == n => {
                let p = &self.probs[self.starts[i] as usize..self.starts[i + 1] as usize];
                let total: f64 = p.iter().map(|&x| x as f64).sum();
                if total > 0.0 {
                    p.iter().map(|&x| x as f64 / total).collect()
                } else {
                    vec![1.0 / n as f64; n]
                }
            }
            _ => vec![1.0 / n as f64; n],
        }
    }
}

/// Probabilities as bytes summing to 255 (largest remainders get the
/// leftover units).
fn quantize(probs: &[f64]) -> Vec<u8> {
    let scaled: Vec<f64> = probs.iter().map(|p| p.max(0.0) * 255.0).collect();
    let mut out: Vec<u8> = scaled.iter().map(|x| x.floor() as u8).collect();
    let short = 255u32.saturating_sub(out.iter().map(|&x| x as u32).sum::<u32>());
    let mut order: Vec<usize> = (0..out.len()).collect();
    order.sort_by(|&a, &b| {
        (scaled[b] - scaled[b].floor()).total_cmp(&(scaled[a] - scaled[a].floor()))
    });
    for &i in order.iter().cycle().take(short as usize) {
        out[i] = out[i].saturating_add(1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Config, Discount, Mccfr,
        holdem::{abstraction::CardAbstraction, cards::parse},
    };

    type Holdem<'a> = MultiwayGame<'a, CardAbstraction>;

    fn play(game: &Holdem, s: &MultiwayState, a: MwAction) -> MultiwayState {
        let node = &game.tree.nodes[s.node as usize];
        let k = (0..node.actions as usize)
            .find(|&k| game.tree.nodes[node.first_child as usize + k].via() == a)
            .unwrap_or_else(|| panic!("{a:?} not on the menu"));
        game.apply(s, k)
    }

    fn deal(game: &Holdem, holes: [&str; 6], board: &str) -> MultiwayState {
        let hole = holes.map(|h| {
            let c = parse(h).expect("cards");
            [c[0], c[1]]
        });
        let b = parse(board).expect("board");
        game.deal(hole, [b[0], b[1], b[2], b[3], b[4]])
    }

    const HOLES: [&str; 6] = ["As Ah", "Ks Kh", "Qs Qh", "Js Jh", "Ts 9s", "7c 2d"];
    const BOARD: &str = "2c 3d 8h Tc 4s";

    #[test]
    fn folds_around_and_raises() {
        let game = Holdem::new(MultiwayConfig::holdem(100), None);
        let mut s = deal(&game, HOLES, BOARD);
        assert_eq!(game.turn(&s), Turn::Player(0));
        // Only the small blind may limp.
        let c = &game.config;
        assert_eq!(s.betting.actions(c), [MwAction::Fold, MwAction::Raise(5)]);
        let sb = (0..4).fold(s.betting.clone(), |b, _| b.play(MwAction::Fold));
        assert_eq!(sb.to_act, 4);
        assert_eq!(
            sb.actions(c),
            [MwAction::Fold, MwAction::Call, MwAction::Raise(5)]
        );
        for _ in 0..5 {
            s = play(&game, &s, MwAction::Fold);
        }
        assert_eq!(game.turn(&s), Turn::Terminal);
        assert_eq!(game.payoffs(&s), [0.0, 0.0, 0.0, 0.0, -1.0, 1.0]);

        // UTG opens to 2.5 BB and everyone folds.
        let mut s = deal(&game, HOLES, BOARD);
        s = play(&game, &s, MwAction::Raise(5));
        for _ in 0..5 {
            s = play(&game, &s, MwAction::Fold);
        }
        assert_eq!(game.payoffs(&s), [3.0, 0.0, 0.0, 0.0, -1.0, -2.0]);
    }

    #[test]
    fn multiway_to_the_river_and_side_pots() {
        let game = Holdem::new(
            MultiwayConfig {
                limp: Limp::Any,
                ..MultiwayConfig::holdem(100)
            },
            None,
        );
        // Everyone limps, the big blind checks: six see the flop, and the
        // small blind acts first there.
        let mut s = deal(&game, HOLES, BOARD);
        for _ in 0..5 {
            s = play(&game, &s, MwAction::Call);
        }
        s = play(&game, &s, MwAction::Check);
        assert_eq!(s.betting.street, 1);
        assert_eq!(game.turn(&s), Turn::Player(4));
        // Checked down: the aces win 5 big blinds' worth from the others.
        while game.turn(&s) != Turn::Terminal {
            s = play(&game, &s, MwAction::Check);
        }
        assert_eq!(game.payoffs(&s), [10.0, -2.0, -2.0, -2.0, -2.0, -2.0]);

        // Side pots: the jacks are all-in for less, the aces cover. Set up
        // by hand: jacks put in 50, aces and kings 200, the rest folded.
        let mut s = deal(&game, HOLES, BOARD);
        s.betting.contributed = [200, 200, 0, 50, 1, 2];
        s.betting.folded = 1 << 2 | 1 << 4 | 1 << 5;
        s.betting.done = true;
        // Main pot 50 × 3 + 3 blinds = 153 to the aces, side pot 300 too.
        assert_eq!(game.payoffs(&s), [253.0, -200.0, 0.0, -50.0, -1.0, -2.0]);
        // Make the jacks the best hand: they win the main pot only.
        s.score[3] = u32::MAX;
        assert_eq!(game.payoffs(&s), [100.0, -200.0, 0.0, 103.0, -1.0, -2.0]);
    }

    #[test]
    fn tree_counts_match_the_built_tree() {
        let c = MultiwayConfig {
            stack: 40,
            ..MultiwayConfig::holdem(20)
        };
        let stats = MultiwayTree::count(&c, u64::MAX).expect("small");
        let tree = MultiwayTree::build(&c);
        assert_eq!(stats.nodes, tree.nodes.len() as u64);
        let decisions = tree.nodes.iter().filter(|n| n.actions > 0).count() as u64;
        assert_eq!(stats.sequences.iter().sum::<u64>(), decisions);
        assert!(stats.sequences[3] > 0, "reaches the river");
        assert!(stats.by_players[1][6] > 0, "six-way flops");
        assert!(MultiwayTree::count(&c, 10).is_none());
        // Pot-limit with PLO's menu builds too.
        let plo = MultiwayConfig {
            stack: 40,
            ..MultiwayConfig::pot_limit_omaha(20)
        };
        assert!(MultiwayTree::count(&plo, u64::MAX).is_some());
    }

    #[test]
    fn trains_and_stays_zero_sum() {
        let cards = CardAbstraction::quick(4);
        let game = Holdem::new(
            MultiwayConfig {
                stack: 20,
                ..MultiwayConfig::holdem(10)
            },
            Some(&cards),
        );
        let mut rng = Rng::new(3);
        for _ in 0..50 {
            let mut s = game.sample_chance(&game.root(), &mut rng);
            while let Turn::Player(_) = game.turn(&s) {
                let n = game.num_actions(&s);
                s = game.apply(&s, (rng.next_u64() % n as u64) as usize);
            }
            let total: f64 = game.payoffs(&s).iter().sum();
            assert!(total.abs() < 1e-9, "{:?}", game.payoffs(&s));
        }
        let mut m = Mccfr::new(
            &game,
            Config {
                seed: 1,
                batch: 64,
                discount: Discount::DCFR,
                prune: None,
            },
        );
        m.run(256);
        assert!(m.num_infosets() > 100);
        let mut bytes = Vec::new();
        MultiwayBlueprint::write("test", m.num_infosets(), |f| m.average_each(f), &mut bytes)
            .expect("write");
        let bp = MultiwayBlueprint::load(&bytes).expect("load");
        assert_eq!(bp.len(), m.num_infosets());
        assert_eq!(bp.settings, "test");
        // The root (UTG's first decision) with bucket 0.
        let n = game.tree.nodes[0].actions as usize;
        let want = m.average_at(&0).expect("seen");
        let got = bp.probs(0, n);
        for (a, b) in want.iter().zip(&got) {
            assert!((a - b).abs() < 0.01, "{want:?} vs {got:?}");
        }
        assert!(MultiwayBlueprint::load(&bytes[..bytes.len() - 1]).is_err());
    }

    #[test]
    fn table_buckets_match_one_hand_at_a_time() {
        let cards = CardAbstraction::quick(20);
        let game = Holdem::new(MultiwayConfig::holdem(100), Some(&cards));
        let mut rng = Rng::new(9);
        for _ in 0..20 {
            let s = game.sample_chance(&game.root(), &mut rng);
            for p in 0..SEATS {
                for (street, n) in [0usize, 3, 4, 5].into_iter().enumerate() {
                    let want = cards.hand_bucket(&s.hole[p], &s.board[..n]);
                    assert_eq!(s.buckets[p][street], want, "seat {p}, street {street}");
                }
            }
        }
    }

    #[test]
    fn quantized_probabilities_sum_to_255() {
        for p in [vec![1.0 / 3.0; 3], vec![0.5, 0.5], vec![0.999, 0.001, 0.0]] {
            assert_eq!(quantize(&p).iter().map(|&x| x as u32).sum::<u32>(), 255);
        }
    }
}
