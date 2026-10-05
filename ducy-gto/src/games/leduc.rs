//! Leduc hold'em: a six-card deck (J, Q, K in two suits), one private card
//! each, an ante of 1, and two betting rounds with a public card dealt
//! between them. Bets are 2 in the first round and 4 in the second, with at
//! most a bet and a raise per round. A pair with the board wins, otherwise
//! the higher card; equal cards split.
//!
//! Small enough to solve exactly (288 information sets), yet it has the
//! things that make Hold'em hard: hidden cards, a board that changes hand
//! values, and more than one betting round.

use crate::game::{Game, Turn};

/// Player 0's game value at equilibrium, about −0.0856 (Southey et al. 2005).
pub const GAME_VALUE: f64 = -0.0856;

const RANKS: [char; 3] = ['J', 'Q', 'K'];
const BET: [u32; 2] = [2, 4];
const MAX_RAISES: u8 = 2;

/// Leduc hold'em.
#[derive(Clone, Copy, Debug, Default)]
pub struct Leduc;

/// A Leduc hand in progress. Cards are 0..6; a card's rank is `card / 2`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeducState {
    pub cards: Option<[u8; 2]>,
    pub board: Option<u8>,
    pub round: usize,
    /// Every action so far: `k` check, `b` bet, `c` call, `r` raise, `f`
    /// fold, with `/` between the rounds.
    pub history: String,
    /// Chips each player has put in, antes included.
    pub contrib: [u32; 2],
    pub to_act: usize,
    raises: u8,
    acted: bool,
    folded: Option<usize>,
    done: bool,
}

impl LeducState {
    fn facing(&self) -> bool {
        self.contrib[self.to_act] < self.contrib[1 - self.to_act]
    }

    /// The actions open to the player to act, in action-index order.
    pub fn actions(&self) -> &'static [char] {
        match (self.facing(), self.raises < MAX_RAISES) {
            (false, _) => &['k', 'b'],
            (true, true) => &['f', 'c', 'r'],
            (true, false) => &['f', 'c'],
        }
    }

    fn end_round(&mut self) {
        if self.round == 0 {
            self.round = 1;
            self.history.push('/');
            self.raises = 0;
            self.acted = false;
            self.to_act = 0;
        } else {
            self.done = true;
        }
    }
}

impl Game for Leduc {
    type State = LeducState;
    /// The player's card, the board card (or `?`) and the betting, e.g.
    /// `"QK:kbc/b"`.
    type Info = String;

    fn root(&self) -> LeducState {
        LeducState {
            cards: None,
            board: None,
            round: 0,
            history: String::new(),
            contrib: [1, 1],
            to_act: 0,
            raises: 0,
            acted: false,
            folded: None,
            done: false,
        }
    }

    fn turn(&self, s: &LeducState) -> Turn {
        if s.cards.is_none() || (s.round == 1 && s.board.is_none() && !s.done && s.folded.is_none())
        {
            Turn::Chance
        } else if s.done || s.folded.is_some() {
            Turn::Terminal
        } else {
            Turn::Player(s.to_act)
        }
    }

    fn utility(&self, s: &LeducState) -> f64 {
        if let Some(p) = s.folded {
            return if p == 0 {
                -(s.contrib[0] as f64)
            } else {
                s.contrib[1] as f64
            };
        }
        let [c0, c1] = s.cards.expect("dealt");
        let b = s.board.expect("board dealt") / 2;
        let (r0, r1) = (c0 / 2, c1 / 2);
        let win = if r0 == b {
            1.0
        } else if r1 == b {
            -1.0
        } else if r0 != r1 {
            if r0 > r1 { 1.0 } else { -1.0 }
        } else {
            0.0
        };
        win * s.contrib[0] as f64
    }

    fn chance_outcomes(&self, s: &LeducState) -> Vec<(LeducState, f64)> {
        let mut out = Vec::new();
        match s.cards {
            None => {
                for c0 in 0..6u8 {
                    for c1 in (0..6u8).filter(|&c| c != c0) {
                        let mut next = s.clone();
                        next.cards = Some([c0, c1]);
                        out.push((next, 1.0 / 30.0));
                    }
                }
            }
            Some([c0, c1]) => {
                for b in (0..6u8).filter(|&c| c != c0 && c != c1) {
                    let mut next = s.clone();
                    next.board = Some(b);
                    out.push((next, 1.0 / 4.0));
                }
            }
        }
        out
    }

    fn num_actions(&self, s: &LeducState) -> usize {
        s.actions().len()
    }

    fn apply(&self, s: &LeducState, action: usize) -> LeducState {
        let mut next = s.clone();
        let a = s.actions()[action];
        next.history.push(a);
        let me = s.to_act;
        match a {
            'k' => {
                if s.acted {
                    next.end_round();
                } else {
                    next.acted = true;
                    next.to_act = 1 - me;
                }
            }
            'b' | 'r' => {
                next.contrib[me] = s.contrib[1 - me] + BET[s.round];
                next.raises += 1;
                next.acted = true;
                next.to_act = 1 - me;
            }
            'c' => {
                next.contrib[me] = s.contrib[1 - me];
                next.end_round();
            }
            'f' => next.folded = Some(me),
            _ => unreachable!(),
        }
        next
    }

    fn info(&self, s: &LeducState) -> String {
        let cards = s.cards.expect("dealt");
        let mut key = String::new();
        key.push(RANKS[(cards[s.to_act] / 2) as usize]);
        key.push(s.board.map_or('?', |b| RANKS[(b / 2) as usize]));
        key.push(':');
        key.push_str(&s.history);
        key
    }
}
