//! Trains Leduc hold'em with Monte Carlo CFR under each discounting scheme
//! and prints exploitability (milli-chips per hand) as training goes, next to
//! full-tree CFR+ for reference.
//!
//!     cargo run --release -p ducy-gto --example mccfr_leduc

use std::time::Instant;

use ducy_gto::{
    Cfr, Config, Discount, Mccfr, Prune, Variant, expected_value, exploitability,
    games::leduc::{GAME_VALUE, Leduc},
};

fn main() {
    let checkpoints = [10_000u64, 100_000, 1_000_000, 4_000_000];
    let runs: [(&str, Discount, Option<Prune>); 4] = [
        ("plain", Discount::None, None),
        ("linear", Discount::Linear { until: u64::MAX }, None),
        ("dcfr", Discount::DCFR, None),
        (
            "dcfr+prune",
            Discount::DCFR,
            Some(Prune {
                after: 100_000,
                threshold: -20.0,
                full_every: 20,
            }),
        ),
    ];
    println!("Leduc hold'em, external-sampling MCCFR (value at equilibrium {GAME_VALUE})");
    println!(
        "{:>11} {:>9} {:>16} {:>9} {:>8}",
        "run", "iters", "exploit (mchips)", "value", "time"
    );
    for (name, discount, prune) in runs {
        let mut m = Mccfr::new(
            &Leduc,
            Config {
                seed: 7,
                batch: 256,
                discount,
                prune,
            },
        );
        let start = Instant::now();
        let mut done = 0;
        for &n in &checkpoints {
            m.run(n - done);
            done = n;
            let avg = m.average();
            println!(
                "{name:>11} {n:>9} {:>16.3} {:>9.4} {:>7.1}s",
                1000.0 * exploitability(&Leduc, &avg),
                expected_value(&Leduc, &avg),
                start.elapsed().as_secs_f64()
            );
        }
    }
    let mut cfr = Cfr::new(&Leduc, Variant::Plus);
    let start = Instant::now();
    cfr.run(1000);
    println!(
        "{:>11} {:>9} {:>16.3} {:>9.4} {:>7.1}s",
        "full CFR+",
        1000,
        1000.0 * exploitability(&Leduc, &cfr.average()),
        expected_value(&Leduc, &cfr.average()),
        start.elapsed().as_secs_f64()
    );
}
