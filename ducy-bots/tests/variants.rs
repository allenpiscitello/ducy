//! Comparing versions of a bot on the same hands (#216): identical versions
//! come out exactly even, and a paired comparison is much tighter than
//! comparing two separate win rates.

use ducy_bots::{
    MatchConfig,
    personality::{Personality, PersonalityBot},
    run_variants,
};
use ducy_play::{Bot, TableRules};

fn opponents(k: usize, seed: u64) -> Box<dyn Bot> {
    let lineup = [
        Personality::DougPoker,
        Personality::JenSilly,
        Personality::MilkKing,
    ];
    Box::new(lineup[k % lineup.len()].bot(Some(seed)))
}

#[test]
fn identical_versions_come_out_exactly_even() {
    let config = MatchConfig::new(TableRules::no_limit_holdem(1, 2), 40, 3).duplicate();
    let tom = |seed| Box::new(Personality::TomCollins.bot(Some(seed))) as Box<dyn Bot>;
    let r = run_variants(&config, &[&tom, &tom], &opponents, 4).unwrap();
    assert_eq!(r.hands, 40 * 4);
    assert_eq!(r.net[0], r.net[1], "same bot, same seeds, same hands");
    let c = r.compare(1, 0);
    assert_eq!((c.bb_per_100, c.ci95), (0.0, 0.0));
    assert_eq!(r.fallbacks, vec![0, 0]);
}

#[test]
fn paired_comparisons_take_out_most_of_the_luck() {
    let config = MatchConfig::new(TableRules::no_limit_holdem(1, 2), 150, 11).duplicate();
    // Two close versions of one bot, as training compares.
    let math = Personality::TheMathematician;
    let mut bolder = math.style();
    bolder.aggression = (bolder.aggression + 0.1).min(1.0);
    bolder.bluff += 0.05;
    let a = |seed| Box::new(math.bot(Some(seed))) as Box<dyn Bot>;
    let b =
        move |seed| Box::new(PersonalityBot::new(math.name(), bolder, Some(seed))) as Box<dyn Bot>;
    let r = run_variants(&config, &[&a, &b], &opponents, 3).unwrap();
    let (ra, rb) = (r.win_rate(0), r.win_rate(1));
    let paired = r.compare(1, 0);
    // The difference of the two rates is the paired mean, exactly.
    assert!((paired.bb_per_100 - (rb.bb_per_100 - ra.bb_per_100)).abs() < 1e-9);
    // Unpaired, the two intervals add up; paired, the shared cards cancel.
    let unpaired = (ra.ci95.powi(2) + rb.ci95.powi(2)).sqrt();
    assert!(
        paired.ci95 < 0.8 * unpaired,
        "paired ±{:.1} against unpaired ±{:.1}",
        paired.ci95,
        unpaired
    );
}
