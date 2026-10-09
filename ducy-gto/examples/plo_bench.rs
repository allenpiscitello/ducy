//! Speeds for heads-up PLO training (#129):
//!
//!     cargo run --release -p ducy-gto --example plo_bench
//!
//! - showdowns per second (two four-card hands on a full board), one core
//! - sampled equity against a random hand, per street, one core
//! - MCCFR iterations per second on every core, at 100 BB with the PLO menu:
//!   with every hand in one bucket (the tree and showdowns only), and with
//!   buckets that cost N equity samples per hand per street, to show what an
//!   abstraction can afford.

use std::time::Instant;

use ducy_gto::{
    Config, Discount, Mccfr, Rng,
    holdem::{
        cards::Card,
        hunl::{Buckets, HuPlo, HunlConfig},
    },
    omaha::showdown::{PairTable, RiverBoard, draw, equity_vs_random},
};

/// Buckets from `samples` equity samples (sampled equity in 16 bins), or a
/// hand's top two ranks when `samples` is 0.
struct Sampled {
    samples: usize,
}

impl Buckets for Sampled {
    fn hand_bucket(&self, hole: &[Card], board: &[Card]) -> u16 {
        if self.samples == 0 {
            let mut r: Vec<u16> = hole.iter().map(|&c| (c / 4) as u16).collect();
            r.sort_unstable();
            return r[3] / 4 * 4 + r[2] / 4;
        }
        let seed = hole
            .iter()
            .chain(board)
            .fold(17u64, |h, &c| h.wrapping_mul(31).wrapping_add(c as u64));
        let mut rng = Rng::new(seed);
        let e = equity_vs_random(hole, board, self.samples, &mut rng).mean;
        ((e * 16.0) as u16).min(15)
    }

    fn bucket_count(&self, _: usize) -> usize {
        16
    }
}

fn main() {
    let mut rng = Rng::new(1);
    let n = 200_000;
    let deals: Vec<([Card; 4], [Card; 4], [Card; 5])> = (0..n)
        .map(|_| {
            let mut used = 0;
            (
                std::array::from_fn(|_| draw(&mut used, &mut rng)),
                std::array::from_fn(|_| draw(&mut used, &mut rng)),
                std::array::from_fn(|_| draw(&mut used, &mut rng)),
            )
        })
        .collect();
    let t = Instant::now();
    let mut wins = 0i64;
    for (a, b, board) in &deals {
        let river = RiverBoard::new(board);
        wins += river.score(a).cmp(&river.score(b)) as i64;
    }
    let s = t.elapsed().as_secs_f64();
    println!(
        "showdowns: {:.0}/s on one core ({:.2} µs each; checksum {wins})",
        n as f64 / s,
        s * 1e6 / n as f64
    );
    let t = Instant::now();
    for (_, _, board) in deals.iter().take(2000) {
        std::hint::black_box(PairTable::new(board));
    }
    println!(
        "river pair table: {:.0} µs to build",
        t.elapsed().as_secs_f64() * 1e6 / 2000.0
    );
    for (street, cards) in [("preflop", 0), ("flop", 3), ("turn", 4), ("river", 5)] {
        let samples = 1000;
        let t = Instant::now();
        let mut total = 0.0;
        for (a, _, board) in deals.iter().take(200) {
            total += equity_vs_random(a, &board[..cards], samples, &mut rng).mean;
        }
        let per = t.elapsed().as_secs_f64() / (200 * samples) as f64;
        println!(
            "equity vs random, {street}: {:.2} µs per sample ({:.1} ms for 1,000; mean {:.3})",
            per * 1e6,
            per * 1e6,
            total / 200.0
        );
    }

    let config = HunlConfig::pot_limit_omaha();
    for samples in [None, Some(0), Some(25), Some(100)] {
        let cards = samples.map(|samples| Sampled { samples });
        let game = HuPlo::with_cards(config.clone(), cards.as_ref());
        let mut m = Mccfr::new(
            &game,
            Config {
                seed: 1,
                batch: 4096,
                discount: Discount::DCFR,
                prune: None,
            },
        );
        m.run(20_000); // warm up the tables
        let iters = match samples {
            Some(s) if s >= 100 => 100_000,
            _ => 400_000,
        };
        let t = Instant::now();
        m.run(iters);
        let rate = iters as f64 / t.elapsed().as_secs_f64();
        let label = match samples {
            None => "one bucket (tree + showdowns only)".to_string(),
            Some(0) => "top-two-ranks buckets (no equity)".to_string(),
            Some(s) => format!("{s} equity samples per hand per street"),
        };
        println!(
            "MCCFR, PLO 100 BB, {label}: {rate:.0} iterations/s on {} cores",
            std::thread::available_parallelism().map_or(1, |n| n.get())
        );
    }
    println!(
        "betting tree: {} nodes, {:?} decision sequences per street",
        HuPlo::<Sampled>::with_cards(config.clone(), None)
            .tree
            .nodes
            .len(),
        HuPlo::<Sampled>::with_cards(config, None)
            .tree_stats()
            .sequences
    );
}
