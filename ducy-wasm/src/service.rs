//! A club's services (ducy-play's `club_service`, ducy-web #456).
//!
//! `clubService(clubJson, op, args, now)` runs `op` on the club and returns
//! `{ok: true, value, club}`, or `{ok: false, status, message}` with
//! nothing changed (`status`: the HTTP status to answer a request with).
//! Ops: "add" {id, alg, key, o}, "update" {id, o}, "revoke" {id}, and
//! "request" {id, body, member}: a checked request carried out, its value
//! `{value, member?, members?}`.

use ducy_play::club_host::ClubState;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

use crate::play::{err, to_js};

/// Runs service op `op` with `args` (JSON) on the club.
#[wasm_bindgen(js_name = clubService)]
pub fn club_service(club: &str, op: &str, args: &str, now: f64) -> Result<JsValue, JsError> {
    let mut c: ClubState = serde_json::from_str(club).map_err(err)?;
    let args: Value = serde_json::from_str(args).map_err(err)?;
    let now = now as u64;
    let s = |k: &str| {
        args.get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let o = args.get("o").cloned().unwrap_or_else(|| json!({}));
    let r: Result<Value, (u16, String)> = match op {
        "add" => c
            .add_service(&s("id"), &s("alg"), &s("key"), &o, now)
            .map_err(|e| (400, e.0)),
        "update" => c.update_service(&s("id"), &o, now).map_err(|e| (400, e.0)),
        "revoke" => c.revoke_service(&s("id"), now).map_err(|e| (400, e.0)),
        "request" => {
            let member = args.get("member").and_then(Value::as_str);
            let body = args.get("body").cloned().unwrap_or(Value::Null);
            c.service_request(&s("id"), &body, member, now)
                .map(|a| serde_json::to_value(a).unwrap_or(Value::Null))
                .map_err(|e| (e.status, e.reason))
        }
        _ => Err((400, format!("no service op {op}"))),
    };
    to_js(&match r {
        Ok(value) => json!({"ok": true, "value": value, "club": c}),
        Err((status, message)) => json!({"ok": false, "status": status, "message": message}),
    })
}
