//! What the club's pages show (ducy-play's `club_views`, ducy-web #459).
//!
//! - `clubView(clubJson, op, args)`: `{ok: true, value}` or `{ok: false,
//!   message}`; ops are `ClubState::view`'s.
//! - `tableSessions(entriesJson)`, `chipLines(entriesJson)` and
//!   `leftSummary(entriesJson, n)`: over entries as JSON, the club's ledger
//!   or a member's own copy.

use ducy_play::club_host::ClubState;
use ducy_play::club_views::{chip_lines, left_summary, table_sessions};
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

use crate::play::{err, to_js};

/// Runs view op `op` with `args` (JSON) on the club.
#[wasm_bindgen(js_name = clubView)]
pub fn club_view(club: &str, op: &str, args: &str) -> Result<JsValue, JsError> {
    let c: ClubState = serde_json::from_str(club).map_err(err)?;
    let args: Value = serde_json::from_str(args).map_err(err)?;
    to_js(&match c.view(op, &args) {
        Ok(value) => json!({"ok": true, "value": value}),
        Err(e) => json!({"ok": false, "message": e.0}),
    })
}

/// The table sessions in `entries`, newest first.
#[wasm_bindgen(js_name = tableSessions)]
pub fn table_sessions_js(entries: &str) -> Result<JsValue, JsError> {
    let entries: Vec<Value> = serde_json::from_str(entries).map_err(err)?;
    to_js(&table_sessions(&entries))
}

/// A member's chip history from their own entries, newest first.
#[wasm_bindgen(js_name = chipLines)]
pub fn chip_lines_js(entries: &str) -> Result<JsValue, JsError> {
    let entries: Vec<Value> = serde_json::from_str(entries).map_err(err)?;
    to_js(&chip_lines(&entries))
}

/// After standing up (entry `n`): the table, the result, the balance after
/// it and what's still owed, or null.
#[wasm_bindgen(js_name = leftSummary)]
pub fn left_summary_js(entries: &str, n: f64) -> Result<JsValue, JsError> {
    let entries: Vec<Value> = serde_json::from_str(entries).map_err(err)?;
    to_js(&left_summary(&entries, n as i64))
}
