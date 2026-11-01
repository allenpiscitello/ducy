//! ducy.cards' games against the bots: the heads-up ladder ([`ladder`]), the
//! Regulars' table ([`regulars`]), the hand reviews against the GTO bot
//! ([`review`]) and what the bots say at the table ([`chat`]).
//!
//! The bots themselves (their play) are ducy-play's and ducy-gto's: this is
//! the site's games around them, with the pages' random draws passed in.

#![warn(missing_docs)]

pub mod chat;
pub mod ladder;
pub mod regulars;
pub mod review;

/// `Number(v) || 0` in JavaScript: a number, a numeric string, or 0.
pub(crate) fn num(v: Option<&serde_json::Value>) -> f64 {
    use serde_json::Value;
    let x = match v {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                0.0
            } else {
                t.parse().unwrap_or(0.0)
            }
        }
        Some(Value::Bool(true)) => 1.0,
        _ => 0.0,
    };
    if x.is_finite() { x } else { 0.0 }
}
