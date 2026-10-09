//! Builds the PLO card abstraction and measures it (#128):
//!
//!     cargo run --release -p ducy-gto --example plo_abstraction -- --out plo-cards.bin
//!
//! Options: `--out FILE`, `--preflop N` (0 for one bucket per class),
//! `--flop N`, `--turn N`, `--river N`, `--fit N` (sampled hands per street),
//! `--check N` (hands per street for the quality check).
//!
//! For each street it reports the within-bucket spread of equity (measured
//! with 2,000 samples per hand, ±1.1%) for:
//! - these buckets;
//! - equity buckets at the same cost: sampled equity with as many samples as
//!   these features cost, in equal-mass bins (the plain-equity baseline);
//! - equity buckets from a separate 2,000-sample estimate: what equity
//!   buckets reach when cost is no object (hundreds of times too slow for
//!   training), with the measurement's own noise.
//!
//! Then the bucketing cost per hand, its share of a training iteration, and
//! the file's size and load time.

use std::time::Instant;

use ducy_gto::{
    Config, Discount, Mccfr, Rng,
    holdem::{
        cards::Card,
        hunl::{Buckets, HuPlo, HunlConfig},
    },
    omaha::{
        abstraction::{BoardView, PloAbstraction, PloAbstractionConfig, random_spot},
        showdown::{PairTable, equity_vs_random},
    },
};

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn num(name: &str, default: usize) -> usize {
    arg(name).map_or(default, |v| v.replace('_', "").parse().expect(name))
}

/// The mean within-bucket standard deviation of `values`, weighted by
/// bucket size.
fn spread(buckets: &[u16], values: &[f64]) -> f64 {
    let n = buckets.iter().map(|&b| b as usize).max().unwrap_or(0) + 1;
    let mut sum = vec![0.0; n];
    let mut sq = vec![0.0; n];
    let mut count = vec![0.0; n];
    for (&b, &v) in buckets.iter().zip(values) {
        sum[b as usize] += v;
        sq[b as usize] += v * v;
        count[b as usize] += 1.0;
    }
    let mut total = 0.0;
    for b in 0..n {
        if count[b] > 0.0 {
            let mean = sum[b] / count[b];
            total += count[b] * (sq[b] / count[b] - mean * mean).max(0.0).sqrt();
        }
    }
    total / values.len() as f64
}

/// Equal-mass bins of `values`.
fn quantile_buckets(values: &[f64], buckets: usize) -> Vec<u16> {
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
    let mut out = vec![0u16; values.len()];
    for (rank, &i) in order.iter().enumerate() {
        out[i] = (rank * buckets / values.len()) as u16;
    }
    out
}

fn par_map<T: Send, R: Send>(items: Vec<T>, f: impl Fn(T) -> R + Sync + Send) -> Vec<R> {
    use rayon::prelude::*;
    items.into_par_iter().map(f).collect()
}

fn main() {
    let config = PloAbstractionConfig {
        preflop: num("--preflop", 500),
        flop: num("--flop", 200),
        turn: num("--turn", 200),
        river: num("--river", 200),
        fit_hands: num("--fit", 40_000),
        ..PloAbstractionConfig::default()
    };
    let check = num("--check", 4000);
    let start = Instant::now();
    let cards = PloAbstraction::build(config.clone(), |s| {
        println!("[{:>5.1}s] {s}", start.elapsed().as_secs_f64())
    });
    let bytes = cards.save();
    if let Some(out) = arg("--out") {
        std::fs::write(&out, &bytes).expect("write --out");
    }
    let t = Instant::now();
    let loaded = PloAbstraction::load(&bytes).expect("loads");
    println!(
        "file: {:.1} KB; native load {:.0} ms (enumerating the preflop classes)",
        bytes.len() as f64 / 1e3,
        t.elapsed().as_secs_f64() * 1e3
    );
    assert_eq!(loaded.save(), bytes, "load and save round-trip");

    println!(
        "\nWithin-bucket spread of equity (std. dev., {check} hands per street, equity from 2,000 samples):"
    );
    println!(
        "| Street | Buckets | These buckets | Equity buckets, same cost | Equity buckets, 2,000 samples | Cost per hand |"
    );
    println!("|---|---|---|---|---|---|");
    for (name, board_len) in [("Flop", 3), ("Turn", 4), ("River", 5)] {
        let spots: Vec<(Vec<Card>, Vec<Card>, f64)> = par_map((0..check).collect(), |i| {
            let mut rng = Rng::new(99 + board_len as u64 * 7919 + i as u64 * 104_729);
            let (hole, board) = random_spot(4, board_len, &mut rng);
            let e = equity_vs_random(&hole, &board, 2000, &mut rng).mean;
            (hole, board, e)
        });
        let truth: Vec<f64> = spots.iter().map(|s| s.2).collect();
        // Cost: the board's work shared by two hands, plus each hand's own.
        let t = Instant::now();
        let ours: Vec<u16> = spots
            .iter()
            .map(|(h, b, _)| cards.bucket_on(h, &BoardView::new(b)))
            .collect();
        let per_hand_us = t.elapsed().as_secs_f64() * 1e6 / check as f64;
        let view_cost = {
            let t = Instant::now();
            for (_, b, _) in spots.iter().take(500) {
                std::hint::black_box(BoardView::new(b));
            }
            t.elapsed().as_secs_f64() * 1e6 / 500.0
        };
        let shared_cost = per_hand_us - view_cost / 2.0;
        // Sampled equity's budget at the same cost: about 2.5 µs per sample
        // before the river. On the river a sample is 0.25 µs once the board's
        // pair table is built, so the baseline gets the same per-board
        // sharing: its half of a table, then samples.
        let k = if board_len == 5 {
            let t = Instant::now();
            for (_, b, _) in spots.iter().take(500) {
                let board = [b[0], b[1], b[2], b[3], b[4]];
                std::hint::black_box(PairTable::new(&board));
            }
            let table = t.elapsed().as_secs_f64() * 1e6 / 500.0;
            ((shared_cost - table / 2.0) / 0.25).round() as usize
        } else {
            (shared_cost / 2.5).round() as usize
        }
        .max(1);
        let sampled = |k: usize, salt: u64| -> Vec<f64> {
            par_map(spots.clone(), |(h, b, _)| {
                let mut rng = Rng::new(h.iter().chain(&b).fold(salt, |a, &c| a * 53 + c as u64));
                equity_vs_random(&h, &b, k, &mut rng).mean
            })
        };
        let n = cards.bucket_count(board_len);
        println!(
            "| {name} | {n} | {:.3} | {:.3} ({k} samples) | {:.3} | {shared_cost:.0} µs |",
            spread(&ours, &truth),
            spread(&quantile_buckets(&sampled(k, 3), n), &truth),
            spread(&quantile_buckets(&sampled(2000, 4), n), &truth),
        );
    }

    // Preflop: the classes' equity spread inside each bucket.
    let mut rng = Rng::new(5);
    let pre: Vec<(u16, f64)> = (0..check)
        .map(|_| {
            let (h, _) = random_spot(4, 0, &mut rng);
            (
                cards.preflop_bucket(&h),
                equity_vs_random(&h, &[], 2000, &mut rng).mean,
            )
        })
        .collect();
    let (b, e): (Vec<u16>, Vec<f64>) = pre.into_iter().unzip();
    println!(
        "| Preflop | {} | {:.3} | | | lookup |",
        cards.bucket_count(0),
        spread(&b, &e),
    );

    // A whole deal's buckets, and their share of training time.
    let mut rng = Rng::new(6);
    let deals: Vec<([Card; 4], [Card; 4], [Card; 5])> = (0..2000)
        .map(|_| {
            let (h, b) = random_spot(8, 5, &mut rng);
            (
                [h[0], h[1], h[2], h[3]],
                [h[4], h[5], h[6], h[7]],
                [b[0], b[1], b[2], b[3], b[4]],
            )
        })
        .collect();
    let t = Instant::now();
    for (a, b, board) in &deals {
        std::hint::black_box(cards.deal_buckets([a, b], board));
    }
    let per_deal = t.elapsed().as_secs_f64() / deals.len() as f64;
    let game = HuPlo::with_cards(HunlConfig::pot_limit_omaha(), Some(&cards));
    let mut m = Mccfr::new(
        &game,
        Config {
            seed: 1,
            batch: 4096,
            discount: Discount::DCFR,
            prune: None,
        },
    );
    m.run(20_000);
    let t = Instant::now();
    let iters = 100_000;
    m.run(iters);
    let rate = iters as f64 / t.elapsed().as_secs_f64();
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;
    println!(
        "\nBuckets for a whole deal (both players, every street): {:.0} µs.\n\
         MCCFR with these buckets, PLO 100 BB: {rate:.0} iterations/s on {cores} cores; \
         bucketing is about {:.0}% of the time.",
        per_deal * 1e6,
        100.0 * (per_deal * rate / cores).min(1.0)
    );
}
