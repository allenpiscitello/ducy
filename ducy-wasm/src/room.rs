//! An always-on club's room (ducy-play's `club_room`, ducy-web #463): the
//! relay's room asks it what to make of requests and what the super admin
//! sees, and keeps its play clock.
//!
//! `clubRoom(op, now)` takes an op (`{op, …}`) and returns `{ok: true,
//! value}` (and `clock`, for the play clock's ops) or `{ok: false,
//! message}`.

use ducy_club::registry::Record;
use ducy_club::room::{
    PlayClock, admin_full, admin_people, admin_person, check_create, check_import, code_info,
    hand_no_cards, plan_of,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use wasm_bindgen::prelude::*;

use crate::util::{err, to_js};

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Op {
    CheckCreate {
        env: Value,
        code: String,
    },
    CheckImport {
        env: Value,
        code: String,
    },
    CodeInfo {
        club: Value,
        #[serde(default)]
        invite: String,
    },
    PlanOf {
        record: Option<Record>,
    },
    HandNoCards {
        hand: Value,
    },
    AdminPeople {
        club: Value,
        #[serde(default)]
        seen: Map<String, Value>,
    },
    AdminPerson {
        club: Value,
        #[serde(default)]
        seen: Map<String, Value>,
        id: String,
    },
    AdminFull {
        club: Value,
        #[serde(default)]
        seen: Map<String, Value>,
        #[serde(default)]
        reveal: bool,
        #[serde(default)]
        hands: Vec<Value>,
    },
    Playing {
        #[serde(default)]
        clock: PlayClock,
    },
    Check {
        #[serde(default)]
        clock: PlayClock,
    },
    Report {
        #[serde(default)]
        clock: PlayClock,
    },
}

/// Runs `op` at time `now`.
#[wasm_bindgen(js_name = clubRoom)]
pub fn club_room(op: JsValue, now: f64) -> Result<JsValue, JsError> {
    let op: Op = serde_wasm_bindgen::from_value(op).map_err(err)?;
    let now = now as u64;
    let clocked = |mut c: PlayClock,
                   f: fn(&mut PlayClock, u64) -> Option<ducy_club::room::Usage>| {
        let u = f(&mut c, now);
        json!({"ok": true, "value": u, "clock": c})
    };
    let out = match op {
        Op::CheckCreate { env, code } => match check_create(&env, &code, now) {
            Ok(name) => json!({"ok": true, "value": name}),
            Err(message) => json!({"ok": false, "message": message}),
        },
        Op::CheckImport { env, code } => match check_import(&env, &code, now) {
            Ok(name) => json!({"ok": true, "value": name}),
            Err(message) => json!({"ok": false, "message": message}),
        },
        Op::CodeInfo { club, invite } => json!({"ok": true, "value": code_info(&club, &invite)}),
        Op::PlanOf { record } => json!({"ok": true, "value": plan_of(record.as_ref())}),
        Op::HandNoCards { hand } => json!({"ok": true, "value": hand_no_cards(&hand)}),
        Op::AdminPeople { club, seen } => json!({"ok": true, "value": admin_people(&club, &seen)}),
        Op::AdminPerson { club, seen, id } => {
            json!({"ok": true, "value": admin_person(&club, &seen, &id)})
        }
        Op::AdminFull {
            club,
            seen,
            reveal,
            hands,
        } => json!({"ok": true, "value": admin_full(&club, &seen, reveal, &hands)}),
        Op::Playing { clock } => clocked(clock, PlayClock::playing),
        Op::Check { clock } => clocked(clock, PlayClock::check),
        Op::Report { clock } => clocked(clock, |c, _| c.report()),
    };
    to_js(&out)
}
