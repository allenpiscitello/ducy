//! Measures range-aware play (#205): each personality judging hands against
//! opponents' ranges, head to head against itself judging against random
//! hands, and both against EquityBot.
//!
//!     cargo run --release -p ducy-bots --example range_bench -- --game nlhe --deals 400 --chunks 8
//!
//! Options: `--game nlhe|plo4|plo6`, `--deals N` per chunk (default 300),
//! `--chunks N` (default 8; each chunk is a duplicate match with its own
//! seed, run in parallel, and the 95% intervals come from the spread across
//! chunks), `--only NAME` (one personality, by name or id).

use std::thread;

use ducy_bots::{MatchConfig, Personality, PersonalityBot, bots::EquityBot, run_match};
use ducy_play::{Bot, TableRules, Variant};

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn mean_ci(x: &[f64]) -> (f64, f64) {
    let n = x.len().max(2) as f64;
    let m = x.iter().sum::<f64>() / n;
    let v = x.iter().map(|y| (y - m).powi(2)).sum::<f64>() / (n - 1.0);
    (m, 1.96 * (v / n).sqrt())
}

fn bot(p: Personality, ranges: bool, seed: u64) -> PersonalityBot {
    let mut style = p.style();
    style.ranges = ranges;
    PersonalityBot::new(p.name(), style, Some(seed))
}

/// bb/100 for bot 0 in each chunk's duplicate match between `make(seed)`'s two bots.
fn chunks<F>(rules: TableRules, deals: usize, n: usize, make: F) -> Vec<f64>
where
    F: Fn(u64) -> [Box<dyn Bot>; 2] + Sync,
{
    thread::scope(|s| {
        let handles: Vec<_> = (0..n)
            .map(|c| {
                let make = &make;
                s.spawn(move || {
                    let mut bots: Vec<Box<dyn Bot>> = make(c as u64 * 7 + 1).into();
                    let config = MatchConfig::new(rules, deals, 1000 + c as u64).duplicate();
                    run_match(&config, &mut bots).expect("match").bb_per_100(0)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("chunk"))
            .collect()
    })
}

fn main() {
    let game = arg("--game").unwrap_or_else(|| "nlhe".into());
    let rules = match game.as_str() {
        "nlhe" => TableRules::no_limit_holdem(1, 2),
        "plo4" => TableRules::pot_limit_omaha(1, 2),
        "plo6" => TableRules::pot_limit_omaha(1, 2).with_variant(Variant::Omaha { hole_cards: 6 }),
        g => panic!("unknown game {g}"),
    };
    let deals: usize = arg("--deals").map_or(300, |v| v.parse().expect("--deals"));
    let n: usize = arg("--chunks").map_or(8, |v| v.parse().expect("--chunks"));
    let only = arg("--only");
    println!(
        "{game}: {n} chunks of {deals} duplicate deals; bb/100 ± 95%\n\
         {:<20} {:>16} {:>16} {:>16} {:>16}",
        "personality", "ranges vs old", "ranges vs Equity", "old vs Equity", "gain vs Equity"
    );
    for p in Personality::ALL {
        if only
            .as_deref()
            .is_some_and(|o| o != p.name() && o != p.id())
        {
            continue;
        }
        let head = chunks(rules, deals, n, |s| {
            [Box::new(bot(p, true, s)), Box::new(bot(p, false, s + 1))]
        });
        let new = chunks(rules, deals, n, |s| {
            [
                Box::new(bot(p, true, s)),
                Box::new(EquityBot::new(150, Some(s + 1))),
            ]
        });
        let old = chunks(rules, deals, n, |s| {
            [
                Box::new(bot(p, false, s)),
                Box::new(EquityBot::new(150, Some(s + 1))),
            ]
        });
        let gain: Vec<f64> = new.iter().zip(&old).map(|(a, b)| a - b).collect();
        let f = |x: &[f64]| {
            let (m, ci) = mean_ci(x);
            format!("{m:+7.1} ± {ci:5.1}")
        };
        println!(
            "{:<20} {:>16} {:>16} {:>16} {:>16}",
            p.name(),
            f(&head),
            f(&new),
            f(&old),
            f(&gain)
        );
    }
}
