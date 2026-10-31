//! A club's book (ducy-play's `club_book`, ducy-web #456): its rules and
//! money together, for chips given and taken back, chip requests, credit
//! limits, agents and the activity log.
//!
//! As `clubRules`: each call takes the club's part ({code, hostId, members,
//! money, chips, requestLog, log}) as JSON and an op, and returns `{ok:
//! true, value, club}`, `club` being {members, money, chips, requestLog,
//! log} after the op, or `{ok: false, message}` with nothing changed.

use ducy_play::club_book::{self, Book, Limits, TableRefusal};
use ducy_play::club_money::Entry;
use ducy_play::club_rules::RuleError;
use serde::Deserialize;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

use crate::util::{err, to_js};

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
    StartMoney,
    BuyIn {
        id: String,
        table: String,
        amount: Option<f64>,
        limits: Limits,
    },
    TopUp {
        id: String,
        amount: Option<f64>,
        max: Option<f64>,
    },
    RefundTopUp {
        id: String,
        amount: Option<f64>,
    },
    CashOut {
        id: String,
    },
    ChipsForTable {
        id: String,
        table: String,
        amount: Option<f64>,
        limits: Limits,
    },
    LeftTable {
        id: String,
        chips: f64,
    },
    CashOutSeated {
        #[serde(default)]
        keep: Vec<String>,
        only: Option<String>,
    },
    TourneyBuyIn {
        id: String,
        tourney: String,
        amount: Option<f64>,
    },
    Pay {
        id: String,
        what: String,
        amount: Option<f64>,
        about: String,
    },
    VoidPrize {
        id: String,
        amount: Option<f64>,
        promo: String,
    },
    RatholeMin {
        id: String,
        table: String,
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

/// Runs `op` on the club's book. Throws only for JSON or an op it can't
/// read. A table move refused for want of chips says so (`short: true`).
#[wasm_bindgen(js_name = clubBook)]
pub fn club_book(club: &str, op: JsValue, now: f64) -> Result<JsValue, JsError> {
    let op: Op = serde_wasm_bindgen::from_value(op).map_err(err)?;
    let now = now as u64;
    let mut b: Book = serde_json::from_str(club).map_err(err)?;
    let mut changes = true;
    let table =
        |r: Result<Option<Entry>, TableRefusal>| r.and_then(|e| value(e).map_err(Into::into));
    let r: Result<Value, TableRefusal> = match op {
        Op::BuyIn {
            id,
            table: t,
            amount,
            limits,
        } => table(b.buy_in(&id, &t, whole(amount), limits, now)),
        Op::TopUp { id, amount, max } => table(b.top_up(
            &id,
            whole(amount),
            whole(max).map_or(u64::MAX, |m| m.max(0) as u64),
            now,
        )),
        Op::RefundTopUp { id, amount } => table(b.refund_top_up(&id, whole(amount), now)),
        Op::CashOut { id } => table(b.cash_out(&id, now)),
        Op::ChipsForTable {
            id,
            table: t,
            amount,
            limits,
        } => table(b.chips_for_table(&id, &t, whole(amount), limits, now)),
        Op::LeftTable { id, chips } => table(b.left_table(&id, chips.max(0.0) as u64, now)),
        Op::CashOutSeated { keep, only } => b
            .cash_out_seated(&keep, only.as_deref(), now)
            .and_then(|e| value(e).map_err(Into::into)),
        Op::TourneyBuyIn {
            id,
            tourney,
            amount,
        } => table(b.tourney_buy_in(&id, &tourney, whole(amount), now)),
        Op::Pay {
            id,
            what,
            amount,
            about,
        } => table(b.pay(&id, &what, whole(amount), &about, now)),
        Op::VoidPrize { id, amount, promo } => table(b.void_prize(&id, whole(amount), &promo, now)),
        other => rule_op(&mut b, other, now, &mut changes).map_err(Into::into),
    };
    to_js(&match r {
        Ok(v) if changes => json!({"ok": true, "value": v, "club": club_book::written(&b)}),
        Ok(v) => json!({"ok": true, "value": v}),
        Err(e) => json!({"ok": false, "message": e.message, "short": e.short}),
    })
}

/// The book's other ops: its rules, which refuse in words alone.
fn rule_op(b: &mut Book, op: Op, now: u64, changes: &mut bool) -> Result<Value, RuleError> {
    match op {
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
            *changes = false;
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
        Op::StartMoney => b.start_money(now).map(|()| Value::Null),
        Op::LogAction { by, text, amount } => {
            b.log_action(&by, &text, now, whole(amount));
            Ok(Value::Null)
        }
        Op::RatholeMin { id, table } => {
            *changes = false;
            Ok(b.rathole_min(&id, &table, now).into())
        }
        _ => unreachable!("table moves are handled in club_book"),
    }
}
