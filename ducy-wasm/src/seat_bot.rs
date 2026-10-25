//! A bot that plays one seat from that seat's table view, as a person at the
//! table would: for bots that sit at a table as players and send their moves
//! like anyone else (e.g. the bot members of ducy-web's demo clubs), rather
//! than being played by the table itself as [`crate::play::BotTable`]'s are.

use ducy_play::{Action, Bot, TableView, fallback_action};
use serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::play::{bot_for, default_chip, err, to_js};

/// One bot, by id: a personality ("nit"), a generic player ("g-rock-123"),
/// or any other id `BotTable` seats.
#[wasm_bindgen]
pub struct SeatBot {
    bot: Box<dyn Bot>,
}

#[derive(Serialize)]
struct Move {
    kind: &'static str,
    /// The street total for a bet or raise, otherwise 0.
    amount: u64,
}

#[wasm_bindgen]
impl SeatBot {
    #[wasm_bindgen(constructor)]
    pub fn new(id: &str, seed: u64) -> Result<SeatBot, JsError> {
        Ok(SeatBot {
            bot: bot_for(id, seed)?,
        })
    }

    /// What the bot does now, from its seat's table view (a `TableView`, as
    /// the table sends it): `{kind, amount}`, `kind` "fold", "check",
    /// "call", "bet", "raise" or "allin". Null when it isn't its turn.
    pub fn act(&mut self, view: JsValue) -> Result<JsValue, JsError> {
        let view: TableView = serde_wasm_bindgen::from_value(view).map_err(err)?;
        let Some(mut obs) = view.observation() else {
            return Ok(JsValue::NULL);
        };
        // The table's chip, as `rules_for` gives every table.
        obs.rules = obs.rules.with_chip(default_chip(obs.rules.small_blind));
        let action = self
            .bot
            .act(&obs)
            .unwrap_or_else(|| fallback_action(&obs.legal));
        let (kind, amount) = match action {
            Action::Fold => ("fold", 0),
            Action::Check => ("check", 0),
            Action::Call => ("call", 0),
            Action::Bet(to) => ("bet", to),
            Action::Raise(to) => ("raise", to),
            Action::AllIn => ("allin", 0),
        };
        to_js(&Move { kind, amount })
    }
}
