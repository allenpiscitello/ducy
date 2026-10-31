//! The club manager's Overview and notices (ducy-play's `club_overview`,
//! ducy-web #398, #456).
//!
//! `clubOverview(clubJson, op, args, now)` reads the club (ducy-web's
//! object, as JSON) and returns `{ok: true, value}` (see
//! `ClubState::overview` for the ops), or `{ok: false, message}`.
//! `dealClock(clockJson, on, now)` is a table's dealing clock after it is
//! (or isn't) dealing at `now`.

use ducy_play::club_host::ClubState;
use ducy_play::club_overview::deal_clock;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

use crate::play::{err, to_js};

/// Runs Overview op `op` with `args` (JSON) on the club.
#[wasm_bindgen(js_name = clubOverview)]
pub fn club_overview(club: &str, op: &str, args: &str, now: f64) -> Result<JsValue, JsError> {
    let c: ClubState = serde_json::from_str(club).map_err(err)?;
    let args: Value = serde_json::from_str(args).map_err(err)?;
    to_js(&match c.overview(op, &args, now as u64) {
        Ok(value) => json!({"ok": true, "value": value}),
        Err(e) => json!({"ok": false, "message": e.0}),
    })
}

/// What a member may buy in for at table `t` (JSON, `{id, min, max}`) by
/// the rathole rule, from their own `entries` (JSON): `{min, max, rathole}`.
#[wasm_bindgen(js_name = buyInRange)]
pub fn buy_in_range_js(
    t: &str,
    entries: &str,
    rathole_hours: f64,
    now: f64,
) -> Result<JsValue, JsError> {
    let t: Value = serde_json::from_str(t).map_err(err)?;
    let entries: Vec<Value> = serde_json::from_str(entries).map_err(err)?;
    to_js(&ducy_play::club_book::buy_in_range(
        &t,
        &entries,
        rathole_hours,
        now as u64,
    ))
}

/// A table's dealing clock (`clock`, JSON or null) after it is (`on`) or
/// isn't dealing at `now`.
#[wasm_bindgen(js_name = dealClock)]
pub fn deal_clock_js(clock: &str, on: bool, now: f64) -> Result<JsValue, JsError> {
    let clock: Value = serde_json::from_str(clock).map_err(err)?;
    to_js(&deal_clock(&clock, on, now as u64))
}
