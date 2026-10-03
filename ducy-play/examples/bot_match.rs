//! Runs a duplicate match between the built-in bots, plus any external bot
//! programs given on the command line.
//!
//! cargo run --release -p ducy-play --example bot_match
//! cargo run --release -p ducy-play --example bot_match -- python3 ducy-play/examples/bots/simple_bot.py

use ducy_play::bots::{CallingStation, EquityBot, Raiser, RandomBot};
use ducy_play::{Bot, MatchConfig, ProcessBot, TableRules, run_match};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut names = vec!["calling station", "raiser", "random", "equity"];
    let mut bots: Vec<Box<dyn Bot>> = vec![
        Box::new(CallingStation),
        Box::new(Raiser),
        Box::new(RandomBot::new(Some(1))),
        Box::new(EquityBot::new(200, Some(2))),
    ];
    if let Some((program, rest)) = args.split_first() {
        let rest: Vec<&str> = rest.iter().map(String::as_str).collect();
        let bot = ProcessBot::spawn(program, &rest).expect("start bot program");
        bots.push(Box::new(bot));
        names.push("external");
    }

    for (label, rules) in [
        ("No-limit Hold'em", TableRules::no_limit_holdem(1, 2)),
        ("Pot-limit Omaha", TableRules::pot_limit_omaha(1, 2)),
    ] {
        let config = MatchConfig::new(rules, 200, 7).duplicate();
        let result = run_match(&config, &mut bots).expect("match");
        println!("{label}: {} hands, duplicate", result.hands);
        for (i, name) in names.iter().enumerate() {
            println!(
                "  {name:16} {:>+9.1} bb/100  ({} fallbacks)",
                result.bb_per_100(i),
                result.fallbacks[i]
            );
        }
    }
}
