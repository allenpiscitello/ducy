//! Measures how often Omaha hands flop well against opponents who are strong
//! on that flop, for building hand rankings.
//!
//! cargo run --release --example omaha_flop_quality -- \
//!     <cards 4|5|6> <hands.csv> <range.txt> <out.csv> [options as key=value]
//!
//! Options (defaults): flops=400 range_sample=3000 continue=0.6 continue_samples=24
//!   hand_flops=100 opponents=8 runouts=12 favorable=0.55 seed=1
//!
//! Model: opponents play a preflop range (`range.txt`, one hand per line, e.g.
//! a sample of the top 25% of hands). On a flop, only the part of that range
//! that is strong there gives action: hands with at least `continue` equity
//! against a random hand on that flop. For each hand in `hands.csv` (first
//! column, header skipped) we deal flops, face those continuing opponents, and
//! sample turn and river:
//! - `favorable`: share of flops where the hand has at least `favorable`
//!   equity against them, i.e. flops it can play for stacks without being
//!   dominated (top set over bottom set, the top of a straight, nut draws)
//! - `surplus`: mean of max(0, equity - 0.5) over flops, a measure of how far
//!   ahead it is when it connects
//! - `flop_equity`: mean equity against continuing opponents
//!
//! A fixed set of `flops` is shared across hands so the continuing ranges are
//! computed once per flop. Everything is seeded and reproducible.

use std::collections::HashMap;
use std::io::{BufRead, Write};

use ducy::deck::Deck;
use ducy::games::flop_game::FlopGame;
use ducy::games::omaha::{OmahaGameEvaluation, OmahaGameState};
use ducy::games::omaha_bomb_pot::{OmahaBombPotGameEvaluation, OmahaBombPotGameState};
use rayon::prelude::*;
use rust_decimal::prelude::ToPrimitive;

const RANKS: &[u8] = b"23456789TJQKA";
const SUITS: &[u8] = b"shdc";

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn parse_hand(s: &str) -> Vec<usize> {
    s.as_bytes()
        .chunks(2)
        .map(|c| {
            let r = RANKS.iter().position(|&x| x == c[0]).expect("rank");
            let su = SUITS.iter().position(|&x| x == c[1]).expect("suit");
            r + 13 * su
        })
        .collect()
}

fn to_deck(cards: &[usize]) -> Deck {
    let s: Vec<String> = cards
        .iter()
        .map(|&c| format!("{}{}", RANKS[c % 13] as char, SUITS[c / 13] as char))
        .collect();
    Deck::parse(&s.join(" ")).unwrap()
}

fn deal(n: usize, exclude: &[usize], state: &mut u64) -> Vec<usize> {
    let mut deck: Vec<usize> = (0..52).filter(|c| !exclude.contains(c)).collect();
    for i in 0..n {
        let j = i + (next(state) % (deck.len() - i) as u64) as usize;
        deck.swap(i, j);
    }
    deck.truncate(n);
    deck
}

fn overlaps(a: &[usize], b: &[usize]) -> bool {
    a.iter().any(|c| b.contains(c))
}

fn read_lines(path: &str) -> Vec<String> {
    std::io::BufReader::new(std::fs::File::open(path).expect(path))
        .lines()
        .map(|l| l.unwrap())
        .filter(|l| !l.is_empty())
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 {
        eprintln!(
            "usage: omaha_flop_quality <cards> <hands.csv> <range.txt> <out.csv> [key=value ...]"
        );
        std::process::exit(2);
    }
    let cards: usize = args[1].parse().expect("cards");
    let opts: HashMap<String, String> = args[5..]
        .iter()
        .map(|a| {
            let (k, v) = a.split_once('=').expect("options are key=value");
            (k.to_string(), v.to_string())
        })
        .collect();
    let opt = |k: &str, d: f64| opts.get(k).map_or(d, |v| v.parse().expect(k));
    let n_flops = opt("flops", 400.0) as usize;
    let range_sample = opt("range_sample", 3000.0) as usize;
    let cont = opt("continue", 0.6);
    let cont_samples = opt("continue_samples", 24.0) as usize;
    let hand_flops = opt("hand_flops", 100.0) as usize;
    let opponents = opt("opponents", 8.0) as usize;
    let runouts = opt("runouts", 12.0) as usize;
    let favorable = opt("favorable", 0.55);
    let seed = opt("seed", 1.0) as u64;

    let hands: Vec<String> = read_lines(&args[2])
        .into_iter()
        .skip(1)
        .map(|l| l.split(',').next().unwrap().to_string())
        .collect();
    let mut rng = seed;
    let mut range: Vec<Vec<usize>> = read_lines(&args[3]).iter().map(|h| parse_hand(h)).collect();
    for i in 0..range.len().min(range_sample) {
        let j = i + (next(&mut rng) % (range.len() - i) as u64) as usize;
        range.swap(i, j);
    }
    range.truncate(range_sample);

    // Shared flops, each with the range hands that continue on it.
    let flops: Vec<Vec<usize>> = (0..n_flops).map(|_| deal(3, &[], &mut rng)).collect();
    let bomb = OmahaBombPotGameEvaluation {};
    let continuing: Vec<Vec<usize>> = flops
        .par_iter()
        .enumerate()
        .map(|(f, flop)| {
            range
                .iter()
                .enumerate()
                .filter(|(_, h)| !overlaps(h, flop))
                .filter(|(i, h)| {
                    let mut s = OmahaBombPotGameState::new(1, cards as u32);
                    s.add_player(to_deck(h)).unwrap();
                    s.set_flop(0, to_deck(flop)).unwrap();
                    let r = bomb
                        .sample_seeded(
                            &s,
                            &[1],
                            cont_samples,
                            Some(seed ^ ((f * 100_003 + i) as u64)),
                        )
                        .unwrap();
                    r.equity_sum[0] / cont_samples as f64 >= cont
                })
                .map(|(i, _)| i)
                .collect()
        })
        .collect();
    let avg_cont = continuing.iter().map(|c| c.len()).sum::<usize>() as f64 / n_flops as f64;
    eprintln!(
        "{n_flops} flops; on average {:.0} of {} range hands ({:.0}%) continue",
        avg_cont,
        range.len(),
        avg_cont / range.len() as f64 * 100.0
    );

    let eval = OmahaGameEvaluation {};
    let rows: Vec<String> = hands
        .par_iter()
        .enumerate()
        .map(|(i, hand)| {
            let hero = parse_hand(hand);
            let mut rng = seed ^ (i as u64 + 7).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let (mut played, mut good, mut surplus, mut eq_total) = (0, 0, 0.0, 0.0);
            let mut tries = 0;
            while played < hand_flops && tries < hand_flops * 4 {
                tries += 1;
                let f = (next(&mut rng) % n_flops as u64) as usize;
                if overlaps(&flops[f], &hero) {
                    continue;
                }
                let pool: Vec<usize> = continuing[f]
                    .iter()
                    .copied()
                    .filter(|&o| !overlaps(&range[o], &hero))
                    .collect();
                if pool.is_empty() {
                    continue;
                }
                let mut eq = 0.0;
                for _ in 0..opponents {
                    let opp = &range[pool[(next(&mut rng) % pool.len() as u64) as usize]];
                    let mut s = OmahaGameState::new(cards as u32);
                    s.add_player(to_deck(&hero)).unwrap();
                    s.add_player(to_deck(opp)).unwrap();
                    s.set_flop(to_deck(&flops[f])).unwrap();
                    eq += eval.sample_equity_seeded(&s, runouts, Some(next(&mut rng)))[0]
                        .to_f64()
                        .unwrap_or(0.0);
                }
                let e = eq / opponents as f64;
                played += 1;
                eq_total += e;
                surplus += (e - 0.5).max(0.0);
                if e >= favorable {
                    good += 1;
                }
            }
            let p = played.max(1) as f64;
            format!(
                "{hand},{:.4},{:.4},{:.4}",
                good as f64 / p,
                surplus / p,
                eq_total / p
            )
        })
        .collect();

    let mut out = std::io::BufWriter::new(std::fs::File::create(&args[4]).expect("create output"));
    writeln!(out, "hand,favorable,surplus,flop_equity").unwrap();
    for row in rows {
        writeln!(out, "{row}").unwrap();
    }
    eprintln!("wrote {} PLO{cards} hands to {}", hands.len(), args[4]);
}
