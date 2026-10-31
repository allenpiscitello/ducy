//! A club's running tournaments (ducy-play's `club_tourney_host`, ducy-web
//! #456): what the club makes of each one's start time and each result
//! from its runner.
//!
//! `clubTourneyHost(clubJson, tid, op, args, now)` takes the club
//! (ducy-web's object, as JSON) and an op, and returns `{ok: true, value,
//! club}`: `value` is the step for the caller to carry out ({sends, events,
//! chipsChanged, changed, liveChanged, live, onPlay, over, counts, value}).
//! Or `{ok: false, message}`, with nothing changed.

use ducy_play::club_host::ClubState;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

use crate::util::{err, to_js};

/// Runs `op` with `args` (JSON) for tournament `tid` on the club.
#[wasm_bindgen(js_name = clubTourneyHost)]
pub fn club_tourney_host(
    club: &str,
    tid: &str,
    op: &str,
    args: &str,
    now: f64,
) -> Result<JsValue, JsError> {
    let mut c: ClubState = serde_json::from_str(club).map_err(err)?;
    let args: Value = serde_json::from_str(args).map_err(err)?;
    to_js(&match c.tourney_host(tid, op, &args, now as u64) {
        Ok(out) => json!({"ok": true, "value": out, "club": c}),
        Err(e) => json!({"ok": false, "message": e.0}),
    })
}
