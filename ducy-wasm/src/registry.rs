//! The relay's registry of always-on clubs (ducy-play's `club_registry`,
//! ducy-web #463): ducy-web's relay keeps its parts in storage and runs each
//! op through here.
//!
//! `clubRegistry(registryJson, relayLimitsJson, op, now)` takes the parts
//! `{clubs, grants, trials}` and an op (`{op, …}`), and returns `{ok: true,
//! value, registry}` (the parts after the op) or `{ok: false, message}` with
//! nothing changed.

use std::collections::BTreeMap;

use ducy_club::registry::{
    FREE_LIMITS, PlanLimits, Registry, RelayLimits, admin_seen, clean_limits, day_of, usage_day,
};
use serde::Deserialize;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

use crate::util::{err, to_js};

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Op {
    Claim {
        owner: String,
        code: String,
        #[serde(default)]
        name: String,
        #[serde(default = "yes")]
        temporary: bool,
    },
    Proved {
        code: String,
        owner: String,
    },
    Expired,
    Has {
        code: String,
    },
    Status {
        code: String,
    },
    Release {
        code: String,
    },
    Used {
        code: String,
        #[serde(default)]
        minutes: f64,
    },
    List,
    Grant {
        id: String,
        #[serde(default)]
        note: String,
        #[serde(default)]
        max_clubs: u64,
        limits: Option<Value>,
    },
    Revoke {
        id: String,
    },
    SetClub {
        code: String,
        change: Value,
    },
    CleanLimits {
        input: Value,
        base: Option<PlanLimits>,
    },
    UsageDay {
        day: String,
        minutes: f64,
    },
    AdminSeen {
        seen: BTreeMap<String, u64>,
        seq: String,
        window_ms: Option<u64>,
    },
    DayOf {
        at: u64,
    },
}

fn yes() -> bool {
    true
}

/// Runs `op` on the registry's parts (`registry`, JSON) with the relay's
/// limits (`limits`, JSON).
#[wasm_bindgen(js_name = clubRegistry)]
pub fn club_registry(
    registry: &str,
    limits: &str,
    op: JsValue,
    now: f64,
) -> Result<JsValue, JsError> {
    let op: Op = serde_wasm_bindgen::from_value(op).map_err(err)?;
    let mut r: Registry = serde_json::from_str(registry).map_err(err)?;
    let limits: RelayLimits = serde_json::from_str(limits).map_err(err)?;
    let now = now as u64;
    let mut changes = true;
    let out: Result<Value, String> = match op {
        Op::Claim {
            owner,
            code,
            name,
            temporary,
        } => r
            .claim(&owner, &code, &name, temporary, &limits, now)
            .map(|x| json!(x)),
        Op::Proved { code, owner } => r.proved(&code, &owner, now).map(|x| json!(x)),
        Op::Expired => {
            changes = false;
            Ok(json!(r.expired(now)))
        }
        Op::Has { code } => {
            changes = false;
            Ok(json!(r.clubs.contains_key(&code)))
        }
        Op::Status { code } => {
            changes = false;
            Ok(r.status(&code, now))
        }
        Op::Release { code } => {
            r.clubs.remove(&code);
            Ok(Value::Null)
        }
        Op::Used { code, minutes } => {
            r.used(&code, minutes, now);
            Ok(Value::Null)
        }
        Op::List => {
            changes = false;
            Ok(r.list(&limits, now))
        }
        Op::Grant {
            id,
            note,
            max_clubs,
            limits: l,
        } => r
            .grant(&id, &note, max_clubs, l.as_ref(), now)
            .map(|g| json!(g)),
        Op::Revoke { id } => {
            r.revoke(&id);
            Ok(Value::Null)
        }
        Op::SetClub { code, change } => r.set_club(&code, &change, now).map(|x| json!(x)),
        Op::CleanLimits { input, base } => {
            changes = false;
            Ok(json!(clean_limits(&input, base.unwrap_or(FREE_LIMITS))))
        }
        Op::UsageDay { day, minutes } => {
            changes = false;
            Ok(usage_day(&day, minutes))
        }
        Op::AdminSeen {
            seen,
            seq,
            window_ms,
        } => {
            changes = false;
            admin_seen(&seen, &seq, now, window_ms.unwrap_or(5 * 60_000)).map(|s| json!(s))
        }
        Op::DayOf { at } => {
            changes = false;
            Ok(json!(day_of(at)))
        }
    };
    to_js(&match out {
        Ok(v) if changes => json!({"ok": true, "value": v, "registry": r}),
        Ok(v) => json!({"ok": true, "value": v}),
        Err(message) => json!({"ok": false, "message": message}),
    })
}
