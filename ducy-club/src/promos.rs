//! A club's promotions (ducy-web #457, from its #267): club chips paid for
//! big hands, set up by the club's owner, run after every hand and when a
//! period ends.
//!
//! Two kinds:
//! - `high_hand`: the best hand shown down in each game during a period (an
//!   hour, a day, a week, or a session window) wins its prizes when the
//!   period ends. With rollover, a period played without a qualifying hand
//!   adds its prize to the next.
//! - `bonus`: a fixed prize for each qualifying hand, by tier (quads 10, a
//!   straight flush 25…), each tier capped per period on its own.
//!
//! Any promotion can be on only at set hours on set days.
//!
//! Qualifying hands are shown down, made with two of the player's own cards
//! and three from the board (each board on its own when there are two), and
//! meet the promotion's conditions (players dealt in, pot, tables).
//!
//! Prizes are club chips the club issues; a void takes them back. The caller
//! moves them through the money model: an award these rules mark paid
//! (`status` "paid", no `entry` yet) is the caller's to pay.
//!
//! Kept as ducy-web's JSON (`club.promos`), so a club saved before these
//! rules loads as it is.

use std::collections::{BTreeMap, BTreeSet};

use rand::rngs::StdRng;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::rules::{GAMES, Member, RuleError, chip_text, game, random36, refuse};

/// Hand categories, weakest first.
pub const CATEGORIES: [&str; 10] = [
    "high_card",
    "pair",
    "two_pair",
    "trips",
    "straight",
    "flush",
    "full_house",
    "quads",
    "straight_flush",
    "royal_flush",
];

/// Each category in words.
pub fn category_name(c: &str) -> &'static str {
    match c {
        "high_card" => "High card",
        "pair" => "Pair",
        "two_pair" => "Two pair",
        "trips" => "Three of a kind",
        "straight" => "A straight",
        "flush" => "A flush",
        "full_house" => "A full house",
        "quads" => "Four of a kind",
        "straight_flush" => "A straight flush",
        "royal_flush" => "A royal flush",
        _ => "",
    }
}

const RANKS: &str = "23456789TJQKA";
const RANK_WORDS: [&str; 13] = [
    "twos", "threes", "fours", "fives", "sixes", "sevens", "eights", "nines", "tens", "jacks",
    "queens", "kings", "aces",
];
const RANK_ONE: [&str; 13] = [
    "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "jack", "queen",
    "king", "ace",
];

/// The most promotions running or paused at once.
pub const MAX_PROMOS: usize = 10;
/// The longest promotion name.
pub const MAX_PROMO_NAME: usize = 40;
/// How long after it's paid an award can be voided.
pub const VOID_MS: u64 = 24 * 3_600_000;
const HISTORY_KEEP: usize = 100;
const HANDS_KEEP: usize = 200;
const AWARDS_KEEP: usize = 500;
const BOARD_KEEP: usize = 10;
const SEEN_KEEP: usize = 400;
const MAX_CHIPS: u64 = 1_000_000_000_000;
const H: i64 = 3_600_000;
const D: i64 = 24 * H;

// ---- hands ----

/// A card: its rank (0: deuce … 12: ace) and suit, from "As" or "10h".
fn parse(c: &str) -> Option<(usize, char)> {
    let r = if c.len() == 3 && c.starts_with("10") {
        'T'
    } else {
        c.chars().next()?.to_ascii_uppercase()
    };
    let r = RANKS.find(r)?;
    let s = c.chars().last()?.to_ascii_lowercase();
    Some((r, s))
}

/// A five-card hand's strength.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Score {
    /// Higher is better, comparable across any five cards.
    pub score: u64,
    /// Its category's index in [`CATEGORIES`].
    pub cat: usize,
    /// (count, rank) by count then rank, highest first.
    pub groups: Vec<(usize, usize)>,
    /// A straight's high card (the wheel is five-high).
    pub high: Option<usize>,
}

/// Scores five cards ("As", "Kd", …).
pub fn score5(cards: &[String]) -> Option<Score> {
    let cs: Vec<(usize, char)> = cards.iter().map(|c| parse(c)).collect::<Option<_>>()?;
    let mut counts: BTreeMap<usize, usize> = BTreeMap::new();
    for &(r, _) in &cs {
        *counts.entry(r).or_default() += 1;
    }
    let mut groups: Vec<(usize, usize)> = counts.iter().map(|(&r, &n)| (n, r)).collect();
    groups.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    let flush = cs.iter().all(|c| c.1 == cs[0].1);
    let uniq: Vec<usize> = counts.keys().rev().copied().collect();
    let mut high = None;
    if uniq.len() == 5 {
        if uniq[0] - uniq[4] == 4 {
            high = Some(uniq[0]);
        } else if uniq == [12, 3, 2, 1, 0] {
            high = Some(3);
        }
    }
    let n = |i: usize| groups.get(i).map_or(0, |g| g.0);
    let cat = match () {
        _ if high.is_some() && flush => {
            if high == Some(12) {
                9
            } else {
                8
            }
        }
        _ if n(0) == 4 => 7,
        _ if n(0) == 3 && n(1) == 2 => 6,
        _ if flush => 5,
        _ if high.is_some() => 4,
        _ if n(0) == 3 => 3,
        _ if n(0) == 2 && n(1) == 2 => 2,
        _ if n(0) == 2 => 1,
        _ => 0,
    };
    // A royal flush scores as the ace-high straight flush it is.
    let base = if cat == 9 { 8 } else { cat } as u64;
    let kick: Vec<usize> = match high {
        Some(h) => vec![h],
        None => groups.iter().map(|g| g.1).collect(),
    };
    let mut score = base;
    for i in 0..5 {
        score = score * 13 + kick.get(i).copied().unwrap_or(0) as u64;
    }
    Some(Score {
        score,
        cat,
        groups,
        high,
    })
}

fn combos(arr: &[String], k: usize) -> Vec<Vec<String>> {
    fn go(
        arr: &[String],
        k: usize,
        start: usize,
        pick: &mut Vec<String>,
        out: &mut Vec<Vec<String>>,
    ) {
        if pick.len() == k {
            out.push(pick.clone());
            return;
        }
        for i in start..arr.len() {
            pick.push(arr[i].clone());
            go(arr, k, i + 1, pick, out);
            pick.pop();
        }
    }
    let mut out = vec![];
    go(arr, k, 0, &mut vec![], &mut out);
    out
}

/// The best five cards using exactly two of `hole` and three of `board`,
/// with them; none when the board is short.
pub fn best_with_two(hole: &[String], board: &[String]) -> Option<(Score, Vec<String>)> {
    if hole.len() < 2 || board.len() < 3 {
        return None;
    }
    let mut best: Option<(Score, Vec<String>)> = None;
    for h in combos(hole, 2) {
        for b in combos(board, 3) {
            let five: Vec<String> = h.iter().chain(&b).cloned().collect();
            let s = score5(&five)?;
            if best.as_ref().is_none_or(|(x, _)| s.score > x.score) {
                best = Some((s, five));
            }
        }
    }
    best
}

/// The hand in words: "Quad kings", "A royal flush", "A full house, aces
/// full of tens".
pub fn hand_name(s: &Score) -> String {
    let g = |i: usize| s.groups.get(i).map_or(0, |g| g.1);
    let high = s.high.unwrap_or(0);
    match CATEGORIES[s.cat] {
        "royal_flush" => "A royal flush".into(),
        "straight_flush" => format!("A straight flush, {} high", RANK_ONE[high]),
        "quads" => format!("Quad {}", RANK_WORDS[g(0)]),
        "full_house" => format!(
            "A full house, {} full of {}",
            RANK_WORDS[g(0)],
            RANK_WORDS[g(1)]
        ),
        "flush" => format!("A flush, {} high", RANK_ONE[g(0)]),
        "straight" => format!("A straight, {} high", RANK_ONE[high]),
        "trips" => format!("Three {}", RANK_WORDS[g(0)]),
        "two_pair" => format!("Two pair, {} and {}", RANK_WORDS[g(0)], RANK_WORDS[g(1)]),
        "pair" => format!("A pair of {}", RANK_WORDS[g(0)]),
        _ => {
            let w = RANK_ONE[g(0)];
            format!("{}{} high", w[..1].to_uppercase(), &w[1..])
        }
    }
}

/// A shown-down hand, its best five with two hole cards on one board.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Shown {
    /// The member who held it.
    pub member: String,
    /// The name at the table.
    pub name: String,
    /// Their seat.
    pub seat: u64,
    /// Which board: 1, or 2.
    pub board: u64,
    /// Its score.
    pub score: u64,
    /// Its category's index.
    pub cat: usize,
    /// Its category.
    pub category: String,
    /// Its name.
    pub hand: String,
    /// The five cards.
    pub cards: Vec<String>,
    /// The hole cards.
    pub hole: Vec<String>,
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|c| c.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

fn num(v: Option<&Value>) -> f64 {
    v.and_then(Value::as_f64).unwrap_or(0.0)
}

/// As JavaScript writes a value into a string (`${v}`).
fn js_text(v: Option<&Value>) -> String {
    match v {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 1e21 => format!("{}", f as i64),
            _ => n.to_string(),
        },
        Some(other) => other.to_string(),
    }
}

/// Every shown-down hand in a finished hand's record (ducy-web's hand
/// record), each board on its own. `players`: the table's [{seat,
/// client_id}], to know who held each seat.
pub fn shown_hands(rec: &Value, players: &[Value]) -> Vec<Shown> {
    if !truthy(rec.get("showdown")) {
        return vec![];
    }
    let mut boards = vec![strings(rec.get("board"))];
    let board2 = strings(rec.get("board2"));
    if !board2.is_empty() {
        boards.push(board2);
    }
    let mut out = vec![];
    for s in rec
        .get("seats")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(cards) = s.get("cards").and_then(Value::as_array) else {
            continue;
        };
        let cards: Vec<String> = cards
            .iter()
            .map(|c| c.as_str().unwrap_or("").to_string())
            .collect();
        if !truthy(s.get("inHand"))
            || truthy(s.get("folded"))
            || cards.len() < 2
            || cards.iter().any(|c| parse(c).is_none())
        {
            continue;
        }
        let i = num(s.get("i"));
        let member = players
            .iter()
            .find(|p| num(p.get("seat")) == i)
            .and_then(|p| p.get("client_id"))
            .and_then(Value::as_str)
            .filter(|m| !m.is_empty());
        let Some(member) = member else { continue };
        for (b, board) in boards.iter().enumerate() {
            if let Some((best, five)) = best_with_two(&cards, board) {
                out.push(Shown {
                    member: member.into(),
                    name: s.get("name").and_then(Value::as_str).unwrap_or("").into(),
                    seat: i as u64,
                    board: b as u64 + 1,
                    score: best.score,
                    cat: best.cat,
                    category: CATEGORIES[best.cat].into(),
                    hand: hand_name(&best),
                    cards: five,
                    hole: cards.clone(),
                });
            }
        }
    }
    out
}

// ---- settings ----

/// One bonus tier.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tier {
    /// The least hand it pays for.
    pub category: String,
    /// Club chips.
    pub amount: u64,
    /// Times a period, 0 for no cap; an older promotion's one cap for all
    /// when missing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cap: Option<u64>,
}

/// A promotion.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Promo {
    /// Its id.
    pub id: String,
    /// "high_hand" or "bonus".
    pub kind: String,
    /// Its name.
    pub name: String,
    /// The games it counts in.
    pub games: Vec<String>,
    /// "hour", "day", "week" or "session".
    pub every: String,
    /// The manager's time zone, as minutes to add to UTC.
    pub tz: i64,
    /// A session's days, or set hours' (0: Sunday).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days: Option<Vec<i64>>,
    /// A session's or set hours' start, in minutes of the day.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<i64>,
    /// Their end.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<i64>,
    /// On only at set hours.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hours: Option<bool>,
    /// A high hand's prizes by place.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prizes: Option<Vec<u64>>,
    /// The least hand that qualifies for a high hand.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_category: Option<String>,
    /// "first" (the first to make it wins a tie) or "split".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ties: Option<String>,
    /// A period without a qualifying hand adds its prize to the next.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rollover: Option<bool>,
    /// A bonus's tiers, best first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tiers: Option<Vec<Tier>>,
    /// Bonuses per player a period, 0 for no cap.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cap_per_player: Option<u64>,
    /// An older promotion's one cap for every tier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cap_per_tier: Option<u64>,
    /// Players dealt in, at least.
    pub min_players: u64,
    /// The pot, at least.
    pub min_pot: u64,
    /// Only these tables (none: all).
    pub tables: Vec<String>,
    /// Tournament hands count too.
    pub tourneys: bool,
    /// Chips a period, 0 for no budget.
    pub budget: u64,
    /// When it starts (0: now).
    pub starts_at: u64,
    /// When it ends (0: never).
    pub ends_at: u64,
    /// "running", "paused" or "ended".
    pub status: String,
    /// When it was made.
    pub created_at: u64,
    /// Changes waiting for the next period.
    pub next: Option<Value>,
    /// Anything else kept on it.
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

impl Promo {
    fn games_label(&self) -> String {
        self.games
            .iter()
            .map(|g| game(g).map_or(g.as_str(), |x| x.label))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// A tier's cap per period, 0 for none.
    pub fn tier_cap(&self, t: &Tier) -> u64 {
        t.cap.or(self.cap_per_tier).unwrap_or(0)
    }

    fn tier_for(&self, cat: usize) -> Option<&Tier> {
        self.tiers.as_deref().unwrap_or_default().iter().find(|t| {
            CATEGORIES
                .iter()
                .position(|c| *c == t.category)
                .is_some_and(|i| i <= cat)
        })
    }

    fn days(&self) -> &[i64] {
        self.days.as_deref().unwrap_or_default()
    }
}

fn int(v: Option<&Value>) -> Option<i64> {
    let v = v?;
    v.as_i64().or_else(|| {
        v.as_f64()
            .filter(|f| f.fract() == 0.0 && f.abs() <= 9e15)
            .map(|f| f as i64)
    })
}

/// `v[k]` unless missing or null.
fn given<'a>(v: &'a Value, k: &str) -> Option<&'a Value> {
    v.get(k).filter(|x| !x.is_null())
}

/// A whole number in [lo, hi] at `k`, `d` when missing; `None` when it's
/// something else.
fn whole_or(s: &Value, k: &str, d: i64, lo: i64, hi: i64) -> Option<i64> {
    match given(s, k) {
        None => Some(d),
        v => int(v).filter(|n| (lo..=hi).contains(n)),
    }
}

fn chips_ok(v: &Value) -> Option<u64> {
    int(Some(v))
        .filter(|&n| n >= 0 && n as u64 <= MAX_CHIPS)
        .map(|n| n as u64)
}

/// Checks a promotion's settings; returns them clean, or what's wrong.
pub fn promo_settings(s: &Value) -> Result<Map<String, Value>, RuleError> {
    let kind = match s.get("kind").and_then(Value::as_str) {
        Some("bonus") => "bonus",
        Some("high_hand") => "high_hand",
        _ => return refuse("Pick a kind of promotion."),
    };
    let name: String = match given(s, "name") {
        Some(Value::String(n)) => n.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
    .trim()
    .chars()
    .take(MAX_PROMO_NAME)
    .collect();
    let name = if name.is_empty() {
        if kind == "bonus" {
            "Hand Bonus"
        } else {
            "High Hand"
        }
        .to_string()
    } else {
        name
    };
    let mut games: Vec<String> = vec![];
    for g in strings(s.get("games")) {
        if GAMES.iter().any(|x| x.id == g) && !games.contains(&g) {
            games.push(g);
        }
    }
    if games.is_empty() {
        return refuse("Pick at least one game.");
    }
    let every = match s.get("every").and_then(Value::as_str) {
        Some(e @ ("hour" | "day" | "week" | "session")) => e,
        _ => return refuse("Pick a period."),
    };
    let tz = whole_or(s, "tz", 0, -14 * 60, 14 * 60).unwrap_or(0);
    let mut out = Map::new();
    out.insert("kind".into(), kind.into());
    out.insert("name".into(), name.into());
    out.insert("games".into(), json!(games));
    out.insert("every".into(), every.into());
    out.insert("tz".into(), tz.into());
    let mut days: Vec<i64> = s
        .get("days")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|d| int(Some(d)).filter(|d| (0..=6).contains(d)))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    days.sort();
    let from = int(given(s, "from"));
    let to = int(given(s, "to"));
    if every == "session" {
        if days.is_empty() {
            return refuse("Pick the days the session runs.");
        }
        match (from, to) {
            (Some(f), Some(t)) if (0..=1439).contains(&f) && (1..=1440).contains(&t) && t > f => {}
            _ => return refuse("The session must end after it starts, the same day."),
        }
        out.insert("days".into(), json!(days));
        out.insert("from".into(), json!(from));
        out.insert("to".into(), json!(to));
    } else if s.get("hours") == Some(&Value::Bool(true)) {
        if days.is_empty() {
            return refuse("Pick the days it’s on.");
        }
        match (from, to) {
            (Some(f), Some(t)) if (0..=1439).contains(&f) && (0..=1440).contains(&t) && t != f => {}
            _ => return refuse("Give the hours it’s on."),
        }
        out.insert("hours".into(), true.into());
        out.insert("days".into(), json!(days));
        out.insert("from".into(), json!(from));
        out.insert("to".into(), json!(to));
    }
    if kind == "high_hand" {
        let given_prizes: Vec<&Value> = s
            .get("prizes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|x| !x.is_null() && x.as_str() != Some(""))
            .take(3)
            .collect();
        let prizes: Option<Vec<u64>> = given_prizes.iter().map(|x| chips_ok(x)).collect();
        let Some(prizes) = prizes.filter(|p| !p.is_empty() && p[0] > 0) else {
            return refuse("Give a prize, in chips.");
        };
        out.insert("prizes".into(), json!(prizes));
        let min = s
            .get("minCategory")
            .and_then(Value::as_str)
            .filter(|c| CATEGORIES.contains(c))
            .unwrap_or("full_house");
        out.insert("minCategory".into(), min.into());
        let ties = if s.get("ties").and_then(Value::as_str) == Some("split") {
            "split"
        } else {
            "first"
        };
        out.insert("ties".into(), ties.into());
        out.insert(
            "rollover".into(),
            (s.get("rollover") == Some(&Value::Bool(true))).into(),
        );
    } else {
        let tiers: Vec<&Value> = s
            .get("tiers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|t| {
                t.get("category")
                    .and_then(Value::as_str)
                    .is_some_and(|c| CATEGORIES.contains(&c))
                    && truthy(t.get("amount"))
            })
            .collect();
        if tiers.is_empty() {
            return refuse("Add at least one bonus: a hand and its prize.");
        }
        if !tiers
            .iter()
            .all(|t| chips_ok(&t["amount"]).is_some_and(|a| a > 0))
        {
            return refuse("Each bonus is a number of chips above 0.");
        }
        let cats: BTreeSet<&str> = tiers
            .iter()
            .filter_map(|t| t["category"].as_str())
            .collect();
        if cats.len() != tiers.len() {
            return refuse("Each hand can only have one bonus.");
        }
        let cap = |c: Option<&Value>| match c.filter(|x| !x.is_null()) {
            None => 0,
            v => int(v).filter(|n| (0..=1000).contains(n)).unwrap_or(0),
        };
        let mut list: Vec<Value> = tiers
            .iter()
            .map(|t| {
                let c = given(t, "cap").or_else(|| given(s, "capPerTier"));
                json!({"category": t["category"], "amount": chips_ok(&t["amount"]), "cap": cap(c)})
            })
            .collect();
        let rank = |t: &Value| {
            CATEGORIES
                .iter()
                .position(|c| Some(*c) == t["category"].as_str())
                .unwrap_or(0)
        };
        list.sort_by_key(|t| std::cmp::Reverse(rank(t)));
        out.insert("tiers".into(), list.into());
        out.insert(
            "capPerPlayer".into(),
            whole_or(s, "capPerPlayer", 0, 0, 1000).unwrap_or(0).into(),
        );
    }
    out.insert(
        "minPlayers".into(),
        whole_or(s, "minPlayers", 4, 2, 9).unwrap_or(4).into(),
    );
    out.insert(
        "minPot".into(),
        match given(s, "minPot") {
            None => 0,
            Some(v) => chips_ok(v).unwrap_or(0),
        }
        .into(),
    );
    let tables: Vec<String> = s
        .get("tables")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|t| js_text(Some(t)))
        .take(50)
        .collect();
    out.insert("tables".into(), json!(tables));
    out.insert(
        "tourneys".into(),
        (s.get("tourneys") == Some(&Value::Bool(true))).into(),
    );
    out.insert(
        "budget".into(),
        match given(s, "budget") {
            None => 0,
            Some(v) => chips_ok(v).unwrap_or(0),
        }
        .into(),
    );
    let starts = whole_or(s, "startsAt", 0, 0, 100_000_000_000_000).unwrap_or(0);
    let ends = whole_or(s, "endsAt", 0, 0, 100_000_000_000_000).unwrap_or(0);
    if ends != 0 && ends <= starts {
        return refuse("It must end after it starts.");
    }
    out.insert("startsAt".into(), starts.into());
    out.insert("endsAt".into(), ends.into());
    Ok(out)
}

// ---- state ----

/// An entry on a high hand's leaderboard.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BoardEntry {
    /// The member.
    pub member: String,
    /// Their name.
    pub name: String,
    /// The hand's score.
    pub score: u64,
    /// Its category.
    pub category: String,
    /// Its name.
    pub hand: String,
    /// Its five cards.
    pub cards: Vec<String>,
    /// The hand record's id.
    pub hand_id: Option<String>,
    /// The table.
    pub table: Option<String>,
    /// When.
    pub at: u64,
    /// Anything else.
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

/// A prize, held or paid or void.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Award {
    /// Its id.
    pub id: String,
    /// The promotion's id.
    pub promo: String,
    /// The promotion's name.
    pub promo_name: String,
    /// "bonus" or "high_hand".
    pub kind: String,
    /// The period's key.
    pub period: String,
    /// The member.
    pub member: String,
    /// Their name.
    pub name: String,
    /// Club chips.
    pub amount: u64,
    /// The hand record's id.
    pub hand: Option<String>,
    /// The table.
    pub table: Option<String>,
    /// The game.
    pub game: String,
    /// The hand's category.
    pub category: String,
    /// The hand's name.
    pub hand_name: String,
    /// The hand's five cards.
    pub cards: Vec<String>,
    /// A high hand's place (0 for a bonus).
    pub place: u64,
    /// When it was made.
    pub at: u64,
    /// "held", "paid" or "void".
    pub status: String,
    /// The ledger entry that paid it, once the caller has.
    pub entry: Option<u64>,
    /// Anything else (paidAt, voidEntry, voidWhy, voidedAt…).
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

/// A finished period of a high hand.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct History {
    /// The promotion's id.
    pub promo: String,
    /// Its name.
    pub promo_name: String,
    /// The period's key.
    pub period: String,
    /// When it started.
    pub start: i64,
    /// When it ended.
    pub end: i64,
    /// The game.
    pub game: String,
    /// The awards it made.
    pub winners: Vec<String>,
    /// Its leaderboard.
    pub top: Vec<BoardEntry>,
    /// What rolled over, when no one made it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rolled: Option<u64>,
}

/// A club's promotions (`club.promos`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PromoState {
    /// The promotions, newest last.
    pub list: Vec<Promo>,
    /// Running periods' leaderboards, by `promo|period|game`.
    pub boards: BTreeMap<String, Vec<BoardEntry>>,
    /// Bonus counts by `promo|period|tier:…` and `promo|period|p:…`.
    pub counts: BTreeMap<String, u64>,
    /// Paid per `promo|period`.
    pub paid: BTreeMap<String, u64>,
    /// Every prize, held or paid.
    pub awards: Vec<Award>,
    /// Periods played, for a high hand that rolls over.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub played: BTreeMap<String, bool>,
    /// Prizes rolled over, by `promo|game`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub carry: BTreeMap<String, u64>,
    /// Finished periods.
    pub history: Vec<History>,
    /// Recent qualifying hands.
    pub hands: Vec<Value>,
    /// Hands counted already.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub seen: Vec<String>,
}

/// A period: its key (its start, as text) and bounds, in milliseconds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Period {
    /// Its key.
    #[serde(skip_serializing)]
    pub key: String,
    /// Its start.
    pub start: i64,
    /// Its end.
    pub end: i64,
}

/// The period `promo` is in at `t`, or none when it isn't running then (a
/// session's off hours, set hours' off hours, before it starts or after it
/// ends).
pub fn period_of(promo: &Promo, t: i64) -> Option<Period> {
    if promo.starts_at != 0 && t < promo.starts_at as i64 {
        return None;
    }
    if promo.ends_at != 0 && t >= promo.ends_at as i64 {
        return None;
    }
    let off = promo.tz * 60_000;
    let local = t + off;
    let day = local.div_euclid(D);
    let day_start = day * D;
    let (mut start, mut end);
    match promo.every.as_str() {
        "hour" => {
            start = local.div_euclid(H) * H;
            end = start + H;
        }
        "day" => {
            start = day_start;
            end = start + D;
        }
        "week" => {
            // Weeks start on Monday (1970-01-01 was a Thursday).
            let dow = (day + 3).rem_euclid(7);
            start = day_start - dow * D;
            end = start + 7 * D;
        }
        _ => {
            let weekday = (day + 4).rem_euclid(7);
            let mins = (local - day_start) as f64 / 60_000.0;
            let (from, to) = (promo.from.unwrap_or(0), promo.to.unwrap_or(0));
            if !promo.days().contains(&weekday) || mins < from as f64 || mins >= to as f64 {
                return None;
            }
            start = day_start + from * 60_000;
            end = day_start + to * 60_000;
        }
    }
    if promo.hours == Some(true) && !on_hours(promo, local) {
        return None;
    }
    start -= off;
    end -= off;
    if promo.ends_at != 0 {
        end = end.min(promo.ends_at as i64);
    }
    Some(Period {
        key: start.to_string(),
        start,
        end,
    })
}

/// Whether local time `local` is within a promotion's set hours.
fn on_hours(promo: &Promo, local: i64) -> bool {
    let day = local.div_euclid(D);
    let mins = (local - day * D) as f64 / 60_000.0;
    let weekday = |d: i64| (d + 4).rem_euclid(7);
    let (from, to) = (promo.from.unwrap_or(0) as f64, promo.to.unwrap_or(0) as f64);
    let days = promo.days();
    if to > from {
        return days.contains(&weekday(day)) && mins >= from && mins < to;
    }
    (days.contains(&weekday(day)) && mins >= from)
        || (days.contains(&weekday(day - 1)) && mins < to)
}

/// The period starting at `start` for `promo`.
fn period_bounds(promo: &Promo, start: i64) -> (i64, i64) {
    let len = match promo.every.as_str() {
        "hour" => H,
        "day" => D,
        "week" => 7 * D,
        _ => (promo.to.unwrap_or(0) - promo.from.unwrap_or(0)) * 60_000,
    };
    let end = start + len;
    (
        start,
        if promo.ends_at != 0 {
            end.min(promo.ends_at as i64)
        } else {
            end
        },
    )
}

fn key_start(k: &str) -> i64 {
    k.split('|')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// What a finished hand did ([`Promos::process_hand`]).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct HandOutcome {
    /// Bonuses paid (the caller pays them).
    pub paid: Vec<Award>,
    /// New leaders: {promo, game, hand}.
    pub leads: Vec<Value>,
    /// Promotions whose budget stopped a payout.
    pub budget: Vec<String>,
}

/// A voided award.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Voided {
    /// The award, now void.
    pub award: Award,
    /// The next best hand's award, held for the manager, for a high hand.
    pub offered: Option<Award>,
    /// Chips to take back from the member (it had been paid).
    pub take: u64,
}

/// A club's promotions with its members (read, never changed here).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Promos {
    /// The club's code.
    #[serde(skip_serializing)]
    pub code: String,
    /// The members.
    #[serde(skip_serializing)]
    pub members: BTreeMap<String, Member>,
    /// The promotions' state.
    pub promos: PromoState,
}

/// When an award waits for the manager instead of being paid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hold {
    /// Never: over the budget, there's no award.
    Never,
    /// Over the period's budget.
    OverBudget,
    /// Always (the next best hand, offered).
    Always,
}

/// What to pay or hold, for [`Promos::award`].
struct Prize<'a> {
    member: &'a str,
    amount: u64,
    hand_id: Option<String>,
    table: Option<String>,
    category: String,
    hand_name: String,
    cards: Vec<String>,
    kind: &'a str,
    place: u64,
    game: &'a str,
}

impl Promos {
    /// A promotion by id.
    pub fn promo(&self, id: &str) -> Option<&Promo> {
        self.promos.list.iter().find(|p| p.id == id)
    }

    fn name_of(&self, member: &str, or: &str) -> String {
        self.members
            .get(member)
            .map_or(or.to_string(), |m| m.name.clone())
    }

    /// A new promotion from its settings, or why they won't do.
    pub fn new_promo(&mut self, s: &Value, now: u64, rng: &mut StdRng) -> Result<Promo, RuleError> {
        if self
            .promos
            .list
            .iter()
            .filter(|p| p.status != "ended")
            .count()
            >= MAX_PROMOS
        {
            return refuse(format!("A club can run {MAX_PROMOS} promotions at once."));
        }
        let mut v = Value::Object(promo_settings(s)?);
        v["id"] = random36(rng, 6).into();
        v["status"] = "running".into();
        v["createdAt"] = now.into();
        v["next"] = Value::Null;
        let promo: Promo = serde_json::from_value(v).map_err(|e| RuleError(e.to_string()))?;
        self.promos.list.push(promo.clone());
        Ok(promo)
    }

    /// Changes take effect from the next period: a running period keeps
    /// its rules.
    pub fn edit_promo(&mut self, id: &str, s: &Value, now: u64) -> Result<Promo, RuleError> {
        let Some(promo) = self.promo(id).filter(|p| p.status != "ended").cloned() else {
            return refuse("No such promotion.");
        };
        let mut s = s.clone();
        if !s.is_object() {
            s = json!({});
        }
        s["kind"] = promo.kind.clone().into();
        let clean = promo_settings(&s)?;
        let started =
            period_of(&promo, now as i64).is_some_and(|cur| self.has_activity(id, &cur.key));
        let i = self
            .promos
            .list
            .iter()
            .position(|p| p.id == id)
            .expect("found above");
        if started {
            self.promos.list[i].next = Some(Value::Object(clean));
        } else {
            let p = assign(&self.promos.list[i], &clean)?;
            self.promos.list[i] = p;
            self.promos.list[i].next = None;
        }
        Ok(self.promos.list[i].clone())
    }

    fn has_activity(&self, id: &str, period: &str) -> bool {
        let prefix = format!("{id}|{period}|");
        self.promos
            .boards
            .keys()
            .chain(self.promos.counts.keys())
            .any(|k| k.starts_with(&prefix))
    }

    /// Runs, pauses or ends a promotion.
    pub fn set_status(&mut self, id: &str, status: &str) -> Result<Promo, RuleError> {
        let Some(p) = self.promos.list.iter_mut().find(|p| p.id == id) else {
            return refuse("No such promotion.");
        };
        if !["running", "paused", "ended"].contains(&status) {
            return refuse("Not a promotion state.");
        }
        if p.status == "ended" {
            return refuse("That promotion has ended.");
        }
        p.status = status.into();
        Ok(p.clone())
    }

    /// Makes an award: paid (the caller moves the chips), or held for the
    /// manager as `hold` says; none when over the budget and not held.
    fn award(
        &mut self,
        promo: &Promo,
        period: &str,
        x: Prize,
        now: u64,
        hold: Hold,
        rng: &mut StdRng,
    ) -> Option<Award> {
        let pk = format!("{}|{period}", promo.id);
        let paid_so_far = self.promos.paid.get(&pk).copied().unwrap_or(0);
        let over_budget = promo.budget != 0 && paid_so_far + x.amount > promo.budget;
        let over = hold == Hold::Always || over_budget;
        if over && hold == Hold::Never {
            return None;
        }
        let mut a = Award {
            id: random36(rng, 8),
            promo: promo.id.clone(),
            promo_name: promo.name.clone(),
            kind: x.kind.into(),
            period: period.into(),
            member: x.member.into(),
            name: self.name_of(x.member, x.member),
            amount: x.amount,
            hand: x.hand_id,
            table: x.table,
            game: x.game.into(),
            category: x.category,
            hand_name: x.hand_name,
            cards: x.cards,
            place: x.place,
            at: now,
            status: if over { "held" } else { "paid" }.into(),
            entry: None,
            other: Map::new(),
        };
        if !over {
            self.pay(&mut a, now);
        }
        self.promos.awards.push(a.clone());
        let n = self.promos.awards.len();
        if n > AWARDS_KEEP {
            self.promos.awards.drain(..n - AWARDS_KEEP);
        }
        Some(a)
    }

    fn pay(&mut self, a: &mut Award, now: u64) {
        a.status = "paid".into();
        a.other.insert("paidAt".into(), now.into());
        *self
            .promos
            .paid
            .entry(format!("{}|{}", a.promo, a.period))
            .or_default() += a.amount;
    }

    /// A finished hand (`rec`, ducy-web's hand record): records its
    /// qualifying hands, pays bonuses and updates the high-hand
    /// leaderboards. `players`: the table's [{seat, client_id}].
    pub fn process_hand(
        &mut self,
        rec: &Value,
        players: &[Value],
        now: u64,
        rng: &mut StdRng,
    ) -> HandOutcome {
        let mut out = HandOutcome::default();
        if !rec.is_object() {
            return out;
        }
        // A hand counts once, even if its table reports it again. `sub`: a
        // tournament's own table, whose hand numbers are its own.
        let sub = match rec.get("sub") {
            None | Some(Value::Null) => String::new(),
            v => js_text(v),
        };
        let key = format!(
            "{}|{sub}|{}",
            js_text(rec.get("table")),
            js_text(rec.get("hand"))
        );
        if truthy(rec.get("hand")) && self.promos.seen.contains(&key) {
            return out;
        }
        let game = rec
            .get("game")
            .and_then(Value::as_str)
            .filter(|g| !g.is_empty())
            .unwrap_or("nlhe")
            .to_string();
        let dealt = rec
            .get("seats")
            .and_then(Value::as_array)
            .map_or(0, |s| s.iter().filter(|x| truthy(x.get("inHand"))).count())
            as u64;
        let at = num(rec.get("at")) as i64;
        // A high hand that rolls over: this period was played, shown down or not.
        let list = self.promos.list.clone();
        for promo in &list {
            if promo.kind != "high_hand"
                || promo.rollover != Some(true)
                || promo.status != "running"
            {
                continue;
            }
            if let Some(period) = period_of(promo, at)
                && ineligible(promo, rec, &game, dealt).is_empty()
            {
                self.promos
                    .played
                    .insert(format!("{}|{}|{game}", promo.id, period.key), true);
            }
        }
        let shown = shown_hands(rec, players);
        if shown.is_empty() {
            return out;
        }
        self.promos.seen.push(key);
        let n = self.promos.seen.len();
        if n > SEEN_KEEP {
            self.promos.seen.drain(..n - SEEN_KEEP);
        }
        let rec_id = rec.get("id").and_then(Value::as_str).map(str::to_string);
        let rec_table = rec.get("table").and_then(Value::as_str).map(str::to_string);
        for promo in &list {
            if promo.status != "running" {
                continue;
            }
            let Some(period) = period_of(promo, at) else {
                continue;
            };
            let why = ineligible(promo, rec, &game, dealt);
            if promo.kind == "bonus" {
                // Each member's best across the boards, paid once at its highest tier.
                let mut best: Vec<&Shown> = vec![];
                for h in &shown {
                    match best.iter().position(|b| b.member == h.member) {
                        Some(i) if h.score > best[i].score => best[i] = h,
                        Some(_) => {}
                        None => best.push(h),
                    }
                }
                for h in best {
                    let Some(tier) = promo.tier_for(h.cat).cloned() else {
                        continue;
                    };
                    self.record(rec, h, &game, promo, &why);
                    if !why.is_empty() {
                        continue;
                    }
                    let tk = format!("{}|{}|tier:{}", promo.id, period.key, tier.category);
                    let mk = format!("{}|{}|p:{}", promo.id, period.key, h.member);
                    let cap = promo.tier_cap(&tier);
                    if cap != 0 && self.promos.counts.get(&tk).copied().unwrap_or(0) >= cap {
                        continue;
                    }
                    let per_player = promo.cap_per_player.unwrap_or(0);
                    if per_player != 0
                        && self.promos.counts.get(&mk).copied().unwrap_or(0) >= per_player
                    {
                        continue;
                    }
                    let prize = Prize {
                        member: &h.member,
                        amount: tier.amount,
                        hand_id: rec_id.clone(),
                        table: rec_table.clone(),
                        category: h.category.clone(),
                        hand_name: h.hand.clone(),
                        cards: h.cards.clone(),
                        kind: "bonus",
                        place: 0,
                        game: &game,
                    };
                    let Some(a) = self.award(promo, &period.key, prize, now, Hold::Never, rng)
                    else {
                        out.budget.push(promo.name.clone());
                        continue;
                    };
                    *self.promos.counts.entry(tk).or_default() += 1;
                    *self.promos.counts.entry(mk).or_default() += 1;
                    out.paid.push(a);
                }
            } else {
                let min = CATEGORIES
                    .iter()
                    .position(|c| Some(*c) == promo.min_category.as_deref())
                    .unwrap_or(0);
                for h in &shown {
                    if h.cat < min {
                        continue;
                    }
                    self.record(rec, h, &game, promo, &why);
                    if !why.is_empty() {
                        continue;
                    }
                    let key = format!("{}|{}|{game}", promo.id, period.key);
                    let name = self.name_of(&h.member, &h.name);
                    let board = self.promos.boards.entry(key).or_default();
                    let leader = board.first().map(|b| b.score);
                    board.push(BoardEntry {
                        member: h.member.clone(),
                        name,
                        score: h.score,
                        category: h.category.clone(),
                        hand: h.hand.clone(),
                        cards: h.cards.clone(),
                        hand_id: rec_id.clone(),
                        table: rec_table.clone(),
                        at: at as u64,
                        other: Map::new(),
                    });
                    board.sort_by(|a, b| b.score.cmp(&a.score).then(a.at.cmp(&b.at)));
                    board.truncate(BOARD_KEEP);
                    let top = &board[0];
                    if top.hand_id == rec_id
                        && top.member == h.member
                        && leader.is_none_or(|l| l < h.score)
                    {
                        out.leads
                            .push(json!({"promo": promo.name, "game": game, "hand": top}));
                    }
                }
            }
        }
        out
    }

    fn record(&mut self, rec: &Value, h: &Shown, game: &str, promo: &Promo, why: &str) {
        let players = rec
            .get("seats")
            .and_then(Value::as_array)
            .map_or(0, |s| s.iter().filter(|x| truthy(x.get("inHand"))).count());
        self.promos.hands.push(json!({
            "hand": rec.get("id"), "table": rec.get("table"), "game": game, "member": h.member, "name": h.name,
            "category": h.category, "handName": h.hand, "cards": h.cards, "board": h.board, "at": rec.get("at"),
            "players": players, "pot": rec.get("pot"), "promo": promo.id, "ineligible": why,
        }));
        let n = self.promos.hands.len();
        if n > HANDS_KEEP {
            self.promos.hands.drain(..n - HANDS_KEEP);
        }
    }

    /// Pays the high hands of every period that has ended by `now`, starts
    /// the promotions' waiting changes, and drops counters of past periods.
    /// The awards paid or held.
    pub fn settle_periods(&mut self, now: u64, rng: &mut StdRng) -> Vec<Award> {
        let now_i = now as i64;
        let mut out = vec![];
        let mut keys: Vec<String> = self.promos.boards.keys().cloned().collect();
        for k in self.promos.played.keys() {
            if !keys.contains(k) {
                keys.push(k.clone());
            }
        }
        keys.sort_by_key(|k| key_start(k));
        for key in keys {
            let mut parts = key.splitn(3, '|');
            let (id, start, game) = (
                parts.next().unwrap_or("").to_string(),
                parts.next().unwrap_or("").to_string(),
                parts.next().unwrap_or("").to_string(),
            );
            let promo = self.promo(&id).cloned();
            let bounds = promo
                .as_ref()
                .map(|p| period_bounds(p, start.parse().unwrap_or(0)));
            if let Some((_, end)) = bounds
                && end > now_i
            {
                continue;
            }
            let board = self.promos.boards.remove(&key).unwrap_or_default();
            self.promos.played.remove(&key);
            let (Some(promo), Some((pstart, pend))) = (promo, bounds) else {
                continue;
            };
            let ck = format!("{id}|{game}");
            if board.is_empty() {
                // No one made it: the prize rolls over to the next period.
                if promo.rollover == Some(true) {
                    let first = promo
                        .prizes
                        .as_deref()
                        .and_then(<[u64]>::first)
                        .copied()
                        .unwrap_or(0);
                    let c = self.promos.carry.entry(ck).or_default();
                    *c += first;
                    let rolled = *c;
                    self.promos.history.push(History {
                        promo: id.clone(),
                        promo_name: promo.name.clone(),
                        period: start.clone(),
                        start: pstart,
                        end: pend,
                        game: game.clone(),
                        winners: vec![],
                        top: vec![],
                        rolled: Some(rolled),
                    });
                }
                continue;
            }
            // A prize that rolled over goes with first place.
            let carry = self.promos.carry.remove(&ck).unwrap_or(0);
            let winners = place_winners(&promo, &board, carry);
            let mut paid = vec![];
            for (w, amount, place) in winners {
                let prize = Prize {
                    member: &w.member,
                    amount,
                    hand_id: w.hand_id.clone(),
                    table: w.table.clone(),
                    category: w.category.clone(),
                    hand_name: w.hand.clone(),
                    cards: w.cards.clone(),
                    kind: "high_hand",
                    place,
                    game: &game,
                };
                if let Some(a) = self.award(&promo, &start, prize, now, Hold::OverBudget, rng) {
                    paid.push(a);
                }
            }
            self.promos.history.push(History {
                promo: id.clone(),
                promo_name: promo.name.clone(),
                period: start.clone(),
                start: pstart,
                end: pend,
                game: game.clone(),
                winners: paid.iter().map(|a| a.id.clone()).collect(),
                top: board,
                rolled: None,
            });
            out.extend(paid);
        }
        let n = self.promos.history.len();
        if n > HISTORY_KEEP {
            self.promos.history.drain(..n - HISTORY_KEEP);
        }
        for i in 0..self.promos.list.len() {
            let promo = self.promos.list[i].clone();
            let cur = period_of(&promo, now_i);
            if let Some(next) = promo.next.clone().and_then(|n| n.as_object().cloned())
                && !cur
                    .as_ref()
                    .is_some_and(|c| self.has_activity(&promo.id, &c.key))
                && let Ok(mut p) = assign(&promo, &next)
            {
                p.next = None;
                self.promos.list[i] = p;
            }
            let promo = &self.promos.list[i];
            let prefix = format!("{}|", promo.id);
            if promo.status != "ended"
                && promo.ends_at != 0
                && now >= promo.ends_at
                && !self.promos.boards.keys().any(|k| k.starts_with(&prefix))
            {
                self.promos.list[i].status = "ended".into();
            }
        }
        // Counters and budgets of periods that are over.
        let list = self.promos.list.clone();
        let over = |k: &str| {
            let id = k.split('|').next().unwrap_or("");
            list.iter()
                .find(|p| p.id == id)
                .is_none_or(|p| period_bounds(p, key_start(k)).1 <= now_i)
        };
        self.promos.counts.retain(|k, _| !over(k));
        self.promos.paid.retain(|k, _| !over(k));
        out
    }

    /// The soonest time a period with something to settle ends.
    pub fn next_settle(&self) -> Option<i64> {
        self.promos
            .boards
            .keys()
            .chain(self.promos.played.keys())
            .filter_map(|k| {
                let id = k.split('|').next()?;
                let promo = self.promo(id)?;
                Some(period_bounds(promo, key_start(k)).1)
            })
            .min()
    }

    /// Approves an award held for being over the budget: it's to be paid now.
    pub fn approve_award(&mut self, id: &str, now: u64) -> Result<Award, RuleError> {
        let Some(i) = self
            .promos
            .awards
            .iter()
            .position(|a| a.id == id && a.status == "held")
        else {
            return refuse("No such award waiting.");
        };
        let a = &self.promos.awards[i];
        if !self
            .members
            .get(&a.member)
            .is_some_and(|m| m.status == crate::rules::Status::Member)
        {
            return refuse(format!("{} isn’t a member any more.", a.name));
        }
        let mut a = a.clone();
        self.pay(&mut a, now);
        self.promos.awards[i] = a.clone();
        Ok(a)
    }

    /// Voids an award (paid within [`VOID_MS`], or held): chips to take
    /// back. For a high hand, the next best hand of that period is offered
    /// instead, held for the manager to approve.
    pub fn void_award(
        &mut self,
        id: &str,
        now: u64,
        why: &str,
        rng: &mut StdRng,
    ) -> Result<Voided, RuleError> {
        let Some(i) = self
            .promos
            .awards
            .iter()
            .position(|a| a.id == id && a.status != "void")
        else {
            return refuse("No such award.");
        };
        let mut a = self.promos.awards[i].clone();
        let paid_at = a
            .other
            .get("paidAt")
            .and_then(Value::as_u64)
            .unwrap_or(a.at);
        if a.status == "paid" && now.saturating_sub(paid_at) > VOID_MS {
            return refuse("Awards can only be voided within 24 hours.");
        }
        let mut take = 0;
        if a.status == "paid" {
            let balance = self
                .members
                .get(&a.member)
                .and_then(|m| m.other.get("balance"))
                .and_then(Value::as_f64);
            if !balance.is_some_and(|b| b >= a.amount as f64) {
                let has = chip_text(balance.unwrap_or(0.0).max(0.0) as u64);
                return refuse(format!(
                    "{} has only {has} chips in the club now; take what’s left from their balance first, or leave the award.",
                    a.name
                ));
            }
            take = a.amount;
            if !why.is_empty() {
                a.other.insert(
                    "voidWhy".into(),
                    why.chars().take(60).collect::<String>().into(),
                );
            }
            let pk = format!("{}|{}", a.promo, a.period);
            if let Some(p) = self.promos.paid.get_mut(&pk) {
                *p = p.saturating_sub(a.amount);
            }
        }
        a.status = "void".into();
        a.other.insert("voidedAt".into(), now.into());
        self.promos.awards[i] = a.clone();
        let mut offered = None;
        if a.kind == "high_hand" {
            let h = self.promos.history.iter().position(|x| {
                x.promo == a.promo && x.period == a.period && x.winners.contains(&a.id)
            });
            let taken: BTreeSet<String> = self
                .promos
                .awards
                .iter()
                .filter(|x| x.promo == a.promo && x.period == a.period && x.status != "void")
                .map(|x| format!("{}{}", x.hand.as_deref().unwrap_or("null"), x.member))
                .collect();
            let promo = self.promo(&a.promo).cloned();
            if let (Some(h), Some(promo)) = (h, promo) {
                let hist = self.promos.history[h].clone();
                let next = hist.top.iter().find(|t| {
                    t.member != a.member
                        && !taken.contains(&format!(
                            "{}{}",
                            t.hand_id.as_deref().unwrap_or("null"),
                            t.member
                        ))
                });
                if let Some(next) = next {
                    let prize = Prize {
                        member: &next.member,
                        amount: a.amount,
                        hand_id: next.hand_id.clone(),
                        table: next.table.clone(),
                        category: next.category.clone(),
                        hand_name: next.hand.clone(),
                        cards: next.cards.clone(),
                        kind: "high_hand",
                        place: a.place,
                        game: &hist.game,
                    };
                    // Offered, not paid: the manager approves it.
                    offered = self.award(&promo, &a.period, prize, now, Hold::Always, rng);
                    if let Some(o) = &offered {
                        self.promos.history[h].winners.push(o.id.clone());
                    }
                }
            }
        }
        Ok(Voided {
            award: a,
            offered,
            take,
        })
    }

    /// Everything members are told: running promotions with their
    /// leaderboards and caps used, and recent awards.
    pub fn body(&self, now: u64) -> Value {
        let p = &self.promos;
        let promos: Vec<Value> = p
            .list
            .iter()
            .filter(|x| x.status != "ended")
            .map(|promo| {
                let period = period_of(promo, now as i64);
                let mut v = json!({
                    "id": promo.id, "name": promo.name, "kind": promo.kind, "status": promo.status, "rules": promo_rules(promo),
                    "games": promo.games, "period": period, "minPlayers": promo.min_players, "tables": promo.tables, "tourneys": promo.tourneys,
                });
                if promo.kind == "high_hand" {
                    let leaders: Map<String, Value> = promo
                        .games
                        .iter()
                        .map(|g| {
                            let list: Vec<Value> = period
                                .as_ref()
                                .and_then(|pd| p.boards.get(&format!("{}|{}|{g}", promo.id, pd.key)))
                                .map(|b| {
                                    b.iter()
                                        .take(3)
                                        .map(|b| json!({"name": b.name, "hand": b.hand, "cards": b.cards, "at": b.at}))
                                        .collect()
                                })
                                .unwrap_or_default();
                            (g.clone(), Value::Array(list))
                        })
                        .collect();
                    v["leaders"] = Value::Object(leaders);
                    v["prize"] = self.prizes_now(promo);
                } else {
                    v["caps"] = self.caps(promo, period.as_ref());
                }
                v
            })
            .collect();
        let recent: Vec<Value> = p
            .awards
            .iter()
            .filter(|a| a.status == "paid")
            .rev()
            .take(10)
            .map(|a| {
                json!({"id": a.id, "promo": a.promo_name, "name": a.name, "hand": a.hand_name, "cards": a.cards, "amount": a.amount,
                    "at": a.other.get("paidAt").cloned().unwrap_or(a.at.into()), "member": a.member})
            })
            .collect();
        json!({"type": "promos", "club": self.code, "promos": promos, "recent": recent})
    }

    fn prizes_now(&self, promo: &Promo) -> Value {
        let first = promo
            .prizes
            .as_deref()
            .and_then(<[u64]>::first)
            .copied()
            .unwrap_or(0);
        Value::Object(
            promo
                .games
                .iter()
                .map(|g| {
                    let carry = self
                        .promos
                        .carry
                        .get(&format!("{}|{g}", promo.id))
                        .copied()
                        .unwrap_or(0);
                    (g.clone(), (first + carry).into())
                })
                .collect(),
        )
    }

    fn caps(&self, promo: &Promo, period: Option<&Period>) -> Value {
        let tiers = promo.tiers.clone().unwrap_or_default();
        Value::Array(
            tiers
                .iter()
                .map(|t| {
                    let used = period.map_or(0, |pd| {
                        self.promos
                            .counts
                            .get(&format!("{}|{}|tier:{}", promo.id, pd.key, t.category))
                            .copied()
                            .unwrap_or(0)
                    });
                    json!({"category": t.category, "amount": t.amount, "used": used, "cap": promo.tier_cap(t)})
                })
                .collect(),
        )
    }

    /// The manager's view: everything, with awards and budgets.
    pub fn admin(&self, now: u64) -> Value {
        let p = &self.promos;
        let list: Vec<Value> = p
            .list
            .iter()
            .map(|promo| {
                let period = period_of(promo, now as i64);
                let mut v = serde_json::to_value(promo).unwrap_or_default();
                v["rules"] = promo_rules(promo).into();
                v["period"] = period
                    .as_ref()
                    .map_or(Value::Null, |pd| json!({"key": pd.key, "start": pd.start, "end": pd.end}));
                v["paid"] = period
                    .as_ref()
                    .map_or(0, |pd| p.paid.get(&format!("{}|{}", promo.id, pd.key)).copied().unwrap_or(0))
                    .into();
                v["leaders"] = match (&period, promo.kind.as_str()) {
                    (Some(pd), "high_hand") => Value::Object(
                        promo
                            .games
                            .iter()
                            .map(|g| {
                                let top: Vec<Value> = p
                                    .boards
                                    .get(&format!("{}|{}|{g}", promo.id, pd.key))
                                    .map(|b| b.iter().take(3).filter_map(|x| serde_json::to_value(x).ok()).collect())
                                    .unwrap_or_default();
                                (g.clone(), Value::Array(top))
                            })
                            .collect(),
                    ),
                    _ => Value::Null,
                };
                v["prize"] = if promo.kind == "high_hand" { self.prizes_now(promo) } else { Value::Null };
                v["caps"] = match (&period, promo.kind.as_str()) {
                    (Some(pd), "bonus") => Value::Array(
                        promo
                            .tiers
                            .clone()
                            .unwrap_or_default()
                            .iter()
                            .map(|t| {
                                let used = p
                                    .counts
                                    .get(&format!("{}|{}|tier:{}", promo.id, pd.key, t.category))
                                    .copied()
                                    .unwrap_or(0);
                                json!({"category": t.category, "amount": t.amount, "cap": promo.tier_cap(t), "used": used})
                            })
                            .collect(),
                    ),
                    _ => Value::Null,
                };
                v
            })
            .collect();
        let awards: Vec<Value> = p
            .awards
            .iter()
            .rev()
            .take(50)
            .filter_map(|a| serde_json::to_value(a).ok())
            .collect();
        let hands: Vec<Value> = p.hands.iter().rev().take(30).cloned().collect();
        json!({"list": list, "awards": awards, "hands": hands})
    }
}

/// `promo` with each of `changes`' fields set (as `Object.assign`).
fn assign(promo: &Promo, changes: &Map<String, Value>) -> Result<Promo, RuleError> {
    let mut v = serde_json::to_value(promo).map_err(|e| RuleError(e.to_string()))?;
    if let Some(o) = v.as_object_mut() {
        for (k, x) in changes {
            o.insert(k.clone(), x.clone());
        }
    }
    serde_json::from_value(v).map_err(|e| RuleError(e.to_string()))
}

/// Why a hand can't count for `promo` ("" when it can): its conditions.
fn ineligible(promo: &Promo, rec: &Value, game: &str, players: u64) -> String {
    if truthy(rec.get("tourney")) && !promo.tourneys {
        return "tournament hand".into();
    }
    if !promo.games.iter().any(|g| g == game) {
        return "game".into();
    }
    if !promo.tables.is_empty()
        && !promo
            .tables
            .iter()
            .any(|t| Some(t.as_str()) == rec.get("table").and_then(Value::as_str))
    {
        return "table".into();
    }
    if players < promo.min_players {
        return format!("fewer than {} players", promo.min_players);
    }
    if num(rec.get("pot")) < promo.min_pot as f64 {
        return "pot too small".into();
    }
    String::new()
}

/// The prizes by place, ties split equally or to the first to make it;
/// `carry`, rolled over from earlier periods, adds to first place's.
fn place_winners<'a>(
    promo: &Promo,
    board: &'a [BoardEntry],
    carry: u64,
) -> Vec<(&'a BoardEntry, u64, u64)> {
    let mut out = vec![];
    let prizes: Vec<u64> = promo
        .prizes
        .clone()
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(i, &x)| if i == 0 { x + carry } else { x })
        .collect();
    let mut place = 0usize;
    let mut i = 0usize;
    while i < board.len() && place < prizes.len() {
        let tied: Vec<&BoardEntry> = board.iter().filter(|b| b.score == board[i].score).collect();
        let span: Vec<u64> = prizes
            .iter()
            .skip(place)
            .take(tied.len())
            .copied()
            .collect();
        if promo.ties.as_deref() != Some("split") || tied.len() == 1 {
            for (j, t) in tied.iter().take(span.len()).enumerate() {
                if span[j] != 0 {
                    out.push((*t, span[j], (place + j + 1) as u64));
                }
            }
        } else {
            let pot: u64 = span.iter().sum();
            let n = tied.len() as u64;
            for (j, t) in tied.iter().enumerate() {
                // Hundredths left over go to the first to make it.
                let amount = pot / n + u64::from((j as u64) < pot % n);
                if amount != 0 {
                    out.push((*t, amount, (place + 1) as u64));
                }
            }
        }
        place += tied.len();
        i += tied.len();
    }
    out
}

const DAY_NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

fn hhmm(m: i64) -> String {
    format!("{:02}:{:02}", (m / 60) % 24, m % 60)
}

/// When a session or set hours are on: "Fri, Sat 20:00–02:00".
fn hours_text(promo: &Promo) -> String {
    if promo.every != "session" && promo.hours != Some(true) {
        return String::new();
    }
    let days = promo.days();
    let d = if days.len() == 7 {
        "Every day".to_string()
    } else {
        days.iter()
            .map(|&d| DAY_NAMES[d as usize % 7])
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "{d} {}–{}",
        hhmm(promo.from.unwrap_or(0)),
        hhmm(promo.to.unwrap_or(0))
    )
}

fn lower_category(c: &str) -> String {
    let n = category_name(c);
    n.strip_prefix("An ")
        .or_else(|| n.strip_prefix("A "))
        .unwrap_or(n)
        .to_lowercase()
}

/// A promotion's rules in a sentence.
pub fn promo_rules(promo: &Promo) -> String {
    let mut s = rules_core(promo);
    if promo.tourneys {
        s.push_str(" Tournament hands count too.");
    }
    s
}

fn rules_core(promo: &Promo) -> String {
    let when = match promo.every.as_str() {
        "hour" => "each hour",
        "day" => "each day",
        "week" => "each week",
        _ => "each session",
    };
    let games = promo.games_label();
    let hours = hours_text(promo);
    if promo.kind == "high_hand" {
        let prizes: Vec<String> = promo
            .prizes
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(chip_text)
            .collect();
        let places = if prizes.len() == 1 {
            format!("wins {}", prizes[0])
        } else {
            format!(
                "pays {} for 1st{}",
                prizes.join(" / "),
                if prizes.len() > 2 {
                    ", 2nd and 3rd"
                } else {
                    " and 2nd"
                }
            )
        };
        let min = lower_category(promo.min_category.as_deref().unwrap_or("full_house"));
        let mut s = format!("Best {games} hand {when} {places} ({min} or better).");
        if !hours.is_empty() {
            s.push_str(&format!(" {hours}."));
        }
        if promo.rollover == Some(true) {
            s.push_str(" No qualifying hand: the prize rolls over.");
        }
        return s;
    }
    let per = match promo.every.as_str() {
        "hour" => "an hour",
        "day" => "a day",
        "week" => "a week",
        _ => "a session",
    };
    let tiers: Vec<String> = promo
        .tiers
        .clone()
        .unwrap_or_default()
        .iter()
        .map(|t| {
            let cap = promo.tier_cap(t);
            let up = if cap != 0 {
                format!(" (up to {cap} {per})")
            } else {
                String::new()
            };
            format!(
                "{} {}{up}",
                lower_category(&t.category),
                chip_text(t.amount)
            )
        })
        .collect();
    let per_player = match promo.cap_per_player.unwrap_or(0) {
        0 => String::new(),
        n => format!(", {n} per player"),
    };
    let hours = if hours.is_empty() {
        String::new()
    } else {
        format!(" {hours}.")
    };
    format!("{games}: {}{per_player}.{hours}", tiers.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    // A new seed each call, so ids made in one test differ.
    fn rng() -> StdRng {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEED: AtomicU64 = AtomicU64::new(1);
        StdRng::seed_from_u64(SEED.fetch_add(1, Ordering::Relaxed))
    }

    fn cards(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn five_card_scores_and_names() {
        let name = |s: &str| hand_name(&score5(&cards(s)).unwrap());
        assert_eq!(name("Ah Kh Qh Jh Th"), "A royal flush");
        assert_eq!(name("9h 8h 7h 6h 5h"), "A straight flush, nine high");
        assert_eq!(name("Kh Kd Ks Kc 2h"), "Quad kings");
        assert_eq!(name("Ah Ad As Th Td"), "A full house, aces full of tens");
        assert_eq!(name("Ah 9h 7h 4h 2h"), "A flush, ace high");
        assert_eq!(name("5h 4d 3s 2c Ah"), "A straight, five high");
        assert_eq!(name("7h 7d 7s Kc 2h"), "Three sevens");
        assert_eq!(name("Jh Jd 4s 4c 2h"), "Two pair, jacks and fours");
        assert_eq!(name("10h 10d 4s 3c 2h"), "A pair of tens");
        assert_eq!(name("Ah Jd 9s 4c 2h"), "Ace high");
        // The wheel loses to a six-high straight; a royal beats a king-high straight flush.
        let sc = |s: &str| score5(&cards(s)).unwrap().score;
        assert!(sc("5h 4d 3s 2c Ah") < sc("6h 5d 4s 3c 2h"));
        assert!(sc("Ah Kh Qh Jh Th") > sc("Kh Qh Jh Th 9h"));
        assert_eq!(score5(&cards("Ah Kh Qh Jh Th")).unwrap().cat, 9);
    }

    #[test]
    fn qualifying_hands_use_two_hole_cards() {
        // Four hearts on the board and one in hand: no flush with two hole cards.
        let (best, _) = best_with_two(&cards("Ah 2c"), &cards("Kh Qh Jh 9h 3d")).unwrap();
        assert_ne!(CATEGORIES[best.cat], "flush");
        let rec = json!({"showdown": true, "id": "h1", "board": cards("Kh Qh Jh 9h 3d"), "seats": [
            {"i": 0, "name": "Ann", "cards": ["Ah", "Th"], "inHand": true, "folded": false},
            {"i": 1, "name": "Bob", "cards": null, "inHand": true, "folded": true},
        ]});
        let shown = shown_hands(&rec, &[json!({"seat": 0, "client_id": "ann"})]);
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].hand, "A royal flush");
        assert_eq!(shown[0].member, "ann");
    }

    fn club(n: usize) -> Promos {
        let mut c = Promos {
            code: "ABCD-EFGH".into(),
            ..Default::default()
        };
        for i in 0..n {
            let id = format!("p{i}");
            let m: Member = serde_json::from_value(
                json!({"id": id, "name": id.to_uppercase(), "status": "member", "balance": 100000}),
            )
            .unwrap();
            c.members.insert(id, m);
        }
        c
    }

    fn quads_hand(id: &str, at: u64, seat_member: u64) -> (Value, Vec<Value>) {
        let rec = json!({"id": id, "table": "t1", "hand": at, "game": "nlhe", "at": at, "pot": 1000, "showdown": true,
        "board": cards("Kh Kd 2s 7c 9d"), "seats": [
            {"i": 0, "name": "X", "cards": ["Ks", "Kc"], "inHand": true, "folded": false},
            {"i": 1, "name": "Y", "cards": ["2h", "3h"], "inHand": true, "folded": false},
            {"i": 2, "inHand": true}, {"i": 3, "inHand": true},
        ]});
        let players = vec![
            json!({"seat": 0, "client_id": format!("p{seat_member}")}),
            json!({"seat": 1, "client_id": "p9"}),
        ];
        (rec, players)
    }

    #[test]
    fn settings_are_checked_and_said_in_a_sentence() {
        let e = |s: Value| promo_settings(&s).unwrap_err().0;
        assert_eq!(e(json!({})), "Pick a kind of promotion.");
        assert_eq!(
            e(json!({"kind": "bonus", "every": "day"})),
            "Pick at least one game."
        );
        assert_eq!(
            e(json!({"kind": "bonus", "games": ["nlhe"], "every": "day"})),
            "Add at least one bonus: a hand and its prize."
        );
        assert_eq!(
            e(
                json!({"kind": "bonus", "games": ["nlhe"], "every": "day", "tiers": [{"category": "quads", "amount": 100}, {"category": "quads", "amount": 200}]})
            ),
            "Each hand can only have one bonus."
        );
        assert_eq!(
            e(json!({"kind": "high_hand", "games": ["nlhe"], "every": "day"})),
            "Give a prize, in chips."
        );
        assert_eq!(
            e(json!({"kind": "high_hand", "games": ["nlhe"], "every": "session", "prizes": [100]})),
            "Pick the days the session runs."
        );
        let mut c = club(0);
        let hh = c
            .new_promo(&json!({"kind": "high_hand", "games": ["nlhe", "plo4"], "every": "day", "prizes": [50000, 20000], "minCategory": "quads", "rollover": true}), 0, &mut rng())
            .unwrap();
        assert_eq!(
            promo_rules(&hh),
            "Best Hold’em, Omaha 4 hand each day pays 500 / 200 for 1st and 2nd (four of a kind or better). No qualifying hand: the prize rolls over."
        );
        let b = c
            .new_promo(&json!({"kind": "bonus", "games": ["nlhe"], "every": "week", "tiers": [{"category": "quads", "amount": 1000, "cap": 2}, {"category": "straight_flush", "amount": 2500}], "capPerPlayer": 1, "tourneys": true}), 0, &mut rng())
            .unwrap();
        assert_eq!(b.tiers.as_ref().unwrap()[0].category, "straight_flush");
        assert_eq!(
            promo_rules(&b),
            "Hold’em: straight flush 25, four of a kind 10 (up to 2 a week), 1 per player. Tournament hands count too."
        );
    }

    #[test]
    fn bonuses_pay_once_per_hand_capped_and_a_hand_counts_once() {
        let mut c = club(10);
        let b = c
            .new_promo(&json!({"kind": "bonus", "games": ["nlhe"], "every": "day", "tiers": [{"category": "quads", "amount": 1000, "cap": 1}]}), 0, &mut rng())
            .unwrap();
        let (rec, players) = quads_hand("h1", 1000, 0);
        let out = c.process_hand(&rec, &players, 1000, &mut rng());
        assert_eq!(out.paid.len(), 1);
        assert_eq!(
            (
                out.paid[0].member.as_str(),
                out.paid[0].amount,
                out.paid[0].status.as_str()
            ),
            ("p0", 1000, "paid")
        );
        assert_eq!(out.paid[0].hand_name, "Quad kings");
        // The same hand again counts once; another hits the tier's cap.
        assert!(
            c.process_hand(&rec, &players, 1000, &mut rng())
                .paid
                .is_empty()
        );
        let (rec2, players2) = quads_hand("h2", 2000, 1);
        assert!(
            c.process_hand(&rec2, &players2, 2000, &mut rng())
                .paid
                .is_empty()
        );
        assert_eq!(c.promos.paid[&format!("{}|0", b.id)], 1000);
        // The next day the cap starts again; the old day's counters go.
        c.settle_periods(86_400_000 + 1, &mut rng());
        assert!(c.promos.counts.is_empty());
        let (rec3, players3) = quads_hand("h3", 86_400_000 + 5, 1);
        assert_eq!(
            c.process_hand(&rec3, &players3, 86_400_000 + 5, &mut rng())
                .paid
                .len(),
            1
        );
    }

    #[test]
    fn a_high_hand_pays_at_the_end_of_its_period_and_can_be_voided() {
        let mut c = club(10);
        let hh = c
            .new_promo(&json!({"kind": "high_hand", "games": ["nlhe"], "every": "hour", "prizes": [5000], "minCategory": "trips"}), 0, &mut rng())
            .unwrap();
        let (rec, players) = quads_hand("h1", 1000, 0);
        let out = c.process_hand(&rec, &players, 1000, &mut rng());
        assert_eq!(out.leads.len(), 1);
        // Another hand of the same period: trips for p1, second on the board.
        let rec2 = json!({"id": "h2", "table": "t1", "hand": 2, "game": "nlhe", "at": 2000, "pot": 10, "showdown": true,
            "board": cards("7h 7d 2s 4c 9d"), "seats": [{"i": 0, "name": "Z", "cards": ["7s", "Ac"], "inHand": true}, {"i": 1, "inHand": true}, {"i": 2, "inHand": true}, {"i": 3, "inHand": true}]});
        c.process_hand(
            &rec2,
            &[json!({"seat": 0, "client_id": "p1"})],
            2000,
            &mut rng(),
        );
        assert_eq!(c.next_settle(), Some(3_600_000));
        assert!(c.settle_periods(3_599_999, &mut rng()).is_empty());
        let paid = c.settle_periods(3_600_000, &mut rng());
        assert_eq!(paid.len(), 1);
        assert_eq!(
            (paid[0].member.as_str(), paid[0].amount, paid[0].place),
            ("p0", 5000, 1)
        );
        assert_eq!(c.promos.history.len(), 1);
        // Voided within a day: the chips to take back, and the next best offered.
        let v = c
            .void_award(&paid[0].id, 3_700_000, "a misdeal", &mut rng())
            .unwrap();
        assert_eq!(v.take, 5000);
        let offered = v.offered.unwrap();
        assert_eq!(
            (offered.member.as_str(), offered.status.as_str()),
            ("p1", "held")
        );
        assert!(
            c.void_award(&paid[0].id, 3_700_000, "", &mut rng())
                .is_err()
        );
        let ok = c.approve_award(&offered.id, 3_800_000).unwrap();
        assert_eq!(ok.status, "paid");
        let _ = hh;
    }

    #[test]
    fn a_period_without_a_qualifying_hand_rolls_over() {
        let mut c = club(10);
        c.new_promo(&json!({"kind": "high_hand", "games": ["nlhe"], "every": "hour", "prizes": [1000], "minCategory": "quads", "rollover": true}), 0, &mut rng())
            .unwrap();
        // Played, shown down, but nothing good enough.
        let rec = json!({"id": "h1", "table": "t1", "hand": 1, "game": "nlhe", "at": 1000, "pot": 10, "showdown": true,
            "board": cards("7h 8d 2s 4c 9d"), "seats": [{"i": 0, "name": "Z", "cards": ["Ks", "Ac"], "inHand": true}, {"i": 1, "inHand": true}, {"i": 2, "inHand": true}, {"i": 3, "inHand": true}]});
        c.process_hand(
            &rec,
            &[json!({"seat": 0, "client_id": "p1"})],
            1000,
            &mut rng(),
        );
        assert!(c.settle_periods(3_600_000, &mut rng()).is_empty());
        assert_eq!(c.promos.history[0].rolled, Some(1000));
        let body = c.body(3_700_000);
        assert_eq!(body["promos"][0]["prize"]["nlhe"], 2000);
    }

    #[test]
    fn periods_in_the_managers_time_zone() {
        let promo: Promo = serde_json::from_value(json!({"every": "week", "tz": -300})).unwrap();
        // 1970-01-05 (Monday) 05:00 UTC is midnight in New York.
        let p = period_of(&promo, 4 * D + 5 * H).unwrap();
        assert_eq!((p.start, p.end), (4 * D + 5 * H, 11 * D + 5 * H));
        let session: Promo = serde_json::from_value(
            json!({"every": "session", "days": [5], "from": 1200, "to": 1380}),
        )
        .unwrap();
        // Friday 1970-01-02, 20:30 UTC.
        assert!(period_of(&session, D + 20 * H + 30 * 60_000).is_some());
        assert!(period_of(&session, D + 19 * H).is_none());
    }

    #[test]
    fn a_saved_promo_state_loads_and_saves_the_same() {
        let saved = json!({
            "list": [{"id": "x1", "kind": "bonus", "name": "Hand Bonus", "games": ["nlhe"], "every": "day", "tz": 0,
                "tiers": [{"category": "quads", "amount": 100, "cap": 0}], "capPerPlayer": 0, "minPlayers": 4, "minPot": 0,
                "tables": [], "tourneys": false, "budget": 0, "startsAt": 0, "endsAt": 0, "status": "running", "createdAt": 5, "next": null}],
            "boards": {}, "counts": {"x1|0|tier:quads": 1}, "paid": {"x1|0": 100},
            "awards": [{"id": "a1", "promo": "x1", "promoName": "Hand Bonus", "kind": "bonus", "period": "0", "member": "p0", "name": "P0",
                "amount": 100, "hand": "h1", "table": "t1", "game": "nlhe", "category": "quads", "handName": "Quad kings", "cards": ["Kh"],
                "place": 0, "at": 9, "status": "paid", "entry": 4, "paidAt": 9}],
            "history": [], "hands": [{"hand": "h1", "anything": true}], "seen": ["t1||1"]
        });
        let s: PromoState = serde_json::from_value(saved.clone()).unwrap();
        assert_eq!(serde_json::to_value(&s).unwrap(), saved);
    }
}
