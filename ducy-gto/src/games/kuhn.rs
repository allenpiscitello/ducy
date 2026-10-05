//! Kuhn poker: three cards (J, Q, K), one each, an ante of 1 and a single
//! bet of 1. The smallest interesting poker game; its equilibria are known in
//! closed form and player 0 loses 1/18 per hand at any of them.

use crate::game::{Game, Turn};

/// Pass (check or fold) and bet (bet or call).
pub const PASS: usize = 0;
pub const BET: usize = 1;

/// Kuhn poker.
#[derive(Clone, Copy, Debug, Default)]
pub struct Kuhn;

/// Cards dealt (0 = J, 1 = Q, 2 = K) and the actions so far.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KuhnState {
    pub cards: Option<[u8; 2]>,
    pub history: Vec<usize>,
}

/// Player 0's game value at equilibrium.
pub const GAME_VALUE: f64 = -1.0 / 18.0;

impl Game for Kuhn {
    type State = KuhnState;
    /// The player's card and the actions so far, e.g. `"K:pb"`.
    type Info = String;

    fn root(&self) -> KuhnState {
        KuhnState {
            cards: None,
            history: Vec::new(),
        }
    }

    fn turn(&self, s: &KuhnState) -> Turn {
        if s.cards.is_none() {
            return Turn::Chance;
        }
        match s.history.as_slice() {
            [PASS, PASS] | [BET, _] | [PASS, BET, _] => Turn::Terminal,
            h => Turn::Player(h.len() % 2),
        }
    }

    fn utility(&self, s: &KuhnState) -> f64 {
        let [c0, c1] = s.cards.expect("dealt");
        let showdown = if c0 > c1 { 1.0 } else { -1.0 };
        match s.history.as_slice() {
            [PASS, PASS] => showdown,
            [BET, PASS] => 1.0,
            [PASS, BET, PASS] => -1.0,
            [BET, BET] | [PASS, BET, BET] => 2.0 * showdown,
            h => panic!("not terminal: {h:?}"),
        }
    }

    fn chance_outcomes(&self, _: &KuhnState) -> Vec<(KuhnState, f64)> {
        let mut out = Vec::with_capacity(6);
        for c0 in 0..3u8 {
            for c1 in (0..3u8).filter(|&c| c != c0) {
                out.push((
                    KuhnState {
                        cards: Some([c0, c1]),
                        history: Vec::new(),
                    },
                    1.0 / 6.0,
                ));
            }
        }
        out
    }

    fn num_actions(&self, _: &KuhnState) -> usize {
        2
    }

    fn apply(&self, s: &KuhnState, action: usize) -> KuhnState {
        let mut next = s.clone();
        next.history.push(action);
        next
    }

    fn info(&self, s: &KuhnState) -> String {
        let cards = s.cards.expect("dealt");
        let me = s.history.len() % 2;
        let mut key = String::from(["J", "Q", "K"][cards[me] as usize]);
        key.push(':');
        key.extend(s.history.iter().map(|&a| if a == PASS { 'p' } else { 'b' }));
        key
    }
}
