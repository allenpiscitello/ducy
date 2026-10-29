//! A club's host (ducy-play's `club_host`, ducy-web #456): the manager's
//! commands over the whole club.
//!
//! `clubHost(clubJson, op, args, now)` takes the club (ducy-web's object, as
//! JSON) and a command, and returns `{ok: true, value, club}`: `value` is
//! what the command did ({value, replies, accounts, tell, tablesTo, helpTo,
//! armPromos}) and `club` the club after it. Or `{ok: false, message}`, with
//! nothing changed.

use ducy_play::club_host::ClubState;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

use crate::play::{err, to_js};

/// Runs command `op` with `args` (JSON) on the club (JSON).
#[wasm_bindgen(js_name = clubHost)]
pub fn club_host(club: &str, op: &str, args: &str, now: f64) -> Result<JsValue, JsError> {
    let mut c: ClubState = serde_json::from_str(club).map_err(err)?;
    let args: Value = serde_json::from_str(args).map_err(err)?;
    to_js(&match c.run(op, &args, now as u64, &mut rand::make_rng()) {
        Ok(out) => json!({"ok": true, "value": out, "club": c}),
        Err(e) => json!({"ok": false, "message": e.0}),
    })
}
