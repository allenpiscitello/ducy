//! Grouped tables with must-move (ducy-web #501): tables of one type chained
//! in order, the main table first, then must-move 1, 2, 3.
//!
//! - **Moving up:** when a seat opens at a table, the player longest in the
//!   group at the table below moves up to it, after their hand. Moves are
//!   forced. The time in the group counts from when they first sat at any of
//!   its tables, and carries across its tables.
//! - **A new table:** when the group's tables are full, everyone online in
//!   the club but those at its tables is asked "We're starting a new table.
//!   Join?" for [`ASK_MS`]. It opens once the group's minimum have said yes;
//!   those seated elsewhere are moved to it after their hand. When time runs
//!   out first, it doesn't open, and those who said yes are told.
//! - A must-move table that drops below 2 players stays open and waits.
//!
//! The caller runs the tables: it carries out the moves (as a manager's
//! move, ducy-web's reserve), sends the questions and the answers, and
//! times the question.

use std::collections::BTreeMap;

use rand::rngs::StdRng;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::club_rules::{Club, RuleError, Status, TableStatus, random36, refuse};

/// The most tables in a group: the main table and 3 must-move.
pub const MAX_GROUP_TABLES: usize = 4;
/// A new table's minimum when the manager doesn't set one.
pub const DEFAULT_GROUP_MIN: u32 = 4;
/// How long the "Join?" question stays open.
pub const ASK_MS: u64 = 60_000;
/// Who a move up is by, for the caller's moves (a manager's move, but no
/// one's): the hit-and-run rule doesn't hold it.
pub const GROUP_BY: &str = "group";

/// A group of tables.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Group {
    /// Its id.
    pub id: String,
    /// Its tables, in order: the main table first.
    pub tables: Vec<String>,
    /// How many must say yes before a new table opens.
    pub min: u32,
    /// When each player first sat at one of its tables, by member id.
    pub joined: BTreeMap<String, u64>,
    /// The "Join?" question, while it's open.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ask: Option<Ask>,
}

/// The question "We're starting a new table. Join?".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Ask {
    /// When it was asked.
    pub at: u64,
    /// When it closes.
    pub until: u64,
    /// Who was asked.
    pub asked: Vec<String>,
    /// Who said yes, in order.
    pub yes: Vec<String>,
}

/// A player to move up: from table `from` to the seat at `to`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MoveUp {
    /// The member.
    pub id: String,
    /// The table they're at.
    pub from: String,
    /// The table above.
    pub to: String,
}

/// How a question ended.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AskEnd {
    /// The table that opened, if enough said yes.
    pub opened: Option<String>,
    /// Who said yes.
    pub yes: Vec<String>,
    /// Those of them seated at another table, to move to it: (member, table).
    pub moves: Vec<MoveUp>,
}

/// What the caller tells members about a group's question, and does.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupNews {
    /// Messages to send now: `{to, kind, …}` (kind "ask", "closed" or "open").
    pub sends: Vec<Value>,
    /// The table that opened: start it, then send `after_open`.
    pub opened: Option<String>,
    /// Messages to send once the new table runs.
    pub after_open: Vec<Value>,
    /// Those seated elsewhere to move to the new table, by [`GROUP_BY`].
    pub moves: Vec<MoveUp>,
    /// When the question closes: time it.
    pub until: Option<u64>,
    /// Enough said yes: close the question now.
    pub finish: bool,
}

impl Club {
    /// Asks about a new table for group `id` (see [`Club::ask_group`]): the
    /// question for each member asked. Refused, with why, if too few are
    /// online to ask.
    pub fn ask_about(
        &mut self,
        id: &str,
        online: &[String],
        now: u64,
    ) -> Result<GroupNews, RuleError> {
        let min = self.group(id).map_or(0, |g| g.min);
        let asked = self.ask_group(id, online, now)?;
        if asked.is_empty() {
            return refuse(format!("Fewer than {min} players are online to ask."));
        }
        let g = self.group(id).expect("asked");
        let until = g.ask.as_ref().map(|a| a.until);
        let name = g
            .tables
            .first()
            .and_then(|t| self.table(t))
            .map_or(String::new(), |t| t.name.clone());
        Ok(GroupNews {
            sends: asked
                .iter()
                .map(|m| json!({"to": m, "kind": "ask", "name": name, "until": until}))
                .collect(),
            until,
            ..GroupNews::default()
        })
    }

    /// Member `who` answers group `id`'s question: `finish` once enough said
    /// yes; a closed question or someone not asked is told it's closed.
    pub fn group_answer(&mut self, id: &str, who: &str, yes: bool, now: u64) -> GroupNews {
        match self.answer_group(id, who, yes, now) {
            Ok(finish) => GroupNews {
                finish,
                ..GroupNews::default()
            },
            Err(e) => GroupNews {
                sends: vec![json!({"to": who, "kind": "closed", "reason": e.0})],
                ..GroupNews::default()
            },
        }
    }

    /// Closes group `id`'s question (see [`Club::end_ask`]). Everyone asked
    /// hears how it ended: with no table, those who said yes hear why; with
    /// one, those who said no hear it's closed, and those who said yes and
    /// aren't moving are told to sit down there once it runs.
    pub fn finish_ask(
        &mut self,
        id: &str,
        now: u64,
        most: Option<u64>,
        rng: &mut StdRng,
    ) -> GroupNews {
        let asked = self
            .group(id)
            .and_then(|g| g.ask.as_ref())
            .map(|a| a.asked.clone())
            .unwrap_or_default();
        let end = self.end_ask(id, now, most, rng).unwrap_or_default();
        let mut out = GroupNews::default();
        let opened = end
            .opened
            .as_deref()
            .and_then(|t| self.table(t))
            .map(|t| (t.id.clone(), t.name.clone()));
        for m in &asked {
            let said_yes = end.yes.contains(m);
            match (&opened, said_yes) {
                (None, true) => out.sends.push(
                    json!({"to": m, "kind": "closed", "reason": "Not enough players for a new table."}),
                ),
                (None, false) | (Some(_), false) => {
                    out.sends.push(json!({"to": m, "kind": "closed"}))
                }
                (Some(_), true) => {}
            }
        }
        if let Some((tid, name)) = opened {
            for m in &end.yes {
                if !end.moves.iter().any(|x| &x.id == m) {
                    out.after_open
                        .push(json!({"to": m, "kind": "open", "table": tid, "name": name}));
                }
            }
            out.opened = Some(tid);
            out.moves = end.moves;
        }
        out
    }

    /// The group with id `id`.
    pub fn group(&self, id: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    fn group_mut(&mut self, id: &str) -> Result<&mut Group, RuleError> {
        match self.groups.iter_mut().find(|g| g.id == id) {
            Some(g) => Ok(g),
            None => refuse("No such group."),
        }
    }

    /// The group table `table` is in, and its place in it (0: the main table).
    pub fn group_of(&self, table: &str) -> Option<(&Group, usize)> {
        self.groups
            .iter()
            .find_map(|g| g.tables.iter().position(|t| t == table).map(|i| (g, i)))
    }

    /// Makes a group with `main` as its main table; `min` (2 up to its
    /// seats; [`DEFAULT_GROUP_MIN`] when not given, or its seats if fewer)
    /// must say yes before a new table opens.
    pub fn make_group(
        &mut self,
        main: &str,
        min: Option<u32>,
        now: u64,
        rng: &mut StdRng,
    ) -> Result<&Group, RuleError> {
        let seats = match self.table(main) {
            Some(t) if t.status == TableStatus::Open => t.seats,
            _ => return refuse("That table isn’t open."),
        };
        if self.group_of(main).is_some() {
            return refuse("That table is in a group already.");
        }
        let min = min.unwrap_or(DEFAULT_GROUP_MIN.min(seats));
        if !(2..=seats).contains(&min) {
            return refuse(format!("A new table needs 2 to {seats} players to start."));
        }
        // Whoever is at the main table now is in the group from now.
        let joined = self
            .members
            .values()
            .filter(|m| m.seat_table() == Some(main))
            .map(|m| (m.id.clone(), now))
            .collect();
        self.groups.push(Group {
            id: random36(rng, 6),
            tables: vec![main.into()],
            min,
            joined,
            ask: None,
        });
        Ok(self.groups.last().expect("just added"))
    }

    /// Sets group `id`'s minimum to start a new table.
    pub fn set_group_min(&mut self, id: &str, min: u32) -> Result<(), RuleError> {
        let main = self.group(id).and_then(|g| g.tables.first()).cloned();
        let seats = main.and_then(|t| self.table(&t)).map_or(0, |t| t.seats);
        if !(2..=seats).contains(&min) {
            return refuse(format!("A new table needs 2 to {seats} players to start."));
        }
        self.group_mut(id)?.min = min;
        Ok(())
    }

    /// Ends group `id`: its tables go on as single tables.
    pub fn end_group(&mut self, id: &str) -> Result<(), RuleError> {
        self.group_mut(id)?;
        self.groups.retain(|g| g.id != id);
        Ok(())
    }

    /// Member `id` sat at table `table` at `now`: in its group from now,
    /// unless they're in it already.
    pub fn sat_in_group(&mut self, id: &str, table: &str, now: u64) {
        let Some(gid) = self.group_of(table).map(|(g, _)| g.id.clone()) else {
            return;
        };
        if let Ok(g) = self.group_mut(&gid) {
            g.joined.entry(id.into()).or_insert(now);
        }
    }

    /// Member `id` left table `table`, for table `to` if they're moving:
    /// out of its group unless `to` is in it too.
    pub fn left_group(&mut self, id: &str, table: &str, to: Option<&str>) {
        let Some((g, _)) = self.group_of(table) else {
            return;
        };
        if to.is_some_and(|to| g.tables.iter().any(|t| t == to)) {
            return;
        }
        let gid = g.id.clone();
        if let Ok(g) = self.group_mut(&gid) {
            g.joined.remove(id);
        }
    }

    /// Tables closed drop out of their groups; a group whose main table
    /// closed ends. The must-move below a closed one move up a place.
    pub fn prune_groups(&mut self) {
        let open: Vec<String> = self.open_tables().map(|t| t.id.clone()).collect();
        for g in &mut self.groups {
            if g.tables.first().is_some_and(|m| !open.contains(m)) {
                g.tables.clear();
            }
            g.tables.retain(|t| open.contains(t));
        }
        self.groups.retain(|g| !g.tables.is_empty());
    }

    /// Who moves up in group `id` now: at each table with a seat free
    /// (`seated`: who's at each of its tables; `held`: seats held at each
    /// for someone on their way), the player longest in the group at the
    /// table below who isn't moving already, one per free seat.
    pub fn moves_up(
        &self,
        id: &str,
        seated: &BTreeMap<String, Vec<String>>,
        held: &BTreeMap<String, usize>,
    ) -> Vec<MoveUp> {
        let Some(g) = self.group(id) else {
            return vec![];
        };
        let mut out = vec![];
        for pair in g.tables.windows(2) {
            let (upper, lower) = (&pair[0], &pair[1]);
            let Some(t) = self.table(upper) else { continue };
            let at = |tid: &str| seated.get(tid).map_or(0, Vec::len);
            let mut free = (t.seats as usize)
                .saturating_sub(at(upper) + held.get(upper.as_str()).copied().unwrap_or(0));
            // Those with chips at the table below (a seat in the ledger),
            // not on their way somewhere already.
            let mut below: Vec<&String> = seated
                .get(lower.as_str())
                .map(|v| {
                    v.iter()
                        .filter(|m| self.seated_at(m, lower) && !self.moving(m))
                        .collect()
                })
                .unwrap_or_default();
            below.sort_by_key(|m| {
                (
                    g.joined.get(m.as_str()).copied().unwrap_or(u64::MAX),
                    (*m).clone(),
                )
            });
            for m in below {
                if free == 0 {
                    break;
                }
                free -= 1;
                out.push(MoveUp {
                    id: m.clone(),
                    from: lower.clone(),
                    to: upper.clone(),
                });
            }
        }
        out
    }

    fn seated_at(&self, id: &str, table: &str) -> bool {
        self.members.get(id).and_then(|m| m.seat_table()) == Some(table)
    }

    /// Whether member `id` is on their way to a seat held at another table.
    fn moving(&self, id: &str) -> bool {
        self.members
            .get(id)
            .and_then(|m| m.seat())
            .is_some_and(|s| s.get("moving").is_some_and(|v| !v.is_null()))
    }

    /// Asks everyone in `online` but those at group `id`'s tables whether
    /// they'd join a new table, when there's room for one in the group and
    /// enough to ask. Returns who's asked (none: not asked).
    pub fn ask_group(
        &mut self,
        id: &str,
        online: &[String],
        now: u64,
    ) -> Result<Vec<String>, RuleError> {
        let g = self
            .group(id)
            .ok_or_else(|| RuleError("No such group.".into()))?;
        if g.tables.len() >= MAX_GROUP_TABLES {
            return refuse(format!("A group has at most {MAX_GROUP_TABLES} tables."));
        }
        if g.ask.as_ref().is_some_and(|a| a.until > now) {
            return refuse("They’re being asked already.");
        }
        let min = g.min as usize;
        let tables = g.tables.clone();
        let asked: Vec<String> = online
            .iter()
            .filter(|m| {
                self.members.get(m.as_str()).is_some_and(|m| {
                    m.status == Status::Member
                        && !m
                            .seat_table()
                            .is_some_and(|t| tables.iter().any(|x| x == t))
                })
            })
            .cloned()
            .collect();
        if asked.len() < min {
            return Ok(vec![]);
        }
        self.group_mut(id)?.ask = Some(Ask {
            at: now,
            until: now + ASK_MS,
            asked: asked.clone(),
            yes: vec![],
        });
        Ok(asked)
    }

    /// Member `who` answers group `id`'s question. Returns whether enough
    /// have said yes to open the table now.
    pub fn answer_group(
        &mut self,
        id: &str,
        who: &str,
        yes: bool,
        now: u64,
    ) -> Result<bool, RuleError> {
        let g = self.group_mut(id)?;
        let min = g.min as usize;
        let Some(a) = g.ask.as_mut().filter(|a| a.until > now) else {
            return refuse("That question has closed.");
        };
        if !a.asked.iter().any(|m| m == who) {
            return refuse("You weren’t asked.");
        }
        a.yes.retain(|m| m != who);
        if yes {
            a.yes.push(who.into());
        }
        Ok(a.yes.len() >= min)
    }

    /// Closes group `id`'s question (enough said yes, or its time is up):
    /// with enough yes, a new table like the main table opens at the end of
    /// the group, and those of them seated at other tables move to it.
    pub fn end_ask(
        &mut self,
        id: &str,
        now: u64,
        most: Option<u64>,
        rng: &mut StdRng,
    ) -> Result<AskEnd, RuleError> {
        let g = self.group_mut(id)?;
        let Some(a) = g.ask.take() else {
            return Ok(AskEnd::default());
        };
        let main = g.tables.first().cloned().unwrap_or_default();
        let min = g.min as usize;
        // Those who said yes and are still around to sit.
        let yes: Vec<String> = a
            .yes
            .into_iter()
            .filter(|m| {
                self.members
                    .get(m)
                    .is_some_and(|x| x.status == Status::Member)
            })
            .collect();
        if yes.len() < min
            || self
                .group(id)
                .is_none_or(|g| g.tables.len() >= MAX_GROUP_TABLES)
        {
            return Ok(AskEnd {
                opened: None,
                yes,
                moves: vec![],
            });
        }
        let Some(t) = self.table(&main).cloned() else {
            return refuse("That table isn’t open.");
        };
        let spec = json!({
            "game": t.game, "sb": t.sb, "bb": t.bb, "minBB": t.min_bb, "maxBB": t.max_bb, "seats": t.seats,
            "turnSecs": t.turn_secs, "sitOutMins": t.sit_out_mins, "stayHands": t.stay_hands,
        });
        let new = self.new_table(&spec, now, most, rng)?.id.clone();
        self.group_mut(id)?.tables.push(new.clone());
        let moves = yes
            .iter()
            .filter_map(|m| {
                let from = self.members.get(m)?.seat_table()?.to_string();
                Some(MoveUp {
                    id: m.clone(),
                    from,
                    to: new.clone(),
                })
            })
            .collect();
        Ok(AskEnd {
            opened: Some(new),
            yes,
            moves,
        })
    }

    /// What members are told about a table's group: {group, place} (place 0
    /// is the main table, 1 must-move 1, …), or null.
    pub fn group_label(&self, table: &str) -> Value {
        match self.group_of(table) {
            Some((g, i)) => json!({"group": g.id, "place": i}),
            None => Value::Null,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club_rules::Table;
    use rand::SeedableRng;

    fn rng() -> StdRng {
        StdRng::seed_from_u64(9)
    }

    // A club with members `ids` and one open 6-seat Hold'em table, "t0".
    fn club(ids: &[&str]) -> Club {
        let mut members = serde_json::Map::new();
        for id in ids {
            members.insert(
                (*id).into(),
                json!({"id": id, "name": id.to_uppercase(), "status": "member"}),
            );
        }
        let mut c: Club = serde_json::from_value(json!({
            "code": "ABCD-EFGH", "hostId": "HOST", "members": members,
            "tables": [{"id": "t0", "name": "Main", "game": "nlhe", "sb": 1, "bb": 2, "minBB": 20, "maxBB": 100,
                        "seats": 6, "status": "open"}],
        }))
        .unwrap();
        c.groups.clear();
        c
    }

    fn sit(c: &mut Club, id: &str, table: &str) {
        c.members.get_mut(id).unwrap().other.insert(
            "seat".into(),
            json!({"table": table, "stack": 100, "put": 100}),
        );
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn making_a_group_and_its_minimum() {
        let mut c = club(&["a", "b"]);
        sit(&mut c, "a", "t0");
        let g = c.make_group("t0", None, 5, &mut rng()).unwrap().clone();
        assert_eq!((g.tables.clone(), g.min), (ids(&["t0"]), 4));
        // Whoever is at the main table is in the group from now.
        assert_eq!(g.joined.get("a"), Some(&5));
        assert!(!g.joined.contains_key("b"));
        assert!(
            c.make_group("t0", None, 5, &mut rng())
                .unwrap_err()
                .0
                .contains("in a group already")
        );
        assert!(
            c.make_group("nope", None, 5, &mut rng())
                .unwrap_err()
                .0
                .contains("isn’t open")
        );
        assert!(c.set_group_min(&g.id, 7).unwrap_err().0.contains("2 to 6"));
        assert!(c.set_group_min(&g.id, 1).is_err());
        c.set_group_min(&g.id, 3).unwrap();
        assert_eq!(c.group(&g.id).unwrap().min, 3);
        // Members are told each table's place.
        let body = c.tables_body(&BTreeMap::new());
        assert_eq!(
            body["tables"][0]["group"],
            json!({"group": g.id, "place": 0})
        );
        c.end_group(&g.id).unwrap();
        assert!(c.groups.is_empty());
        assert!(
            c.tables_body(&BTreeMap::new())["tables"][0]
                .get("group")
                .is_none()
        );
    }

    #[test]
    fn time_in_the_group_carries_across_its_tables() {
        let mut c = club(&["a"]);
        let gid = c
            .make_group("t0", Some(2), 0, &mut rng())
            .unwrap()
            .id
            .clone();
        c.groups[0].tables.push("t1".into());
        c.sat_in_group("a", "t1", 10);
        c.sat_in_group("a", "t0", 20);
        assert_eq!(c.group(&gid).unwrap().joined["a"], 10);
        // Moving within the group keeps it; leaving the group doesn't.
        c.left_group("a", "t1", Some("t0"));
        assert_eq!(c.group(&gid).unwrap().joined["a"], 10);
        c.left_group("a", "t0", Some("elsewhere"));
        assert!(c.group(&gid).unwrap().joined.is_empty());
        // A table not in a group: nothing.
        c.sat_in_group("a", "t9", 30);
        assert!(c.group(&gid).unwrap().joined.is_empty());
    }

    #[test]
    fn the_longest_in_the_group_below_moves_up_one_per_free_seat() {
        let mut c = club(&["a", "b", "c", "d", "e"]);
        let gid = c
            .make_group("t0", Some(2), 0, &mut rng())
            .unwrap()
            .id
            .clone();
        c.groups[0].tables.extend(ids(&["t1", "t2"]));
        for (id, at) in [("a", 30), ("b", 10), ("c", 20), ("d", 5), ("e", 1)] {
            c.groups[0].joined.insert(id.into(), at);
        }
        for t in ["t1", "t2"] {
            let mut x = c.table("t0").unwrap().clone();
            x.id = t.into();
            c.tables.push(x);
        }
        // t0: 4 of 6, 1 held: 1 free. t1 (full): b, c, a, d on its way
        // elsewhere, and two with no chips there yet. t2: e.
        for (id, t) in [("a", "t1"), ("b", "t1"), ("c", "t1"), ("e", "t2")] {
            sit(&mut c, id, t);
        }
        let mut seated = BTreeMap::new();
        seated.insert("t0".to_string(), ids(&["x1", "x2", "x3", "x4"]));
        seated.insert("t1".to_string(), ids(&["a", "b", "c", "d", "x5", "x6"]));
        seated.insert("t2".to_string(), ids(&["e"]));
        c.members.get_mut("d").unwrap().other.insert(
            "seat".into(),
            json!({"table": "t1", "moving": {"to": "t0"}}),
        );
        let held = BTreeMap::from([("t0".to_string(), 1)]);
        let moves = c.moves_up(&gid, &seated, &held);
        // b has been in the group longest at t1 (d is moving already); t1 is
        // full, so e stays.
        assert_eq!(
            moves,
            vec![MoveUp {
                id: "b".into(),
                from: "t1".into(),
                to: "t0".into()
            }]
        );
        // A free seat at t1 too: e moves up from t2.
        seated.get_mut("t1").unwrap().pop();
        let moves = c.moves_up(&gid, &seated, &held);
        assert_eq!(moves.len(), 2);
        assert_eq!(
            moves[1],
            MoveUp {
                id: "e".into(),
                from: "t2".into(),
                to: "t1".into()
            }
        );
        // No one below with a free seat above: no moves.
        assert!(
            c.moves_up(&gid, &BTreeMap::new(), &BTreeMap::new())
                .is_empty()
        );
    }

    #[test]
    fn who_hears_what_about_a_new_table() {
        let mut c = club(&["a", "b", "c", "d"]);
        let gid = c
            .make_group("t0", Some(2), 0, &mut rng())
            .unwrap()
            .id
            .clone();
        c.tables.push(
            c.table("t0")
                .map(|t| Table {
                    id: "other".into(),
                    ..t.clone()
                })
                .unwrap(),
        );
        sit(&mut c, "b", "other");
        assert!(
            c.ask_about(&gid, &ids(&["a"]), 100)
                .unwrap_err()
                .0
                .contains("Fewer than 2")
        );
        let n = c.ask_about(&gid, &ids(&["a", "b", "c"]), 100).unwrap();
        assert_eq!(n.until, Some(100 + ASK_MS));
        assert_eq!(
            n.sends[0],
            json!({"to": "a", "kind": "ask", "name": "Main", "until": 100 + ASK_MS})
        );
        assert_eq!(n.sends.len(), 3);
        // Someone not asked hears it's closed to them.
        let n = c.group_answer(&gid, "d", true, 101);
        assert_eq!(n.sends[0]["kind"], "closed");
        assert!(!n.finish);
        assert!(!c.group_answer(&gid, "a", false, 101).finish);
        assert!(!c.group_answer(&gid, "b", true, 102).finish);
        assert!(c.group_answer(&gid, "c", true, 103).finish);
        let n = c.finish_ask(&gid, 103, None, &mut rng());
        let new = n.opened.clone().unwrap();
        // A said no: closed. B moves; C sits down there once it runs.
        assert_eq!(n.sends, vec![json!({"to": "a", "kind": "closed"})]);
        assert_eq!(n.moves.len(), 1);
        assert_eq!(n.moves[0].id, "b");
        let name = c.table(&new).unwrap().name.clone();
        assert_eq!(
            n.after_open,
            vec![json!({"to": "c", "kind": "open", "table": new, "name": name})]
        );
    }

    #[test]
    fn time_up_those_who_said_yes_hear_why() {
        let mut c = club(&["a", "b", "c"]);
        let gid = c
            .make_group("t0", Some(3), 0, &mut rng())
            .unwrap()
            .id
            .clone();
        c.ask_about(&gid, &ids(&["a", "b", "c"]), 100).unwrap();
        c.group_answer(&gid, "a", true, 101);
        let n = c.finish_ask(&gid, 100 + ASK_MS, None, &mut rng());
        assert!(n.opened.is_none() && n.after_open.is_empty());
        assert_eq!(
            n.sends,
            vec![
                json!({"to": "a", "kind": "closed", "reason": "Not enough players for a new table."}),
                json!({"to": "b", "kind": "closed"}),
                json!({"to": "c", "kind": "closed"}),
            ]
        );
        // Closed now: answering late is told so.
        assert_eq!(
            c.group_answer(&gid, "b", true, 100 + ASK_MS).sends[0]["reason"],
            "That question has closed."
        );
    }

    #[test]
    fn a_new_table_opens_once_enough_say_yes() {
        let mut c = club(&["a", "b", "c", "d", "e"]);
        let gid = c
            .make_group("t0", Some(3), 0, &mut rng())
            .unwrap()
            .id
            .clone();
        sit(&mut c, "a", "t0");
        // Seated elsewhere: asked too.
        c.tables.push(
            c.table("t0")
                .map(|t| Table {
                    id: "other".into(),
                    ..t.clone()
                })
                .unwrap(),
        );
        sit(&mut c, "b", "other");
        let online = ids(&["a", "b", "c", "d", "HOST", "z"]);
        let asked = c.ask_group(&gid, &online, 100).unwrap();
        assert_eq!(asked, ids(&["b", "c", "d"]));
        assert!(
            c.ask_group(&gid, &online, 100)
                .unwrap_err()
                .0
                .contains("already")
        );
        assert!(
            c.answer_group(&gid, "a", true, 101)
                .unwrap_err()
                .0
                .contains("weren’t asked")
        );
        assert!(!c.answer_group(&gid, "b", true, 101).unwrap());
        assert!(!c.answer_group(&gid, "c", true, 102).unwrap());
        assert!(!c.answer_group(&gid, "c", false, 103).unwrap());
        assert!(!c.answer_group(&gid, "c", true, 104).unwrap());
        assert!(c.answer_group(&gid, "d", true, 105).unwrap());
        let end = c.end_ask(&gid, 105, None, &mut rng()).unwrap();
        let new = end.opened.clone().unwrap();
        assert_eq!(end.yes, ids(&["b", "c", "d"]));
        // b is seated at another table: moved to the new one.
        assert_eq!(
            end.moves,
            vec![MoveUp {
                id: "b".into(),
                from: "other".into(),
                to: new.clone()
            }]
        );
        let g = c.group(&gid).unwrap();
        assert_eq!(g.tables, vec!["t0".to_string(), new.clone()]);
        assert!(g.ask.is_none());
        // Like the main table.
        let t = c.table(&new).unwrap();
        assert_eq!((t.game.as_str(), t.bb, t.seats), ("nlhe", 2, 6));
        assert_eq!(
            c.tables_body(&BTreeMap::new())["tables"]
                .as_array()
                .unwrap()
                .iter()
                .find(|x| x["id"] == new.as_str())
                .unwrap()["group"]["place"],
            1
        );
        assert!(
            c.answer_group(&gid, "d", true, 106)
                .unwrap_err()
                .0
                .contains("closed")
        );
    }

    #[test]
    fn too_few_and_too_many() {
        let mut c = club(&["a", "b", "c"]);
        let gid = c
            .make_group("t0", Some(3), 0, &mut rng())
            .unwrap()
            .id
            .clone();
        // Too few to ask.
        assert!(c.ask_group(&gid, &ids(&["a", "b"]), 0).unwrap().is_empty());
        assert!(c.group(&gid).unwrap().ask.is_none());
        // Time runs out with two yes: no table, and they're told.
        c.ask_group(&gid, &ids(&["a", "b", "c"]), 0).unwrap();
        c.answer_group(&gid, "a", true, 1).unwrap();
        c.answer_group(&gid, "b", true, 1).unwrap();
        assert!(c.answer_group(&gid, "c", true, ASK_MS).is_err());
        let end = c.end_ask(&gid, ASK_MS, None, &mut rng()).unwrap();
        assert_eq!((end.opened, end.yes), (None, ids(&["a", "b"])));
        assert_eq!(c.tables.len(), 1);
        // The plan's table limit holds.
        c.ask_group(&gid, &ids(&["a", "b", "c"]), ASK_MS).unwrap();
        for m in ["a", "b", "c"] {
            c.answer_group(&gid, m, true, ASK_MS + 1).unwrap();
        }
        assert!(
            c.end_ask(&gid, ASK_MS + 1, Some(1), &mut rng())
                .unwrap_err()
                .0
                .contains("1 table open")
        );
        // Four tables at most.
        c.groups[0].tables = ids(&["t0", "x", "y", "z"]);
        assert!(
            c.ask_group(&gid, &ids(&["a", "b", "c"]), 9 * ASK_MS)
                .unwrap_err()
                .0
                .contains("at most 4")
        );
    }

    #[test]
    fn closed_tables_leave_their_group() {
        let mut c = club(&[]);
        let gid = c
            .make_group("t0", Some(2), 0, &mut rng())
            .unwrap()
            .id
            .clone();
        for t in ["t1", "t2"] {
            let mut x = c.table("t0").unwrap().clone();
            x.id = t.into();
            c.tables.push(x);
            c.groups[0].tables.push(t.into());
        }
        c.tables[1].status = TableStatus::Closed;
        c.prune_groups();
        assert_eq!(c.group(&gid).unwrap().tables, ids(&["t0", "t2"]));
        c.tables[0].status = TableStatus::Closed;
        c.prune_groups();
        assert!(c.groups.is_empty());
    }
}
