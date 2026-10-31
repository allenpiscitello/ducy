//! The member's side of a club (ducy-play's `club_member`, ducy-web #456).
//!
//! - `memberFromHost(memJson, envJson)`: whether an envelope is worth
//!   checking (from the club's host, newer, from the pinned key).
//! - `memberTake(memJson, envJson)`: takes in an envelope whose signature
//!   checked out: `{changed, mem}`, the membership as it now is.
//! - `checkOwnLedger(entriesJson, balance)`: whether a member's own entries
//!   add up to their balance.

use ducy_play::club_member::{check_own_ledger, from_host, take_host_message};
use serde_json::{Map, Value, json};
use wasm_bindgen::prelude::*;

use crate::util::{err, to_js};

/// Whether envelope `env` is worth checking for membership `mem`.
#[wasm_bindgen(js_name = memberFromHost)]
pub fn member_from_host(mem: &str, env: &str) -> Result<bool, JsError> {
    let mem: Map<String, Value> = serde_json::from_str(mem).map_err(err)?;
    let env: Value = serde_json::from_str(env).map_err(err)?;
    Ok(from_host(&mem, &env))
}

/// Takes in envelope `env`, its signature checked: `{changed, mem}`.
#[wasm_bindgen(js_name = memberTake)]
pub fn member_take(mem: &str, env: &str) -> Result<JsValue, JsError> {
    let mut mem: Map<String, Value> = serde_json::from_str(mem).map_err(err)?;
    let env: Value = serde_json::from_str(env).map_err(err)?;
    let changed = take_host_message(&mut mem, &env);
    to_js(&json!({"changed": changed, "mem": mem}))
}

/// Whether a member's own entries add up to `balance`.
#[wasm_bindgen(js_name = checkOwnLedger)]
pub fn check_own_ledger_js(entries: &str, balance: f64) -> Result<bool, JsError> {
    let entries: Vec<Value> = serde_json::from_str(entries).map_err(err)?;
    Ok(balance.fract() == 0.0 && check_own_ledger(&entries, balance as i64))
}
