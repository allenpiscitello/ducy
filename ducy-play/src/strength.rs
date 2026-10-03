//! Hand-strength estimates that bots can use: equity against random hands,
//! and where a starting hand ranks among all starting hands.

use std::sync::{Arc, Mutex, OnceLock};

use ducy::deck::{Card, Deck};
use rand::{RngExt, SeedableRng, rngs::StdRng};

use crate::{bot::Observation, rules::Variant, showdown::best_hands};

/// Share of the pot `hole_cards` wins on average against `opponents` random
/// hands, with the rest of the board dealt at random, from `samples`
/// Monte Carlo deals. Returns 0 when there aren't enough cards.
pub fn equity_vs_random(
    variant: Variant,
    hole_cards: Deck,
    board: &[Card],
    opponents: usize,
    samples: usize,
    rng: &mut StdRng,
) -> f64 {
    let per_player = variant.hole_cards();
    let mut known = hole_cards;
    for &card in board {
        known |= card;
    }
    let mut unknown: Vec<Card> = (Deck::all_cards() - known).iter(false).collect();
    let board_needed = 5usize.saturating_sub(board.len());
    let needed = opponents * per_player + board_needed;
    if needed > unknown.len() || samples == 0 {
        return 0.0;
    }

    // Seat 0 is the hero, seats 1..=opponents the random hands.
    let seats: Vec<usize> = (0..=opponents).collect();
    let mut hands = vec![Deck::empty(); opponents + 1];
    hands[0] = hole_cards;
    let mut won = 0.0;
    let mut counted = 0;
    for _ in 0..samples {
        for i in 0..needed {
            let j = rng.random_range(i..unknown.len());
            unknown.swap(i, j);
        }
        let mut next = unknown.iter().copied();
        for hand in hands.iter_mut().skip(1) {
            *hand = Deck::empty();
            for card in next.by_ref().take(per_player) {
                *hand |= card;
            }
        }
        let mut full = board.to_vec();
        full.extend(next.take(board_needed));
        let Ok(full) = <[Card; 5]>::try_from(full) else {
            continue;
        };
        if let Ok((winners, _)) = best_hands(variant, &hands, &full, &seats) {
            counted += 1;
            if winners.contains(&0) {
                won += 1.0 / winners.len() as f64;
            }
        }
    }
    if counted == 0 {
        0.0
    } else {
        won / counted as f64
    }
}

/// [`equity_vs_random`] for the player to act, against every opponent still
/// in the hand.
pub fn observation_equity(obs: &Observation, samples: usize, rng: &mut StdRng) -> f64 {
    let opponents = obs
        .seats
        .iter()
        .enumerate()
        .filter(|&(s, v)| s != obs.seat && !v.folded)
        .count();
    equity_vs_random(
        obs.rules.variant,
        obs.hole_cards,
        &obs.board,
        opponents,
        samples,
        rng,
    )
}

/// Reference hands per variant for [`preflop_percentile`].
const REFERENCE_HANDS: usize = 300;
/// Monte Carlo deals per reference hand.
const REFERENCE_SAMPLES: usize = 100;

type Table = Arc<Vec<f64>>;

/// Heads-up equities of random starting hands, sorted, per variant. Built
/// once per variant from a fixed seed, so it's the same on every run.
fn reference(variant: Variant) -> Table {
    static TABLES: OnceLock<Mutex<Vec<(Variant, Table)>>> = OnceLock::new();
    let tables = TABLES.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(tables) = tables.lock() {
        if let Some((_, table)) = tables.iter().find(|(v, _)| *v == variant) {
            return table.clone();
        }
    }

    let mut rng = StdRng::seed_from_u64(0x5EED);
    let mut cards: Vec<Card> = Deck::all_cards().iter(false).collect();
    let mut equities: Vec<f64> = (0..REFERENCE_HANDS)
        .map(|_| {
            for i in 0..variant.hole_cards() {
                let j = rng.random_range(i..cards.len());
                cards.swap(i, j);
            }
            let mut hand = Deck::empty();
            for &card in &cards[..variant.hole_cards()] {
                hand |= card;
            }
            equity_vs_random(variant, hand, &[], 1, REFERENCE_SAMPLES, &mut rng)
        })
        .collect();
    equities.sort_by(f64::total_cmp);
    let table = Arc::new(equities);
    if let Ok(mut tables) = tables.lock() {
        tables.push((variant, table.clone()));
    }
    table
}

/// Where a starting hand ranks among all starting hands of the variant, from
/// 0 (worst) to 1 (best), by heads-up equity against a random hand. A hand
/// at 0.9 is in the top 10%.
///
/// The ranking is estimated by Monte Carlo (`samples` deals for this hand),
/// so hands near a cutoff can land on either side of it from one call to
/// the next.
pub fn preflop_percentile(
    variant: Variant,
    hole_cards: Deck,
    samples: usize,
    rng: &mut StdRng,
) -> f64 {
    let table = reference(variant);
    let equity = equity_vs_random(variant, hole_cards, &[], 1, samples, rng);
    let below = table.partition_point(|&e| e < equity);
    below as f64 / table.len().max(1) as f64
}
