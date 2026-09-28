use std::collections::HashMap;

use crate::deck::Deck;
use rust_decimal::Decimal;

/// A single hand in a range with its associated weight.
pub struct RangeItem {
    deck: Deck,
    weight: Decimal,
}

impl RangeItem {
    /// Returns the cards in this range item.
    pub fn get_deck(&self) -> Deck {
        self.deck
    }
    /// Returns the weight (frequency) of this range item.
    pub fn get_weight(&self) -> Decimal {
        self.weight
    }
}

/// A collection of weighted hands representing a player's possible holdings.
pub trait Range {
    /// Returns an iterator over the hands in this range.
    fn iter(&self) -> impl Iterator<Item = RangeItem>;
}

/// Stores deck-to-weight mappings for building ranges.
pub struct RangeBase {
    weights: HashMap<Deck, Decimal>,
}
impl RangeBase {
    pub(crate) fn new() -> Self {
        Self {
            weights: HashMap::new(),
        }
    }

    /// Adds or replaces the weight for a specific hand.
    pub fn add_deck_weight(&mut self, deck: Deck, weight: Decimal) {
        self.weights.insert(deck, weight);
    }
}

impl Range for RangeBase {
    fn iter(&self) -> impl Iterator<Item = RangeItem> {
        self.weights.iter().map(|(deck, weight)| RangeItem {
            deck: *deck,
            weight: *weight,
        })
    }
}
