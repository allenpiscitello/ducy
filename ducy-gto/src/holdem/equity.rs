//! Equity against a uniformly random opponent hand.
//!
//! [`river_equities`] does every hole hand on a five-card board at once:
//! score the 1,081 hands the remaining cards allow, sort them, and count each
//! hand's wins and ties with binary searches, correcting for opponent hands
//! that share a card with it. That's about as fast as scoring the hands, which
//! makes enumerating every runout of every flop practical.

use super::cards::{Card, NUM_CARDS, NUM_HOLES, bit, hole_index, mask, score};

/// Every hand's share of the pot against a random opponent hand on `board`,
/// indexed by [`hole_index`]. Hands that use a board card get -1.
pub fn river_equities(board: &[Card; 5], out: &mut [f32; NUM_HOLES]) {
    strengths(board, out);
}

/// Every hand's share against a random opponent hand with `board` (3 to 5
/// cards) as it is now, ignoring cards to come: the river equity on a full
/// board, and on the flop and turn the same as
/// [`hand_strength`](super::abstraction::hand_strength). Hands that use a
/// board card get -1.
pub fn strengths(board: &[Card], out: &mut [f32; NUM_HOLES]) {
    let board_mask = mask(board);
    let free: Vec<Card> = (0..NUM_CARDS as Card)
        .filter(|&c| board_mask & bit(c) == 0)
        .collect();
    // Opponent hands left once the board and our hand are out.
    let rest = free.len() - 2;
    let opponents = (rest * (rest - 1) / 2) as f32;
    let mut hands: Vec<(u32, Card, Card)> = Vec::with_capacity(1176);
    for (i, &a) in free.iter().enumerate() {
        for &b in &free[i + 1..] {
            hands.push((score(board_mask | bit(a) | bit(b)), a, b));
        }
    }
    hands.sort_unstable_by_key(|h| h.0);
    let all: Vec<u32> = hands.iter().map(|h| h.0).collect();
    // Scores of the hands holding each card, in ascending order.
    let mut by_card: Vec<Vec<u32>> = vec![Vec::new(); NUM_CARDS];
    for &(s, a, b) in &hands {
        by_card[a as usize].push(s);
        by_card[b as usize].push(s);
    }
    let below = |v: &[u32], s: u32| v.partition_point(|&x| x < s);
    let upto = |v: &[u32], s: u32| v.partition_point(|&x| x <= s);
    out.fill(-1.0);
    for &(s, a, b) in &hands {
        let (ca, cb) = (&by_card[a as usize], &by_card[b as usize]);
        let less = below(&all, s) - below(ca, s) - below(cb, s);
        // Ties: this hand is in all three lists, so subtracting both card
        // lists removes it twice; add one back to leave it out exactly once.
        let ties = |v: &[u32]| upto(v, s) - below(v, s);
        let equal = ties(&all) + 1 - ties(ca) - ties(cb);
        out[hole_index(a, b)] = (less as f32 + 0.5 * equal as f32) / opponents;
    }
}

/// One hand's equity on a five-card board, by scoring every opponent hand.
pub fn river_equity(hole: [Card; 2], board: &[Card; 5]) -> f32 {
    let used = mask(board) | bit(hole[0]) | bit(hole[1]);
    let me = score(used);
    let board_mask = mask(board);
    let free: Vec<Card> = (0..NUM_CARDS as Card)
        .filter(|&c| used & bit(c) == 0)
        .collect();
    let mut won = 0.0;
    for (i, &a) in free.iter().enumerate() {
        for &b in &free[i + 1..] {
            let them = score(board_mask | bit(a) | bit(b));
            won += match me.cmp(&them) {
                std::cmp::Ordering::Greater => 1.0,
                std::cmp::Ordering::Equal => 0.5,
                std::cmp::Ordering::Less => 0.0,
            };
        }
    }
    won / 990.0
}

/// Equity against a random hand on any street, averaging over every runout
/// (exact; slow before the flop, meant for checks rather than training).
pub fn equity(hole: [Card; 2], board: &[Card]) -> f32 {
    let used = mask(board) | bit(hole[0]) | bit(hole[1]);
    let free: Vec<Card> = (0..NUM_CARDS as Card)
        .filter(|&c| used & bit(c) == 0)
        .collect();
    let need = 5 - board.len();
    let mut runout = [0u8; 5];
    runout[..board.len()].copy_from_slice(board);
    // Walk every `need`-card combination of the free cards.
    let mut idx: Vec<usize> = (0..need).collect();
    let (mut total, mut count) = (0.0f64, 0u64);
    loop {
        for (k, &i) in idx.iter().enumerate() {
            runout[board.len() + k] = free[i];
        }
        total += river_equity(hole, &runout) as f64;
        count += 1;
        // Advance to the next combination.
        let mut k = need;
        loop {
            if k == 0 {
                return (total / count as f64) as f32;
            }
            k -= 1;
            if idx[k] < free.len() - need + k {
                break;
            }
        }
        idx[k] += 1;
        for j in k + 1..need {
            idx[j] = idx[j - 1] + 1;
        }
    }
}
