//! Poker hand analysis library supporting Texas Hold'em and Omaha.
//!
//! Provides card/deck primitives, hand ranking, winner evaluation, and equity calculation.

/// Card, deck, and range primitives using bitfield representation.
pub mod deck;
/// Custom error types for the library.
pub mod error;
/// Game state, evaluation, and equity calculation for poker variants.
pub mod games;
/// Preflop starting-hand classes, equity table, and push/fold solver.
pub mod preflop;
/// Hand ranking systems for poker hands.
pub mod ranking;

#[cfg(test)]
pub(crate) mod test_util {
    use crate::deck::{Card, Deck};

    pub fn deck_from_cards(val: &str) -> Deck {
        let card_strs = val.split(" ");
        let cards: Vec<Card> = card_strs.map(|x| Card::parse(x).unwrap()).collect();
        let mut deck = Deck::empty();
        deck.insert_cards(cards.iter());
        deck
    }
}
