//! What the club's pages show (ducy-web #459), as plain data for the page
//! to draw: the ledger's entries as people see them, table sessions and
//! what each won or lost, a member's chip history, what a backer settles,
//! and an agent's view of their players.
//!
//! The session and history views work on entries as JSON, so the same code
//! reads the club's whole ledger and a member's own signed copy of theirs.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Value, json};

use crate::host::ClubState;
use crate::money::{CLUB, EntryKind};
use crate::rules::{RuleError, Status, refuse};

fn int(v: Option<&Value>) -> i64 {
    v.and_then(|x| x.as_i64().or_else(|| x.as_f64().map(|f| f as i64)))
        .unwrap_or(0)
}

fn text(v: Option<&Value>) -> &str {
    v.and_then(Value::as_str).unwrap_or("")
}

fn kind(e: &Value) -> &str {
    text(e.get("kind"))
}

/// An entry on someone else's link for a player's move (an agent's).
fn for_player(e: &Value) -> bool {
    e.get("player").is_some_and(|p| !p.is_null() && p != "")
}

/// A stretch at one table: from a buy-in to the cash-out.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Session {
    /// The member.
    pub member: String,
    /// The table.
    pub table: String,
    /// When they bought in.
    pub start: u64,
    /// When they left; none while they're there.
    pub end: Option<u64>,
    /// What they brought to it in all.
    pub put: i64,
    /// What they left with.
    pub took: i64,
    /// What they won (or, below zero, lost).
    pub net: i64,
    /// Still at the table.
    pub open: bool,
}

fn sorted(entries: &[Value]) -> Vec<&Value> {
    let mut s: Vec<&Value> = entries.iter().collect();
    s.sort_by_key(|e| int(e.get("n")));
    s
}

/// The table sessions in `entries` (a member's own, or the whole ledger),
/// newest first. A top-up the table handed back comes off what they put
/// in; a refund about anything else is a tournament's.
pub fn table_sessions(entries: &[Value]) -> Vec<Session> {
    let mut open: BTreeMap<String, usize> = BTreeMap::new();
    let mut out: Vec<Session> = vec![];
    for e in sorted(entries) {
        let k = kind(e);
        if for_player(e) || !["buy_in", "top_up", "refund", "cash_out"].contains(&k) {
            continue;
        }
        let member = text(e.get("member")).to_string();
        let amount = int(e.get("amount"));
        let about = text(e.get("about"));
        if k == "refund" {
            if let Some(&i) = open.get(&member)
                && out[i].table == about
            {
                out[i].put -= amount;
                out[i].net = out[i].took - out[i].put;
            }
            continue;
        }
        let i = match open.get(&member) {
            Some(&i) if k != "buy_in" => i,
            _ => {
                out.push(Session {
                    member: member.clone(),
                    table: about.into(),
                    start: int(e.get("at")) as u64,
                    open: true,
                    ..Session::default()
                });
                open.insert(member.clone(), out.len() - 1);
                out.len() - 1
            }
        };
        let s = &mut out[i];
        if k == "cash_out" {
            s.took = amount;
            s.end = Some(int(e.get("at")) as u64);
            s.open = false;
            open.remove(&member);
        } else {
            s.put -= amount;
        }
        s.net = s.took - s.put;
    }
    out.reverse();
    out
}

/// Each member's net over the sessions they've finished.
pub fn totals_by_member(sessions: &[Session]) -> BTreeMap<String, i64> {
    let mut t = BTreeMap::new();
    for s in sessions.iter().filter(|s| !s.open) {
        *t.entry(s.member.clone()).or_insert(0) += s.net;
    }
    t
}

/// A member's chip history from their own entries, newest first: a table
/// session is one line, `left` (what they won or lost, the balance after,
/// what they still owe) or `at` (still there: what they've brought, the
/// balance now); every other entry is a line of its own. Each line has
/// its `entry`.
pub fn chip_lines(entries: &[Value]) -> Vec<Value> {
    let entries = sorted(entries);
    let owned: Vec<Value> = entries.iter().map(|e| (*e).clone()).collect();
    let mut sessions = table_sessions(&owned);
    sessions.reverse();
    let mut out: Vec<Value> = vec![];
    let (mut next, mut last): (usize, Option<usize>) = (0, None);
    let mut at_line: Option<usize> = None;
    for e in entries {
        let k = kind(e);
        let of_session = ["buy_in", "top_up", "refund", "cash_out"].contains(&k)
            && !for_player(e)
            && (k != "refund" || last.is_some_and(|l| sessions[l].table == text(e.get("about"))));
        if !of_session {
            out.push(json!({
                "kind": k, "amount": e.get("amount"), "balance": e.get("balance"),
                "note": text(e.get("note")), "at": e.get("at"), "entry": e,
            }));
            continue;
        }
        if k == "buy_in" || last.is_none() {
            last = (next < sessions.len()).then_some(next);
            next += 1;
            at_line = None;
        }
        let Some(l) = last else { continue };
        let s = &sessions[l];
        if k == "cash_out" {
            let balance = int(e.get("balance"));
            out.push(json!({
                "kind": "left", "table": text(e.get("about")), "amount": s.net, "balance": balance,
                "at": e.get("at"), "owes": (-balance).max(0), "entry": e,
            }));
            last = None;
            at_line = None;
        } else if s.open {
            let line = json!({
                "kind": "at", "table": s.table, "amount": -s.put, "balance": e.get("balance"),
                "at": e.get("at"), "entry": e,
            });
            match at_line {
                Some(i) => out[i] = line,
                None => {
                    out.push(line);
                    at_line = Some(out.len() - 1);
                }
            }
        }
    }
    out.reverse();
    out
}

/// After standing up (entry `n`, a cash-out): the table, the result there,
/// the balance after it and what's still owed.
pub fn left_summary(entries: &[Value], n: i64) -> Value {
    chip_lines(entries)
        .into_iter()
        .find(|l| l["kind"] == "left" && int(l["entry"].get("n")) == n)
        .map_or(Value::Null, |l| {
            json!({"table": l["table"], "net": l["amount"], "balance": l["balance"], "owes": l["owes"]})
        })
}

impl ClubState {
    /// An entry as pages and members' copies have it: no empty note or
    /// about; `player` when it's on someone else's link for a player's move
    /// along their path; for a promotion prize or its void, the award's
    /// `hand` and a note naming it.
    pub fn entry_view(&self, e: &crate::money::Entry) -> Value {
        let mut out = serde_json::to_value(e).unwrap_or(Value::Null);
        let Some(o) = out.as_object_mut() else {
            return out;
        };
        for k in ["note", "about"] {
            if o.get(k).is_none_or(|v| v.is_null() || v == "") {
                o.remove(k);
            }
        }
        let ledger = self.book.money.ledger();
        let at = (e.n as usize).checked_sub(1);
        if e.kind.is_path_move() && at.and_then(|i| ledger.get(i)).is_some_and(|x| x.n == e.n) {
            let mut i = at.unwrap_or(0);
            while i > 0 && ledger[i - 1].tx == e.tx {
                i -= 1;
            }
            if ledger[i].member != e.member {
                o.insert("player".into(), ledger[i].member.clone().into());
            }
        }
        if matches!(e.kind, EntryKind::Prize | EntryKind::Void) {
            let award = self.promos.awards.iter().find(|a| {
                a.entry == Some(e.n)
                    || a.other.get("voidEntry").and_then(Value::as_u64) == Some(e.n)
            });
            if let Some(a) = award {
                if let Some(h) = &a.hand {
                    o.insert("hand".into(), h.clone().into());
                }
                let note = if e.kind == EntryKind::Void {
                    let why = text(a.other.get("voidWhy"));
                    if why.is_empty() {
                        format!("Voided: {}", a.promo_name)
                    } else {
                        format!("Voided: {}, {why}", a.promo_name)
                    }
                } else {
                    format!("{}: {}", a.promo_name, a.hand_name)
                };
                o.insert("note".into(), note.into());
            }
        }
        out
    }

    /// Member `id`'s own entries (on their links up) after entry `since`,
    /// as they see them.
    pub fn entries_of(&self, id: &str, since: u64) -> Vec<Value> {
        self.book
            .money
            .ledger()
            .iter()
            .filter(|e| e.member == id && e.n > since)
            .map(|e| self.entry_view(e))
            .collect()
    }

    /// The whole ledger as pages see it.
    pub fn ledger_view(&self) -> Vec<Value> {
        self.book
            .money
            .ledger()
            .iter()
            .map(|e| self.entry_view(e))
            .collect()
    }

    /// What `backer` settles with those below them (the Settle page): each
    /// of their links with a balance, largest first; below zero, the member
    /// owes the backer. An agent's link carries their players' play too.
    pub fn settle_rows(&self, backer: &str) -> Vec<Value> {
        let mut rows: Vec<(&str, i64)> = self
            .book
            .money
            .links()
            .iter()
            .filter(|l| l.backer == backer && l.balance != 0)
            .map(|l| (l.member.as_str(), l.balance))
            .collect();
        rows.sort_by(|a, b| b.1.abs().cmp(&a.1.abs()).then(a.0.cmp(b.0)));
        rows.into_iter()
            .map(|(m, b)| json!({"member": m, "balance": b}))
            .collect()
    }

    /// Member `id`'s result at the tables: what they won or lost over the
    /// sessions they've finished.
    pub fn tables_result(&self, id: &str) -> i64 {
        let mine = self.entries_of(id, 0);
        totals_by_member(&table_sessions(&mine))
            .get(id)
            .copied()
            .unwrap_or(0)
    }

    fn players_of(&self, id: &str, status: Status) -> Vec<&crate::rules::Member> {
        self.book
            .club
            .members
            .values()
            .filter(|m| m.agent_of.as_deref() == Some(id) && m.status == status)
            .collect()
    }

    /// An agent's account: their own chips, their players' (shares), the
    /// whole link, their limit, credit used, room, and their players' ids.
    pub fn account_of(&self, id: &str) -> Value {
        let players = self.players_of(id, Status::Member);
        let shares: i64 = players.iter().map(|p| self.book.balance(&p.id)).sum();
        let total = self.book.balance(id);
        json!({
            "own": total - shares, "shares": shares, "total": total, "limit": self.book.credit(id),
            "creditUsed": (-total).max(0), "room": self.book.available(id),
            "players": players.iter().map(|p| p.id.clone()).collect::<Vec<_>>(),
        })
    }

    fn seat_name(&self, table: &str) -> String {
        if let Some(t) = table.strip_prefix("tourney:") {
            return self
                .tourneys
                .iter()
                .find(|x| x.id == t)
                .map_or("A tournament".into(), |x| x.name.clone());
        }
        self.book
            .club
            .table(table)
            .map_or("A table".into(), |t| t.name.clone())
    }

    /// What agent `id` sees: their account, what they'd owe the club if
    /// their players stood up now (credit used less their players' chips at
    /// the tables), their invite, recruits waiting, and each player with
    /// their balance, request, seat and result at the tables.
    pub fn agent_body(&self, id: &str) -> Result<Value, RuleError> {
        let Some(me) = self.book.club.members.get(id) else {
            return refuse("No such member.");
        };
        let a = self.account_of(id);
        let players = self.players_of(id, Status::Member);
        let total = int(a.get("total"));
        let at_tables: i64 = players
            .iter()
            .filter_map(|p| p.seat())
            .map(|s| int(s.get("stack")))
            .sum();
        let list: Vec<Value> = players
            .iter()
            .map(|m| {
                let mut p = json!({
                    "id": m.id, "name": m.name, "balance": self.book.balance(&m.id),
                    "request": m.other.get("request").filter(|r| !r.is_null()).cloned().unwrap_or(Value::Null),
                    "seated": m.seat().is_some(), "result": self.tables_result(&m.id),
                });
                if let Some(s) = m.seat() {
                    p["table"] = self.seat_name(text(s.get("table"))).into();
                    p["stack"] = int(s.get("stack")).into();
                }
                p
            })
            .collect();
        let recruits: Vec<Value> = self
            .players_of(id, Status::Pending)
            .iter()
            .map(|m| json!({"id": m.id, "name": m.name, "at": m.requested_at}))
            .collect();
        Ok(json!({
            "type": "agent", "club": self.book.club.code,
            "own": a["own"], "shares": a["shares"], "total": total, "limit": a["limit"],
            "creditUsed": a["creditUsed"], "room": a["room"],
            "owed": (-(total + at_tables)).max(0),
            "invite": me.agent_invite, "recruits": recruits, "players": list,
        }))
    }

    /// Runs view op `op` with `args`: "entriesOf" {id, since}, "ledger",
    /// "entryViews" {ns} (those entries, by number), "settleRows"
    /// {backer}, "accountOf" {id}, "tableResult" {id}, "agentBody" {id}.
    pub fn view(&self, op: &str, args: &Value) -> Result<Value, RuleError> {
        let id = text(args.get("id"));
        Ok(match op {
            "entriesOf" => self
                .entries_of(id, args.get("since").and_then(Value::as_u64).unwrap_or(0))
                .into(),
            "ledger" => self.ledger_view().into(),
            "entryViews" => {
                let ns: Vec<u64> = args
                    .get("ns")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_u64).collect())
                    .unwrap_or_default();
                let ledger = self.book.money.ledger();
                ns.iter()
                    .filter_map(|n| {
                        ledger
                            .get((*n as usize).checked_sub(1)?)
                            .filter(|e| e.n == *n)
                    })
                    .map(|e| self.entry_view(e))
                    .collect::<Vec<_>>()
                    .into()
            }
            "settleRows" => {
                let b = args.get("backer").and_then(Value::as_str).unwrap_or(CLUB);
                self.settle_rows(b).into()
            }
            "accountOf" => self.account_of(id),
            "tableResult" => self.tables_result(id).into(),
            "agentBody" => self.agent_body(id)?,
            _ => return refuse(format!("no view op {op}")),
        })
    }
}

/// The entries' views as a map, for tests.
#[cfg(test)]
fn by_kind(lines: &[Value]) -> Vec<String> {
    lines
        .iter()
        .map(|l| l["kind"].as_str().unwrap_or("").to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(n: i64, kind: &str, amount: i64, balance: i64, about: &str) -> Value {
        json!({"n": n, "at": n * 10, "tx": n, "member": "ann", "backer": "club", "kind": kind,
               "amount": amount, "balance": balance, "about": about})
    }

    #[test]
    fn sessions_and_the_chip_history() {
        let entries = vec![
            e(1, "give", 1000, 1000, ""),
            e(2, "buy_in", -200, 800, "t1"),
            e(3, "top_up", -100, 700, "t1"),
            // A top-up handed back.
            e(4, "refund", 50, 750, "t1"),
            e(5, "cash_out", 500, 1250, "t1"),
            e(6, "buy_in", -300, 950, "t2"),
        ];
        let s = table_sessions(&entries);
        assert_eq!(s.len(), 2);
        // Newest first: still at t2.
        assert!(s[0].open && s[0].table == "t2" && s[0].put == 300);
        assert_eq!(
            (s[1].put, s[1].took, s[1].net, s[1].end),
            (250, 500, 250, Some(50))
        );
        assert_eq!(totals_by_member(&s).get("ann"), Some(&250));
        let lines = chip_lines(&entries);
        assert_eq!(by_kind(&lines), ["at", "left", "give"]);
        assert_eq!(lines[0]["amount"], -300);
        assert_eq!(lines[1]["amount"], 250);
        assert_eq!(
            left_summary(&entries, 5),
            json!({"table": "t1", "net": 250, "balance": 1250, "owes": 0})
        );
        assert_eq!(left_summary(&entries, 2), Value::Null);
        // An agent's entries for a player's move aren't a session of theirs.
        let mut theirs = e(7, "buy_in", -100, 850, "t3");
        theirs["player"] = "bob".into();
        assert_eq!(table_sessions(&[theirs]).len(), 0);
    }

    fn club() -> ClubState {
        let mut c: ClubState = serde_json::from_value(json!({
            "code": "ABCD-EFGH", "hostId": "HOST",
            "members": {
                "al": {"id": "al", "name": "Al", "status": "member", "agent": "yes", "agentInvite": "INV"},
                "bo": {"id": "bo", "name": "Bo", "status": "member", "agentOf": "al"},
                "cy": {"id": "cy", "name": "Cy", "status": "pending", "agentOf": "al", "requestedAt": 7},
                "di": {"id": "di", "name": "Di", "status": "member"},
            },
            "tables": [{"id": "t1", "name": "Main", "game": "nlhe", "sb": 1, "bb": 2, "minBB": 20, "maxBB": 100, "seats": 6, "status": "open"}],
        }))
        .unwrap();
        c.book.ensure_links(0).unwrap();
        c.book.sync();
        c
    }

    #[test]
    fn settle_rows_entries_and_the_agent_view() {
        let mut c = club();
        c.book.issue("al", Some(1000), "", 1).unwrap();
        c.book.issue("di", Some(300), "", 1).unwrap();
        c.book.backer_give("al", "bo", Some(400), "", 2).unwrap();
        c.book
            .chips_for_table(
                "bo",
                "t1",
                Some(200),
                crate::book::Limits { min: 40, max: 200 },
                3,
            )
            .unwrap();
        // The club settles with Al (his whole tree) and Di.
        assert_eq!(
            c.settle_rows(CLUB),
            vec![
                json!({"member": "al", "balance": 800}),
                json!({"member": "di", "balance": 300})
            ]
        );
        // Bo's buy-in also moved Al's link: on Al's, it's marked as Bo's.
        let al = c.entries_of("al", 0);
        assert!(
            al.iter()
                .any(|x| x["kind"] == "buy_in" && x["player"] == "bo")
        );
        assert!(
            c.entries_of("bo", 0)
                .iter()
                .all(|x| x.get("player").is_none())
        );
        let v = c.agent_body("al").unwrap();
        assert_eq!(v["recruits"], json!([{"id": "cy", "name": "Cy", "at": 7}]));
        assert_eq!(v["players"][0]["table"], "Main");
        assert_eq!(v["players"][0]["stack"], 200);
        assert_eq!(v["invite"], "INV");
        assert_eq!(v["total"], c.book.balance("al"));
        assert!(c.agent_body("nobody").is_err());
        assert_eq!(c.view("tableResult", &json!({"id": "bo"})).unwrap(), 0);
    }
}
