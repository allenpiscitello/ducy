//! Solves Kuhn poker and Leduc hold'em with CFR and CFR+ and prints how
//! fast each converges: exploitability (what a best response wins against
//! the average strategy, in milli-chips per hand) and player 0's value.
//!
//!     cargo run --release -p ducy-gto --example toy_games

use std::time::Instant;

use ducy_gto::{
    Cfr, Game, Variant, expected_value, exploitability,
    games::{kuhn, leduc},
};

fn report<G: Game>(name: &str, game: &G, value: f64, checkpoints: &[u64]) {
    println!("\n{name} (player 0's equilibrium value {value:.4})");
    println!(
        "{:>8} {:>8} {:>16} {:>10} {:>9}",
        "variant", "iters", "exploit (mchips)", "value", "time"
    );
    for variant in [Variant::Vanilla, Variant::Plus] {
        let mut cfr = Cfr::new(game, variant);
        let start = Instant::now();
        let mut done = 0;
        for &n in checkpoints {
            cfr.run(n - done);
            done = n;
            let avg = cfr.average();
            println!(
                "{:>8} {:>8} {:>16.3} {:>10.4} {:>8.2}s",
                format!("{variant:?}"),
                n,
                1000.0 * exploitability(game, &avg),
                expected_value(game, &avg),
                start.elapsed().as_secs_f64()
            );
        }
        if variant == Variant::Plus {
            println!("{} information sets", cfr.num_infosets());
        }
    }
}

fn main() {
    report(
        "Kuhn poker",
        &kuhn::Kuhn,
        kuhn::GAME_VALUE,
        &[10, 100, 1000, 10_000],
    );
    report(
        "Leduc hold'em",
        &leduc::Leduc,
        leduc::GAME_VALUE,
        &[10, 100, 1000, 3000],
    );
}
