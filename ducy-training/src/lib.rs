//! ducy.cards' study tools (ducy-web #464): practising Ducybot's pre-flop
//! charts ([`chart`]) and top-% ranges ([`skills`], [`practice`]), with
//! their scoring, mastery and pacing. The pages draw; this decides. Random
//! draws come in from the caller (numbers in `0..1`, or a seed for the
//! practice's dice), so the same draws give the same hands.

#![warn(missing_docs)]

pub mod chart;
pub mod holdem;
pub mod holdem_rank;
pub mod practice;
pub mod skills;

/// A number as JavaScript writes it (`String(x)`): "2.5", "3", "0.1".
pub(crate) fn js_number(x: f64) -> String {
    ryu_js::Buffer::new().format(x).to_string()
}

/// `x` rounded to `places` decimals, as JavaScript's `+x.toFixed(places)`.
pub(crate) fn fixed(x: f64, places: i32) -> f64 {
    let p = 10f64.powi(places);
    (x * p).round() / p
}
