//! The club manager's Overview and notices (ducy-web #398, #456): the
//! club's chip totals, the numbers on top, the member filters, when members
//! last played and were last active, each open table's dealing time, and
//! what needs the manager. All read the club; nothing changes it but
//! [`deal_clock`], which the caller keeps on each table.
//!
//! Dealing time: a table's clock runs only while hands are being dealt, and
//! stops when fewer than two players are in hands. The caller keeps it on
//! the table as `dealing`: `{ms, since, stoppedAt}`, `ms` dealt before
//! `since` (when it last started; null while stopped), `stoppedAt` when it
//! last stopped.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::club_host::ClubState;
use crate::club_money::{CLUB, Entry, EntryKind};
use crate::club_rules::{RuleError, Status, TableStatus, refuse};

/// A day, in milliseconds.
pub const DAY: u64 = 86_400_000;
/// Joined within this many days: new.
pub const NEW_DAYS: u64 = 7;
/// No hand played in this many days: inactive.
pub const INACTIVE_DAYS: u64 = 30;
/// Cash-outs left owing kept as notices.
pub const LEFT_OWING_KEPT: usize = 50;

/// The clock after the table is (`on`) or isn't dealing at `now`.
pub fn deal_clock(clock: &Value, on: bool, now: u64) -> Value {
    let ms = clock.get("ms").and_then(Value::as_u64).unwrap_or(0);
    let since = clock.get("since").and_then(Value::as_u64);
    let stopped = clock
        .get("stoppedAt")
        .and_then(Value::as_u64)
        .filter(|s| *s > 0);
    match (on, since) {
        (true, None) => match stopped {
            Some(s) => json!({"ms": ms, "since": now, "stoppedAt": s}),
            None => json!({"ms": ms, "since": now}),
        },
        (false, Some(s)) => {
            json!({"ms": ms + now.saturating_sub(s), "since": null, "stoppedAt": now})
        }
        _ => match stopped {
            Some(s) => json!({"ms": ms, "since": since, "stoppedAt": s}),
            None => json!({"ms": ms, "since": since}),
        },
    }
}

/// Whether the clock is running.
pub fn is_dealing(clock: Option<&Value>) -> bool {
    clock
        .and_then(|c| c.get("since"))
        .is_some_and(|s| !s.is_null())
}

/// How long the table has dealt in all, up to `now`.
pub fn dealing_ms(clock: Option<&Value>, now: u64) -> u64 {
    let Some(c) = clock else { return 0 };
    let ms = c.get("ms").and_then(Value::as_u64).unwrap_or(0);
    ms + c
        .get("since")
        .and_then(Value::as_u64)
        .map_or(0, |s| now.saturating_sub(s))
}

/// The club's books: what it issued and took back, where the chips are, and
/// what's owed each way on its direct links (an agent's link is their
/// players' too).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    /// Issued.
    pub issued: i64,
    /// Taken back.
    pub reclaimed: i64,
    /// On the club's links.
    pub balances: i64,
    /// At tables.
    pub on_tables: i64,
    /// Held by tournaments until they pay out or refund.
    pub in_tourneys: i64,
    /// What its direct members owe it.
    pub owed_to_club: i64,
    /// What it owes them.
    pub owed_to_players: i64,
    /// Every chip issued and not taken back is somewhere.
    pub conserved: bool,
}

/// The first entry of each transaction: the member's own move.
fn own_entries(ledger: &[Entry]) -> Vec<&Entry> {
    let mut out: Vec<&Entry> = vec![];
    for e in ledger {
        if out.last().is_none_or(|l| l.tx != e.tx) {
            out.push(e);
        }
    }
    out
}

fn num(v: Option<&Value>) -> u64 {
    match v {
        Some(Value::Number(n)) => n
            .as_u64()
            .or_else(|| n.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64))
            .unwrap_or(0),
        Some(Value::String(s)) => s.trim().parse::<f64>().map_or(0, |f| f.max(0.0) as u64),
        _ => 0,
    }
}

fn text(v: Option<&Value>) -> String {
    v.and_then(Value::as_str).unwrap_or("").to_string()
}

impl ClubState {
    /// The club's books.
    pub fn totals(&self) -> Totals {
        let links: Vec<i64> = self
            .book
            .money
            .links()
            .iter()
            .filter(|l| l.backer == CLUB)
            .map(|l| l.balance)
            .collect();
        let on_tables: i64 = self
            .book
            .club
            .members
            .values()
            .filter_map(|m| m.seat())
            .map(|s| num(s.get("stack")) as i64)
            .sum();
        let in_tourneys: i64 = self.tourneys.iter().map(|t| t.held as i64).sum();
        let balances: i64 = links.iter().sum();
        let c = self.book.chips;
        Totals {
            issued: c.issued,
            reclaimed: c.reclaimed,
            balances,
            on_tables,
            in_tourneys,
            owed_to_club: links.iter().map(|b| (-b).max(0)).sum(),
            owed_to_players: links.iter().map(|b| (*b).max(0)).sum(),
            conserved: c.issued - c.reclaimed == balances + on_tables + in_tourneys,
        }
    }

    /// When each member last played (a table or tournament entry in the
    /// ledger), by id.
    pub fn last_played(&self) -> BTreeMap<String, u64> {
        let mut out: BTreeMap<String, u64> = BTreeMap::new();
        for e in own_entries(self.book.money.ledger()) {
            let played = matches!(
                e.kind,
                EntryKind::BuyIn | EntryKind::TopUp | EntryKind::CashOut | EntryKind::TourneyBuyIn
            );
            if played && e.at > out.get(&e.member).copied().unwrap_or(0) {
                out.insert(e.member.clone(), e.at);
            }
        }
        out
    }

    /// The members in a filter, by id: "all", "new" (joined in the last
    /// [`NEW_DAYS`]), "inactive" (no hand in [`INACTIVE_DAYS`], not seated;
    /// one who joined since isn't yet), "owes" (a negative balance),
    /// "agents", "noagent" (under no agent), or "agent" (agent `agent`'s
    /// players). Anything else is all.
    pub fn member_filter(
        &self,
        filter: &str,
        now: u64,
        played: &BTreeMap<String, u64>,
        agent: &str,
    ) -> Vec<String> {
        let members = &self.book.club.members;
        let is_agent = |id: &str| {
            members
                .get(id)
                .is_some_and(|m| m.agent.as_deref() == Some("yes"))
        };
        members
            .values()
            .filter(|m| m.status == Status::Member)
            .filter(|m| {
                let joined = m.joined_at.unwrap_or(0);
                match filter {
                    "new" => now.saturating_sub(joined) < NEW_DAYS * DAY || joined > now,
                    "inactive" => {
                        let last = played.get(&m.id).copied().unwrap_or(0).max(joined);
                        m.seat().is_none() && now.saturating_sub(last) >= INACTIVE_DAYS * DAY
                    }
                    "owes" => self.book.balance(&m.id) < 0,
                    "agents" => is_agent(&m.id),
                    "noagent" => m.agent_of.as_deref().is_none_or(|a| !is_agent(a)),
                    "agent" => !agent.is_empty() && m.agent_of.as_deref() == Some(agent),
                    _ => true,
                }
            })
            .map(|m| m.id.clone())
            .collect()
    }

    /// When each member was last active, by id: last heard from (`seen`),
    /// last played, or joined, whichever is latest; 0 if none is known.
    pub fn last_active(
        &self,
        seen: &Map<String, Value>,
        played: &BTreeMap<String, u64>,
    ) -> BTreeMap<String, u64> {
        self.book
            .club
            .members
            .values()
            .map(|m| {
                let at = num(seen.get(&m.id))
                    .max(played.get(&m.id).copied().unwrap_or(0))
                    .max(m.joined_at.unwrap_or(0));
                (m.id.clone(), at)
            })
            .collect()
    }

    /// The numbers on top: tables dealing of those open, chips out (given
    /// out less taken back), members, joined lately, inactive, and what's
    /// owed each way.
    pub fn overview_numbers(&self, now: u64) -> Value {
        let tables: Vec<_> = self.book.club.open_tables().collect();
        let t = self.totals();
        let played = self.last_played();
        let count = |f: &str| self.member_filter(f, now, &played, "").len();
        json!({
            "tablesActive": tables.iter().filter(|x| is_dealing(x.other.get("dealing"))).count(),
            "tables": tables.len(),
            "chipsOut": t.issued - t.reclaimed,
            "members": count("all"),
            "joined": count("new"),
            "inactive": count("inactive"),
            "owedToClub": t.owed_to_club,
            "owedToPlayers": t.owed_to_players,
        })
    }

    /// Each open table for the Overview: its game, status, dealing time and
    /// seats taken (`seated`, by table).
    pub fn table_summaries(&self, seated: &Map<String, Value>, now: u64) -> Vec<Value> {
        self.book
            .club
            .open_tables()
            .map(|t| {
                let d = t.other.get("dealing");
                let on = is_dealing(d);
                json!({
                    "id": t.id, "name": t.name, "game": t.game, "sb": t.sb, "bb": t.bb, "seats": t.seats,
                    "taken": num(seated.get(&t.id)), "status": t.status, "dealing": on,
                    "ms": dealing_ms(d, now),
                    "stoppedAt": if on { 0 } else { num(d.and_then(|d| d.get("stoppedAt"))) },
                })
            })
            .collect()
    }

    /// Credit notices (#404), newest first: players at their limit asking
    /// for more, and the last `keep` cash-outs that left a player's own
    /// link below zero, with what they owe now.
    pub fn credit_notices(&self, keep: usize) -> Vec<Value> {
        let mut out = vec![];
        for m in self.book.club.members.values() {
            let Some(q) = m.other.get("request").filter(|q| q.is_object()) else {
                continue;
            };
            if q.get("atLimit").and_then(Value::as_bool) == Some(true) {
                out.push(json!({
                    "kind": "at_limit", "member": m.id, "name": m.name, "asked": num(q.get("amount")),
                    "credit": self.book.credit(&m.id), "available": self.book.available(&m.id),
                    "table": text(q.get("table")), "at": num(q.get("at")),
                }));
            }
        }
        let left: Vec<&Entry> = own_entries(self.book.money.ledger())
            .into_iter()
            .filter(|e| e.kind == EntryKind::CashOut && e.balance < 0)
            .collect();
        for e in &left[left.len().saturating_sub(keep)..] {
            let name = self
                .book
                .club
                .members
                .get(&e.member)
                .map_or(e.member.clone(), |m| m.name.clone());
            out.push(json!({
                "kind": "left_owing", "member": e.member, "name": name, "owes": -e.balance,
                "owesNow": (-self.book.balance(&e.member)).max(0), "table": e.about.clone().unwrap_or_default(),
                "at": e.at, "n": e.n,
            }));
        }
        out.sort_by_key(|n| std::cmp::Reverse(num(n.get("at"))));
        out
    }

    /// What needs the manager, newest first, each with a `key` (unique and
    /// stable while it stands) and what its quick actions need: join, chips,
    /// at_limit, help, prize, stopped, left_owing. Those `cleared` are left
    /// out; `seated`: seats taken by table.
    pub fn notices(&self, cleared: &[String], seated: &Map<String, Value>) -> Vec<Value> {
        let members = &self.book.club.members;
        let mut out = vec![];
        for m in members.values() {
            if m.status == Status::Pending {
                let via = m
                    .agent_of
                    .as_deref()
                    .and_then(|a| members.get(a))
                    .map_or(String::new(), |a| a.name.clone());
                let at = m.requested_at.unwrap_or(0);
                out.push(json!({
                    "key": format!("join:{}:{at}", m.id), "kind": "join", "member": m.id, "name": m.name,
                    "via": via, "removedAt": m.removed_at.unwrap_or(0), "at": at,
                }));
            }
            // An agent's player asks their agent; one at their limit is an
            // at_limit notice.
            let q = m.other.get("request").filter(|q| q.is_object());
            if let Some(q) = q.filter(|_| m.status == Status::Member && m.agent_of.is_none())
                && q.get("atLimit").and_then(Value::as_bool) != Some(true)
            {
                let at = num(q.get("at"));
                out.push(json!({
                    "key": format!("chips:{}:{at}", m.id), "kind": "chips", "member": m.id, "name": m.name,
                    "amount": num(q.get("amount")), "note": text(q.get("note")), "table": text(q.get("table")), "at": at,
                }));
            }
        }
        for mut c in self.credit_notices(LEFT_OWING_KEPT) {
            let key = if c["kind"] == "at_limit" {
                format!("at_limit:{}:{}", text(c.get("member")), num(c.get("at")))
            } else if num(c.get("owesNow")) > 0 {
                format!("left_owing:{}", num(c.get("n")))
            } else {
                // Settled since: nothing to say.
                continue;
            };
            c["key"] = key.into();
            out.push(c);
        }
        for h in self.book.club.help.iter().filter(|h| h.handled.is_none()) {
            let watch = members
                .get(&h.from)
                .and_then(|m| m.seat_table())
                .unwrap_or("");
            out.push(json!({
                "key": format!("help:{}", h.id), "kind": "help", "id": h.id, "member": h.from, "name": h.name,
                "text": h.text, "table": h.table, "watch": watch, "at": h.at,
            }));
        }
        for a in self.promos.awards.iter().filter(|a| a.status == "held") {
            out.push(json!({
                "key": format!("prize:{}", a.id), "kind": "prize", "id": a.id, "member": a.member, "name": a.name,
                "amount": a.amount, "promo": a.promo_name, "hand": a.hand.clone().unwrap_or_default(),
                "handName": a.hand_name, "at": a.at,
            }));
        }
        // A table that dealt and has stopped, still open.
        for t in self.book.club.open_tables() {
            let d = t.other.get("dealing");
            let stopped = num(d.and_then(|d| d.get("stoppedAt")));
            if t.status != TableStatus::Open || is_dealing(d) || stopped == 0 {
                continue;
            }
            out.push(json!({
                "key": format!("stopped:{}:{stopped}", t.id), "kind": "stopped", "table": t.id, "name": t.name,
                "seated": num(seated.get(&t.id)), "at": stopped,
            }));
        }
        out.retain(|n| {
            !cleared
                .iter()
                .any(|c| Some(c.as_str()) == n["key"].as_str())
        });
        out.sort_by_key(|n| std::cmp::Reverse(num(n.get("at"))));
        out
    }

    /// The size from which a chip move in the log counts as big (#190): a
    /// full buy-in at the club's biggest open table (its maximum in big
    /// blinds times its big blind), or none with no table open.
    pub fn big_move(&self) -> Option<u64> {
        self.book
            .club
            .open_tables()
            .filter(|t| t.bb > 0)
            .map(|t| t.bb * if t.max_bb > 0 { t.max_bb } else { 100 })
            .max()
    }

    /// Runs Overview op `op` with `args`: "totals", "numbers", "bigMove", "filter"
    /// {filter, agent}, "lastPlayed", "lastActive" {seen}, "tables"
    /// {seated}, "creditNotices" {keep}, "notices" {cleared, seated}.
    pub fn overview(&self, op: &str, args: &Value, now: u64) -> Result<Value, RuleError> {
        let obj = |k: &str| {
            args.get(k)
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default()
        };
        fn to<T: Serialize>(v: &T) -> Value {
            serde_json::to_value(v).unwrap_or(Value::Null)
        }
        Ok(match op {
            "totals" => to(&self.totals()),
            "numbers" => self.overview_numbers(now),
            "bigMove" => to(&self.big_move()),
            "filter" => to(&self.member_filter(
                &text(args.get("filter")),
                now,
                &self.last_played(),
                &text(args.get("agent")),
            )),
            "lastPlayed" => to(&self.last_played()),
            "lastActive" => to(&self.last_active(&obj("seen"), &self.last_played())),
            "tables" => to(&self.table_summaries(&obj("seated"), now)),
            "creditNotices" => {
                let keep = args
                    .get("keep")
                    .and_then(Value::as_u64)
                    .map_or(LEFT_OWING_KEPT, |k| k as usize);
                to(&self.credit_notices(keep))
            }
            "notices" => {
                let cleared: Vec<String> = args
                    .get("cleared")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|c| c.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                to(&self.notices(&cleared, &obj("seated")))
            }
            _ => return refuse(format!("no overview op {op}")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club_book::Limits;

    const NOW: u64 = 1_800_000_000_000;
    const MIN: u64 = 60_000;

    // Ann, Ben (joined 2 days ago), Cy and Dee are members; Eve asks to join.
    fn club() -> ClubState {
        let joined = NOW - 60 * DAY;
        let mut c: ClubState = serde_json::from_value(json!({
            "code": "AAAA-BBBB", "hostId": "host", "tables": [],
            "members": {
                "ann": {"id": "ann", "name": "Ann", "status": "member", "joinedAt": joined},
                "ben": {"id": "ben", "name": "Ben", "status": "member", "joinedAt": NOW - 2 * DAY},
                "cy": {"id": "cy", "name": "Cy", "status": "member", "joinedAt": joined},
                "dee": {"id": "dee", "name": "Dee", "status": "member", "joinedAt": joined},
                "eve": {"id": "eve", "name": "Eve", "status": "pending", "requestedAt": NOW - 5 * MIN},
            },
        }))
        .unwrap();
        c.book.ensure_links(0).unwrap();
        c.book.sync();
        c
    }

    fn table(c: &mut ClubState, id: &str, dealing: Value) {
        let mut t: crate::club_rules::Table = serde_json::from_value(json!({
            "id": id, "name": id.to_uppercase(), "game": "nlhe", "sb": 100, "bb": 200, "minBB": 40, "maxBB": 100,
            "seats": 9, "status": "open",
        }))
        .unwrap();
        t.other.insert("dealing".into(), dealing);
        c.book.club.tables.push(t);
    }

    fn limits() -> Limits {
        Limits {
            min: 8000,
            max: 20_000,
        }
    }

    #[test]
    fn the_dealing_clock_runs_only_while_dealing() {
        let c = deal_clock(&Value::Null, false, NOW);
        assert_eq!(c, json!({"ms": 0, "since": null}));
        let c = deal_clock(&c, true, NOW);
        assert!(is_dealing(Some(&c)));
        assert_eq!(dealing_ms(Some(&c), NOW + 30 * MIN), 30 * MIN);
        assert_eq!(deal_clock(&c, true, NOW + 10 * MIN), c);
        let c = deal_clock(&c, false, NOW + 40 * MIN);
        assert_eq!(
            c,
            json!({"ms": 40 * MIN, "since": null, "stoppedAt": NOW + 40 * MIN})
        );
        assert_eq!(dealing_ms(Some(&c), NOW + 120 * MIN), 40 * MIN);
        let c = deal_clock(&c, true, NOW + 60 * MIN);
        assert_eq!(dealing_ms(Some(&c), NOW + 75 * MIN), 55 * MIN);
    }

    #[test]
    fn the_numbers_the_filters_and_the_tables() {
        let mut c = club();
        table(&mut c, "t1", deal_clock(&Value::Null, true, NOW - 47 * MIN));
        let stopped = deal_clock(
            &deal_clock(&Value::Null, true, NOW - 60 * MIN),
            false,
            NOW - 18 * MIN,
        );
        table(&mut c, "t2", stopped);
        c.book
            .issue("ann", Some(100_000), "", NOW - 40 * DAY)
            .unwrap();
        c.book
            .issue("cy", Some(50_000), "", NOW - 40 * DAY)
            .unwrap();
        c.book
            .chips_for_table("cy", "t1", Some(20_000), limits(), NOW - 10 * DAY)
            .unwrap();
        c.book.cash_out("cy", NOW - 10 * DAY).unwrap();
        c.book
            .set_credit("dee", Some(20_000), NOW - 2 * DAY)
            .unwrap();
        c.book
            .chips_for_table("dee", "t1", Some(20_000), limits(), NOW - DAY)
            .unwrap();
        c.book.money.set_stack("dee", 5000).unwrap();
        if let Some(s) = c
            .book
            .club
            .members
            .get_mut("dee")
            .unwrap()
            .other
            .get_mut("seat")
        {
            s["stack"] = 5000.into();
        }
        c.book.issue("ann", Some(15_000), "", NOW - DAY).unwrap();
        c.book.cash_out("dee", NOW - DAY).unwrap();
        assert_eq!(
            c.overview_numbers(NOW),
            json!({"tablesActive": 1, "tables": 2, "chipsOut": 165_000, "members": 4, "joined": 1, "inactive": 1,
                   "owedToClub": 15_000, "owedToPlayers": 165_000})
        );
        // A big move is a full buy-in at the biggest open table.
        assert_eq!(c.big_move(), Some(200 * 100));
        let played = c.last_played();
        assert_eq!(played.get("cy"), Some(&(NOW - 10 * DAY)));
        assert_eq!(played.get("dee"), Some(&(NOW - DAY)));
        assert_eq!(c.member_filter("new", NOW, &played, ""), ["ben"]);
        assert_eq!(c.member_filter("inactive", NOW, &played, ""), ["ann"]);
        assert_eq!(c.member_filter("owes", NOW, &played, ""), ["dee"]);
        assert_eq!(c.member_filter("all", NOW, &played, "").len(), 4);
        let rows = c.table_summaries(&json!({"t1": 6}).as_object().unwrap().clone(), NOW);
        assert_eq!(
            (
                rows[0]["dealing"].clone(),
                rows[0]["ms"].clone(),
                rows[0]["taken"].clone()
            ),
            (json!(true), json!(47 * MIN), json!(6))
        );
        assert_eq!(
            (rows[1]["ms"].clone(), rows[1]["stoppedAt"].clone()),
            (json!(42 * MIN), json!(NOW - 18 * MIN))
        );
        // Last active: heard from, played, or joined.
        let seen = json!({"cy": NOW - 5 * MIN, "ann": "not a time"});
        let active = c.last_active(seen.as_object().unwrap(), &played);
        assert_eq!(active["cy"], NOW - 5 * MIN);
        assert_eq!(active["dee"], NOW - DAY);
        assert_eq!(active["ben"], NOW - 2 * DAY);
    }

    #[test]
    fn agents_and_their_players() {
        let mut c = club();
        c.book.club.members.get_mut("ann").unwrap().agent = Some("yes".into());
        for id in ["ben", "cy", "eve"] {
            c.book.club.members.get_mut(id).unwrap().agent_of = Some("ann".into());
        }
        let p = BTreeMap::new();
        assert_eq!(c.member_filter("agents", NOW, &p, ""), ["ann"]);
        assert_eq!(c.member_filter("agent", NOW, &p, "ann"), ["ben", "cy"]);
        assert_eq!(c.member_filter("noagent", NOW, &p, ""), ["ann", "dee"]);
        c.book.club.members.get_mut("ann").unwrap().agent = None;
        assert_eq!(c.member_filter("noagent", NOW, &p, "").len(), 4);
    }

    #[test]
    fn notices_newest_first_and_cleared() {
        let mut c = club();
        table(
            &mut c,
            "t1",
            json!({"ms": 30 * MIN, "since": null, "stoppedAt": NOW - 30_000}),
        );
        c.book.issue("ann", Some(10_000), "", NOW - DAY).unwrap();
        c.book
            .request_chips("ann", Some(50_000), "for tonight", "", NOW - 4 * MIN)
            .unwrap();
        c.book.set_credit("dee", Some(20_000), NOW - DAY).unwrap();
        c.book
            .chips_for_table("dee", "t1", Some(20_000), limits(), NOW - 20 * MIN)
            .unwrap();
        c.book.money.set_stack("dee", 0).unwrap();
        if let Some(s) = c
            .book
            .club
            .members
            .get_mut("dee")
            .unwrap()
            .other
            .get_mut("seat")
        {
            s["stack"] = 0.into();
        }
        c.book
            .issue("ann", Some(20_000), "", NOW - 20 * MIN)
            .unwrap();
        c.book.cash_out("dee", NOW - 10_000).unwrap();
        let none = Map::new();
        let list = c.notices(&[], &none);
        let kinds: Vec<&str> = list.iter().map(|n| n["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["left_owing", "stopped", "chips", "join"]);
        assert_eq!(list[0]["owesNow"], 20_000);
        assert_eq!(list[2]["note"], "for tonight");
        assert_eq!(list[3]["member"], "eve");
        // Cleared ones stay away.
        let cleared: Vec<String> = list[..2]
            .iter()
            .map(|n| n["key"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(c.notices(&cleared, &none).len(), 2);
        // A debt settled says nothing.
        c.book.issue("dee", Some(20_000), "Cash", NOW).unwrap();
        assert!(
            !c.notices(&[], &none)
                .iter()
                .any(|n| n["kind"] == "left_owing")
        );
        assert_eq!(
            c.overview("nope", &Value::Null, NOW).unwrap_err().0,
            "no overview op nope"
        );
    }
}
