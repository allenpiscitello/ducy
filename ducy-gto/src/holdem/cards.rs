//! Compact cards for heads-up Hold'em: a card is a `u8` from 0 to 51, and a
//! set of cards a `u64` mask in ducy's [`Deck`] layout, so hands are scored
//! with ducy's evaluator.

use ducy::{deck::Deck, ranking::hand_rank::StandardHandRanker};

/// A card: `rank * 4 + suit`, rank 0 (deuce) to 12 (ace), suit 0 to 3.
pub type Card = u8;

pub const NUM_CARDS: usize = 52;

pub fn rank(c: Card) -> u8 {
    c / 4
}

pub fn suit(c: Card) -> u8 {
    c % 4
}

pub fn card(rank: u8, suit: u8) -> Card {
    rank * 4 + suit
}

/// The card's bits in ducy's [`Deck`]: 16 bits per suit, ranks from bit 1,
/// and an ace also sets bit 0 (it plays low in A-2-3-4-5).
pub fn bit(c: Card) -> u64 {
    let base = 16 * suit(c) as u32;
    let high = 1u64 << (base + rank(c) as u32 + 1);
    if rank(c) == 12 {
        high | 1u64 << base
    } else {
        high
    }
}

pub fn mask(cards: &[Card]) -> u64 {
    cards.iter().fold(0, |m, &c| m | bit(c))
}

/// Strength of the best five of 5 to 7 cards (higher is better).
pub fn score(mask: u64) -> u32 {
    StandardHandRanker::score(&Deck::from(mask))
}

/// Converts from a ducy card.
pub fn from_ducy(c: ducy::deck::Card) -> Card {
    let b = 63 - u64::from(c.get_deck()).leading_zeros() as u8;
    card(b % 16 - 1, b / 16)
}

/// Parses cards like `"As Kd"`.
pub fn parse(s: &str) -> Option<Vec<Card>> {
    s.split_whitespace()
        .map(|t| ducy::deck::Card::parse(t).ok().map(from_ducy))
        .collect()
}

pub fn to_string(c: Card) -> String {
    let r = b"23456789TJQKA"[rank(c) as usize] as char;
    let s = b"cdhs"[suit(c) as usize] as char;
    format!("{r}{s}")
}

/// Index of a two-card hand among all 1,326: `hi * (hi - 1) / 2 + lo`.
pub fn hole_index(a: Card, b: Card) -> usize {
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    hi as usize * (hi as usize - 1) / 2 + lo as usize
}

pub const NUM_HOLES: usize = 1326;

/// The cards of hole index `i` (inverse of [`hole_index`]), low card first.
pub fn hole_cards(i: usize) -> (Card, Card) {
    let mut hi = 1;
    while (hi + 1) * hi / 2 <= i {
        hi += 1;
    }
    (((i - hi * (hi - 1) / 2) as Card), hi as Card)
}
