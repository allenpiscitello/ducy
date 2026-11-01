//! The study tools' rules (ducy-training, ducy-web #464): practising
//! Ducybot's charts and the range trainer's skills.
//!
//! `training(op)` takes an op (`{op, …}`) and returns its value. Random
//! draws come in from the page (`r`, numbers in `0..1`, or `seed` for the
//! range trainer's dice).

use std::collections::BTreeMap;

use ducy_training::chart::{self, SceneSizes, Spot};
use ducy_training::holdem_rank::HOLDEM_PERCENTILE;
use ducy_training::practice::{self, Dice, Practice};
use ducy_training::skills::{self, Record};
use serde::Deserialize;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

use crate::util::{err, to_js};

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Op {
    CellHand {
        i: usize,
    },
    SizedVerbs {
        spot: Spot,
    },
    SpotName {
        spot: Spot,
    },
    SpotNameFromId {
        id: String,
    },
    SpotQuestion {
        spot: Spot,
        #[serde(default)]
        hu: bool,
    },
    TrainingActions {
        spot: Spot,
    },
    HandMix {
        spot: Spot,
        i: usize,
    },
    HandMixes {
        spot: Spot,
    },
    HoldemTag {
        cards: Vec<String>,
    },
    FixedAnswers {
        mix: Vec<f64>,
    },
    FixedSeverity {
        mix: Vec<f64>,
        k: usize,
    },
    MixDistance {
        guess: Vec<f64>,
        truth: Vec<f64>,
    },
    Difficulty {
        mix: Vec<f64>,
    },
    MainAction {
        mix: Vec<f64>,
    },
    ObviousCells {
        spot: Spot,
    },
    PickCell {
        spot: Spot,
        #[serde(default = "full_band")]
        band: [f64; 2],
        #[serde(default)]
        probe: bool,
        r: f64,
        avoid: Option<usize>,
    },
    CardsFor {
        i: usize,
        r: [f64; 3],
    },
    SpotFingerprint {
        spot: Spot,
    },
    FixedPenalty {
        severity: f64,
        #[serde(default)]
        probe: bool,
    },
    MixedPenalty {
        distance: f64,
    },
    SetShare {
        mix: Vec<f64>,
        k: usize,
        v: f64,
    },
    TableScene {
        spot: Spot,
        #[serde(default)]
        hu: bool,
        #[serde(default)]
        sizes: SceneSizes,
    },
    GainFor {
        score: f64,
    },
    AnswersToMaster {
        from: f64,
    },
    SkillKey {
        cards: u32,
        pct: u32,
    },
    ParseSkillKey {
        key: String,
    },
    SkillState {
        rec: Option<Record>,
    },
    SkillTable {
        #[serde(default)]
        progress: BTreeMap<String, Value>,
        games: Vec<u32>,
        targets: Vec<u32>,
    },
    NextSkillAfter {
        #[serde(default)]
        progress: BTreeMap<String, Value>,
        games: Vec<u32>,
        targets: Vec<u32>,
        cards: u32,
        pct: u32,
    },
    // ---- The range trainer's practice (`seed`: the dice for one deal) ----
    PracticeRules,
    HandKind {
        cards: Vec<String>,
    },
    TopPercentile {
        cards: Vec<String>,
    },
    TopTags {
        cards: Vec<String>,
    },
    DealTop {
        cards: usize,
        seed: u64,
        #[serde(default)]
        prev: Option<Vec<String>>,
    },
    PracticeNext {
        pr: Practice,
        seed: u64,
    },
    PracticeAnswer {
        pr: Practice,
        yes: bool,
    },
    Diagnosis {
        pr: Practice,
    },
    DrillTags {
        cards: usize,
        pct: f64,
        seed: u64,
    },
    WeakestKinds {
        #[serde(default)]
        rec: Option<Value>,
        tags: Vec<String>,
    },
    TopExamples {
        cards: usize,
        pct: f64,
        seed: u64,
    },
}

fn full_band() -> [f64; 2] {
    [0.0, 1.01]
}

/// The page's progress records, read leniently (anything else is no record).
fn records(p: BTreeMap<String, Value>) -> BTreeMap<String, Record> {
    p.into_iter()
        .filter_map(|(k, v)| serde_json::from_value(v).ok().map(|r| (k, r)))
        .collect()
}

/// Runs a study op.
#[wasm_bindgen]
pub fn training(op: JsValue) -> Result<JsValue, JsError> {
    let op: Op = serde_wasm_bindgen::from_value(op).map_err(err)?;
    let score = |s: f64| s.clamp(0.0, 1000.0) as u32;
    let v = match op {
        Op::CellHand { i } => json!(chart::cell_hand(i)),
        Op::SizedVerbs { spot } => json!(chart::sized_verbs(&spot)),
        Op::SpotName { spot } => json!(chart::spot_name(&spot)),
        Op::SpotNameFromId { id } => json!(chart::spot_name_from_id(&id)),
        Op::SpotQuestion { spot, hu } => json!(chart::spot_question(&spot, hu)),
        Op::TrainingActions { spot } => json!(chart::training_actions(&spot)),
        Op::HandMix { spot, i } => {
            json!(chart::hand_mix(&spot, i, &chart::training_actions(&spot)))
        }
        Op::HoldemTag { cards } => {
            let c: Vec<&str> = cards.iter().map(String::as_str).collect();
            json!(ducy_training::holdem::holdem_tag(&c))
        }
        // Every cell's mix at once, for drawing the grid.
        Op::HandMixes { spot } => {
            let ta = chart::training_actions(&spot);
            let all: Vec<Vec<f64>> = (0..spot.cells.len())
                .map(|i| chart::hand_mix(&spot, i, &ta))
                .collect();
            json!(all)
        }
        Op::FixedAnswers { mix } => json!(chart::fixed_answers(&mix)),
        Op::FixedSeverity { mix, k } => json!(chart::fixed_severity(&mix, k)),
        Op::MixDistance { guess, truth } => json!(chart::mix_distance(&guess, &truth)),
        Op::Difficulty { mix } => json!(chart::difficulty(&mix)),
        Op::MainAction { mix } => json!(chart::main_action(&mix)),
        Op::ObviousCells { spot } => {
            json!(chart::obvious_cells(&spot, &chart::training_actions(&spot)))
        }
        Op::PickCell {
            spot,
            band,
            probe,
            r,
            avoid,
        } => json!(chart::pick_cell(&spot, band, probe, r, avoid)),
        Op::CardsFor { i, r } => json!(chart::cards_for(i, r)),
        Op::SpotFingerprint { spot } => json!(chart::spot_fingerprint(&spot)),
        Op::FixedPenalty { severity, probe } => json!(chart::fixed_penalty(severity, probe)),
        Op::MixedPenalty { distance } => json!(chart::mixed_penalty(distance)),
        Op::SetShare { mix, k, v } => json!(chart::set_share(&mix, k, v)),
        Op::TableScene { spot, hu, sizes } => json!(chart::table_scene(&spot, hu, &sizes)),
        Op::GainFor { score: s } => json!(skills::gain_for(score(s))),
        Op::AnswersToMaster { from } => json!(skills::answers_to_master(score(from))),
        Op::SkillKey { cards, pct } => json!(skills::skill_key(cards, pct)),
        Op::ParseSkillKey { key } => json!(skills::parse_skill_key(&key)),
        Op::SkillState { rec } => json!(skills::skill_state(rec.as_ref())),
        Op::SkillTable {
            progress,
            games,
            targets,
        } => {
            json!(skills::skill_table(&records(progress), &games, &targets))
        }
        Op::NextSkillAfter {
            progress,
            games,
            targets,
            cards,
            pct,
        } => {
            json!(skills::next_skill_after(
                &records(progress),
                &games,
                &targets,
                cards,
                pct
            ))
        }
        Op::PracticeRules => {
            let pairs = |l: &[(&str, &str)]| {
                l.iter()
                    .map(|(k, v)| (k.to_string(), json!(v)))
                    .collect::<serde_json::Map<_, _>>()
            };
            json!({
                "levels": practice::LEVELS, "closeCall": practice::CLOSE_CALL, "mastery": skills::MASTERY,
                "holdemPercentile": HOLDEM_PERCENTILE.iter().map(|(c, p)| (c.to_string(), json!(p))).collect::<serde_json::Map<_, _>>(),
                "holdemLabels": pairs(&ducy_training::holdem::HOLDEM_TAG_LABELS),
            })
        }
        Op::HandKind { cards } => json!(practice::hand_kind(&cards)),
        Op::TopPercentile { cards } => json!(practice::top_percentile(&cards)),
        Op::TopTags { cards } => json!(practice::top_tags(&cards)),
        Op::DealTop { cards, seed, prev } => {
            let mut d = Dice::new(seed);
            let h = practice::deal_not_again(
                || practice::Dealt {
                    cards: practice::deal_top(cards, &mut d),
                    ..Default::default()
                },
                prev.as_deref(),
            );
            json!(h.cards)
        }
        Op::PracticeNext { pr, seed } => json!(practice::practice_next(&pr, &mut Dice::new(seed))),
        Op::PracticeAnswer { pr, yes } => json!(practice::practice_answer(&pr, yes)),
        Op::Diagnosis { pr } => json!(practice::diagnose(&pr).map(|d| {
            let text = practice::diagnosis_text(&d, pr.cards, pr.pct);
            json!({"kind": d.kind, "d": d, "text": text})
        })),
        Op::DrillTags { cards, pct, seed } => {
            json!(practice::drill_tags(cards, pct, &mut Dice::new(seed)))
        }
        Op::WeakestKinds { rec, tags } => json!(practice::weakest_kinds(rec.as_ref(), &tags)),
        Op::TopExamples { cards, pct, seed } => {
            let (inside, outside) = practice::top_examples(cards, pct, &mut Dice::new(seed));
            json!({"inside": inside, "outside": outside})
        }
    };
    to_js(&v)
}
