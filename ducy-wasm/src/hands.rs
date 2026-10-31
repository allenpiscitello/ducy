//! What the table shows about the hand in play (ducy-play's `hands`, ducy-web
//! #461): the pots as they form, a hand in words, who posts the next big
//! blind, and the record kept of a finished hand.

use ducy_play::{
    Event, TableView,
    hands::{self, HandRecord, RecordOf, SeatChips},
};
use wasm_bindgen::prelude::*;

use crate::util::{err, to_js};

/// The pots to show now, from the seats ({stack, street_bet, folded, all_in,
/// empty, sitting_out}) and the hand's events: {pots: [{amount, eligible}],
/// uncalled: {seat, amount} | null}, main pot first; null when there's just
/// the one pot and nothing uncalled.
#[wasm_bindgen(js_name = shownPots)]
pub fn shown_pots(seats: JsValue, events: JsValue) -> Result<JsValue, JsError> {
    let seats: Vec<SeatChips> = serde_wasm_bindgen::from_value(seats).map_err(err)?;
    let events: Vec<Event> = serde_wasm_bindgen::from_value(events).map_err(err)?;
    to_js(&hands::shown_pots(&seats, &events))
}

/// The seat that posts the big blind in the next hand, or -1 if fewer than
/// two seats would be dealt in. `refill`: seats with no chips are dealt in
/// anyway (a table that tops its bots up).
#[wasm_bindgen(js_name = nextBigBlind)]
pub fn next_big_blind(seats: JsValue, button: usize, refill: bool) -> Result<i32, JsError> {
    let seats: Vec<SeatChips> = serde_wasm_bindgen::from_value(seats).map_err(err)?;
    Ok(hands::next_big_blind(&seats, button, refill).map_or(-1, |s| s as i32))
}

/// What `hole` makes with `board` (3 to 5 cards), in words, like
/// "Straight, 8 high"; undefined before the flop.
#[wasm_bindgen(js_name = handWords)]
pub fn hand_words(hole: Vec<String>, board: Vec<String>) -> Option<String> {
    hands::hand_words(&hole, &board)
}

/// A pot's `winning_hand` ("Straight 8 high") in words ("Straight, 8
/// high"); undefined if it isn't a hand's name.
#[wasm_bindgen(js_name = handNameWords)]
pub fn hand_name_words(name: &str) -> Option<String> {
    hands::name_words(name)
}

/// The record kept of the finished hand in `view` (a table view), `of`
/// {club, table, tableName, game, who}, at `now` (milliseconds).
#[wasm_bindgen(js_name = handRecord)]
pub fn hand_record(view: JsValue, of: JsValue, now: f64) -> Result<JsValue, JsError> {
    let view: TableView = serde_wasm_bindgen::from_value(view).map_err(err)?;
    let of: RecordOf = serde_wasm_bindgen::from_value(of).map_err(err)?;
    to_js(&HandRecord::new(&view, of, now as u64))
}

/// A kept hand replayed: frames of {street, board, pot, seats: [{stack,
/// bet, folded, all_in}], text, event}, one before any action and one after
/// each event.
#[wasm_bindgen(js_name = recordFrames)]
pub fn record_frames(record: JsValue) -> Result<JsValue, JsError> {
    let record: HandRecord = serde_wasm_bindgen::from_value(record).map_err(err)?;
    to_js(&record.frames())
}
