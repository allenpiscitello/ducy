//! One-off private games (ducy-play's `club_oneoff`, ducy-web #457): names,
//! points, what players see, standings and who owes whom, and a cash
//! game's ledger. Each op reads only what it's given and returns
//! `{ok: true, value}` or `{ok: false, message}`.

use std::collections::BTreeMap;

use ducy_play::club_host::ClubState;
use ducy_play::club_oneoff::{self, Ledger, Seated};
use ducy_play::club_rules::{Member, RuleError};
use ducy_play::club_tourneys::Tourney;
use serde::Deserialize;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

use crate::play::{err, to_js};

/// An op, as ducy-web sends it.
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Op {
    CleanName {
        name: String,
    },
    UniqueName {
        taken: Vec<String>,
        name: String,
    },
    CheckBuyIn {
        #[serde(default)]
        buy_in: Value,
    },
    CheckPrizes {
        list: Value,
    },
    PaidPlaces {
        tourney: Tourney,
    },
    AutoPrizes {
        tourney: Tourney,
    },
    PrizeList {
        tourney: Tourney,
    },
    PrizeTotals {
        tourney: Tourney,
    },
    LobbyBody {
        code: String,
        members: BTreeMap<String, Member>,
        tourney: Tourney,
        #[serde(default)]
        live: Value,
    },
    Results {
        body: Value,
    },
    Standings {
        body: Value,
    },
    SettleUp {
        rows: Vec<Value>,
    },
    Give {
        ledger: Ledger,
        id: String,
        #[serde(default)]
        name: String,
        amount: f64,
    },
    HandBack {
        ledger: Ledger,
        id: String,
        amount: f64,
    },
    CashOut {
        ledger: Ledger,
        id: String,
        chips: f64,
    },
    AtTable {
        ledger: Ledger,
        seated: Vec<Seated>,
    },
    CashStandings {
        ledger: Ledger,
    },
}

fn value(v: impl serde::Serialize) -> Result<Value, RuleError> {
    serde_json::to_value(v).map_err(|e| RuleError(e.to_string()))
}

/// Runs one-off tournament op `op` ("new", "join", "leave", "prizes"; see
/// `ClubState::private_tourney`) with `args` (JSON) on the stand-in club.
/// Returns `{ok: true, value, club}`, or `{ok: false, message}` with
/// nothing changed.
#[wasm_bindgen(js_name = privateTourney)]
pub fn private_tourney(club: &str, op: &str, args: &str, now: f64) -> Result<JsValue, JsError> {
    let mut c: ClubState = serde_json::from_str(club).map_err(err)?;
    let args: Value = serde_json::from_str(args).map_err(err)?;
    to_js(
        &match c.private_tourney(op, &args, now as u64, &mut rand::make_rng()) {
            Ok(value) => json!({"ok": true, "value": value, "club": c}),
            Err(e) => json!({"ok": false, "message": e.0}),
        },
    )
}

/// Runs `op`. Throws only for an op it can't read.
#[wasm_bindgen(js_name = oneOff)]
pub fn one_off(op: JsValue) -> Result<JsValue, JsError> {
    let op: Op = serde_wasm_bindgen::from_value(op).map_err(err)?;
    let r: Result<Value, RuleError> = match op {
        Op::CleanName { name } => Ok(club_oneoff::clean_name(&name).into()),
        Op::UniqueName { taken, name } => Ok(club_oneoff::unique_name(&taken, &name).into()),
        Op::CheckBuyIn { buy_in } => club_oneoff::check_buy_in(&buy_in).and_then(value),
        Op::CheckPrizes { list } => club_oneoff::check_prizes(&list).and_then(value),
        Op::PaidPlaces { tourney } => value(club_oneoff::paid_places(&tourney)),
        Op::AutoPrizes { tourney } => value(club_oneoff::auto_prizes(&tourney)),
        Op::PrizeList { tourney } => value(club_oneoff::prize_list(&tourney)),
        Op::PrizeTotals { tourney } => Ok(club_oneoff::prize_totals(&tourney)),
        Op::LobbyBody {
            code,
            members,
            tourney,
            live,
        } => Ok(club_oneoff::lobby_body(
            &code,
            &members,
            &tourney,
            Some(&live),
        )),
        Op::Results { body } => value(club_oneoff::results(&body)),
        Op::Standings { body } => value(club_oneoff::tourney_standings(&body)),
        Op::SettleUp { rows } => value(club_oneoff::settle_up(&rows)),
        Op::Give {
            mut ledger,
            id,
            name,
            amount,
        } => {
            ledger.give(&id, &name, amount as i64);
            value(ledger)
        }
        Op::HandBack {
            mut ledger,
            id,
            amount,
        } => {
            ledger.hand_back(&id, amount as i64);
            value(ledger)
        }
        Op::CashOut {
            mut ledger,
            id,
            chips,
        } => {
            ledger.cash_out(&id, chips as i64);
            value(ledger)
        }
        Op::AtTable { mut ledger, seated } => {
            ledger.at_table(&seated);
            value(ledger)
        }
        Op::CashStandings { ledger } => value(ledger.standings()),
    };
    to_js(&match r {
        Ok(v) => json!({"ok": true, "value": v}),
        Err(e) => json!({"ok": false, "message": e.0}),
    })
}
