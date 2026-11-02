//! Blind structures (ducy_play::structure, #156): the same presets, checks
//! and estimates as ducy.cards had in JavaScript (structure-fixture.json,
//! made from its tourney-structure.js). ducy-bots checks the estimate
//! against bot-only tournaments.
#![cfg(feature = "serde")]

use ducy_play::structure::{
    EstimateOptions, Step, StepInput, check_structure, estimate, preset_structure,
};
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
