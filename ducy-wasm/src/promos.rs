//! A club's promotions (ducy-play's `club_promos`, ducy-web #457), for
//! ducy-web's club host: High Hands and hand bonuses.
//!
//! As `clubRules` (rules.rs): each call takes the promotions' part of the
//! club ({code, members, promos}) as JSON and an op, and returns `{ok: true,
//! value, club}`, `club` being {promos} after an op that changes them, or
//! `{ok: false, message}`. A few ops read only what they're given (`score5`,
//! `bestWithTwo`, `handName`, `periodOf`, `shownHands`, `rules`); pass "{}".

use ducy_club::promos::{self as club_promos, Promo, Promos, Score};
use ducy_club::rules::RuleError;
use serde::Deserialize;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

use crate::util::{err, to_js};

/// An op, as ducy-web sends it.
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Op {
    New {
        s: Value,
    },
    Edit {
        id: String,
        s: Value,
    },
    SetStatus {
        id: String,
        status: String,
    },
    ProcessHand {
        rec: Value,
        #[serde(default)]
        players: Vec<Value>,
    },
    Settle,
    NextSettle,
    Approve {
        id: String,
    },
    Void {
        id: String,
        #[serde(default)]
        why: String,
    },
    Body,
    Admin,
    Score5 {
        cards: Vec<String>,
    },
    BestWithTwo {
        hole: Vec<String>,
        board: Vec<String>,
    },
    HandName {
        cat: usize,
        #[serde(default)]
        groups: Vec<(usize, usize)>,
        #[serde(default)]
        high: i64,
    },
    PeriodOf {
        promo: Promo,
        t: f64,
    },
    ShownHands {
        rec: Value,
        #[serde(default)]
        players: Vec<Value>,
    },
    Rules {
        promo: Promo,
    },
}

fn value(v: impl serde::Serialize) -> Result<Value, RuleError> {
    serde_json::to_value(v).map_err(|e| RuleError(e.to_string()))
}

/// A score as ducy-web has it: {score, cat, groups: [[count, rank]], high}
/// (high -1 for no straight).
fn score_json(s: &Score) -> Value {
    json!({"score": s.score, "cat": s.cat, "groups": s.groups, "high": s.high.map_or(-1, |h| h as i64)})
}

/// Runs `op` on the club's promotions (`club`: JSON of {code, members,
/// promos}). Throws only for JSON or an op it can't read.
#[wasm_bindgen(js_name = clubPromos)]
pub fn club_promos(club: &str, op: JsValue, now: f64) -> Result<JsValue, JsError> {
    let op: Op = serde_wasm_bindgen::from_value(op).map_err(err)?;
    let now = now as u64;
    let mut c: Promos = serde_json::from_str(club).map_err(err)?;
    let mut changes = true;
    let mut rng = rand::make_rng();
    let r: Result<Value, RuleError> = match op {
        Op::New { s } => c.new_promo(&s, now, &mut rng).and_then(value),
        Op::Edit { id, s } => c.edit_promo(&id, &s, now).and_then(value),
        Op::SetStatus { id, status } => c.set_status(&id, &status).and_then(value),
        Op::ProcessHand { rec, players } => value(c.process_hand(&rec, &players, now, &mut rng)),
        Op::Settle => value(c.settle_periods(now, &mut rng)),
        Op::NextSettle => {
            changes = false;
            value(c.next_settle())
        }
        Op::Approve { id } => c.approve_award(&id, now).and_then(value),
        Op::Void { id, why } => c.void_award(&id, now, &why, &mut rng).and_then(value),
        Op::Body => {
            changes = false;
            Ok(c.body(now))
        }
        Op::Admin => {
            changes = false;
            Ok(c.admin(now))
        }
        Op::Score5 { cards } => {
            changes = false;
            Ok(club_promos::score5(&cards).map_or(Value::Null, |s| score_json(&s)))
        }
        Op::BestWithTwo { hole, board } => {
            changes = false;
            Ok(
                club_promos::best_with_two(&hole, &board).map_or(Value::Null, |(s, cards)| {
                    let mut v = score_json(&s);
                    v["cards"] = json!(cards);
                    v
                }),
            )
        }
        Op::HandName { cat, groups, high } => {
            changes = false;
            let s = Score {
                score: 0,
                cat: cat.min(9),
                groups,
                high: usize::try_from(high).ok(),
            };
            Ok(club_promos::hand_name(&s).into())
        }
        Op::PeriodOf { promo, t } => {
            changes = false;
            Ok(club_promos::period_of(&promo, t as i64).map_or(
                Value::Null,
                |p| json!({"key": p.key, "start": p.start, "end": p.end}),
            ))
        }
        Op::ShownHands { rec, players } => {
            changes = false;
            value(club_promos::shown_hands(&rec, &players))
        }
        Op::Rules { promo } => {
            changes = false;
            Ok(club_promos::promo_rules(&promo).into())
        }
    };
    to_js(&match r {
        Ok(v) if changes => json!({"ok": true, "value": v, "club": c}),
        Ok(v) => json!({"ok": true, "value": v}),
        Err(e) => json!({"ok": false, "message": e.0}),
    })
}
