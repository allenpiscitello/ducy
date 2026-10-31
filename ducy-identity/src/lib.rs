//! Player identities for ducy (ducy-web #65, #460): the signed message
//! envelope, canonical JSON, player IDs and how people see them, and the
//! checks a host (or the relay) makes on every message.
//!
//! A player's signing key stays where it was made, in the browser's
//! WebCrypto: everything here works with public keys only, so it runs the
//! same in the page and on the relay.
//!
//! An envelope is `{v, from, alg, pub, seq, body, sig}`:
//! - `from`: the sender's player ID, 8 Base58 digits from the SHA-256 of
//!   their public key;
//! - `alg`: "ed25519", or "p256" (ECDSA P-256 with SHA-256) where a browser
//!   has no Ed25519;
//! - `pub`: the public key, raw, base64url;
//! - `seq`: a number that only goes up;
//! - `sig`: the signature (base64url) over the canonical JSON of the rest.

use std::collections::BTreeMap;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// The characters of a player ID.
pub const B58: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
/// A player ID's length.
pub const ID_LENGTH: usize = 8;
/// The characters of an ID as people see it: capitals and digits with no
/// I, L, O, 0 or 1.
pub const SHOWN: &str = "ABCDEFGHJKMNPQRSTUVWXYZ23456789";
/// An ID as people see it: 10 characters, shown in two groups of five.
pub const SHOWN_LENGTH: usize = 10;

const ID_RANGE: u64 = 58u64.pow(ID_LENGTH as u32);
const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

// ---- encoding ----

/// Bytes as base64url, unpadded.
pub fn b64url(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Base64url (padded or not) as bytes, or none.
pub fn from_b64url(s: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).ok()
}

/// Whether `s` is a player ID.
pub fn is_player_id(s: &str) -> bool {
    s.chars().count() == ID_LENGTH && s.chars().all(|c| B58.contains(c))
}

/// The player ID for a raw public key: its SHA-256 as a number mod 58^8,
/// in exactly 8 Base58 digits ('1' is zero).
pub fn player_id(raw_public_key: &[u8]) -> String {
    let hash = Sha256::digest(raw_public_key);
    let n = hash.iter().fold(0u128, |n, &b| {
        ((n << 8) | u128::from(b)) % u128::from(ID_RANGE)
    }) as u64;
    digits(n, B58, ID_LENGTH)
}

fn digits(mut n: u64, alphabet: &str, len: usize) -> String {
    let a: Vec<char> = alphabet.chars().collect();
    let base = a.len() as u64;
    let mut out = vec![a[0]; len];
    for slot in out.iter_mut().rev() {
        *slot = a[(n % base) as usize];
        n /= base;
    }
    out.into_iter().collect()
}

fn value_of(s: &str, alphabet: &str) -> Option<u64> {
    let base = alphabet.chars().count() as u64;
    s.chars().try_fold(0u64, |n, c| {
        let d = alphabet.chars().position(|x| x == c)? as u64;
        n.checked_mul(base)?.checked_add(d)
    })
}

/// How people see a player ID: the same number in [`SHOWN`]'s characters,
/// "7KQ2M-XR4TA", so it reads aloud and compares the same in any case.
/// Anything that isn't an ID comes back as it is.
pub fn show_id(id: &str) -> String {
    let Some(n) = is_player_id(id).then(|| value_of(id, B58)).flatten() else {
        return id.into();
    };
    let s = digits(n, SHOWN, SHOWN_LENGTH);
    format!("{}-{}", &s[..5], &s[5..])
}

/// A player ID from what someone wrote: the shown form in any case, with or
/// without the dash or spaces, or the 8-character ID itself.
pub fn parse_shown_id(text: &str) -> Option<String> {
    let raw = text.trim();
    if is_player_id(raw) {
        return Some(raw.into());
    }
    let s: String = raw
        .to_uppercase()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    if s.chars().count() != SHOWN_LENGTH {
        return None;
    }
    let n = value_of(&s, SHOWN)?;
    (n < ID_RANGE).then(|| digits(n, B58, ID_LENGTH))
}

// ---- canonical JSON ----

/// JSON with object keys sorted at every level (as JavaScript sorts them,
/// by UTF-16 code units), numbers and strings written as JavaScript's
/// `JSON.stringify` writes them: the bytes a message is signed over.
pub fn canonical(v: &Value) -> String {
    let mut out = String::new();
    write_canonical(v, &mut out);
    out
}

fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&js_number(n)),
        Value::String(s) => js_string(s, out),
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(x, out);
            }
            out.push(']');
        }
        Value::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                js_string(k, out);
                out.push(':');
                write_canonical(&o[k], out);
            }
            out.push('}');
        }
    }
}

/// A number as JavaScript writes it.
fn js_number(n: &serde_json::Number) -> String {
    if let Some(i) = n.as_i64() {
        return i.to_string();
    }
    if let Some(u) = n.as_u64() {
        return u.to_string();
    }
    let f = n.as_f64().unwrap_or(0.0);
    if !f.is_finite() {
        return "null".into();
    }
    ryu_js::Buffer::new().format(f).to_string()
}

/// A string as `JSON.stringify` writes it.
fn js_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

// ---- envelopes ----

/// Why an envelope was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Refusal {
    /// Not an envelope: missing or wrong fields, or a key or signature that
    /// can't be read.
    Malformed,
    /// The ID isn't the one its key makes.
    IdMismatch,
    /// The signature doesn't check out.
    BadSignature,
    /// A known ID with a different key (or two IDs that collide).
    IdInUse,
    /// Not newer than the last message accepted from this sender.
    Replayed,
}

impl Refusal {
    /// The reason as ducy-web says it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Malformed => "malformed",
            Self::IdMismatch => "id does not match key",
            Self::BadSignature => "bad signature",
            Self::IdInUse => "id in use",
            Self::Replayed => "replayed",
        }
    }
}

/// Checks an envelope's signature and that its ID is its key's. It doesn't
/// know other senders or replays: [`Verifier`] does.
pub fn check_signature(env: &Value) -> Result<(), Refusal> {
    let o = env.as_object().ok_or(Refusal::Malformed)?;
    let s = |k: &str| o.get(k).and_then(Value::as_str);
    let alg = s("alg").filter(|a| *a == "ed25519" || *a == "p256");
    let (Some(alg), Some(from), Some(public), Some(sig)) = (alg, s("from"), s("pub"), s("sig"))
    else {
        return Err(Refusal::Malformed);
    };
    // As JavaScript's Number.isSafeInteger.
    let seq_ok = o
        .get("seq")
        .and_then(Value::as_f64)
        .is_some_and(|f| f.fract() == 0.0 && f.abs() <= MAX_SAFE);
    if o.get("v").and_then(Value::as_f64) != Some(1.0) || !is_player_id(from) || !seq_ok {
        return Err(Refusal::Malformed);
    }
    let raw = from_b64url(public).ok_or(Refusal::Malformed)?;
    let sig = from_b64url(sig).ok_or(Refusal::Malformed)?;
    let mut signed = o.clone();
    signed.remove("sig");
    let data = canonical(&Value::Object(signed));
    let ok = match alg {
        "ed25519" => {
            let key: [u8; 32] = raw.as_slice().try_into().map_err(|_| Refusal::Malformed)?;
            let key =
                ed25519_dalek::VerifyingKey::from_bytes(&key).map_err(|_| Refusal::Malformed)?;
            if player_id(&raw) != from {
                return Err(Refusal::IdMismatch);
            }
            ed25519_dalek::Signature::from_slice(&sig)
                .is_ok_and(|sig| key.verify_strict(data.as_bytes(), &sig).is_ok())
        }
        _ => {
            use p256::ecdsa::signature::Verifier as _;
            let key =
                p256::ecdsa::VerifyingKey::from_sec1_bytes(&raw).map_err(|_| Refusal::Malformed)?;
            if player_id(&raw) != from {
                return Err(Refusal::IdMismatch);
            }
            p256::ecdsa::Signature::from_slice(&sig)
                .is_ok_and(|sig| key.verify(data.as_bytes(), &sig).is_ok())
        }
    };
    if ok {
        Ok(())
    } else {
        Err(Refusal::BadSignature)
    }
}

/// A sender a host has heard from: their key and the last `seq` accepted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sender {
    /// Their public key.
    #[serde(rename = "pub")]
    pub public: String,
    /// Its algorithm.
    pub alg: String,
    /// The last sequence number accepted.
    pub seq: u64,
}

/// A host's record of who it has heard from, by player ID: it refuses a
/// known ID with a different key ("id in use", which also covers two
/// players whose IDs collide) and replays (a `seq` not above the last).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Verifier {
    /// The senders.
    pub senders: BTreeMap<String, Sender>,
}

impl Verifier {
    /// Checks envelope `env`; a new valid one is remembered, and its
    /// sender's ID returned.
    pub fn verify(&mut self, env: &Value) -> Result<String, Refusal> {
        check_signature(env)?;
        let from = env["from"].as_str().unwrap_or("").to_string();
        let public = env["pub"].as_str().unwrap_or("").to_string();
        let alg = env["alg"].as_str().unwrap_or("").to_string();
        let seq = env["seq"].as_f64().unwrap_or(0.0) as u64;
        if let Some(k) = self.senders.get(&from) {
            if k.public != public || k.alg != alg {
                return Err(Refusal::IdInUse);
            }
            if seq <= k.seq {
                return Err(Refusal::Replayed);
            }
        }
        self.senders
            .insert(from.clone(), Sender { public, alg, seq });
        Ok(from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Envelopes signed by ducy-web's JS (identity.js), tampered copies, and
    /// its canonical JSON and shown IDs: what's signed today must keep
    /// checking out.
    fn vectors() -> Value {
        serde_json::from_str(include_str!("../tests/js-vectors.json")).unwrap()
    }

    #[test]
    fn messages_signed_in_the_browser_check_out_and_tampered_ones_dont() {
        let v = vectors();
        let all = v["envelopes"].as_array().unwrap();
        assert!(all.len() >= 50);
        for x in all {
            let got = check_signature(&x["env"]);
            if x["ok"] == true {
                assert_eq!(got, Ok(()), "{} {}", x["alg"], x["env"]["body"]);
            } else {
                assert_eq!(
                    got.map_err(Refusal::as_str),
                    Err(x["why"].as_str().unwrap()),
                    "{}",
                    x["alg"]
                );
            }
        }
    }

    #[test]
    fn canonical_json_is_byte_for_byte_javascripts() {
        for c in vectors()["canonical"].as_array().unwrap() {
            assert_eq!(canonical(&c["value"]), c["text"].as_str().unwrap());
        }
        // Keys sort by UTF-16, as JavaScript sorts them: the emoji's first
        // unit (a surrogate, 0xD83D) comes before U+FF61, though its UTF-8
        // bytes sort after.
        assert_eq!(
            canonical(&json!({"｡": 2, "😀": 1, "a": 3})),
            r#"{"a":3,"😀":1,"｡":2}"#
        );
        assert_eq!(
            canonical(&json!([1.5, -0.0, 1e21, 1e-7, 100])),
            "[1.5,0,1e+21,1e-7,100]"
        );
    }

    #[test]
    fn ids_and_how_people_see_them() {
        for s in vectors()["shown"].as_array().unwrap() {
            let id = s["id"].as_str().unwrap();
            assert_eq!(show_id(id), s["shown"].as_str().unwrap());
            assert_eq!(
                parse_shown_id(s["shown"].as_str().unwrap()).as_deref(),
                Some(id)
            );
            assert_eq!(
                parse_shown_id(
                    &s["shown"]
                        .as_str()
                        .unwrap()
                        .to_lowercase()
                        .replace('-', " ")
                )
                .as_deref(),
                Some(id)
            );
        }
        assert_eq!(show_id("nope"), "nope");
        assert_eq!(parse_shown_id("ABC"), None);
        // Past the last ID there is.
        assert_eq!(parse_shown_id("99999-99999"), None);
        assert!(is_player_id("AbCdEfGh") && !is_player_id("AbCdEfG0"));
    }

    #[test]
    fn a_verifier_refuses_replays_and_a_known_id_with_another_key() {
        let v = vectors();
        let good: Vec<&Value> = v["envelopes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|x| x["ok"] == true && x["alg"] == "ed25519")
            .map(|x| &x["env"])
            .collect();
        let mut ver = Verifier::default();
        let first = good[0];
        assert_eq!(ver.verify(first).unwrap(), first["from"].as_str().unwrap());
        assert_eq!(ver.verify(first), Err(Refusal::Replayed));
        // A later one from the same sender is taken.
        assert!(ver.verify(good[1]).is_ok());
        // Someone else claiming that ID with their own key.
        let mut other = ver.clone();
        other
            .senders
            .get_mut(first["from"].as_str().unwrap())
            .unwrap()
            .public = "x".into();
        assert_eq!(other.verify(good[2]), Err(Refusal::IdInUse));
        // Saved and loaded, it's the same.
        let saved = serde_json::to_value(&ver).unwrap();
        let back: Verifier = serde_json::from_value(saved).unwrap();
        assert_eq!(back, ver);
    }
}
