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

use super::{
    abstraction::CardAbstraction,
    cards::{Card, NUM_CARDS, bit, mask, score},
};
use crate::{
    game::{Game, Turn},
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

/// Blinds, stacks and bet sizes.
#[derive(Clone, Debug, PartialEq)]
pub struct HunlConfig {
    pub small_blind: u64,
    pub big_blind: u64,
    /// Each player's stack at the start of a hand, blinds included.
    pub stack: u64,
    pub menu: BetMenu,
}

impl Default for HunlConfig {
    /// Blinds 1/2, 100 big blind stacks, the default menu.
    fn default() -> Self {
        Self {
            small_blind: 1,
            big_blind: 2,
            stack: 200,
            menu: BetMenu::default(),
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

/// A hand in the abstract game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HunlState {
    /// Hole cards for player 0 and 1, and the whole board, dealt at the root.
    pub hole: [[Card; 2]; 2],
    pub board: [Card; 5],
    /// Each player's bucket on each street.
    pub buckets: [[u16; 4]; 2],
    dealt: bool,
    /// 0 preflop to 3 river.
    pub street: usize,
    pub stack: [u64; 2],
    pub street_bet: [u64; 2],
    pub contributed: [u64; 2],
    pub current_bet: u64,
    last_raise: u64,
    big_blind: u64,
    pub to_act: usize,
    acted: [bool; 2],
    /// Bets and raises this round (preflop, the big blind counts as one).
    raises: usize,
    folded: Option<usize>,
    done: bool,
    /// Action indices so far, with 255 between streets.
    pub history: Vec<u8>,
}

impl HunlState {
    pub fn is_over(&self) -> bool {
        self.done || self.folded.is_some()
    }

    fn to_call(&self) -> u64 {
        self.current_bet - self.street_bet[self.to_act]
    }

    fn all_in_to(&self) -> u64 {
        self.street_bet[self.to_act] + self.stack[self.to_act]
    }
}

/// Heads-up no-limit Hold'em, abstracted.
pub struct Hunl<'a> {
    pub config: HunlConfig,
    /// `None` puts every hand in bucket 0: just the betting tree, for tests
    /// and tree statistics.
    pub cards: Option<&'a CardAbstraction>,
}

impl<'a> Hunl<'a> {
    pub fn new(config: HunlConfig, cards: Option<&'a CardAbstraction>) -> Self {
        Self { config, cards }
    }

    /// A hand with these cards dealt and the blinds posted.
    pub fn deal(&self, hole: [[Card; 2]; 2], board: [Card; 5]) -> HunlState {
        let c = &self.config;
        let mut buckets = [[0u16; 4]; 2];
        if let Some(cards) = self.cards {
            for (p, b) in buckets.iter_mut().enumerate() {
                for (street, n) in [0usize, 3, 4, 5].into_iter().enumerate() {
                    b[street] = cards.bucket(hole[p], &board[..n]);
                }
            }
        }
        let sb = c.small_blind.min(c.stack);
        let bb = c.big_blind.min(c.stack);
        HunlState {
            hole,
            board,
            buckets,
            dealt: true,
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
            history: Vec::new(),
        }
    }

    /// The actions open at `s`, in action-index order: fold (when facing a
    /// bet), check or call, then bets or raises from smallest to all-in.
    pub fn actions(&self, s: &HunlState) -> Vec<HunlAction> {
        let me = s.to_act;
        let to_call = s.to_call();
        let mut out = Vec::with_capacity(6);
        if to_call > 0 {
            out.push(HunlAction::Fold);
            out.push(HunlAction::Call);
        } else {
            out.push(HunlAction::Check);
        }
        // No raising when the opponent is all-in or we can only just call.
        if s.stack[1 - me] == 0 || s.stack[me] <= to_call {
            return out;
        }
        let all_in = s.all_in_to();
        let min_to = (s.current_bet + s.last_raise).min(all_in);
        let menu = if s.street == 0 {
            &self.config.menu.preflop
        } else {
            &self.config.menu.postflop
        };
        let depth = if s.street == 0 {
            s.raises - 1
        } else {
            s.raises
        };
        let sizes = &menu[depth.min(menu.len() - 1)];
        let pot = s.contributed[0] + s.contributed[1];
        let mut tos: Vec<u64> = sizes
            .iter()
            .map(|&size| {
                let to = match size {
                    Size::Pot(f) => s.current_bet + (f * (pot + to_call) as f64).round() as u64,
                    Size::Bb(x) => (x * self.config.big_blind as f64).round() as u64,
                    Size::Times(x) => (x * s.current_bet as f64).round() as u64,
                    Size::AllIn => all_in,
                };
                to.clamp(min_to, all_in)
            })
            .collect();
        tos.sort_unstable();
        tos.dedup();
        for to in tos {
            out.push(if s.current_bet == 0 {
                HunlAction::Bet(to)
            } else {
                HunlAction::Raise(to)
            });
        }
        out
    }

    /// Plays `action` (one of [`Self::actions`]).
    pub fn play(&self, s: &HunlState, action: HunlAction, index: usize) -> HunlState {
        let mut n = s.clone();
        let me = s.to_act;
        n.history.push(index as u8);
        match action {
            HunlAction::Fold => {
                n.folded = Some(me);
                return n;
            }
            HunlAction::Check => {}
            HunlAction::Call => {
                let amount = s.to_call().min(s.stack[me]);
                n.put_in(me, amount);
            }
            HunlAction::Bet(to) | HunlAction::Raise(to) => {
                let increase = to - s.current_bet;
                n.put_in(me, to - s.street_bet[me]);
                n.current_bet = to;
                if increase >= s.last_raise {
                    n.last_raise = increase;
                }
                n.raises += 1;
                n.acted[1 - me] = false;
            }
        }
        n.acted[me] = true;
        let other = 1 - me;
        let needs = |st: &HunlState, p: usize| {
            st.stack[p] > 0
                && (st.street_bet[p] < st.current_bet || (!st.acted[p] && st.stack[1 - p] > 0))
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
}

impl HunlState {
    fn put_in(&mut self, p: usize, amount: u64) {
        let amount = amount.min(self.stack[p]);
        self.stack[p] -= amount;
        self.street_bet[p] += amount;
        self.contributed[p] += amount;
    }

    /// Ends the betting round: deals on to the next street where someone can
    /// act, or to the showdown.
    fn next_street(&mut self) {
        loop {
            if self.street == 3 {
                self.done = true;
                return;
            }
            self.street += 1;
            self.history.push(255);
            self.street_bet = [0, 0];
            self.current_bet = 0;
            self.raises = 0;
            self.acted = [false, false];
            // After the flop the big blind (player 1) acts first.
            if self.stack[0] > 0 && self.stack[1] > 0 {
                self.to_act = 1;
                self.last_raise = self.big_blind;
                return;
            }
        }
    }
}

impl Game for Hunl<'_> {
    type State = HunlState;
    /// The street, the player's bucket on it, then the action indices so far.
    type Info = Vec<u8>;

    fn root(&self) -> HunlState {
        let blank = Hunl::new(self.config.clone(), None);
        let mut s = blank.deal([[0, 1], [2, 3]], [4, 5, 6, 7, 8]);
        s.dealt = false;
        s
    }

    fn turn(&self, s: &HunlState) -> Turn {
        if !s.dealt {
            Turn::Chance
        } else if s.is_over() {
            Turn::Terminal
        } else {
            Turn::Player(s.to_act)
        }
    }

    fn utility(&self, s: &HunlState) -> f64 {
        if let Some(p) = s.folded {
            return if p == 0 {
                -(s.contributed[0] as f64)
            } else {
                s.contributed[1] as f64
            };
        }
        let board = mask(&s.board);
        let mine = score(board | bit(s.hole[0][0]) | bit(s.hole[0][1]));
        let theirs = score(board | bit(s.hole[1][0]) | bit(s.hole[1][1]));
        let at_stake = s.contributed[0].min(s.contributed[1]) as f64;
        match mine.cmp(&theirs) {
            std::cmp::Ordering::Greater => at_stake,
            std::cmp::Ordering::Less => -at_stake,
            std::cmp::Ordering::Equal => 0.0,
        }
    }

    /// Too many deals to list; see [`Game::sample_chance`].
    fn chance_outcomes(&self, _: &HunlState) -> Vec<(HunlState, f64)> {
        Vec::new()
    }

    fn sample_chance(&self, _: &HunlState, rng: &mut Rng) -> HunlState {
        let mut cards = [0u8; 9];
        let mut used = 0u64;
        for c in cards.iter_mut() {
            loop {
                let x = (rng.next_u64() % NUM_CARDS as u64) as Card;
                if used & bit(x) == 0 {
                    used |= bit(x);
                    *c = x;
                    break;
                }
            }
        }
        self.deal(
            [[cards[0], cards[1]], [cards[2], cards[3]]],
            [cards[4], cards[5], cards[6], cards[7], cards[8]],
        )
    }

    fn num_actions(&self, s: &HunlState) -> usize {
        self.actions(s).len()
    }

    fn apply(&self, s: &HunlState, action: usize) -> HunlState {
        let a = self.actions(s)[action];
        self.play(s, a, action)
    }

    fn info(&self, s: &HunlState) -> Vec<u8> {
        let b = s.buckets[s.to_act][s.street];
        let mut key = Vec::with_capacity(3 + s.history.len());
        key.push(s.street as u8);
        key.extend(b.to_le_bytes());
        key.extend(&s.history);
        key
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
    pub fn infosets(&self, cards: &CardAbstraction) -> [u64; 4] {
        let mut out = [0; 4];
        for (street, n) in [0usize, 3, 4, 5].into_iter().enumerate() {
            out[street] = self.sequences[street] * cards.num_buckets(n) as u64;
        }
        out
    }

    /// Bytes for a regret and a strategy sum (f32 each) per action of every
    /// information set.
    pub fn table_bytes(&self, cards: &CardAbstraction) -> u64 {
        let mut total = 0;
        for (street, n) in [0usize, 3, 4, 5].into_iter().enumerate() {
            total += self.actions[street] * cards.num_buckets(n) as u64 * 8;
        }
        total
    }
}

impl Hunl<'_> {
    /// Walks the whole betting tree once.
    pub fn tree_stats(&self) -> TreeStats {
        fn walk(g: &Hunl, s: &HunlState, out: &mut TreeStats) {
            if s.is_over() {
                return;
            }
            let actions = g.actions(s);
            out.sequences[s.street] += 1;
            out.actions[s.street] += actions.len() as u64;
            for (i, &a) in actions.iter().enumerate() {
                walk(g, &g.play(s, a, i), out);
            }
        }
        let mut out = TreeStats::default();
        let blank = Hunl::new(self.config.clone(), None);
        walk(
            &blank,
            &blank.deal([[0, 1], [2, 3]], [4, 5, 6, 7, 8]),
            &mut out,
        );
        out
    }
}
