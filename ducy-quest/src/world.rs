//! Bankroll Quest's card rooms and their schedules (ducy-web #301): made-up
//! rooms in three regions (Texas, Los Angeles, Florida), defined as data
//! (`world.json`), whose games come and go with the hour and the day.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::roll;

/// A city.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct City {
    /// Its id.
    pub id: String,
    /// Its name.
    pub name: String,
    /// A small town: where a run starts.
    #[serde(default)]
    pub small: bool,
}

/// A region you can put down roots in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Region {
    /// Its id.
    pub id: String,
    /// Its name.
    pub name: String,
    /// Its cities, the small town first.
    pub cities: Vec<City>,
}

/// A game a room spreads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoomGame {
    /// Its stakes: `1/2`, `2/5`, …
    pub stakes: String,
    /// `nlhe` or `plo4`.
    pub game: String,
    /// How many tables run at its busiest.
    pub tables: f64,
    /// How often it runs at all in an hour that could have it.
    #[serde(default)]
    pub chance: Option<f64>,
    /// The hours a big game can start: `evenings` or `weekend-nights`.
    #[serde(default)]
    pub when: Option<String>,
}

/// A card room.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Room {
    /// Its id (no dashes: a table id starts with it).
    pub id: String,
    /// Its city.
    pub city: String,
    /// Its name.
    pub name: String,
    /// A home club, known only to the people who live there.
    #[serde(default)]
    pub local: bool,
    /// Its own minimum buy-in, in big blinds.
    #[serde(default)]
    pub min_buy_in: Option<f64>,
    /// Its own maximum buy-in, in big blinds.
    #[serde(default)]
    pub max_buy_in: Option<f64>,
    /// Its games.
    pub games: Vec<RoomGame>,
}

/// A room's buy-in limits, in big blinds: none for the usual 40 to 200.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    /// The room's minimum.
    #[serde(default)]
    pub min_buy_in: Option<f64>,
    /// The room's maximum.
    #[serde(default)]
    pub max_buy_in: Option<f64>,
}

impl Limits {
    /// Room `id`'s limits (the usual for an unknown room).
    pub fn of(id: &str) -> Self {
        room_by_id(id).map(Self::from).unwrap_or_default()
    }
}

impl From<&Room> for Limits {
    fn from(r: &Room) -> Self {
        Limits {
            min_buy_in: r.min_buy_in,
            max_buy_in: r.max_buy_in,
        }
    }
}

/// A face base's identity: what names and backgrounds come from.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Base {
    /// The name pool.
    pub pool: String,
    /// `f` or `m`.
    pub gender: String,
    /// `young`, `middle` or `older`.
    pub age: String,
    /// The skin tone.
    pub skin: String,
    /// Fixed headwear, or empty.
    #[serde(default)]
    pub wear: String,
}

pub(crate) type Weights = Vec<(String, f64)>;

/// The world's data.
#[derive(Debug, Deserialize)]
pub struct World {
    /// Stakes by name, in hundredths: [small blind, big blind].
    pub stakes: Vec<(String, [i64; 2])>,
    /// The regions.
    pub regions: Vec<Region>,
    /// The rooms.
    pub rooms: Vec<Room>,
    /// The face bases.
    pub bases: Vec<Base>,
    /// First names: pool → gender → age → names.
    pub names: BTreeMap<String, BTreeMap<String, BTreeMap<String, Vec<String>>>>,
    /// The backgrounds of the people in each region's rooms.
    pub region_mix: BTreeMap<String, Weights>,
    /// And within a background, which name pools.
    pub pool_mix: BTreeMap<String, BTreeMap<String, Weights>>,
}

/// The world's data.
pub fn world() -> &'static World {
    static W: OnceLock<World> = OnceLock::new();
    W.get_or_init(|| serde_json::from_str(include_str!("world.json")).expect("world.json"))
}

/// Blinds at stakes `name`.
pub fn stakes(name: &str) -> Option<[i64; 2]> {
    world()
        .stakes
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, b)| *b)
}

/// A game's key for its dice and table ids: Hold'em's is its stakes, so
/// worlds from before Omaha keep their Hold'em tables.
pub fn game_key(g: &RoomGame) -> String {
    if g.game == "nlhe" {
        g.stakes.clone()
    } else {
        format!("{} {}", g.game, g.stakes)
    }
}

/// A room by id.
pub fn room_by_id(id: &str) -> Option<&'static Room> {
    world().rooms.iter().find(|r| r.id == id)
}

/// A city by id, with its region.
pub fn city_by_id(id: &str) -> Option<(&'static City, &'static Region)> {
    world()
        .regions
        .iter()
        .find_map(|r| r.cities.iter().find(|c| c.id == id).map(|c| (c, r)))
}

/// A room's region.
pub fn region_of(room: &Room) -> Option<&'static Region> {
    city_by_id(&room.city).map(|(_, r)| r)
}

/// The room a run starts in, in region `region_id`: home, its small town's
/// club.
pub fn start_room(region_id: &str) -> Option<&'static Room> {
    let w = world();
    let town = w
        .regions
        .iter()
        .find(|r| r.id == region_id)?
        .cities
        .iter()
        .find(|c| c.small)?;
    w.rooms
        .iter()
        .find(|r| r.local && r.city == town.id)
        .or_else(|| w.rooms.iter().find(|r| r.city == town.id))
}

/// How far room `to` is from room `from`: `town`, `region`, `far`, or none
/// for the same room.
pub fn trip_kind(from: &Room, to: &Room) -> Option<&'static str> {
    if from.id == to.id {
        return None;
    }
    if from.city == to.city {
        return Some("town");
    }
    let same = matches!((region_of(from), region_of(to)), (Some(a), Some(b)) if a.id == b.id);
    Some(if same { "region" } else { "far" })
}

/// Game minutes to get from one room to another: across town, to another
/// city in the region, or to another region.
pub fn travel_minutes(from: &Room, to: &Room) -> f64 {
    match trip_kind(from, to) {
        Some("town") => 6.0,
        Some("region") => 120.0,
        Some(_) => 360.0,
        None => 0.0,
    }
}

/// What a trip costs: across town is free, another city is gas, another
/// region airfare.
pub fn trip_cost(from: &Room, to: &Room) -> f64 {
    match trip_kind(from, to) {
        Some("region") => 4000.0,
        Some("far") => 30000.0,
        _ => 0.0,
    }
}

/// A hotel night where `room` is.
pub fn hotel_cost(room: &Room) -> f64 {
    if city_by_id(&room.city).is_some_and(|(c, _)| c.small) {
        8000.0
    } else {
        15000.0
    }
}

/// What moving home from city `from_city` to `room`'s city costs, or 0 if
/// it's already home.
pub fn move_cost(from_city: Option<&str>, room: &Room) -> f64 {
    let Some((to, region)) = city_by_id(&room.city) else {
        return 0.0;
    };
    if Some(to.id.as_str()) == from_city {
        return 0.0;
    }
    let same = from_city
        .and_then(city_by_id)
        .is_some_and(|(_, r)| r.id == region.id);
    if same { 50000.0 } else { 200000.0 }
}

// How busy the rooms are, 0 to 1, by hour (0 midnight).
const HOURLY: [f64; 24] = [
    0.36, 0.28, 0.2, 0.12, 0.08, 0.08, 0.08, 0.12, 0.16, 0.2, 0.28, 0.36, 0.4, 0.44, 0.48, 0.48,
    0.52, 0.56, 0.64, 0.72, 0.8, 0.8, 0.72, 0.56,
];

/// How busy the rooms are on `day` (0 Monday) at `hour`: quiet mornings,
/// busy evenings, Friday and Saturday nights busiest and late on into the
/// night.
pub fn activity(day: i64, hour: i64) -> f64 {
    let weekend_night = (day == 4 || day == 5) && (hour >= 18 || hour < 1);
    let late_weekend = (day == 5 || day == 6) && (0..4).contains(&hour);
    let mut a = HOURLY[hour.clamp(0, 23) as usize];
    if weekend_night {
        a *= 1.25;
    }
    if late_weekend {
        a = a.max(0.55);
    }
    a.min(1.0)
}

/// `Math.round`: halves go up.
pub fn js_round(x: f64) -> f64 {
    let f = x.floor();
    if x - f >= 0.5 { f + 1.0 } else { f }
}

/// Day of the week (0 Monday), hour and minute of game time `minutes`.
pub fn day_hour(minutes: f64) -> (i64, i64, i64) {
    let m = minutes.floor() as i64;
    (
        m.div_euclid(1440) % 7,
        m.rem_euclid(1440) / 60,
        m.rem_euclid(60),
    )
}

fn in_window(when: Option<&str>, day: i64, hour: i64) -> bool {
    match when {
        Some("evenings") => hour >= 19 || hour < 2,
        Some("weekend-nights") => (day == 4 || day == 5) && (hour >= 20 || hour < 3),
        _ => true,
    }
}

// Big games run in blocks of this many game hours once they start.
const BLOCK_HOURS: i64 = 4;

/// How many tables of a room's game run at game time `minutes`.
pub fn tables_running(seed: u64, room: &Room, game: &RoomGame, minutes: f64) -> usize {
    let m = minutes.floor() as i64;
    let (day, hour, _) = day_hour(minutes);
    let a = activity(day, hour);
    if game.when.is_some() || game.chance.is_some() {
        if !in_window(game.when.as_deref(), day, hour) {
            return 0;
        }
        // A big game runs for a block of hours once it starts: one roll a block.
        let block = m.div_euclid(60 * BLOCK_HOURS);
        let chance = game.chance.unwrap_or(1.0) * if game.when.is_some() { 1.0 } else { a };
        if roll!(seed; room.id, game_key(game), "block", block) >= chance {
            return 0;
        }
        return js_round(game.tables * a).max(1.0) as usize;
    }
    // A little noise an hour, so two Fridays aren't identical.
    let hour_index = m.div_euclid(60);
    let noise = 0.85 + 0.3 * roll!(seed; room.id, game_key(game), "hour", hour_index);
    let least = if a >= 0.2 { 1.0 } else { 0.0 };
    js_round(game.tables * a * noise).max(least) as usize
}

/// A game running in a room now.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Running {
    /// Its stakes.
    pub stakes: String,
    /// `nlhe` or `plo4`.
    pub game: String,
    /// Its blinds.
    pub blinds: [i64; 2],
    /// How many tables.
    pub tables: usize,
    /// The tables' ids, stable while a table runs.
    pub ids: Vec<String>,
}

/// A room's games now.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RoomNow {
    /// The room's id.
    pub room: String,
    /// Its games running.
    pub games: Vec<Running>,
}

/// Every room's games at game time `minutes`.
pub fn lobby(seed: u64, minutes: f64) -> Vec<RoomNow> {
    world()
        .rooms
        .iter()
        .map(|room| RoomNow {
            room: room.id.clone(),
            games: room
                .games
                .iter()
                .filter_map(|g| {
                    let tables = tables_running(seed, room, g, minutes);
                    let key = game_key(g).replacen(' ', "-", 1);
                    (tables > 0).then(|| Running {
                        stakes: g.stakes.clone(),
                        game: g.game.clone(),
                        blinds: stakes(&g.stakes).unwrap_or([0, 0]),
                        tables,
                        ids: (1..=tables)
                            .map(|i| format!("{}-{key}-{i}", room.id))
                            .collect(),
                    })
                })
                .collect(),
        })
        .collect()
}

/// The smallest minimum buy-in anywhere: below it, a quest is broke.
pub fn cheapest_buy_in(min_bb: f64) -> f64 {
    world()
        .rooms
        .iter()
        .flat_map(|r| {
            r.games.iter().map(move |g| {
                r.min_buy_in.unwrap_or(min_bb) * stakes(&g.stakes).map_or(0, |b| b[1]) as f64
            })
        })
        .fold(f64::INFINITY, f64::min)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_world_reads() {
        let w = world();
        assert_eq!(w.regions.len(), 3);
        assert!(w.rooms.iter().all(|r| !r.id.contains('-')));
        assert!(w.rooms.iter().all(|r| city_by_id(&r.city).is_some()));
        assert_eq!(w.bases.len(), 60);
        assert_eq!(start_room("tx").unwrap().id, "clubmesquite");
        assert_eq!(cheapest_buy_in(40.0), 8000.0);
    }

    #[test]
    fn trips() {
        let r = |id| room_by_id(id).unwrap();
        assert_eq!(trip_kind(r("deuce"), r("deuce")), None);
        assert_eq!(trip_kind(r("mesquite"), r("clubmesquite")), Some("town"));
        assert_eq!(trip_kind(r("deuce"), r("bayou")), Some("region"));
        assert_eq!(trip_kind(r("deuce"), r("ivory")), Some("far"));
        assert_eq!(travel_minutes(r("deuce"), r("ivory")), 360.0);
        assert_eq!(trip_cost(r("deuce"), r("bayou")), 4000.0);
        assert_eq!(hotel_cost(r("mesquite")), 8000.0);
        assert_eq!(move_cost(Some("mesquite"), r("deuce")), 50000.0);
        assert_eq!(move_cost(Some("mesquite"), r("ivory")), 200000.0);
        assert_eq!(move_cost(Some("austin"), r("deuce")), 0.0);
    }

    #[test]
    fn js_round_rounds_halves_up() {
        assert_eq!(js_round(2.5), 3.0);
        assert_eq!(js_round(-2.5), -2.0);
        assert_eq!(js_round(0.499_999_999_999_999_94), 0.0);
    }
}
