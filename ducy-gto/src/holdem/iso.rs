//! Suit isomorphism: hands that differ only by relabeling suits play the
//! same, so they share a key. A hand is hole cards, then the board as dealt
//! (flop, turn, river); the canonical form relabels suits to make the hand's
//! encoding as small as possible, with cards sorted within each round.

use super::cards::{Card, card, rank, suit};

/// All 24 ways to relabel the four suits.
const PERMS: [[u8; 4]; 24] = {
    let mut out = [[0u8; 4]; 24];
    let mut n = 0;
    let mut a = 0;
    while a < 4 {
        let mut b = 0;
        while b < 4 {
            let mut c = 0;
            while c < 4 {
                let d = 6 - a - b - c;
                if a != b && a != c && b != c && d < 4 && d != a && d != b && d != c {
                    out[n] = [a as u8, b as u8, c as u8, d as u8];
                    n += 1;
                }
                c += 1;
            }
            b += 1;
        }
        a += 1;
    }
    out
};

/// Round sizes: hole cards, flop, turn, river.
const ROUNDS: [usize; 4] = [2, 3, 1, 1];

fn relabel(c: Card, p: &[u8; 4]) -> Card {
    card(rank(c), p[suit(c) as usize])
}

/// Packs cards (each below 64) into a key, first card most significant, with
/// the count in the low bits so different lengths never collide.
fn pack(cards: &[Card]) -> u64 {
    let mut k = 0u64;
    for &c in cards {
        k = (k << 6) | (c as u64 + 1);
    }
    k
}

/// The canonical form of `cards` (hole cards then board in deal order, up to
/// seven) under `perm`: relabeled, sorted within each round.
fn form(cards: &[Card], perm: &[u8; 4], out: &mut [Card; 7]) -> usize {
    let mut i = 0;
    for len in ROUNDS {
        if i >= cards.len() {
            break;
        }
        let end = (i + len).min(cards.len());
        for j in i..end {
            out[j] = relabel(cards[j], perm);
        }
        out[i..end].sort_unstable();
        i = end;
    }
    cards.len()
}

/// The canonical key of a hand: hole cards then board (0, 3, 4 or 5 cards)
/// in deal order. Hands equal up to suits get the same key.
pub fn canonical(cards: &[Card]) -> u64 {
    canonical_with_perm(cards).0
}

/// The canonical key and a suit relabeling that produces it.
pub fn canonical_with_perm(cards: &[Card]) -> (u64, [u8; 4]) {
    let mut best = (u64::MAX, PERMS[0]);
    let mut buf = [0u8; 7];
    for p in &PERMS {
        let n = form(cards, p, &mut buf);
        let k = pack(&buf[..n]);
        if k < best.0 {
            best = (k, *p);
        }
    }
    best
}

/// The canonical key of a board on its own (all cards one round).
pub fn canonical_board(board: &[Card]) -> (u64, [u8; 4]) {
    let mut best = (u64::MAX, PERMS[0]);
    let mut buf = [0u8; 5];
    for p in &PERMS {
        for (o, &c) in buf.iter_mut().zip(board) {
            *o = relabel(c, p);
        }
        buf[..board.len()].sort_unstable();
        let k = pack(&buf[..board.len()]);
        if k < best.0 {
            best = (k, *p);
        }
    }
    best
}

/// Applies a suit relabeling to a card.
pub fn apply(c: Card, perm: &[u8; 4]) -> Card {
    relabel(c, perm)
}

/// The 169 strategically distinct starting hands: 13 pairs, then 78 suited
/// and 78 offsuit hands by (high, low) rank.
pub fn preflop_class(a: Card, b: Card) -> usize {
    let (hi, lo) = if rank(a) >= rank(b) {
        (rank(a), rank(b))
    } else {
        (rank(b), rank(a))
    };
    if hi == lo {
        return hi as usize;
    }
    let idx = hi as usize * (hi as usize - 1) / 2 + lo as usize;
    if suit(a) == suit(b) {
        13 + idx
    } else {
        91 + idx
    }
}

pub const NUM_PREFLOP_CLASSES: usize = 169;
