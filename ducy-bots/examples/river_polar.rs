//! Does the polarized river (`Style::polar_river`) win more? Each Regular
//! plays with it and without it on the same deals against the same five
//! opponents, six-handed duplicate, and the paired difference is printed.
//!
//!     cargo run --release -p ducy-bots --example river_polar -- --deals 1000

use std::time::Instant;

use ducy_bots::{
    MatchConfig,
    personality::{Personality, PersonalityBot},
    run_variants,
};
use ducy_play::{Bot, TableRules};

fn main() {
    let mut deals = 1000usize;
    let mut omaha = false;
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--deals" => deals = it.next().expect("--deals N").parse().expect("--deals"),
            "--omaha" => omaha = true,
            f => panic!("unknown option {f}"),
        }
    }
    let lineup = [
        Personality::DougPoker,
        Personality::JenSilly,
        Personality::MilkKing,
        Personality::RobinJaneLewd,
        Personality::BradOwned,
    ];
    let opponents = |k: usize, seed: u64| -> Box<dyn Bot> { Box::new(lineup[k].bot(Some(seed))) };
    let rules = if omaha {
        TableRules::pot_limit_omaha(1, 2)
    } else {
        TableRules::no_limit_holdem(1, 2)
    };
    let config = MatchConfig::new(rules, deals, 1).duplicate();
    let (mut sum, mut better) = (0.0, 0);
    for p in Personality::ALL {
        let t = Instant::now();
        let mut old = p.style();
        old.polar_river = false;
        let new = p.style();
        let a = move |s| Box::new(PersonalityBot::new(p.name(), old, Some(s))) as Box<dyn Bot>;
        let b = move |s| Box::new(PersonalityBot::new(p.name(), new, Some(s))) as Box<dyn Bot>;
        let r = run_variants(&config, &[&a, &b], &opponents, 6).expect("match");
        let d = r.compare(1, 0);
        sum += d.bb_per_100;
        if d.bb_per_100 > 0.0 {
            better += 1;
        }
        println!(
            "{:<20} {:+7.1} → {:+7.1} bb/100: polarized river {:+6.1} ± {:.1} ({} hands, {:.0}s)",
            p.name(),
            r.win_rate(0).bb_per_100,
            r.win_rate(1).bb_per_100,
            d.bb_per_100,
            d.ci95,
            r.hands,
            t.elapsed().as_secs_f64()
        );
    }
    println!(
        "\naverage {:+.1} bb/100; better for {better} of {}",
        sum / Personality::ALL.len() as f64,
        Personality::ALL.len()
    );
}
