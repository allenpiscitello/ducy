//! A club's tournaments (ducy-play's `club_tourneys`, ducy-web #457), for
//! ducy-web's club host: making them, registering, results and payouts.
//!
//! As `clubRules` (rules.rs): each call takes the tournaments' part of the
//! club ({code, members, tourneys, payoutTables}) as JSON and an op, and
//! returns `{ok: true, value, club}`, `club` being {tourneys, payoutTables}
//! after an op that changes them, or `{ok: false, message}`.

use ducy_play::club_rules::RuleError;
use ducy_play::club_tourneys::{self, PayoutTable, Tourney, Tourneys};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use wasm_bindgen::prelude::*;

use crate::play::{err, to_js};

/// An op, as ducy-web sends it.
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Op {
    New {
        s: Value,
        most: Option<u64>,
    },
    CheckRegister {
        tid: String,
        id: String,
    },
    Register {
        tid: String,
        id: String,
    },
    Unregister {
        tid: String,
        id: String,
    },
    CheckLateEntry {
        tid: String,
        id: String,
        level: u64,
    },
    LateEntry {
        tid: String,
        id: String,
        level: u64,
    },
    AtStart {
        tid: String,
    },
    Cancel {
        tid: String,
        #[serde(default)]
        why: String,
    },
    Start {
        tid: String,
    },
    KnockedOut {
        tid: String,
        id: String,
        place: u64,
    },
    Finish {
        tid: String,
        winner: String,
    },
    SetPayout {
        tid: String,
        payout: Value,
    },
    SetPayoutTable {
        tid: String,
        table: Option<String>,
    },
    AddPayoutTable {
        s: Value,
    },
    EditPayoutTable {
        id: String,
        s: Value,
    },
    DeletePayoutTable {
        id: String,
    },
    Body {
        #[serde(default)]
        live: Map<String, Value>,
    },
}

fn value(v: impl serde::Serialize) -> Result<Value, RuleError> {
    serde_json::to_value(v).map_err(|e| RuleError(e.to_string()))
}

/// Runs `op` on the club's tournaments (`club`: JSON of {code, members,
/// tourneys, payoutTables}). Throws only for JSON or an op it can't read.
#[wasm_bindgen(js_name = clubTourneys)]
pub fn club_tourneys(club: &str, op: JsValue, now: f64) -> Result<JsValue, JsError> {
    let op: Op = serde_wasm_bindgen::from_value(op).map_err(err)?;
    let now = now as u64;
    let mut c: Tourneys = serde_json::from_str(club).map_err(err)?;
    let mut changes = true;
    let r: Result<Value, RuleError> = match op {
        Op::New { s, most } => c
            .new_tourney(&s, now, most, &mut rand::make_rng())
            .and_then(value),
        Op::CheckRegister { tid, id } => {
            changes = false;
            c.check_register(&tid, &id, now).and_then(value)
        }
        Op::Register { tid, id } => c.register(&tid, &id, now).and_then(value),
        Op::Unregister { tid, id } => c.unregister(&tid, &id).and_then(value),
        Op::CheckLateEntry { tid, id, level } => {
            changes = false;
            c.check_late_entry(&tid, &id, level).and_then(value)
        }
        Op::LateEntry { tid, id, level } => c.late_entry(&tid, &id, level, now).and_then(value),
        Op::AtStart { tid } => {
            changes = false;
            c.at_start(&tid, now).and_then(value)
        }
        Op::Cancel { tid, why } => c.cancel(&tid, &why, now).and_then(value),
        Op::Start { tid } => c.start(&tid, now).and_then(value),
        Op::KnockedOut { tid, id, place } => c.knocked_out(&tid, &id, place).and_then(value),
        Op::Finish { tid, winner } => c.finish(&tid, &winner, now).and_then(value),
        Op::SetPayout { tid, payout } => c.set_payout(&tid, &payout).and_then(value),
        Op::SetPayoutTable { tid, table } => {
            c.set_payout_table(&tid, table.as_deref()).and_then(value)
        }
        Op::AddPayoutTable { s } => c
            .add_payout_table(&s, &mut rand::make_rng())
            .and_then(value),
        Op::EditPayoutTable { id, s } => c.edit_payout_table(&id, &s).and_then(value),
        Op::DeletePayoutTable { id } => c.delete_payout_table(&id).and_then(value),
        Op::Body { live } => {
            changes = false;
            Ok(c.body(&live, now))
        }
    };
    to_js(&match r {
        Ok(v) if changes => json!({"ok": true, "value": v, "club": c}),
        Ok(v) => json!({"ok": true, "value": v}),
        Err(e) => json!({"ok": false, "message": e.0}),
    })
}

/// A tournament's payouts now: {shares, prizes}, each paid place's share
/// (basis points) and prize (club chips), first place first. `tourney` and
/// the club's payout `tables` as JSON.
#[wasm_bindgen(js_name = tourneyPayouts)]
pub fn tourney_payouts(tourney: &str, tables: &str) -> Result<JsValue, JsError> {
    let t: Tourney = serde_json::from_str(tourney).map_err(err)?;
    let tables: Vec<PayoutTable> = serde_json::from_str(tables).map_err(err)?;
    to_js(&json!({"shares": t.payout_shares(&tables), "prizes": t.prizes(&tables)}))
}

/// A payout table's rows as typed (JSON), checked: `{ok: true, rows}`, the
/// clean rows, or `{ok: false, message}` saying what's wrong.
#[wasm_bindgen(js_name = checkPayoutRows)]
pub fn check_payout_rows(rows: &str) -> Result<JsValue, JsError> {
    let rows: Value = serde_json::from_str(rows).map_err(err)?;
    to_js(&match club_tourneys::check_payout_rows(&rows) {
        Ok(rows) => json!({"ok": true, "rows": rows}),
        Err(e) => json!({"ok": false, "message": e.0}),
    })
}

/// The default shares for `entries` entries, in basis points.
#[wasm_bindgen(js_name = defaultPayout)]
pub fn default_payout(entries: u32) -> Vec<u32> {
    club_tourneys::default_payout(u64::from(entries))
        .into_iter()
        .map(|b| b as u32)
        .collect()
}
