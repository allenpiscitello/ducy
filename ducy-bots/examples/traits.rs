//! Every personality's traits (ducy_bots::traits), how far each is from its
//! targets, and which traits are out of tolerance. With `--targets`, prints
//! the measured traits as the `TARGETS` table for traits.rs instead.
//!
//! The targets come from seed 1; check drift on a fresh sample with another.
//!
//! cargo run --release -p ducy-bots --example traits -- [hands] [--seed N] [--targets]

use ducy_bots::Personality;
use ducy_bots::traits::{MeasureConfig, Trait, Traits, drift, measure, out_of_tolerance};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let emit = args.iter().any(|a| a == "--targets");
    let seed: u64 = args
        .iter()
        .position(|a| a == "--seed")
        .map_or(1, |i| args[i + 1].parse().expect("seed"));
    let hands: usize = args
        .iter()
        .enumerate()
        .find(|&(i, a)| !a.starts_with("--") && (i == 0 || args[i - 1] != "--seed"))
        .map_or(10_000, |(_, s)| s.parse().expect("hands"));
    let config = MeasureConfig::new(hands, seed);
    let rows: Vec<(Personality, Traits)> = std::thread::scope(|scope| {
        let handles: Vec<_> = Personality::ALL
            .into_iter()
            .map(|p| {
                scope.spawn(move || {
                    let mut bot = p.bot(Some(7));
                    (p, measure(&mut bot, Some(p), &config))
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    if emit {
        println!("// {hands} hands each, seed {seed}.");
        println!("const TARGETS: [[f64; TRAITS]; {}] = [", rows.len());
        for (p, t) in &rows {
            let cols: Vec<String> = t.0.iter().map(|v| format!("{v:.4}")).collect();
            println!("    [{}], // {}", cols.join(", "), p.name());
        }
        println!("];");
        return;
    }

    let names: Vec<&str> = Trait::ALL.iter().map(|t| t.name()).collect();
    println!("name,{},drift,out_of_tolerance", names.join(","));
    for (p, t) in &rows {
        let cols: Vec<String> = t.0.iter().map(|v| format!("{v:.3}")).collect();
        let out: Vec<String> = out_of_tolerance(*p, t)
            .iter()
            .map(|(t, d)| format!("{} {d:+.3}", t.name()))
            .collect();
        println!(
            "{},{},{:.2},{}",
            p.name(),
            cols.join(","),
            drift(*p, t),
            out.join(" ")
        );
    }
}
