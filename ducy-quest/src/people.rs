//! The card rooms' generic players (ducy-web #302, #305): the regs and
//! tourists, played by ducy-play's generic bots, which seat by the id
//! `g-<family>-<seed>`.
//!
//! - Families: rock, calling station, maniac, grinder.
//! - Regs are a room's familiar faces: a roster per room, the same in every
//!   reload of a world. Each has a home room and the stakes they usually play.
//! - Tourists pass through: one is made for a room and a game day.
//! - Leans, never rules: regs are more often grinders or rocks, tourists more
//!   often stations or maniacs; late weekend nights run wilder; older players
//!   lean rock.
//! - A person's name comes from one of 60 face bases (their name pool, gender
//!   and age) and their seed alone, so a bot id always names the same person.
//!   Who a room gets follows its region's mix. Their drawn look is the page's.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde::Serialize;

use crate::dice::{roll_str, seed_from};
use crate::roll;
use crate::world::{Base, Weights, region_of, room_by_id, world};

/// The generic players' families.
pub const FAMILIES: [&str; 4] = ["rock", "station", "maniac", "grinder"];

/// The weighted pick: the first key whose weight takes `r` of the total
/// below zero, in order.
pub(crate) fn pick(weights: &[(String, f64)], r: f64) -> &str {
    let total: f64 = weights.iter().map(|(_, w)| w).sum();
    let mut x = r * total;
    for (k, w) in weights {
        x -= w;
        if x < 0.0 {
            return k;
        }
    }
    weights.last().map_or("", |(k, _)| k)
}

fn weights(pairs: &[(&str, f64)]) -> Weights {
    pairs.iter().map(|&(k, w)| (k.to_string(), w)).collect()
}

/// A family for a new player: `kind` `reg` or `tourist`, `wild` 0 to 1 (how
/// late and how much of a weekend it is), `r` a number in `0..1`.
pub fn family_for(kind: &str, wild: f64, r: f64) -> String {
    let [rock, station, maniac, grinder] = if kind == "reg" {
        [0.3, 0.15, 0.15, 0.4]
    } else {
        [0.15, 0.45, 0.33, 0.07]
    };
    let w = weights(&[
        ("rock", rock * (1.0 - wild / 2.0)),
        ("station", station * (1.0 + wild / 2.0)),
        ("maniac", maniac * (1.0 + wild)),
        ("grinder", grinder),
    ]);
    pick(&w, r).to_string()
}

/// The bot id that seats a person.
pub fn bot_id(family: &str, seed: u64) -> String {
    format!("g-{family}-{seed}")
}

/// A generic bot id's family and seed.
pub fn parse_bot_id(id: &str) -> Option<(&str, u64)> {
    let rest = id.strip_prefix("g-")?;
    let (family, seed) = rest.split_once('-')?;
    if !FAMILIES.contains(&family) || seed.is_empty() || !seed.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((family, seed.parse().ok()?))
}

// How much older players lean rock, and young ones maniac.
fn age_lean(age: &str, family: &str) -> f64 {
    match (age, family) {
        ("older", "rock") => 2.4,
        ("older", "maniac") => 0.6,
        ("young", "maniac") => 1.3,
        _ => 1.0,
    }
}

/// The face base a person with `family` and `seed` is drawn from.
pub fn base_for(family: &str, seed: u64) -> (usize, &'static Base) {
    let bases = &world().bases;
    let w: Weights = bases
        .iter()
        .enumerate()
        .map(|(i, b)| (i.to_string(), age_lean(&b.age, family)))
        .collect();
    let i: usize = pick(&w, roll!(seed; "face", "base")).parse().unwrap_or(0);
    (i, &bases[i])
}

/// A base's background, for who you meet where.
pub fn background_of(base: &Base) -> &'static str {
    if base.skin == "s5" || base.skin == "s6" {
        return "black";
    }
    match base.pool.as_str() {
        "english" | "irish" | "italian" | "polish" | "greek" => "white",
        "hispanic" => "hispanic",
        "indian" => "southasian",
        "arabic" | "persian" => "mideast",
        _ => "eastasian",
    }
}

const INITIALS: &str = "ABCDEFGHJKLMNPRSTVW";

fn one<T>(list: &[T], r: f64) -> Option<&T> {
    list.get((r * list.len() as f64).floor() as usize)
}

/// A first name from the base's pool, gender and age, and a last initial:
/// "Kenji R.".
pub fn name_for(base: &Base, seed: u64) -> String {
    let list = world()
        .names
        .get(&base.pool)
        .and_then(|g| g.get(&base.gender))
        .and_then(|a| a.get(&base.age))
        .map_or(&[][..], Vec::as_slice);
    let first = one(list, roll!(seed; "first")).map_or("", String::as_str);
    let initial = INITIALS.as_bytes()
        [(roll!(seed; "initial") * INITIALS.len() as f64).floor() as usize]
        as char;
    format!("{first} {initial}.")
}

/// A player's usual buy-in in big blinds: stations and rocks often short,
/// grinders around 100, maniacs deep.
pub fn usual_buy_in_bb(family: &str, seed: u64) -> f64 {
    let list: &[f64] = match family {
        "rock" => &[60.0, 100.0, 100.0],
        "station" => &[40.0, 60.0, 100.0],
        "maniac" => &[100.0, 150.0, 200.0],
        _ => &[100.0, 100.0, 150.0],
    };
    *one(list, roll!(seed; "buyin")).unwrap_or(&100.0)
}

/// What's known of a generic player from their bot id.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    /// Their bot id.
    pub id: String,
    /// Their family.
    pub family: String,
    /// Their seed.
    pub seed: u64,
    /// Their face base.
    pub base: usize,
    /// Their name.
    pub name: String,
    /// Their usual buy-in, in big blinds.
    pub buy_in_bb: f64,
}

/// A generic player, from their bot id; none for other ids.
pub fn person_of(id: &str) -> Option<Person> {
    let (family, seed) = parse_bot_id(id)?;
    let (i, base) = base_for(family, seed);
    Some(Person {
        id: id.to_string(),
        family: family.to_string(),
        seed,
        base: i,
        name: name_for(base, seed),
        buy_in_bb: usual_buy_in_bb(family, seed),
    })
}

/// A generic player's name, or none for other ids.
pub fn name_of(id: &str) -> Option<String> {
    let (family, seed) = parse_bot_id(id)?;
    Some(name_for(base_for(family, seed).1, seed))
}

/// A person for room `room_id`: their seed, from the dice on `seed` and
/// `keys`, chosen so they come from a background the room's region has, with
/// `family_of(seed)` their style. Only which id the room gets is chosen.
fn local_person(
    room_id: &str,
    family_of: &dyn Fn(u64) -> String,
    seed: u64,
    keys: &str,
) -> (u64, String) {
    let region = room_by_id(room_id)
        .and_then(region_of)
        .map(|r| r.id.as_str());
    let first = seed_from(roll_str(seed, keys));
    let w = world();
    let Some(mix) = region.and_then(|r| w.region_mix.get(r)) else {
        return (first, family_of(first));
    };
    let want = pick(mix, roll!(first; "background"));
    let pool = region
        .and_then(|r| w.pool_mix.get(r))
        .and_then(|p| p.get(want))
        .map(|pools| pick(pools, roll!(first; "pool")));
    for k in 0..120u32 {
        let s = if k == 0 {
            first
        } else {
            seed_from(roll_str(seed, &format!("{keys}|local|{k}")))
        };
        let family = family_of(s);
        let (_, base) = base_for(&family, s);
        if background_of(base) == want && pool.is_none_or(|p| base.pool == p) {
            return (s, family);
        }
    }
    (first, family_of(first))
}

/// Regs on each room's roster.
pub const ROSTER_SIZE: usize = 40;

/// A reg on a room's roster.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Reg {
    /// Their bot id.
    pub id: String,
    /// Their home room.
    pub home: String,
    /// The stakes they usually play: `low` (1/2, 2/5) or `mid` (5/10, 10/20).
    pub level: &'static str,
}

/// A room's regs, the same for a world's `seed`: a third of them play mid.
pub fn roster(seed: u64, room_id: &str) -> Vec<Reg> {
    type Rosters = HashMap<(u64, String), Vec<Reg>>;
    static CACHE: OnceLock<Mutex<Rosters>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let key = (seed, room_id.to_string());
    if let Some(r) = cache.lock().ok().and_then(|c| c.get(&key).cloned()) {
        return r;
    }
    let reg_family = |x: u64| family_for("reg", 0.0, roll!(x; "family"));
    let out: Vec<Reg> = (0..ROSTER_SIZE)
        .map(|i| {
            let (s, family) =
                local_person(room_id, &reg_family, seed, &format!("{room_id}|reg|{i}"));
            Reg {
                id: bot_id(&family, s),
                home: room_id.to_string(),
                level: if roll!(s; "level") < 1.0 / 3.0 {
                    "mid"
                } else {
                    "low"
                },
            }
        })
        .collect();
    if let Ok(mut c) = cache.lock() {
        if c.len() > 64 {
            c.clear();
        }
        c.insert(key, out.clone());
    }
    out
}

/// Whether `id` is a reg of any of `room_ids`' rosters in world `seed`.
pub fn is_reg(seed: u64, room_ids: &[String], id: &str) -> bool {
    room_ids
        .iter()
        .any(|r| roster(seed, r).iter().any(|p| p.id == id))
}

/// How wild the games are at game time `minutes`, 0 to 1: late on Friday and
/// Saturday nights most, weekday afternoons least.
pub fn wildness(minutes: f64) -> f64 {
    let (day, hour, _) = crate::world::day_hour(minutes);
    let weekend_night =
        ((day == 4 || day == 5) && hour >= 20) || ((day == 5 || day == 6) && hour < 4);
    if weekend_night {
        1.0
    } else if hour >= 22 || hour < 4 {
        0.6
    } else if hour >= 18 {
        0.4
    } else {
        0.1
    }
}

/// The share of seats at low stakes that tourists take at game time
/// `minutes`: most on a weekend evening, fewest on a weekday afternoon.
pub fn tourist_share(minutes: f64) -> f64 {
    let (day, hour, _) = crate::world::day_hour(minutes);
    let weekend = day >= 4 && !(day == 4 && hour < 17);
    let evening = hour >= 17 || hour < 3;
    if weekend && evening {
        0.7
    } else if !weekend && (9..17).contains(&hour) {
        0.3
    } else if evening {
        0.55
    } else {
        0.45
    }
}

/// A tourist: made for a room, a game day and a slot, and gone after that
/// day. `family` gives them that style, otherwise it's the tourists' mix.
pub fn tourist(
    seed: u64,
    room_id: &str,
    day: i64,
    slot: i64,
    minutes: f64,
    family: Option<&str>,
) -> String {
    let wild = wildness(minutes);
    let family_of = |x: u64| match family {
        Some(f) => f.to_string(),
        None => family_for("tourist", wild, roll!(x; "family")),
    };
    let (s, f) = local_person(
        room_id,
        &family_of,
        seed,
        &format!("{room_id}|tourist|{day}|{slot}"),
    );
    bot_id(&f, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bot_ids_round_trip() {
        assert_eq!(parse_bot_id("g-rock-123"), Some(("rock", 123)));
        assert_eq!(parse_bot_id("g-wizard-1"), None);
        assert_eq!(parse_bot_id("doug_poker"), None);
        let p = person_of("g-grinder-77").unwrap();
        assert_eq!(p.family, "grinder");
        assert!(p.name.ends_with('.'));
        assert_eq!(Some(p.name), name_of("g-grinder-77"));
    }

    #[test]
    fn rosters_are_the_same_for_a_seed() {
        let a = roster(7, "deuce");
        assert_eq!(a.len(), ROSTER_SIZE);
        assert_eq!(a, roster(7, "deuce"));
        assert_ne!(a, roster(8, "deuce"));
        assert!(a.iter().all(|r| parse_bot_id(&r.id).is_some()));
    }
}
