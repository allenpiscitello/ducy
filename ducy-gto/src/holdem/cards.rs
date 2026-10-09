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

/// Omaha: the best hand from exactly two of `hole` and three of the 5-card
/// `board` (higher is better), over every such pair and triple.
pub fn omaha_score(hole: &[Card], board: &[Card]) -> u32 {
    let mut best = 0;
    for i in 0..hole.len() {
        for j in i + 1..hole.len() {
            let two = bit(hole[i]) | bit(hole[j]);
            for a in 0..board.len() {
                for b in a + 1..board.len() {
                    for c in b + 1..board.len() {
                        let five = two | bit(board[a]) | bit(board[b]) | bit(board[c]);
                        best = best.max(score(five));
                    }
                }
            }
        }
    }
    best
}

/// A showdown hand's strength: Hold'em's best five of seven for two hole
/// cards, Omaha's exactly-two-plus-three for more.
pub fn showdown_score(hole: &[Card], board: &[Card]) -> u32 {
    if hole.len() == 2 {
        score(mask(board) | mask(hole))
    } else {
        omaha_score(hole, board)
    }
}

/// n choose k.
pub fn binomial(n: usize, k: usize) -> usize {
    if k > n {
        return 0;
    }
    (0..k).fold(1, |acc, i| acc * (n - i) / (i + 1))
}

/// How many different `k`-card hands there are: 1,326 for Hold'em, 270,725
/// for Omaha, 2,598,960 for five-card and 20,358,520 for six-card Omaha.
pub fn num_combos(k: usize) -> usize {
    binomial(NUM_CARDS, k)
}

/// Index of a hand of any size among all [`num_combos`] of its size: the
/// combinatorial number system, `C(c1, 1) + C(c2, 2) + …` over its cards in
/// rising order. For two cards it equals [`hole_index`].
pub fn combo_index(cards: &[Card]) -> usize {
    let mut sorted = cards.to_vec();
    sorted.sort_unstable();
    sorted
        .iter()
        .enumerate()
        .map(|(i, &c)| binomial(c as usize, i + 1))
        .sum()
}

/// The `k` cards of combo index `i`, in rising order (inverse of
/// [`combo_index`]).
pub fn combo_cards(mut i: usize, k: usize) -> Vec<Card> {
    let mut out = vec![0; k];
    for slot in (0..k).rev() {
        // The largest card c with C(c, slot + 1) <= i.
        let mut c = slot;
        while binomial(c + 1, slot + 1) <= i {
            c += 1;
        }
        out[slot] = c as Card;
        i -= binomial(c, slot + 1);
    }
    out
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
