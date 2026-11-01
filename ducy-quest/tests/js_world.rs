//! The same seed gives the same world as ducy-web's JavaScript did
//! (ducy-web #466): `fixtures/js_world.json` is what the JS gave for a spread
//! of seeds and times (QUEST_FIXTURE names another file to check).

use ducy_quest::lineup::{lineup_for, lineup_stacks, room_lineups, seated_hours};
use ducy_quest::people::{person_of, roster, tourist};
use ducy_quest::waitlist::{seat_gap, waiting_at};
use ducy_quest::world::{cheapest_buy_in, lobby, room_by_id};
use serde_json::Value;

fn fixture() -> Value {
    let path = std::env::var("QUEST_FIXTURE").unwrap_or_else(|_| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/js_world.json").to_string()
    });
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}
fn u(v: &Value) -> u64 {
    v.as_u64().unwrap()
}
fn f(v: &Value) -> f64 {
    v.as_f64().unwrap()
}
fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| s(x).to_string())
        .collect()
}
fn each<'a>(fx: &'a Value, k: &str) -> &'a [Value] {
    fx[k].as_array().unwrap()
}

#[test]
fn the_dice_and_the_lobby() {
    let fx = fixture();
    for r in each(&fx, "rolls") {
        assert_eq!(
            ducy_quest::dice::roll_str(u(&r["seed"]), s(&r["keys"])),
            f(&r["r"]),
            "{r}"
        );
    }
    assert_eq!(cheapest_buy_in(40.0), f(&fx["cheapest"]));
    for l in each(&fx, "lobby") {
        let got: Vec<Value> = lobby(u(&l["seed"]), f(&l["minutes"]))
            .iter()
            .map(|r| {
                serde_json::json!([
                    r.room,
                    r.games
                        .iter()
                        .map(|g| serde_json::json!([g.game, g.stakes, g.tables, g.ids]))
                        .collect::<Vec<_>>()
                ])
            })
            .collect();
        assert_eq!(
            Value::Array(got),
            l["rooms"],
            "seed {} minutes {}",
            l["seed"],
            l["minutes"]
        );
    }
}

#[test]
fn the_people() {
    let fx = fixture();
    for p in each(&fx, "people") {
        let got = person_of(s(&p["id"])).unwrap();
        assert_eq!(got.name, s(&p["name"]), "{p}");
        assert_eq!(got.buy_in_bb, f(&p["buyInBb"]), "{p}");
        assert_eq!(got.base as u64, u(&p["base"]), "{p}");
    }
    for r in each(&fx, "rosters") {
        let got = serde_json::to_value(roster(u(&r["seed"]), s(&r["room"]))).unwrap();
        assert_eq!(got, r["regs"], "seed {} room {}", r["seed"], r["room"]);
    }
    for t in each(&fx, "tourists") {
        let got = tourist(
            u(&t["seed"]),
            s(&t["room"]),
            t["day"].as_i64().unwrap(),
            t["slot"].as_i64().unwrap(),
            f(&t["minutes"]),
            None,
        );
        assert_eq!(got, s(&t["id"]), "{t}");
    }
}

#[test]
fn the_waitlists() {
    let fx = fixture();
    for w in each(&fx, "waiting") {
        let got = waiting_at(
            u(&w["seed"]),
            s(&w["room"]),
            s(&w["game"]),
            s(&w["stakes"]),
            f(&w["minutes"]),
        );
        assert_eq!(got, strings(&w["names"]), "{w}");
    }
    for g in each(&fx, "gaps") {
        assert_eq!(
            seat_gap(
                f(&g["tables"]),
                g["home"].as_bool().unwrap(),
                s(&g["stakes"])
            ),
            f(&g["gap"]),
            "{g}"
        );
    }
}

#[test]
fn the_lineups_and_stacks() {
    let fx = fixture();
    for l in each(&fx, "lineups") {
        let got: Vec<Value> = room_lineups(u(&l["seed"]), s(&l["room"]), f(&l["minutes"]), None)
            .into_iter()
            .map(|(id, t)| serde_json::json!([id, t.seats, t.bench]))
            .collect();
        assert_eq!(
            Value::Array(got),
            l["tables"],
            "seed {} room {} minutes {}",
            l["seed"],
            l["room"],
            l["minutes"]
        );
    }
    for r in each(&fx, "regulars") {
        let got = lineup_for(
            u(&r["seed"]),
            s(&r["table"]),
            s(&r["stakes"]),
            f(&r["minutes"]),
            None,
            false,
        );
        assert_eq!(got, strings(&r["ids"]), "{r}");
    }
    for st in each(&fx, "stacks") {
        let blinds = [
            st["blinds"][0].as_i64().unwrap(),
            st["blinds"][1].as_i64().unwrap(),
        ];
        let got = lineup_stacks(
            u(&st["seed"]),
            s(&st["table"]),
            &strings(&st["ids"]),
            blinds,
            room_by_id(s(&st["room"])),
            f(&st["minutes"]),
        );
        let want: Vec<(f64, f64)> = st["stacks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| (f(&x["stack"]), f(&x["buyIn"])))
            .collect();
        assert_eq!(
            got.iter().map(|x| (x.stack, x.buy_in)).collect::<Vec<_>>(),
            want,
            "{st}"
        );
    }
    for h in each(&fx, "hours") {
        assert_eq!(seated_hours(f(&h["u"]), f(&h["v"])), f(&h["h"]), "{h}");
    }
}
