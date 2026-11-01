//! Each game's waitlist (ducy-web #518). A game is its tables' type, stakes
//! and seats ([`Table::game_key`]); players join and leave its list
//! themselves, and seats are given from it in order, never by a manager.
//!
//! - **An offer:** when a seat is free at one of the game's tables, the next
//!   player waiting is offered it for [`OFFER_MS`], and the seat is held for
//!   them (a reservation, by [`LIST_BY`]). Sitting down there takes them off
//!   the list.
//! - **A missed offer:** they stay on the list, passed over, and the seat
//!   goes to the next. Back within [`AWAY_MS`], they take their place in line
//!   again; after that they're off the list.
//!
//! The caller runs the tables: it says which seats are taken, carries out the
//! offers (sending them, and timing [`Club::next_list_due`]), and tells the
//! club when someone sits down.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::rules::{Club, Reservation, RuleError, Status, Table, TableStatus, refuse};

/// How long an offered seat is held.
pub const OFFER_MS: u64 = 60_000;
/// How long someone who missed their offer keeps their place.
pub const AWAY_MS: u64 = 10 * 60_000;
/// Who a seat held for an offer is by.
pub const LIST_BY: &str = "list";

/// Someone on a game's list.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OnList {
    /// The member.
    pub member: String,
    /// When they joined it.
    pub at: u64,
    /// The seat offered them, while it's held.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offer: Option<Offer>,
    /// When they missed an offer: passed over until they're back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missed: Option<u64>,
    /// For a game that isn't running: the fewest players they'd start a new
    /// table with (none: a full table).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<u32>,
}

impl OnList {
    /// Whether they're next in line for a seat: not offered one, not
    /// passed over.
    pub fn waiting(&self) -> bool {
        self.offer.is_none() && self.missed.is_none()
    }
}

/// A seat offered.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Offer {
    /// The table.
    pub table: String,
    /// When it goes to the next.
    pub until: u64,
}

impl Club {
    /// The open tables dealing game `key`, in the order seats are given:
    /// a must-move game's from the end of its chain, then the rest.
    pub fn game_tables(&self, key: &str) -> Vec<&Table> {
        let mut ts: Vec<&Table> = self
            .tables
            .iter()
            .filter(|t| t.status == TableStatus::Open && t.game_key() == key)
            .collect();
        let place = |t: &Table| self.group_of(&t.id).map_or(0, |(g, i)| g.tables.len() - i);
        ts.sort_by_key(|t| (place(t), t.created_at));
        ts
    }

    /// Game `key`'s list, oldest first.
    pub fn list(&self, key: &str) -> &[OnList] {
        self.lists.get(key).map_or(&[], Vec::as_slice)
    }

    /// Member `id`'s place on game `key`'s list, from 1, counting those
    /// ahead of them who aren't passed over.
    pub fn place_on_list(&self, id: &str, key: &str) -> Option<usize> {
        let list = self.list(key);
        let i = list.iter().position(|e| e.member == id)?;
        Some(list[..i].iter().filter(|e| e.missed.is_none()).count() + 1)
    }

    /// Member `id` joins game `key`'s list (back in line, if they were passed
    /// over); their place.
    pub fn join_list(&mut self, id: &str, key: &str, now: u64) -> Result<usize, RuleError> {
        self.join_list_with(id, key, None, now)
    }

    /// As [`Club::join_list`]; for an offered game that isn't running (its
    /// interest list), `min` is the fewest players they'd start a table with
    /// (none: a full one).
    pub fn join_list_with(
        &mut self,
        id: &str,
        key: &str,
        min: Option<u32>,
        now: u64,
    ) -> Result<usize, RuleError> {
        if !self
            .members
            .get(id)
            .is_some_and(|m| m.status == Status::Member)
        {
            return refuse("Only club members can join a list.");
        }
        let seats = match (self.game_tables(key).first(), self.offered(key)) {
            (Some(t), _) => t.seats,
            (None, Some(g)) => g.seats,
            (None, None) => return refuse("That game isn’t running."),
        };
        if min.is_some_and(|m| !(2..=seats).contains(&m)) {
            return refuse(format!("Pick 2 to {seats} players."));
        }
        let at = self
            .members
            .get(id)
            .and_then(|m| m.seat_table())
            .and_then(|t| self.table(t));
        if at.is_some_and(|t| t.game_key() == key) {
            return refuse("You’re playing it already.");
        }
        let list = self.lists.entry(key.into()).or_default();
        match list.iter_mut().find(|e| e.member == id) {
            Some(e) => {
                e.missed = None;
                if min.is_some() {
                    e.min = min;
                }
            }
            None => list.push(OnList {
                member: id.into(),
                at: now,
                offer: None,
                missed: None,
                min,
            }),
        }
        Ok(self.place_on_list(id, key).unwrap_or(1))
    }

    /// Member `id` leaves game `key`'s list (or a manager takes them off);
    /// a seat held for them goes. Whether they were on it.
    pub fn leave_list(&mut self, id: &str, key: &str) -> bool {
        let Some(list) = self.lists.get_mut(key) else {
            return false;
        };
        let Some(i) = list.iter().position(|e| e.member == id) else {
            return false;
        };
        let e = list.remove(i);
        if list.is_empty() {
            self.lists.remove(key);
        }
        if let Some(o) = e.offer {
            self.drop_offer_hold(&o.table, id);
        }
        true
    }

    /// Member `id`, passed over on game `key`'s list, is back within
    /// [`AWAY_MS`]: their place in line again.
    pub fn back_on_list(&mut self, id: &str, key: &str, now: u64) -> Result<usize, RuleError> {
        let e = self
            .lists
            .get_mut(key)
            .and_then(|l| l.iter_mut().find(|e| e.member == id));
        match e {
            Some(e) if e.missed.is_some_and(|m| now < m + AWAY_MS) => e.missed = None,
            Some(e) if e.missed.is_none() => {}
            _ => return refuse("You’re not on that list."),
        }
        Ok(self.place_on_list(id, key).unwrap_or(1))
    }

    /// Member `id` sat down at table `table`: off its game's list.
    pub fn sat_down(&mut self, id: &str, table: &str) -> bool {
        let Some(key) = self.table(table).map(Table::game_key) else {
            return false;
        };
        let offered = self
            .list(&key)
            .iter()
            .any(|e| e.member == id && e.offer.is_some());
        let on = self.leave_list(id, &key);
        // The seat held for them is theirs now; any other they were offered
        // is someone else's.
        if offered {
            self.drop_offer_hold(table, id);
        }
        on
    }

    fn drop_offer_hold(&mut self, table: &str, id: &str) {
        let Some(list) = self.waitlist.get_mut(table) else {
            return;
        };
        list.retain(|r| !(r.member == id && r.by.as_deref() == Some(LIST_BY)));
        if list.is_empty() {
            self.waitlist.remove(table);
        }
    }

    /// Offers the free seats at each game's running tables (`taken`: seats
    /// taken at each, people there and seats held) to the next on its list,
    /// one offer per player at a time. What to send: `{to, body}`.
    pub fn offer_seats(&mut self, taken: &BTreeMap<String, usize>, now: u64) -> Vec<Value> {
        let mut out = vec![];
        let mut taken = taken.clone();
        let keys: Vec<String> = self.lists.keys().cloned().collect();
        for key in keys {
            let tables: Vec<(String, String, usize)> = self
                .game_tables(&key)
                .into_iter()
                .filter(|t| taken.contains_key(&t.id))
                .map(|t| (t.id.clone(), t.name.clone(), t.seats as usize))
                .collect();
            for (tid, name, seats) in tables {
                loop {
                    let n = taken.get(&tid).copied().unwrap_or(0);
                    if n >= seats {
                        break;
                    }
                    let Some(id) = self.next_waiting(&key) else {
                        break;
                    };
                    let until = now + OFFER_MS;
                    if let Some(e) = self
                        .lists
                        .get_mut(&key)
                        .and_then(|l| l.iter_mut().find(|e| e.member == id))
                    {
                        e.offer = Some(Offer {
                            table: tid.clone(),
                            until,
                        });
                    }
                    self.waitlist
                        .entry(tid.clone())
                        .or_default()
                        .push(Reservation {
                            member: id.clone(),
                            at: now,
                            until,
                            from: None,
                            by: Some(LIST_BY.into()),
                        });
                    taken.insert(tid.clone(), n + 1);
                    out.push(json!({"to": id, "body": {"type": "list", "club": self.code, "kind": "offer",
                        "key": key, "table": tid, "name": name, "until": until}}));
                }
            }
        }
        out
    }

    /// The next on game `key`'s list to offer a seat: waiting, still a
    /// member, and not offered one at another game.
    fn next_waiting(&self, key: &str) -> Option<String> {
        let offered = |id: &str| {
            self.lists
                .values()
                .flatten()
                .any(|e| e.member == id && e.offer.is_some())
        };
        self.list(key)
            .iter()
            .filter(|e| e.waiting() && !offered(&e.member))
            .find(|e| {
                self.members
                    .get(&e.member)
                    .is_some_and(|m| m.status == Status::Member)
            })
            .map(|e| e.member.clone())
    }

    /// Offers not taken in time pass their player over, and those passed
    /// over too long leave the list; lists of games no longer running go.
    /// What to send: `{to, body}`.
    pub fn lapse_lists(&mut self, now: u64) -> Vec<Value> {
        let mut out = vec![];
        let code = self.code.clone();
        let keys: Vec<String> = self.lists.keys().cloned().collect();
        for key in keys {
            // An offered game's list stays while it isn't running: it's its
            // interest list.
            let running = !self.game_tables(&key).is_empty() || self.offered(&key).is_some();
            let list = self.lists.remove(&key).unwrap_or_default();
            let tell = |to: &str, kind: &str, until: Option<u64>| {
                let mut b = json!({"type": "list", "club": code, "kind": kind, "key": key});
                if let Some(u) = until {
                    b["until"] = u.into();
                }
                json!({"to": to, "body": b})
            };
            let mut keep = vec![];
            for mut e in list {
                if !running {
                    out.push(tell(&e.member, "off", None));
                    if let Some(o) = &e.offer {
                        self.drop_offer_hold(&o.table, &e.member);
                    }
                    continue;
                }
                if let Some(o) = e.offer.clone().filter(|o| o.until <= now) {
                    self.drop_offer_hold(&o.table, &e.member);
                    e.offer = None;
                    e.missed = Some(now);
                    out.push(tell(&e.member, "missed", Some(now + AWAY_MS)));
                } else if e.missed.is_some_and(|m| m + AWAY_MS <= now) {
                    out.push(tell(&e.member, "off", None));
                    continue;
                }
                keep.push(e);
            }
            if !keep.is_empty() {
                self.lists.insert(key, keep);
            }
        }
        out
    }

    /// When an offer next lapses, or someone passed over leaves a list.
    pub fn next_list_due(&self) -> Option<u64> {
        self.lists
            .values()
            .flatten()
            .filter_map(|e| {
                e.offer
                    .as_ref()
                    .map(|o| o.until)
                    .or(e.missed.map(|m| m + AWAY_MS))
            })
            .min()
    }

    /// How many are on each game's list, for everyone.
    pub fn list_counts(&self) -> BTreeMap<String, usize> {
        self.lists
            .iter()
            .map(|(k, l)| (k.clone(), l.len()))
            .collect()
    }

    /// What member `id` is told of the lists they're on: each with its
    /// count and their place, an offer, or when their place goes.
    pub fn lists_for(&self, id: &str) -> Value {
        let mine: Vec<Value> = self
            .lists
            .iter()
            .filter_map(|(key, list)| {
                let e = list.iter().find(|e| e.member == id)?;
                let mut v = json!({"key": key, "count": list.len()});
                if let Some(m) = e.min {
                    v["min"] = m.into();
                }
                if let Some(o) = &e.offer {
                    let name = self.table(&o.table).map_or("", |t| t.name.as_str());
                    v["offer"] = json!({"table": o.table, "name": name, "until": o.until});
                } else if let Some(m) = e.missed {
                    v["missed"] = (m + AWAY_MS).into();
                } else {
                    v["place"] = self.place_on_list(id, key).unwrap_or(1).into();
                }
                Some(v)
            })
            .collect();
        json!({"type": "lists", "club": self.code, "lists": mine})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A club with members `ids` and two open 6-seat Hold'em 1/2 tables, t0
    // and t1, and a 2/5 one, t2.
    fn club(ids: &[&str]) -> Club {
        let mut members = serde_json::Map::new();
        for id in ids {
            members.insert(
                (*id).into(),
                json!({"id": id, "name": id.to_uppercase(), "status": "member"}),
            );
        }
        let t = |id: &str, sb: u64, bb: u64, at: u64| {
            json!({"id": id, "name": id.to_uppercase(), "game": "nlhe", "sb": sb, "bb": bb, "minBB": 20, "maxBB": 100,
                   "seats": 6, "status": "open", "createdAt": at})
        };
        serde_json::from_value(json!({
            "code": "ABCD-EFGH", "hostId": "HOST", "members": members,
            "tables": [t("t0", 1, 2, 1), t("t1", 1, 2, 2), t("t2", 2, 5, 3)],
        }))
        .unwrap()
    }

    const K: &str = "nlhe:1/2:6";

    fn taken(v: &[(&str, usize)]) -> BTreeMap<String, usize> {
        v.iter().map(|(t, n)| ((*t).to_string(), *n)).collect()
    }

    #[test]
    fn joining_and_leaving() {
        let mut c = club(&["a", "b", "c"]);
        assert_eq!(c.tables[0].game_key(), K);
        assert_eq!(c.join_list("a", K, 1).unwrap(), 1);
        assert_eq!(c.join_list("b", K, 2).unwrap(), 2);
        // Again: the same place.
        assert_eq!(c.join_list("a", K, 3).unwrap(), 1);
        assert!(
            c.join_list("z", K, 3)
                .unwrap_err()
                .0
                .contains("Only club members")
        );
        assert!(
            c.join_list("a", "plo:1/2:6", 3)
                .unwrap_err()
                .0
                .contains("isn’t running")
        );
        // Seated at the game already.
        c.members
            .get_mut("c")
            .unwrap()
            .other
            .insert("seat".into(), json!({"table": "t1"}));
        assert!(c.join_list("c", K, 3).unwrap_err().0.contains("already"));
        // At another game: fine.
        c.members
            .get_mut("c")
            .unwrap()
            .other
            .insert("seat".into(), json!({"table": "t2"}));
        assert_eq!(c.join_list("c", K, 3).unwrap(), 3);
        assert!(c.leave_list("a", K));
        assert!(!c.leave_list("a", K));
        assert_eq!(c.place_on_list("b", K), Some(1));
        assert_eq!(c.list_counts()[K], 2);
        let mine = c.lists_for("c");
        assert_eq!(mine["lists"], json!([{"key": K, "count": 2, "place": 2}]));
    }

    #[test]
    fn a_free_seat_is_offered_to_the_next_in_line() {
        let mut c = club(&["a", "b", "c"]);
        for (i, m) in ["a", "b", "c"].iter().enumerate() {
            c.join_list(m, K, i as u64).unwrap();
        }
        // t0 full, t1 with one seat free; t2 isn't the game.
        let sends = c.offer_seats(&taken(&[("t0", 6), ("t1", 5), ("t2", 0)]), 100);
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0]["to"], "a");
        assert_eq!(
            sends[0]["body"],
            json!({"type": "list", "club": "ABCD-EFGH", "kind": "offer", "key": K, "table": "t1", "name": "T1", "until": 100 + OFFER_MS})
        );
        // The seat is held for her.
        assert_eq!(c.held_seats("t1", 100, ""), 1);
        assert_eq!(
            c.lists_for("a")["lists"][0]["offer"],
            json!({"table": "t1", "name": "T1", "until": 100 + OFFER_MS})
        );
        // Counted as taken now: no second offer.
        assert!(
            c.offer_seats(&taken(&[("t0", 6), ("t1", 6)]), 101)
                .is_empty()
        );
        // b is next, once a seat is free: a's offer is still out.
        let sends = c.offer_seats(&taken(&[("t0", 5), ("t1", 6)]), 102);
        assert_eq!(sends[0]["to"], "b");
        assert_eq!(c.next_list_due(), Some(100 + OFFER_MS));
        // She sits down: off the list, her seat hers.
        assert!(c.sat_down("a", "t1"));
        assert_eq!(c.held_seats("t1", 103, ""), 0);
        // b, offered a seat, is still ahead of c.
        assert_eq!(c.place_on_list("c", K), Some(2));
    }

    #[test]
    fn a_missed_offer_passes_them_over_for_a_while() {
        let mut c = club(&["a", "b"]);
        c.join_list("a", K, 0).unwrap();
        c.join_list("b", K, 1).unwrap();
        c.offer_seats(&taken(&[("t0", 5)]), 100);
        // Not yet.
        assert!(c.lapse_lists(100 + OFFER_MS - 1).is_empty());
        let t = 100 + OFFER_MS;
        let sends = c.lapse_lists(t);
        assert_eq!(
            sends,
            vec![
                json!({"to": "a", "body": {"type": "list", "club": "ABCD-EFGH", "kind": "missed", "key": K, "until": t + AWAY_MS}})
            ]
        );
        assert_eq!(c.held_seats("t0", t, ""), 0);
        // Still on the list, passed over: the seat goes to b.
        assert_eq!(c.lists_for("a")["lists"][0]["missed"], t + AWAY_MS);
        assert_eq!(c.place_on_list("b", K), Some(1));
        let sends = c.offer_seats(&taken(&[("t0", 5)]), t);
        assert_eq!(sends[0]["to"], "b");
        // Back in time: her place in line again, ahead of b.
        assert_eq!(c.back_on_list("a", K, t + 5).unwrap(), 1);
        // Missed again, and gone too long: off the list.
        c.leave_list("b", K);
        c.offer_seats(&taken(&[("t0", 5)]), t + 10);
        c.lapse_lists(t + 10 + OFFER_MS);
        assert_eq!(c.next_list_due(), Some(t + 10 + OFFER_MS + AWAY_MS));
        assert!(c.back_on_list("a", K, t + 10 + OFFER_MS + AWAY_MS).is_err());
        let sends = c.lapse_lists(t + 10 + OFFER_MS + AWAY_MS);
        assert_eq!(sends[0]["body"]["kind"], "off");
        assert!(c.lists.is_empty());
    }

    #[test]
    fn one_offer_at_a_time_and_lists_of_games_gone() {
        let mut c = club(&["a"]);
        c.join_list("a", K, 0).unwrap();
        c.join_list("a", "nlhe:2/5:6", 0).unwrap();
        let sends = c.offer_seats(&taken(&[("t0", 5), ("t2", 5)]), 10);
        assert_eq!(sends.len(), 1);
        // The 1/2 tables close: that list goes, her offer with it.
        c.tables[0].status = TableStatus::Closed;
        c.tables[1].status = TableStatus::Closed;
        let sends = c.lapse_lists(11);
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0]["body"]["kind"], "off");
        assert!(c.waitlist.is_empty());
        assert_eq!(c.list(K).len(), 0);
        assert_eq!(c.list("nlhe:2/5:6").len(), 1);
    }
}
