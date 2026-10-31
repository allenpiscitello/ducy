//! Provably fair deals in the browser (ducy-play's `fair_deal`, #141): each
//! player and the host make a secret, send its commitment, then reveal it;
//! the hand's seed comes from the revealed values, and anyone can rebuild
//! the deal from them. Bytes cross to JS as lowercase hex.

use ducy_play::Deal;
use ducy_play::fair_deal::{Bytes32, Reveal, hand_seed};
use serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::util::rules_for;

pub(crate) fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub(crate) fn bytes32(s: &str) -> Result<Bytes32, JsError> {
    let bad = || JsError::new("expected 64 hex digits");
    if s.len() != 64 {
        return Err(bad());
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).map_err(|_| bad())?;
    }
    Ok(out)
}

#[derive(Serialize)]
struct Secret {
    value: String,
    nonce: String,
    commitment: String,
}

/// A fresh secret for one hand: `{value, nonce, commitment}`. Send the
/// commitment first; reveal value and nonce once every commitment is in.
#[wasm_bindgen(js_name = fairSecret)]
pub fn fair_secret() -> Result<JsValue, JsError> {
    let r = Reveal::random();
    let s = Secret {
        value: hex(&r.value),
        nonce: hex(&r.nonce),
        commitment: hex(&r.commitment()),
    };
    serde_wasm_bindgen::to_value(&s).map_err(|e| JsError::new(&e.to_string()))
}

/// The commitment for a value and nonce, to check someone's reveal.
#[wasm_bindgen(js_name = fairCommitment)]
pub fn fair_commitment(value: &str, nonce: &str) -> Result<String, JsError> {
    let r = Reveal {
        value: bytes32(value)?,
        nonce: bytes32(nonce)?,
    };
    Ok(hex(&r.commitment()))
}

/// The hand's seed from the revealed values, in seat order.
#[wasm_bindgen(js_name = fairSeed)]
pub fn fair_seed(values: Vec<String>) -> Result<String, JsError> {
    let values = values
        .iter()
        .map(|v| bytes32(v))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(hex(&hand_seed(&values)))
}

fn b32(s: &str) -> Result<Bytes32, String> {
    bytes32(s).map_err(|_| "expected 64 hex digits".to_string())
}

/// A commitment list from the host: [{id, c}].
fn list_of(v: &serde_json::Value) -> Result<Vec<(String, Bytes32)>, String> {
    v.as_array()
        .ok_or("no list")?
        .iter()
        .map(|e| {
            let id = e
                .get("id")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            Ok((id, b32(e.get("c").and_then(|x| x.as_str()).unwrap_or(""))?))
        })
        .collect()
}

fn reveal_of(v: &serde_json::Value) -> Result<Reveal, String> {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("");
    Ok(Reveal {
        value: b32(s("value"))?,
        nonce: b32(s("nonce"))?,
    })
}

/// Whether to reveal this player's secret to the host's commitment list
/// (JSON, [{id, c}]): it has their `commitment` and the host's, each id
/// once.
#[wasm_bindgen(js_name = fairMayReveal)]
pub fn fair_may_reveal(list: &str, commitment: &str) -> bool {
    let list = serde_json::from_str(list)
        .ok()
        .and_then(|v: serde_json::Value| list_of(&v).ok());
    match (list, b32(commitment)) {
        (Some(l), Ok(c)) => ducy_play::fair_deal::may_reveal(&l, &c),
        _ => false,
    }
}

#[derive(Serialize)]
struct Check {
    ok: bool,
    why: String,
    ours: bool,
}

/// A player's check of the host's published deal (`m`, JSON: {game, dealt,
/// index, seed, list: [{id, c}], reveals: [{id, value, nonce}]}) against
/// what they saw (`mine`, JSON: {cards, board}) and their own secret, if
/// they took part (JSON {value, nonce}, or "null"): {ok, why, ours}.
#[wasm_bindgen(js_name = fairCheck)]
pub fn fair_check(m: &str, mine: &str, secret: &str) -> Result<JsValue, JsError> {
    let checked = (|| -> Result<Result<bool, String>, String> {
        let m: serde_json::Value = serde_json::from_str(m).map_err(|e| e.to_string())?;
        let mine: serde_json::Value = serde_json::from_str(mine).unwrap_or_default();
        let secret: serde_json::Value = serde_json::from_str(secret).unwrap_or_default();
        let num = |k: &str| {
            m.get(k)
                .and_then(|x| {
                    x.as_u64()
                        .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
                })
                .unwrap_or(0) as usize
        };
        let rules = rules_for(m.get("game").and_then(|g| g.as_str()), 1, 2)
            .map_err(|_| "not a game ducy deals".to_string())?;
        let reveals = m
            .get("reveals")
            .and_then(|r| r.as_array())
            .map(|a| {
                a.iter()
                    .map(|u| {
                        let id = u
                            .get("id")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string();
                        Ok((id, reveal_of(u)?))
                    })
                    .collect::<Result<Vec<_>, String>>()
            })
            .transpose()?
            .unwrap_or_default();
        let p = ducy_play::fair_deal::Published {
            variant: rules.variant,
            dealt: num("dealt"),
            index: num("index"),
            seed: b32(m.get("seed").and_then(|s| s.as_str()).unwrap_or(""))?,
            list: list_of(m.get("list").unwrap_or(&serde_json::Value::Null)).unwrap_or_default(),
            reveals,
        };
        let strings = |k: &str| -> Vec<String> {
            mine.get(k)
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|c| c.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        let own = if secret.is_object() {
            Some(reveal_of(&secret)?)
        } else {
            None
        };
        Ok(ducy_play::fair_deal::check_published(
            &p,
            &strings("cards"),
            &strings("board"),
            own.as_ref(),
        ))
    })();
    let out = match checked {
        Ok(Ok(ours)) => Check {
            ok: true,
            why: String::new(),
            ours,
        },
        Ok(Err(why)) => Check {
            ok: false,
            why,
            ours: false,
        },
        Err(e) => Check {
            ok: false,
            why: format!("it couldn’t be checked ({e})"),
            ours: false,
        },
    };
    serde_wasm_bindgen::to_value(&out).map_err(|e| JsError::new(&e.to_string()))
}

#[derive(Serialize)]
struct DealOut {
    hole_cards: Vec<Vec<String>>,
    board: Vec<String>,
}

/// The deal for a seed: `{hole_cards: [[card, …] per player, in dealing
/// order], board: [5 cards]}`, for checking a hand afterwards. `game` is
/// "nlhe", "plo4", "plo5" or "plo6".
#[wasm_bindgen(js_name = fairDeal)]
pub fn fair_deal(game: &str, players: usize, seed: &str) -> Result<JsValue, JsError> {
    let rules = rules_for(Some(game), 1, 2)?;
    let deal = Deal::from_seed(rules.variant, players, &bytes32(seed)?)
        .map_err(|e| JsError::new(&e.to_string()))?;
    let out = DealOut {
        hole_cards: deal
            .hole_cards()
            .iter()
            .map(|h| h.iter(false).map(|c| c.to_string()).collect())
            .collect(),
        board: deal.board().iter().map(|c| c.to_string()).collect(),
    };
    serde_wasm_bindgen::to_value(&out).map_err(|e| JsError::new(&e.to_string()))
}
