//! How long bot-only tournaments take under a blind structure: for checking
//! the duration estimate on the tournament setup page (DUCY-CARDS/ducy-web#78).
//!
//! cargo run --release -p ducy-play --example tourney_time -- structure.json
//!
//! structure.json:
//!
//! ```json
//! {"stack": 10000, "hands_per_hour": 30, "table_size": 9,
//!  "players": [6, 9, 18], "runs": 20, "seed": 1,
//!  "levels": [{"sb": 25, "bb": 50, "ante": 0, "minutes": 15}, {"break": 10}, ...]}
//! ```
//!
//! Every table plays `hands_per_hour` hands an hour, all at once, and the
//! level clock runs on that time; breaks add their minutes without hands.
//! The field is the personality bots, picked at random. Prints one JSON
//! line per field size with the minutes each run took to find a winner.

use ducy_play::{
    BettingStructure, Bot, Entrant, Level, Personality, Tournament, TournamentConfig, Variant,
};
use rand::{RngExt, SeedableRng, rngs::StdRng};
use serde_json::Value;

/// One entry in the structure: a level, or a break.
enum Step {
    Level(Level, f64),
    Break(f64),
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: tourney_time structure.json");
    let spec: Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("read")).expect("json");
    let int = |k: &str| spec[k].as_u64().unwrap_or_else(|| panic!("{k}"));
    let stack = int("stack");
    let hands_per_hour = spec["hands_per_hour"].as_f64().expect("hands_per_hour");
    let table_size = int("table_size") as usize;
    let runs = int("runs");
    let seed = spec["seed"].as_u64().unwrap_or(1);
    let steps: Vec<Step> = spec["levels"]
        .as_array()
        .expect("levels")
        .iter()
        .map(|l| match l["break"].as_f64() {
            Some(m) => Step::Break(m),
            None => Step::Level(
                Level::new(
                    l["sb"].as_u64().unwrap(),
                    l["bb"].as_u64().unwrap(),
                    l["ante"].as_u64().unwrap_or(0),
                ),
                l["minutes"].as_f64().unwrap(),
            ),
        })
        .collect();
    let levels: Vec<Level> = steps
        .iter()
        .filter_map(|s| match s {
            Step::Level(l, _) => Some(*l),
            Step::Break(_) => None,
        })
        .collect();
    // When each level starts, and when each break comes, in play time (minutes).
    let (mut starts, mut break_at, mut at) = (Vec::new(), Vec::new(), 0.0);
    for s in &steps {
        match *s {
            Step::Level(_, m) => {
                starts.push(at);
                at += m;
            }
            Step::Break(m) => break_at.push((at, m)),
        }
    }
    let minutes_per_hand = 60.0 / hands_per_hour;

    for n in spec["players"]
        .as_array()
        .expect("players")
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
    {
        let mut times = Vec::new();
        let mut hands = Vec::new();
        for run in 0..runs {
            let run_seed = seed * 1_000_003 + n as u64 * 1_009 + run;
            let mut rng = StdRng::seed_from_u64(run_seed);
            let field = (0..n)
                .map(|i| {
                    let p = Personality::ALL[rng.random_range(0..Personality::ALL.len())];
                    let bot: Box<dyn Bot> = Box::new(p.bot(Some(run_seed * 100 + i as u64)));
                    Entrant::bot(format!("b{i}"), p.name(), bot)
                })
                .collect();
            let config = TournamentConfig {
                variant: Variant::Holdem,
                structure: BettingStructure::NoLimit,
                table_size,
                starting_stack: stack,
                levels: levels.clone(),
                paid: (n / 6).max(1),
                seed: run_seed,
            };
            let mut t = Tournament::new(config, field).expect("tournament");
            let (mut played, mut rounds) = (0.0, 0u64);
            while !t.is_over() {
                let level = starts
                    .iter()
                    .filter(|&&s| s <= played)
                    .count()
                    .saturating_sub(1);
                t.set_level(level.min(levels.len() - 1));
                let mut dealt = false;
                for id in t.table_ids() {
                    if !t.can_deal(id) {
                        continue;
                    }
                    t.new_hand(id).unwrap();
                    let table = t.table_mut(id).unwrap();
                    while table.advance().unwrap() {}
                    t.finish_hand(id).unwrap();
                    dealt = true;
                }
                assert!(dealt, "no table could deal");
                played += minutes_per_hand;
                rounds += 1;
            }
            let breaks: f64 = break_at
                .iter()
                .filter(|&&(b, _)| b > 0.0 && b <= played)
                .map(|&(_, m)| m)
                .sum();
            times.push(played + breaks);
            hands.push(rounds);
        }
        let mut sorted = times.clone();
        sorted.sort_by(f64::total_cmp);
        let mean = times.iter().sum::<f64>() / times.len() as f64;
        println!(
            "{{\"players\": {n}, \"mean\": {mean:.1}, \"median\": {:.1}, \"minutes\": [{}], \"rounds\": {hands:?}}}",
            sorted[sorted.len() / 2],
            times
                .iter()
                .map(|m| format!("{m:.1}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
}
