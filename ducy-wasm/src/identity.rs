//! Player identities (ducy-identity, ducy-web #460): signed envelopes'
//! checks, canonical JSON and player IDs. Signing stays in the browser's
//! WebCrypto, where the key lives.
//!
//! - `checkSignature(envJson)`: "" when the envelope checks out, else why.
//! - `verifyEnvelope(knownJson, envJson)`: a host's check of a message from
//!   a sender it may know (`{pub, alg, seq}` or null): `{reason, sender}`,
//!   `sender` the record to keep when it's taken.
//! - `canonicalJson(json)`: the bytes a message is signed over.
//! - `playerIdFor(raw)`, `showId(id)`, `parseShownId(text)`.
//! - Backups: `passwordProblem`, `readBackup`, `backupWithClub`,
//!   `proveClubBackup` (the file around the key; WebCrypto locks and
//!   unlocks the key itself).

use ducy_identity::{
    Sender, Verifier, backup, canonical, check_signature, parse_shown_id, player_id, show_id,
};
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

use crate::util::{err, to_js};

/// "" when envelope `env` (JSON) checks out, else why not.
#[wasm_bindgen(js_name = checkSignature)]
pub fn check_signature_js(env: &str) -> String {
    match serde_json::from_str::<Value>(env) {
        Ok(v) => check_signature(&v).err().map_or("", |r| r.as_str()).into(),
        Err(_) => "malformed".into(),
    }
}

/// A host's check of envelope `env` from a sender it may know already
/// (`known`: `{pub, alg, seq}`, or null): `{reason, sender}`.
#[wasm_bindgen(js_name = verifyEnvelope)]
pub fn verify_envelope(known: &str, env: &str) -> Result<JsValue, JsError> {
    let known: Option<Sender> = serde_json::from_str(known).map_err(err)?;
    let Ok(env) = serde_json::from_str::<Value>(env) else {
        return to_js(&json!({"reason": "malformed", "sender": null}));
    };
    let mut v = Verifier::default();
    if let (Some(k), Some(from)) = (known, env.get("from").and_then(Value::as_str)) {
        v.senders.insert(from.into(), k);
    }
    to_js(&match v.verify(&env) {
        Ok(from) => json!({"reason": "", "sender": v.senders.get(&from)}),
        Err(r) => json!({"reason": r.as_str(), "sender": null}),
    })
}

/// The canonical JSON of `json`: the bytes a message is signed over.
#[wasm_bindgen(js_name = canonicalJson)]
pub fn canonical_json(json: &str) -> Result<String, JsError> {
    let v: Value = serde_json::from_str(json).map_err(err)?;
    Ok(canonical(&v))
}

/// The player ID for a raw public key.
#[wasm_bindgen(js_name = playerIdFor)]
pub fn player_id_for(raw: &[u8]) -> String {
    player_id(raw)
}

/// How people see player ID `id`.
#[wasm_bindgen(js_name = showId)]
pub fn show_id_js(id: &str) -> String {
    show_id(id)
}

/// A player ID from what someone wrote, or null.
#[wasm_bindgen(js_name = parseShownId)]
pub fn parse_shown_id_js(text: &str) -> Option<String> {
    parse_shown_id(text)
}

/// Why `password` won't do for a new backup (with the club's `name`, if
/// any), or "".
#[wasm_bindgen(js_name = passwordProblem)]
pub fn password_problem_js(password: &str, name: &str) -> String {
    backup::password_problem(password, name)
}

/// A backup file read (ducy-identity's backup): `{ok: true, value: {v, id,
/// alg, seq, key?, lock?, club, head}}` (`head`, canonical JSON, what a
/// version 2 file binds as associated data), or `{ok: false, message}`.
#[wasm_bindgen(js_name = readBackup)]
pub fn read_backup_js(text: &str) -> Result<JsValue, JsError> {
    to_js(&match backup::read_backup(text) {
        Ok(b) => {
            let mut v = serde_json::to_value(&b).map_err(err)?;
            v["head"] = canonical(&b.head()).into();
            json!({"ok": true, "value": v})
        }
        Err(message) => json!({"ok": false, "message": message}),
    })
}

/// A club's backup: player ID backup `locked` (JSON) with the club added.
#[wasm_bindgen(js_name = backupWithClub)]
pub fn backup_with_club(locked: &str, code: &str, name: &str) -> Result<String, JsError> {
    backup::with_club(locked, code, name).map_err(|e| JsError::new(&e))
}

/// Whether an opened backup (its `club`, JSON or "null", and the player ID
/// `id` its key holds) brings back club `code` for owner `owner`: "" if so,
/// else why not.
#[wasm_bindgen(js_name = proveClubBackup)]
pub fn prove_club_backup(club: &str, id: &str, code: &str, owner: &str) -> String {
    let club: Option<backup::Club> = serde_json::from_str(club).unwrap_or(None);
    backup::prove_club(club.as_ref(), id, code, owner)
        .err()
        .unwrap_or_default()
}
