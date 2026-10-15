//! The six-handed preflop game (#249): six seats, a small raise menu and
//! the 169 starting-hand classes. Once the preflop betting closes, each
//! player still in gets a share of the pot from the equity tables, scaled by
//! how much of its equity its position lets it realize after the flop.
//!
//! Chips are counted in half big blinds: the small blind is 1, the big
//! blind 2. Seats are numbered by position: 0 UTG, 1 HJ, 2 CO, 3 BTN, 4 SB,
//! 5 BB. Preflop, UTG acts first; after the flop the small blind does, and
//! the button acts last.

use super::equity::PreflopEquity;

pub const SEATS: usize = 6;
pub const POSITIONS: [&str; SEATS] = ["UTG", "HJ", "CO", "BTN", "SB", "BB"];
const SB: usize = 4;
const BB: usize = 5;

/// Raises after the open, as multiples of the bet faced: the 3-bet (in
/// position, and out of position), then the 4-bet. Anything after is all-in.
const THREE_BET_IP: f64 = 3.0;
const THREE_BET_OOP: f64 = 3.5;
const FOUR_BET: f64 = 2.3;
/// The open, in half big blinds: 2.5 BB, or 3 BB from the small blind.
const OPEN: u32 = 5;
const OPEN_SB: u32 = 6;

/// How much of its equity a hand realizes after the flop, when it isn't
/// all-in: the player last to act after the flop realizes more, the others
/// less (first numbers; to be fitted from the heads-up blueprint).
pub const REALIZE_LAST: f64 = 1.08;
pub const REALIZE_OTHERS: f64 = 0.9;

/// An action at a decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Fold,
    /// Call, or check (the big blind with nothing to call).
    Call,
    /// Raise to this many half big blinds in all.
    Raise(u32),
    AllIn,
}

/// A node of the preflop tree.
#[derive(Clone, Debug)]
pub enum Node {
    Decision {
        seat: usize,
        /// Index of this decision among all decisions, for strategy tables.
        id: u32,
        actions: Vec<(Act, u32)>,
    },
    /// The betting is over: chips each seat put in, and who is still in.
    End {
        put_in: [u32; SEATS],
        live: [bool; SEATS],
        all_in: bool,
    },
}

#[derive(Clone, Copy)]
struct State {
    put_in: [u32; SEATS],
    folded: [bool; SEATS],
    /// Raises so far (the open is the first).
    raises: u32,
    /// The current bet to call.
    bet: u32,
    /// The last raiser, if any.
    raiser: Option<usize>,
    /// Seats that still have to act on the current bet.
    to_act: [bool; SEATS],
    /// Someone went all-in.
    all_in: bool,
}

/// Seats in their order after the flop: SB, BB, UTG, HJ, CO, BTN.
fn postflop_order(seat: usize) -> usize {
    (seat + 2) % SEATS
}

/// The whole preflop tree at one depth.
#[derive(Clone, Debug)]
pub struct PreflopGame {
    /// Stacks, in half big blinds.
    pub stack: u32,
    pub nodes: Vec<Node>,
    /// Number of decision nodes.
    pub decisions: u32,
}

impl PreflopGame {
    /// The tree with `big_blinds`-deep stacks.
    pub fn new(big_blinds: u32) -> Self {
        let mut g = Self {
            stack: 2 * big_blinds,
            nodes: Vec::new(),
            decisions: 0,
        };
        let mut put_in = [0; SEATS];
        put_in[SB] = 1;
        put_in[BB] = 2;
        let start = State {
            put_in,
            folded: [false; SEATS],
            raises: 0,
            bet: 2,
            raiser: None,
            to_act: [true; SEATS],
            all_in: false,
        };
        g.build(start, 0);
        g
    }

    /// The root is node 0.
    pub fn root(&self) -> usize {
        0
    }

    fn next_seat(s: &State, from: usize) -> Option<usize> {
        (1..=SEATS)
            .map(|i| (from + i) % SEATS)
            .find(|&x| s.to_act[x] && !s.folded[x])
    }

    fn build(&mut self, s: State, seat: usize) -> usize {
        let live = s.folded.map(|f| !f);
        let in_count = live.iter().filter(|&&l| l).count();
        if in_count == 1 || !s.to_act.iter().zip(&s.folded).any(|(&t, &f)| t && !f) {
            self.nodes.push(Node::End {
                put_in: s.put_in,
                live,
                all_in: s.all_in,
            });
            return self.nodes.len() - 1;
        }
        if !s.to_act[seat] || s.folded[seat] {
            let next = Self::next_seat(&s, seat).expect("someone to act");
            return self.build(s, next);
        }
        let idx = self.nodes.len();
        let id = self.decisions;
        self.decisions += 1;
        self.nodes.push(Node::Decision {
            seat,
            id,
            actions: Vec::new(),
        });
        let mut acts = Vec::new();
        let facing = s.bet > s.put_in[seat];
        if facing {
            acts.push(Act::Fold);
        }
        // No limping, except the small blind completing.
        let limp = s.raises == 0 && seat != BB && seat != SB;
        if !limp {
            acts.push(Act::Call);
        }
        let cap = self.stack;
        if !s.all_in && s.bet < cap {
            let to = match s.raises {
                0 if seat == SB => Some(OPEN_SB),
                0 => Some(OPEN),
                1 => {
                    let raiser = s.raiser.expect("an opener");
                    let oop = postflop_order(seat) < postflop_order(raiser);
                    let k = if oop { THREE_BET_OOP } else { THREE_BET_IP };
                    Some((s.bet as f64 * k).round() as u32)
                }
                2 => Some((s.bet as f64 * FOUR_BET).round() as u32),
                _ => None,
            };
            if let Some(to) = to.filter(|&to| to < cap) {
                acts.push(Act::Raise(to));
            }
            acts.push(Act::AllIn);
        }
        let mut children = Vec::with_capacity(acts.len());
        for &a in &acts {
            let mut t = s;
            match a {
                Act::Fold => {
                    t.folded[seat] = true;
                    t.to_act[seat] = false;
                }
                Act::Call => {
                    t.put_in[seat] = t.bet.min(cap);
                    t.to_act[seat] = false;
                }
                Act::Raise(_) | Act::AllIn => {
                    let to = if let Act::Raise(to) = a { to } else { cap };
                    t.put_in[seat] = to;
                    t.bet = to;
                    t.raises += 1;
                    t.raiser = Some(seat);
                    t.all_in |= to >= cap;
                    for x in 0..SEATS {
                        t.to_act[x] = x != seat && !t.folded[x];
                    }
                }
            }
            let child = match Self::next_seat(&t, seat) {
                Some(next) => self.build(t, next),
                None => self.build(t, seat),
            };
            children.push((a, child as u32));
        }
        if let Node::Decision { actions, .. } = &mut self.nodes[idx] {
            *actions = children;
        }
        idx
    }

    /// Each seat's result, in big blinds, at an end node with these hand
    /// classes: its share of the pot less what it put in.
    pub fn payoffs(
        &self,
        node: usize,
        classes: &[usize; SEATS],
        eq: &PreflopEquity,
    ) -> [f64; SEATS] {
        let Node::End {
            put_in,
            live,
            all_in,
        } = &self.nodes[node]
        else {
            panic!("not an end node");
        };
        let pot: u32 = put_in.iter().sum();
        let mut share = [0f64; SEATS];
        let players: Vec<usize> = (0..SEATS).filter(|&s| live[s]).collect();
        if players.len() == 1 {
            share[players[0]] = 1.0;
        } else {
            // The chance each beats every other, treating the matchups as
            // independent, then realization by position after the flop.
            let last = *players
                .iter()
                .max_by_key(|&&s| postflop_order(s))
                .expect("players");
            let mut total = 0.0;
            for &i in &players {
                let mut p = 1.0;
                for &j in &players {
                    if j != i {
                        p *= eq.heads_up(classes[i], classes[j]);
                    }
                }
                if !all_in {
                    p *= if i == last {
                        REALIZE_LAST
                    } else {
                        REALIZE_OTHERS
                    };
                }
                share[i] = p;
                total += p;
            }
            for s in &mut share {
                *s /= total;
            }
        }
        let mut out = [0f64; SEATS];
        for s in 0..SEATS {
            out[s] = (share[s] * pot as f64 - put_in[s] as f64) / 2.0;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tree_is_small_enough() {
        for bb in [20, 100, 200] {
            let g = PreflopGame::new(bb);
            assert!(g.decisions > 100, "{bb} BB: {} decisions", g.decisions);
            assert!(g.decisions < 200_000, "{bb} BB: {} decisions", g.decisions);
        }
    }

    #[test]
    fn everyone_folds_to_the_big_blind() {
        let g = PreflopGame::new(100);
        let mut node = g.root();
        let mut seats = Vec::new();
        while let Node::Decision { seat, actions, .. } = &g.nodes[node] {
            seats.push(*seat);
            node = actions
                .iter()
                .find(|(a, _)| *a == Act::Fold)
                .expect("fold")
                .1 as usize;
        }
        // UTG to SB fold; the big blind never has to act.
        assert_eq!(seats, vec![0, 1, 2, 3, 4]);
        let eq = PreflopEquity::compute(4, 1, 2);
        let p = g.payoffs(node, &[0; SEATS], &eq);
        assert_eq!(p[BB], 0.5);
        assert_eq!(p[SB], -0.5);
        assert!((p.iter().sum::<f64>()).abs() < 1e-9);
    }
}
