//! A table where one person plays no-limit Hold'em or pot-limit Omaha (PLO4,
//! PLO5, PLO6) against ducy-play's personality bots.
//!
//! The page drives it one action at a time: [`BotTable::advance`] makes the
//! next bot act, and [`BotTable::act`] applies the person's action, so the UI
//! can show each decision as it happens. [`BotTable::state`] returns what the
//! person may see: their own cards, public chip counts, and other players'
//! cards only once they're shown down.
//!
//! [`MultiTable`] hosts the same kind of table for people on other devices:
//! the page passes it each player's messages and sends back what it returns.

use ducy_play::{
    Action, Command, Outgoing, Personality, Table, TableHost, TableRules, TableSeat, Variant,
};
use serde::Serialize;
use wasm_bindgen::prelude::*;

fn err(e: impl std::fmt::Debug) -> JsError {
    JsError::new(&format!("{e:?}"))
}

#[derive(Serialize)]
struct PersonalityInfo {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    catchphrase: &'static str,
}

/// Every bot personality, for the page's seat picker.
#[wasm_bindgen(js_name = botPersonalities)]
pub fn bot_personalities() -> Result<JsValue, JsError> {
    let list: Vec<PersonalityInfo> = Personality::ALL
        .iter()
        .map(|p| PersonalityInfo {
            id: p.id(),
            name: p.name(),
            description: p.description(),
            catchphrase: p.catchphrase(),
        })
        .collect();
    serde_wasm_bindgen::to_value(&list).map_err(err)
}

fn rules_for(game: Option<&str>, small_blind: u64, big_blind: u64) -> Result<TableRules, JsError> {
    Ok(match game.unwrap_or("nlhe") {
        "nlhe" => TableRules::no_limit_holdem(small_blind, big_blind),
        g @ ("plo4" | "plo5" | "plo6") => TableRules::pot_limit_omaha(small_blind, big_blind)
            .with_variant(Variant::Omaha {
                hole_cards: g[3..].parse().unwrap_or(4),
            }),
        g => return Err(JsError::new(&format!("unknown game {g}"))),
    })
}

/// "You" in seat 0, then one seat per bot id.
fn seats_for(bots: &[String], seed: u64) -> Result<Vec<TableSeat>, JsError> {
    if bots.is_empty() || bots.len() + 1 > ducy_play::MAX_PLAYERS {
        return Err(JsError::new("between 1 and 8 bots"));
    }
    let mut seats = vec![TableSeat::human("You", "you")];
    for (i, id) in bots.iter().enumerate() {
        let p =
            Personality::from_name(id).ok_or_else(|| JsError::new(&format!("unknown bot {id}")))?;
        seats.push(TableSeat::bot(
            p.id(),
            p.bot(Some(seed.wrapping_add(i as u64 + 1))),
        ));
    }
    Ok(seats)
}

fn action(kind: &str, amount: u64) -> Result<Action, JsError> {
    Ok(match kind {
        "fold" => Action::Fold,
        "check" => Action::Check,
        "call" => Action::Call,
        "bet" => Action::Bet(amount),
        "raise" => Action::Raise(amount),
        "allin" => Action::AllIn,
        _ => return Err(JsError::new(&format!("unknown action {kind}"))),
    })
}

/// A Hold'em or Omaha table: the person in seat 0 and up to eight bots.
#[wasm_bindgen]
pub struct BotTable {
    table: Table,
}

#[wasm_bindgen]
impl BotTable {
    /// `bots` are personality ids (see `botPersonalities`), one per bot seat;
    /// the person sits in seat 0. Everyone starts with `buy_in` chips, and a
    /// player who goes broke is topped back up before the next hand. `game`
    /// is "nlhe" (no-limit Hold'em, the default) or "plo4", "plo5", "plo6"
    /// (pot-limit Omaha with that many hole cards).
    #[wasm_bindgen(constructor)]
    pub fn new(
        bots: Vec<String>,
        buy_in: u64,
        small_blind: u64,
        big_blind: u64,
        seed: u64,
        game: Option<String>,
    ) -> Result<BotTable, JsError> {
        let rules = rules_for(game.as_deref(), small_blind, big_blind)?;
        if small_blind == 0 || big_blind < small_blind || buy_in < big_blind {
            return Err(JsError::new("invalid blinds or buy-in"));
        }
        let table = Table::new(rules, seats_for(&bots, seed)?, buy_in, seed).map_err(err)?;
        Ok(BotTable { table })
    }

    /// Deals the next hand (moving the button and topping up broke players)
    /// and returns the state.
    #[wasm_bindgen(js_name = newHand)]
    pub fn new_hand(&mut self) -> Result<JsValue, JsError> {
        if self.table.in_hand() {
            return Err(JsError::new("the current hand isn't over"));
        }
        self.table.new_hand().map_err(err)?;
        self.state()
    }

    /// Lets the next bot act if it's a bot's turn; returns the state either way.
    pub fn advance(&mut self) -> Result<JsValue, JsError> {
        if self.table.hand().is_none() {
            return Err(JsError::new("no hand dealt"));
        }
        self.table.advance().map_err(err)?;
        self.state()
    }

    /// Applies the person's action: "fold", "check", "call", "bet", "raise"
    /// or "allin". `amount` is the street total for a bet or raise.
    pub fn act(&mut self, kind: &str, amount: u64) -> Result<JsValue, JsError> {
        if self.table.hand().is_none() {
            return Err(JsError::new("no hand dealt"));
        }
        if self.table.to_act() != Some(0) {
            return Err(JsError::new("it isn't your turn"));
        }
        self.table.act(0, action(kind, amount)?).map_err(err)?;
        self.state()
    }

    /// Whether it's a bot's turn (the page calls `advance` while this is true).
    #[wasm_bindgen(js_name = botToAct)]
    pub fn bot_to_act(&self) -> bool {
        self.table.auto_to_act()
    }

    /// What the person may see right now.
    pub fn state(&self) -> Result<JsValue, JsError> {
        if self.table.hand().is_none() {
            return Err(JsError::new("no hand dealt"));
        }
        to_js(&self.table.view(0))
    }
}

fn to_js(value: &impl Serialize) -> Result<JsValue, JsError> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(err)
}

#[derive(Serialize)]
struct Message<'a> {
    to: &'a str,
    data: &'a ducy_play::Update,
}

/// What a host call returns: the host's own view, whether a bot or away
/// player should act next, and the messages to send to each player.
#[derive(Serialize)]
struct HostResult<'a> {
    state: ducy_play::TableView,
    #[serde(rename = "botToAct")]
    bot_to_act: bool,
    #[serde(rename = "turnMsLeft")]
    turn_ms_left: Option<u64>,
    #[serde(rename = "openSeats")]
    open_seats: usize,
    out: Vec<Message<'a>>,
}

/// A table hosted for people on other devices: the host in seat 0, bots in
/// the other seats, and some of those seats open for people to take. Every
/// call takes the time in milliseconds (e.g. `Date.now()`) and returns
/// `{state, botToAct, turnMsLeft, openSeats, out}`, where `out` lists
/// `{to, data}` messages for the page to send to each player.
#[wasm_bindgen]
pub struct MultiTable {
    host: TableHost,
}

#[wasm_bindgen]
impl MultiTable {
    /// Like [`BotTable::new`], with `open` saying which bot seats people may
    /// take (one flag per bot) and `host_name` the host's shown name.
    /// `turn_ms` is how long a person has to act (0 for no limit).
    #[wasm_bindgen(constructor)]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        bots: Vec<String>,
        open: Vec<u8>,
        host_name: String,
        buy_in: u64,
        small_blind: u64,
        big_blind: u64,
        seed: u64,
        game: Option<String>,
        turn_ms: u64,
    ) -> Result<MultiTable, JsError> {
        let rules = rules_for(game.as_deref(), small_blind, big_blind)?;
        if small_blind == 0 || big_blind < small_blind || buy_in < big_blind {
            return Err(JsError::new("invalid blinds or buy-in"));
        }
        if open.len() != bots.len() {
            return Err(JsError::new("one open flag per bot"));
        }
        let mut seats = seats_for(&bots, seed)?;
        let name: String = host_name.trim().chars().take(ducy_play::MAX_NAME).collect();
        if !name.is_empty() {
            seats[0].name = name;
        }
        let table = Table::new(rules, seats, buy_in, seed).map_err(err)?;
        let open = std::iter::once(false)
            .chain(open.iter().map(|&o| o != 0))
            .collect();
        let host = TableHost::new(table, open, turn_ms).map_err(err)?;
        Ok(MultiTable { host })
    }

    fn result(&self, out: &[Outgoing], now: u64) -> Result<JsValue, JsError> {
        to_js(&HostResult {
            state: self.host.host_view(),
            bot_to_act: self.host.auto_to_act(),
            turn_ms_left: self.host.turn_ms_left(now),
            open_seats: self.host.open_seats(),
            out: out
                .iter()
                .map(|o| Message {
                    to: &o.to,
                    data: &o.update,
                })
                .collect(),
        })
    }

    /// The host's view, with no messages.
    pub fn state(&self, now: f64) -> Result<JsValue, JsError> {
        self.result(&[], now as u64)
    }

    /// A message from player `client` (a JSON command object).
    pub fn handle(&mut self, client: &str, command: JsValue, now: f64) -> Result<JsValue, JsError> {
        let now = now as u64;
        let out = match serde_wasm_bindgen::from_value::<Command>(command) {
            Ok(c) => self.host.handle(client, c, now),
            Err(_) => vec![Outgoing {
                to: client.to_string(),
                update: ducy_play::Update::Rejected {
                    reason: "bad message".to_string(),
                },
            }],
        };
        self.result(&out, now)
    }

    /// Player `client`'s connection dropped.
    pub fn disconnected(&mut self, client: &str, now: f64) -> Result<JsValue, JsError> {
        let out = self.host.disconnected(client, now as u64);
        self.result(&out, now as u64)
    }

    /// Deals the next hand.
    #[wasm_bindgen(js_name = newHand)]
    pub fn new_hand(&mut self, now: f64) -> Result<JsValue, JsError> {
        let out = self.host.new_hand(now as u64).map_err(err)?;
        self.result(&out, now as u64)
    }

    /// Lets a bot (or an away player) act.
    pub fn advance(&mut self, now: f64) -> Result<JsValue, JsError> {
        let out = self.host.advance(now as u64).map_err(err)?;
        self.result(&out, now as u64)
    }

    /// The host's own action.
    pub fn act(&mut self, kind: &str, amount: u64, now: f64) -> Result<JsValue, JsError> {
        let out = self.host.host_act(kind, amount, now as u64).map_err(err)?;
        self.result(&out, now as u64)
    }

    /// Runs the turn clock; call it every half second or so.
    pub fn tick(&mut self, now: f64) -> Result<JsValue, JsError> {
        let out = self.host.tick(now as u64);
        self.result(&out, now as u64)
    }
}
