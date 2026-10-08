//! Blind structures (ducy_play::structure, #156): the same presets, checks
//! and estimates as ducy.cards had in JavaScript (structure-fixture.json,
//! made from its tourney-structure.js), and the estimate against bot-only
//! tournaments played here.
#![cfg(feature = "serde")]

use ducy_play::structure::{
    EstimateOptions, Step, StepInput, check_structure, estimate, preset_structure,
};
use ducy_play::{
    BettingStructure, Bot, Entrant, Level, Personality, Tournament, TournamentConfig, Variant,
};
use rand::{RngExt, SeedableRng, rngs::StdRng};
use serde_json::Value;

fn fixture() -> Value {
    serde_json::from_str(include_str!("structure-fixture.json")).unwrap()
}

#[test]
fn presets_and_estimates_match_the_page() {
    let f = fixture();
    for case in f["presets"].as_array().unwrap() {
        let (preset, stack) = (
            case["preset"].as_str().unwrap(),
            case["stack"].as_u64().unwrap(),
        );
        let want: Vec<Step> = serde_json::from_value(case["steps"].clone()).unwrap();
        let steps = preset_structure(preset, stack).unwrap();
        assert_eq!(steps, want, "{preset} at {stack}");
        for e in case["estimates"].as_array().unwrap() {
            let o = EstimateOptions {
                stack,
                players: e["players"].as_u64().unwrap() as u32,
                table_size: e["tableSize"].as_u64().unwrap() as u32,
                hands_per_hour: e["handsPerHour"].as_f64().unwrap(),
                ..Default::default()
            };
            let got = estimate(&steps, &o);
            let want = (
                e["minutes"].as_u64().unwrap() as u32,
                e["hands"].as_u64().unwrap() as u32,
                e["level"].as_u64().unwrap() as usize,
            );
            assert_eq!(
                (got.minutes, got.hands, got.level),
                want,
                "{preset} at {stack}, {o:?}"
            );
        }
    }
    assert!(preset_structure("regular", 19).is_err());
    assert!(preset_structure("nope", 1000).is_err());
}

#[test]
fn checks_match_the_page() {
    for case in fixture()["checks"].as_array().unwrap() {
        let steps: Vec<StepInput> = serde_json::from_value(case["steps"].clone()).unwrap();
        let want: Vec<String> = serde_json::from_value(case["errors"].clone()).unwrap();
        assert_eq!(check_structure(&steps), want, "{}", case["steps"]);
    }
    // A preset is always playable.
    for p in ducy_play::structure::PRESETS {
        let steps = preset_structure(p.id, 10_000).unwrap();
        let input: Vec<StepInput> = steps.iter().map(StepInput::from).collect();
        assert!(check_structure(&input).is_empty(), "{}", p.id);
    }
}

/// Minutes to a winner in one bot-only tournament under `steps`: every
/// table plays `hands_per_hour` hands an hour at once, the level clock runs
/// on that time, and breaks add their minutes (as the tourney_time example).
fn bot_tournament(
    steps: &[Step],
    stack: u64,
    players: usize,
    table_size: usize,
    hands_per_hour: f64,
    seed: u64,
) -> f64 {
    let mut levels = Vec::new();
    let (mut starts, mut breaks, mut at) = (Vec::new(), Vec::new(), 0.0);
    for s in steps {
        match *s {
            Step::Level {
                sb,
                bb,
                ante,
                minutes,
            } => {
                levels.push(Level::new(sb, bb, ante));
                starts.push(at);
                at += minutes as f64;
            }
            Step::Break { minutes, .. } => breaks.push((at, minutes as f64)),
        }
    }
    let mut rng = StdRng::seed_from_u64(seed);
    let field = (0..players)
        .map(|i| {
            let p = Personality::ALL[rng.random_range(0..Personality::ALL.len())];
            let bot: Box<dyn Bot> = Box::new(p.bot(Some(seed * 100 + i as u64)));
            Entrant::bot(format!("b{i}"), p.name(), bot)
        })
        .collect();
    let config = TournamentConfig {
        variant: Variant::Holdem,
        structure: BettingStructure::NoLimit,
        table_size,
        starting_stack: stack,
        levels: levels.clone(),
        paid: (players / 6).max(1),
        seed,
    };
    let mut t = Tournament::new(config, field).unwrap();
    let mut played = 0.0;
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
        played += 60.0 / hands_per_hour;
    }
    let breaks: f64 = breaks
        .iter()
        .filter(|&&(b, _)| b > 0.0 && b <= played)
        .map(|&(_, m)| m)
        .sum();
    played + breaks
}

/// The estimate against bot-only tournaments (#78's criterion: within about
/// ±25% of their average). Seeded, so it's the same every run.
#[test]
fn the_estimate_is_within_a_quarter_of_bot_tournaments() {
    // Small fields keep this quick in a debug build; the 60-case calibration
    // with fields up to 45 is the tourney_time example.
    const RUNS: u64 = 10;
    for (preset, stack, players, table_size, hph) in [
        ("turbo", 1500, 9, 9, 30.0),
        ("regular", 1500, 9, 9, 30.0),
        ("deep", 1500, 6, 6, 40.0),
    ] {
        let steps = preset_structure(preset, stack).unwrap();
        let mean = (0..RUNS)
            .map(|run| bot_tournament(&steps, stack, players, table_size, hph, 1_000 + run))
            .sum::<f64>()
            / RUNS as f64;
        let e = estimate(
            &steps,
            &EstimateOptions {
                stack,
                players: players as u32,
                table_size: table_size as u32,
                hands_per_hour: hph,
                ..Default::default()
            },
        );
        let off = e.minutes as f64 / mean - 1.0;
        println!(
            "{preset} {stack} {players}p: estimate {} vs bots {mean:.0} ({:+.0}%)",
            e.minutes,
            off * 100.0
        );
        assert!(
            off.abs() <= 0.25,
            "{preset} {stack} {players}p: estimate {} vs bots {mean:.0} ({:.0}%)",
            e.minutes,
            off * 100.0
        );
    }
}
