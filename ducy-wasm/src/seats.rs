//! A club table's seats (ducy-play's `club_seats`, ducy-web #456): what the
//! club makes of each result from a table's runner.
//!
//! `clubSeats(clubJson, seatingJson, table, op, args, now)` takes the club
//! (ducy-web's object, as JSON), the table's [`Seating`] and an op, and
//! returns `{ok: true, value, seating, club}`: `value` is the step for the
//! caller to carry out ({sends, events, chipsChanged, onPlay, onHand, save,
//! tablesChanged, armHolds, value}). Or `{ok: false, message}`, with nothing
//! changed.

use ducy_play::club_host::ClubState;
use ducy_play::club_seats::Seating;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

use crate::util::{err, to_js};

/// Runs `op` with `args` (JSON) for table `table` on the club and its seating.
#[wasm_bindgen(js_name = clubSeats)]
pub fn club_seats(
    club: &str,
    seating: &str,
    table: &str,
    op: &str,
    args: &str,
    now: f64,
) -> Result<JsValue, JsError> {
    let mut c: ClubState = serde_json::from_str(club).map_err(err)?;
    let mut s: Seating = serde_json::from_str(seating).map_err(err)?;
    let args: Value = serde_json::from_str(args).map_err(err)?;
    to_js(&match c.seats(&mut s, table, op, &args, now as u64) {
        Ok(out) => json!({"ok": true, "value": out, "seating": s, "club": c}),
        Err(e) => json!({"ok": false, "message": e.0}),
    })
}
