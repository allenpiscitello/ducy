//! What every part of the bindings shares: errors and values for JS, a
//! game's rules, and a club table's host. Kept apart from the bot tables
//! (`play`), so the relay's build can leave those out (ducy-web #463).

use ducy_play::{Table, TableRules, TableSeat, Variant};

use ducy_host::TableHost;
use serde::Serialize;
use wasm_bindgen::prelude::*;

pub(crate) fn err(e: impl std::fmt::Debug) -> JsError {
    JsError::new(&format!("{e:?}"))
}

pub(crate) fn to_js(value: &impl Serialize) -> Result<JsValue, JsError> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(err)
}

/// The casino chips, in the page's hundredths: $1, $5, $25, $100, $500,
/// $1,000, $5,000, $25,000.
const CHIPS: [u64; 8] = [100, 500, 2_500, 10_000, 50_000, 100_000, 500_000, 2_500_000];

/// A game's smallest chip, by default: the largest casino chip not above the
/// small blind. So 1/2 and 2/5 bet in $1, 5/10 and 10/25 in $5, 25/50 in $25.
/// Below a $1 small blind, any amount.
pub(crate) fn default_chip(small_blind: u64) -> u64 {
    CHIPS
        .iter()
        .rev()
        .copied()
        .find(|&c| c <= small_blind)
        .unwrap_or(1)
}

pub(crate) fn rules_for(
    game: Option<&str>,
    small_blind: u64,
    big_blind: u64,
) -> Result<TableRules, JsError> {
    Ok(rules_for_game(game, small_blind, big_blind)?.with_chip(default_chip(small_blind)))
}

fn rules_for_game(
    game: Option<&str>,
    small_blind: u64,
    big_blind: u64,
) -> Result<TableRules, JsError> {
    Ok(match game.unwrap_or("nlhe") {
        "nlhe" => TableRules::no_limit_holdem(small_blind, big_blind),
        g @ ("plo4" | "plo5" | "plo6") => TableRules::pot_limit_omaha(small_blind, big_blind)
            .with_variant(Variant::Omaha {
                hole_cards: g[3..].parse().unwrap_or(4),
            }),
        // A PLO bomb pot on two boards: everyone antes the big blind, and the
        // hand starts on the flop.
        "plo4-bomb" => {
            TableRules::pot_limit_omaha(small_blind, big_blind).bomb_pot(big_blind, true)
        }
        g => return Err(JsError::new(&format!("unknown game {g}"))),
    })
}

/// A club table's host: `seats` empty seats and no seat for the host.
#[allow(clippy::too_many_arguments)]
pub(crate) fn club_host(
    seats: usize,
    min_buy_in: u64,
    max_buy_in: u64,
    small_blind: u64,
    big_blind: u64,
    seed: u64,
    game: Option<String>,
    turn_ms: u64,
) -> Result<TableHost, JsError> {
    let rules = rules_for(game.as_deref(), small_blind, big_blind)?;
    if small_blind == 0 || big_blind < small_blind || min_buy_in < big_blind {
        return Err(JsError::new("invalid blinds or buy-in"));
    }
    if !(2..=ducy_play::MAX_PLAYERS).contains(&seats) {
        return Err(JsError::new("a table seats 2 to 10"));
    }
    let all = (0..seats).map(|_| TableSeat::empty()).collect();
    let mut table = Table::new(rules, all, min_buy_in, seed)
        .map_err(err)?
        .with_secure_deals();
    table.set_run_it_twice(true);
    TableHost::without_host(table, turn_ms, min_buy_in, max_buy_in).map_err(err)
}
