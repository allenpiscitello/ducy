//! Builds the heads-up Hold'em card abstraction and saves it.
//!
//!     cargo run --release -p ducy-gto --example build_abstraction -- [out-file] [flop turn river buckets]
//!
//! Defaults: `abstraction.bin`, 200 buckets per postflop street. Prints how
//! long each stage takes, the table sizes, and some sanity checks.
//!
//! It also writes the compact form next to it (`<out-file>.compact`, about
//! 100 KB, for shipping to the browser) and checks that it gives the same
//! bucket as the tables for thousands of random hands. With `COMPARE=old.bin`
//! in the environment it also checks that an earlier build of the same
//! settings assigns every hand identically (the build is deterministic).

use std::time::Instant;

use ducy_gto::holdem::{
    abstraction::{AbstractionConfig, CardAbstraction},
    cards::{Card, parse},
    equity::equity,
};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args
        .first()
        .cloned()
        .unwrap_or_else(|| "abstraction.bin".into());
    let mut config = AbstractionConfig::default();
    if args.len() >= 4 {
        config.flop_buckets = args[1].parse().expect("flop buckets");
        config.turn_buckets = args[2].parse().expect("turn buckets");
        config.river_buckets = args[3].parse().expect("river buckets");
    }
    println!("{config:?}");
    let start = Instant::now();
    let mut last = Instant::now();
    let abs = CardAbstraction::build(config, |stage| {
        println!("[{:>7.1}s] {stage}", start.elapsed().as_secs_f64());
        last = Instant::now();
    });
    let _ = last;
    let built = start.elapsed().as_secs_f64();
    let bytes = abs.save();
    std::fs::write(&out, &bytes).expect("write the abstraction");
    let (flop, turn) = abs.table_sizes();
    println!(
        "built in {built:.1}s; {flop} flop hands, {turn} turn hands; {out}: {:.1} MB",
        bytes.len() as f64 / 1e6
    );
    let loaded = CardAbstraction::load(&std::fs::read(&out).unwrap()).expect("reload");
    assert_eq!(loaded, abs, "saved file reloads to the same abstraction");

    let show = |hole: &str, board: &str| {
        let h: Vec<Card> = parse(hole).unwrap();
        let b: Vec<Card> = parse(board).unwrap();
        let bucket = abs.bucket([h[0], h[1]], &b);
        let e = equity([h[0], h[1]], &b);
        println!("  {hole:<6} on {board:<15} bucket {bucket:>3}  (equity vs random {e:.3})");
        bucket
    };
    println!("sanity:");
    let set = show("9c 9h", "9s 4d 2h");
    let set2 = show("9d 9h", "9c 4s 2d");
    assert_eq!(set, set2, "isomorphic hands share a bucket");
    let draw = show("Ah Kh", "9h 4h 2c");
    let air = show("7c 6d", "Ks Qd 2h");
    assert!(set != draw && draw != air);
    let nuts = show("Th Jh", "Ah Kh Qh 2c 7d");
    let second = show("9h 8h", "Ah Kh Qh 2c 7d");
    let weak = show("3c 4d", "Ah Kh Qh 2c 7d");
    assert!(nuts as usize == config.river_buckets - 1 && nuts >= second && second > weak);
    let compact = abs.compact().expect("a fresh build keeps its centres");
    let compact_path = format!("{out}.compact");
    std::fs::write(&compact_path, compact.save()).expect("write the compact abstraction");
    println!(
        "{compact_path}: {:.1} KB",
        compact.save().len() as f64 / 1e3
    );
    let mut rng = ducy_gto::Rng::new(77);
    let t = Instant::now();
    let checks = 2000;
    for i in 0..checks {
        let mut cards: Vec<Card> = Vec::new();
        while cards.len() < 6 {
            let c = (rng.next_u64() % 52) as Card;
            if !cards.contains(&c) {
                cards.push(c);
            }
        }
        let n = if i % 2 == 0 { 3 } else { 4 };
        let hole = [cards[0], cards[1]];
        let board = &cards[2..2 + n];
        assert_eq!(
            compact.bucket(hole, board),
            abs.bucket(hole, board),
            "{cards:?}"
        );
    }
    println!(
        "compact buckets match the tables on {checks} random flop and turn hands ({:.1} ms each)",
        t.elapsed().as_secs_f64() * 1e3 / checks as f64
    );
    if let Ok(old) = std::env::var("COMPARE") {
        let old = CardAbstraction::load(&std::fs::read(&old).expect("read COMPARE"))
            .expect("an abstraction");
        assert!(
            abs.same_buckets(&old),
            "the rebuild differs from {old:?}",
            old = old.config
        );
        println!("identical buckets to the earlier build");
    }
    println!("ok");
}
