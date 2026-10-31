//! The six-handed preflop game (#249): six seats, a small raise menu and
//! the 169 starting-hand classes. Once the preflop betting closes, each
//! player still in gets a share of the pot from the equity tables, scaled by
//! how much of its equity its position lets it realize after the flop.
//!
//! Chips are counted in half big blinds: the small blind is 1, the big
//! blind 2. Seats are numbered by position: 0 UTG, 1 HJ, 2 CO, 3 BTN, 4 SB,
//! 5 BB. Preflop, UTG acts first; after the flop the small blind does, and
//! the button acts last.
//!
//! Pot-limit Omaha (#290) uses the same tree with pot-sized raises
//! ([`PreflopGame::pot_limit`]) over the PLO abstraction's preflop buckets
//! instead of the 169 classes: a game's `classes` is how many hand classes
//! its strategies, equity and realization tables have.

use super::{equity::PreflopEquity, realize::Realization};
use crate::holdem::iso::NUM_PREFLOP_CLASSES;
/// Postflop tables are over hold'em's classes.
const NUM_CLASSES: usize = NUM_PREFLOP_CLASSES;

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
/// Limpers the tree allows before the next player must raise or fold.
const MAX_LIMPERS: u32 = 2;
/// Pot limit: raises allowed (open, 3-bet, 4-bet, 5-bet), each the size of
/// the pot; one that reaches the stack is the all-in.
const MAX_POT_RAISES: u32 = 4;

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
    /// How much of its equity each hand realizes after the flop, when it
    /// isn't all-in: in position (last to act after the flop) or not.
    pub realize: Realization,
    /// Hand classes: 169 for hold'em, the preflop buckets for PLO.
    pub classes: usize,
    /// Pot-limit raises (PLO) rather than the no-limit sizes.
    pub pot_limit: bool,
    /// Measured postflop values for single-raised pots ([`PostflopTable`]):
    /// where the preflop betting ends in one of these, its pot shares come
    /// from the table instead of equity times realization.
    pub postflop: Vec<PostflopTable>,
    /// Realization adds instead of scaling (#278): each player's share of
    /// the pot is its equity times its realization, and what that leaves
    /// over (or takes beyond) the pot is split evenly, so a share can pass
    /// 1 (winning more than the pot after the flop, as deep stacks do) or
    /// drop below 0. Off: the products are rescaled to sum to 1, which caps
    /// every share at the pot.
    pub additive: bool,
}

/// What a two-player pot (limped, single-raised, 3-bet …) is worth after the
/// flop, measured by playing a trained postflop spot (`sixmax::spot`): for
/// each pair of hand classes, the share of the flop pot the player in
/// position ends up with (more than 1 when they win more than the pot after
/// the flop, less than 0 when they lose more than they put in). The other
/// player gets the rest. A preflop line uses it when exactly these two
/// players reach the flop with `to` each in.
#[derive(Clone, Debug, PartialEq)]
pub struct PostflopTable {
    /// The two players' positions (0 UTG … 5 BB), in either order.
    pub opener: usize,
    pub caller: usize,
    /// Each put this in preflop, in half big blinds.
    pub to: u32,
    /// `[ip class * 169 + oop class]`.
    pub share: Vec<f32>,
}

const TABLES_MAGIC: &[u8; 8] = b"DUCY6PT\0";

impl PostflopTable {
    /// Tables as one file: the magic, a count, then per table the opener,
    /// the caller (a byte each), `to` (u32) and the shares (f32s).
    pub fn to_bytes(tables: &[PostflopTable]) -> Vec<u8> {
        let mut out = TABLES_MAGIC.to_vec();
        out.extend((tables.len() as u32).to_le_bytes());
        for t in tables {
            out.push(t.opener as u8);
            out.push(t.caller as u8);
            out.extend(t.to.to_le_bytes());
            for s in &t.share {
                out.extend(s.to_le_bytes());
            }
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Vec<PostflopTable>, String> {
        let bad = || "not six-handed postflop tables".to_string();
        if bytes.get(..8) != Some(TABLES_MAGIC) {
            return Err(bad());
        }
        let n = u32::from_le_bytes(bytes.get(8..12).ok_or_else(bad)?.try_into().expect("4"));
        let size = 6 + 4 * NUM_CLASSES * NUM_CLASSES;
        let mut out = Vec::new();
        for i in 0..n as usize {
            let at = 12 + i * size;
            let b = bytes.get(at..at + size).ok_or_else(bad)?;
            out.push(PostflopTable {
                opener: b[0] as usize,
                caller: b[1] as usize,
                to: u32::from_le_bytes(b[2..6].try_into().expect("4")),
                share: b[6..]
                    .chunks(4)
                    .map(|c| f32::from_le_bytes(c.try_into().expect("4")))
                    .collect(),
            });
        }
        Ok(out)
    }
}

impl PreflopGame {
    /// The tree with `big_blinds`-deep stacks, and the default realization
    /// (in position 1.08, out of position 0.9, for every hand).
    pub fn new(big_blinds: u32) -> Self {
        Self::with_realization(big_blinds, Realization::default())
    }

    /// The tree with realization fitted for this depth (`sixmax_realize`).
    pub fn with_realization(big_blinds: u32, realize: Realization) -> Self {
        assert_eq!(realize.ip.len(), NUM_PREFLOP_CLASSES, "hold'em classes");
        Self::build_game(big_blinds, realize, false)
    }

    /// Pot-limit Omaha: pot-sized raises, over as many hand classes (preflop
    /// buckets) as `realize` has.
    pub fn pot_limit(big_blinds: u32, realize: Realization) -> Self {
        Self::build_game(big_blinds, realize, true)
    }

    fn build_game(big_blinds: u32, realize: Realization, pot_limit: bool) -> Self {
        let mut g = Self {
            stack: 2 * big_blinds,
            nodes: Vec::new(),
            decisions: 0,
            classes: realize.ip.len(),
            realize,
            pot_limit,
            postflop: Vec::new(),
            additive: false,
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
        // Limpers so far (the small blind completing counts).
        let limpers = (0..SEATS)
            .filter(|&x| x != BB && x != seat && !s.folded[x] && s.put_in[x] == 2)
            .count() as u32;
        // Anyone may limp (the Regulars do, so the solve has to know those
        // pots), up to MAX_LIMPERS before the rest must raise or fold, which
        // keeps the tree a size that trains; the small blind may complete
        // and the big blind check whatever came before.
        if s.raises > 0 || seat == SB || seat == BB || limpers < MAX_LIMPERS {
            acts.push(Act::Call);
        }
        let cap = self.stack;
        if self.pot_limit {
            // One raise size, the pot: call, then raise by the pot that
            // makes. A pot raise the stack can't cover is the all-in.
            if !s.all_in && s.bet < cap && s.raises < MAX_POT_RAISES {
                let pot: u32 = s.put_in.iter().sum();
                let to = s.bet + pot + (s.bet - s.put_in[seat]);
                acts.push(if to >= cap {
                    Act::AllIn
                } else {
                    Act::Raise(to)
                });
            }
        } else if !s.all_in && s.bet < cap {
            // Raising over limpers costs a big blind more per limper.
            let to = match s.raises {
                0 if seat == SB => Some(OPEN_SB + 2 * limpers),
                0 => Some(OPEN + 2 * limpers),
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
        // A single-raised pot with a measured postflop table.
        let table = (players.len() == 2 && !all_in)
            .then(|| {
                self.postflop.iter().find(|t| {
                    let pair = [t.opener, t.caller];
                    pair.contains(&players[0])
                        && pair.contains(&players[1])
                        && put_in[t.opener] == t.to
                        && put_in[t.caller] == t.to
                })
            })
            .flatten();
        if players.len() == 1 {
            share[players[0]] = 1.0;
        } else if let Some(t) = table {
            let (ip, oop) = if postflop_order(players[0]) > postflop_order(players[1]) {
                (players[0], players[1])
            } else {
                (players[1], players[0])
            };
            let s = t.share[classes[ip] * NUM_CLASSES + classes[oop]] as f64;
            share[ip] = s;
            share[oop] = 1.0 - s;
        } else {
            // The chance each beats every other, treating the matchups as
            // independent, then realization by position after the flop.
            let last = *players
                .iter()
                .max_by_key(|&&s| postflop_order(s))
                .expect("players");
            let mut total = 0.0;
            if self.additive && !all_in {
                // Equity first (the products normalized), then realization
                // added: the shares still sum to 1, but aren't capped.
                let mut e = [0f64; SEATS];
                let mut sum = 0.0;
                for &i in &players {
                    let mut p = 1.0;
                    for &j in &players {
                        if j != i {
                            p *= eq.heads_up(classes[i], classes[j]);
                        }
                    }
                    e[i] = p;
                    sum += p;
                }
                let mut realized = 0.0;
                for &i in &players {
                    let r = if i == last {
                        &self.realize.ip
                    } else {
                        &self.realize.oop
                    };
                    share[i] = e[i] / sum * r[classes[i]] as f64;
                    realized += share[i];
                }
                let even = (1.0 - realized) / players.len() as f64;
                for &i in &players {
                    share[i] += even;
                }
                let mut out = [0f64; SEATS];
                for s in 0..SEATS {
                    out[s] = (share[s] * pot as f64 - put_in[s] as f64) / 2.0;
                }
                return out;
            }
            for &i in &players {
                let mut p = 1.0;
                for &j in &players {
                    if j != i {
                        p *= eq.heads_up(classes[i], classes[j]);
                    }
                }
                if !all_in {
                    let r = if i == last {
                        &self.realize.ip
                    } else {
                        &self.realize.oop
                    };
                    p *= r[classes[i]] as f64;
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
    fn pot_limit_raises_are_pot_sized() {
        let g = PreflopGame::pot_limit(100, Realization::uniform(500));
        assert_eq!(g.classes, 500);
        assert!(
            g.decisions > 100 && g.decisions < 200_000,
            "{}",
            g.decisions
        );
        // UTG opens to the pot: 3.5 BB (7 half blinds).
        let Node::Decision { actions, .. } = &g.nodes[g.root()] else {
            panic!("root");
        };
        let open = actions
            .iter()
            .find_map(|(a, c)| matches!(a, Act::Raise(_)).then_some((*a, *c)))
            .expect("an open");
        assert_eq!(open.0, Act::Raise(7));
        assert!(
            !actions.iter().any(|(a, _)| *a == Act::AllIn),
            "no shove at 100 BB"
        );
        // Folds to the button, which 3-bets the pot: 3.5 + 3.5 + 1.5 called
        // = 8.5 BB pot, so to 3.5 + 8.5 = 12 BB (24 half blinds).
        let mut node = open.1 as usize;
        loop {
            let Node::Decision { seat, actions, .. } = &g.nodes[node] else {
                panic!("decision");
            };
            if *seat == 3 {
                let r = actions.iter().find(|(a, _)| matches!(a, Act::Raise(_)));
                assert_eq!(r.map(|x| x.0), Some(Act::Raise(24)));
                break;
            }
            node = actions.iter().find(|(a, _)| *a == Act::Fold).unwrap().1 as usize;
        }
        // At 10 BB the open's 3-bet is already the all-in.
        let short = PreflopGame::pot_limit(10, Realization::uniform(4));
        let Node::Decision { actions, .. } = &short.nodes[short.root()] else {
            panic!("root");
        };
        let open = actions.iter().find(|(a, _)| *a == Act::Raise(7)).unwrap().1;
        let Node::Decision { actions, .. } = &short.nodes[open as usize] else {
            panic!("decision");
        };
        assert!(actions.iter().any(|(a, _)| *a == Act::AllIn));
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
