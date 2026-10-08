//! Provably fair deals in the browser (ducy-play's `fair_deal`, #141): each
//! player and the host make a secret, send its commitment, then reveal it;
//! the hand's seed comes from the revealed values, and anyone can rebuild
//! the deal from them. Bytes cross to JS as lowercase hex.

use ducy_play::Deal;
use ducy_play::fair_deal::{Bytes32, Reveal, hand_seed};
use serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::play::rules_for;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn bytes32(s: &str) -> Result<Bytes32, JsError> {
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
