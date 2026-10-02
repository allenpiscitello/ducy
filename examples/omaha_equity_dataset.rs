//! Writes a CSV of random Omaha starting hands with strength measures, for
//! building hand rankings (e.g. ducy-web's Omaha percentiles).
//!
//! cargo run --release --example omaha_equity_dataset -- \
//!     <cards 4|5|6> <hands> <samples> <seed> <out.csv> [flops=200] [flop_samples=60]
//!
//! Each line is `hand,hu,mw,flop_hit`:
//! - `hu`: equity against one random hand (all-in, random board)
//! - `mw`: equity against two random hands (rewards hands that make the nuts)
//! - `flop_hit`: share of random flops where the hand has at least 60% equity
//!   against a random hand, i.e. how often it flops well enough to keep going
//!
//! Hands are uniform over all starting hands, and everything is seeded, so the
//! output is reproducible.

use std::io::Write;

use ducy::deck::Deck;
use ducy::games::omaha_bomb_pot::{OmahaBombPotGameEvaluation, OmahaBombPotGameState};
use rayon::prelude::*;

const RANKS: &[u8] = b"23456789TJQKA";
const SUITS: &[u8] = b"shdc";
const FLOP_FAVORITE: f64 = 0.6;

// SplitMix64: tiny, seedable, and plenty for picking hands and flops.
fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn card(c: usize) -> String {
    format!("{}{}", RANKS[c % 13] as char, SUITS[c / 13] as char)
}

/// Deals `n` distinct cards (0..52) not in `exclude`.
fn deal(n: usize, exclude: &[usize], state: &mut u64) -> Vec<usize> {
    let mut deck: Vec<usize> = (0..52).filter(|c| !exclude.contains(c)).collect();
    for i in 0..n {
        let j = i + (next(state) % (deck.len() - i) as u64) as usize;
        deck.swap(i, j);
    }
    deck.truncate(n);
    deck
}

fn to_deck(cards: &[usize]) -> Deck {
    Deck::parse(&cards.iter().map(|&c| card(c)).collect::<Vec<_>>().join(" ")).unwrap()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if !(6..=8).contains(&args.len()) {
        eprintln!(
            "usage: omaha_equity_dataset <cards 4|5|6> <hands> <samples> <seed> <out.csv> [flops=200] [flop_samples=60]"
        );
        std::process::exit(2);
    }
    let cards: usize = args[1].parse().expect("cards");
    let hands: usize = args[2].parse().expect("hands");
    let samples: usize = args[3].parse().expect("samples");
    let seed: u64 = args[4].parse().expect("seed");
    let flops: usize = args.get(6).map_or(200, |s| s.parse().expect("flops"));
    let flop_samples: usize = args.get(7).map_or(60, |s| s.parse().expect("flop_samples"));
    assert!((4..=6).contains(&cards), "cards must be 4, 5 or 6");

    let mut rng = seed;
    let picked: Vec<Vec<usize>> = (0..hands).map(|_| deal(cards, &[], &mut rng)).collect();

    let eval = OmahaBombPotGameEvaluation {};
    let rows: Vec<String> = picked
        .par_iter()
        .enumerate()
        .map(|(i, hand)| {
            let hand_seed = seed ^ (i as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let mut state = OmahaBombPotGameState::new(1, cards as u32);
            state.add_player(to_deck(hand)).unwrap();

            let hu = eval
                .sample_seeded(&state, &[1], samples, Some(hand_seed))
                .unwrap();
            let mw = eval
                .sample_seeded(&state, &[1, 2], samples, Some(hand_seed ^ 1))
                .unwrap();

            let mut flop_rng = hand_seed ^ 2;
            let mut hits = 0;
            for f in 0..flops {
                let flop = deal(3, hand, &mut flop_rng);
                let mut on_flop = OmahaBombPotGameState::new(1, cards as u32);
                on_flop.add_player(to_deck(hand)).unwrap();
                on_flop.set_flop(0, to_deck(&flop)).unwrap();
                let r = eval
                    .sample_seeded(
                        &on_flop,
                        &[1],
                        flop_samples,
                        Some(hand_seed ^ (f as u64 + 3)),
                    )
                    .unwrap();
                if r.equity_sum[0] / flop_samples as f64 >= FLOP_FAVORITE {
                    hits += 1;
                }
            }

            let name: String = hand.iter().map(|&c| card(c)).collect();
            format!(
                "{name},{:.5},{:.5},{:.5}",
                hu.equity_sum[0] / samples as f64,
                mw.equity_sum[0] / samples as f64,
                hits as f64 / flops as f64
            )
        })
        .collect();

    let mut out = std::io::BufWriter::new(std::fs::File::create(&args[5]).expect("create output"));
    writeln!(out, "hand,hu,mw,flop_hit").unwrap();
    for row in rows {
        writeln!(out, "{row}").unwrap();
    }
    eprintln!("wrote {hands} PLO{cards} hands to {}", args[5]);
}
