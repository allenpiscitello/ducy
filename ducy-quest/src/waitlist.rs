//! The waitlists: every game in every room has players waiting for a seat,
//! more when it's busy and at the small games, fewer at the big ones. The
//! same for a world's seed: a list changes every 10 game minutes, as people
//! are seated and others put their names down.

use serde::Serialize;

use crate::dice::seed_from;
use crate::people::{bot_id, family_for, name_of};
use crate::roll;
use crate::world::{Running, activity, day_hour, lobby};

// A list's names change this often, in game minutes.
const LIST_MINUTES: i64 = 10;
// The longest a list gets.
const MAX_WAITING: f64 = 5.0;

/// A game's list key in a room: `nlhe 1/2`, `plo4 2/5`.
pub fn list_key(game: &str, stakes: &str) -> String {
    format!("{game} {stakes}")
}

// Game minutes between seats opening at a single table, by stakes.
fn seat_gap_at(stakes: &str) -> f64 {
    match stakes {
        "1/2" => 1.0,
        "2/5" => 1.3,
        "5/10" => 2.5,
        "10/20" => 3.0,
        "25/50" => 3.5,
        _ => 4.0,
    }
}
// At your home club the floor gets you in almost straight away.
const HOME_SEAT_GAP: f64 = 0.15;

/// Game minutes between seats opening in a game at `stakes` spreading
/// `tables` tables (at your home club, `home`, almost none), faster with more
/// tables.
pub fn seat_gap(tables: f64, home: bool, stakes: &str) -> f64 {
    let gap = if home {
        HOME_SEAT_GAP
    } else {
        seat_gap_at(stakes)
    };
    gap.min((gap / 4.0).max(gap / tables.max(1.0)))
}

/// Who's waiting for a game in a room at game time `minutes`: their names.
pub fn waiting_at(seed: u64, room_id: &str, game: &str, stakes: &str, minutes: f64) -> Vec<String> {
    let m = minutes.floor() as i64;
    let (day, hour, _) = day_hour(minutes);
    let bucket = m.div_euclid(LIST_MINUTES);
    let small = stakes == "1/2" || stakes == "2/5";
    let most = if small { MAX_WAITING } else { 3.0 } * activity(day, hour);
    let key = list_key(game, stakes);
    let n = MAX_WAITING.min((roll!(seed; room_id, key, "waiting", bucket) * (most + 1.0)).floor())
        as usize;
    let mut names: Vec<String> = Vec::new();
    for i in 0..n {
        let s = seed_from(roll!(seed; room_id, key, "name", bucket, i));
        let id = bot_id(&family_for("tourist", 0.0, roll!(s; "family")), s);
        if let Some(name) = name_of(&id).filter(|n| !names.contains(n)) {
            names.push(name);
        }
    }
    names
}

/// A game running now, with its list.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Listed {
    /// The game.
    #[serde(flatten)]
    pub game: Running,
    /// Its list key.
    pub key: String,
    /// Who's waiting.
    pub waiting: Vec<String>,
    /// Game minutes between seats opening.
    pub gap: f64,
}

/// A room's games now, with their lists.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RoomLists {
    /// The room's id.
    pub room: String,
    /// Its games.
    pub games: Vec<Listed>,
}

/// Every game in every room now with its list. `home_room_id`, your home
/// club, is where seats come quickest.
pub fn lists_now(seed: u64, minutes: f64, home_room_id: Option<&str>) -> Vec<RoomLists> {
    lobby(seed, minutes)
        .into_iter()
        .map(|r| {
            let home = home_room_id == Some(r.room.as_str());
            let games = r
                .games
                .into_iter()
                .map(|g| Listed {
                    key: list_key(&g.game, &g.stakes),
                    waiting: waiting_at(seed, &r.room, &g.game, &g.stakes, minutes),
                    gap: seat_gap(g.tables as f64, home, &g.stakes),
                    game: g,
                })
                .collect();
            RoomLists {
                room: r.room,
                games,
            }
        })
        .collect()
}
