//! A club table and its runner (ducy-play's `runner`, ducy-web #452), for
//! ducy-web's club-engine.js, with events and steps as plain JS objects.
//! Bytes cross as lowercase hex.

use ducy_host::runner::{
    ASK_MS, Action, Event, FairResult, IDLE_MS, NEXT_HAND_MS, PlayerCommand, RunnerConfig,
    RunnerUpdate, SeedPicker, Step, TableResult, TableRunner,
};
use ducy_host::{Command, HostSnapshot, TableHost};
use ducy_play::fair_deal::{Bytes32, Reveal};
use serde::Deserialize;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

use crate::fair::{bytes32, hex};
use crate::util::{club_host, err, to_js};

/// The table and runner options from club-host.js.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    #[serde(default)]
    seats: usize,
    #[serde(default)]
    min_buy_in: u64,
    #[serde(default)]
    max_buy_in: u64,
    #[serde(default)]
    sb: u64,
    #[serde(default)]
    bb: u64,
    #[serde(default)]
    seed: u64,
    #[serde(default)]
    game: Option<String>,
    #[serde(default)]
    turn_ms: u64,
    #[serde(default)]
    sit_out_ms: Option<u64>,
    #[serde(default = "yes")]
    open: bool,
    #[serde(default = "yes")]
    link: bool,
    #[serde(default)]
    active: std::collections::BTreeMap<String, f64>,
    #[serde(default)]
    idle_ms: Option<u64>,
    #[serde(default)]
    ask_ms: Option<u64>,
    #[serde(default)]
    next_hand_ms: Option<u64>,
}

fn yes() -> bool {
    true
}

impl Config {
    fn runner(&self, id: String, paused: bool) -> RunnerConfig {
        RunnerConfig {
            id,
            game: self.game.clone().unwrap_or_else(|| "nlhe".to_string()),
            open: self.open,
            link: self.link,
            paused,
            active: self
                .active
                .iter()
                .map(|(k, v)| (k.clone(), *v as u64))
                .collect(),
            idle_ms: self.idle_ms.unwrap_or(IDLE_MS),
            ask_ms: self.ask_ms.unwrap_or(ASK_MS),
            next_hand_ms: self.next_hand_ms.unwrap_or(NEXT_HAND_MS),
        }
    }
}

const SIT_OUT_MS: u64 = 10 * 60_000;

/// A club table run by its runner. Every call takes the time in
/// milliseconds (e.g. `Date.now()`).
#[wasm_bindgen]
pub struct ClubTable {
    runner: TableRunner,
}

#[wasm_bindgen]
impl ClubTable {
    /// A new table `id`: `config` = {seats, minBuyIn, maxBuyIn, sb, bb, seed,
    /// game, turnMs, sitOutMs, open, link, idleMs, askMs, nextHandMs}.
    pub fn create(id: String, config: JsValue) -> Result<ClubTable, JsError> {
        let c: Config = serde_wasm_bindgen::from_value(config).map_err(err)?;
        let mut host = club_host(
            c.seats,
            c.min_buy_in,
            c.max_buy_in,
            c.sb,
            c.bb,
            c.seed,
            c.game.clone(),
            c.turn_ms,
        )?;
        host.set_sit_out_limit(c.sit_out_ms.unwrap_or(SIT_OUT_MS));
        Ok(ClubTable {
            runner: TableRunner::new(host, c.runner(id, false)),
        })
    }

    /// A table saved with `save`, waiting for its people; `config` as for
    /// `create`, plus `active` ({member: last activity}). Null if it was saved
    /// holding a chip request (the club may have paid it). Throws if it can't
    /// be loaded.
    pub fn restore(
        id: String,
        json: &str,
        config: JsValue,
        now: f64,
    ) -> Result<Option<ClubTable>, JsError> {
        let c: Config = serde_wasm_bindgen::from_value(config).map_err(err)?;
        let s: HostSnapshot = serde_json::from_str(json).map_err(err)?;
        let mut host = TableHost::restore(&s, now as u64, |_| None).map_err(err)?;
        if !host.chip_requests().is_empty() {
            return Ok(None);
        }
        host.set_sit_out_limit(c.sit_out_ms.unwrap_or(SIT_OUT_MS));
        Ok(Some(ClubTable {
            runner: TableRunner::new(host, c.runner(id, true)),
        }))
    }

    /// Takes in an event ({type, …}, the runner's `Event`) and returns the step:
    /// {actions, wake, stopped, paused}.
    pub fn event(&mut self, ev: JsValue, now: f64) -> Result<JsValue, JsError> {
        let v: Value = serde_wasm_bindgen::from_value(ev).map_err(err)?;
        let step = self.runner.event(event(&v)?, now as u64);
        to_js(&step_json(&step)?)
    }

    /// The table as JSON, to `restore` after a restart.
    pub fn save(&mut self, now: f64) -> Result<String, JsError> {
        serde_json::to_string(&self.runner.snapshot(now as u64)).map_err(err)
    }

    /// Deals each hand from `pick`'s seed when it gives one, in place of the
    /// fair-deal round (ducy-web's demo club): `pick({state, players})`, the
    /// table about to deal, returns a seed in hex or nothing. Null stops it.
    #[wasm_bindgen(js_name = setSeedPicker)]
    pub fn set_seed_picker(&mut self, pick: Option<SeedFn>) {
        self.runner.set_seed_picker(pick.map(|f| -> SeedPicker {
            Box::new(move |host: &TableHost| {
                let table =
                    to_js(&json!({"state": host.host_view(), "players": host.seated()})).ok()?;
                let seed = f.call(&JsValue::NULL, &table).ok()?.as_string()?;
                bytes32(&seed).ok()
            })
        }));
    }
}

#[wasm_bindgen]
extern "C" {
    /// A JS function for `setSeedPicker`.
    #[wasm_bindgen(typescript_type = "(table: any) => string | undefined")]
    pub type SeedFn;

    #[wasm_bindgen(method, catch)]
    fn call(this: &SeedFn, ctx: &JsValue, table: &JsValue) -> Result<JsValue, JsValue>;
}

fn s(v: &Value, k: &str) -> Result<String, JsError> {
    v[k].as_str()
        .map(str::to_string)
        .ok_or_else(|| JsError::new(&format!("event: no {k}")))
}

fn b(v: &Value, k: &str) -> bool {
    v[k].as_bool().unwrap_or(false)
}

fn n(v: &Value, k: &str) -> Result<usize, JsError> {
    v[k].as_u64()
        .map(|x| x as usize)
        .ok_or_else(|| JsError::new(&format!("event: no {k}")))
}

/// 64 lowercase hex digits, as pages send them; anything else is None.
fn bytes(v: &Value) -> Option<Bytes32> {
    let s = v.as_str()?;
    if s.len() != 64
        || !s
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return None;
    }
    crate::fair::bytes32(s).ok()
}

fn event(v: &Value) -> Result<Event, JsError> {
    Ok(match v["type"].as_str().unwrap_or("") {
        "cmd" => Event::Cmd {
            id: s(v, "id")?,
            cmd: player_command(&v["cmd"]),
            online: v["online"].as_bool(),
        },
        "disconnected" => Event::Disconnected { id: s(v, "id")? },
        "presence" => Event::Presence {
            id: s(v, "id")?,
            on: b(v, "on"),
        },
        "link" => Event::Link { ok: b(v, "ok") },
        "chips" => Event::Chips {
            seat: n(v, "seat")?,
            ok: b(v, "ok"),
        },
        "remove" => Event::Remove {
            seat: n(v, "seat")?,
        },
        "leaving" => Event::Leaving {
            id: s(v, "id")?,
            on: b(v, "on"),
        },
        "mustPlay" => Event::MustPlay {
            id: s(v, "id")?,
            on: b(v, "on"),
        },
        "open" => Event::Open { on: b(v, "on") },
        "state" => Event::State,
        "wake" => Event::Wake,
        other => return Err(JsError::new(&format!("no event {other}"))),
    })
}

fn player_command(c: &Value) -> PlayerCommand {
    let round = || c["round"].as_str().unwrap_or("").to_string();
    match c["type"].as_str() {
        Some("still_here") => PlayerCommand::StillHere,
        Some("fair_commit") => match bytes(&c["c"]) {
            Some(commitment) => PlayerCommand::FairCommit {
                round: round(),
                commitment,
            },
            None => PlayerCommand::Ignored,
        },
        Some("fair_reveal") => match (bytes(&c["value"]), bytes(&c["nonce"])) {
            (Some(value), Some(nonce)) => PlayerCommand::FairReveal {
                round: round(),
                reveal: Reveal { value, nonce },
            },
            _ => PlayerCommand::Ignored,
        },
        _ => match serde_json::from_value::<Command>(c.clone()) {
            Ok(cmd) => PlayerCommand::Table(cmd),
            Err(_) => PlayerCommand::Unreadable,
        },
    }
}

fn step_json(step: &Step) -> Result<Value, JsError> {
    let actions = step
        .actions
        .iter()
        .map(|a| {
            Ok(match a {
                Action::Engine(res) => json!({"type": "engine", "res": result_json(res)?}),
                Action::Send { to, update } => {
                    json!({"type": "send", "to": to, "update": update_json(update)})
                }
                Action::Active { id, at } => json!({"type": "active", "id": id, "at": at}),
                Action::Idle { id } => json!({"type": "idle", "id": id}),
            })
        })
        .collect::<Result<Vec<_>, JsError>>()?;
    Ok(
        json!({"actions": actions, "wake": step.wake, "stopped": step.stopped, "paused": step.paused}),
    )
}

fn to(x: &impl serde::Serialize) -> Result<Value, JsError> {
    serde_json::to_value(x).map_err(err)
}

fn result_json(r: &TableResult) -> Result<Value, JsError> {
    let out: Vec<Value> = r
        .out
        .iter()
        .map(|o| Ok(json!({"to": o.to, "data": to(&o.update)?})))
        .collect::<Result<_, JsError>>()?;
    let mut v = json!({
        "seq": r.seq,
        "state": to(&r.state)?,
        "botToAct": r.bot_to_act,
        "turnMsLeft": r.turn_ms_left,
        "playersIn": r.players_in,
        "chipRequests": to(&r.chip_requests)?,
        "players": to(&r.players)?,
        "departed": to(&r.departed)?,
        "refunds": to(&r.refunds)?,
        "out": out,
    });
    // Only from a new hand: the seed it was dealt from, or null.
    if let Some(seed) = r.seed {
        v["seed"] = seed.map(|s| Value::from(hex(&s))).unwrap_or(Value::Null);
    }
    Ok(v)
}

fn list_json(list: &[(String, Bytes32)]) -> Value {
    list.iter()
        .map(|(id, c)| json!({"id": id, "c": hex(c)}))
        .collect()
}

fn update_json(u: &RunnerUpdate) -> Value {
    match u {
        RunnerUpdate::FairCommit { round } => {
            json!({"type": "fair", "step": "commit", "round": round})
        }
        RunnerUpdate::FairReveal { round, list } => {
            json!({"type": "fair", "step": "reveal", "round": round, "list": list_json(list)})
        }
        RunnerUpdate::FairResult(FairResult {
            hand,
            game,
            dealt,
            index,
            seed,
            list,
            reveals,
        }) => json!({
            "type": "fair", "step": "result", "hand": hand, "game": game, "dealt": dealt, "index": index,
            "seed": hex(seed), "list": list_json(list),
            "reveals": reveals.iter().map(|(id, r)| json!({"id": id, "value": hex(&r.value), "nonce": hex(&r.nonce)})).collect::<Vec<_>>(),
        }),
        RunnerUpdate::IdleAsk { secs } => json!({"type": "idle", "secs": secs}),
        RunnerUpdate::IdleDone => json!({"type": "idle", "done": true}),
    }
}
