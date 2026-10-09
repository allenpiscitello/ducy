//! The abstract heads-up no-limit Hold'em game the blueprint is trained on.
//!
//! The real game has a bet of every size at every point. Here each player
//! chooses from a short menu ([`BetMenu`]): fold, check or call, a few bet or
//! raise sizes, and all-in, with fewer options deeper in a betting round.
//! Cards enter only through the [`CardAbstraction`]: a player's information
//! set is their bucket on the current street plus the betting so far.
//!
//! The betting follows ducy-play's rules exactly: blinds, minimum raises,
//! all-ins, the button acting first preflop and last after the flop, so every
//! abstract action is a legal action in a real `ducy_play::Hand`. Both players
//! start each hand with the same stack. Player 0 is the button (small blind).
//!
//! The betting doesn't depend on the cards, so it's compiled once into a
//! [`BettingTree`] of numbered nodes. A hand in progress is just the deal and
//! a node number, and an information set is a node and a bucket, which makes
//! training fast.

use super::{
    abstraction::CardAbstraction,
    cards::{Card, NUM_CARDS, bit, showdown_score},
};
use crate::{
    game::{Game, Turn},
    omaha::showdown::RiverBoard,
    rng::Rng,
};

/// A bet or raise size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Size {
    /// Raise by this fraction of the pot after calling (1.0 is a pot-sized
    /// bet or raise).
    Pot(f64),
    /// Raise to this many big blinds (for preflop opens).
    Bb(f64),
    /// Raise to this multiple of the current bet (for preflop re-raises).
    Times(f64),
    AllIn,
}

/// The sizes on offer, by how many bets and raises the round has seen. The
/// last list repeats for deeper raises. Preflop, the big blind counts as the
/// first bet, so `preflop[0]` is the open.
#[derive(Clone, Debug, PartialEq)]
pub struct BetMenu {
    pub preflop: Vec<Vec<Size>>,
    pub postflop: Vec<Vec<Size>>,
}

impl Default for BetMenu {
    /// Preflop: open to 2.5 big blinds, 3-bet to 3×, 4-bet to 2.3×, then
    /// all-in. After the flop: bet a third, three quarters or 1.25× the pot,
    /// raise the pot, then all-in. All-in is always on the menu.
    fn default() -> Self {
        use Size::*;
        Self {
            preflop: vec![
                vec![Bb(2.5), AllIn],
                vec![Times(3.0), AllIn],
                vec![Times(2.3), AllIn],
                vec![AllIn],
            ],
            postflop: vec![
                vec![Pot(0.33), Pot(0.75), Pot(1.25), AllIn],
                vec![Pot(1.0), AllIn],
                vec![AllIn],
            ],
        }
    }
}

impl BetMenu {
    /// For pot-limit Omaha. Preflop: open to 2.5 big blinds or the pot (3
    /// heads-up), 3-bet the pot or ⅔ of it, 4-bet the pot, then the pot again.
    /// After the flop: bet a third, half, two thirds or the pot, raise ⅔ or the
    /// pot, then the pot. `Pot(1.0)` and `AllIn` both mean the largest legal
    /// bet, so all-in is only on offer once the pot covers the stack.
    pub fn pot_limit() -> Self {
        use Size::*;
        Self {
            preflop: vec![
                vec![Bb(2.5), Pot(1.0)],
                vec![Pot(0.67), Pot(1.0)],
                vec![Pot(1.0)],
            ],
            postflop: vec![
                vec![Pot(0.33), Pot(0.5), Pot(0.67), Pot(1.0)],
                vec![Pot(0.67), Pot(1.0)],
                vec![Pot(1.0)],
            ],
        }
    }

    /// A lean pot-limit menu that one machine can train (#130): every
    /// preflop raise is pot-sized; after the flop, bet half the pot or the
    /// pot, and raise the pot. At 100 big blinds its tree has 9,128 nodes and
    /// 2,842 river betting sequences, against 84,428 for
    /// [`BetMenu::pot_limit`].
    pub fn pot_limit_lean() -> Self {
        use Size::*;
        Self {
            preflop: vec![vec![Pot(1.0)]],
            postflop: vec![vec![Pot(0.5), Pot(1.0)], vec![Pot(1.0)]],
        }
    }
}

/// Blinds, stacks and bet sizes.
#[derive(Clone, Debug, PartialEq)]
pub struct HunlConfig {
    pub small_blind: u64,
    pub big_blind: u64,
    /// Each player's stack at the start of a hand, blinds included.
    pub stack: u64,
    pub menu: BetMenu,
    /// Pot-limit: no bet or raise bigger than the pot after calling, as in
    /// ducy-play's [`BettingStructure::PotLimit`]. All-in is then only legal
    /// when it fits under that. Otherwise no-limit.
    ///
    /// [`BettingStructure::PotLimit`]: ducy_play::BettingStructure::PotLimit
    pub pot_limit: bool,
}

impl HunlConfig {
    /// Heads-up pot-limit Omaha: blinds 1/2, 100 big blind stacks, the
    /// [`BetMenu::pot_limit`] menu.
    pub fn pot_limit_omaha() -> Self {
        Self {
            menu: BetMenu::pot_limit(),
            pot_limit: true,
            ..Self::default()
        }
    }

    /// The PLO game the blueprint trains on: blinds 1/2, `big_blinds` deep,
    /// the [`BetMenu::pot_limit_lean`] menu.
    pub fn pot_limit_omaha_lean(big_blinds: u64) -> Self {
        Self {
            stack: 2 * big_blinds,
            menu: BetMenu::pot_limit_lean(),
            pot_limit: true,
            ..Self::default()
        }
    }
}

impl Default for HunlConfig {
    /// No-limit, blinds 1/2, 100 big blind stacks, the default menu.
    fn default() -> Self {
        Self {
            small_blind: 1,
            big_blind: 2,
            stack: 200,
            menu: BetMenu::default(),
            pot_limit: false,
        }
    }
}

/// An action at a node: what the player does, with amounts as the street
/// total ("raise to"), as in ducy-play.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HunlAction {
    Fold,
    Check,
    Call,
    Bet(u64),
    Raise(u64),
}

/// The betting part of a hand: chips, whose turn, and the round so far.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Betting {
    /// 0 preflop to 3 river.
    pub street: usize,
    pub stack: [u64; 2],
    pub street_bet: [u64; 2],
    pub contributed: [u64; 2],
    pub current_bet: u64,
    pub last_raise: u64,
    big_blind: u64,
    pub to_act: usize,
    acted: [bool; 2],
    /// Bets and raises this round (preflop, the big blind counts as one).
    pub raises: usize,
    pub folded: Option<usize>,
    done: bool,
}

impl Betting {
    /// Blinds posted, the button (player 0) to act.
    pub fn start(c: &HunlConfig) -> Self {
        let sb = c.small_blind.min(c.stack);
        let bb = c.big_blind.min(c.stack);
        Self {
            street: 0,
            stack: [c.stack - sb, c.stack - bb],
            street_bet: [sb, bb],
            contributed: [sb, bb],
            current_bet: c.big_blind,
            last_raise: c.big_blind,
            big_blind: c.big_blind,
            to_act: 0,
            acted: [false; 2],
            raises: 1,
            folded: None,
            done: false,
        }
    }

    /// The start of a betting round after the flop: `contributed` chips in
    /// the pot, `stack` behind, nothing bet yet, the big blind (player 1) to
    /// act. For solving one street from where a real hand stands.
    pub fn street_start(
        street: usize,
        stack: [u64; 2],
        contributed: [u64; 2],
        big_blind: u64,
    ) -> Self {
        Self {
            street,
            stack,
            street_bet: [0, 0],
            contributed,
            current_bet: 0,
            last_raise: big_blind,
            big_blind,
            to_act: 1,
            acted: [false; 2],
            raises: 0,
            folded: None,
            done: stack[0] == 0 || stack[1] == 0,
        }
    }

    /// A bet or raise to `to` as a fraction of the pot after calling.
    pub fn pot_fraction(&self, to: u64) -> f64 {
        pot_fraction(to, self.current_bet, self.to_call(), self.pot())
    }

    pub fn is_over(&self) -> bool {
        self.done || self.folded.is_some()
    }

    pub fn pot(&self) -> u64 {
        self.contributed[0] + self.contributed[1]
    }

    pub fn to_call(&self) -> u64 {
        self.current_bet - self.street_bet[self.to_act]
    }

    /// The street total that puts the player to act all-in.
    pub fn all_in_to(&self) -> u64 {
        self.street_bet[self.to_act] + self.stack[self.to_act]
    }

    /// The largest legal bet or raise: all-in, or under pot-limit the pot
    /// after calling if that's less.
    pub fn max_to(&self, pot_limit: bool) -> u64 {
        let all_in = self.all_in_to();
        if pot_limit {
            all_in.min(self.current_bet + self.pot() + self.to_call())
        } else {
            all_in
        }
    }

    /// The smallest legal bet or raise (all-in if that's less).
    pub fn min_to(&self) -> u64 {
        (self.current_bet + self.last_raise).min(self.all_in_to())
    }

    /// Whether the player to act may bet or raise at all.
    pub fn can_raise(&self) -> bool {
        let me = self.to_act;
        self.stack[1 - me] > 0 && self.stack[me] > self.to_call()
    }

    /// The actions on `menu` at this point, in action-index order: fold
    /// (when facing a bet), check or call, then bets or raises from smallest
    /// to all-in.
    pub fn actions(&self, c: &HunlConfig) -> Vec<HunlAction> {
        let to_call = self.to_call();
        let mut out = Vec::with_capacity(6);
        if to_call > 0 {
            out.push(HunlAction::Fold);
            out.push(HunlAction::Call);
        } else {
            out.push(HunlAction::Check);
        }
        if !self.can_raise() {
            return out;
        }
        // Under pot-limit the cap is the pot, and all-in means the cap.
        let all_in = self.max_to(c.pot_limit);
        let min_to = self.min_to().min(all_in);
        let menu = if self.street == 0 {
            &c.menu.preflop
        } else {
            &c.menu.postflop
        };
        let depth = if self.street == 0 {
            self.raises - 1
        } else {
            self.raises
        };
        let sizes = &menu[depth.min(menu.len() - 1)];
        let pot = self.pot();
        let mut tos: Vec<u64> = sizes
            .iter()
            .map(|&size| {
                let to = match size {
                    Size::Pot(f) => self.current_bet + (f * (pot + to_call) as f64).round() as u64,
                    Size::Bb(x) => (x * c.big_blind as f64).round() as u64,
                    Size::Times(x) => (x * self.current_bet as f64).round() as u64,
                    Size::AllIn => all_in,
                };
                to.clamp(min_to, all_in)
            })
            .collect();
        tos.sort_unstable();
        tos.dedup();
        for to in tos {
            out.push(if self.current_bet == 0 {
                HunlAction::Bet(to)
            } else {
                HunlAction::Raise(to)
            });
        }
        out
    }

    /// The betting after `action`, which must be legal here (any legal
    /// amount, not only the menu's).
    pub fn play(&self, action: HunlAction) -> Self {
        let mut n = self.clone();
        let me = self.to_act;
        match action {
            HunlAction::Fold => {
                n.folded = Some(me);
                return n;
            }
            HunlAction::Check => {}
            HunlAction::Call => {
                let amount = self.to_call().min(self.stack[me]);
                n.put_in(me, amount);
            }
            HunlAction::Bet(to) | HunlAction::Raise(to) => {
                let increase = to - self.current_bet;
                n.put_in(me, to - self.street_bet[me]);
                n.current_bet = to;
                if increase >= self.last_raise {
                    n.last_raise = increase;
                }
                n.raises += 1;
                n.acted[1 - me] = false;
            }
        }
        n.acted[me] = true;
        let other = 1 - me;
        let needs = |b: &Betting, p: usize| {
            b.stack[p] > 0
                && (b.street_bet[p] < b.current_bet || (!b.acted[p] && b.stack[1 - p] > 0))
        };
        if needs(&n, other) {
            n.to_act = other;
        } else if needs(&n, me) {
            n.to_act = me;
        } else {
            n.next_street();
        }
        n
    }

    fn put_in(&mut self, p: usize, amount: u64) {
        let amount = amount.min(self.stack[p]);
        self.stack[p] -= amount;
        self.street_bet[p] += amount;
        self.contributed[p] += amount;
    }

    /// Ends the betting round: on to the next street where someone can act,
    /// or to the showdown.
    fn next_street(&mut self) {
        loop {
            if self.street == 3 {
                self.done = true;
                return;
            }
            self.street += 1;
            self.street_bet = [0, 0];
            self.current_bet = 0;
            self.raises = 0;
            self.acted = [false, false];
            self.last_raise = self.big_blind;
            // After the flop the big blind (player 1) acts first.
            if self.stack[0] > 0 && self.stack[1] > 0 {
                self.to_act = 1;
                return;
            }
        }
    }
}

/// A bet or raise to `to` as a fraction of the pot after calling, given the
/// current bet, what the bettor has to call, and the pot before the action.
pub fn pot_fraction(to: u64, current_bet: u64, to_call: u64, pot: u64) -> f64 {
    (to.saturating_sub(current_bet)) as f64 / (pot + to_call).max(1) as f64
}

/// One point in the betting tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub betting: Betting,
    /// The menu's actions here (empty once the hand is over).
    pub actions: Vec<HunlAction>,
    /// The node each action leads to.
    pub children: Vec<u32>,
}

/// Every betting sequence the menu allows, numbered; node 0 is the start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BettingTree {
    pub nodes: Vec<Node>,
}

impl BettingTree {
    pub fn build(c: &HunlConfig) -> Self {
        fn add(c: &HunlConfig, b: Betting, nodes: &mut Vec<Node>) -> u32 {
            let id = nodes.len() as u32;
            let actions = if b.is_over() {
                Vec::new()
            } else {
                b.actions(c)
            };
            nodes.push(Node {
                betting: b.clone(),
                actions: actions.clone(),
                children: Vec::new(),
            });
            let children = actions.iter().map(|&a| add(c, b.play(a), nodes)).collect();
            nodes[id as usize].children = children;
            id
        }
        let mut nodes = Vec::new();
        add(c, Betting::start(c), &mut nodes);
        Self { nodes }
    }

    /// Decision points and actions per street.
    pub fn stats(&self) -> TreeStats {
        let mut out = TreeStats::default();
        for n in self.nodes.iter().filter(|n| !n.actions.is_empty()) {
            out.sequences[n.betting.street] += 1;
            out.actions[n.betting.street] += n.actions.len() as u64;
        }
        out
    }
}

/// How the abstract game sees the cards: a bucket for a hand on a board.
/// Hold'em's [`CardAbstraction`] is one; Omaha brings its own.
pub trait Buckets {
    /// The bucket of `hole` on `board` (0, 3, 4 or 5 cards).
    fn hand_bucket(&self, hole: &[Card], board: &[Card]) -> u16;
    /// How many buckets there are with `board_len` board cards.
    fn bucket_count(&self, board_len: usize) -> usize;
    /// The settings that shape the buckets, for a blueprint's settings
    /// hash: two abstractions with the same text must bucket alike.
    fn settings(&self) -> String {
        String::new()
    }
    /// Both players' buckets on every street for one deal. By default each
    /// is [`Buckets::hand_bucket`]; an abstraction whose work is mostly per
    /// board (Omaha's) does that work once here.
    fn deal_buckets(&self, hole: [&[Card]; 2], board: &[Card; 5]) -> [[u16; 4]; 2] {
        let mut out = [[0u16; 4]; 2];
        for (p, b) in out.iter_mut().enumerate() {
            for (street, n) in [0usize, 3, 4, 5].into_iter().enumerate() {
                b[street] = self.hand_bucket(hole[p], &board[..n]);
            }
        }
        out
    }
}

impl Buckets for CardAbstraction {
    fn hand_bucket(&self, hole: &[Card], board: &[Card]) -> u16 {
        self.bucket([hole[0], hole[1]], board)
    }

    fn bucket_count(&self, board_len: usize) -> usize {
        self.num_buckets(board_len)
    }

    fn settings(&self) -> String {
        format!("{:?}", self.config)
    }
}

/// A hand in the abstract game: the deal, each player's buckets, and where
/// the betting is. `H` is the number of hole cards: 2 for Hold'em (the
/// default), 4 to 6 for Omaha.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HunlState<const H: usize = 2> {
    /// Hole cards for player 0 and 1, and the whole board, dealt at the root.
    pub hole: [[Card; H]; 2],
    pub board: [Card; 5],
    /// Each player's bucket on each street.
    pub buckets: [[u16; 4]; 2],
    /// Index into the betting tree.
    pub node: u32,
    /// Player 0's showdown result: 1 win, -1 loss, 0 split.
    pub showdown: i8,
    dealt: bool,
}

/// Heads-up no-limit Hold'em, abstracted; or, with `H` hole cards and an
/// Omaha abstraction `A`, heads-up Omaha (see [`Hunl::with_cards`]).
pub struct Hunl<'a, A = CardAbstraction, const H: usize = 2> {
    pub config: HunlConfig,
    /// `None` puts every hand in bucket 0: just the betting tree, for tests
    /// and tree statistics.
    pub cards: Option<&'a A>,
    pub tree: BettingTree,
}

/// Heads-up Omaha with four hole cards, abstracted by `A`.
pub type HuPlo<'a, A> = Hunl<'a, A, 4>;

impl<'a> Hunl<'a> {
    /// Hold'em with Hold'em's card abstraction.
    pub fn new(config: HunlConfig, cards: Option<&'a CardAbstraction>) -> Self {
        Self::with_cards(config, cards)
    }
}

impl<'a, A: Buckets, const H: usize> Hunl<'a, A, H> {
    /// The game for any hole-card count and abstraction, e.g.
    /// `HuPlo::with_cards(config, Some(&omaha_buckets))`.
    pub fn with_cards(config: HunlConfig, cards: Option<&'a A>) -> Self {
        let tree = BettingTree::build(&config);
        Self {
            config,
            cards,
            tree,
        }
    }

    /// A hand with these cards dealt and the blinds posted.
    pub fn deal(&self, hole: [[Card; H]; 2], board: [Card; 5]) -> HunlState<H> {
        let buckets = match self.cards {
            Some(cards) => cards.deal_buckets([&hole[0], &hole[1]], &board),
            None => [[0u16; 4]; 2],
        };
        let (mine, theirs) = if H == 2 {
            (
                showdown_score(&hole[0], &board),
                showdown_score(&hole[1], &board),
            )
        } else {
            // Omaha: the board's triples once for both hands.
            let river = RiverBoard::new(&board);
            (river.score(&hole[0]), river.score(&hole[1]))
        };
        HunlState {
            hole,
            board,
            buckets,
            node: 0,
            showdown: mine.cmp(&theirs) as i8,
            dealt: true,
        }
    }

    pub fn node(&self, s: &HunlState<H>) -> &Node {
        &self.tree.nodes[s.node as usize]
    }

    /// The betting at `s`.
    pub fn betting(&self, s: &HunlState<H>) -> &Betting {
        &self.node(s).betting
    }

    /// The actions open at `s`, in action-index order.
    pub fn actions(&self, s: &HunlState<H>) -> &[HunlAction] {
        &self.node(s).actions
    }

    /// Statistics of the betting tree.
    pub fn tree_stats(&self) -> TreeStats {
        self.tree.stats()
    }
}

impl<A: Buckets, const H: usize> Game for Hunl<'_, A, H> {
    type State = HunlState<H>;
    /// `node << 16 | bucket`: the betting so far and the player's bucket on
    /// this street.
    type Info = u64;

    fn root(&self) -> HunlState<H> {
        HunlState {
            hole: [
                std::array::from_fn(|i| i as Card),
                std::array::from_fn(|i| (H + i) as Card),
            ],
            board: std::array::from_fn(|i| (2 * H + i) as Card),
            buckets: [[0; 4]; 2],
            node: 0,
            showdown: 0,
            dealt: false,
        }
    }

    fn turn(&self, s: &HunlState<H>) -> Turn {
        if !s.dealt {
            return Turn::Chance;
        }
        let b = self.betting(s);
        if b.is_over() {
            Turn::Terminal
        } else {
            Turn::Player(b.to_act)
        }
    }

    fn utility(&self, s: &HunlState<H>) -> f64 {
        let b = self.betting(s);
        if let Some(p) = b.folded {
            return if p == 0 {
                -(b.contributed[0] as f64)
            } else {
                b.contributed[1] as f64
            };
        }
        s.showdown as f64 * b.contributed[0].min(b.contributed[1]) as f64
    }

    /// Too many deals to list; see [`Game::sample_chance`].
    fn chance_outcomes(&self, _: &HunlState<H>) -> Vec<(HunlState<H>, f64)> {
        Vec::new()
    }

    /// Both hands, then the board: the same draws as before for Hold'em, so
    /// seeded training is unchanged.
    fn sample_chance(&self, _: &HunlState<H>, rng: &mut Rng) -> HunlState<H> {
        let mut used = 0u64;
        let mut draw = || loop {
            let x = (rng.next_u64() % NUM_CARDS as u64) as Card;
            if used & bit(x) == 0 {
                used |= bit(x);
                return x;
            }
        };
        let hole: [[Card; H]; 2] = std::array::from_fn(|_| std::array::from_fn(|_| draw()));
        let board: [Card; 5] = std::array::from_fn(|_| draw());
        self.deal(hole, board)
    }

    fn num_actions(&self, s: &HunlState<H>) -> usize {
        self.node(s).actions.len()
    }

    fn apply(&self, s: &HunlState<H>, action: usize) -> HunlState<H> {
        let mut n = s.clone();
        n.node = self.node(s).children[action];
        n
    }

    fn info(&self, s: &HunlState<H>) -> u64 {
        let b = self.betting(s);
        let bucket = s.buckets[b.to_act][b.street];
        (s.node as u64) << 16 | bucket as u64
    }
}

/// Counts of the betting tree, ignoring cards: decision points (betting
/// sequences where someone acts) and actions, per street.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeStats {
    pub sequences: [u64; 4],
    pub actions: [u64; 4],
}

impl TreeStats {
    /// Information sets per street: betting sequences times buckets.
    pub fn infosets(&self, cards: &impl Buckets) -> [u64; 4] {
        let mut out = [0; 4];
        for (street, n) in [0usize, 3, 4, 5].into_iter().enumerate() {
            out[street] = self.sequences[street] * cards.bucket_count(n) as u64;
        }
        out
    }

    /// Bytes for a regret and a strategy sum (f32 each) per action of every
    /// information set.
    pub fn table_bytes(&self, cards: &impl Buckets) -> u64 {
        let mut total = 0;
        for (street, n) in [0usize, 3, 4, 5].into_iter().enumerate() {
            total += self.actions[street] * cards.bucket_count(n) as u64 * 8;
        }
        total
    }
}
