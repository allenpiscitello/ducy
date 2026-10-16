//! How a field of players actually plays preflop (#245, option B): counts of
//! the actions they take at each decision of the six-handed tree, by hand
//! class, recorded from their hands. Smoothed toward a prior strategy (the
//! equilibrium solve), the counts become a strategy for the field that the
//! solver can play against (`PreflopSolver::against_field`), so the result
//! exploits the field instead of assuming it plays well.

use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

use ducy_play::Event;

use super::{
    bot::{PreflopStrategy, follow},
    preflop::{Node, PreflopGame},
};
use crate::holdem::{cards::Card, iso::NUM_PREFLOP_CLASSES, iso::preflop_class};

const C: usize = NUM_PREFLOP_CLASSES;
const MAGIC: &[u8; 8] = b"DUCY6FL\0";
const VERSION: u32 = 1;

/// Action counts per decision and class. Threads can record into one table
/// at once.
pub struct FieldCounts {
    stack: u32,
    /// Per decision id: where its counts start, and its number of actions.
    offsets: Vec<(usize, usize)>,
    counts: Vec<AtomicU32>,
}

impl FieldCounts {
    pub fn new(game: &PreflopGame) -> Self {
        let mut offsets = vec![(0, 0); game.decisions as usize];
        let mut total = 0;
        for node in &game.nodes {
            if let Node::Decision { id, actions, .. } = node {
                offsets[*id as usize] = (total, actions.len());
                total += actions.len() * C;
            }
        }
        Self {
            stack: game.stack,
            offsets,
            counts: (0..total).map(|_| AtomicU32::new(0)).collect(),
        }
    }

    fn slot(&self, id: u32, class: usize, action: usize) -> &AtomicU32 {
        let (start, n) = self.offsets[id as usize];
        &self.counts[start + class * n + action]
    }

    /// How many times the field took each action at decision `id` with
    /// hand class `class`.
    pub fn counts(&self, id: u32, class: usize) -> Vec<u32> {
        let (_, n) = self.offsets[id as usize];
        (0..n)
            .map(|a| self.slot(id, class, a).load(Relaxed))
            .collect()
    }

    /// All decisions recorded.
    pub fn total(&self) -> u64 {
        self.counts.iter().map(|c| c.load(Relaxed) as u64).sum()
    }

    /// Records a finished six-handed hand: every preflop decision on the tree,
    /// by the class of the hand that made it, for the seats `record` accepts.
    /// `holes` are each table seat's two hole cards. Decisions after the hand
    /// leaves the tree aren't recorded. Returns the decisions recorded.
    pub fn record(
        &self,
        game: &PreflopGame,
        events: &[Event],
        button: usize,
        holes: &[[Card; 2]],
        record: impl Fn(usize) -> bool,
    ) -> usize {
        let mut n = 0;
        follow(game, events, button, |node, seat, action| {
            if !record(seat) {
                return;
            }
            let Node::Decision { id, .. } = &game.nodes[node] else {
                return;
            };
            let class = preflop_class(holes[seat][0], holes[seat][1]);
            self.slot(*id, class, action).fetch_add(1, Relaxed);
            n += 1;
        });
        n
    }

    /// The field's strategy: at each decision and class, its counts plus
    /// `prior_weight` decisions' worth of `prior`'s strategy there, so spots
    /// it rarely reaches stay close to the prior.
    pub fn strategy(&self, prior: &PreflopStrategy, prior_weight: f32) -> PreflopStrategy {
        let game = prior.game();
        assert_eq!(game.stack, self.stack, "counts for another depth");
        PreflopStrategy::from_fn(game, |id, class| {
            let counts = self.counts(id, class);
            let total: u32 = counts.iter().sum();
            let p = prior.probs(id, class);
            counts
                .iter()
                .zip(p)
                .map(|(&c, &q)| (c as f32 + prior_weight * q) / (total as f32 + prior_weight))
                .collect()
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(24 + 4 * self.counts.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&self.stack.to_le_bytes());
        out.extend_from_slice(&(self.counts.len() as u64).to_le_bytes());
        for c in &self.counts {
            out.extend_from_slice(&c.load(Relaxed).to_le_bytes());
        }
        out
    }

    /// Counts saved by `to_bytes`, for this tree.
    pub fn from_bytes(game: &PreflopGame, bytes: &[u8]) -> Result<Self, String> {
        let me = Self::new(game);
        let n = me.counts.len();
        if bytes.len() != 24 + 4 * n || &bytes[..8] != MAGIC {
            return Err("not six-handed field counts for this tree".into());
        }
        let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().expect("4"));
        let u64_at = |i: usize| u64::from_le_bytes(bytes[i..i + 8].try_into().expect("8"));
        if u32_at(8) != VERSION || u32_at(12) != game.stack || u64_at(16) as usize != n {
            return Err("field counts for another version or depth".into());
        }
        for (i, c) in me.counts.iter().enumerate() {
            c.store(u32_at(24 + 4 * i), Relaxed);
        }
        Ok(me)
    }

    /// Adds another table's counts (from another recording run) to these.
    pub fn merge(&self, other: &FieldCounts) {
        assert_eq!(self.counts.len(), other.counts.len(), "same tree");
        for (a, b) in self.counts.iter().zip(&other.counts) {
            a.fetch_add(b.load(Relaxed), Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        holdem::{cards::from_ducy, chart::class_name},
        sixmax::preflop::Act,
    };
    use ducy_play::{Action, Deal, Hand, TableRules};

    fn class(name: &str) -> usize {
        (0..C).find(|&c| class_name(c) == name).expect("class")
    }

    #[test]
    fn records_a_hand_and_smooths_toward_the_prior() {
        let game = PreflopGame::new(100);
        // Blinds 1/2, button seat 0, so seat 3 is UTG: UTG opens, all fold.
        let rules = TableRules::no_limit_holdem(1, 2);
        let deal = Deal::random(rules.variant, 6, Some(7)).expect("deal");
        let mut hand = Hand::new(rules, &[200; 6], 0, deal).expect("hand");
        hand.act(Action::Raise(5)).expect("UTG opens");
        for _ in 0..5 {
            hand.act(Action::Fold).expect("fold");
        }
        let holes: Vec<[Card; 2]> = (0..6)
            .map(|s| {
                let c: Vec<Card> = hand
                    .hole_cards(s)
                    .expect("cards")
                    .iter(false)
                    .map(from_ducy)
                    .collect();
                [c[0], c[1]]
            })
            .collect();
        let counts = FieldCounts::new(&game);
        // The open and five folds.
        assert_eq!(counts.record(&game, hand.events(), 0, &holes, |_| true), 6);
        // Leaving UTG out records the folds only.
        let others = FieldCounts::new(&game);
        assert_eq!(
            others.record(&game, hand.events(), 0, &holes, |s| s != 3),
            5
        );

        let Node::Decision { id, actions, .. } = &game.nodes[game.root()] else {
            panic!("root");
        };
        let utg = preflop_class(holes[3][0], holes[3][1]);
        let raise = actions
            .iter()
            .position(|(a, _)| matches!(a, Act::Raise(_)))
            .expect("raise");
        assert_eq!(counts.counts(*id, utg)[raise], 1);

        // One open, smoothed with a uniform prior of weight 1: half of it.
        let prior = PreflopStrategy::from_fn(&game, |id, _| {
            let n = counts.counts(id, 0).len();
            vec![1.0 / n as f32; n]
        });
        let field = counts.strategy(&prior, 1.0);
        let n = actions.len() as f32;
        let p = field.probs(*id, utg)[raise];
        assert!((p - (1.0 + 1.0 / n) / 2.0).abs() < 1e-6, "{p}");
        // An unseen class keeps the prior.
        let aa = class("AA");
        if aa != utg {
            assert!((field.probs(*id, aa)[raise] - 1.0 / n).abs() < 1e-6);
        }

        let back = FieldCounts::from_bytes(&game, &counts.to_bytes()).expect("load");
        assert_eq!(back.total(), 6);
        back.merge(&others);
        assert_eq!(back.total(), 11);
    }
}
