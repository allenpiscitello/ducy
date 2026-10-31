//! A club's own rules (ducy-web #455): its members and their names, the
//! invite, help requests, the lobby chat, the club info, and its tables
//! with their reserved seats.
//!
//! [`Club`] is the part of ducy-web's club object these rules keep, with the
//! same fields and JSON, so a club saved before them loads as it is. The
//! fields other parts keep on a member (their chips, their seat at a table,
//! an agent's settings) ride along in [`Member::other`] untouched, and the
//! rules read the seat from there. The money is
//! [`club_money`](crate::club_money)'s.
//!
//! Every operation changes the club in place and returns what to tell
//! people; refusals are [`RuleError`]s, with nothing changed. Ids and invite
//! tokens come from the caller's random numbers.

use std::collections::BTreeMap;
use std::fmt;

use rand::RngExt;
use rand::rngs::StdRng;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value, json};
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// The longest club name, in characters.
pub const MAX_CLUB_NAME: usize = 50;
/// The longest player name, in characters.
pub const MAX_PLAYER_NAME: usize = 20;
/// How many help requests the club keeps.
pub const MAX_HELP: usize = 50;
/// The longest help request, in characters.
pub const MAX_HELP_TEXT: usize = 200;
/// The longest reply to one, in characters.
pub const MAX_HELP_REPLY: usize = 200;
/// How many of their own help requests a member sees.
pub const HELP_MINE: usize = 3;
/// How many lobby lines the club keeps.
pub const LOBBY_KEEP: usize = 50;
/// How long a lobby line is kept.
pub const LOBBY_MAX_AGE_MS: u64 = 24 * 3_600_000;
/// The longest chat line, in characters.
pub const CHAT_MAX: usize = 200;
/// The longest section of the club info, in characters.
pub const MAX_INFO_TEXT: usize = 2000;
/// The most upcoming events in the club info.
pub const MAX_EVENTS: usize = 20;
/// The longest event, in characters.
pub const MAX_EVENT_TEXT: usize = 80;
/// The longest table name, in characters.
pub const MAX_TABLE_NAME: usize = 30;
/// The most seats at any table.
pub const MAX_SEATS: u32 = 9;
/// How long a seat may sit out before it's given up, unless the club says.
pub const DEFAULT_SIT_OUT_MINS: u32 = 10;
/// The longest the hit-and-run rule can hold someone, in hands.
pub const MAX_STAY_HANDS: u32 = 500;
/// How long a reserved seat is held once its member leaves their table.
pub const HOLD_MS: u64 = 60_000;

const TOKEN_CHARS: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const MANAGER: &str = "Manager";

/// Why the club refused something; the message is for people.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleError(pub String);

impl fmt::Display for RuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RuleError {}

pub(crate) fn refuse<T>(why: impl Into<String>) -> Result<T, RuleError> {
    Err(RuleError(why.into()))
}

/// Where someone is with the club.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Asked to join; the host hasn't answered.
    Pending,
    /// In the club.
    Member,
    /// Asked, and was turned down.
    Declined,
    /// Was a member, and was removed.
    Removed,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::Member => "member",
            Status::Declined => "declined",
            Status::Removed => "removed",
        }
    }
}

/// Someone who has asked to join, or is in the club.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    /// Their player id.
    #[serde(default)]
    pub id: String,
    /// What they go by here; no one else in the club has a name that reads
    /// the same ([`name_key`]).
    #[serde(default)]
    pub name: String,
    /// Where they are with the club.
    pub status: Status,
    /// When they last asked to join.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_at: Option<u64>,
    /// When they were let in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub joined_at: Option<u64>,
    /// When the host last changed their status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed_at: Option<u64>,
    /// When they were last removed. Kept when they ask to join again, so
    /// the manager knows: nobody is ever forgotten (ducy-web #488).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed_at: Option<u64>,
    /// "yes" when they're an agent (ducy-web #74).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// An agent's own invite token, for the people they bring.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_invite: Option<String>,
    /// The agent who brought them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_of: Option<String>,
    /// Everything else other parts keep on them (chips, seat, …).
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

impl Member {
    fn new(id: &str, name: &str) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            status: Status::Pending,
            requested_at: None,
            joined_at: None,
            changed_at: None,
            removed_at: None,
            agent: None,
            agent_invite: None,
            agent_of: None,
            other: Map::new(),
        }
    }

    pub(crate) fn is(&self, status: Status) -> bool {
        self.status == status
    }

    /// Their seat at a table, if they're at one.
    pub fn seat(&self) -> Option<&Map<String, Value>> {
        self.other.get("seat")?.as_object()
    }

    /// The table they're seated at.
    pub fn seat_table(&self) -> Option<&str> {
        self.seat()?.get("table")?.as_str()
    }

    fn is_agent(&self) -> bool {
        self.agent.as_deref() == Some("yes") && self.is(Status::Member)
    }
}

/// A member's call for help.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Help {
    /// Its id.
    pub id: String,
    /// The member who asked.
    pub from: String,
    /// Their name then.
    pub name: String,
    /// What they need.
    pub text: String,
    /// The table they asked from, or "".
    #[serde(default)]
    pub table: String,
    /// When.
    pub at: u64,
    /// The host's reply, or "".
    #[serde(default)]
    pub reply: String,
    /// Who handled it, and when.
    #[serde(default)]
    pub handled: Option<Handled>,
}

/// Who handled a help request, and when.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Handled {
    /// Their name.
    pub by: String,
    /// When.
    pub at: u64,
}

/// A line in the lobby chat.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LobbyLine {
    /// Its id.
    pub id: String,
    /// Who said it.
    pub from: String,
    /// Their name in the chat.
    pub name: String,
    /// What they said.
    pub text: String,
    /// When.
    pub at: u64,
    /// Said by the host.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub host: bool,
}

/// An upcoming event in the club info.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// When it is.
    pub at: u64,
    /// A few words.
    pub text: String,
}

/// What the host writes for members (ducy-web #398), each part optional.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Info {
    /// About the club.
    pub about: String,
    /// Getting chips.
    pub chips: String,
    /// House rules.
    pub rules: String,
    /// Upcoming events, by time.
    pub events: Vec<Event>,
    /// When each part last changed, by its id.
    pub at: BTreeMap<String, u64>,
}

/// The parts of the club info, in order: id and heading.
pub const SECTIONS: [(&str, &str); 4] = [
    ("about", "About the club"),
    ("events", "Upcoming events"),
    ("chips", "Getting chips"),
    ("rules", "House rules"),
];

/// A game the club's tables can deal.
#[derive(Clone, Copy, Debug)]
pub struct Game {
    /// Its id.
    pub id: &'static str,
    /// Its name.
    pub label: &'static str,
    /// Its name in a table's default name.
    pub short: &'static str,
    /// The most it seats: fewer where the deck runs out (two boards come
    /// from 52 cards, leaving 42 for hole cards).
    pub seats: u32,
    /// A new table's size.
    pub default_seats: u32,
}

/// The games, in order.
pub const GAMES: [Game; 4] = [
    Game {
        id: "nlhe",
        label: "Hold’em",
        short: "NLH",
        seats: MAX_SEATS,
        default_seats: 9,
    },
    Game {
        id: "plo4",
        label: "Omaha 4",
        short: "PLO",
        seats: MAX_SEATS,
        default_seats: 8,
    },
    Game {
        id: "plo5",
        label: "Omaha 5",
        short: "PLO5",
        seats: 8,
        default_seats: 6,
    },
    Game {
        id: "plo6",
        label: "Omaha 6",
        short: "PLO6",
        seats: 6,
        default_seats: 6,
    },
];

/// A game by its id.
pub fn game(id: &str) -> Option<&'static Game> {
    GAMES.iter().find(|g| g.id == id)
}

/// "A Hold’em", "An Omaha 6": the game with its article.
pub fn a_game(id: &str) -> String {
    let label = game(id).map_or(id, |g| g.label);
    let an = label
        .chars()
        .next()
        .is_some_and(|c| "AEIOUaeiou".contains(c));
    format!("{} {label}", if an { "An" } else { "A" })
}

/// Where a table is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TableStatus {
    /// Open: people sit down.
    #[default]
    Open,
    /// No one new sits; everyone stands up at the end of the hand.
    Closing,
    /// Kept for the history.
    Closed,
}

/// A table of the club's.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Table {
    /// Its id.
    pub id: String,
    /// Its name.
    pub name: String,
    /// The game's id.
    pub game: String,
    /// The small blind, in hundredths.
    pub sb: u64,
    /// The big blind, in hundredths.
    pub bb: u64,
    /// The smallest buy-in, in big blinds.
    #[serde(rename = "minBB")]
    pub min_bb: u64,
    /// The largest, in big blinds.
    #[serde(rename = "maxBB")]
    pub max_bb: u64,
    /// How many seats.
    pub seats: u32,
    /// Seconds to act; 0 for no limit.
    #[serde(default)]
    pub turn_secs: u32,
    /// Minutes a seat may sit out before it's given up; 0 for no limit.
    #[serde(default)]
    pub sit_out_mins: u32,
    /// The hit-and-run rule: hands someone up at the table plays before
    /// they may leave; 0 is off.
    #[serde(default)]
    pub stay_hands: u32,
    /// Where it is.
    pub status: TableStatus,
    /// When it was made.
    #[serde(default)]
    pub created_at: u64,
    /// Anything else kept on it.
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

impl Table {
    /// The buy-in range, in hundredths: (min, max).
    pub fn limits(&self) -> (u64, u64) {
        (self.min_bb * self.bb, self.max_bb * self.bb)
    }

    /// The game it deals, as players pick one: its type, stakes and seats.
    /// Tables with the same key share a waitlist (ducy-web #518).
    pub fn game_key(&self) -> String {
        format!("{}:{}/{}:{}", self.game, self.sb, self.bb, self.seats)
    }

    /// Its stakes, as tables are filtered by them.
    pub fn stakes(&self) -> Stakes {
        Stakes::of(self.bb)
    }
}

/// How high a table's stakes are, by its big blind, for filtering tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stakes {
    /// A big blind under 1.
    Micro,
    /// 1 to 3 (1/2, 1/3).
    Low,
    /// Over 3, up to 10 (2/5, 5/10).
    Medium,
    /// Over 10.
    High,
}

impl Stakes {
    /// The band of big blind `bb` (in hundredths).
    pub fn of(bb: u64) -> Self {
        match bb {
            0..100 => Stakes::Micro,
            100..=300 => Stakes::Low,
            301..=1000 => Stakes::Medium,
            _ => Stakes::High,
        }
    }

    /// Its name, as filters show it.
    pub fn name(self) -> &'static str {
        match self {
            Stakes::Micro => "Micro",
            Stakes::Low => "Low",
            Stakes::Medium => "Medium",
            Stakes::High => "High",
        }
    }
}

/// A seat held at a table for a member (ducy-web #425).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reservation {
    /// Who it's for.
    pub member: String,
    /// When it was made.
    pub at: u64,
    /// When it lapses, unless they're still at `from`.
    pub until: u64,
    /// The table they're leaving for it; it doesn't lapse while they're there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Who moved them there, when it wasn't them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
}

/// A club's own state, as ducy-web keeps it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Club {
    /// The version of the saved club.
    #[serde(default = "one")]
    pub v: u32,
    /// Its code (the relay room).
    #[serde(default)]
    pub code: String,
    /// Its name.
    #[serde(default)]
    pub name: String,
    /// The player id that signs for it.
    #[serde(default)]
    pub host_id: String,
    /// An always-on club's owner, who runs it from their page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<String>,
    /// The token in the club's invite link.
    #[serde(default)]
    pub invite: String,
    /// When it was made.
    #[serde(default)]
    pub created_at: u64,
    /// Whether people may ask to join with the club's code alone (#393).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_join: Option<bool>,
    /// Everyone who has asked to join, by id.
    #[serde(default)]
    pub members: BTreeMap<String, Member>,
    /// Help requests, oldest first.
    #[serde(default)]
    pub help: Vec<Help>,
    /// The lobby chat, oldest first.
    #[serde(default)]
    pub lobby: Vec<LobbyLine>,
    /// The club info.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<Info>,
    /// Its tables, closed ones too.
    #[serde(default)]
    pub tables: Vec<Table>,
    /// A new table's sit-out limit, in minutes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sit_out_mins: Option<Number>,
    /// A new table's hit-and-run rule, in hands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stay_hands: Option<Number>,
    /// The rathole rule's window, in hours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rathole_hours: Option<Number>,
    /// Reserved seats, by table, oldest first.
    #[serde(default)]
    pub waitlist: BTreeMap<String, Vec<Reservation>>,
    /// Grouped tables with must-move (ducy-web #501).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<crate::club_groups::Group>,
    /// Each game's waitlist, by [`Table::game_key`] (ducy-web #518).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub lists: BTreeMap<String, Vec<crate::club_lists::OnList>>,
    /// Outside servers that may give members chips, by their key's id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub services: BTreeMap<String, Service>,
    /// Requests refused from keys that are no service of the club's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_refused: Option<Refusals>,
}

fn one() -> u32 {
    1
}

/// What the club makes of a message from someone ([`Club::receive`]), in
/// ducy-web's shape: status replies to send, whether the club changed, why a
/// message was refused, and what the caller handles.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Received {
    /// Replies to sign and send: `{to, body}`.
    pub replies: Vec<Value>,
    /// Whether the club changed (save it).
    pub changed: bool,
    /// Why the message was refused.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// From an always-on club's owner: `{type, from, body}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<Value>,
    /// An agent's answer or command: `{type, from, body}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admin: Option<Value>,
    /// A table command: `{id, cmd, from}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<Value>,
    /// A tournament command: `{id, op, from}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tourney: Option<Value>,
    /// A help request: `{from, text, table}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<Value>,
    /// A lobby line: `{from, text}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lobby: Option<Value>,
    /// A chip request or its cancelling, from a member: the message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chips: Option<Value>,
    /// A member checked in: send them their account, with the ledger
    /// entries after this one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<f64>,
}

impl Received {
    fn refused(replies: Vec<Value>, reason: &str) -> Self {
        Self {
            replies,
            reason: Some(reason.into()),
            ..Self::default()
        }
    }
}

// ---- names ----

/// Runs of whitespace as one space, trimmed.
fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn cut(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn text_of(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

/// A club's name, tidied; refused when empty or too long.
pub fn club_name(s: &str) -> Result<String, RuleError> {
    let name = squash(s);
    if name.is_empty() {
        return refuse("Give the club a name.");
    }
    if name.chars().count() > MAX_CLUB_NAME {
        return refuse(format!(
            "Club names can be at most {MAX_CLUB_NAME} characters."
        ));
    }
    Ok(name)
}

/// A player's name, tidied and cut to length; "Player" when there's none.
pub fn player_name(s: &str) -> String {
    let name = cut(&squash(s), MAX_PLAYER_NAME);
    if name.is_empty() {
        "Player".into()
    } else {
        name
    }
}

fn lookalike(c: char) -> Option<&'static str> {
    Some(match c {
        'а' | 'α' => "a",
        'в' | 'β' => "b",
        'е' | 'ё' | 'ε' | '3' => "e",
        'к' | 'κ' => "k",
        'м' => "m",
        'н' => "h",
        'о' | 'ο' | '0' | 'ø' => "o",
        'р' | 'ρ' => "p",
        'с' => "c",
        'т' | 'τ' => "t",
        'у' => "y",
        'х' | 'χ' => "x",
        'і' | 'ї' | 'ι' | 'ı' => "i",
        'ј' => "j",
        'ѕ' | '5' => "s",
        'ԁ' | 'đ' => "d",
        'ԛ' => "q",
        'ԝ' | 'ω' => "w",
        'η' => "n",
        'μ' | 'υ' => "u",
        'ν' => "v",
        '1' | '|' | 'ł' => "l",
        'ß' => "ss",
        _ => return None,
    })
}

/// What a name reads as (#450): two names are the same if their keys are,
/// whatever their case, spacing, punctuation and accents, with look-alike
/// letters (Cyrillic, Greek, 0 for o…) taken as the letters they look like.
pub fn name_key(s: &str) -> String {
    let plain = s
        .nfkd()
        .filter(|&c| !is_combining_mark(c))
        .collect::<String>()
        .to_lowercase();
    let mut key = String::new();
    for c in plain.chars() {
        match lookalike(c) {
            Some(l) => key.push_str(l),
            None if c.is_alphanumeric() => key.push(c),
            None => {}
        }
    }
    if key.is_empty() {
        plain.trim().into()
    } else {
        key
    }
}

/// Chat text, ready to keep: control characters and runs of spaces as one
/// space, cut to [`CHAT_MAX`] characters; "" if nothing's left.
pub fn clean_chat(s: &str) -> String {
    let spaced: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    cut(&squash(&spaced), CHAT_MAX).trim().into()
}

/// A name for a chat line, cut short.
pub fn chat_name(s: &str) -> String {
    let n = cut(&clean_chat(s), MAX_PLAYER_NAME);
    if n.is_empty() { "A player".into() } else { n }
}

/// A token for an invite link.
pub fn new_invite(rng: &mut StdRng) -> String {
    (0..12)
        .map(|_| TOKEN_CHARS[rng.random::<u8>() as usize % TOKEN_CHARS.len()] as char)
        .collect()
}

fn base36(mut n: u64) -> String {
    let mut out = Vec::new();
    loop {
        out.push(std::char::from_digit((n % 36) as u32, 36).unwrap_or('0'));
        n /= 36;
        if n == 0 {
            break;
        }
    }
    out.iter().rev().collect()
}

pub(crate) fn random36(rng: &mut StdRng, len: usize) -> String {
    (0..len)
        .map(|_| std::char::from_digit(u32::from(rng.random::<u8>()) % 36, 36).unwrap_or('0'))
        .collect()
}

/// An id for a help request or a lobby line: the time, and four random
/// characters.
fn line_id(now: u64, rng: &mut StdRng) -> String {
    format!("{}{}", base36(now), random36(rng, 4))
}

// ---- the club info ----

/// Text as kept: line endings as `\n`, other whitespace runs as one space, at
/// most one blank line in a row, trimmed and cut to `max`.
pub(crate) fn info_text(s: &str, max: usize) -> String {
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::new();
    for c in s.chars() {
        if c == '\n' {
            // At most one blank line.
            if !out.ends_with("\n\n") {
                out.push('\n');
            }
        } else if c.is_whitespace() {
            if !out.ends_with(' ') {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
    cut(out.trim(), max)
}

fn safe_int(v: Option<&Value>) -> Option<u64> {
    let n = match v? {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    (n.fract() == 0.0 && n > 0.0 && n <= 9_007_199_254_740_991.0).then_some(n as u64)
}

fn events_from(list: Option<&Value>) -> Vec<Event> {
    let mut out: Vec<Event> = list
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(|e| {
                    let at = safe_int(e.get("at"))?;
                    let text =
                        info_text(&text_of(e.get("text")), MAX_EVENT_TEXT).replace('\n', " ");
                    (!text.is_empty()).then_some(Event { at, text })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|e| e.at);
    out.truncate(MAX_EVENTS);
    out
}

/// The club info as given (the host's form), checked: each part trimmed to
/// its length, and `at` saying when each changed since `prev`.
pub fn clean_info(input: &Value, prev: Option<&Info>, now: u64) -> Info {
    let mut out = Info {
        about: info_text(&text_of(input.get("about")), MAX_INFO_TEXT),
        chips: info_text(&text_of(input.get("chips")), MAX_INFO_TEXT),
        rules: info_text(&text_of(input.get("rules")), MAX_INFO_TEXT),
        events: events_from(input.get("events")),
        at: BTreeMap::new(),
    };
    let empty = Info::default();
    let was = prev.unwrap_or(&empty);
    for (k, _) in SECTIONS {
        let same = match k {
            "about" => out.about == was.about,
            "chips" => out.chips == was.chips,
            "rules" => out.rules == was.rules,
            _ => out.events == was.events,
        };
        let then = was.at.get(k).copied().unwrap_or(0);
        if !same {
            out.at.insert(k.into(), now);
        } else if then > 0 {
            out.at.insert(k.into(), then);
        }
    }
    out
}

/// A table's default name: "NLH - 1/2", in plain amounts.
fn plain_amount(n: u64) -> String {
    let (whole, cents) = (n / 100, n % 100);
    match cents {
        0 => whole.to_string(),
        c if c % 10 == 0 => format!("{whole}.{}", c / 10),
        c => format!("{whole}.{c:02}"),
    }
}

fn whole(v: f64) -> bool {
    v.fract() == 0.0 && (0.0..=9_007_199_254_740_991.0).contains(&v)
}

/// A number from a form: given (not null), or the default.
fn num(spec: &Value, key: &str, default: f64) -> f64 {
    match spec.get(key) {
        None | Some(Value::Null) => default,
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(_) => f64::NAN,
    }
}

impl Club {
    /// A new club, with an invite.
    pub fn new(
        name: &str,
        code: &str,
        host_id: &str,
        now: u64,
        rng: &mut StdRng,
    ) -> Result<Club, RuleError> {
        Ok(Club {
            v: 1,
            code: code.into(),
            name: club_name(name)?,
            host_id: host_id.into(),
            owner_id: None,
            invite: new_invite(rng),
            created_at: now,
            code_join: None,
            members: BTreeMap::new(),
            help: Vec::new(),
            lobby: Vec::new(),
            info: None,
            tables: Vec::new(),
            sit_out_mins: None,
            stay_hands: None,
            rathole_hours: None,
            waitlist: BTreeMap::new(),
            groups: vec![],
            lists: BTreeMap::new(),
            services: BTreeMap::new(),
            service_refused: None,
        })
    }

    fn is_host(&self, id: &str) -> bool {
        id == self.host_id || self.owner_id.as_deref() == Some(id)
    }

    fn member(&self, id: &str) -> Option<&Member> {
        self.members.get(id).filter(|m| m.is(Status::Member))
    }

    /// A new invite token: the old link stops working for people who haven't
    /// asked yet. Requests and members aren't affected.
    pub fn revoke_invite(&mut self, rng: &mut StdRng) {
        self.invite = new_invite(rng);
    }

    /// The member or person asking to join, other than `id`, whose name
    /// reads as `name` (#450).
    pub fn name_taken_by(&self, id: &str, name: &str) -> Option<&Member> {
        let key = name_key(name);
        self.members.values().find(|m| {
            m.id != id
                && (m.is(Status::Member) || m.is(Status::Pending))
                && name_key(&m.name) == key
        })
    }

    /// The status message for `id`.
    pub fn status_body(&self, id: &str) -> Value {
        let status = self
            .members
            .get(id)
            .map_or("unknown", |m| m.status.as_str());
        json!({"type": "status", "club": self.code, "name": self.name, "status": status})
    }

    fn status_with(&self, id: &str, key: &str, value: &str) -> Value {
        let mut body = self.status_body(id);
        body[key] = value.into();
        json!({"to": id, "body": body})
    }

    fn reply(&self, id: &str) -> Value {
        json!({"to": id, "body": self.status_body(id)})
    }

    /// A message from `from`, whose signature the caller has checked.
    pub fn receive(&mut self, from: &str, body: &Value, now: u64) -> Received {
        if body.get("club").and_then(Value::as_str) != Some(self.code.as_str()) {
            return Received::refused(vec![], "wrong club");
        }
        let kind = body.get("type").and_then(Value::as_str).unwrap_or("");
        let str_of = |k: &str| body.get(k).and_then(Value::as_str);
        if kind == "owner_hello" || kind == "owner_cmd" {
            if self.owner_id.as_deref() != Some(from) {
                return Received::refused(vec![], "not the owner");
            }
            return Received {
                owner: Some(json!({"type": kind, "from": from, "body": body})),
                ..Received::default()
            };
        }
        if kind == "join" {
            return self.join(from, body, now);
        }
        if kind == "hello" {
            return self.hello(from, body);
        }
        let known = [
            "agent_reply",
            "agent_cmd",
            "table",
            "tourney",
            "help_ask",
            "lobby_say",
            "chip_request",
            "cancel_request",
        ];
        if !known.contains(&kind) {
            return Received::refused(vec![], "unknown message");
        }
        if self.member(from).is_none() {
            return Received::refused(vec![self.reply(from)], "not a member");
        }
        let text = str_of("text").filter(|t| !t.trim().is_empty());
        let mut r = Received::default();
        match kind {
            "agent_reply" | "agent_cmd" => {
                r.admin = Some(json!({"type": kind, "from": from, "body": body}));
            }
            "table" => {
                let cmd = body.get("cmd").filter(|c| c.is_object() || c.is_array());
                match (str_of("table"), cmd) {
                    (Some(t), Some(cmd)) => {
                        r.table = Some(json!({"id": t, "cmd": cmd, "from": from}))
                    }
                    _ => return Received::refused(vec![], "bad table command"),
                }
            }
            "tourney" => match (str_of("tourney"), str_of("op")) {
                (Some(t), Some(op @ ("register" | "unregister" | "enter"))) => {
                    r.tourney = Some(json!({"id": t, "op": op, "from": from}));
                }
                _ => return Received::refused(vec![], "bad tournament command"),
            },
            "help_ask" => match text {
                Some(t) => {
                    r.help = Some(
                        json!({"from": from, "text": t, "table": str_of("table").unwrap_or("")}),
                    );
                }
                None => return Received::refused(vec![], "bad help request"),
            },
            "lobby_say" => match text {
                Some(t) => r.lobby = Some(json!({"from": from, "text": t})),
                None => return Received::refused(vec![], "bad lobby message"),
            },
            _ => r.chips = Some(body.clone()),
        }
        r
    }

    fn join(&mut self, from: &str, body: &Value, now: u64) -> Received {
        let name = player_name(&text_of(body.get("name")));
        let taken = self.name_taken_by(from, &name).is_some();
        if let Some(m) = self
            .members
            .get_mut(from)
            .filter(|m| m.is(Status::Member) || m.is(Status::Pending))
        {
            // Someone in the club already goes by it: they're told, and pick another.
            if taken && m.name != name {
                return Received::refused(
                    vec![self.status_with(from, "nameTaken", &name)],
                    "name taken",
                );
            }
            let changed = m.name != name;
            m.name = name;
            return Received {
                replies: vec![self.reply(from)],
                changed,
                ..Received::default()
            };
        }
        let invite = body
            .get("invite")
            .and_then(Value::as_str)
            .filter(|i| !i.is_empty());
        // The club's code alone (#393): only when the club allows it.
        let Some(invite) = invite.or(if self.code_join == Some(true) {
            Some("")
        } else {
            None
        }) else {
            let to_them = json!({"to": from, "body": {"type": "status", "club": self.code, "status": "invite_only"}});
            return Received::refused(vec![to_them], "no invite");
        };
        // The club's link, or an agent's (#74): their recruits wait for the agent.
        let agent = (!invite.is_empty())
            .then(|| {
                self.members
                    .values()
                    .find(|a| a.is_agent() && a.agent_invite.as_deref() == Some(invite))
            })
            .flatten()
            .map(|a| a.id.clone());
        if !invite.is_empty() && invite != self.invite && agent.is_none() {
            return Received::refused(
                vec![self.status_with(from, "status", "bad_invite")],
                "bad invite",
            );
        }
        if taken {
            let mut reply = self.status_with(from, "status", "unknown");
            reply["body"]["nameTaken"] = name.into();
            return Received::refused(vec![reply], "name taken");
        }
        // New, or asking again after being declined or removed.
        let m = self
            .members
            .entry(from.into())
            .or_insert_with(|| Member::new(from, &name));
        // Removed before this was kept: when their status last changed.
        if m.is(Status::Removed) && m.removed_at.is_none() {
            m.removed_at = m.changed_at.or(Some(now));
        }
        m.name = name;
        m.status = Status::Pending;
        m.requested_at = Some(now);
        if agent.is_some() {
            m.agent_of = agent;
        }
        Received {
            replies: vec![self.reply(from)],
            changed: true,
            ..Received::default()
        }
    }

    fn hello(&mut self, from: &str, body: &Value) -> Received {
        let mut changed = false;
        let mut taken_name = None;
        let asked = text_of(body.get("name"));
        if !asked.is_empty() && self.members.contains_key(from) {
            let name = player_name(&asked);
            let same = self.members[from].name == name;
            // A new name someone else in the club goes by is refused (#450).
            if !same && self.name_taken_by(from, &name).is_some() {
                taken_name = Some(name);
            } else if let Some(m) = self.members.get_mut(from) {
                changed = !same;
                m.name = name;
            }
        }
        let reply = match &taken_name {
            Some(n) => self.status_with(from, "nameTaken", n),
            None => self.reply(from),
        };
        // `since` counts in the money model's ledger only from a copy that
        // already is one (#454); an older copy gets every entry.
        let links = body.get("ledger").and_then(Value::as_str) == Some("links");
        let since = if links {
            body.get("since").and_then(Value::as_f64).unwrap_or(0.0)
        } else {
            0.0
        };
        Received {
            replies: vec![reply],
            changed,
            account: self.member(from).map(|_| since),
            ..Received::default()
        }
    }

    fn set_status(&mut self, id: &str, to: Status, from: &[Status], now: u64) -> Option<Value> {
        let m = self
            .members
            .get_mut(id)
            .filter(|m| from.contains(&m.status))?;
        m.status = to;
        if to == Status::Member {
            m.joined_at = Some(now);
        }
        if to == Status::Removed {
            m.removed_at = Some(now);
        }
        m.changed_at = Some(now);
        Some(self.reply(id))
    }

    /// Lets in someone who asked; the reply to send, or None if they hadn't.
    pub fn approve(&mut self, id: &str, now: u64) -> Option<Value> {
        self.set_status(id, Status::Member, &[Status::Pending], now)
    }

    /// Turns down someone who asked; the reply, or None if they hadn't.
    pub fn decline(&mut self, id: &str, now: u64) -> Option<Value> {
        self.set_status(id, Status::Declined, &[Status::Pending], now)
    }

    /// Whether `id` can be removed now: a member or someone asking, not
    /// seated at a table (the table cashes them out first).
    pub fn removable(&self, id: &str) -> bool {
        self.members.get(id).is_some_and(|m| {
            (m.is(Status::Member) || m.is(Status::Pending))
                && m.other.get("seat").is_none_or(Value::is_null)
        })
    }

    /// Removes `id` (the caller has taken back their chips); the reply, or
    /// None if they can't be.
    pub fn remove(&mut self, id: &str, now: u64) -> Option<Value> {
        if !self.removable(id) {
            return None;
        }
        // Off every waitlist (#518).
        let keys: Vec<String> = self.lists.keys().cloned().collect();
        for k in keys {
            self.leave_list(id, &k);
        }
        self.set_status(id, Status::Removed, &[Status::Member, Status::Pending], now)
    }

    // ---- help ----

    /// Member `from` asks for help; a new request replaces their open one.
    pub fn ask_help(
        &mut self,
        from: &str,
        text: &str,
        table: &str,
        now: u64,
        rng: &mut StdRng,
    ) -> Result<&Help, RuleError> {
        let Some(m) = self.member(from) else {
            return refuse("Only members can ask for help.");
        };
        let text = cut(&squash(text), MAX_HELP_TEXT);
        if text.is_empty() {
            return refuse("Say what you need help with.");
        }
        let name = m.name.clone();
        self.help.retain(|h| h.from != from || h.handled.is_some());
        self.help.push(Help {
            id: line_id(now, rng),
            from: from.into(),
            name,
            text,
            table: cut(&squash(table), 50),
            at: now,
            reply: String::new(),
            handled: None,
        });
        if self.help.len() > MAX_HELP {
            self.help.drain(..self.help.len() - MAX_HELP);
        }
        Ok(self.help.last().expect("just added"))
    }

    /// The host (`by`) answers request `id`: a reply, and/or marking it handled.
    pub fn answer_help(
        &mut self,
        by: &str,
        id: &str,
        reply: &str,
        handled: bool,
        now: u64,
    ) -> Result<&Help, RuleError> {
        if !self.is_host(by) {
            return refuse("Only the club’s manager answers help requests.");
        }
        let name = self
            .members
            .get(by)
            .map_or(MANAGER.to_string(), |m| m.name.clone());
        let Some(h) = self.help.iter_mut().find(|h| h.id == id) else {
            return refuse("No such help request.");
        };
        let reply = cut(&squash(reply), MAX_HELP_REPLY);
        if !reply.is_empty() {
            h.reply = reply;
        }
        if handled {
            h.handled = Some(Handled { by: name, at: now });
        }
        Ok(h)
    }

    fn host_name(&self) -> String {
        self.owner_id
            .as_ref()
            .and_then(|o| self.members.get(o))
            .map_or(MANAGER.to_string(), |m| m.name.clone())
    }

    fn owner_online(&self, online: &[String]) -> bool {
        self.owner_id.as_ref().is_some_and(|o| online.contains(o))
    }

    /// Who's around to help, given who's connected and whether the host's
    /// own page is running the club: `[{name, role: "host"}]`, or none.
    pub fn staff_online(&self, online: &[String], host_here: bool) -> Vec<Value> {
        if !host_here && !self.owner_online(online) {
            return vec![];
        }
        vec![json!({"name": self.host_name(), "role": "host"})]
    }

    /// What member `id` sees of help: who's around, and their latest requests.
    pub fn help_body(&self, id: &str, staff: &[Value]) -> Value {
        let mine: Vec<&Help> = self.help.iter().filter(|h| h.from == id).collect();
        let mine: Vec<Value> = mine[mine.len().saturating_sub(HELP_MINE)..]
            .iter()
            .map(|h| {
                json!({
                    "id": h.id, "text": h.text, "at": h.at, "reply": h.reply,
                    "handled": h.handled.is_some(),
                    "by": h.handled.as_ref().map_or("", |x| x.by.as_str()),
                })
            })
            .collect();
        json!({"type": "help", "club": self.code, "staff": staff, "mine": mine})
    }

    // ---- the lobby ----

    fn fresh_lobby(&self, now: u64) -> impl Iterator<Item = &LobbyLine> {
        self.lobby
            .iter()
            .filter(move |x| now.saturating_sub(x.at) < LOBBY_MAX_AGE_MS)
    }

    /// `from`, a member or the host, says `text` in the lobby.
    pub fn say_in_lobby(
        &mut self,
        from: &str,
        text: &str,
        now: u64,
        rng: &mut StdRng,
    ) -> Result<&LobbyLine, RuleError> {
        let host = self.is_host(from);
        let name = match self.member(from) {
            _ if host => self.host_name(),
            Some(m) => chat_name(&m.name),
            None => return refuse("Only members can chat in the lobby."),
        };
        let text = clean_chat(text);
        if text.is_empty() {
            return refuse("Say something first.");
        }
        let mut lobby: Vec<LobbyLine> = self.fresh_lobby(now).cloned().collect();
        lobby.push(LobbyLine {
            id: line_id(now, rng),
            from: from.into(),
            name,
            text,
            at: now,
            host,
        });
        let drop = lobby.len().saturating_sub(LOBBY_KEEP);
        lobby.drain(..drop);
        self.lobby = lobby;
        Ok(self.lobby.last().expect("just added"))
    }

    /// Who's online, given who's connected (`online`) and whether the host's
    /// own page is running the club: `{id, name, role, table}`, role "host"
    /// or "", and the name of the table they're at. The host first, then by
    /// name.
    pub fn online_list(&self, online: &[String], host_here: bool) -> Vec<Value> {
        let mut out = Vec::new();
        if host_here || self.owner_online(online) {
            let id = self.owner_id.as_ref().unwrap_or(&self.host_id);
            out.push(json!({"id": id, "name": self.host_name(), "role": "host", "table": ""}));
        }
        let mut rest: Vec<(String, Value)> = online
            .iter()
            .filter(|id| self.owner_id.as_ref() != Some(*id))
            .filter_map(|id| self.member(id))
            .map(|m| {
                let table = m.seat_table().map_or(String::new(), |t| {
                    self.table(t).map_or("a table".into(), |t| t.name.clone())
                });
                (
                    m.name.clone(),
                    json!({"id": m.id, "name": m.name, "role": "", "table": table}),
                )
            })
            .collect();
        rest.sort_by(|a, b| {
            a.0.to_lowercase()
                .cmp(&b.0.to_lowercase())
                .then(a.0.cmp(&b.0))
        });
        out.extend(rest.into_iter().map(|(_, v)| v));
        out
    }

    /// What every member online gets: who's here, and the chat.
    pub fn lobby_body(&self, online: &[String], host_here: bool, now: u64) -> Value {
        let chat: Vec<Value> = self
            .fresh_lobby(now)
            .map(|x| {
                let mut line = json!({"id": x.id, "name": x.name, "text": x.text, "at": x.at});
                if x.host {
                    line["host"] = true.into();
                }
                line
            })
            .collect();
        json!({"type": "lobby", "club": self.code, "online": self.online_list(online, host_here), "chat": chat})
    }

    // ---- the club info ----

    /// Sets the club info from the host's form.
    pub fn set_info(&mut self, input: &Value, now: u64) -> &Info {
        let info = clean_info(input, self.info.as_ref(), now);
        self.info.insert(info)
    }

    /// What members are sent of the club info.
    pub fn info_body(&self) -> Value {
        json!({"type": "info", "club": self.code, "info": self.info})
    }

    // ---- tables ----

    /// A table by its id.
    pub fn table(&self, id: &str) -> Option<&Table> {
        self.tables.iter().find(|t| t.id == id)
    }

    /// The tables not closed.
    pub fn open_tables(&self) -> impl Iterator<Item = &Table> {
        self.tables
            .iter()
            .filter(|t| t.status != TableStatus::Closed)
    }

    /// A new table's name when the host doesn't give one: the game and
    /// blinds, "NLH - 1/2", with " - #2", " - #3"… when an open table has it.
    pub fn default_table_name(&self, game_id: &str, sb: u64, bb: u64) -> String {
        let short = game(game_id).map_or(game_id, |g| g.short);
        let base = format!("{short} - {}/{}", plain_amount(sb), plain_amount(bb));
        let taken = |n: &str| self.open_tables().any(|t| t.name == n);
        if !taken(&base) {
            return base;
        }
        (2..)
            .map(|i| format!("{base} - #{i}"))
            .find(|n| !taken(n))
            .expect("some number is free")
    }

    /// Refused when the club's plan (`most` tables open at once) allows no more.
    pub fn check_table_limit(&self, most: Option<u64>) -> Result<(), RuleError> {
        let Some(most) = most else { return Ok(()) };
        if self.open_tables().count() as u64 >= most {
            return refuse(match most {
                0 => "This club can’t open tables. Ask ducy about it.".to_string(),
                1 => "This club can have 1 table open at once. Close one first, or ask ducy for more.".into(),
                n => format!("This club can have {n} tables open at once. Close one first, or ask ducy for more."),
            });
        }
        Ok(())
    }

    /// Adds a table from the host's choices (`{name, game, sb, bb, minBB,
    /// maxBB, seats, turnSecs, sitOutMins, stayHands}`, each optional), within
    /// the plan's `most` tables.
    pub fn new_table(
        &mut self,
        spec: &Value,
        now: u64,
        most: Option<u64>,
        rng: &mut StdRng,
    ) -> Result<&Table, RuleError> {
        self.check_table_limit(most)?;
        let game_id = match spec.get("game") {
            None | Some(Value::Null) => "nlhe",
            Some(g) => g.as_str().unwrap_or(""),
        };
        let Some(g) = game(game_id) else {
            return refuse("Pick a game.");
        };
        let pos = |n: f64| whole(n) && n > 0.0;
        let (sb, bb) = (num(spec, "sb", 1.0), num(spec, "bb", 2.0));
        if !pos(sb) || !pos(bb) || bb < sb {
            return refuse("The big blind must be at least the small blind, both above 0.");
        }
        let (min_bb, max_bb) = (num(spec, "minBB", 40.0), num(spec, "maxBB", 100.0));
        if !pos(min_bb) || !pos(max_bb) || max_bb < min_bb {
            return refuse("The maximum buy-in must be at least the minimum.");
        }
        let seats = num(spec, "seats", f64::from(g.default_seats));
        if !whole(seats) || seats < 2.0 || seats > f64::from(g.seats) {
            return refuse(format!("{} table seats 2 to {}.", a_game(g.id), g.seats));
        }
        let turn_secs = num(spec, "turnSecs", 30.0);
        if !whole(turn_secs) {
            return refuse("Pick a turn time.");
        }
        let setting = |n: &Option<Number>, default: f64| {
            n.as_ref().and_then(Number::as_f64).unwrap_or(default)
        };
        let club_sit_out = setting(&self.sit_out_mins, f64::from(DEFAULT_SIT_OUT_MINS));
        let sit_out_mins = num(spec, "sitOutMins", club_sit_out);
        if !whole(sit_out_mins) {
            return refuse("Pick a sit-out limit.");
        }
        let stay_hands = num(spec, "stayHands", setting(&self.stay_hands, 0.0));
        if !whole(stay_hands) || stay_hands > f64::from(MAX_STAY_HANDS) {
            return refuse(format!("Call time: 0 (off) to {MAX_STAY_HANDS} hands."));
        }
        let (sb, bb) = (sb as u64, bb as u64);
        let given = cut(text_of(spec.get("name")).trim(), MAX_TABLE_NAME);
        let name = if given.is_empty() {
            self.default_table_name(g.id, sb, bb)
        } else {
            given
        };
        let id = random36(rng, 6);
        self.tables.push(Table {
            id: id.clone(),
            name,
            game: g.id.into(),
            sb,
            bb,
            min_bb: min_bb as u64,
            max_bb: max_bb as u64,
            seats: seats as u32,
            turn_secs: turn_secs as u32,
            sit_out_mins: sit_out_mins as u32,
            stay_hands: stay_hands as u32,
            status: TableStatus::Open,
            created_at: now,
            other: Map::new(),
        });
        // A new table of a must-move game goes at the end of its chain (#518).
        self.join_group(&id);
        Ok(self.table(&id).expect("just added"))
    }

    /// What members are told about the tables (no one's chips), with how
    /// many are seated at each.
    pub fn tables_body(&self, seated: &BTreeMap<String, u32>) -> Value {
        let tables: Vec<Value> = self
            .tables
            .iter()
            .filter(|t| t.status == TableStatus::Open)
            .map(|t| {
                let (min, max) = t.limits();
                let mut v = json!({
                    "id": t.id, "name": t.name, "game": t.game, "sb": t.sb, "bb": t.bb,
                    "min": min, "max": max, "seats": t.seats,
                    "taken": seated.get(&t.id).copied().unwrap_or(0),
                    // Its game, for the waitlists (#518).
                    "key": t.game_key(),
                    // Its stakes band, for filtering.
                    "stakes": t.stakes(),
                });
                // The hit-and-run rule, so members know before they sit.
                if t.stay_hands > 0 {
                    v["stayHands"] = t.stay_hands.into();
                }
                // Must move (#501, #518): a main table, or the tables a
                // must-move one feeds and its move order.
                let g = self.group_label(&t.id);
                if !g.is_null() {
                    v["group"] = g;
                }
                v
            })
            .collect();
        // The rathole rule's window, so members see it before they sit (#187).
        // How many are on each game's list (#518).
        json!({"type": "tables", "club": self.code, "tables": tables, "rathole": self.rathole_hours.clone().unwrap_or(0.into()),
               "lists": self.list_counts()})
    }

    /// May member `id` join table `table`? "" if so, else why not. `at` is
    /// the table they're at in the host's running session, if any.
    pub fn can_join(&self, id: &str, table: &str, at: Option<&str>) -> String {
        let Some(m) = self.member(id) else {
            return "Only club members can sit down.".into();
        };
        if self
            .table(table)
            .is_none_or(|t| t.status != TableStatus::Open)
        {
            return "That table isn’t open.".into();
        }
        let elsewhere = |t: Option<&str>| t.is_some_and(|t| !t.is_empty() && t != table);
        if elsewhere(at) || elsewhere(m.seat_table()) {
            return "You’re already at another table. Leave it first.".into();
        }
        String::new()
    }

    /// The hit-and-run rule: marks when each seated member (`players`,
    /// between hands, their stacks already updated) first got ahead at their
    /// table, as of hand `hand`; back to even or down clears it.
    pub fn note_stacks(&mut self, players: &[String], hand: u64) {
        for id in players {
            let Some(seat) = self
                .members
                .get_mut(id)
                .and_then(|m| m.other.get_mut("seat"))
                .and_then(Value::as_object_mut)
            else {
                continue;
            };
            // Seated before the ledger kept what they brought.
            let Some(put) = seat.get("put").and_then(Value::as_f64) else {
                continue;
            };
            let stack = seat.get("stack").and_then(Value::as_f64).unwrap_or(0.0);
            if stack > put {
                seat.entry("upAt").or_insert(hand.into());
            } else {
                seat.remove("upAt");
            }
        }
    }

    /// How many more hands member `id` plays at table `table` before they
    /// may leave, with `completed` hands finished there. 0 when they may go.
    pub fn hands_to_stay(&self, id: &str, table: &str, completed: u64) -> u64 {
        let need = self.table(table).map_or(0, |t| u64::from(t.stay_hands));
        let Some(s) = self.members.get(id).and_then(Member::seat) else {
            return 0;
        };
        let f = |k: &str| s.get(k).and_then(Value::as_f64);
        let (Some(up_at), Some(put)) = (f("upAt"), f("put")) else {
            return 0;
        };
        let here = s.get("table").and_then(Value::as_str) == Some(table);
        if need == 0 || !here || f("stack").unwrap_or(0.0) <= put {
            return 0;
        }
        need.saturating_sub(completed.saturating_sub(up_at as u64))
    }

    /// Tables that were closing close, except those in `keep` (they may
    /// still finish their hand).
    pub fn close_closing(&mut self, keep: &[String]) {
        for t in &mut self.tables {
            if t.status == TableStatus::Closing && !keep.contains(&t.id) {
                t.status = TableStatus::Closed;
            }
        }
    }

    // ---- reserved seats (#425) ----

    fn still_at(&self, e: &Reservation) -> bool {
        e.from.as_deref().is_some_and(|f| {
            !f.is_empty() && self.members.get(&e.member).and_then(Member::seat_table) == Some(f)
        })
    }

    /// The reservations at table `table` still held at `now`.
    pub fn reservations(&self, table: &str, now: u64) -> Vec<&Reservation> {
        self.waitlist
            .get(table)
            .map(|l| {
                l.iter()
                    .filter(|e| e.until > now || self.still_at(e))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Seats held at table `table` for anyone but `except`.
    pub fn held_seats(&self, table: &str, now: u64, except: &str) -> usize {
        self.reservations(table, now)
            .iter()
            .filter(|e| e.member != except)
            .count()
    }

    /// Holds a seat at table `table` for member `id`, who is at table `from`
    /// (or at none); `seated` are at it now. One reservation at a time.
    #[allow(clippy::too_many_arguments)]
    pub fn reserve_seat(
        &mut self,
        id: &str,
        table: &str,
        seated: usize,
        from: Option<&str>,
        by: Option<&str>,
        now: u64,
        hold_ms: u64,
    ) -> Result<&Reservation, RuleError> {
        if self.member(id).is_none() {
            return refuse("Only club members can sit down.");
        }
        let seats = match self.table(table) {
            Some(t) if t.status == TableStatus::Open => t.seats as usize,
            _ => return refuse("That table isn’t open."),
        };
        let from = from.filter(|f| !f.is_empty());
        if from == Some(table) {
            return refuse("You’re at that table already.");
        }
        if seated + self.held_seats(table, now, id) >= seats {
            return refuse("That table is full.");
        }
        self.prune_reservations(now);
        let tables: Vec<String> = self.waitlist.keys().cloned().collect();
        for t in tables {
            self.drop_reservation(&t, id);
        }
        let list = self.waitlist.entry(table.into()).or_default();
        list.push(Reservation {
            member: id.into(),
            at: now,
            until: now + hold_ms,
            from: from.map(Into::into),
            by: by.filter(|b| !b.is_empty()).map(Into::into),
        });
        Ok(list.last().expect("just added"))
    }

    /// Gives up member `id`'s reservation at table `table`; whether there was one.
    pub fn drop_reservation(&mut self, table: &str, id: &str) -> bool {
        let Some(list) = self.waitlist.get_mut(table) else {
            return false;
        };
        let before = list.len();
        list.retain(|e| e.member != id);
        let dropped = list.len() < before;
        if list.is_empty() {
            self.waitlist.remove(table);
        }
        dropped
    }

    /// Member `id`'s reservation at `table` lapses `hold_ms` from `now` (they
    /// left the table they were at); whether they had one.
    pub fn hold_from(&mut self, table: &str, id: &str, now: u64, hold_ms: u64) -> bool {
        let e = self
            .waitlist
            .get_mut(table)
            .and_then(|l| l.iter_mut().find(|e| e.member == id));
        e.map(|e| e.until = now + hold_ms).is_some()
    }

    /// Drops the reservations that have lapsed; returns them, each with its
    /// table.
    pub fn prune_reservations(&mut self, now: u64) -> Vec<(String, Reservation)> {
        let mut gone = Vec::new();
        let tables: Vec<String> = self.waitlist.keys().cloned().collect();
        for t in tables {
            let list = self.waitlist.remove(&t).unwrap_or_default();
            let (keep, lapsed): (Vec<_>, Vec<_>) = list
                .into_iter()
                .partition(|e| e.until > now || self.still_at(e));
            gone.extend(lapsed.into_iter().map(|e| (t.clone(), e)));
            if !keep.is_empty() {
                self.waitlist.insert(t, keep);
            }
        }
        gone
    }

    /// When the next reservation lapses, if any will.
    pub fn next_lapse(&self) -> Option<u64> {
        self.waitlist
            .values()
            .flatten()
            .filter(|e| !self.still_at(e))
            .map(|e| e.until)
            .min()
    }
}

// ---- services ----
//
// Outside servers an always-on club's owner lets give members chips, within
// limits (a rewards server paying for a league). A service signs its
// requests with its own key; the caller checks the signature, and moves the
// chips through the money model. Amounts here are hundredths, as in the
// club; services count in chips, so their answers do too.

/// How far a request's time may be from the club's.
pub const SKEW_MS: u64 = 5 * 60_000;
/// The most services a club has at once.
pub const MAX_SERVICES: usize = 10;
const DAY_MS: u64 = 86_400_000;
const MAX_DONE: usize = 500;
const MAX_SERVICE_NAME: usize = 40;
const MAX_SERVICE_NOTE: usize = 100;
const MAX_KEY: usize = 64;

/// What a service may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Perms {
    /// Give members chips.
    pub give: bool,
    /// Take chips back.
    pub take: bool,
    /// List the members.
    pub read: bool,
}

/// Chips a service gave, and when.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Given {
    /// When.
    pub at: u64,
    /// Hundredths.
    pub amount: u64,
}

/// A give or take done with a key, and its answer, to give again.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Done {
    /// When.
    pub at: u64,
    /// The answer.
    pub value: Value,
}

/// A refused request, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    /// When.
    pub at: u64,
    /// Why.
    pub reason: String,
}

/// Requests refused from unknown keys.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusals {
    /// How many.
    pub count: u64,
    /// The last, when.
    pub at: u64,
}

/// Whether a service still works.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceStatus {
    /// Its requests are taken.
    Active,
    /// Its key is forgotten.
    Revoked,
}

/// An outside server that may give members chips.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Service {
    /// Its key's id.
    pub id: String,
    /// Its name.
    pub name: String,
    /// Its key's algorithm.
    pub alg: String,
    /// Its public key; gone once it's revoked.
    #[serde(rename = "pub", default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Whether it still works.
    pub status: ServiceStatus,
    /// When it was added.
    pub added_at: u64,
    /// What it may do.
    pub perms: Perms,
    /// Chips it may still give, in hundredths.
    pub budget: u64,
    /// The most in one give; 0 for no cap.
    pub per_give: u64,
    /// The most given in any 24 hours; 0 for no cap.
    pub per_day: u64,
    /// When it stops working.
    pub expires: Option<u64>,
    /// What it gave in the last day.
    #[serde(default)]
    pub gives: Vec<Given>,
    /// The request times seen lately (each once), and when.
    #[serde(default)]
    pub seen: BTreeMap<String, u64>,
    /// Gives and takes done with a key, by the key.
    #[serde(default)]
    pub done: BTreeMap<String, Done>,
    /// How many of its requests were refused.
    #[serde(default)]
    pub failures: u64,
    /// The last one.
    #[serde(default)]
    pub last_failure: Option<Failure>,
    /// When it was last used.
    #[serde(default)]
    pub last_used: Option<u64>,
    /// When it was revoked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<u64>,
}

impl Service {
    /// Chips given in the 24 hours to `now`.
    pub fn given_today(&self, now: u64) -> u64 {
        self.gives
            .iter()
            .filter(|g| now.saturating_sub(g.at) < DAY_MS)
            .map(|g| g.amount)
            .sum()
    }
}

/// A request a service may not make: the HTTP status to answer, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Refused {
    /// The HTTP status.
    pub status: u16,
    /// Why.
    pub reason: String,
}

fn refused<T>(status: u16, reason: impl Into<String>) -> Result<T, Refused> {
    Err(Refused {
        status,
        reason: reason.into(),
    })
}

/// What a service's request comes to, once it's checked.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Prepared {
    /// The answer, with nothing more to do.
    Value(Value),
    /// The members, for the caller to list.
    Members,
    /// Chips to give or take back: the caller moves them, then calls
    /// [`Club::service_done`].
    Move {
        /// "give" or "take".
        op: String,
        /// The member.
        member: String,
        /// Hundredths.
        amount: u64,
        /// The ledger note.
        note: String,
        /// What they wrote, if anything.
        said: String,
        /// The key that makes it once only, if any.
        key: Option<String>,
    },
}

/// An amount for people: "200", "1,234.50".
pub fn chip_text(n: u64) -> String {
    let whole = (n / 100).to_string();
    let mut grouped = String::new();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    match n % 100 {
        0 => grouped,
        c => format!("{grouped}.{c:02}"),
    }
}

/// A number as JavaScript's `Number()` reads it, for the owner's form.
fn number(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::String(s) if s.trim().is_empty() => 0.0,
        Value::String(s) => s.trim().parse().unwrap_or(f64::NAN),
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Null => 0.0,
        _ => f64::NAN,
    }
}

/// Whether JavaScript takes `v` as true.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

fn given(o: &Value, key: &str) -> Option<Value> {
    o.get(key).filter(|v| !v.is_null()).cloned()
}

/// A service's limits from the owner's form, over `old`'s.
fn limits_from(
    o: &Value,
    old: Option<&Service>,
) -> Result<(Perms, u64, u64, u64, Option<u64>), RuleError> {
    let perms = match given(o, "perms") {
        Some(p) => {
            let on = |k: &str| p.get(k).is_some_and(truthy);
            Perms {
                give: on("give"),
                take: on("take"),
                read: on("read"),
            }
        }
        None => old.map_or(
            Perms {
                give: true,
                take: false,
                read: true,
            },
            |s| s.perms,
        ),
    };
    let mut caps = [0u64; 3];
    for (i, (k, was)) in [
        ("budget", old.map(|s| s.budget)),
        ("perGive", old.map(|s| s.per_give)),
        ("perDay", old.map(|s| s.per_day)),
    ]
    .into_iter()
    .enumerate()
    {
        let n = given(o, k).map_or(was.unwrap_or(0) as f64, |v| number(&v));
        if !whole(n) {
            return refuse("Budgets and caps are whole numbers of chips (0 for no cap).");
        }
        caps[i] = n as u64;
    }
    let expires = match o.get("expires") {
        None => old.and_then(|s| s.expires),
        Some(Value::Null) => None,
        Some(Value::String(s)) if s.is_empty() => None,
        Some(v) => {
            let n = number(v);
            if !whole(n) {
                return refuse("Not a valid expiry.");
            }
            Some(n as u64)
        }
    };
    Ok((perms, caps[0], caps[1], caps[2], expires))
}

impl Club {
    /// The owner adds a service by its key (`id`, `alg`, `key`, from its
    /// service line), with `{name, perms, budget, perGive, perDay, expires}`.
    pub fn add_service(
        &mut self,
        id: &str,
        alg: &str,
        key: &str,
        o: &Value,
        now: u64,
    ) -> Result<&Service, RuleError> {
        if self.owner_id.is_none() {
            return refuse("Only an always-on club can have services.");
        }
        let active = |s: &Service| s.status == ServiceStatus::Active;
        if self.services.get(id).is_some_and(active) {
            return refuse("That service is already added.");
        }
        if self.services.values().filter(|s| active(s)).count() >= MAX_SERVICES {
            return refuse(format!("A club can have at most {MAX_SERVICES} services."));
        }
        if self.members.contains_key(id) {
            return refuse("That key is a member’s, not a service’s.");
        }
        let name = cut(text_of(o.get("name")).trim(), MAX_SERVICE_NAME);
        if name.is_empty() {
            return refuse("Give the service a name.");
        }
        let (perms, budget, per_give, per_day, expires) = limits_from(o, None)?;
        let s = Service {
            id: id.into(),
            name,
            alg: alg.into(),
            key: Some(key.into()),
            status: ServiceStatus::Active,
            added_at: now,
            perms,
            budget,
            per_give,
            per_day,
            expires,
            gives: vec![],
            seen: BTreeMap::new(),
            done: BTreeMap::new(),
            failures: 0,
            last_failure: None,
            last_used: None,
            revoked_at: None,
        };
        self.services.insert(id.into(), s);
        Ok(&self.services[id])
    }

    fn active_service(&mut self, id: &str) -> Result<&mut Service, RuleError> {
        match self.services.get_mut(id) {
            Some(s) if s.status == ServiceStatus::Active => Ok(s),
            _ => refuse("No such service."),
        }
    }

    /// The owner changes a service's limits (`{name, perms, budget, perGive,
    /// perDay, expires}`, each optional); `addBudget` tops its budget up.
    pub fn update_service(&mut self, id: &str, o: &Value) -> Result<&Service, RuleError> {
        let add = given(o, "addBudget").map_or(0.0, |v| number(&v));
        if !whole(add) {
            return refuse("Add a whole number of chips.");
        }
        let s = self.active_service(id)?;
        let (perms, budget, per_give, per_day, expires) = limits_from(o, Some(s))?;
        s.perms = perms;
        s.budget = budget + add as u64;
        s.per_give = per_give;
        s.per_day = per_day;
        s.expires = expires;
        let name = cut(text_of(o.get("name")).trim(), MAX_SERVICE_NAME);
        if !name.is_empty() {
            s.name = name;
        }
        Ok(s)
    }

    /// Revoking: the key is forgotten at once. The record stays, for the
    /// history.
    pub fn revoke_service(&mut self, id: &str, now: u64) -> Result<&Service, RuleError> {
        let s = self.active_service(id)?;
        s.status = ServiceStatus::Revoked;
        s.revoked_at = Some(now);
        s.key = None;
        s.seen.clear();
        s.done.clear();
        Ok(s)
    }

    /// What the owner's page shows of each service.
    pub fn services_view(&self, now: u64) -> Vec<Value> {
        self.services
            .values()
            .map(|s| {
                json!({
                    "id": s.id, "name": s.name, "status": s.status, "perms": s.perms, "budget": s.budget,
                    "perGive": s.per_give, "perDay": s.per_day, "expires": s.expires,
                    "givenToday": s.given_today(now), "failures": s.failures, "lastFailure": s.last_failure,
                    "lastUsed": s.last_used, "addedAt": s.added_at,
                })
            })
            .collect()
    }

    fn fail(&mut self, id: Option<&str>, status: u16, reason: &str, now: u64) -> Refused {
        match id.and_then(|id| self.services.get_mut(id)) {
            Some(s) => {
                s.failures += 1;
                s.last_failure = Some(Failure {
                    at: now,
                    reason: reason.into(),
                });
            }
            None => {
                let count = self.service_refused.as_ref().map_or(0, |r| r.count) + 1;
                self.service_refused = Some(Refusals { count, at: now });
            }
        }
        Refused {
            status,
            reason: reason.into(),
        }
    }

    /// Checks a request's envelope from key `from` (`alg`, `key`, signed at
    /// `seq`, with `body`): from an active service of this club, by its key,
    /// for this club, fresh, and not seen before. `sig` is what the caller
    /// found wrong with the signature ("" if nothing), or None when it
    /// hasn't checked yet: then Ok(false) means "check it, and ask again".
    /// Ok(true): go ahead. A refusal is counted against the service.
    #[allow(clippy::too_many_arguments)]
    pub fn check_service_request(
        &mut self,
        from: &str,
        alg: &str,
        key: &str,
        seq: u64,
        body: &Value,
        sig: Option<&str>,
        now: u64,
    ) -> Result<bool, Refused> {
        let Some(s) = self.services.get(from) else {
            return Err(self.fail(None, 401, "This club has no such service.", now));
        };
        let why = if s.status != ServiceStatus::Active {
            Some((401, "This service was revoked.".to_string()))
        } else if s.key.as_deref() != Some(key) || s.alg != alg {
            Some((401, "Not this service’s key.".into()))
        } else {
            None
        };
        if let Some((status, why)) = why {
            return Err(self.fail(Some(from), status, &why, now));
        }
        let Some(sig) = sig else { return Ok(false) };
        let expires = s.expires;
        let why = if !sig.is_empty() {
            Some((401, format!("The signature doesn’t check out ({sig}).")))
        } else if body.get("type").and_then(Value::as_str) != Some("service")
            || body.get("club").and_then(Value::as_str) != Some(self.code.as_str())
        {
            Some((400, "Not a request for this club.".into()))
        } else if now.abs_diff(seq) > SKEW_MS {
            Some((
                401,
                "The request is too old or from the future: check the server’s clock.".into(),
            ))
        } else if expires.is_some_and(|e| now >= e) {
            Some((401, "This service has expired.".into()))
        } else {
            None
        };
        if let Some((status, why)) = why {
            return Err(self.fail(Some(from), status, &why, now));
        }
        let s = self.services.get_mut(from).expect("found above");
        // Each request once, while it's within the window.
        s.seen
            .retain(|_, at| now.saturating_sub(*at) <= 2 * SKEW_MS);
        if s.seen.contains_key(&seq.to_string()) {
            return Err(self.fail(Some(from), 401, "This request was already received.", now));
        }
        s.seen.insert(seq.to_string(), now);
        Ok(true)
    }

    /// What service `id`'s checked request (`body`) comes to. `member` is
    /// the member it names, found by the caller (by their id, or the one
    /// members see).
    pub fn prepare_service_request(
        &mut self,
        id: &str,
        body: &Value,
        member: Option<&str>,
        now: u64,
    ) -> Result<Prepared, Refused> {
        let Some(s) = self.services.get_mut(id) else {
            return refused(401, "This club has no such service.");
        };
        s.last_used = Some(now);
        let s = s.clone();
        let args = body
            .get("args")
            .filter(|a| a.is_object())
            .cloned()
            .unwrap_or(json!({}));
        let op = text_of(body.get("op"));
        let chips = |n: u64| n as f64 / 100.0;
        match op.as_str() {
            "status" => {
                return Ok(Prepared::Value(json!({
                    "name": s.name, "perms": s.perms, "budget": chips(s.budget), "perGive": chips(s.per_give),
                    "perDay": chips(s.per_day), "givenToday": chips(s.given_today(now)), "expires": s.expires,
                })));
            }
            "members" if !s.perms.read => {
                return refused(403, "This service may not list members.");
            }
            "members" => return Ok(Prepared::Members),
            "give" | "take" => {}
            _ => return refused(400, format!("No service action {op}.")),
        }
        if !(if op == "give" {
            s.perms.give
        } else {
            s.perms.take
        }) {
            return refused(403, format!("This service may not {op} chips."));
        }
        let key = args
            .get("key")
            .filter(|k| !k.is_null())
            .map(|k| cut(&text_of(Some(k)), MAX_KEY));
        if let Some(done) = key.as_ref().and_then(|k| s.done.get(k)) {
            let mut v = done.value.clone();
            v["repeat"] = true.into();
            return Ok(Prepared::Value(v));
        }
        let Some(member) = member.filter(|m| self.member(m).is_some()) else {
            return refused(404, "No such member.");
        };
        let n = args.get("amount").map_or(f64::NAN, number);
        let amount = (n * 100.0).round();
        if !(amount > 0.0 && amount <= 9e15 && (amount - n * 100.0).abs() <= 1e-6) {
            return refused(
                400,
                "The amount is a number of chips above 0, with at most two decimals.",
            );
        }
        let amount = amount as u64;
        if op == "give" {
            if s.per_give > 0 && amount > s.per_give {
                return refused(
                    403,
                    format!("At most {} in one give.", chip_text(s.per_give)),
                );
            }
            if s.per_day > 0 && s.given_today(now) + amount > s.per_day {
                return refused(
                    403,
                    format!(
                        "That would be over the {} a day this service may give.",
                        chip_text(s.per_day)
                    ),
                );
            }
            if amount > s.budget {
                return refused(
                    403,
                    format!("The service’s budget has {} left.", chip_text(s.budget)),
                );
            }
        }
        let said = cut(&text_of(args.get("note")), MAX_SERVICE_NOTE);
        Ok(Prepared::Move {
            op,
            member: member.into(),
            amount,
            note: if said.is_empty() {
                s.name.clone()
            } else {
                format!("{}: {said}", s.name)
            },
            said,
            key,
        })
    }

    /// Service `id` gave (or took back) `amount`, answered with `value`:
    /// its budget and the day's gives follow, and a key keeps the answer.
    pub fn service_done(
        &mut self,
        id: &str,
        op: &str,
        amount: u64,
        key: Option<&str>,
        value: &Value,
        now: u64,
    ) {
        let Some(s) = self.services.get_mut(id) else {
            return;
        };
        if op == "give" {
            s.budget = s.budget.saturating_sub(amount);
            s.gives.push(Given { at: now, amount });
            s.gives.retain(|g| now.saturating_sub(g.at) < DAY_MS);
        }
        if let Some(k) = key {
            s.done.insert(
                k.into(),
                Done {
                    at: now,
                    value: value.clone(),
                },
            );
            s.done.retain(|_, d| now.saturating_sub(d.at) <= DAY_MS);
            while s.done.len() > MAX_DONE {
                let oldest = s
                    .done
                    .iter()
                    .min_by_key(|(_, d)| d.at)
                    .map(|(k, _)| k.clone());
                oldest.map(|k| s.done.remove(&k));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn rng() -> StdRng {
        StdRng::seed_from_u64(7)
    }

    fn club() -> Club {
        Club::new("Friday Game", "ABCD-EFGH", "HOSTHOST", 1000, &mut rng()).unwrap()
    }

    fn join(c: &Club, invite: &str, name: &str) -> Value {
        json!({"type": "join", "club": c.code, "invite": invite, "name": name})
    }

    fn status(r: &Received) -> &str {
        r.replies[0]["body"]["status"].as_str().unwrap()
    }

    fn with_members(ids: &[&str]) -> Club {
        let mut c = club();
        for id in ids {
            let mut m = Member::new(id, &id.to_uppercase());
            m.status = Status::Member;
            c.members.insert(id.to_string(), m);
        }
        c
    }

    fn seat(c: &mut Club, id: &str, seat: Value) {
        c.members
            .get_mut(id)
            .unwrap()
            .other
            .insert("seat".into(), seat);
    }

    #[test]
    fn club_names_are_1_to_50_characters_trimmed() {
        assert_eq!(club_name("  Friday   Game ").unwrap(), "Friday Game");
        assert_eq!(club_name(&"x".repeat(50)).unwrap().len(), 50);
        assert!(
            club_name(&"x".repeat(51))
                .unwrap_err()
                .0
                .contains("at most 50")
        );
        assert!(club_name("   ").unwrap_err().0.contains("name"));
        assert_eq!(club().invite.len(), 12);
    }

    #[test]
    fn join_then_approve() {
        let mut c = club();
        let r = c.receive("ann", &join(&c, &c.invite.clone(), "Ann"), 2000);
        assert!(r.changed);
        assert_eq!(
            r.replies,
            vec![
                json!({"to": "ann", "body": {"type": "status", "club": "ABCD-EFGH", "name": "Friday Game", "status": "pending"}})
            ]
        );
        assert_eq!(c.members["ann"].requested_at, Some(2000));
        let a = c.approve("ann", 3000).unwrap();
        assert_eq!(a["body"]["status"], "member");
        assert_eq!(c.members["ann"].joined_at, Some(3000));
        assert_eq!(c.approve("ann", 3000), None, "only once");
    }

    #[test]
    fn a_wrong_or_revoked_invite_is_refused_and_members_are_unaffected() {
        let mut c = club();
        let invite = c.invite.clone();
        c.receive("ann", &join(&c, &invite, "Ann"), 0);
        c.approve("ann", 0);
        let bad = c.receive("bob", &join(&c, "nope", "Bob"), 0);
        assert_eq!(status(&bad), "bad_invite");
        assert!(!c.members.contains_key("bob"));
        c.revoke_invite(&mut StdRng::seed_from_u64(8));
        assert_ne!(c.invite, invite);
        assert_eq!(
            status(&c.receive("bob", &join(&c, &invite, "Bob"), 0)),
            "bad_invite"
        );
        assert_eq!(
            status(&c.receive("ann", &join(&c, &invite, "Ann"), 0)),
            "member"
        );
    }

    #[test]
    fn declined_and_removed_can_ask_again() {
        let mut c = club();
        let invite = c.invite.clone();
        c.receive("ann", &join(&c, &invite, "Ann"), 0);
        assert_eq!(c.decline("ann", 0).unwrap()["body"]["status"], "declined");
        assert_eq!(c.approve("ann", 0), None);
        assert_eq!(
            status(&c.receive("ann", &join(&c, &invite, "Ann"), 0)),
            "pending"
        );
        c.approve("ann", 0);
        assert_eq!(c.remove("ann", 0).unwrap()["body"]["status"], "removed");
        assert_eq!(c.remove("ann", 0), None);
    }

    #[test]
    fn a_removed_player_asking_again_is_remembered() {
        let mut c = club();
        let invite = c.invite.clone();
        c.receive("ann", &join(&c, &invite, "Ann"), 0);
        c.approve("ann", 10);
        c.remove("ann", 20);
        c.receive("ann", &join(&c, &invite, "Ann"), 30);
        let ann = &c.members["ann"];
        assert!(ann.is(Status::Pending));
        assert_eq!(ann.removed_at, Some(20));
        // Let in again, it's still on record.
        c.approve("ann", 40);
        assert_eq!(c.members["ann"].removed_at, Some(20));
        // A club saved before this kept only when their status changed.
        c.remove("ann", 50);
        c.members.get_mut("ann").unwrap().removed_at = None;
        c.receive("ann", &join(&c, &invite, "Ann"), 60);
        assert_eq!(c.members["ann"].removed_at, Some(50));
    }

    #[test]
    fn someone_seated_is_not_removed() {
        let mut c = with_members(&["ann"]);
        seat(&mut c, "ann", json!({"table": "t1", "stack": 100}));
        assert!(!c.removable("ann"));
        seat(&mut c, "ann", Value::Null);
        assert!(c.removable("ann"));
    }

    #[test]
    fn asking_with_the_code_alone_needs_the_club_to_allow_it() {
        let mut c = club();
        let no = c.receive("ann", &join(&c, "", "Ann"), 0);
        assert_eq!(
            no.replies,
            vec![
                json!({"to": "ann", "body": {"type": "status", "club": "ABCD-EFGH", "status": "invite_only"}})
            ]
        );
        assert!(!no.changed);
        c.code_join = Some(true);
        assert_eq!(
            status(&c.receive("ann", &join(&c, "", "Ann"), 0)),
            "pending"
        );
        c.approve("ann", 0);
        assert_eq!(
            status(&c.receive("bob", &join(&c, "nope", "Bob"), 0)),
            "bad_invite"
        );
        c.code_join = Some(false);
        assert_eq!(status(&c.receive("ann", &join(&c, "", "Ann"), 0)), "member");
    }

    #[test]
    fn an_agents_invite_brings_their_recruits() {
        let mut c = with_members(&["ag"]);
        let ag = c.members.get_mut("ag").unwrap();
        ag.agent = Some("yes".into());
        ag.agent_invite = Some("AGENT".into());
        c.receive("pat", &join(&c, "AGENT", "Pat"), 0);
        assert_eq!(c.members["pat"].agent_of.as_deref(), Some("ag"));
    }

    #[test]
    fn hello_reports_status_updates_the_name_and_asks_for_the_account() {
        let mut c = club();
        let invite = c.invite.clone();
        c.receive("ann", &join(&c, &invite, "Ann"), 0);
        let r = c.receive(
            "ann",
            &json!({"type": "hello", "club": c.code, "name": "Annie"}),
            0,
        );
        assert_eq!(status(&r), "pending");
        assert_eq!(r.account, None, "not a member yet");
        assert_eq!(c.members["ann"].name, "Annie");
        c.approve("ann", 0);
        let r = c.receive(
            "ann",
            &json!({"type": "hello", "club": c.code, "since": 5, "ledger": "links"}),
            0,
        );
        assert_eq!(r.account, Some(5.0));
        let r = c.receive(
            "ann",
            &json!({"type": "hello", "club": c.code, "since": 5}),
            0,
        );
        assert_eq!(r.account, Some(0.0), "an older copy gets every entry");
        let u = c.receive("eve", &json!({"type": "hello", "club": c.code}), 0);
        assert_eq!(status(&u), "unknown");
    }

    #[test]
    fn names_in_a_club_are_unique_however_written() {
        let mut c = club();
        let invite = c.invite.clone();
        c.receive("ann", &join(&c, &invite, "Tom Reyes"), 0);
        for name in [
            "Tom Reyes",
            "tom reyes",
            "TomReyes",
            "Tom.Reyes",
            "Tóm Réyes",
            "T0m Reyes",
            "Tоm Reyes",
        ] {
            assert_eq!(name_key(name), name_key("Tom Reyes"), "{name}");
            let r = c.receive("bob", &join(&c, &invite, name), 0);
            assert_eq!(r.reason.as_deref(), Some("name taken"), "{name}");
            assert_eq!(
                r.replies[0]["body"],
                json!({"type": "status", "club": "ABCD-EFGH", "name": "Friday Game", "status": "unknown", "nameTaken": name})
            );
            assert!(!c.members.contains_key("bob"));
        }
        assert_ne!(name_key("Tom Reyes"), name_key("Tom Reyes Jr"));
        assert_eq!(
            status(&c.receive("bob", &join(&c, &invite, "Bob"), 0)),
            "pending"
        );
        c.approve("bob", 0);
        let no = c.receive(
            "bob",
            &json!({"type": "hello", "club": c.code, "name": "Tom  Reyes"}),
            0,
        );
        assert_eq!(no.replies[0]["body"]["nameTaken"], "Tom Reyes");
        assert_eq!(c.members["bob"].name, "Bob");
        assert!(!no.changed);
        let mine = c.receive(
            "bob",
            &json!({"type": "hello", "club": c.code, "name": "BOB"}),
            0,
        );
        assert_eq!(mine.replies[0]["body"].get("nameTaken"), None);
        assert_eq!(c.members["bob"].name, "BOB");
        // Someone declined no longer holds their name.
        c.decline("ann", 0);
        assert_eq!(
            status(&c.receive("cy", &join(&c, &invite, "Tom Reyes"), 0)),
            "pending"
        );
    }

    #[test]
    fn messages_for_another_club_or_from_non_members_are_refused() {
        let mut c = with_members(&["p"]);
        let r = c.receive(
            "p",
            &json!({"type": "help_ask", "club": "ZZZZ", "text": "hi"}),
            0,
        );
        assert_eq!(r.reason.as_deref(), Some("wrong club"));
        let r = c.receive(
            "p",
            &json!({"type": "help_ask", "club": c.code, "text": "stuck", "table": "T1"}),
            0,
        );
        assert_eq!(
            r.help,
            Some(json!({"from": "p", "text": "stuck", "table": "T1"}))
        );
        let r = c.receive(
            "q",
            &json!({"type": "lobby_say", "club": c.code, "text": "hi"}),
            0,
        );
        assert_eq!(r.reason.as_deref(), Some("not a member"));
        let r = c.receive(
            "p",
            &json!({"type": "table", "club": c.code, "table": "t1"}),
            0,
        );
        assert_eq!(r.reason.as_deref(), Some("bad table command"));
        let r = c.receive(
            "p",
            &json!({"type": "tourney", "club": c.code, "tourney": "x", "op": "enter"}),
            0,
        );
        assert_eq!(
            r.tourney,
            Some(json!({"id": "x", "op": "enter", "from": "p"}))
        );
        let r = c.receive("p", &json!({"type": "owner_cmd", "club": c.code}), 0);
        assert_eq!(r.reason.as_deref(), Some("not the owner"));
        let r = c.receive(
            "p",
            &json!({"type": "chip_request", "club": c.code, "amount": 5}),
            0,
        );
        assert_eq!(r.chips.unwrap()["amount"], 5);
        let r = c.receive("p", &json!({"type": "nonsense", "club": c.code}), 0);
        assert_eq!(r.reason.as_deref(), Some("unknown message"));
    }

    #[test]
    fn members_carry_other_fields_through() {
        let c: Club = serde_json::from_value(json!({
            "code": "C", "name": "N", "hostId": "H", "invite": "I",
            "members": {"a": {"id": "a", "name": "A", "status": "member", "balance": 500, "seat": null, "left": {}}},
        }))
        .unwrap();
        let back = serde_json::to_value(&c).unwrap();
        assert_eq!(back["members"]["a"]["balance"], 500);
        assert_eq!(back["members"]["a"]["seat"], Value::Null);
        assert_eq!(back["members"]["a"].get("agentOf"), None);
    }

    #[test]
    fn help_ask_reply_handled_one_open_request_each() {
        let mut c = with_members(&["p", "a"]);
        let mut r = rng();
        assert!(
            c.ask_help("x", "hi", "", 0, &mut r)
                .unwrap_err()
                .0
                .contains("Only members")
        );
        assert!(
            c.ask_help("p", "   ", "", 0, &mut r)
                .unwrap_err()
                .0
                .contains("Say what")
        );
        assert_eq!(
            c.ask_help("p", "Table 2 is   stuck", "NLH - 1/2", 1, &mut r)
                .unwrap()
                .text,
            "Table 2 is stuck"
        );
        let h2 = c
            .ask_help("p", "Can someone top me up?", "", 2, &mut r)
            .unwrap()
            .id
            .clone();
        let open: Vec<_> = c
            .help
            .iter()
            .filter(|h| h.handled.is_none())
            .map(|h| h.id.clone())
            .collect();
        assert_eq!(open, vec![h2.clone()]);
        assert!(
            c.answer_help("p", &h2, "me", true, 3)
                .unwrap_err()
                .0
                .contains("Only the club’s manager")
        );
        c.answer_help("HOSTHOST", &h2, "On it", false, 3).unwrap();
        assert!(c.help[0].handled.is_none());
        c.answer_help("HOSTHOST", &h2, "", true, 4).unwrap();
        assert_eq!(
            c.help_body("p", &[])["mine"],
            json!([{"id": h2, "text": "Can someone top me up?", "at": 2, "reply": "On it", "handled": true, "by": "Manager"}])
        );
    }

    #[test]
    fn the_help_list_keeps_the_last_ones() {
        let ids: Vec<String> = (0..60).map(|i| format!("m{i}")).collect();
        let mut c = with_members(&ids.iter().map(String::as_str).collect::<Vec<_>>());
        let mut r = rng();
        for (i, id) in ids.iter().enumerate() {
            c.ask_help(id, &format!("help {i}"), "", i as u64, &mut r)
                .unwrap();
        }
        assert_eq!(c.help.len(), MAX_HELP);
        assert_eq!(c.help[0].text, "help 10");
    }

    #[test]
    fn whos_around_to_help() {
        let mut c = with_members(&["a", "b"]);
        let on = |ids: &[&str]| ids.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(c.staff_online(&on(&["a"]), false), Vec::<Value>::new());
        assert_eq!(
            c.staff_online(&[], true),
            vec![json!({"name": "Manager", "role": "host"})]
        );
        c.owner_id = Some("OWNER".into());
        assert!(c.staff_online(&on(&["b"]), false).is_empty());
        assert_eq!(c.staff_online(&on(&["OWNER"]), false).len(), 1);
    }

    #[test]
    fn members_and_the_host_chat_and_old_and_extra_lines_go() {
        let mut c = with_members(&["p", "q"]);
        c.members.insert("x".into(), Member::new("x", "X"));
        let mut r = rng();
        assert!(
            c.say_in_lobby("x", "hi", 0, &mut r)
                .unwrap_err()
                .0
                .contains("Only members")
        );
        assert!(
            c.say_in_lobby("p", "  \n ", 0, &mut r)
                .unwrap_err()
                .0
                .contains("Say something")
        );
        let a = c
            .say_in_lobby("p", "anyone   up for\nPLO?", 1, &mut r)
            .unwrap();
        assert_eq!(
            (a.name.as_str(), a.text.as_str(), a.host),
            ("P", "anyone up for PLO?", false)
        );
        let h = c
            .say_in_lobby("HOSTHOST", "Opening a table at 8", 2, &mut r)
            .unwrap();
        assert_eq!((h.name.as_str(), h.host), ("Manager", true));
        for i in 0..LOBBY_KEEP + 5 {
            c.say_in_lobby("q", &format!("line {i}"), 10 + i as u64, &mut r)
                .unwrap();
        }
        assert_eq!(c.lobby.len(), LOBBY_KEEP);
        assert_eq!(
            c.lobby.last().unwrap().text,
            format!("line {}", LOBBY_KEEP + 4)
        );
        c.say_in_lobby("p", "morning", 10 + LOBBY_MAX_AGE_MS + 100, &mut r)
            .unwrap();
        assert_eq!(
            c.lobby.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(),
            ["morning"]
        );
        let body = c.lobby_body(&[], false, 10 + LOBBY_MAX_AGE_MS + 100);
        assert_eq!(
            body["chat"][0].get("from"),
            None,
            "members don’t get each other’s ids"
        );
    }

    #[test]
    fn whos_online_the_host_first_then_by_name_with_their_table() {
        let mut c = with_members(&["zed", "amy", "bo"]);
        c.members
            .insert("pend".into(), Member::new("pend", "Pending"));
        c.tables = vec![serde_json::from_value(json!({
            "id": "t1", "name": "NLH 1/2", "game": "nlhe", "sb": 1, "bb": 2, "minBB": 40, "maxBB": 100, "seats": 9, "status": "open",
        }))
        .unwrap()];
        seat(&mut c, "zed", json!({"table": "t1", "stack": 100}));
        let on: Vec<String> = ["zed", "amy", "bo", "pend"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let list = c.online_list(&on, false);
        let rows: Vec<(String, String)> = list
            .iter()
            .map(|x| {
                (
                    x["name"].as_str().unwrap().into(),
                    x["table"].as_str().unwrap().into(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            [("AMY", ""), ("BO", ""), ("ZED", "NLH 1/2")]
                .map(|(a, b)| (a.to_string(), b.to_string()))
        );
        assert_eq!(c.online_list(&on, true)[0]["role"], "host");
        c.owner_id = Some("OWNER".into());
        let mut olive = Member::new("OWNER", "Olive");
        olive.status = Status::Member;
        c.members.insert("OWNER".into(), olive);
        let list = c.online_list(&["OWNER".into(), "amy".into()], false);
        assert_eq!(list[0]["name"], "Olive");
        assert_eq!(list[0]["role"], "host");
        assert_eq!(list.len(), 2);
    }

    const T: u64 = 1_800_000_000_000;

    #[test]
    fn club_info_is_trimmed_and_checked_and_says_when_each_part_changed() {
        let a = clean_info(
            &json!({
                "about": "  Friday   nights\r\n\r\n\r\n\r\nat Sam’s  ",
                "rules": "x".repeat(MAX_INFO_TEXT + 50),
                "events": [{"at": T + 2, "text": "Second"}, {"at": T + 1, "text": " First\nline "}, {"at": "soon", "text": "Bad"}, {"at": T, "text": ""}],
                "extra": "<script>",
            }),
            None,
            T,
        );
        assert_eq!(a.about, "Friday nights\n\nat Sam’s");
        assert_eq!(a.rules.len(), MAX_INFO_TEXT);
        assert_eq!(
            a.events,
            vec![
                Event {
                    at: T + 1,
                    text: "First line".into()
                },
                Event {
                    at: T + 2,
                    text: "Second".into()
                }
            ]
        );
        let at = |pairs: &[(&str, u64)]| {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), *v))
                .collect::<BTreeMap<_, _>>()
        };
        assert_eq!(a.at, at(&[("about", T), ("events", T), ("rules", T)]));
        let mut again = serde_json::to_value(&a).unwrap();
        again["rules"] = "No phones at the table".into();
        let b = clean_info(&again, Some(&a), T + 100);
        assert_eq!(b.at, at(&[("about", T), ("events", T), ("rules", T + 100)]));
        let many: Vec<Value> = (0..30)
            .map(|i| json!({"at": T + i, "text": format!("E{i}")}))
            .collect();
        assert_eq!(
            clean_info(&json!({"events": many}), None, 0).events.len(),
            MAX_EVENTS
        );
    }

    #[test]
    fn info_text_keeps_lines_and_paragraphs() {
        assert_eq!(info_text("a  b\n c\n\n\n\nd", 100), "a b\n c\n\nd");
        assert_eq!(info_text("- one\n- two", 100), "- one\n- two");
    }

    fn spec(v: Value) -> Value {
        v
    }

    #[test]
    fn tables_defaults_validation_and_what_members_are_told() {
        let mut c = club();
        let mut r = rng();
        let t = c
            .new_table(&spec(json!({"game": "plo4", "sb": 1, "bb": 2, "minBB": 50, "maxBB": 100, "seats": 6})), 0, None, &mut r)
            .unwrap()
            .clone();
        assert_eq!(t.name, "PLO - 0.01/0.02");
        assert_eq!(t.limits(), (100, 200));
        let err = |c: &mut Club, s: Value| c.new_table(&s, 0, None, &mut rng()).unwrap_err().0;
        assert!(err(&mut c, json!({"game": "stud"})).contains("Pick a game"));
        assert!(err(&mut c, json!({"sb": 5, "bb": 2})).contains("big blind"));
        assert!(err(&mut c, json!({"minBB": 100, "maxBB": 50})).contains("maximum buy-in"));
        assert!(err(&mut c, json!({"seats": 10})).contains("2 to 9"));
        assert!(err(&mut c, json!({"sb": 1.5})).contains("big blind"));
        c.new_table(
            &json!({"name": "High stakes", "sb": 5, "bb": 10}),
            0,
            None,
            &mut r,
        )
        .unwrap();
        let body = c.tables_body(&BTreeMap::from([(t.id.clone(), 2)]));
        assert_eq!(body["tables"].as_array().unwrap().len(), 2);
        assert_eq!(
            body["tables"][0],
            json!({"id": t.id, "name": "PLO - 0.01/0.02", "game": "plo4", "sb": 1, "bb": 2, "min": 100, "max": 200, "seats": 6, "taken": 2,
                   "key": "plo4:1/2:6", "stakes": "micro"})
        );
        assert_eq!(body["lists"], json!({}));
    }

    #[test]
    fn stakes_bands_by_the_big_blind() {
        let band = |bb| Stakes::of(bb).name();
        assert_eq!(
            [band(50), band(99), band(100), band(200), band(300)],
            ["Micro", "Micro", "Low", "Low", "Low"]
        );
        assert_eq!(
            [band(500), band(1000), band(1001), band(2500)],
            ["Medium", "Medium", "High", "High"]
        );
    }

    #[test]
    fn default_names_are_numbered_when_an_open_table_has_the_name() {
        let mut c = club();
        let mut r = rng();
        let blinds = json!({"game": "nlhe", "sb": 100, "bb": 200});
        assert_eq!(
            c.new_table(&blinds, 0, None, &mut r).unwrap().name,
            "NLH - 1/2"
        );
        assert_eq!(
            c.new_table(&blinds, 0, None, &mut r).unwrap().name,
            "NLH - 1/2 - #2"
        );
        assert_eq!(
            c.new_table(&blinds, 0, None, &mut r).unwrap().name,
            "NLH - 1/2 - #3"
        );
        assert_eq!(
            c.new_table(
                &json!({"game": "plo6", "sb": 25, "bb": 50}),
                0,
                None,
                &mut r
            )
            .unwrap()
            .name,
            "PLO6 - 0.25/0.5"
        );
        c.tables[0].status = TableStatus::Closed;
        assert_eq!(
            c.new_table(&blinds, 0, None, &mut r).unwrap().name,
            "NLH - 1/2"
        );
        assert_eq!(c.default_table_name("plo5", 500, 1000), "PLO5 - 5/10");
        let named = json!({"name": "Main", "game": "nlhe", "sb": 100, "bb": 200});
        assert_eq!(c.new_table(&named, 0, None, &mut r).unwrap().name, "Main");
    }

    #[test]
    fn seats_no_more_than_the_game_can_deal_to() {
        let mut c = club();
        let mut r = rng();
        let err = c
            .new_table(&json!({"game": "plo6", "seats": 9}), 0, None, &mut r)
            .unwrap_err();
        assert!(err.0.contains("An Omaha 6 table seats 2 to 6"), "{err}");
        assert!(
            c.new_table(&json!({"game": "plo5", "seats": 9}), 0, None, &mut r)
                .is_err()
        );
        for (g, n) in [("nlhe", 9), ("plo4", 8), ("plo5", 6), ("plo6", 6)] {
            assert_eq!(
                c.new_table(&json!({"game": g}), 0, None, &mut r)
                    .unwrap()
                    .seats,
                n
            );
        }
    }

    #[test]
    fn sit_out_and_call_time_default_to_the_clubs() {
        let mut c = club();
        let mut r = rng();
        assert_eq!(
            c.new_table(&json!({}), 0, None, &mut r)
                .unwrap()
                .sit_out_mins,
            10
        );
        c.sit_out_mins = Some(30.into());
        assert_eq!(
            c.new_table(&json!({}), 0, None, &mut r)
                .unwrap()
                .sit_out_mins,
            30
        );
        assert_eq!(
            c.new_table(&json!({"sitOutMins": 0}), 0, None, &mut r)
                .unwrap()
                .sit_out_mins,
            0
        );
        assert!(
            c.new_table(&json!({"sitOutMins": -1}), 0, None, &mut r)
                .unwrap_err()
                .0
                .contains("sit-out limit")
        );
        assert_eq!(
            c.new_table(&json!({}), 0, None, &mut r).unwrap().stay_hands,
            0
        );
        for bad in [-1, 501] {
            assert!(
                c.new_table(&json!({"stayHands": bad}), 0, None, &mut r)
                    .unwrap_err()
                    .0
                    .contains("0 (off) to 500")
            );
        }
    }

    #[test]
    fn a_plan_limits_the_tables_open_at_once() {
        let mut c = club();
        let mut r = rng();
        c.new_table(&json!({}), 0, Some(2), &mut r).unwrap();
        c.new_table(&json!({}), 0, Some(2), &mut r).unwrap();
        assert!(
            c.new_table(&json!({}), 0, Some(2), &mut r)
                .unwrap_err()
                .0
                .contains("2 tables open at once")
        );
        c.tables[0].status = TableStatus::Closing;
        assert!(
            c.new_table(&json!({}), 0, Some(2), &mut r).is_err(),
            "closing still counts"
        );
        c.tables[0].status = TableStatus::Closed;
        c.new_table(&json!({}), 0, Some(2), &mut r).unwrap();
        assert!(
            c.new_table(&json!({}), 0, Some(1), &mut r)
                .unwrap_err()
                .0
                .contains("1 table open at once")
        );
        assert!(
            c.new_table(&json!({}), 0, Some(0), &mut r)
                .unwrap_err()
                .0
                .contains("can’t open tables")
        );
        c.new_table(&json!({}), 0, None, &mut r).unwrap();
        assert_eq!(c.open_tables().count(), 3);
    }

    #[test]
    fn who_can_join_members_open_tables_one_table_at_a_time() {
        let mut c = with_members(&["ann"]);
        let mut r = rng();
        let t1 = c.new_table(&json!({}), 0, None, &mut r).unwrap().id.clone();
        let t2 = c.new_table(&json!({}), 0, None, &mut r).unwrap().id.clone();
        assert_eq!(c.can_join("ann", &t1, None), "");
        assert!(c.can_join("eve", &t1, None).contains("members"));
        assert!(c.can_join("ann", &t1, Some(&t2)).contains("another table"));
        seat(
            &mut c,
            "ann",
            json!({"table": t1, "stack": 100, "put": 100}),
        );
        assert!(c.can_join("ann", &t2, None).contains("another table"));
        c.tables[1].status = TableStatus::Closing;
        assert!(c.can_join("ann", &t2, None).contains("isn’t open"));
    }

    #[test]
    fn hit_and_run_once_up_a_player_plays_the_tables_hands() {
        let mut c = with_members(&["ann", "bob"]);
        let mut r = rng();
        let t = c
            .new_table(&json!({"stayHands": 10}), 0, None, &mut r)
            .unwrap()
            .id
            .clone();
        assert_eq!(
            c.tables_body(&BTreeMap::new())["tables"][0]["stayHands"],
            10
        );
        let stacks = |c: &mut Club, ann: u64, bob: u64, put_bob: u64, hand: u64| {
            for (id, stack, put) in [("ann", ann, 100), ("bob", bob, put_bob)] {
                let s = c
                    .members
                    .get_mut(id)
                    .unwrap()
                    .other
                    .entry("seat")
                    .or_insert(json!({"table": t}));
                s["stack"] = stack.into();
                s["put"] = put.into();
            }
            c.note_stacks(&["ann".into(), "bob".into()], hand);
        };
        stacks(&mut c, 100, 100, 100, 0);
        assert_eq!(c.hands_to_stay("ann", &t, 0), 0);
        stacks(&mut c, 140, 60, 100, 3);
        assert_eq!(c.hands_to_stay("ann", &t, 3), 10);
        assert_eq!(c.hands_to_stay("bob", &t, 3), 0, "down: free to go");
        stacks(&mut c, 150, 50, 100, 9);
        assert_eq!(
            c.hands_to_stay("ann", &t, 9),
            4,
            "still up: the count keeps going"
        );
        // A top-up counts as chips brought.
        stacks(&mut c, 120, 130, 150, 10);
        assert_eq!(c.hands_to_stay("bob", &t, 10), 0);
        stacks(&mut c, 120, 160, 150, 10);
        assert_eq!(c.hands_to_stay("bob", &t, 10), 10);
        stacks(&mut c, 100, 150, 150, 11);
        assert_eq!(c.hands_to_stay("ann", &t, 11), 0);
        stacks(&mut c, 110, 140, 150, 12);
        assert_eq!(c.hands_to_stay("ann", &t, 12), 10, "the count starts over");
        assert_eq!(c.hands_to_stay("ann", &t, 22), 0, "played them");
    }

    #[test]
    fn closing_tables_close_unless_kept() {
        let mut c = club();
        let mut r = rng();
        let a = c
            .new_table(&json!({"name": "A"}), 0, None, &mut r)
            .unwrap()
            .id
            .clone();
        c.new_table(&json!({"name": "B"}), 0, None, &mut r).unwrap();
        c.tables[0].status = TableStatus::Closing;
        c.tables[1].status = TableStatus::Closing;
        c.close_closing(&[a]);
        assert_eq!(c.tables[0].status, TableStatus::Closing);
        assert_eq!(c.tables[1].status, TableStatus::Closed);
    }

    #[test]
    fn reserved_seats_are_held_one_at_a_time_and_lapse() {
        let mut c = with_members(&["ann", "bob"]);
        let mut r = rng();
        let a = c
            .new_table(&json!({"name": "A", "seats": 2}), 0, None, &mut r)
            .unwrap()
            .id
            .clone();
        let b = c
            .new_table(&json!({"name": "B"}), 0, None, &mut r)
            .unwrap()
            .id
            .clone();
        seat(&mut c, "ann", json!({"table": b, "stack": 100}));
        assert!(
            c.reserve_seat("eve", &a, 0, None, None, 0, HOLD_MS)
                .unwrap_err()
                .0
                .contains("members")
        );
        assert!(
            c.reserve_seat("ann", &b, 0, Some(&b), None, 0, HOLD_MS)
                .unwrap_err()
                .0
                .contains("already")
        );
        c.reserve_seat("ann", &a, 1, Some(&b), None, 0, HOLD_MS)
            .unwrap();
        assert_eq!(c.held_seats(&a, 0, ""), 1);
        assert!(
            c.reserve_seat("bob", &a, 1, None, None, 0, HOLD_MS)
                .unwrap_err()
                .0
                .contains("full")
        );
        // Still at B: held past the time; gone from B, it lapses.
        assert_eq!(c.reservations(&a, HOLD_MS * 5).len(), 1);
        assert_eq!(c.next_lapse(), None);
        seat(&mut c, "ann", Value::Null);
        assert!(c.hold_from(&a, "ann", 1000, HOLD_MS));
        assert_eq!(c.next_lapse(), Some(1000 + HOLD_MS));
        let gone = c.prune_reservations(1000 + HOLD_MS);
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].0, a);
        assert!(c.waitlist.is_empty());
        // A new reservation replaces the old one.
        c.reserve_seat("bob", &a, 0, None, None, 0, HOLD_MS)
            .unwrap();
        c.reserve_seat("bob", &b, 0, None, None, 0, HOLD_MS)
            .unwrap();
        assert!(c.reservations(&a, 0).is_empty());
        assert!(c.drop_reservation(&b, "bob"));
        assert!(!c.drop_reservation(&b, "bob"));
    }

    fn always_on() -> Club {
        let mut c = with_members(&["ann", "bob"]);
        c.owner_id = Some("OWNER".into());
        c
    }

    fn give(c: &mut Club, args: Value, now: u64) -> Result<Prepared, Refused> {
        let member = args["member"].as_str().map(String::from);
        c.prepare_service_request(
            "SVC",
            &json!({"op": "give", "args": args}),
            member.as_deref(),
            now,
        )
    }

    #[test]
    fn the_owner_adds_a_service_to_an_always_on_club_named_once() {
        let mut c = with_members(&["ann"]);
        let o = json!({"name": "Rewards server", "budget": 100_000, "perGive": 30_000, "perDay": 50_000});
        assert!(
            c.add_service("SVC", "ed25519", "KEY", &o, 0)
                .unwrap_err()
                .0
                .contains("always-on")
        );
        c.owner_id = Some("OWNER".into());
        assert!(
            c.add_service("SVC", "ed25519", "KEY", &json!({"name": " "}), 0)
                .unwrap_err()
                .0
                .contains("name")
        );
        assert!(
            c.add_service("ann", "ed25519", "KEY", &json!({"name": "Ann?"}), 0)
                .unwrap_err()
                .0
                .contains("member’s")
        );
        let s = c.add_service("SVC", "ed25519", "KEY", &o, 0).unwrap();
        assert_eq!(
            s.perms,
            Perms {
                give: true,
                take: false,
                read: true
            }
        );
        assert!(
            c.add_service("SVC", "ed25519", "KEY", &json!({"name": "Again"}), 0)
                .unwrap_err()
                .0
                .contains("already added")
        );
    }

    #[test]
    fn a_service_gives_within_its_budget_and_caps() {
        let mut c = always_on();
        c.add_service(
            "SVC",
            "ed25519",
            "KEY",
            &json!({"name": "Rewards", "budget": 100_000, "perGive": 30_000, "perDay": 50_000}),
            0,
        )
        .unwrap();
        let Prepared::Move { amount, note, .. } = give(
            &mut c,
            json!({"member": "ann", "amount": 200, "note": "League week 1"}),
            0,
        )
        .unwrap() else {
            panic!("a move")
        };
        assert_eq!((amount, note.as_str()), (20_000, "Rewards: League week 1"));
        c.service_done("SVC", "give", amount, None, &json!({}), 0);
        let Prepared::Move { amount, .. } =
            give(&mut c, json!({"member": "bob", "amount": 100.5}), 0).unwrap()
        else {
            panic!("a move")
        };
        assert_eq!(amount, 10_050);
        c.service_done("SVC", "give", amount, None, &json!({}), 0);
        let why = |r: Result<Prepared, Refused>| {
            let e = r.unwrap_err();
            (e.status, e.reason)
        };
        let (s, r) = why(give(&mut c, json!({"member": "ann", "amount": 301}), 0));
        assert_eq!(s, 403);
        assert!(r.contains("At most 300"), "{r}");
        assert!(
            why(give(&mut c, json!({"member": "ann", "amount": 250}), 0))
                .1
                .contains("500 a day")
        );
        assert_eq!(
            why(give(&mut c, json!({"member": "ann", "amount": 0}), 0)).0,
            400
        );
        assert!(
            why(give(&mut c, json!({"member": "ann", "amount": 1.234}), 0))
                .1
                .contains("two decimals")
        );
        assert_eq!(
            why(give(&mut c, json!({"member": "nobody", "amount": 5}), 0)).0,
            404
        );
        assert_eq!(c.services_view(0)[0]["givenToday"], 30_050);
        c.update_service("SVC", &json!({"perDay": 0, "perGive": 0}))
            .unwrap();
        assert!(
            why(give(&mut c, json!({"member": "ann", "amount": 700}), 0))
                .1
                .contains("budget has 699.50 left")
        );
        c.update_service("SVC", &json!({"addBudget": 30_000}))
            .unwrap();
        assert_eq!(c.services["SVC"].budget, 99_950);
    }

    #[test]
    fn a_give_with_a_key_is_answered_again_not_paid_again() {
        let mut c = always_on();
        c.add_service(
            "SVC",
            "ed25519",
            "KEY",
            &json!({"name": "Rewards", "budget": 100_000}),
            0,
        )
        .unwrap();
        let args = json!({"member": "ann", "amount": 100, "key": "week-1-ann"});
        let Prepared::Move { amount, key, .. } = give(&mut c, args.clone(), 0).unwrap() else {
            panic!("a move")
        };
        c.service_done(
            "SVC",
            "give",
            amount,
            key.as_deref(),
            &json!({"entry": 7}),
            0,
        );
        assert_eq!(
            give(&mut c, args, 1).unwrap(),
            Prepared::Value(json!({"entry": 7, "repeat": true}))
        );
    }

    #[test]
    fn requests_copied_stale_misaddressed_or_by_another_key_are_refused_and_counted() {
        let mut c = always_on();
        c.add_service("SVC", "ed25519", "KEY", &json!({"name": "Rewards"}), 0)
            .unwrap();
        let body = json!({"type": "service", "club": c.code, "op": "status"});
        let now = 10 * SKEW_MS;
        assert_eq!(
            c.check_service_request("SVC", "ed25519", "KEY", now, &body, None, now),
            Ok(false),
            "check the signature"
        );
        assert_eq!(
            c.check_service_request("SVC", "ed25519", "KEY", now, &body, Some(""), now),
            Ok(true)
        );
        let again = c
            .check_service_request("SVC", "ed25519", "KEY", now, &body, Some(""), now)
            .unwrap_err();
        assert!(again.reason.contains("already received"));
        let stale = c
            .check_service_request("SVC", "ed25519", "KEY", 1, &body, Some(""), now)
            .unwrap_err();
        assert!(stale.reason.contains("too old"));
        let bad = c
            .check_service_request(
                "SVC",
                "ed25519",
                "KEY",
                now + 1,
                &body,
                Some("bad signature"),
                now,
            )
            .unwrap_err();
        assert!(bad.reason.contains("signature"));
        let other = json!({"type": "service", "club": "OTHR", "op": "status"});
        assert_eq!(
            c.check_service_request("SVC", "ed25519", "KEY", now + 2, &other, Some(""), now)
                .unwrap_err()
                .status,
            400
        );
        let forged = c
            .check_service_request("SVC", "ed25519", "OTHER", now, &body, None, now)
            .unwrap_err();
        assert!(forged.reason.contains("Not this service’s key"));
        assert!(
            c.check_service_request("NOPE", "ed25519", "K", now, &body, None, now)
                .unwrap_err()
                .reason
                .contains("no such service")
        );
        assert_eq!(c.service_refused.as_ref().unwrap().count, 1);
        assert_eq!(c.services["SVC"].failures, 5);
    }

    #[test]
    fn permissions_expiry_and_revoking() {
        let mut c = always_on();
        c.add_service("SVC", "ed25519", "KEY", &json!({"name": "Rewards", "budget": 100_000, "perms": {"give": true, "take": false, "read": false}}), 0).unwrap();
        let take = json!({"op": "take", "args": {"member": "ann", "amount": 50}});
        assert!(
            c.prepare_service_request("SVC", &take, Some("ann"), 0)
                .unwrap_err()
                .reason
                .contains("may not take")
        );
        let list = json!({"op": "members"});
        assert!(
            c.prepare_service_request("SVC", &list, None, 0)
                .unwrap_err()
                .reason
                .contains("may not list")
        );
        let status = c
            .prepare_service_request("SVC", &json!({"op": "status"}), None, 0)
            .unwrap();
        let Prepared::Value(v) = status else {
            panic!("a value")
        };
        assert_eq!(v["budget"], 1000.0);
        c.update_service(
            "SVC",
            &json!({"perms": {"give": true, "take": true, "read": true}}),
        )
        .unwrap();
        assert_eq!(
            c.prepare_service_request("SVC", &list, None, 0),
            Ok(Prepared::Members)
        );
        assert!(matches!(
            c.prepare_service_request("SVC", &take, Some("ann"), 0),
            Ok(Prepared::Move { .. })
        ));
        c.update_service("SVC", &json!({"expires": 5})).unwrap();
        let body = json!({"type": "service", "club": c.code});
        assert!(
            c.check_service_request("SVC", "ed25519", "KEY", 10, &body, Some(""), 10)
                .unwrap_err()
                .reason
                .contains("expired")
        );
        c.update_service("SVC", &json!({"expires": null})).unwrap();
        assert_eq!(c.services["SVC"].expires, None);
        c.revoke_service("SVC", 20).unwrap();
        assert_eq!(c.services["SVC"].key, None);
        assert!(
            c.check_service_request("SVC", "ed25519", "KEY", 20, &body, None, 20)
                .unwrap_err()
                .reason
                .contains("revoked")
        );
        c.add_service(
            "SVC",
            "ed25519",
            "KEY",
            &json!({"name": "Rewards 2", "budget": 500}),
            30,
        )
        .unwrap();
        assert_eq!(
            c.services["SVC"].budget, 500,
            "added again, it starts fresh"
        );
    }

    #[test]
    fn chip_text_reads_like_the_page() {
        assert_eq!(chip_text(30_000), "300");
        assert_eq!(chip_text(69_950), "699.50");
        assert_eq!(chip_text(123_456_789), "1,234,567.89");
    }

    #[test]
    fn a_saved_club_loads_and_saves_the_same() {
        let saved = json!({
            "v": 1, "code": "C", "name": "N", "hostId": "H", "invite": "I", "createdAt": 5,
            "members": {"a": {"id": "a", "name": "A", "status": "member", "joinedAt": 1, "balance": 0, "seat": null, "request": null, "left": {}}},
            "help": [], "lobby": [],
            "tables": [{"id": "t", "name": "T", "game": "nlhe", "sb": 1, "bb": 2, "minBB": 40, "maxBB": 100, "seats": 9, "turnSecs": 30, "sitOutMins": 10, "stayHands": 0, "status": "open", "createdAt": 0}],
            "sitOutMins": 10, "ratholeHours": 2.5, "waitlist": {},
        });
        let c: Club = serde_json::from_value(saved.clone()).unwrap();
        assert_eq!(serde_json::to_value(&c).unwrap(), saved);
    }
}
