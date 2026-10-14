//! How much luck comparing variants on the same hands removes (#216): two
//! Regulars, then two close versions of one, against the same five
//! opponents, six-handed duplicate: their win rates and the paired difference.
//!
//!     cargo run --release -p ducy-play --example variants -- --deals 2000

use std::time::Instant;

use ducy_play::{
    Bot, MatchConfig, TableRules,
    personality::{Personality, PersonalityBot},
    run_variants,
};

fn main() {
    let mut deals = 2000usize;
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--deals" => deals = it.next().expect("--deals N").parse().expect("--deals"),
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
    let config = MatchConfig::new(TableRules::no_limit_holdem(1, 2), deals, 1).duplicate();
    // Two different characters, then two close versions of one (as training
    // compares checkpoints): pairing helps most when the variants are close.
    let math = Personality::TheMathematician;
    let mut bolder = math.style();
    bolder.aggression = (bolder.aggression + 0.1).min(1.0);
    bolder.bluff += 0.05;
    type Make<'a> = &'a dyn Fn(u64) -> Box<dyn Bot>;
    let m = |s| Box::new(math.bot(Some(s))) as Box<dyn Bot>;
    let tom = |s| Box::new(Personality::TomCollins.bot(Some(s))) as Box<dyn Bot>;
    let bold = move |s| Box::new(PersonalityBot::new(math.name(), bolder, Some(s))) as Box<dyn Bot>;
    let pairs: Vec<(&str, &str, [Make; 2])> = vec![
        ("The Mathematician", "Tom Collins", [&m, &tom]),
        ("The Mathematician", "a bolder Mathematician", [&m, &bold]),
    ];
    for (na, nb, variants) in pairs {
        let t = Instant::now();
        let r = run_variants(&config, &variants, &opponents, 6).expect("match");
        let (a, b, d) = (r.win_rate(0), r.win_rate(1), r.compare(1, 0));
        let unpaired = (a.ci95.powi(2) + b.ci95.powi(2)).sqrt();
        println!(
            "\n{na} against {nb}: {} hands each, {:.0}s",
            r.hands,
            t.elapsed().as_secs_f64()
        );
        println!("  {na}: {:+.1} ± {:.1} bb/100", a.bb_per_100, a.ci95);
        println!("  {nb}: {:+.1} ± {:.1} bb/100", b.bb_per_100, b.ci95);
        println!(
            "  difference, paired: {:+.1} ± {:.1} (separate matches: ± {:.1}; {:.1}x fewer hands for the same precision)",
            d.bb_per_100,
            d.ci95,
            unpaired,
            (unpaired / d.ci95).powi(2)
        );
    }
}
