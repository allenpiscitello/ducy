//! Tournaments for people (allenpiscitello/ducy#142): `ClubTournament` runs
//! every table of a tournament for remote players, as `MultiTable` runs one
//! table (ducy-play's `TournamentHost` over `Tournament`). Each player gets
//! their own seat's view and acts with the same commands as at a club
//! table; the host watches any table; every table deals securely; and each
//! call says who was knocked out (with their places), who moved where, and
//! when the level changed.

use ducy_play::tourney_runner::{
    BOT_PACE_MS, NEXT_HAND_MS, StructureStep, TableInfo, TourneyAction, TourneyConfig,
    TourneyEvent, TourneyResult, TourneyRunner,
};
use ducy_play::{Command, Level, Outgoing, TournamentConfig, TournamentHost, Update};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

use crate::play::rules_for;

fn err(e: impl std::fmt::Debug) -> JsError {
    JsError::new(&format!("{e:?}"))
}

fn to_js(value: &impl Serialize) -> Result<JsValue, JsError> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(err)
}

#[derive(Deserialize)]
struct LevelIn {
    sb: u64,
    bb: u64,
    #[serde(default)]
    ante: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfigIn {
    #[serde(default)]
    game: Option<String>,
    table_size: usize,
    starting_stack: u64,
    levels: Vec<LevelIn>,
    #[serde(default)]
    paid: usize,
    #[serde(default)]
    seed: u64,
}

#[derive(Deserialize)]
struct PlayerIn {
    id: String,
    name: String,
}

#[derive(Serialize)]
struct Message<'a> {
    to: &'a str,
    data: &'a Update,
}

/// What every call returns: the messages to send each player, what happened
/// (`events`: busted with places, moves, broken tables, winner, level), the
/// standings and tables, and what to do next (`autoToAct`: call `advance`;
/// `canDeal`: call `newHands`).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Result_<'a> {
    seq: u64,
    out: Vec<Message<'a>>,
    events: &'a ducy_play::TournamentEvents,
    standings: &'a [ducy_play::Standing],
    tables: &'a [TableInfo],
    level: usize,
    players_left: usize,
    total_chips: u64,
    hand_for_hand: bool,
    over: bool,
    auto_to_act: bool,
    can_deal: bool,
}

impl<'a> Result_<'a> {
    fn of(r: &'a TourneyResult) -> Self {
        Result_ {
            seq: r.seq,
            out: r
                .out
                .iter()
                .map(|o| Message {
                    to: &o.to,
                    data: &o.update,
                })
                .collect(),
            events: &r.events,
            standings: &r.standings,
            tables: &r.tables,
            level: r.level,
            players_left: r.players_left,
            total_chips: r.total_chips,
            hand_for_hand: r.hand_for_hand,
            over: r.over,
            auto_to_act: r.auto_to_act,
            can_deal: r.can_deal,
        }
    }
}

/// A tournament's host from `config` and `players`, as for `ClubTournament`'s constructor.
fn new_host(config: JsValue, players: JsValue, turn_ms: f64) -> Result<TournamentHost, JsError> {
    let c: ConfigIn = serde_wasm_bindgen::from_value(config).map_err(err)?;
    let players: Vec<PlayerIn> = serde_wasm_bindgen::from_value(players).map_err(err)?;
    let first = c
        .levels
        .first()
        .ok_or_else(|| JsError::new("no blind levels"))?;
    let rules = rules_for(c.game.as_deref(), first.sb, first.bb)?;
    let config = TournamentConfig {
        variant: rules.variant,
        structure: rules.structure,
        table_size: c.table_size,
        starting_stack: c.starting_stack,
        levels: c
            .levels
            .iter()
            .map(|l| Level::new(l.sb, l.bb, l.ante))
            .collect(),
        paid: c.paid,
        seed: c.seed,
    };
    let players = players.into_iter().map(|p| (p.id, p.name)).collect();
    TournamentHost::new(config, players, turn_ms as u64).map_err(err)
}

/// A tournament for people, every table run here.
#[wasm_bindgen]
pub struct ClubTournament {
    host: TournamentHost,
}

#[wasm_bindgen]
impl ClubTournament {
    /// A tournament: `config` = {game ('nlhe', 'plo4'…), tableSize,
    /// startingStack, levels: [{sb, bb, ante}], paid, seed}, `players` =
    /// [{id, name}] seated at random, `turnMs` to act (0: no clock).
    /// Everyone is absent (blinded off) until they join.
    #[wasm_bindgen(constructor)]
    pub fn new(config: JsValue, players: JsValue, turn_ms: f64) -> Result<ClubTournament, JsError> {
        Ok(ClubTournament {
            host: new_host(config, players, turn_ms)?,
        })
    }

    fn result(&mut self, out: &[Outgoing], now: u64) -> Result<JsValue, JsError> {
        let r = TourneyResult::of(&mut self.host, out.to_vec(), now);
        to_js(&Result_::of(&r))
    }

    /// The whole tournament as JSON, hands in play and clocks included, for
    /// `restore` after the host restarts. It holds every card of every hand
    /// in play: keep it where only the host can read it.
    pub fn save(&self, now: f64) -> Result<String, JsError> {
        serde_json::to_string(&self.host.snapshot(now as u64)).map_err(err)
    }

    /// Loads a tournament saved with `save`: it carries on where it was,
    /// each turn with the time it had left. No one is connected; people are
    /// back when they join again. Fails for a snapshot of another version.
    pub fn restore(json: &str, now: f64) -> Result<ClubTournament, JsError> {
        let s: ducy_play::TournamentHostSnapshot = serde_json::from_str(json).map_err(err)?;
        let host = TournamentHost::restore(&s, now as u64, |_| None).map_err(err)?;
        Ok(ClubTournament { host })
    }

    /// Where things stand, with no messages.
    pub fn state(&mut self, now: f64) -> Result<JsValue, JsError> {
        self.result(&[], now as u64)
    }

    /// A message from player `client` (a command object, as at a club
    /// table): join (here, or back), act, sit_out / leave (blinded off),
    /// sit_in.
    pub fn handle(&mut self, client: &str, command: JsValue, now: f64) -> Result<JsValue, JsError> {
        let now = now as u64;
        let out = match serde_wasm_bindgen::from_value::<Command>(command) {
            Ok(c) => self.host.handle(client, c, now),
            Err(_) => vec![Outgoing {
                to: client.to_string(),
                update: Update::Rejected {
                    reason: "bad message".to_string(),
                },
            }],
        };
        self.result(&out, now)
    }

    /// Player `client`'s connection dropped: blinded off until they're back.
    pub fn disconnected(&mut self, client: &str, now: f64) -> Result<JsValue, JsError> {
        let out = self.host.disconnected(client, now as u64);
        self.result(&out, now as u64)
    }

    /// Absent players check or fold where it's their turn.
    pub fn advance(&mut self, now: f64) -> Result<JsValue, JsError> {
        let out = self.host.advance(now as u64).map_err(err)?;
        self.result(&out, now as u64)
    }

    /// Runs the turn clocks; call it every half second or so.
    pub fn tick(&mut self, now: f64) -> Result<JsValue, JsError> {
        let out = self.host.tick(now as u64);
        self.result(&out, now as u64)
    }

    /// Deals the next hand at every table that may (between hands, two
    /// players, hand-for-hand caught up).
    #[wasm_bindgen(js_name = newHands)]
    pub fn new_hands(&mut self, now: f64) -> Result<JsValue, JsError> {
        let out = self.host.new_hands(now as u64).map_err(err)?;
        self.result(&out, now as u64)
    }

    /// Moves to blind level `level` (from 0); tables use it from their next hand.
    #[wasm_bindgen(js_name = setLevel)]
    pub fn set_level(&mut self, level: usize, now: f64) -> Result<JsValue, JsError> {
        self.host.set_level(level);
        self.result(&[], now as u64)
    }

    /// A late registration or re-entry: a starting stack at the shortest table.
    #[wasm_bindgen(js_name = addEntry)]
    pub fn add_entry(&mut self, id: &str, name: &str, now: f64) -> Result<JsValue, JsError> {
        let out = self.host.add_entry(id, name, now as u64).map_err(err)?;
        self.result(&out, now as u64)
    }

    /// The host's view of table `id`: a spectator's, no one's cards until
    /// they're shown down. Null for a table that's been broken.
    #[wasm_bindgen(js_name = tableView)]
    pub fn table_view(&self, id: usize) -> Result<JsValue, JsError> {
        match self.host.table_view(id) {
            Some(v) => to_js(&v),
            None => Ok(JsValue::NULL),
        }
    }

    /// Player `id`'s view of their table now, or null once they're out.
    #[wasm_bindgen(js_name = viewFor)]
    pub fn view_for(&self, id: &str) -> Result<JsValue, JsError> {
        match self.host.view_for(id) {
            Some(v) => to_js(&v),
            None => Ok(JsValue::NULL),
        }
    }
}

/// The runner's options from club-tourney-host.js.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunnerIn {
    #[serde(default)]
    structure: Vec<StepIn>,
    #[serde(default)]
    level_ms: Option<f64>,
    #[serde(default)]
    next_hand_ms: Option<f64>,
    #[serde(default)]
    bot_pace_ms: Option<f64>,
    #[serde(default)]
    step: usize,
    #[serde(default)]
    step_left_ms: Option<f64>,
    #[serde(default)]
    level: usize,
    #[serde(default)]
    hands: bool,
}

#[derive(Deserialize)]
struct StepIn {
    #[serde(default)]
    minutes: f64,
    #[serde(default, rename = "break")]
    is_break: bool,
}

impl RunnerIn {
    fn config(self, id: String) -> TourneyConfig {
        let ms = |x: Option<f64>| x.map(|v| v.max(0.0) as u64);
        TourneyConfig {
            id,
            structure: self
                .structure
                .iter()
                .map(|s| StructureStep {
                    minutes: s.minutes.max(0.0) as u64,
                    is_break: s.is_break,
                })
                .collect(),
            level_ms: ms(self.level_ms),
            next_hand_ms: ms(self.next_hand_ms).unwrap_or(NEXT_HAND_MS),
            bot_pace_ms: ms(self.bot_pace_ms).unwrap_or(BOT_PACE_MS),
            step: self.step,
            step_left_ms: ms(self.step_left_ms),
            level: self.level,
            hands: self.hands,
        }
    }
}

/// A club tournament run by its runner (ducy-play's `tourney_runner`,
/// ducy-web #458), for ducy-web's club-engine.js, with events and steps as
/// plain JS objects. Every call takes the time in milliseconds.
#[wasm_bindgen]
pub struct ClubTourney {
    runner: TourneyRunner,
}

#[wasm_bindgen]
impl ClubTourney {
    /// A new tournament `id`: `config`, `players` and `turnMs` as for
    /// `ClubTournament`, and `runner` = {structure: [{minutes, break}],
    /// levelMs, nextHandMs, botPaceMs, hands}.
    pub fn create(
        id: String,
        config: JsValue,
        players: JsValue,
        turn_ms: f64,
        runner: JsValue,
    ) -> Result<ClubTourney, JsError> {
        let r: RunnerIn = serde_wasm_bindgen::from_value(runner).map_err(err)?;
        let host = new_host(config, players, turn_ms)?;
        Ok(ClubTourney {
            runner: TourneyRunner::new(host, r.config(id)),
        })
    }

    /// A tournament saved with `save`, its clock where `runner` says (step,
    /// stepLeftMs, level, as well as `create`'s). Throws if it can't be loaded.
    pub fn restore(
        id: String,
        json: &str,
        runner: JsValue,
        now: f64,
    ) -> Result<ClubTourney, JsError> {
        let r: RunnerIn = serde_wasm_bindgen::from_value(runner).map_err(err)?;
        let s: ducy_play::TournamentHostSnapshot = serde_json::from_str(json).map_err(err)?;
        let host = TournamentHost::restore(&s, now as u64, |_| None).map_err(err)?;
        Ok(ClubTourney {
            runner: TourneyRunner::new(host, r.config(id)),
        })
    }

    /// Takes in an event ({type, …}, the runner's `TourneyEvent`) and returns the
    /// step: {actions, wake, step, stepEndsAt, onBreak, level}.
    pub fn event(&mut self, ev: JsValue, now: f64) -> Result<JsValue, JsError> {
        let v: Value = serde_wasm_bindgen::from_value(ev).map_err(err)?;
        let s = |k: &str| {
            v[k].as_str()
                .map(str::to_string)
                .ok_or_else(|| JsError::new(&format!("event: no {k}")))
        };
        let ev = match v["type"].as_str().unwrap_or("") {
            "begin" => TourneyEvent::Begin,
            "cmd" => TourneyEvent::Cmd {
                id: s("id")?,
                cmd: serde_json::from_value::<Command>(v["cmd"].clone()).ok(),
            },
            "disconnected" => TourneyEvent::Disconnected { id: s("id")? },
            "addEntry" => TourneyEvent::AddEntry {
                id: s("id")?,
                name: s("name")?,
            },
            "state" => TourneyEvent::State,
            "wake" => TourneyEvent::Wake,
            other => return Err(JsError::new(&format!("no event {other}"))),
        };
        let step = self.runner.event(ev, now as u64).map_err(err)?;
        let actions = step
            .actions
            .iter()
            .map(|a| {
                Ok(match a {
                    TourneyAction::Engine(res) => json!({"type": "engine", "res": serde_json::to_value(Result_::of(res)).map_err(err)?}),
                    TourneyAction::Hand { table, view } => json!({"type": "hand", "table": table, "view": serde_json::to_value(view).map_err(err)?}),
                })
            })
            .collect::<Result<Vec<Value>, JsError>>()?;
        to_js(&json!({
            "actions": actions, "wake": step.wake, "step": step.step, "stepEndsAt": step.step_ends_at,
            "onBreak": step.on_break, "level": step.level,
        }))
    }

    /// The tournament as JSON, to `restore` after a restart.
    pub fn save(&self, now: f64) -> Result<String, JsError> {
        serde_json::to_string(&self.runner.host().snapshot(now as u64)).map_err(err)
    }

    /// The host's view of table `id` (no one's cards until shown down), or
    /// null for a table that's been broken.
    #[wasm_bindgen(js_name = tableView)]
    pub fn table_view(&self, id: usize) -> Result<JsValue, JsError> {
        match self.runner.host().table_view(id) {
            Some(v) => to_js(&v),
            None => Ok(JsValue::NULL),
        }
    }
}
