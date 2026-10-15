//! Computes the six-handed preflop equity tables (#248) and saves them.
//!
//!     cargo run --release -p ducy-gto --example sixmax_equity -- \
//!         --samples 20000 --out preflop-equity.bin
//!
//! Prints a few well-known matchups as a check.

use std::time::Instant;

use ducy_gto::{
    holdem::{chart::class_name, iso::NUM_PREFLOP_CLASSES},
    sixmax::equity::PreflopEquity,
};

fn main() {
    let mut samples = 20_000u32;
    let mut out = String::from("preflop-equity.bin");
    let mut seed = 1u64;
    let mut threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut v = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--samples" => samples = v().parse().expect("--samples"),
            "--out" => out = v(),
            "--seed" => seed = v().parse().expect("--seed"),
            "--threads" => threads = v().parse().expect("--threads"),
            f => panic!("unknown option {f}"),
        }
    }
    let t = Instant::now();
    let table = PreflopEquity::compute(samples, seed, threads);
    std::fs::write(&out, table.to_bytes()).expect("write --out");
    println!(
        "{samples} samples a pair, {:.0}s, written to {out}",
        t.elapsed().as_secs_f64()
    );
    let class = |name: &str| {
        (0..NUM_PREFLOP_CLASSES)
            .find(|&c| class_name(c) == name)
            .expect("class")
    };
    for (a, b) in [("AA", "KK"), ("AKo", "22"), ("AKs", "QQ"), ("KQs", "AJo")] {
        println!(
            "{a} vs {b}: {:.1}%",
            100.0 * table.heads_up(class(a), class(b))
        );
    }
    for name in ["AA", "KK", "AKs", "T9s", "22", "72o"] {
        let row: Vec<String> = (1..=5)
            .map(|k| format!("{:.1}", 100.0 * table.vs_random(class(name), k)))
            .collect();
        println!("{name} vs 1-5 random hands: {}%", row.join(" / "));
    }
}
