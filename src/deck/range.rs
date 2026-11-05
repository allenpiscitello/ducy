use std::collections::HashMap;

use crate::deck::Deck;

/// A single hand in a range with its associated weight.
pub struct RangeItem {
    deck: Deck,
    weight: f64,
}

impl RangeItem {
    /// Returns the cards in this range item.
    pub fn get_deck(&self) -> Deck {
        self.deck
    }
    /// Returns the weight (frequency) of this range item.
    pub fn get_weight(&self) -> f64 {
        self.weight
    }
}

/// The terms of range text, separated by commas or whitespace, each with
/// its byte offset: `"QQ+, AKs"` gives `(0, "QQ+")` and `(5, "AKs")`.
pub fn terms(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, ch) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ',')))
    {
        if matches!(ch, ',' | ' ' | '\t' | '\n' | '\r') {
            if let Some(s) = start.take() {
                out.push((s, &text[s..i]));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    out
}

/// A collection of weighted hands representing a player's possible holdings.
pub trait Range {
    /// Returns an iterator over the hands in this range.
    fn iter(&self) -> impl Iterator<Item = RangeItem>;
}

/// Stores deck-to-weight mappings for building ranges.
#[derive(Clone)]
pub struct RangeBase {
    weights: HashMap<Deck, f64>,
}
impl RangeBase {
    pub(crate) fn new() -> Self {
        Self {
            weights: HashMap::new(),
        }
    }

    /// Adds or replaces the weight for a specific hand.
    pub fn add_deck_weight(&mut self, deck: Deck, weight: f64) {
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

#[cfg(test)]
mod test {
    use super::terms;

    #[test]
    fn terms_with_their_byte_offsets() {
        assert_eq!(terms("QQ+, AKs"), [(0, "QQ+"), (5, "AKs")]);
        assert_eq!(terms(" é,\tA5s-A2s\r\n"), [(1, "é"), (5, "A5s-A2s")]);
        assert!(terms(" , ").is_empty());
    }
}
