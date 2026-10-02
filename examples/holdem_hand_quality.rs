//! Measures all 169 Hold'em starting-hand classes for building hand rankings:
//! raw equity, and how often each flops well against opponents who are strong
//! on that flop.
//!
//! cargo run --release --example holdem_hand_quality -- <out.csv> [range=<file>] [key=value ...]
//!
//! Options (defaults): hu_samples=20000 flops=600 opponents=6 continue=0.7
//!   continue_samples=40 favorable=0.5 seed=1
//!
//! - `hu`: equity against a random hand.
//! - With `range=<file>` (Hold'em range text such as "QQ+, AKs, ..."), opponents
//!   play that preflop range, and on a flop only the part of it that has at
//!   least `continue` equity against a random hand there gives action. Every
//!   class is measured on the same `flops` (skipping ones that share a card),
//!   facing `opponents` continuing hands per flop, with exact turn and river
//!   enumeration:
//!   - `favorable`: share of flops with at least `favorable` equity
//!   - `surplus`: mean of max(0, equity - 0.5)
//!   - `flop_equity`: mean equity against continuing opponents
//!
//! Output lines are `class,combos,hu,favorable,surplus,flop_equity`; the flop
//! columns are empty without a range. Seeded and reproducible.

use std::collections::HashMap;
use std::io::Write;

use ducy::deck::Deck;
use ducy::deck::range::Range;
use ducy::games::GameEquityEvaluation;
use ducy::games::flop_game::FlopGame;
use ducy::games::holdem::{HoldemGameEvaluation, HoldemGameState, HoldemRange};
use ducy::ranking::hand_rank::{StandardHandRanker, StandardHandRanks};
use rayon::prelude::*;
use rust_decimal::prelude::ToPrimitive;

const RANKS: &[u8] = b"AKQJT98765432";
const ANY_TWO: &str = "22+, A2s+, K2s+, Q2s+, J2s+, T2s+, 92s+, 82s+, 72s+, 62s+, 52s+, 42s+, 32s, \
    A2o+, K2o+, Q2o+, J2o+, T2o+, 92o+, 82o+, 72o+, 62o+, 52o+, 42o+, 32o";

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn bits(d: Deck) -> u64 {
    u64::from(d)
}

#[derive(Clone)]
struct FlopStats {
    favorable: f64,
    surplus: f64,
    flop_equity: f64,
    implied_win: f64,
    implied_loss: f64,
}

/// Trips or better: a hand strong enough to stack an opponent who also likes the board.
fn is_big(rank: &StandardHandRanks) -> bool {
    matches!(
        rank,
        StandardHandRanks::ThreeOfAKind { .. }
            | StandardHandRanks::Straight { .. }
            | StandardHandRanks::Flush { .. }
            | StandardHandRanks::FullHouse { .. }
            | StandardHandRanks::FourOfAKind { .. }
            | StandardHandRanks::StraightFlush { .. }
    )
}

/// The 169 classes in grid order, each with one representative combo (all
/// combos of a class play the same against random flops by suit symmetry).
fn classes() -> Vec<(String, Deck, u32)> {
    let mut out = Vec::new();
    for (r, &a) in RANKS.iter().enumerate() {
        for (c, &b) in RANKS.iter().enumerate() {
            let (a, b) = (a as char, b as char);
            let (label, cards, combos) = if r == c {
                (format!("{a}{b}"), format!("{a}s {b}h"), 6)
            } else if r < c {
                (format!("{a}{b}s"), format!("{a}s {b}s"), 4)
            } else {
                (format!("{b}{a}o"), format!("{b}s {a}h"), 12)
            };
            out.push((label, Deck::parse(&cards).unwrap(), combos));
        }
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: holdem_hand_quality <out.csv> [range=<file>] [key=value ...]");
        std::process::exit(2);
    }
    let opts: HashMap<String, String> = args[2..]
        .iter()
        .map(|a| {
            let (k, v) = a.split_once('=').expect("options are key=value");
            (k.to_string(), v.to_string())
        })
        .collect();
    let num = |k: &str, d: f64| opts.get(k).map_or(d, |v| v.parse().expect(k));
    let hu_samples = num("hu_samples", 20000.0) as usize;
    let n_flops = num("flops", 600.0) as usize;
    let opponents = num("opponents", 6.0) as usize;
    let cont = num("continue", 0.7);
    let cont_samples = num("continue_samples", 40.0) as usize;
    let favorable = num("favorable", 0.5);
    let implied_runouts = num("implied_runouts", 40.0) as usize;
    let seed = num("seed", 1.0) as u64;

    let eval = HoldemGameEvaluation {};
    let any_two = HoldemRange::parse(ANY_TWO).unwrap();
    let classes = classes();

    let hu: Vec<f64> = classes
        .par_iter()
        .enumerate()
        .map(|(i, (_, hand, _))| {
            let mut s = HoldemGameState::new();
            s.add_player(*hand).unwrap();
            let r = eval
                .sample_range_equity_seeded(
                    &s,
                    std::slice::from_ref(&any_two),
                    hu_samples,
                    Some(seed ^ i as u64),
                )
                .unwrap();
            r.equity_sum[0] / r.samples as f64
        })
        .collect();

    let flop_cols: Vec<Option<FlopStats>> = match opts.get("range") {
        None => vec![None; classes.len()],
        Some(path) => {
            let text = std::fs::read_to_string(path).expect("range file");
            let range: Vec<Deck> = HoldemRange::parse(&text)
                .expect("range")
                .iter()
                .map(|x| x.get_deck())
                .collect();

            let mut rng = seed;
            let mut flops = Vec::with_capacity(n_flops);
            while flops.len() < n_flops {
                let mut d = Deck::empty();
                while d.num_cards() < 3 {
                    let c = Deck::all_cards()
                        .try_get_nth_card((next(&mut rng) % 52) as usize)
                        .unwrap();
                    d |= c;
                }
                flops.push(d);
            }
            // On each flop, the range hands that give action there.
            let continuing: Vec<Vec<Deck>> = flops
                .par_iter()
                .enumerate()
                .map(|(f, &flop)| {
                    range
                        .iter()
                        .enumerate()
                        .filter(|(_, h)| bits(**h) & bits(flop) == 0)
                        .filter(|(i, h)| {
                            let mut s = HoldemGameState::new();
                            s.add_player(**h).unwrap();
                            s.set_flop(flop).unwrap();
                            let r = eval
                                .sample_range_equity_seeded(
                                    &s,
                                    std::slice::from_ref(&any_two),
                                    cont_samples,
                                    Some(seed ^ ((f * 100_003 + i) as u64)),
                                )
                                .unwrap();
                            r.equity_sum[0] / r.samples as f64 >= cont
                        })
                        .map(|(_, h)| *h)
                        .collect()
                })
                .collect();
            let avg = continuing.iter().map(|c| c.len()).sum::<usize>() as f64 / n_flops as f64;
            eprintln!(
                "{n_flops} flops; on average {avg:.0} of {} range combos ({:.0}%) give action",
                range.len(),
                avg / range.len() as f64 * 100.0
            );

            classes
                .par_iter()
                .enumerate()
                .map(|(i, (_, hand, _))| {
                    let mut rng = seed ^ (i as u64 + 11).wrapping_mul(0x9E37_79B9_7F4A_7C15);
                    let (mut played, mut good, mut surplus, mut eq_total) = (0, 0, 0.0, 0.0);
                    let (mut big_win, mut big_loss, mut sampled) = (0u64, 0u64, 0u64);
                    for (f, &flop) in flops.iter().enumerate() {
                        if bits(flop) & bits(*hand) != 0 {
                            continue;
                        }
                        let pool: Vec<Deck> = continuing[f]
                            .iter()
                            .copied()
                            .filter(|o| bits(*o) & bits(*hand) == 0)
                            .collect();
                        if pool.is_empty() {
                            continue;
                        }
                        let mut eq = 0.0;
                        for _ in 0..opponents {
                            let opp = pool[(next(&mut rng) % pool.len() as u64) as usize];
                            let mut s = HoldemGameState::new();
                            s.add_player(*hand).unwrap();
                            s.add_player(opp).unwrap();
                            s.set_flop(flop).unwrap();
                            eq += eval.evaluate_equity(&s)[0].to_f64().unwrap_or(0.0);

                            // Implied odds: on sampled turns and rivers, how often the hand
                            // makes a big hand of its own (trips or better, beating the
                            // board) and wins with it, or makes one and still loses.
                            let mut rest = Deck::all_cards();
                            rest -= *hand;
                            rest -= opp;
                            rest -= flop;
                            let n = rest.num_cards() as u64;
                            for _ in 0..implied_runouts {
                                let a = (next(&mut rng) % n) as usize;
                                let mut b = (next(&mut rng) % (n - 1)) as usize;
                                if b >= a {
                                    b += 1;
                                }
                                let mut board = flop;
                                board |= rest.try_get_nth_card(a).unwrap();
                                board |= rest.try_get_nth_card(b).unwrap();
                                let mine = StandardHandRanker::get_rank(&(board | *hand));
                                let (ms, os, bs) = (
                                    mine.get_score(),
                                    StandardHandRanker::get_rank(&(board | opp)).get_score(),
                                    StandardHandRanker::get_rank(&board).get_score(),
                                );
                                if is_big(&mine) && ms > bs {
                                    if ms > os {
                                        big_win += 1;
                                    } else if ms < os {
                                        big_loss += 1;
                                    }
                                }
                                sampled += 1;
                            }
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
                    let r = sampled.max(1) as f64;
                    Some(FlopStats {
                        favorable: good as f64 / p,
                        surplus: surplus / p,
                        flop_equity: eq_total / p,
                        implied_win: big_win as f64 / r,
                        implied_loss: big_loss as f64 / r,
                    })
                })
                .collect()
        }
    };

    let mut out = std::io::BufWriter::new(std::fs::File::create(&args[1]).expect("create output"));
    writeln!(
        out,
        "class,combos,hu,favorable,surplus,flop_equity,implied_win,implied_loss"
    )
    .unwrap();
    for (i, (label, _, combos)) in classes.iter().enumerate() {
        match &flop_cols[i] {
            Some(f) => writeln!(
                out,
                "{label},{combos},{:.5},{:.4},{:.4},{:.4},{:.5},{:.5}",
                hu[i], f.favorable, f.surplus, f.flop_equity, f.implied_win, f.implied_loss
            )
            .unwrap(),
            None => writeln!(out, "{label},{combos},{:.5},,,,,", hu[i]).unwrap(),
        }
    }
    eprintln!("wrote {} classes to {}", classes.len(), args[1]);
}
