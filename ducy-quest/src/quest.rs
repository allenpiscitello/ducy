//! The quest itself (ducy-web #300): the bankroll, the world clock, the seat
//! at a table, rest, travel, the waitlists you're on, and runs.
//!
//! The clock runs [`QUEST_SPEED`] times real time while you're here, so a
//! game day is six real hours. Away from the game it stops, except to finish
//! a trip or a night's sleep ([`resume`]), and then for at most
//! [`AWAY_CAP_DAYS`] game days. Real time `now` is in milliseconds.
//!
//! A quest is the page's save: fields this crate doesn't know (kept by other
//! parts of the page) come back as they went in.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::backer::{Backing, Rep};
use crate::reads::ReadStats;
use crate::scenarios::{LedgerEntry, Message, Stake};
pub use crate::world::Limits;
use crate::world::{day_hour, js_round};

/// The starting bankroll: $1,000.
pub const START_BANKROLL: f64 = 100_000.0;
/// The goal: $1,000,000.
pub const GOAL: f64 = 100_000_000.0;
/// Game time runs this many times real time.
pub const QUEST_SPEED: f64 = 4.0;
/// Time away counts for at most this many game days.
pub const AWAY_CAP_DAYS: f64 = 7.0;
/// The smallest buy-in, in big blinds, unless a room sets its own.
pub const MIN_BUY_IN_BB: f64 = 40.0;
/// The largest buy-in, in big blinds, unless a room sets its own.
pub const MAX_BUY_IN_BB: f64 = 200.0;
/// The world starts on a Friday evening, when the rooms are busiest.
pub const START_MINUTES: f64 = 4.0 * 1440.0 + 18.0 * 60.0;
/// At a table the clock runs only while hands are being played: a minute
/// without one finishing and it stops.
pub const TABLE_IDLE_MS: f64 = 60_000.0;
/// A gap longer than this since the clock was last moved is time away.
pub const AWAY_MS: f64 = 2.0 * 60_000.0;
/// At most this long at the tables between sleeps.
pub const MAX_TABLE_MINUTES: f64 = 12.0 * 60.0;
/// A warning from this long at the tables.
pub const TIRED_MINUTES: f64 = 10.0 * 60.0;
/// A night's sleep, in game minutes.
pub const SLEEP_MINUTES: f64 = 6.0 * 60.0;
/// When your name is called you have this long to take the seat, from when
/// you see the call.
pub const CALL_MINUTES: f64 = 8.0;

const DAY: f64 = 1440.0;
const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// Where the clock was last anchored: real milliseconds and game minutes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Clock {
    /// Real time, in milliseconds.
    pub real: f64,
    /// Game time then, in minutes.
    pub game: f64,
}

/// Your seat at a table.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Seat {
    /// The table's id.
    pub table: String,
    /// Its room.
    pub room: String,
    /// Its blinds, `[sb, bb]` in hundredths.
    pub stakes: [f64; 2],
    /// `nlhe` or `plo4`.
    #[serde(default = "nlhe")]
    pub game: String,
    /// The stack in front of you.
    pub stack: f64,
    /// What you bought in for, top-ups included.
    pub bought: f64,
    /// Hands played.
    #[serde(default)]
    pub hands: f64,
    /// The session's biggest pot won.
    #[serde(default)]
    pub big_win: f64,
    /// The session's biggest pot lost (negative).
    #[serde(default)]
    pub big_loss: f64,
    /// The named Regulars at the table.
    #[serde(default)]
    pub with: Vec<String>,
    /// The game minute you sat down.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<f64>,
    /// Anything else the page keeps.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A trip under way.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Trip {
    /// The room you left.
    pub from: Option<String>,
    /// The room you're going to.
    pub to: String,
    /// Game minutes it started and ends.
    pub start: f64,
    /// When it's over.
    pub end: f64,
    /// `sleep` to go to bed on arrival.
    #[serde(default)]
    pub then: Option<String>,
}

/// A night's sleep.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rest {
    /// When you went to bed.
    pub start: f64,
    /// When you wake.
    pub end: f64,
    /// `home` or `hotel`.
    #[serde(rename = "where")]
    pub place: String,
}

/// Your name on a game's waitlist.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ListEntry {
    /// The list's key, `nlhe 1/2`.
    pub key: String,
    /// The stakes, `1/2`.
    pub stakes: String,
    /// The game.
    #[serde(default = "nlhe")]
    pub game: String,
    /// The room.
    pub room: String,
    /// When you put your name down.
    pub joined: f64,
    /// Game minutes between seats opening.
    pub gap: f64,
    /// When your name is called.
    pub call_at: f64,
    /// When you saw the call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen_at: Option<f64>,
    /// Anything else the page keeps.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A session at a table, once you've left it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Session {
    /// The game minute you left.
    pub at: f64,
    /// The room.
    #[serde(default)]
    pub room: Option<String>,
    /// The blinds.
    #[serde(default)]
    pub stakes: Option<[f64; 2]>,
    /// The game.
    #[serde(default = "nlhe")]
    pub game: String,
    /// What you took away less what you bought in for.
    pub net: f64,
    /// Hands played.
    #[serde(default)]
    pub hands: f64,
    /// The biggest pot won.
    #[serde(default)]
    pub big_win: f64,
    /// The biggest pot lost.
    #[serde(default)]
    pub big_loss: f64,
    /// The named Regulars at the table.
    #[serde(default)]
    pub with: Vec<String>,
    /// The bankroll after.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bankroll: Option<f64>,
    /// Anything else the page keeps.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A run that's ended.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunRecord {
    /// `broke`, `won` or `quit`.
    pub result: String,
    /// Its best bankroll.
    pub peak: f64,
    /// Its last.
    #[serde(rename = "final")]
    pub last: f64,
    /// Sessions played.
    pub sessions: f64,
    /// Hands played.
    #[serde(default)]
    pub hands: f64,
    /// Game minutes it ran.
    pub minutes: f64,
    /// Real time it ended.
    pub ended: f64,
}

/// Questions answered while travelling or waiting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Study {
    /// How many.
    pub n: f64,
    /// How many right.
    pub right: f64,
}

/// A Bankroll Quest: the page's save.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Quest {
    /// The save's version.
    pub v: u32,
    /// The seed the world is built from.
    pub seed: u64,
    /// The bankroll, in hundredths (not counting a stack at a table).
    pub bankroll: f64,
    /// The clock's anchor.
    pub clock: Clock,
    /// Your seat, if you're at a table.
    #[serde(default)]
    pub seat: Option<Seat>,
    /// The room you're in.
    #[serde(default)]
    pub room: Option<String>,
    /// A trip under way.
    #[serde(default)]
    pub trip: Option<Trip>,
    /// The waitlists you're on.
    #[serde(default, deserialize_with = "null_default")]
    pub lists: Vec<ListEntry>,
    /// The run's best bankroll.
    #[serde(default)]
    pub best: f64,
    /// The run's sessions, the newest 500.
    #[serde(default, deserialize_with = "null_default")]
    pub history: Vec<Session>,
    /// Past runs, the newest 20.
    #[serde(default, deserialize_with = "null_default")]
    pub runs: Vec<RunRecord>,
    /// Your home city.
    #[serde(default)]
    pub home_city: Option<String>,
    /// Game minutes at the tables since you last slept (the seat you're in not counted).
    #[serde(default, deserialize_with = "null_default")]
    pub table_minutes: f64,
    /// A night's sleep under way.
    #[serde(default)]
    pub rest: Option<Rest>,
    /// The region you put down roots in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,
    /// The game minute the run started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started: Option<f64>,
    /// Hands played this run, the seat you're in not counted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hands: Option<f64>,
    /// Milestones reached this run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestones: Option<Vec<f64>>,
    /// Study while you wait.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub study: Option<Study>,
    /// Your reads on the generic players, by id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reads: Option<Map<String, Value>>,
    /// How often your reads were right.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_stats: Option<ReadStats>,
    /// What you know about people (kept by the page).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub people: Option<Value>,
    /// The inbox, newest first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inbox: Option<Vec<Message>>,
    /// Messages posted so far.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inbox_n: Option<f64>,
    /// Players you've staked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stakes: Option<Vec<Stake>>,
    /// The money that changed hands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ledger: Option<Vec<LedgerEntry>>,
    /// Your name as a horse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rep: Option<Rep>,
    /// A backer's stake, while it runs. `Some(None)` is a `null` kept.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "kept_null")]
    pub backing: Option<Option<Backing>>,
    /// No backer comes until this game minute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backer_cooldown: Option<f64>,
    /// Anything else the page keeps.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn nlhe() -> String {
    "nlhe".to_string()
}

// A `null` reads as the default (the page's `x || []`).
fn null_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

// A field that may be missing, `null` or a value.
mod kept_null {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer, T: Serialize>(
        v: &Option<Option<T>>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        v.as_ref().and_then(|x| x.as_ref()).serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
        d: D,
    ) -> Result<Option<Option<T>>, D::Error> {
        Ok(Some(Option::<T>::deserialize(d)?))
    }
}

impl Quest {
    /// The backer's stake, if one runs.
    pub fn backing(&self) -> Option<&Backing> {
        self.backing.as_ref().and_then(|b| b.as_ref())
    }
    /// The bankroll with any stack at a table.
    pub fn total(&self) -> f64 {
        self.bankroll + self.seat.as_ref().map_or(0.0, |s| s.stack)
    }
}

/// A new quest: the starting bankroll, the clock anchored at real time
/// `now`, and the seed its world is built from. No room yet: a run starts by
/// choosing where to put down roots ([`start_in`]).
pub fn new_quest(now: f64, seed: u64) -> Quest {
    Quest {
        v: 1,
        seed,
        bankroll: START_BANKROLL,
        clock: Clock {
            real: now,
            game: START_MINUTES,
        },
        best: START_BANKROLL,
        ..Quest::default()
    }
}

/// Game minutes at real time `now`: the anchor plus [`QUEST_SPEED`] times the
/// real time since (at a table, at most [`TABLE_IDLE_MS`] of it), capped at
/// [`AWAY_CAP_DAYS`] game days.
pub fn game_minutes(q: &Quest, now: f64) -> f64 {
    let mut real = (now - q.clock.real).max(0.0);
    if q.seat.is_some() {
        real = real.min(TABLE_IDLE_MS);
    }
    q.clock.game + ((real / 60_000.0) * QUEST_SPEED).min(AWAY_CAP_DAYS * DAY)
}

/// Moves the anchor to `now`, so the cap counts from here.
pub fn sync_clock(q: &Quest, now: f64) -> Quest {
    Quest {
        clock: Clock {
            real: now,
            game: game_minutes(q, now),
        },
        ..q.clone()
    }
}

/// The game minute time away may run the clock to: the end of a trip (and
/// the sleep after it), a night, or the first call on your lists, else
/// nowhere.
pub fn away_until(q: &Quest) -> f64 {
    if let Some(t) = &q.trip {
        return t.end
            + if t.then.as_deref() == Some("sleep") {
                SLEEP_MINUTES
            } else {
                0.0
            };
    }
    if let Some(r) = &q.rest {
        return r.end;
    }
    let calls: Vec<f64> = q
        .lists
        .iter()
        .filter(|l| Some(&l.room) == q.room.as_ref() && l.call_at > q.clock.game)
        .map(|l| l.call_at)
        .collect();
    if q.seat.is_none() && !calls.is_empty() {
        return calls.into_iter().fold(f64::INFINITY, f64::min);
    }
    q.clock.game
}

/// Back after time away (or a hidden tab): the clock moved on only as far
/// as a trip, a night or a seat's wait needs.
pub fn resume(q: &Quest, now: f64) -> Quest {
    let away = now - q.clock.real;
    if away <= AWAY_MS {
        return sync_clock(q, now);
    }
    let ran = ((away / 60_000.0) * QUEST_SPEED).min(AWAY_CAP_DAYS * DAY);
    let game = q.clock.game + 0f64.max(ran.min(away_until(q) - q.clock.game));
    Quest {
        clock: Clock { real: now, game },
        ..q.clone()
    }
}

/// Spends game time without real time passing.
pub fn spend_minutes(q: &Quest, minutes: f64, now: f64) -> Quest {
    let s = sync_clock(q, now);
    let game = s.clock.game + minutes;
    Quest {
        clock: Clock { real: now, game },
        ..s
    }
}

/// The clock as shown: `Fri 9:40 pm`.
pub fn clock_text(minutes: f64) -> String {
    let (day, hour, minute) = day_hour(minutes);
    let h12 = if hour % 12 == 0 { 12 } else { hour % 12 };
    format!(
        "{} {h12}:{minute:02} {}",
        DAYS[day as usize],
        if hour < 12 { "am" } else { "pm" }
    )
}

/// The buy-in range at blinds `[sb, bb]`, in chips: `{min, max}`.
pub fn buy_in_range(stakes: [f64; 2], room: &Limits) -> (f64, f64) {
    let bb = stakes[1];
    (
        room.min_buy_in.unwrap_or(MIN_BUY_IN_BB) * bb,
        room.max_buy_in.unwrap_or(MAX_BUY_IN_BB) * bb,
    )
}

/// Whether the bankroll covers a seat at these stakes.
pub fn can_sit(q: &Quest, stakes: [f64; 2], room: &Limits) -> bool {
    q.bankroll >= buy_in_range(stakes, room).0
}

/// A table to sit at.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TableChoice {
    /// The table's id.
    pub id: String,
    /// Its room.
    pub room: String,
    /// Its blinds.
    pub stakes: [f64; 2],
    /// `nlhe` unless `plo4`.
    #[serde(default)]
    pub game: Option<String>,
    /// The named Regulars there.
    #[serde(default)]
    pub with: Option<Vec<String>>,
}

// Whether a list (stakes `1/2`) is for the game at blinds `blinds`.
fn same_game(list_game: &str, list_stakes: &str, game: &str, blinds: [f64; 2]) -> bool {
    if list_game != game {
        return false;
    }
    let parts: Vec<f64> = list_stakes
        .split('/')
        .map(|n| js_round(n.trim().parse::<f64>().unwrap_or(f64::NAN) * 100.0))
        .collect();
    parts.len() == 2 && parts[0] == blinds[0] && parts[1] == blinds[1]
}

/// Sits down at `table` with `amount` chips from the bankroll, kept within
/// the buy-in range. `None` if the bankroll can't cover the minimum, you're
/// already seated, asleep, on the road or played out.
pub fn sit_down(
    q: &Quest,
    table: &TableChoice,
    amount: f64,
    room: &Limits,
    now: f64,
) -> Option<Quest> {
    let (min, max) = buy_in_range(table.stakes, room);
    if q.seat.is_some()
        || q.bankroll < min
        || q.rest.is_some()
        || q.trip.is_some()
        || exhausted(q, now)
    {
        return None;
    }
    let chips = min.max(max.min(q.bankroll).min(js_round(amount)));
    let game = table.game.clone().unwrap_or_else(nlhe);
    // Sitting down takes your name off this game's list; you stay in line
    // for the others in the room while you play.
    let lists = q
        .lists
        .iter()
        .filter(|l| l.room == table.room && !same_game(&l.game, &l.stakes, &game, table.stakes))
        .cloned()
        .collect();
    Some(Quest {
        bankroll: q.bankroll - chips,
        room: Some(table.room.clone()),
        lists,
        seat: Some(Seat {
            table: table.id.clone(),
            room: table.room.clone(),
            stakes: table.stakes,
            game,
            stack: chips,
            bought: chips,
            with: table.with.clone().unwrap_or_default(),
            since: Some(game_minutes(q, now)),
            ..Seat::default()
        }),
        ..q.clone()
    })
}

/// The stack at the table (after a top-up, or when leaving).
pub fn set_stack(q: &Quest, stack: f64) -> Quest {
    let mut out = q.clone();
    if let Some(s) = &mut out.seat {
        s.stack = js_round(stack).max(0.0);
    }
    out
}

/// A hand played: the stack after it, the hand counted, and its result kept
/// if it's the session's biggest pot won or lost.
pub fn record_hand(q: &Quest, stack: f64) -> Quest {
    let mut out = q.clone();
    if let Some(s) = &mut out.seat {
        let after = js_round(stack).max(0.0);
        let net = after - s.stack;
        s.stack = after;
        s.hands += 1.0;
        s.big_win = s.big_win.max(net);
        s.big_loss = s.big_loss.min(net);
    }
    out
}

/// Tops up between hands to `to` chips (at most the maximum buy-in), from
/// the bankroll as far as it goes.
pub fn top_up(q: &Quest, to: f64, room: &Limits) -> Quest {
    let mut out = q.clone();
    if let Some(s) = &mut out.seat {
        let (_, max) = buy_in_range(s.stakes, room);
        let add = 0f64.max(js_round(to).min(max) - s.stack);
        let paid = add.min(q.bankroll);
        out.bankroll -= paid;
        s.stack += paid;
        s.bought += paid;
    }
    out
}

/// A stack you can top up to.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct TopUpChoice {
    /// In big blinds.
    pub bbs: f64,
    /// The stack after it.
    pub to: f64,
    /// The bankroll can't pay the whole way and this takes all of it.
    pub all: bool,
}

/// The stacks you can top up to from `stack` chips: 40, 100 and 200 big
/// blinds and the room's maximum buy-in, as far as the bankroll pays.
pub fn top_up_choices(q: &Quest, stack: f64, room: &Limits) -> Vec<TopUpChoice> {
    let Some(s) = &q.seat else { return Vec::new() };
    let bb = s.stakes[1];
    let (_, max) = buy_in_range(s.stakes, room);
    let mut levels: Vec<f64> = [40.0, 100.0, 200.0]
        .iter()
        .map(|b| b * bb)
        .filter(|x| *x <= max)
        .collect();
    levels.push(max);
    levels.sort_by(f64::total_cmp);
    levels.dedup();
    let mut out: Vec<TopUpChoice> = Vec::new();
    for level in levels {
        let to = level.min(stack + q.bankroll);
        if to <= stack || out.iter().any(|o| o.to == to) {
            continue;
        }
        out.push(TopUpChoice {
            bbs: js_round(level / bb),
            to,
            all: to < level,
        });
    }
    out
}

/// Leaves the table: the stack goes back to the bankroll, and the session
/// goes into the history.
pub fn leave(q: &Quest, now: f64) -> Quest {
    let Some(s) = &q.seat else { return q.clone() };
    let bankroll = q.bankroll + s.stack;
    let at = game_minutes(q, now);
    let session = Session {
        at: at.floor(),
        room: Some(s.room.clone()),
        stakes: Some(s.stakes),
        game: s.game.clone(),
        net: s.stack - s.bought,
        hands: s.hands,
        big_win: s.big_win,
        big_loss: s.big_loss,
        with: s.with.clone(),
        bankroll: Some(bankroll),
        extra: Map::new(),
    };
    let mut history = q.history.clone();
    history.push(session);
    if history.len() > 500 {
        history.drain(..history.len() - 500);
    }
    Quest {
        bankroll,
        seat: None,
        best: q.best.max(bankroll),
        hands: Some(q.hands.unwrap_or(0.0) + s.hands),
        history,
        table_minutes: table_minutes(q, now),
        // The clock as it stood at the table (stopped if play had), from now on.
        clock: Clock {
            real: now,
            game: at,
        },
        ..q.clone()
    }
}

/// Game minutes at the tables since you last slept, the seat you're in
/// included.
pub fn table_minutes(q: &Quest, now: f64) -> f64 {
    let sat = q
        .seat
        .as_ref()
        .and_then(|s| s.since)
        .map_or(0.0, |since| (game_minutes(q, now) - since).max(0.0));
    q.table_minutes + sat
}

/// Played out: no more tables until you've slept.
pub fn exhausted(q: &Quest, now: f64) -> bool {
    table_minutes(q, now) >= MAX_TABLE_MINUTES
}

/// Goes to sleep where you are, `place` `home` or `hotel`, paying `cost`.
/// Not while seated, on the road or asleep, or if the bankroll can't pay.
/// Your places on lists are given up.
pub fn sleep(q: &Quest, place: &str, cost: f64, now: f64) -> Quest {
    if q.seat.is_some() || q.trip.is_some() || q.rest.is_some() || q.bankroll < cost {
        return q.clone();
    }
    let at = game_minutes(q, now);
    Quest {
        bankroll: q.bankroll - cost,
        lists: Vec::new(),
        rest: Some(Rest {
            start: at,
            end: at + SLEEP_MINUTES,
            place: place.to_string(),
        }),
        ..q.clone()
    }
}

/// Awake again, if the night's over: the hours at the tables start from zero.
pub fn wake(q: &Quest, now: f64) -> Quest {
    match &q.rest {
        Some(r) if game_minutes(q, now) >= r.end => Quest {
            rest: None,
            table_minutes: 0.0,
            ..q.clone()
        },
        _ => q.clone(),
    }
}

/// Makes city `city_id` home, for `cost` from the bankroll.
pub fn move_home(q: &Quest, city_id: &str, cost: f64) -> Quest {
    if q.seat.is_some()
        || q.trip.is_some()
        || q.rest.is_some()
        || city_id.is_empty()
        || q.home_city.as_deref() == Some(city_id)
        || q.bankroll < cost
    {
        return q.clone();
    }
    Quest {
        bankroll: q.bankroll - cost,
        home_city: Some(city_id.to_string()),
        ..q.clone()
    }
}

/// A milestone on the way to [`GOAL`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Milestone {
    /// The bankroll.
    pub at: f64,
    /// What's said.
    pub text: &'static str,
}

/// Milestones on the way to [`GOAL`], each announced once a run.
pub const MILESTONES: [Milestone; 5] = [
    Milestone {
        at: 500_000.0,
        text: "$5,000: five times what you started with.",
    },
    Milestone {
        at: 2_500_000.0,
        text: "$25,000: the big games in the Ivory Room are within reach.",
    },
    Milestone {
        at: 10_000_000.0,
        text: "$100,000: six figures.",
    },
    Milestone {
        at: 25_000_000.0,
        text: "$250,000: a quarter of a million.",
    },
    Milestone {
        at: 50_000_000.0,
        text: "$500,000: halfway there, in dollars.",
    },
];

/// Milestones the bankroll (with any stack at a table) has just reached:
/// the quest with them marked done, and the new ones in order.
pub fn check_milestones(q: &Quest) -> (Quest, Vec<Milestone>) {
    let total = q.total();
    let done = q.milestones.clone().unwrap_or_default();
    let reached: Vec<Milestone> = MILESTONES
        .iter()
        .filter(|m| total >= m.at && !done.contains(&m.at))
        .copied()
        .collect();
    if reached.is_empty() {
        return (q.clone(), reached);
    }
    let mut all = Vec::new();
    for m in done.into_iter().chain(reached.iter().map(|m| m.at)) {
        if !all.contains(&m) {
            all.push(m);
        }
    }
    (
        Quest {
            milestones: Some(all),
            ..q.clone()
        },
        reached,
    )
}

/// The quest's state: `won` (the bankroll, with any stack at the table,
/// reached [`GOAL`]), `broke` (can't cover any seat, `cheapest` the smallest
/// minimum buy-in in the world, and no chips at a table) or `playing`.
pub fn status(q: &Quest, cheapest: f64) -> &'static str {
    if q.total() >= GOAL {
        "won"
    } else if q.seat.is_none() && q.bankroll < cheapest {
        "broke"
    } else {
        "playing"
    }
}

/// Starts a run in region `region_id`'s room `room_id`: where you put down
/// roots. Only before you're anywhere.
pub fn start_in(q: &Quest, region_id: &str, room_id: &str, city_id: Option<&str>) -> Quest {
    if q.room.is_some() || q.trip.is_some() {
        return q.clone();
    }
    Quest {
        home: Some(region_id.to_string()),
        room: Some(room_id.to_string()),
        home_city: city_id.map(str::to_string),
        ..q.clone()
    }
}

/// Sets off for room `room_id`, a trip of `minutes` game minutes, paying
/// `cost`; `then` `sleep` goes to bed on arrival. Your places on waitlists
/// are given up. Not while seated, asleep, already on the road, to the room
/// you're in, or if the bankroll can't pay.
pub fn travel(
    q: &Quest,
    room_id: &str,
    minutes: f64,
    now: f64,
    cost: f64,
    then: Option<&str>,
) -> Quest {
    if q.seat.is_some()
        || q.trip.is_some()
        || q.rest.is_some()
        || q.room.as_deref() == Some(room_id)
        || q.bankroll < cost
    {
        return q.clone();
    }
    let at = game_minutes(q, now);
    Quest {
        bankroll: q.bankroll - cost,
        trip: Some(Trip {
            from: q.room.clone(),
            to: room_id.to_string(),
            start: at,
            end: at + minutes,
            then: then.map(str::to_string),
        }),
        lists: Vec::new(),
        ..q.clone()
    }
}

/// The trip over, if it is: you're in the room you were going to (asleep,
/// if you went home to rest, and maybe awake again if that was long ago).
pub fn arrive(q: &Quest, now: f64) -> Quest {
    let Some(t) = &q.trip else { return q.clone() };
    if game_minutes(q, now) < t.end {
        return q.clone();
    }
    let there = Quest {
        room: Some(t.to.clone()),
        trip: None,
        ..q.clone()
    };
    if t.then.as_deref() != Some("sleep") {
        return there;
    }
    let rest = Rest {
        start: t.end,
        end: t.end + SLEEP_MINUTES,
        place: "home".to_string(),
    };
    wake(
        &Quest {
            rest: Some(rest),
            ..there
        },
        now,
    )
}

/// A game to put your name down for.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ListGame {
    /// The list's key.
    pub key: String,
    /// The stakes, `1/2`.
    pub stakes: String,
    /// The game.
    #[serde(default)]
    pub game: Option<String>,
}

/// Puts your name on a game's list in this room: `ahead` the players
/// waiting before you and `gap` the game minutes between seats opening.
/// Also from a table, for any other game in the room.
pub fn join_list(q: &Quest, game: &ListGame, ahead: f64, gap: f64, now: f64) -> Quest {
    let g = game.game.clone().unwrap_or_else(nlhe);
    let playing = q
        .seat
        .as_ref()
        .is_some_and(|s| same_game(&g, &game.stakes, &s.game, s.stakes));
    if playing
        || q.trip.is_some()
        || q.rest.is_some()
        || exhausted(q, now)
        || q.room.is_none()
        || q.lists.iter().any(|l| l.key == game.key)
    {
        return q.clone();
    }
    let at = game_minutes(q, now);
    let mut out = q.clone();
    out.lists.push(ListEntry {
        key: game.key.clone(),
        stakes: game.stakes.clone(),
        game: g,
        room: q.room.clone().unwrap_or_default(),
        joined: at,
        gap,
        call_at: at + (ahead + 1.0) * gap,
        ..ListEntry::default()
    });
    out
}

/// Takes your name off a game's list.
pub fn leave_list(q: &Quest, key: &str) -> Quest {
    Quest {
        lists: q.lists.iter().filter(|l| l.key != key).cloned().collect(),
        ..q.clone()
    }
}

// A call you haven't seen holds your seat: the minutes to take it start
// once the lobby shows it to you.
fn expires_at(l: &ListEntry) -> f64 {
    l.seen_at.unwrap_or(f64::INFINITY) + CALL_MINUTES
}

fn here(q: &Quest, l: &ListEntry) -> bool {
    Some(&l.room) == q.room.as_ref()
}

/// Where you stand on a list.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListStatus {
    /// The entry.
    #[serde(flatten)]
    pub entry: ListEntry,
    /// Your seat is ready.
    pub called: bool,
    /// The game minute the call runs out (infinite until you've seen it).
    pub expires: f64,
    /// The players still before you.
    pub ahead: f64,
    /// Game minutes until you're called, or, called, left to take the seat.
    pub wait: f64,
}

/// Where you stand on your lists at real time `now`. Lists in other rooms,
/// and seen calls not taken in time, drop off.
pub fn list_status(q: &Quest, now: f64) -> Vec<ListStatus> {
    let at = game_minutes(q, now);
    q.lists
        .iter()
        .filter(|l| here(q, l) && at < expires_at(l))
        .map(|l| {
            let called = at >= l.call_at;
            let expires = expires_at(l);
            ListStatus {
                entry: l.clone(),
                called,
                expires,
                ahead: if called {
                    0.0
                } else {
                    0f64.max(((l.call_at - at) / l.gap).ceil() - 1.0)
                },
                wait: if !called {
                    l.call_at - at
                } else if expires.is_finite() {
                    expires - at
                } else {
                    CALL_MINUTES
                },
            }
        })
        .collect()
}

/// Calls that ran out (seen, and not taken in time): the lists without them.
pub fn drop_missed(q: &Quest, now: f64) -> Quest {
    let at = game_minutes(q, now);
    let kept: Vec<ListEntry> = q
        .lists
        .iter()
        .filter(|l| here(q, l) && at < expires_at(l))
        .cloned()
        .collect();
    if kept.len() == q.lists.len() {
        return q.clone();
    }
    Quest {
        lists: kept,
        ..q.clone()
    }
}

fn unseen(q: &Quest, l: &ListEntry, at: f64) -> bool {
    here(q, l) && at >= l.call_at && l.seen_at.is_none()
}

/// The lobby is showing your calls: their minutes to take the seat start now.
pub fn see_calls(q: &Quest, now: f64) -> Quest {
    let at = game_minutes(q, now);
    let mut out = q.clone();
    for l in &mut out.lists {
        if unseen(q, l, at) {
            l.seen_at = Some(at);
        }
    }
    out
}

/// Calls you haven't seen yet.
pub fn unseen_calls(q: &Quest, now: f64) -> Vec<ListEntry> {
    let at = game_minutes(q, now);
    q.lists
        .iter()
        .filter(|l| unseen(q, l, at))
        .cloned()
        .collect()
}

/// A study question answered.
pub fn record_study(q: &Quest, right: bool) -> Quest {
    let s = q.study.unwrap_or_default();
    Quest {
        study: Some(Study {
            n: s.n + 1.0,
            right: s.right + if right { 1.0 } else { 0.0 },
        }),
        ..q.clone()
    }
}

/// The record of a run that's ended: how (`broke`, `won`, or `quit`), its
/// peak and final bankroll, sessions and how long it ran.
pub fn run_record(q: &Quest, now: f64, result: &str) -> RunRecord {
    RunRecord {
        result: result.to_string(),
        peak: q.best,
        last: q.bankroll,
        sessions: q.history.len() as f64,
        hands: q.hands.unwrap_or(0.0),
        minutes: (game_minutes(q, now) - q.started.unwrap_or(START_MINUTES)).floor(),
        ended: now,
    }
}

/// A run that's over starts again at $1,000 in the same world, on the same
/// clock, with the past runs kept (the newest 20). What you know about the
/// players carries over, and so do the scenarios: your inbox, stakes still
/// out, your name as a horse and a backer still owed.
pub fn new_run(q: &Quest, now: f64, result: &str) -> Quest {
    let fresh = new_quest(now, q.seed);
    let clock = sync_clock(q, now).clock;
    // You start again where you are; after a trip, where it was going.
    let room = match &q.trip {
        Some(t) => Some(t.to.clone()),
        None => q.room.clone(),
    };
    let mut runs = q.runs.clone();
    runs.push(run_record(q, now, result));
    if runs.len() > 20 {
        runs.drain(..runs.len() - 20);
    }
    Quest {
        clock,
        started: Some(clock.game),
        room,
        home: q.home.clone(),
        home_city: q.home_city.clone(),
        runs,
        reads: Some(q.reads.clone().unwrap_or_default()),
        read_stats: q.read_stats,
        people: Some(
            q.people
                .clone()
                .unwrap_or_else(|| Value::Object(Map::new())),
        ),
        study: q.study,
        inbox: Some(q.inbox.clone().unwrap_or_default()),
        inbox_n: Some(q.inbox_n.unwrap_or(0.0)),
        stakes: Some(q.stakes.clone().unwrap_or_default()),
        ledger: Some(q.ledger.clone().unwrap_or_default()),
        rep: q.rep.clone(),
        backing: Some(q.backing().cloned()),
        backer_cooldown: q.backer_cooldown,
        ..fresh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: f64 = 60_000.0;

    #[test]
    fn the_clock_runs_four_times_real_time() {
        let q = new_quest(0.0, 7);
        let start = game_minutes(&q, 0.0);
        assert_eq!(game_minutes(&q, 15.0 * MIN) - start, 60.0);
        assert_eq!(
            game_minutes(&q, 30.0 * 24.0 * 60.0 * MIN) - start,
            AWAY_CAP_DAYS * 1440.0
        );
        assert_eq!(clock_text(4.0 * 1440.0 + 21.0 * 60.0 + 40.0), "Fri 9:40 pm");
        assert_eq!(clock_text(0.0), "Mon 12:00 am");
        assert_eq!(clock_text(game_minutes(&q, 0.0)), "Fri 6:00 pm");
    }

    #[test]
    fn a_session_goes_into_the_history() {
        let q = start_in(&new_quest(0.0, 1), "west", "deuce", None);
        let t = TableChoice {
            id: "deuce-1/2-1".into(),
            room: "deuce".into(),
            stakes: [100.0, 200.0],
            ..TableChoice::default()
        };
        let s = sit_down(&q, &t, 20_000.0, &Limits::default(), 0.0).unwrap();
        assert_eq!(s.bankroll, 80_000.0);
        let s = record_hand(&s, 25_000.0);
        let l = leave(&s, MIN);
        assert_eq!(l.bankroll, 105_000.0);
        assert_eq!(l.history[0].net, 5_000.0);
        assert_eq!(l.history[0].big_win, 5_000.0);
        assert_eq!(l.best, 105_000.0);
        assert!(
            sit_down(
                &l,
                &t,
                1.0,
                &Limits {
                    min_buy_in: Some(1000.0),
                    max_buy_in: None
                },
                MIN
            )
            .is_none()
        );
    }

    #[test]
    fn a_save_reads_back_with_what_the_page_keeps() {
        let json = r#"{"v":1,"seed":5,"bankroll":100,"clock":{"real":0,"game":10},"seat":null,"room":null,"trip":null,"lists":[],"best":100,"history":[],"runs":[],"homeCity":null,"tableMinutes":0,"rest":null,"backerTest":{"x":1},"backing":null}"#;
        let q: Quest = serde_json::from_str(json).unwrap();
        assert_eq!(q.extra["backerTest"], serde_json::json!({"x": 1}));
        assert_eq!(q.backing, Some(None));
        let back = serde_json::to_value(&q).unwrap();
        assert_eq!(back["backerTest"], serde_json::json!({"x": 1}));
        assert!(back["backing"].is_null() && back.get("backing").is_some());
        assert!(back.get("started").is_none());
    }
}
