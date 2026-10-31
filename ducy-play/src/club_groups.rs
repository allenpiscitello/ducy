//! Must move (ducy-web #501, #518): the tables of one game (type, stakes and
//! seats), set up by the manager as one or more main tables and must-move
//! tables feeding them in a chain: C → B → A with one main table, C → (A or
//! B) with two.
//!
//! - **Moving up:** when a seat opens at a table, the player who sat
//!   earliest at the table feeding it moves up, after their hand. Moves are
//!   forced: they move or they quit. The game's waitlist fills the seat they
//!   leave (`club_lists`).
//! - **Main tables only go up:** a must-move table can become a main table,
//!   never the reverse. With "make them main tables automatically", the
//!   first must-move table becomes one once the table behind it is full.
//! - **A new must-move table:** with "open must-move tables automatically",
//!   once every table of the game is full and enough are waiting, the first
//!   of them on the list are asked "We're starting a new table. Join?"
//!   (players at other tables too). Each [`ASK_MS`] without enough yes, the
//!   question goes further down the list. A no takes them off the list. Once
//!   enough say yes, the table opens and they're seated there; when no one is
//!   left to ask, it doesn't open, and those who said yes are told.
//!
//! The caller runs the tables: it carries out the moves (as a manager's
//! move, ducy-web's reserve), sends the questions and the answers, and times
//! the question.

use std::collections::BTreeMap;

use rand::rngs::StdRng;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::club_rules::{Club, HOLD_MS, RuleError, Status, Table, TableStatus, random36, refuse};

/// The most tables a game opens by itself.
pub const MAX_GROUP_TABLES: usize = 4;
/// How long each round of the "Join?" question waits.
pub const ASK_MS: u64 = 60_000;
/// Who a move up is by, for the caller's moves (a manager's move, but no
/// one's): the hit-and-run rule doesn't hold it.
pub const GROUP_BY: &str = "group";

/// A game's must-move tables.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Group {
    /// Its id.
    pub id: String,
    /// Its tables, in order: the main tables, then the must-move tables,
    /// each feeding the one before it (the first, every main table).
    pub tables: Vec<String>,
    /// How many of them are main tables.
    pub mains: usize,
    /// How many must say yes before a new table opens.
    pub min: u32,
    /// Whether must-move tables open by themselves.
    pub auto_open: bool,
    /// Whether a must-move table becomes a main table by itself, once the
    /// table behind it is full.
    pub auto_main: bool,
    /// The "Join?" question, while it's open.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ask: Option<Ask>,
}

impl Default for Group {
    fn default() -> Self {
        Self {
            id: String::new(),
            tables: vec![],
            mains: 1,
            min: 0,
            auto_open: false,
            auto_main: false,
            ask: None,
        }
    }
}

impl Group {
    /// The tables its table `i` feeds: none for a main table.
    pub fn feeds(&self, i: usize) -> &[String] {
        let m = self.mains.min(self.tables.len());
        match i {
            i if i < m => &[],
            i if i == m => &self.tables[..m],
            i => &self.tables[i - 1..i],
        }
    }
}

/// The question "We're starting a new table. Join?".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Ask {
    /// When it was first asked.
    pub at: u64,
    /// When this round ends, and it goes further down the list.
    pub until: u64,
    /// Who was asked, in order.
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
    /// Those of them seated at another table, to move to it.
    pub moves: Vec<MoveUp>,
}

/// What the caller tells members about a game's question, and does.
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
    /// When this round of the question ends: time it.
    pub until: Option<u64>,
    /// Enough said yes, or no one is left to ask: close it now.
    pub finish: bool,
    /// Members whose lists changed (a no takes them off).
    pub lists_to: Vec<String>,
}

/// A game's must-move settings, as the manager sets them.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MustMove {
    /// Must move at all; off, its tables are single tables.
    pub on: bool,
    /// How many main tables.
    pub mains: usize,
    /// Must-move tables open by themselves.
    pub auto_open: bool,
    /// Must-move tables become main tables by themselves.
    pub auto_main: bool,
    /// How many must say yes before a new table opens; the table's seats
    /// when not given.
    pub min: Option<u32>,
}

impl Club {
    /// Asks about a new table for game group `id` (see [`Club::ask_group`]):
    /// the question for each member asked. Refused, with why, if too few are
    /// waiting to ask.
    pub fn ask_about(&mut self, id: &str, now: u64) -> Result<GroupNews, RuleError> {
        let min = self.group(id).map_or(0, |g| g.min);
        let asked = self.ask_group(id, now)?;
        if asked.is_empty() {
            return refuse(format!("Fewer than {min} players are waiting."));
        }
        Ok(self.asking(id, &asked))
    }

    fn asking(&self, id: &str, asked: &[String]) -> GroupNews {
        let g = self.group(id).expect("asked");
        let until = g.ask.as_ref().map(|a| a.until);
        let name = g
            .tables
            .first()
            .and_then(|t| self.table(t))
            .map_or(String::new(), |t| t.name.clone());
        GroupNews {
            sends: asked
                .iter()
                .map(|m| json!({"to": m, "kind": "ask", "name": name, "until": until}))
                .collect(),
            until,
            ..GroupNews::default()
        }
    }

    /// A round of group `id`'s question ended without enough yes: it goes
    /// further down the list, or, with no one left to ask, `finish`.
    pub fn extend_about(&mut self, id: &str, now: u64) -> GroupNews {
        let asked = self.extend_ask(id, now);
        if asked.is_empty() {
            return GroupNews {
                finish: true,
                ..GroupNews::default()
            };
        }
        self.asking(id, &asked)
    }

    /// Member `who` answers group `id`'s question: `finish` once enough said
    /// yes; a no takes them off the list; a closed question or someone not
    /// asked is told it's closed.
    pub fn group_answer(&mut self, id: &str, who: &str, yes: bool, now: u64) -> GroupNews {
        match self.answer_group(id, who, yes, now) {
            Ok(finish) => GroupNews {
                finish,
                lists_to: if yes { vec![] } else { vec![who.into()] },
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
    /// one, the rest hear it's closed, and those who said yes and aren't
    /// moving are told to sit down there once it runs (their seats held).
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
                (_, false) => out.sends.push(json!({"to": m, "kind": "closed"})),
                (Some(_), true) => {}
            }
        }
        if let Some((tid, name)) = opened {
            // Those who said yes sit there: off the game's list.
            let key = self.table(&tid).map(Table::game_key).unwrap_or_default();
            for m in &end.yes {
                if self.leave_list(m, &key) {
                    out.lists_to.push(m.clone());
                }
            }
            for m in &end.yes {
                if !end.moves.iter().any(|x| &x.id == m) {
                    let _ = self.reserve_seat(m, &tid, 0, None, Some(GROUP_BY), now, HOLD_MS);
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

    /// The group table `table` is in, and its place in it (main tables first).
    pub fn group_of(&self, table: &str) -> Option<(&Group, usize)> {
        self.groups
            .iter()
            .find_map(|g| g.tables.iter().position(|t| t == table).map(|i| (g, i)))
    }

    /// The game a group deals: its first table's key.
    pub fn group_key(&self, g: &Group) -> String {
        g.tables
            .first()
            .and_then(|t| self.table(t))
            .map_or(String::new(), Table::game_key)
    }

    /// Game `key`'s group, if it's must move.
    pub fn group_for(&self, key: &str) -> Option<&Group> {
        self.groups.iter().find(|g| self.group_key(g) == key)
    }

    /// Sets game `key`'s must move: off ends it; on, its open tables are
    /// the group's (those in it already keep their places, new ones go at
    /// the end), the first `mains` of them main tables. Main tables never go
    /// back to must move. The group's id, or None when it's off.
    pub fn set_must_move(
        &mut self,
        key: &str,
        s: &MustMove,
        rng: &mut StdRng,
    ) -> Result<Option<String>, RuleError> {
        let mut open: Vec<&Table> = self
            .tables
            .iter()
            .filter(|t| t.status == TableStatus::Open && t.game_key() == key)
            .collect();
        open.sort_by_key(|t| t.created_at);
        let open: Vec<(String, u32)> = open.iter().map(|t| (t.id.clone(), t.seats)).collect();
        let existing = self.group_for(key).map(|g| g.id.clone());
        if !s.on {
            if let Some(id) = existing {
                self.groups.retain(|g| g.id != id);
            }
            return Ok(None);
        }
        let Some(&(_, seats)) = open.first() else {
            return refuse("That game isn’t running.");
        };
        let was = existing.as_deref().and_then(|id| self.group(id)).cloned();
        let min = s.min.or(was.as_ref().map(|g| g.min)).unwrap_or(seats);
        if !(2..=seats).contains(&min) {
            return refuse(format!("A new table needs 2 to {seats} players to start."));
        }
        let mut tables = was.as_ref().map(|g| g.tables.clone()).unwrap_or_default();
        tables.retain(|t| open.iter().any(|(o, _)| o == t));
        for (t, _) in &open {
            if !tables.contains(t) {
                tables.push(t.clone());
            }
        }
        if was.as_ref().is_some_and(|g| s.mains < g.mains) {
            return refuse("A main table can’t go back to must move.");
        }
        if s.mains == 0 || s.mains > tables.len() {
            return refuse(format!("Pick 1 to {} main tables.", tables.len()));
        }
        let g = Group {
            id: existing.clone().unwrap_or_else(|| random36(rng, 6)),
            tables,
            mains: s.mains,
            min,
            auto_open: s.auto_open,
            auto_main: s.auto_open && s.auto_main,
            ask: was.and_then(|g| g.ask),
        };
        let id = g.id.clone();
        match self.groups.iter_mut().find(|x| x.id == id) {
            Some(x) => *x = g,
            None => self.groups.push(g),
        }
        Ok(Some(id))
    }

    /// A new table of a must-move game goes at the end of its chain.
    pub fn join_group(&mut self, table: &str) {
        let Some(key) = self.table(table).map(Table::game_key) else {
            return;
        };
        if self.group_of(table).is_some() {
            return;
        }
        let Some(gid) = self.group_for(&key).map(|g| g.id.clone()) else {
            return;
        };
        if let Ok(g) = self.group_mut(&gid) {
            g.tables.push(table.into());
        }
    }

    /// Tables closed drop out of their groups (the must-move below a closed
    /// one move up a place); a group with no main table left ends.
    pub fn prune_groups(&mut self) {
        let open: Vec<String> = self
            .tables
            .iter()
            .filter(|t| t.status == TableStatus::Open)
            .map(|t| t.id.clone())
            .collect();
        for g in &mut self.groups {
            let m = g.mains.min(g.tables.len());
            let mains = g.tables[..m].iter().filter(|t| open.contains(t)).count();
            g.tables.retain(|t| open.contains(t));
            g.mains = mains;
            if mains == 0 {
                g.tables.clear();
            }
        }
        self.groups.retain(|g| !g.tables.is_empty());
    }

    /// Who's next to move up from table `table`: those with chips there (a
    /// seat in the ledger), not on their way somewhere already, by who sat
    /// there first.
    pub fn move_order(&self, table: &str) -> Vec<String> {
        let mut at: Vec<(u64, &String)> = self
            .members
            .values()
            .filter(|m| m.seat_table() == Some(table))
            .filter(|m| {
                !m.seat()
                    .is_some_and(|s| s.get("moving").is_some_and(|v| !v.is_null()))
            })
            .map(|m| {
                let sat = m
                    .seat()
                    .and_then(|s| s.get("satAt"))
                    .and_then(Value::as_u64)
                    .unwrap_or(u64::MAX);
                (sat, &m.id)
            })
            .collect();
        at.sort();
        at.into_iter().map(|(_, id)| id.clone()).collect()
    }

    /// Who moves up in group `id` now: for each free seat at a table
    /// (`seated`: who's at each of its tables; `held`: seats held at each
    /// for someone on their way), the next in the move order at the table
    /// feeding it.
    pub fn moves_up(
        &self,
        id: &str,
        seated: &BTreeMap<String, Vec<String>>,
        held: &BTreeMap<String, usize>,
    ) -> Vec<MoveUp> {
        let Some(g) = self.group(id) else {
            return vec![];
        };
        let mut free: BTreeMap<&str, usize> = BTreeMap::new();
        for t in &g.tables {
            let Some(x) = self.table(t) else { continue };
            let taken = seated.get(t).map_or(0, Vec::len) + held.get(t).copied().unwrap_or(0);
            free.insert(t, (x.seats as usize).saturating_sub(taken));
        }
        let mut out = vec![];
        for i in g.mains.min(g.tables.len())..g.tables.len() {
            let from = &g.tables[i];
            let here = seated.get(from).cloned().unwrap_or_default();
            for m in self.move_order(from) {
                if !here.contains(&m) {
                    continue;
                }
                let Some(to) = g
                    .feeds(i)
                    .iter()
                    .find(|t| free.get(t.as_str()).copied().unwrap_or(0) > 0)
                else {
                    break;
                };
                *free.get_mut(to.as_str()).expect("counted") -= 1;
                out.push(MoveUp {
                    id: m,
                    from: from.clone(),
                    to: to.clone(),
                });
            }
        }
        out
    }

    /// Whether table `t` is full (`taken`: seats taken at each running table).
    fn full(&self, t: &str, taken: &BTreeMap<String, usize>) -> bool {
        self.table(t)
            .is_some_and(|x| taken.get(t).copied().unwrap_or(0) >= x.seats as usize)
    }

    /// Makes them main tables automatically: while the table behind group
    /// `id`'s first must-move table is full, that one becomes a main table.
    /// Whether any did.
    pub fn promote(&mut self, id: &str, taken: &BTreeMap<String, usize>) -> bool {
        let mut changed = false;
        while let Some(g) = self.group(id) {
            if !g.auto_main || g.tables.len() <= g.mains + 1 {
                break;
            }
            let behind = g.tables[g.mains + 1].clone();
            if !self.full(&behind, taken) {
                break;
            }
            if let Ok(g) = self.group_mut(id) {
                g.mains += 1;
            }
            changed = true;
        }
        changed
    }

    /// Those waiting on group `id`'s game list (not offered a seat, not
    /// passed over), in order.
    fn waiting_for(&self, g: &Group) -> Vec<String> {
        self.list(&self.group_key(g))
            .iter()
            .filter(|e| e.waiting())
            .filter(|e| {
                self.members
                    .get(&e.member)
                    .is_some_and(|m| m.status == Status::Member)
            })
            .map(|e| e.member.clone())
            .collect()
    }

    /// Whether group `id` should ask about a new must-move table: it opens
    /// them automatically, every table is full (`taken`), there's room for
    /// one, and enough are waiting.
    pub fn wants_table(&self, id: &str, taken: &BTreeMap<String, usize>) -> bool {
        let Some(g) = self.group(id) else {
            return false;
        };
        g.auto_open
            && g.ask.is_none()
            && g.tables.len() < MAX_GROUP_TABLES
            && g.tables.iter().all(|t| self.full(t, taken))
            && self.waiting_for(g).len() >= g.min as usize
    }

    /// Asks the first of those waiting on group `id`'s list (as many as it
    /// takes to start one) whether they'd join a new table, when there's
    /// room for one and enough are waiting. Returns who's asked (none: not
    /// asked).
    pub fn ask_group(&mut self, id: &str, now: u64) -> Result<Vec<String>, RuleError> {
        let g = self
            .group(id)
            .ok_or_else(|| RuleError("No such group.".into()))?;
        if g.tables.len() >= MAX_GROUP_TABLES {
            return refuse(format!(
                "A game opens at most {MAX_GROUP_TABLES} tables by itself."
            ));
        }
        if g.ask.is_some() {
            return refuse("They’re being asked already.");
        }
        let min = g.min as usize;
        let waiting = self.waiting_for(g);
        if waiting.len() < min {
            return Ok(vec![]);
        }
        let asked: Vec<String> = waiting.into_iter().take(min).collect();
        self.group_mut(id)?.ask = Some(Ask {
            at: now,
            until: now + ASK_MS,
            asked: asked.clone(),
            yes: vec![],
        });
        Ok(asked)
    }

    /// The question goes further down group `id`'s list: as many more as
    /// are still needed, not asked yet. Returns who's asked now (none: no
    /// one is left).
    pub fn extend_ask(&mut self, id: &str, now: u64) -> Vec<String> {
        let Some(g) = self.group(id) else {
            return vec![];
        };
        let Some(a) = g.ask.as_ref() else {
            return vec![];
        };
        let need = (g.min as usize).saturating_sub(a.yes.len());
        let more: Vec<String> = self
            .waiting_for(g)
            .into_iter()
            .filter(|m| !a.asked.contains(m))
            .take(need)
            .collect();
        if more.is_empty() {
            return more;
        }
        if let Some(a) = self.group_mut(id).ok().and_then(|g| g.ask.as_mut()) {
            a.asked.extend(more.iter().cloned());
            a.until = now + ASK_MS;
        }
        more
    }

    /// Member `who` answers group `id`'s question: a no takes them off the
    /// game's list. Returns whether enough have said yes to open the table.
    pub fn answer_group(
        &mut self,
        id: &str,
        who: &str,
        yes: bool,
        _now: u64,
    ) -> Result<bool, RuleError> {
        let key = self
            .group(id)
            .map(|g| self.group_key(g))
            .unwrap_or_default();
        let g = self.group_mut(id)?;
        let min = g.min as usize;
        let Some(a) = g.ask.as_mut() else {
            return refuse("That question has closed.");
        };
        if !a.asked.iter().any(|m| m == who) {
            return refuse("You weren’t asked.");
        }
        a.yes.retain(|m| m != who);
        if yes {
            a.yes.push(who.into());
        }
        let enough = a.yes.len() >= min;
        if !yes {
            self.leave_list(who, &key);
        }
        Ok(enough)
    }

    /// Closes group `id`'s question: with enough yes, a new table like its
    /// first main table opens at the end of the chain, and those of them
    /// seated at other tables move to it.
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
        // A new table of the game joins the end of its chain.
        let new = self.new_table(&spec, now, most, rng)?.id.clone();
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

    /// What members are told about a table's must move: {group, place,
    /// main}, and for a must-move table the tables it feeds (`to`, by name)
    /// and its move order (`order`, [{id, name}]); null for a single table.
    pub fn group_label(&self, table: &str) -> Value {
        let Some((g, i)) = self.group_of(table) else {
            return Value::Null;
        };
        let main = i < g.mains;
        let mut v = json!({"group": g.id, "place": i, "main": main});
        if !main {
            let to: Vec<&str> = g
                .feeds(i)
                .iter()
                .filter_map(|t| self.table(t))
                .map(|t| t.name.as_str())
                .collect();
            v["to"] = json!(to);
            let order: Vec<Value> = self
                .move_order(table)
                .into_iter()
                .map(|id| {
                    let name = self.members.get(&id).map_or("", |m| m.name.as_str());
                    json!({"id": id, "name": name})
                })
                .collect();
            v["order"] = order.into();
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn rng() -> StdRng {
        StdRng::seed_from_u64(9)
    }

    const K: &str = "nlhe:1/2:6";

    // A club with members `ids` and `n` open 6-seat Hold'em 1/2 tables, t0,
    // t1, … made in that order.
    fn club(ids: &[&str], n: usize) -> Club {
        let mut members = serde_json::Map::new();
        for id in ids {
            members.insert(
                (*id).into(),
                json!({"id": id, "name": id.to_uppercase(), "status": "member"}),
            );
        }
        let tables: Vec<Value> = (0..n)
            .map(|i| {
                json!({"id": format!("t{i}"), "name": format!("T{i}"), "game": "nlhe", "sb": 1, "bb": 2,
                       "minBB": 20, "maxBB": 100, "seats": 6, "status": "open", "createdAt": i})
            })
            .collect();
        serde_json::from_value(json!({
            "code": "ABCD-EFGH", "hostId": "HOST", "members": members, "tables": tables,
        }))
        .unwrap()
    }

    fn sit(c: &mut Club, id: &str, table: &str, at: u64) {
        c.members.get_mut(id).unwrap().other.insert(
            "seat".into(),
            json!({"table": table, "stack": 100, "put": 100, "satAt": at}),
        );
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    fn on(mains: usize) -> MustMove {
        MustMove {
            on: true,
            mains,
            ..MustMove::default()
        }
    }

    fn taken(v: &[(&str, usize)]) -> BTreeMap<String, usize> {
        v.iter().map(|(t, n)| ((*t).to_string(), *n)).collect()
    }

    #[test]
    fn setting_a_game_as_must_move() {
        let mut c = club(&[], 3);
        let gid = c.set_must_move(K, &on(1), &mut rng()).unwrap().unwrap();
        let g = c.group(&gid).unwrap();
        assert_eq!(
            (g.tables.clone(), g.mains, g.min),
            (ids(&["t0", "t1", "t2"]), 1, 6)
        );
        assert_eq!(c.group_for(K).unwrap().id, gid);
        // Two main tables: t2 feeds both.
        c.set_must_move(K, &on(2), &mut rng()).unwrap();
        let g = c.group(&gid).unwrap();
        assert_eq!(g.feeds(2), &ids(&["t0", "t1"])[..]);
        assert!(g.feeds(1).is_empty());
        // A main table never goes back.
        assert!(
            c.set_must_move(K, &on(1), &mut rng())
                .unwrap_err()
                .0
                .contains("can’t go back")
        );
        assert!(c.set_must_move(K, &on(4), &mut rng()).is_err());
        let bad = MustMove {
            min: Some(7),
            ..on(2)
        };
        assert!(
            c.set_must_move(K, &bad, &mut rng())
                .unwrap_err()
                .0
                .contains("2 to 6")
        );
        // Members are told each table's place, and a must-move table's
        // tables above it.
        let body = c.tables_body(&BTreeMap::new());
        assert_eq!(
            body["tables"][0]["group"],
            json!({"group": gid, "place": 0, "main": true})
        );
        assert_eq!(body["tables"][2]["group"]["to"], json!(["T0", "T1"]));
        // A new table of the game joins the end; off, they're single tables.
        c.new_table(
            &json!({"game": "nlhe", "sb": 1, "bb": 2, "seats": 6}),
            9,
            None,
            &mut rng(),
        )
        .unwrap();
        assert_eq!(c.group(&gid).unwrap().tables.len(), 4);
        c.set_must_move(
            K,
            &MustMove {
                on: false,
                ..MustMove::default()
            },
            &mut rng(),
        )
        .unwrap();
        assert!(c.groups.is_empty());
        assert!(
            c.set_must_move("plo:1/2:6", &on(1), &mut rng())
                .unwrap_err()
                .0
                .contains("isn’t running")
        );
    }

    #[test]
    fn who_sat_first_at_the_feeding_table_moves_up() {
        let mut c = club(&["a", "b", "c", "d", "e"], 3);
        let gid = c.set_must_move(K, &on(1), &mut rng()).unwrap().unwrap();
        // t1: c sat first, then a, then b (moving elsewhere already: d).
        for (id, t, at) in [
            ("a", "t1", 20),
            ("b", "t1", 30),
            ("c", "t1", 10),
            ("e", "t2", 5),
        ] {
            sit(&mut c, id, t, at);
        }
        c.members.get_mut("d").unwrap().other.insert(
            "seat".into(),
            json!({"table": "t1", "satAt": 1, "moving": {"to": "t0"}}),
        );
        assert_eq!(c.move_order("t1"), ids(&["c", "a", "b"]));
        let mut seated = BTreeMap::new();
        seated.insert("t0".to_string(), ids(&["x1", "x2", "x3", "x4"]));
        seated.insert("t1".to_string(), ids(&["a", "b", "c", "d", "x5", "x6"]));
        seated.insert("t2".to_string(), ids(&["e"]));
        let held = BTreeMap::from([("t0".to_string(), 1)]);
        // t0: one seat free.
        assert_eq!(
            c.moves_up(&gid, &seated, &held),
            vec![MoveUp {
                id: "c".into(),
                from: "t1".into(),
                to: "t0".into()
            }]
        );
        // A seat free at t1 too: e moves up from t2.
        seated.get_mut("t1").unwrap().pop();
        let moves = c.moves_up(&gid, &seated, &held);
        assert_eq!(moves.len(), 2);
        assert_eq!((moves[1].id.as_str(), moves[1].to.as_str()), ("e", "t1"));
        // Must-move players see their order.
        assert_eq!(
            c.group_label("t1")["order"],
            json!([{"id": "c", "name": "C"}, {"id": "a", "name": "A"}, {"id": "b", "name": "B"}])
        );
    }

    #[test]
    fn two_main_tables_take_from_the_same_must_move_table() {
        let mut c = club(&["a", "b", "c"], 3);
        let gid = c.set_must_move(K, &on(2), &mut rng()).unwrap().unwrap();
        for (id, at) in [("a", 3), ("b", 1), ("c", 2)] {
            sit(&mut c, id, "t2", at);
        }
        let mut seated = BTreeMap::new();
        seated.insert("t0".to_string(), ids(&["x1", "x2", "x3", "x4", "x5"]));
        seated.insert("t1".to_string(), ids(&["y1", "y2", "y3", "y4", "y5"]));
        seated.insert("t2".to_string(), ids(&["a", "b", "c"]));
        let moves = c.moves_up(&gid, &seated, &BTreeMap::new());
        assert_eq!(
            moves
                .iter()
                .map(|m| (m.id.as_str(), m.to.as_str()))
                .collect::<Vec<_>>(),
            vec![("b", "t0"), ("c", "t1")]
        );
    }

    #[test]
    fn a_must_move_table_becomes_main_once_the_one_behind_it_is_full() {
        let mut c = club(&[], 4);
        let s = MustMove {
            auto_open: true,
            auto_main: true,
            ..on(1)
        };
        let gid = c.set_must_move(K, &s, &mut rng()).unwrap().unwrap();
        // t2 (behind t1) not full: nothing.
        assert!(!c.promote(&gid, &taken(&[("t0", 6), ("t1", 6), ("t2", 5)])));
        // Full: t1 is a main table; t3 behind t2 isn't full, so that's all.
        assert!(c.promote(&gid, &taken(&[("t0", 6), ("t1", 6), ("t2", 6), ("t3", 2)])));
        assert_eq!(c.group(&gid).unwrap().mains, 2);
        // Off: never.
        c.groups[0].auto_main = false;
        assert!(!c.promote(&gid, &taken(&[("t2", 6), ("t3", 6)])));
    }

    #[test]
    fn closed_tables_leave_their_group() {
        let mut c = club(&[], 4);
        let gid = c.set_must_move(K, &on(2), &mut rng()).unwrap().unwrap();
        c.tables[2].status = TableStatus::Closed;
        c.prune_groups();
        let g = c.group(&gid).unwrap();
        assert_eq!((g.tables.clone(), g.mains), (ids(&["t0", "t1", "t3"]), 2));
        c.tables[0].status = TableStatus::Closed;
        c.prune_groups();
        assert_eq!(c.group(&gid).unwrap().mains, 1);
        c.tables[1].status = TableStatus::Closed;
        c.prune_groups();
        assert!(c.groups.is_empty());
    }

    // A must-move game opening tables by itself with t0 and t1, needing
    // `min`, and `list` waiting in order.
    fn opening(min: u32, list: &[&str]) -> (Club, String) {
        let mut c = club(&["a", "b", "c", "d", "e", "f"], 2);
        let s = MustMove {
            auto_open: true,
            min: Some(min),
            ..on(1)
        };
        let gid = c.set_must_move(K, &s, &mut rng()).unwrap().unwrap();
        for (i, m) in list.iter().enumerate() {
            c.join_list(m, K, i as u64).unwrap();
        }
        (c, gid)
    }

    #[test]
    fn a_new_table_is_wanted_when_every_table_is_full_and_enough_wait() {
        let (c, gid) = opening(3, &["a", "b", "c"]);
        let full = taken(&[("t0", 6), ("t1", 6)]);
        assert!(c.wants_table(&gid, &full));
        assert!(!c.wants_table(&gid, &taken(&[("t0", 6), ("t1", 5)])));
        let (c, gid) = opening(3, &["a", "b"]);
        assert!(!c.wants_table(&gid, &full));
    }

    #[test]
    fn the_first_on_the_list_are_asked_then_the_next() {
        let (mut c, gid) = opening(2, &["a", "b", "c", "d"]);
        let n = c.ask_about(&gid, 100).unwrap();
        assert_eq!(n.until, Some(100 + ASK_MS));
        assert_eq!(
            n.sends,
            vec![
                json!({"to": "a", "kind": "ask", "name": "T0", "until": 100 + ASK_MS}),
                json!({"to": "b", "kind": "ask", "name": "T0", "until": 100 + ASK_MS}),
            ]
        );
        // b says no: off the list.
        let n = c.group_answer(&gid, "b", false, 101);
        assert_eq!(n.lists_to, ids(&["b"]));
        assert_eq!(c.place_on_list("b", K), None);
        // Not asked: told it's closed to them.
        assert_eq!(
            c.group_answer(&gid, "d", true, 101).sends[0]["kind"],
            "closed"
        );
        assert!(!c.group_answer(&gid, "a", true, 102).finish);
        // A minute on: the next one down (b is off the list).
        let n = c.extend_about(&gid, 100 + ASK_MS);
        assert_eq!(n.sends.len(), 1);
        assert_eq!(n.sends[0]["to"], "c");
        assert_eq!(n.until, Some(100 + 2 * ASK_MS));
        assert!(c.group_answer(&gid, "c", true, 103).finish);
        // c is at another game's table: moved. a sits down there.
        c.tables.push(Table {
            id: "plo".into(),
            game: "plo".into(),
            ..c.tables[0].clone()
        });
        sit(&mut c, "c", "plo", 1);
        let n = c.finish_ask(&gid, 104, None, &mut rng());
        let new = n.opened.clone().unwrap();
        assert_eq!(n.moves.len(), 1);
        assert_eq!(
            (n.moves[0].id.as_str(), n.moves[0].to.as_str()),
            ("c", new.as_str())
        );
        assert_eq!(n.after_open[0]["to"], "a");
        // Both are off the list now; d is still on it.
        assert_eq!(n.lists_to, ids(&["a", "c"]));
        assert_eq!(c.place_on_list("d", K), Some(1));
        // Her seat there is held; the new table is at the end of the chain.
        assert_eq!(c.held_seats(&new, 104, ""), 1);
        assert_eq!(
            c.group(&gid).unwrap().tables,
            vec!["t0".to_string(), "t1".into(), new]
        );
        // b, asked and said no, hears it's closed.
        assert!(n.sends.contains(&json!({"to": "b", "kind": "closed"})));
    }

    #[test]
    fn no_one_left_to_ask_and_too_few_said_yes() {
        let (mut c, gid) = opening(3, &["a", "b", "c"]);
        c.ask_about(&gid, 0).unwrap();
        assert!(c.ask_about(&gid, 0).unwrap_err().0.contains("already"));
        c.group_answer(&gid, "a", true, 1);
        c.group_answer(&gid, "b", true, 1);
        // No one further down: it closes.
        assert!(c.extend_about(&gid, ASK_MS).finish);
        let n = c.finish_ask(&gid, ASK_MS, None, &mut rng());
        assert!(n.opened.is_none());
        assert_eq!(
            n.sends,
            vec![
                json!({"to": "a", "kind": "closed", "reason": "Not enough players for a new table."}),
                json!({"to": "b", "kind": "closed", "reason": "Not enough players for a new table."}),
                json!({"to": "c", "kind": "closed"}),
            ]
        );
        assert_eq!(c.tables.len(), 2);
        // Too few waiting to ask at all.
        c.leave_list("c", K);
        assert!(c.ask_about(&gid, 5).unwrap_err().0.contains("Fewer than 3"));
    }
}
