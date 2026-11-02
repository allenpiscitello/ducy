//! Heads-up Hold'em MCCFR speed: iterations per second once the tables are
//! warm, to check that solver changes don't slow training.
//!
//!     cargo run --release -p ducy-gto --example mccfr_speed -- \
//!         --cards abstraction.bin --warm 500000 --iters 200000 --repeat 5
//!
//! Warms with `--warm` iterations (so most information sets exist), then times
//! `--repeat` runs of `--iters` each and prints every rate and the best.

use std::{path::PathBuf, time::Instant};

use ducy_gto::{
    Config, Discount, Mccfr,
    holdem::{
        abstraction::CardAbstraction,
        hunl::{Hunl, HunlConfig},
    },
};

fn main() {
    let mut cards = PathBuf::new();
    let (mut warm, mut iters, mut repeat, mut batch) = (500_000u64, 200_000u64, 5u32, 4096u64);
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--cards" => cards = value().into(),
            "--warm" => warm = value().replace('_', "").parse().expect("--warm"),
            "--iters" => iters = value().replace('_', "").parse().expect("--iters"),
            "--repeat" => repeat = value().parse().expect("--repeat"),
            "--batch" => batch = value().parse().expect("--batch"),
            f => panic!("unknown option {f}"),
        }
    }
    assert!(!cards.as_os_str().is_empty(), "--cards FILE is required");
    let cards = CardAbstraction::load(&std::fs::read(&cards).expect("read --cards"))
        .expect("a card abstraction");
    let game = Hunl::new(HunlConfig::default(), Some(&cards));
    let config = Config {
        seed: 1,
        batch,
        discount: Discount::DCFR,
        prune: None,
    };
    let mut solver = Mccfr::new(&game, config);
    solver.run(warm);
    let mut best = 0f64;
    for _ in 0..repeat {
        let t = Instant::now();
        solver.run(iters);
        let rate = iters as f64 / t.elapsed().as_secs_f64();
        best = best.max(rate);
        println!("{rate:.0}/s");
    }
    println!("best {best:.0}/s, {} infosets", solver.num_infosets());
}
