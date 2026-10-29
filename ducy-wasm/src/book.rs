//! A club's book (ducy-play's `club_book`, ducy-web #456): its rules and
//! money together, for chips given and taken back, chip requests, credit
//! limits, agents and the activity log.
//!
//! As `clubRules`: each call takes the club's part ({code, hostId, members,
//! money, chips, requestLog, log}) as JSON and an op, and returns `{ok:
//! true, value, club}`, `club` being {members, money, chips, requestLog,
//! log} after the op, or `{ok: false, message}` with nothing changed.

use ducy_play::club_book::{self, Book};
use ducy_play::club_rules::RuleError;
use serde::Deserialize;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

use crate::play::{err, to_js};

/// An op, as ducy-web sends it.
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Op {
    Issue {
        id: String,
        amount: Option<f64>,
        #[serde(default)]
        note: String,
    },
    Reclaim {
        id: String,
        amount: Option<f64>,
        #[serde(default)]
        note: String,
    },
    BackerGive {
        backer: String,
        id: String,
        amount: Option<f64>,
        #[serde(default)]
        note: String,
    },
    BackerTake {
        backer: String,
        id: String,
        amount: Option<f64>,
        #[serde(default)]
        note: String,
    },
    RequestChips {
        id: String,
        amount: Option<f64>,
        #[serde(default)]
        note: String,
        #[serde(default)]
        table: String,
    },
    CheckGrant {
        id: String,
    },
    CancelRequest {
        id: String,
    },
    AnswerRequest {
        id: String,
        outcome: String,
        #[serde(default)]
        given: f64,
        #[serde(default)]
        note: String,
        #[serde(default)]
        by: String,
        limit: Option<f64>,
    },
    GrantRequest {
        id: String,
        amount: Option<f64>,
    },
    DeclineRequest {
        id: String,
        #[serde(default)]
        note: String,
        #[serde(default)]
        by: String,
    },
    SetCredit {
        id: String,
        amount: Option<f64>,
    },
    RaiseCredit {
        id: String,
        amount: Option<f64>,
    },
    OfferAgent {
        id: String,
    },
    AnswerAgent {
        id: String,
        accept: bool,
    },
    RevokeAgent {
        id: String,
    },
    AssignAgent {
        id: String,
        agent: Option<String>,
    },
    RunAgent {
        by: String,
        cmd: Value,
    },
    LogAction {
        by: String,
        text: String,
        amount: Option<f64>,
    },
}

/// A whole number from JavaScript, or none (refused as an amount).
fn whole(n: Option<f64>) -> Option<i64> {
    n.filter(|f| f.fract() == 0.0 && f.abs() <= 9e15)
        .map(|f| f as i64)
}

fn value(v: impl serde::Serialize) -> Result<Value, RuleError> {
    serde_json::to_value(v).map_err(|e| RuleError(e.to_string()))
}

/// Runs `op` on the club's book. Throws only for JSON or an op it can't read.
#[wasm_bindgen(js_name = clubBook)]
pub fn club_book(club: &str, op: JsValue, now: f64) -> Result<JsValue, JsError> {
    let op: Op = serde_wasm_bindgen::from_value(op).map_err(err)?;
    let now = now as u64;
    let mut b: Book = serde_json::from_str(club).map_err(err)?;
    let mut changes = true;
    let r: Result<Value, RuleError> = match op {
        Op::Issue { id, amount, note } => b.issue(&id, whole(amount), &note, now).and_then(value),
        Op::Reclaim { id, amount, note } => {
            b.reclaim(&id, whole(amount), &note, now).and_then(value)
        }
        Op::BackerGive {
            backer,
            id,
            amount,
            note,
        } => b
            .backer_give(&backer, &id, whole(amount), &note, now)
            .and_then(value),
        Op::BackerTake {
            backer,
            id,
            amount,
            note,
        } => b
            .backer_take(&backer, &id, whole(amount), &note, now)
            .and_then(value),
        Op::RequestChips {
            id,
            amount,
            note,
            table,
        } => b.request_chips(&id, whole(amount), &note, &table, now),
        Op::CheckGrant { id } => {
            changes = false;
            b.check_grant(&id).map(|()| Value::Null)
        }
        Op::CancelRequest { id } => Ok(b.cancel_request(&id).into()),
        Op::AnswerRequest {
            id,
            outcome,
            given,
            note,
            by,
            limit,
        } => b
            .answer_request(
                &id,
                &outcome,
                given.max(0.0) as u64,
                &note,
                &by,
                whole(limit).map(|l| l.max(0) as u64),
                now,
            )
            .and_then(value),
        Op::GrantRequest { id, amount } => b.grant_request(&id, whole(amount), now).and_then(value),
        Op::DeclineRequest { id, note, by } => {
            b.decline_request(&id, &note, &by, now).and_then(value)
        }
        Op::SetCredit { id, amount } => b
            .set_credit(&id, whole(amount).or(Some(-1)), now)
            .map(|()| Value::Null),
        Op::RaiseCredit { id, amount } => b
            .raise_credit(&id, whole(amount).or(Some(-1)), now)
            .map(|()| Value::Null),
        Op::OfferAgent { id } => b.offer_agent(&id),
        Op::AnswerAgent { id, accept } => {
            value(b.answer_agent(&id, accept, now, &mut rand::make_rng()))
        }
        Op::RevokeAgent { id } => b.revoke_agent(&id, now).and_then(value),
        Op::AssignAgent { id, agent } => b
            .assign_to_agent(&id, agent.as_deref(), now)
            .map(|()| Value::Null),
        Op::RunAgent { by, cmd } => b.run_agent(&by, &cmd, now).and_then(value),
        Op::LogAction { by, text, amount } => {
            b.log_action(&by, &text, now, whole(amount));
            Ok(Value::Null)
        }
    };
    to_js(&match r {
        Ok(v) if changes => json!({"ok": true, "value": v, "club": club_book::written(&b)}),
        Ok(v) => json!({"ok": true, "value": v}),
        Err(e) => json!({"ok": false, "message": e.0}),
    })
}
