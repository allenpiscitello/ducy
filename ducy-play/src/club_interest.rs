//! The club's menu of games and their interest lists: games a club offers
//! even when no table of them is running, so players can sign up for them.
//!
//! - **The menu:** the manager offers games (type, stakes, seats, buy-in and
//!   the table's settings). Players see the offered games that aren't running
//!   and join their lists ([`Club::join_list_with`]), each saying the fewest
//!   players they'd start a table with (none: a full table).
//! - **Opening one:** once enough are interested to start a table (a full
//!   one, or fewer when enough of them would start short:
//!   [`Club::interest_need`]), the first of them are asked "We're starting a
//!   new table. Join?", players at other tables too. No answer leaves them on
//!   the list, and each [`ASK_MS`] the question goes further down it; a no
//!   takes them off the list; once enough say yes, a table of the game opens
//!   and those who said yes are seated there (moved, if they're at another
//!   table). When no one is left to ask, it opens if every one who said yes
//!   would start with that many; else it doesn't, and they're told.
//!
//! The caller runs the tables: it sends the questions and the answers, times
//! the rounds and starts the table.

use rand::rngs::StdRng;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::club_groups::{ASK_MS, Ask, GROUP_BY, GroupNews, MoveUp};
use crate::club_rules::{Club, HOLD_MS, RuleError, Status, Table, refuse};

/// A game on the club's menu.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MenuGame {
    /// Its key ([`Table::game_key`]).
    pub key: String,
    /// The name a table of it gets ("" for the default).
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
    /// Seconds to act.
    pub turn_secs: u32,
    /// Minutes a seat may sit out.
    pub sit_out_mins: u32,
    /// The hit-and-run rule, in hands.
    pub stay_hands: u32,
}

impl MenuGame {
    /// A table's settings for it, as [`Club::new_table`] takes them.
    pub fn spec(&self) -> Value {
        json!({
            "name": self.name, "game": self.game, "sb": self.sb, "bb": self.bb, "minBB": self.min_bb,
            "maxBB": self.max_bb, "seats": self.seats, "turnSecs": self.turn_secs,
            "sitOutMins": self.sit_out_mins, "stayHands": self.stay_hands,
        })
    }
}

/// The question about an offered game: who was asked and said yes, and how
/// many it takes to start.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InterestAsk {
    /// The question.
    pub ask: Ask,
    /// How many must say yes.
    pub need: usize,
}

/// The group id an offered game's question goes by in messages to members.
pub fn interest_group(key: &str) -> String {
    format!("interest:{key}")
}

impl Club {
    /// The offered game with key `key`.
    pub fn offered(&self, key: &str) -> Option<&MenuGame> {
        self.menu.iter().find(|g| g.key == key)
    }

    /// Offers a game, from a table's settings (as [`Club::new_table`] takes
    /// them, checked the same way). Refused if it's offered already.
    pub fn offer_game(&mut self, spec: &Value, rng: &mut StdRng) -> Result<&MenuGame, RuleError> {
        // The same checks as a new table, on a copy.
        let mut probe = self.clone();
        probe.groups.clear();
        let t: Table = probe.new_table(spec, 0, None, rng)?.clone();
        let key = t.game_key();
        if self.offered(&key).is_some() {
            return refuse("That game is offered already.");
        }
        let named = spec
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|n| !n.trim().is_empty());
        self.menu.push(MenuGame {
            key,
            name: if named { t.name } else { String::new() },
            game: t.game,
            sb: t.sb,
            bb: t.bb,
            min_bb: t.min_bb,
            max_bb: t.max_bb,
            seats: t.seats,
            turn_secs: t.turn_secs,
            sit_out_mins: t.sit_out_mins,
            stay_hands: t.stay_hands,
        });
        Ok(self.menu.last().expect("just added"))
    }

    /// Stops offering game `key`; an open question about it closes. Whether
    /// it was offered.
    pub fn remove_offer(&mut self, key: &str) -> bool {
        let before = self.menu.len();
        self.menu.retain(|g| g.key != key);
        self.interest.remove(key);
        self.interest_tried.remove(key);
        self.menu.len() < before
    }

    /// Those waiting on game `key`'s list (not offered a seat, not passed
    /// over, still members), each with the fewest they'd start with.
    fn interested(&self, key: &str, seats: u32) -> Vec<(String, u32)> {
        self.list(key)
            .iter()
            .filter(|e| e.waiting())
            .filter(|e| {
                self.members
                    .get(&e.member)
                    .is_some_and(|m| m.status == Status::Member)
            })
            .map(|e| (e.member.clone(), e.min.unwrap_or(seats).min(seats)))
            .collect()
    }

    /// How many it takes to start a table of offered game `key` now: the
    /// most, up to a full table, that enough of those interested would start
    /// with. None when it's running, not offered, or too few are interested.
    pub fn interest_need(&self, key: &str) -> Option<usize> {
        let g = self.offered(key)?;
        if !self.game_tables(key).is_empty() {
            return None;
        }
        let people = self.interested(key, g.seats);
        (2..=g.seats as usize)
            .rev()
            .find(|&k| people.iter().filter(|(_, m)| *m as usize <= k).count() >= k)
    }

    /// Whether offered game `key` should ask about a table now: it isn't
    /// running, no question is open, enough are interested, and they aren't
    /// the same people a question just closed on without a table.
    pub fn wants_interest(&self, key: &str) -> bool {
        !self.interest.contains_key(key)
            && self.interest_need(key).is_some()
            && self.interest_tried.get(key) != Some(&self.interest_who(key))
    }

    /// Who's interested in game `key` now and the fewest each would start
    /// with, in order: what a question that closed without a table is
    /// remembered by.
    fn interest_who(&self, key: &str) -> String {
        let seats = self.offered(key).map_or(0, |g| g.seats);
        self.interested(key, seats)
            .iter()
            .map(|(id, m)| format!("{id}:{m}"))
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Asks the first of those interested in game `key` (as many as it takes,
    /// each willing to start with that many) whether they'd join a table now.
    /// The question for each, or why not.
    pub fn ask_interest(&mut self, key: &str, now: u64) -> Result<GroupNews, RuleError> {
        if self.interest.contains_key(key) {
            return refuse("They’re being asked already.");
        }
        let Some(need) = self.interest_need(key) else {
            return refuse("Too few are interested yet.");
        };
        let seats = self.offered(key).map_or(0, |g| g.seats);
        let asked: Vec<String> = self
            .interested(key, seats)
            .into_iter()
            .filter(|(_, m)| *m as usize <= need)
            .map(|(id, _)| id)
            .take(need)
            .collect();
        self.interest.insert(
            key.into(),
            InterestAsk {
                ask: Ask {
                    at: now,
                    until: now + ASK_MS,
                    asked: asked.clone(),
                    yes: vec![],
                },
                need,
            },
        );
        Ok(self.interest_asking(key, &asked))
    }

    fn interest_name(&self, key: &str) -> String {
        self.offered(key).map_or(String::new(), |g| {
            if g.name.is_empty() {
                self.default_table_name(&g.game, g.sb, g.bb)
            } else {
                g.name.clone()
            }
        })
    }

    fn interest_asking(&self, key: &str, asked: &[String]) -> GroupNews {
        let until = self.interest.get(key).map(|a| a.ask.until);
        let name = self.interest_name(key);
        GroupNews {
            sends: asked
                .iter()
                .map(|m| json!({"to": m, "kind": "ask", "name": name, "until": until}))
                .collect(),
            until,
            ..GroupNews::default()
        }
    }

    /// A round of game `key`'s question ended without enough yes: it goes
    /// further down the list (those who didn't answer stay on it), or, with
    /// no one left to ask, `finish`.
    pub fn extend_interest(&mut self, key: &str, now: u64) -> GroupNews {
        let Some(q) = self.interest.get(key).cloned() else {
            return GroupNews::default();
        };
        let seats = self.offered(key).map_or(0, |g| g.seats);
        let more: Vec<String> = self
            .interested(key, seats)
            .into_iter()
            .filter(|(id, m)| *m as usize <= q.need && !q.ask.asked.contains(id))
            .map(|(id, _)| id)
            .take(q.need.saturating_sub(q.ask.yes.len()))
            .collect();
        if more.is_empty() {
            return GroupNews {
                finish: true,
                ..GroupNews::default()
            };
        }
        if let Some(q) = self.interest.get_mut(key) {
            q.ask.asked.extend(more.iter().cloned());
            q.ask.until = now + ASK_MS;
        }
        self.interest_asking(key, &more)
    }

    /// Member `who` answers game `key`'s question: `finish` once enough said
    /// yes; a no takes them off the list; a closed question or someone not
    /// asked is told it's closed.
    pub fn interest_answer(&mut self, key: &str, who: &str, yes: bool) -> GroupNews {
        let refused = |why: &str| GroupNews {
            sends: vec![json!({"to": who, "kind": "closed", "reason": why})],
            ..GroupNews::default()
        };
        let Some(q) = self.interest.get_mut(key) else {
            return refused("That question has closed.");
        };
        if !q.ask.asked.iter().any(|m| m == who) {
            return refused("You weren’t asked.");
        }
        q.ask.yes.retain(|m| m != who);
        if yes {
            q.ask.yes.push(who.into());
        }
        let finish = q.ask.yes.len() >= q.need;
        if !yes {
            self.leave_list(who, key);
        }
        GroupNews {
            finish,
            lists_to: if yes { vec![] } else { vec![who.into()] },
            ..GroupNews::default()
        }
    }

    /// Closes game `key`'s question. With enough yes, a table of the game
    /// opens: those of them at other tables move to it, the rest are told to
    /// sit down there once it runs (their seats held), and all are off the
    /// list. Without, those who said yes hear why. Everyone else asked hears
    /// it's closed; those who didn't answer stay on the list.
    pub fn finish_interest(
        &mut self,
        key: &str,
        now: u64,
        most: Option<u64>,
        rng: &mut StdRng,
    ) -> GroupNews {
        let mut out = GroupNews::default();
        let Some(q) = self.interest.remove(key) else {
            return out;
        };
        let yes: Vec<String> = q
            .ask
            .yes
            .iter()
            .filter(|m| {
                self.members
                    .get(*m)
                    .is_some_and(|x| x.status == Status::Member)
            })
            .cloned()
            .collect();
        let spec = self.offered(key).map(MenuGame::spec);
        // Enough: as many as it took, or fewer that every one of them would
        // start with.
        let seats = self.offered(key).map_or(0, |g| g.seats);
        let willing = |m: &String| {
            self.list(key)
                .iter()
                .find(|e| &e.member == m)
                .map_or(seats, |e| e.min.unwrap_or(seats)) as usize
        };
        let enough =
            yes.len() >= q.need || (yes.len() >= 2 && yes.iter().all(|m| willing(m) <= yes.len()));
        let opened = match spec {
            Some(spec) if enough && self.game_tables(key).is_empty() => self
                .new_table(&spec, now, most, rng)
                .ok()
                .map(|t| (t.id.clone(), t.name.clone())),
            _ => None,
        };
        for m in &q.ask.asked {
            match (&opened, yes.contains(m)) {
                (None, true) => out.sends.push(
                    json!({"to": m, "kind": "closed", "reason": "Not enough players for a new table."}),
                ),
                (_, false) => out.sends.push(json!({"to": m, "kind": "closed"})),
                (Some(_), true) => {}
            }
        }
        let Some((tid, name)) = opened else {
            // Not asked again until someone joins or leaves the list, or
            // changes how few they'd start with.
            let who = self.interest_who(key);
            self.interest_tried.insert(key.into(), who);
            return out;
        };
        self.interest_tried.remove(key);
        for m in &yes {
            if self.leave_list(m, key) {
                out.lists_to.push(m.clone());
            }
            match self.members.get(m).and_then(|x| x.seat_table()) {
                Some(from) => out.moves.push(MoveUp {
                    id: m.clone(),
                    from: from.to_string(),
                    to: tid.clone(),
                }),
                None => {
                    let _ = self.reserve_seat(m, &tid, 0, None, Some(GROUP_BY), now, HOLD_MS);
                    out.after_open
                        .push(json!({"to": m, "kind": "open", "table": tid, "name": name}));
                }
            }
        }
        out.opened = Some(tid);
        out
    }

    /// What members are told of the menu: each offered game that isn't
    /// running, as the tables are ({key, name, game, sb, bb, min, max,
    /// seats}).
    pub fn menu_body(&self) -> Vec<Value> {
        self.menu
            .iter()
            .filter(|g| self.game_tables(&g.key).is_empty())
            .map(|g| {
                json!({
                    "key": g.key, "name": self.interest_name(&g.key), "game": g.game, "sb": g.sb, "bb": g.bb,
                    "min": g.min_bb * g.bb, "max": g.max_bb * g.bb, "seats": g.seats,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn rng() -> StdRng {
        StdRng::seed_from_u64(3)
    }

    const K: &str = "nlhe:1/2:6";

    // A club with members `ids`, no tables, offering Hold'em 1/2 6-max.
    fn club(ids: &[&str]) -> Club {
        let mut members = serde_json::Map::new();
        for id in ids {
            members.insert(
                (*id).into(),
                json!({"id": id, "name": id.to_uppercase(), "status": "member"}),
            );
        }
        let mut c: Club = serde_json::from_value(
            json!({"code": "ABCD-EFGH", "hostId": "HOST", "members": members}),
        )
        .unwrap();
        c.offer_game(
            &json!({"game": "nlhe", "sb": 1, "bb": 2, "seats": 6}),
            &mut rng(),
        )
        .unwrap();
        c
    }

    #[test]
    fn the_menu_and_signing_up() {
        let mut c = club(&["a"]);
        assert_eq!(c.menu[0].key, K);
        assert!(c.tables.is_empty());
        assert!(
            c.offer_game(
                &json!({"game": "nlhe", "sb": 1, "bb": 2, "seats": 6}),
                &mut rng()
            )
            .unwrap_err()
            .0
            .contains("offered already")
        );
        assert!(c.offer_game(&json!({"seats": 20}), &mut rng()).is_err());
        assert_eq!(c.menu_body()[0]["name"], "NLH - 0.01/0.02");
        // Signing up for a game that isn't running, willing to start with 4.
        assert_eq!(c.join_list_with("a", K, Some(4), 1).unwrap(), 1);
        assert_eq!(c.list(K)[0].min, Some(4));
        assert!(
            c.join_list_with("a", K, Some(7), 1)
                .unwrap_err()
                .0
                .contains("2 to 6")
        );
        assert!(c.join_list("a", "plo4:1/2:6", 1).is_err());
        // The list stays while the game is offered; not once it isn't.
        assert!(c.lapse_lists(10).is_empty());
        assert!(c.remove_offer(K));
        assert_eq!(c.lapse_lists(11)[0]["body"]["kind"], "off");
    }

    #[test]
    fn how_many_it_takes() {
        let mut c = club(&["a", "b", "c", "d", "e", "f"]);
        for (i, m) in ["a", "b", "c"].iter().enumerate() {
            c.join_list(m, K, i as u64).unwrap();
        }
        // Three wanting a full table: not enough.
        assert_eq!(c.interest_need(K), None);
        // Three who'd start with 3: a table of 3.
        for m in ["a", "b", "c"] {
            c.join_list_with(m, K, Some(3), 5).unwrap();
        }
        assert_eq!(c.interest_need(K), Some(3));
        // Six: a full one.
        for m in ["d", "e", "f"] {
            c.join_list(m, K, 9).unwrap();
        }
        assert_eq!(c.interest_need(K), Some(6));
        assert!(c.wants_interest(K));
    }

    #[test]
    fn asked_then_further_down_and_it_opens() {
        let mut c = club(&["a", "b", "c", "d", "e"]);
        for (i, m) in ["a", "b", "c", "d"].iter().enumerate() {
            c.join_list_with(m, K, Some(2), i as u64).unwrap();
        }
        c.join_list("e", K, 9).unwrap();
        // Four would start with 2 or more; e wants a full table: 4 needed,
        // and the first four are asked.
        let n = c.ask_interest(K, 100).unwrap();
        assert_eq!(c.interest[K].need, 4);
        assert_eq!(n.sends.len(), 4);
        assert_eq!(n.sends[0]["kind"], "ask");
        assert!(!c.wants_interest(K));
        // a says no: off the list. b and c yes; d doesn't answer.
        let n = c.interest_answer(K, "a", false);
        assert_eq!(n.lists_to, vec!["a".to_string()]);
        assert_eq!(c.place_on_list("a", K), None);
        c.interest_answer(K, "b", true);
        assert!(!c.interest_answer(K, "c", true).finish);
        assert_eq!(c.interest_answer(K, "z", true).sends[0]["kind"], "closed");
        // d answers late, still asked; three of the four.
        assert!(!c.interest_answer(K, "d", true).finish);
        // A minute on: no one left who'd start with 4 (e wants 6). It closes,
        // and three who'd each start with 2 is enough.
        assert!(c.extend_interest(K, 100 + ASK_MS).finish);
        // b is at another table: moved; c and d sit down there.
        c.tables.push(Table {
            id: "plo".into(),
            game: "plo4".into(),
            ..Table::default()
        });
        c.members
            .get_mut("b")
            .unwrap()
            .other
            .insert("seat".into(), json!({"table": "plo"}));
        let n = c.finish_interest(K, 200, None, &mut rng());
        let tid = n.opened.clone().unwrap();
        assert_eq!(c.table(&tid).unwrap().game_key(), K);
        assert_eq!(n.moves[0].id, "b");
        assert_eq!(n.after_open.len(), 2);
        assert_eq!(c.held_seats(&tid, 200, ""), 2);
        assert_eq!(n.lists_to.len(), 3);
        // a, who said no, hears it's closed; e is still on the list.
        assert!(n.sends.contains(&json!({"to": "a", "kind": "closed"})));
        assert_eq!(c.place_on_list("e", K), Some(1));
        // Running now: not shown on the menu, and no more questions.
        assert!(c.menu_body().is_empty());
        assert_eq!(c.interest_need(K), None);
    }

    #[test]
    fn no_one_left_and_too_few_said_yes() {
        let mut c = club(&["a", "b", "c"]);
        for m in ["a", "b"] {
            c.join_list_with(m, K, Some(2), 0).unwrap();
        }
        c.ask_interest(K, 0).unwrap();
        assert!(c.ask_interest(K, 0).unwrap_err().0.contains("already"));
        c.interest_answer(K, "a", true);
        assert!(c.extend_interest(K, ASK_MS).finish);
        let n = c.finish_interest(K, ASK_MS, None, &mut rng());
        assert!(n.opened.is_none());
        assert_eq!(n.sends[0]["reason"], "Not enough players for a new table.");
        // b didn't answer: still on the list, and a too.
        assert_eq!(c.list(K).len(), 2);
        assert!(c.tables.is_empty());
        // The same two aren't asked again until the list changes.
        assert!(!c.wants_interest(K));
        c.join_list_with("c", K, Some(2), 1).unwrap();
        assert!(c.wants_interest(K));
        c.leave_list("c", K);
        assert!(!c.wants_interest(K));
    }
}
