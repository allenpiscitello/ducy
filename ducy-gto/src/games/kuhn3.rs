//! Three-player Kuhn poker (Szafron, Gibson and Sturtevant 2013): four cards
//! (J, Q, K, A), one each, an ante of 1 and a single bet of 1. Players act in
//! turn; once someone bets, each other player calls or folds once. The
//! highest card among the players still in takes the pot.
//!
//! The smallest three-player poker game, for checking that [`Mccfr`] trains
//! games with more than two players. It has a family of equilibria, and CFR
//! approaches one in practice.
//!
//! [`Mccfr`]: crate::Mccfr

use crate::game::{Game, Turn};

pub use super::kuhn::{BET, PASS};

pub const PLAYERS: usize = 3;

/// Three-player Kuhn poker.
#[derive(Clone, Copy, Debug, Default)]
pub struct Kuhn3;

/// Cards dealt (0 = J … 3 = A) and the actions so far.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kuhn3State {
    pub cards: Option<[u8; PLAYERS]>,
    pub history: Vec<usize>,
}

impl Kuhn3State {
    /// Where the bet is in the history, if anyone has bet.
    fn bet_at(&self) -> Option<usize> {
        self.history.iter().position(|&a| a == BET)
    }
}

impl Kuhn3 {
    /// Every player's payoff at a terminal state.
    pub fn payoffs(&self, s: &Kuhn3State) -> [f64; PLAYERS] {
        let cards = s.cards.expect("dealt");
        let mut put_in = [1.0; PLAYERS];
        let mut live = [true; PLAYERS];
        if let Some(b) = s.bet_at() {
            live = [false; PLAYERS];
            live[b] = true;
            put_in[b] += 1.0;
            for (k, &a) in s.history[b + 1..].iter().enumerate() {
                let p = (b + 1 + k) % PLAYERS;
                if a == BET {
                    live[p] = true;
                    put_in[p] += 1.0;
                }
            }
        }
        let pot: f64 = put_in.iter().sum();
        let winner = (0..PLAYERS)
            .filter(|&p| live[p])
            .max_by_key(|&p| cards[p])
            .expect("someone is in");
        std::array::from_fn(|p| if p == winner { pot } else { 0.0 } - put_in[p])
    }
}

impl Game for Kuhn3 {
    type State = Kuhn3State;
    /// The player's card and the actions so far, e.g. `"A:pb"`.
    type Info = String;

    fn root(&self) -> Kuhn3State {
        Kuhn3State {
            cards: None,
            history: Vec::new(),
        }
    }

    fn turn(&self, s: &Kuhn3State) -> Turn {
        if s.cards.is_none() {
            return Turn::Chance;
        }
        let n = s.history.len();
        match s.bet_at() {
            None if n == PLAYERS => Turn::Terminal,
            None => Turn::Player(n),
            Some(b) if n - b - 1 == PLAYERS - 1 => Turn::Terminal,
            Some(_) => Turn::Player(n % PLAYERS),
        }
    }

    fn num_players(&self) -> usize {
        PLAYERS
    }

    fn utility(&self, s: &Kuhn3State) -> f64 {
        self.payoffs(s)[0]
    }

    fn utility_for(&self, s: &Kuhn3State, player: usize) -> f64 {
        self.payoffs(s)[player]
    }

    fn chance_outcomes(&self, _: &Kuhn3State) -> Vec<(Kuhn3State, f64)> {
        let mut out = Vec::with_capacity(24);
        for a in 0..4u8 {
            for b in (0..4u8).filter(|&c| c != a) {
                for c in (0..4u8).filter(|&c| c != a && c != b) {
                    out.push((
                        Kuhn3State {
                            cards: Some([a, b, c]),
                            history: Vec::new(),
                        },
                        1.0 / 24.0,
                    ));
                }
            }
        }
        out
    }

    fn num_actions(&self, _: &Kuhn3State) -> usize {
        2
    }

    fn apply(&self, s: &Kuhn3State, action: usize) -> Kuhn3State {
        let mut next = s.clone();
        next.history.push(action);
        next
    }

    fn info(&self, s: &Kuhn3State) -> String {
        let cards = s.cards.expect("dealt");
        let me = s.history.len() % PLAYERS;
        let mut key = String::from(["J", "Q", "K", "A"][cards[me] as usize]);
        key.push(':');
        key.extend(s.history.iter().map(|&a| if a == PASS { 'p' } else { 'b' }));
        key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(cards: [u8; 3], history: &[usize]) -> Kuhn3State {
        Kuhn3State {
            cards: Some(cards),
            history: history.to_vec(),
        }
    }

    #[test]
    fn turns_and_payoffs() {
        let g = Kuhn3;
        assert_eq!(g.turn(&state([0, 1, 2], &[])), Turn::Player(0));
        assert_eq!(g.turn(&state([0, 1, 2], &[PASS, PASS])), Turn::Player(2));
        // Everyone checks: the king takes the antes.
        let s = state([0, 1, 2], &[PASS, PASS, PASS]);
        assert_eq!(g.turn(&s), Turn::Terminal);
        assert_eq!(g.payoffs(&s), [-1.0, -1.0, 2.0]);
        // Player 1 bets; player 2 calls, then player 0 folds.
        assert_eq!(g.turn(&state([0, 1, 2], &[PASS, BET])), Turn::Player(2));
        assert_eq!(
            g.turn(&state([0, 1, 2], &[PASS, BET, BET])),
            Turn::Player(0)
        );
        let s = state([0, 1, 2], &[PASS, BET, BET, PASS]);
        assert_eq!(g.turn(&s), Turn::Terminal);
        assert_eq!(g.payoffs(&s), [-1.0, -2.0, 3.0]);
        // A bet everyone folds to.
        let s = state([0, 1, 2], &[BET, PASS, PASS]);
        assert_eq!(g.payoffs(&s), [2.0, -1.0, -1.0]);
        assert_eq!(g.chance_outcomes(&g.root()).len(), 24);
    }
}
