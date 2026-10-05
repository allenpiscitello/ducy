//! Ranges: how likely a player is to hold each of the 1,326 hole-card
//! hands, given the actions they've taken, assuming they play the blueprint.
//!
//! A range starts uniform over the hands the known cards allow. Each action
//! multiplies every hand's weight by the blueprint's probability of that
//! action for the hand's bucket at the node it was taken. LBR tracks the
//! blueprint's range this way, and the bot tracks both players' ranges to
//! solve the river.

use super::{
    abstraction::CardAbstraction,
    blueprint::Blueprint,
    cards::{Card, NUM_HOLES, bit, hole_cards, mask},
};

/// A weight per hole-card hand, indexed by
/// [`hole_index`](super::cards::hole_index).
#[derive(Clone, Debug, PartialEq)]
pub struct Range {
    pub weight: Vec<f64>,
}

impl Range {
    /// Uniform over the hands that don't use a card in `blocked`.
    pub fn new(blocked: u64) -> Self {
        let mut r = Self {
            weight: vec![1.0; NUM_HOLES],
        };
        r.remove(blocked);
        r
    }

    /// Drops the hands that use a card in `cards` (e.g. new board cards).
    pub fn remove(&mut self, cards: u64) {
        for (h, w) in self.weight.iter_mut().enumerate() {
            let (a, b) = hole_cards(h);
            if cards & (bit(a) | bit(b)) != 0 {
                *w = 0.0;
            }
        }
    }

    /// Updates for action `a` at blueprint node `node`, with every hand's
    /// bucket on the node's street in `buckets`. If no hand could have taken
    /// the action (a translation the blueprint never plays), the range is
    /// left as it was rather than emptied.
    pub fn update(&mut self, blueprint: &Blueprint, node: u32, a: usize, buckets: &[u16]) {
        // Rows by bucket: the same few rows serve every hand.
        let mut rows: Vec<Option<f64>> = Vec::new();
        self.update_by(|h| {
            let b = buckets[h];
            if b == u16::MAX {
                return 0.0;
            }
            let b = b as usize;
            if rows.len() <= b {
                rows.resize(b + 1, None);
            }
            *rows[b].get_or_insert_with(|| blueprint.probs(node, b as u16)[a])
        });
    }

    /// Multiplies each hand's weight by `p(hand)`, its probability of the
    /// action seen; left as it was if that would empty the range.
    pub fn update_by(&mut self, mut p: impl FnMut(usize) -> f64) {
        let mut next = self.weight.clone();
        for (h, w) in next.iter_mut().enumerate() {
            if *w > 0.0 {
                *w *= p(h);
            }
        }
        if next.iter().sum::<f64>() > 0.0 {
            self.weight = next;
        }
    }

    pub fn total(&self) -> f64 {
        self.weight.iter().sum()
    }
}

/// Every hand's bucket per street, computed once per street and shared.
#[derive(Clone, Debug, Default)]
pub struct StreetBuckets {
    street: Option<usize>,
    pub buckets: Vec<u16>,
}

impl StreetBuckets {
    /// The buckets for `street` (0 preflop to 3 river) with `board` (the
    /// whole deal's board; only the street's cards are used).
    pub fn on(&mut self, cards: &CardAbstraction, street: usize, board: &[Card]) -> &[u16] {
        if self.street != Some(street) {
            self.street = Some(street);
            self.buckets = cards.buckets(board_for(street, board));
        }
        &self.buckets
    }
}

/// The board cards out on `street`.
pub fn board_for(street: usize, board: &[Card]) -> &[Card] {
    &board[..[0, 3, 4, 5][street].min(board.len())]
}

/// Both players' ranges after a sequence of blueprint steps `(node, action,
/// player)`, with `board` the cards out so far (up to the last step's
/// street). Ranges are public: they leave out board cards only.
pub fn replay(
    cards: &CardAbstraction,
    blueprint: &Blueprint,
    tree: &super::hunl::BettingTree,
    steps: &[(u32, usize)],
    board: &[Card],
) -> [Range; 2] {
    let blocked = mask(board);
    let mut ranges = [Range::new(blocked), Range::new(blocked)];
    let mut buckets = StreetBuckets::default();
    for &(node, a) in steps {
        let b = &tree.nodes[node as usize].betting;
        if [0, 3, 4, 5][b.street] > board.len() {
            break;
        }
        let bk = buckets.on(cards, b.street, board);
        ranges[b.to_act].update(blueprint, node, a, bk);
    }
    ranges
}
