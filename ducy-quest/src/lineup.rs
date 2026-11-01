//! Who sits at a Bankroll Quest table (ducy-web #302): bot ids by stakes,
//! room and time.
//!
//! - 1/2 and 2/5: the room's generic players, regs and tourists, more
//!   tourists on a weekend evening. Mostly ABC players (grinders), now and
//!   then a maniac, a rock or a calling station, now and then one of the
//!   softer Regulars, and rarely a crusher.
//! - 5/10: fewer ABC players, more Regulars, and crushers often.
//! - 10/20: four strong Regulars, or three and a rock, and an ABC player or
//!   now and then a bad one.
//! - 25/50 and 50/100: the tough Regulars (the top of the ladder's order),
//!   and sometimes a whale.
//!
//! The same for a table all through a block of game hours, so a table you
//! look at in the lobby is the one you sit down at; within a room, no one
//! sits at two tables at once. [`lineup_stacks`] gives the running game's
//! stacks.

use std::collections::HashSet;

use ducy_bot_games::ladder::ORDER;
use serde::Serialize;

use crate::people::{
    Reg, name_of, parse_bot_id, person_of, roster, tourist, tourist_share, wildness,
};
use crate::roll;
use crate::world::{Limits, js_round, lobby, region_of, room_by_id, world};

/// Bot seats at a table: with you, 6-handed.
pub const LINEUP_SEATS: usize = 5;
/// The wild ones who sometimes sit in big games, and only there.
pub const WHALES: [&str; 2] = ["milk_king", "nik_airbag"];
/// The chance a big game has a whale in it, per block.
pub const WHALE_CHANCE: f64 = 0.35;
// A table keeps its line-up for this many game hours.
const BLOCK_HOURS: f64 = 2.0;
// The chance a reg is in the building in a block: at home, and visiting.
const AT_HOME: f64 = 0.5;
const VISITING: f64 = 0.08;
// Tourists in a room's pool for a game day.
const TOURISTS_A_DAY: f64 = 150.0;
// Spare players a table keeps for seats that come free.
const BENCH: usize = 4;
/// 10/20: three strong players and a rock this often, else four.
pub const ROCK_AT_10_20: f64 = 0.5;
/// 10/20: a bad player in the last seat this often.
pub const BAD_AT_10_20: f64 = 0.2;

/// Who sits in a seat at a generic table, by stakes: the share of each style.
pub fn style_mix(level: &str) -> Vec<(&'static str, f64)> {
    if level == "low" {
        vec![
            ("grinder", 0.66),
            ("rock", 0.09),
            ("maniac", 0.09),
            ("station", 0.08),
            ("character", 0.05),
            ("crusher", 0.03),
        ]
    } else {
        vec![
            ("grinder", 0.4),
            ("rock", 0.07),
            ("maniac", 0.07),
            ("station", 0.04),
            ("character", 0.17),
            ("crusher", 0.25),
        ]
    }
}
const MIX_10_20: [(&str, f64); 5] = [
    ("strong", 0.6),
    ("rock", 0.15),
    ("grinder", 0.17),
    ("station", 0.04),
    ("maniac", 0.04),
];

fn ids(list: &[&str]) -> Vec<String> {
    list.iter()
        .filter(|id| !WHALES.contains(id))
        .map(|s| s.to_string())
        .collect()
}
/// Regulars by stakes level: the bottom of the order at 1/2 and 2/5, the
/// middle at 5/10 and 10/20, the top at 25/50 and 50/100.
pub fn pool(level: &str) -> Vec<String> {
    match level {
        "low" => ids(&ORDER[..11]),
        "mid" => ids(&ORDER[5..15]),
        _ => ids(&ORDER[10..]),
    }
}
/// The crushers: the strongest Regulars.
pub fn crushers() -> Vec<String> {
    ids(&ORDER[ORDER.len() - 6..])
}
/// The strong Regulars who make up a 10/20 game.
pub fn strong() -> Vec<String> {
    ids(&ORDER[ORDER.len() - 10..])
}
/// A stakes' level: `low`, `mid` or `high`.
pub fn pool_for(stakes: &str) -> &'static str {
    match stakes {
        "1/2" | "2/5" => "low",
        "5/10" | "10/20" => "mid",
        _ => "high",
    }
}
/// The big games: 25/50 and up.
pub fn big_game(stakes: &str) -> bool {
    pool_for(stakes) == "high"
}
fn block_of(minutes: f64) -> i64 {
    (minutes / (60.0 * BLOCK_HOURS)).floor() as i64
}
fn has(have: Option<&[String]>, id: &str) -> bool {
    have.is_none_or(|h| h.iter().any(|x| x == id))
}

/// The room a table id belongs to: `deuce-1/2-1` is at `deuce`.
pub fn room_of_table(table_id: &str) -> &str {
    table_id.split('-').next().unwrap_or("")
}

/// The five Regulars (or, with `generic`, players) at table `table_id` at
/// game time `minutes`. `have` is the personality ids this engine has:
/// anyone missing is skipped.
pub fn lineup_for(
    seed: u64,
    table_id: &str,
    stakes: &str,
    minutes: f64,
    have: Option<&[String]>,
    generic: bool,
) -> Vec<String> {
    if generic && !big_game(stakes) {
        if let Some((_, t)) = room_lineups(seed, room_of_table(table_id), minutes, have)
            .into_iter()
            .find(|(id, _)| id == table_id)
        {
            return t.seats;
        }
    }
    regulars_lineup(seed, table_id, stakes, minutes, have)
}

fn regulars_lineup(
    seed: u64,
    table_id: &str,
    stakes: &str,
    minutes: f64,
    have: Option<&[String]>,
) -> Vec<String> {
    let block = block_of(minutes);
    let pool: Vec<String> = pool(pool_for(stakes))
        .into_iter()
        .filter(|id| has(have, id))
        .collect();
    let mut picked: Vec<String> = Vec::new();
    // A whale takes the first seat of some big games.
    if big_game(stakes) && roll!(seed; table_id, "whale", block) < WHALE_CHANCE {
        let whales: Vec<&str> = WHALES.iter().copied().filter(|id| has(have, id)).collect();
        if !whales.is_empty() {
            let i = (roll!(seed; table_id, "which", block) * whales.len() as f64).floor() as usize;
            picked.push(whales[i].to_string());
        }
    }
    let mut free: Vec<String> = pool.into_iter().filter(|id| !picked.contains(id)).collect();
    let mut i = 0usize;
    while picked.len() < LINEUP_SEATS && !free.is_empty() {
        let at = (roll!(seed; table_id, "seat", block, i) * free.len() as f64).floor() as usize;
        picked.push(free.remove(at));
        i += 1;
    }
    picked
}

/// The buy-in range at `blinds` in a room, in chips: the room may set its
/// own limits in big blinds.
pub fn buy_in_range(blinds: [i64; 2], room: &Limits) -> (f64, f64) {
    let bb = blinds[1] as f64;
    let min = room.min_buy_in.unwrap_or(40.0);
    let max = room.max_buy_in.unwrap_or(200.0);
    (min * bb, max * bb)
}

// How strong player `id` is, 0 (softest) to 1 (toughest): a Regular by the
// ladder's order, a generic player by family.
fn strength_of(id: &str) -> f64 {
    if let Some((family, _)) = parse_bot_id(id) {
        return match family {
            "station" => 0.1,
            "maniac" => 0.12,
            "rock" => 0.3,
            "grinder" => 0.65,
            _ => 0.3,
        };
    }
    match ORDER.iter().position(|x| *x == id) {
        Some(at) => at as f64 / (ORDER.len() - 1) as f64,
        None => 0.5,
    }
}

/// What player `id` buys in (and tops up) for at table `table_id`, in chips,
/// within the room's range: the same all through a block of game hours.
/// Weaker Regulars often buy in short, stronger ones often for the maximum,
/// the rest for 100 big blinds; whales always for the maximum; a generic
/// player has their own habit.
pub fn buy_in_for(
    seed: u64,
    table_id: &str,
    id: &str,
    blinds: [i64; 2],
    room: &Limits,
    minutes: f64,
) -> f64 {
    let (min, max) = buy_in_range(blinds, room);
    let bb = blinds[1] as f64;
    let block = block_of(minutes);
    let r = |key: &str| roll!(seed; table_id, "buyin", id, block, key);
    if WHALES.contains(&id) {
        return max;
    }
    if let Some(g) = person_of(id) {
        return min.max(max.min(g.buy_in_bb * bb));
    }
    let strength = strength_of(id);
    let p_short = 0.05 + 0.4 * (1.0 - strength);
    let p_max = 0.15 + 0.45 * strength;
    let h = r("habit");
    if h < p_short {
        return max.min(min + (r("short") * 3.0).floor() * 10.0 * bb);
    }
    if h < p_short + p_max {
        return max;
    }
    min.max(max.min(100.0 * bb))
}

// How long a player you walk up to has been sitting: the share in each
// 2-hour band, and the rest 12 hours or more.
const SEATED_HOURS: [(f64, f64); 6] = [
    (2.0, 0.29),
    (4.0, 0.20),
    (6.0, 0.13),
    (8.0, 0.09),
    (10.0, 0.064),
    (12.0, 0.048),
];
// Past 12 hours: an exponential tail (mean 7 hours), to at most 36.
const LONG_TAIL_HOURS: f64 = 7.0;

/// How long a player has been sitting, in hours, from rolls `u` and `v`.
pub fn seated_hours(u: f64, v: f64) -> f64 {
    let mut below = 0.0;
    for (up_to, share) in SEATED_HOURS {
        if u < below + share {
            return up_to - 2.0 + 2.0 * (u - below) / share;
        }
        below += share;
    }
    36f64.min(12.0 + LONG_TAIL_HOURS * -(1.0 - v).ln())
}

/// A player's stack when you walk up, and what they top up to.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stack {
    /// In front of them.
    pub stack: f64,
    /// Their buy-in.
    pub buy_in: f64,
}

/// What each of `ids` has in front of them when you walk up to table
/// `table_id`, a game already running, fitted to a simulation of live 6-max
/// tables: most within a buy-in or two of where they started, some winners
/// at 2-3x the max buy-in, now and then one who's been there all day at
/// 5-10x. One who lost over half a buy-in has topped up. Whole big blinds.
pub fn lineup_stacks(
    seed: u64,
    table_id: &str,
    ids: &[String],
    blinds: [i64; 2],
    room: &Limits,
    minutes: f64,
) -> Vec<Stack> {
    let (_, max) = buy_in_range(blinds, room);
    let bb = blinds[1] as f64;
    let block_minutes = 60.0 * BLOCK_HOURS;
    let block = (minutes / block_minutes).floor() as i64;
    let into = (minutes % block_minutes) / 60.0;
    ids.iter()
        .map(|id| {
            let r = |key: &str| roll!(seed; table_id, "stack", id, block, key);
            let buy_in = buy_in_for(seed, table_id, id, blinds, room, minutes);
            let strength = strength_of(id);
            let whale = WHALES.contains(&id.as_str());
            let hours = seated_hours(r("hours"), r("tail")) + into;
            let log_mean = 0.25 * (1.0 + hours).ln() - 0.37;
            let log_spread = 0.07 + 0.13 * hours.sqrt();
            let mean = log_mean + 0.31 * (strength - 0.5) + if whale { -0.1 } else { 0.0 };
            let spread = log_spread * if whale { 1.4 } else { 1.0 };
            let (u, v) = (r("u"), r("v"));
            let normal = (-2.0 * (1.0 - u).ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos();
            let ratio = (mean + spread * normal).exp();
            let mut stack = if ratio >= 1.0 {
                buy_in + (ratio - 1.0) * (buy_in + max) / 2.0
            } else {
                buy_in * ratio
            };
            if stack < buy_in / 2.0 {
                stack = buy_in * (0.5 + 0.6 * r("reload"));
            }
            stack = stack.min(12.0 * max);
            Stack {
                stack: bb.max(js_round(stack / bb) * bb),
                buy_in,
            }
        })
        .collect()
}

/// A table's line-up.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Lineup {
    /// The five in the seats.
    pub seats: Vec<String>,
    /// Players who can take a seat that comes free.
    pub bench: Vec<String>,
}

struct Table {
    id: String,
    stakes: String,
    low: bool,
}

struct Seating<'a> {
    seed: u64,
    room_id: &'a str,
    minutes: f64,
    block: i64,
    day: i64,
    have: Option<&'a [String]>,
    regs: Vec<Reg>,
    taken: HashSet<String>,
    names: HashSet<String>,
    share: f64,
    wild: f64,
    slot: i64,
}

impl Seating<'_> {
    fn free(&self, id: &str) -> bool {
        !self.taken.contains(id) && name_of(id).is_none_or(|n| !self.names.contains(&n))
    }
    fn take(&mut self, id: String, list: &mut Vec<String>) {
        if let Some(n) = name_of(&id) {
            self.names.insert(n);
        }
        self.taken.insert(id.clone());
        list.push(id);
    }
    // A tourist, of `family` if one is asked for.
    fn next_tourist(&mut self, family: Option<&str>) -> Option<String> {
        for _ in 0..20 {
            let slot = (roll!(self.seed; self.room_id, "slot", self.block, self.slot)
                * TOURISTS_A_DAY)
                .floor() as i64;
            self.slot += 1;
            let t = tourist(
                self.seed,
                self.room_id,
                self.day,
                slot,
                self.minutes,
                family,
            );
            if self.free(&t) {
                return Some(t);
            }
        }
        None
    }
    // A reg in the building who plays these stakes, of `family` if asked.
    fn next_reg(&self, level: &str, key: &str, family: Option<&str>) -> Option<String> {
        let options: Vec<&Reg> = self
            .regs
            .iter()
            .filter(|r| {
                r.level == level
                    && self.free(&r.id)
                    && family.is_none_or(|f| parse_bot_id(&r.id).is_some_and(|(x, _)| x == f))
            })
            .collect();
        if options.is_empty() {
            return None;
        }
        let i = (roll!(self.seed; key, self.block) * options.len() as f64).floor() as usize;
        Some(options[i].id.clone())
    }
    fn named(&self, pool: &[String], key: &str) -> Option<String> {
        let options: Vec<&String> = pool
            .iter()
            .filter(|id| has(self.have, id) && !self.taken.contains(*id))
            .collect();
        if options.is_empty() {
            return None;
        }
        let i = (roll!(self.seed; key, self.block) * options.len() as f64).floor() as usize;
        Some(options[i].clone())
    }
    // A 10/20 table's five seats, in order.
    fn plan_10_20(&self, t: &Table) -> Vec<&'static str> {
        let r = |k: &str| roll!(self.seed; t.id, "10/20", k, self.block);
        let rock = r("rock") < ROCK_AT_10_20;
        let last = if r("bad") < BAD_AT_10_20 {
            if r("which") < 0.5 {
                "station"
            } else {
                "maniac"
            }
        } else {
            "grinder"
        };
        let mut plan = vec!["strong"; if rock { 3 } else { 4 }];
        if rock {
            plan.push("rock");
        }
        plan.push(last);
        plan
    }
    fn style_for(&self, t: &Table, r: f64) -> &'static str {
        let mut mix: Vec<(&'static str, f64)> = if t.stakes == "10/20" {
            MIX_10_20.to_vec()
        } else {
            style_mix(if t.low { "low" } else { "mid" })
        };
        for (k, w) in &mut mix {
            if *k == "maniac" {
                *w *= 1.0 + self.wild;
            }
        }
        let mut x = r * mix.iter().fold(0.0, |a, (_, w)| a + w);
        for (k, w) in &mix {
            x -= w;
            if x < 0.0 {
                return k;
            }
        }
        "grinder"
    }
    fn fill(&mut self, t: &Table, list: &mut Vec<String>, n: usize, key: &str) {
        let plan = (t.stakes == "10/20" && key == "seat").then(|| self.plan_10_20(t));
        let strong = strong();
        let mut i = 0usize;
        while list.len() < n && i < n * 8 {
            let k = format!("{}|{key}|{i}", t.id);
            let mut style = match &plan {
                Some(p) => p.get(list.len()).copied().unwrap_or(""),
                None => self.style_for(t, roll!(self.seed; t.id, key, "style", self.block, i)),
            };
            if style == "strong"
                && !strong
                    .iter()
                    .any(|id| has(self.have, id) && !self.taken.contains(id))
            {
                style = "grinder";
            }
            let who = match style {
                "strong" => self.named(&strong, &format!("{k}|strong")),
                "crusher" => self.named(&crushers(), &format!("{k}|crusher")),
                "character" => {
                    let c = crushers();
                    let p: Vec<String> = pool(pool_for(&t.stakes))
                        .into_iter()
                        .filter(|id| !c.contains(id))
                        .collect();
                    self.named(&p, &format!("{k}|reg"))
                }
                _ => {
                    // A tourist or a reg (more tourists on a busy night), of that style.
                    let share = if t.low { self.share } else { self.share / 2.0 };
                    let tourist_first = roll!(self.seed; t.id, key, self.block, i) < share;
                    let (level, other) = if t.low {
                        ("low", "mid")
                    } else {
                        ("mid", "low")
                    };
                    let family = (!style.is_empty()).then_some(style);
                    // A reg who usually plays the other stakes will sit in when theirs is short.
                    let reg = |s: &Self| {
                        s.next_reg(level, &k, family)
                            .or_else(|| s.next_reg(other, &format!("{k}|other"), family))
                    };
                    if tourist_first {
                        match self.next_tourist(family) {
                            Some(t) => Some(t),
                            None => reg(self),
                        }
                    } else {
                        match reg(self) {
                            Some(r) => Some(r),
                            None => self.next_tourist(family),
                        }
                    }
                }
            };
            if let Some(w) = who.filter(|w| self.free(w)) {
                self.take(w, list);
            }
            i += 1;
        }
    }
}

/// Every low and mid stakes table at room `room_id` at game time `minutes`,
/// seated together so no one is at two tables: each table's id and line-up.
pub fn room_lineups(
    seed: u64,
    room_id: &str,
    minutes: f64,
    have: Option<&[String]>,
) -> Vec<(String, Lineup)> {
    let lobby = lobby(seed, minutes);
    let Some(here) = lobby.iter().find(|r| r.room == room_id) else {
        return Vec::new();
    };
    let block = block_of(minutes);
    // The regs in the building this block: most of the home roster, and a few
    // from the region's other rooms.
    let region = room_by_id(room_id)
        .and_then(region_of)
        .map(|r| r.id.clone());
    let mut regs = Vec::new();
    for room in world()
        .rooms
        .iter()
        .filter(|r| region_of(r).map(|x| x.id.clone()) == region)
    {
        for r in roster(seed, &room.id) {
            let chance = if room.id == room_id {
                AT_HOME
            } else {
                VISITING
            };
            if roll!(seed; r.id, room_id, "here", block) < chance {
                regs.push(r);
            }
        }
    }
    let mut s = Seating {
        seed,
        room_id,
        minutes,
        block,
        day: (minutes / 1440.0).floor() as i64,
        have,
        regs,
        taken: HashSet::new(),
        names: HashSet::new(),
        share: tourist_share(minutes),
        wild: wildness(minutes),
        slot: 0,
    };
    let tables: Vec<Table> = here
        .games
        .iter()
        .filter(|g| !big_game(&g.stakes))
        .flat_map(|g| {
            g.ids.iter().map(|id| Table {
                id: id.clone(),
                stakes: g.stakes.clone(),
                low: pool_for(&g.stakes) == "low",
            })
        })
        .collect();
    // Every table's seats first, then the benches, so a bench never leaves a
    // seat empty. Names are only kept apart within a table.
    let mut out: Vec<(String, Lineup)> = Vec::new();
    let mut table_names: Vec<HashSet<String>> = Vec::new();
    for t in &tables {
        s.names.clear();
        let mut seats = Vec::new();
        s.fill(t, &mut seats, LINEUP_SEATS, "seat");
        table_names.push(s.names.clone());
        out.push((
            t.id.clone(),
            Lineup {
                seats,
                bench: Vec::new(),
            },
        ));
    }
    for (i, t) in tables.iter().enumerate() {
        s.names = table_names[i].clone();
        let mut bench = Vec::new();
        s.fill(t, &mut bench, BENCH, "bench");
        out[i].1.bench = bench;
    }
    out
}

/// Who replaces a player who leaves mid-session: anyone from the table's
/// pool (the stakes' Regulars), or, at a generic table, its bench.
pub fn replacement_pool(
    stakes: &str,
    have: Option<&[String]>,
    seed: u64,
    table_id: Option<&str>,
    minutes: f64,
    generic: bool,
) -> Vec<String> {
    if let (true, false, Some(tid)) = (generic, big_game(stakes), table_id) {
        if let Some((_, t)) = room_lineups(seed, room_of_table(tid), minutes, have)
            .into_iter()
            .find(|(id, _)| id == tid)
        {
            if !t.bench.is_empty() {
                return t.bench;
            }
        }
    }
    pool(pool_for(stakes))
        .into_iter()
        .filter(|id| has(have, id))
        .collect()
}
