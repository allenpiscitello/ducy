//! Plays the personality bots against each other and prints how each one
//! plays (VPIP, PFR, fold to bet, aggression) and how it does.
//!
//! cargo run --release -p ducy-play --example personalities [hands]

use ducy_play::stats::OpponentModel;
use ducy_play::{Bot, Deal, Hand, MatchConfig, Personality, TableRules, play_hand, run_match};

fn main() {
    let hands: usize = std::env::args()
        .nth(1)
        .map_or(2_000, |s| s.parse().expect("hands"));
    for (label, rules) in [
        ("No-limit Hold'em", TableRules::no_limit_holdem(1, 2)),
        ("Pot-limit Omaha", TableRules::pot_limit_omaha(1, 2)),
    ] {
        println!("{label}");

        println!(
            "  {:18} {:>6} {:>6} {:>12} {:>11} {:>10}",
            "", "VPIP", "PFR", "fold to bet", "aggression", "bb/100"
        );
        // Tables of up to 5 with fixed seats, so per-seat stats are per-bot
        // stats; win rates come from a duplicate match at the same table.
        for table in Personality::ALL.chunks(5) {
            let mut bots: Vec<Box<dyn Bot>> = table
                .iter()
                .enumerate()
                .map(|(i, p)| Box::new(p.bot(Some(i as u64))) as Box<dyn Bot>)
                .collect();
            let n = bots.len();
            let mut model = OpponentModel::new();
            for h in 0..hands {
                let deal = Deal::random(rules.variant, n, Some(h as u64)).unwrap();
                let mut hand = Hand::new(rules, &vec![200; n], h % n, deal).unwrap();
                let mut seated: Vec<&mut dyn Bot> = Vec::new();
                for bot in bots.iter_mut() {
                    seated.push(&mut **bot);
                }
                play_hand(&mut hand, &mut seated).unwrap();
                model.record(hand.events(), n);
            }
            let config = MatchConfig::new(rules, hands / n, 99).duplicate();
            let result = run_match(&config, &mut bots).unwrap();
            for (i, p) in table.iter().enumerate() {
                let s = model.seat(i);
                println!(
                    "  {:18} {:>5.0}% {:>5.0}% {:>11.0}% {:>11.2} {:>+10.1}",
                    p.name(),
                    100.0 * s.vpip_hands as f64 / s.hands as f64,
                    100.0 * s.pfr_hands as f64 / s.hands as f64,
                    100.0 * s.folds_to_bets as f64 / s.faced_bets.max(1) as f64,
                    s.aggressive as f64 / s.calls.max(1) as f64,
                    result.bb_per_100(i),
                );
            }
            println!();
        }
    }
}
