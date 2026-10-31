//! A club's host (ducy-web #456): the manager's commands, over the whole
//! club (its [`Book`], tournaments and promotions), with what each one
//! means for everyone else: replies to send, accounts to update, and what
//! to tell members again.
//!
//! The host keeps no I/O: the caller signs and sends, saves, and runs the
//! tables and tournaments' clocks. Commands that drive a running table or
//! tournament (opening and closing tables, moving players, starting a
//! tournament) stay with the caller's table hosts.

use rand::rngs::StdRng;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value, json};

use crate::club_book::{AccountTo, Book};
use crate::club_groups::MustMove;
use crate::club_money::{CLUB, EntryKind};
use crate::club_promos::{PromoState, Promos};
use crate::club_rules::{RuleError, refuse};
use crate::club_tourneys::{PayoutTable, Tourney, Tourneys};

/// The whole club the host keeps, as ducy-web's JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClubState {
    /// Its rules and money.
    #[serde(flatten)]
    pub book: Book,
    /// Its tournaments.
    #[serde(default)]
    pub tourneys: Vec<Tourney>,
    /// Its payout tables.
    #[serde(default)]
    pub payout_tables: Vec<PayoutTable>,
    /// Its promotions.
    #[serde(default)]
    pub promos: PromoState,
    /// How many days the host keeps hand histories.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_days: Option<Number>,
}

/// Something the members online should hear again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Tell {
    /// Who's online and the chat.
    Lobby,
    /// The tables.
    Tables,
    /// The tournaments.
    Tourneys,
    /// The promotions.
    Promos,
    /// The club info.
    Info,
}

/// What a command did, for the caller to carry out.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    /// What the command returns to whoever ran it.
    pub value: Value,
    /// Status replies to sign and send: {to, body}.
    pub replies: Vec<Value>,
    /// Accounts to send.
    pub accounts: Vec<AccountTo>,
    /// What members online should hear again.
    pub tell: Vec<Tell>,
    /// Members to send the tables to (just let in).
    pub tables_to: Vec<String>,
    /// Members whose help requests changed.
    pub help_to: Vec<String>,
    /// Promotions changed: re-arm the timer for the next period's end.
    pub arm_promos: bool,
    /// New tournaments: time their starts.
    pub schedule: Vec<String>,
    /// Tournaments cancelled: stop their tables.
    pub stop_tourneys: Vec<String>,
    /// Members whose waitlists changed: send them theirs.
    pub lists_to: Vec<String>,
}

impl Outcome {
    fn account(&mut self, id: &str, since: Option<u64>) {
        self.accounts.push(AccountTo {
            id: id.into(),
            since,
        });
    }
}

impl ClubState {
    /// Member `id`'s account, from their latest ledger entry.
    fn account_latest(&self, id: &str, out: &mut Outcome) {
        let n = self
            .book
            .money
            .ledger()
            .iter()
            .rev()
            .find(|e| e.member == id)
            .map(|e| e.n - 1);
        out.account(id, n);
    }
}

fn num(v: &Value, k: &str) -> Option<i64> {
    let x = v.get(k)?;
    x.as_i64().or_else(|| {
        x.as_f64()
            .filter(|f| f.fract() == 0.0 && f.abs() <= 9e15)
            .map(|f| f as i64)
    })
}

fn text(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// A host setting and whether `v` will do for it.
fn setting_ok(k: &str, v: &Value) -> bool {
    let int = |lo: i64, hi: i64| v.as_i64().is_some_and(|n| (lo..=hi).contains(&n));
    match k {
        "ratholeHours" => int(0, 24 * 30),
        "sitOutMins" => int(0, 240),
        // Call time (#476): new tables' hands to play once up, before leaving.
        "stayHands" => int(0, 500),
        "keepDays" => int(1, 3650),
        // Let people ask to join with the club code, no invite link (#393).
        "codeJoin" => v.is_boolean(),
        _ => false,
    }
}

impl ClubState {
    fn actor(&self) -> String {
        self.book
            .club
            .owner_id
            .clone()
            .unwrap_or_else(|| self.book.club.host_id.clone())
    }

    pub(crate) fn with_tourneys<T>(
        &mut self,
        f: impl FnOnce(&mut Tourneys) -> Result<T, RuleError>,
    ) -> Result<T, RuleError> {
        let mut t = Tourneys {
            code: self.book.club.code.clone(),
            members: self.book.club.members.clone(),
            tourneys: std::mem::take(&mut self.tourneys),
            payout_tables: std::mem::take(&mut self.payout_tables),
        };
        let r = f(&mut t);
        self.tourneys = t.tourneys;
        self.payout_tables = t.payout_tables;
        r
    }

    fn with_promos<T>(
        &mut self,
        f: impl FnOnce(&mut Promos) -> Result<T, RuleError>,
    ) -> Result<T, RuleError> {
        let mut p = Promos {
            code: self.book.club.code.clone(),
            members: self.book.club.members.clone(),
            promos: std::mem::take(&mut self.promos),
        };
        let r = f(&mut p);
        self.promos = p.promos;
        r
    }

    fn table_name(&self, id: &str) -> String {
        self.payout_tables
            .iter()
            .find(|p| p.id == id)
            .map_or("Default".into(), |p| p.name.clone())
    }

    /// Removes a member: their backer takes back their whole balance first,
    /// and any open chip request goes. None if they can't be removed now
    /// (seated, or not a member or asking).
    fn remove(&mut self, id: &str, now: u64, out: &mut Outcome) -> Result<(), RuleError> {
        if !self.book.club.removable(id) {
            return Ok(());
        }
        let bal = self.book.balance(id);
        if bal > 0 {
            let backer = self
                .book
                .money
                .account(id)
                .and_then(|a| a.through.clone())
                .unwrap_or_else(|| CLUB.into());
            if backer == CLUB {
                self.book
                    .reclaim(id, Some(bal), "Removed from the club", now)?;
            } else {
                self.book
                    .backer_take(&backer, id, Some(bal), "Removed from the club", now)?;
            }
        }
        if let Some(m) = self.book.club.members.get_mut(id) {
            m.other.insert("request".into(), Value::Null);
        }
        out.replies.extend(self.book.club.remove(id, now));
        if bal > 0 {
            out.account(id, Some(0));
        }
        out.tell.push(Tell::Lobby);
        Ok(())
    }

    /// Pays a promotion award the rules marked paid: up the winner's path.
    fn pay_award(&mut self, id: &str, now: u64, out: &mut Outcome) -> Result<(), RuleError> {
        let Some(i) = self.promos.awards.iter().position(|a| a.id == id) else {
            return Ok(());
        };
        let a = self.promos.awards[i].clone();
        if a.status != "paid" || a.entry.is_some() {
            return Ok(());
        }
        self.book.ensure_links(now)?;
        let entries = self
            .book
            .money
            .pay(&a.member, EntryKind::Prize, &a.promo, a.amount, now)
            .map_err(|e| RuleError(e.to_string()))?;
        self.book.chips.issued += a.amount as i64;
        self.book.sync();
        let n = entries.first().map(|e| e.n);
        self.promos.awards[i].entry = n;
        out.account(&a.member, n.map(|n| n - 1));
        Ok(())
    }

    /// Runs a manager's command `op` with its arguments; what it did, or
    /// why it can't be done.
    pub fn run(
        &mut self,
        op: &str,
        args: &Value,
        now: u64,
        rng: &mut StdRng,
    ) -> Result<Outcome, RuleError> {
        let mut out = Outcome::default();
        let id = text(args, "id");
        let actor = self.actor();
        match op {
            "approve" | "decline" => {
                let reply = if op == "approve" {
                    self.book.club.approve(&id, now)
                } else {
                    self.book.club.decline(&id, now)
                };
                let Some(reply) = reply else { return Ok(out) };
                out.replies.push(reply);
                if op == "approve" {
                    self.book.ensure_links(now)?;
                    self.book.sync();
                    out.tables_to.push(id);
                }
                out.tell.push(Tell::Lobby);
            }
            "remove" => self.remove(&id, now, &mut out)?,
            "revoke_invite" => self.book.club.revoke_invite(rng),
            "give" | "take" => {
                let amt = num(args, "amount");
                let note = text(args, "note");
                let e = if op == "give" {
                    self.book.issue(&id, amt, &note, now)?
                } else {
                    self.book.reclaim(&id, amt, &note, now)?
                };
                out.account(&id, Some(e.n - 1));
            }
            "grant" => {
                let e = self.book.grant_request(&id, num(args, "amount"), now)?;
                out.account(&id, Some(e.n - 1));
            }
            "deny" => {
                self.book
                    .decline_request(&id, &text(args, "note"), "", now)?;
                out.account(&id, None);
            }
            // Credit players (#404): a limit (0: none), and raising it to
            // answer a request at the limit.
            "set_credit" => {
                self.book
                    .set_credit(&id, num(args, "amount").or(Some(-1)), now)?;
                out.account(&id, None);
            }
            "raise_credit" => {
                self.book
                    .raise_credit(&id, num(args, "amount").or(Some(-1)), now)?;
                out.account(&id, None);
            }
            "offer_agent" => out.replies.push(self.book.offer_agent(&id)?),
            "revoke_agent" => out.replies.extend(self.book.revoke_agent(&id, now)?),
            "assign_agent" => {
                let agent = args
                    .get("agent")
                    .and_then(Value::as_str)
                    .filter(|a| !a.is_empty());
                self.book.assign_to_agent(&id, agent, now)?;
            }
            // Answering a help request (#192): a reply, and/or handled.
            "help_answer" => {
                let handled = args.get("handled").and_then(Value::as_bool).unwrap_or(true);
                let h =
                    self.book
                        .club
                        .answer_help(&actor, &id, &text(args, "reply"), handled, now)?;
                out.help_to.push(h.from.clone());
            }
            "settings" => {
                let Some(values) = args.as_object() else {
                    return Ok(out);
                };
                for (k, v) in values {
                    if !setting_ok(k, v) {
                        return refuse(format!("Not a valid {k}."));
                    }
                }
                for (k, v) in values {
                    match k.as_str() {
                        "ratholeHours" => self.book.club.rathole_hours = v.as_number().cloned(),
                        "sitOutMins" => self.book.club.sit_out_mins = v.as_number().cloned(),
                        "stayHands" => self.book.club.stay_hands = v.as_number().cloned(),
                        "keepDays" => self.keep_days = v.as_number().cloned(),
                        "codeJoin" => self.book.club.code_join = v.as_bool(),
                        _ => {}
                    }
                }
            }
            // Club info (#398), sent to every member online.
            "set_info" => {
                let info = args
                    .get("info")
                    .filter(|i| i.is_object())
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                self.book.club.set_info(&info, now);
                self.book
                    .log_action(&actor, "Updated the club info", now, None);
                out.tell.push(Tell::Info);
            }
            // A tournament's own payouts: {rows}, a list of shares, or null.
            "set_payout" => {
                let payout = match (
                    args.get("rows").filter(|r| !r.is_null()),
                    args.get("places"),
                ) {
                    (Some(rows), _) => json!({"rows": rows}),
                    (None, Some(p)) => p.clone(),
                    (None, None) => Value::Null,
                };
                self.with_tourneys(|t| t.set_payout(&id, &payout))?;
                out.tell.push(Tell::Tourneys);
            }
            // Club payout tables by entries (#424).
            "add_payout_table" => {
                let s = args.get("table").cloned().unwrap_or_else(|| json!({}));
                let p = self.with_tourneys(|t| t.add_payout_table(&s, rng))?;
                self.book.log_action(
                    &actor,
                    &format!("Added the payout table {}", p.name),
                    now,
                    None,
                );
                out.value = p.id.into();
            }
            "edit_payout_table" => {
                let was = self
                    .payout_tables
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| p.name.clone());
                let s = args.get("table").cloned().unwrap_or_else(|| json!({}));
                let p = self.with_tourneys(|t| t.edit_payout_table(&id, &s))?;
                let line = match was {
                    Some(w) if w != p.name => format!("Renamed the payout table {w} to {}", p.name),
                    _ => format!("Changed the payout table {}", p.name),
                };
                self.book.log_action(&actor, &line, now, None);
                out.tell.push(Tell::Tourneys);
            }
            "delete_payout_table" => {
                let p = self.with_tourneys(|t| t.delete_payout_table(&id))?;
                self.book.log_action(
                    &actor,
                    &format!("Deleted the payout table {}", p.name),
                    now,
                    None,
                );
            }
            "set_payout_table" => {
                let table = args
                    .get("table")
                    .and_then(Value::as_str)
                    .filter(|t| !t.is_empty())
                    .map(str::to_string);
                self.with_tourneys(|t| t.set_payout_table(&id, table.as_deref()))?;
                let t = self
                    .tourneys
                    .iter()
                    .find(|t| t.id == id)
                    .cloned()
                    .unwrap_or_default();
                let by = t
                    .payout_table
                    .as_deref()
                    .map_or("Default".to_string(), |p| self.table_name(p));
                self.book
                    .log_action(&actor, &format!("{} pays by {by}", t.name), now, None);
                out.tell.push(Tell::Tourneys);
            }
            // High Hand promotions (#267).
            "new_promo" => {
                let s = args.get("promo").cloned().unwrap_or_else(|| json!({}));
                let p = self.with_promos(|p| p.new_promo(&s, now, rng))?;
                out.value = p.id.into();
                out.tell.push(Tell::Promos);
                out.arm_promos = true;
            }
            "edit_promo" => {
                let s = args.get("promo").cloned().unwrap_or_else(|| json!({}));
                self.with_promos(|p| p.edit_promo(&id, &s, now))?;
                out.tell.push(Tell::Promos);
            }
            "promo_status" => {
                let status = text(args, "status");
                self.with_promos(|p| p.set_status(&id, &status))?;
                out.tell.push(Tell::Promos);
                out.arm_promos = true;
            }
            "approve_award" => {
                let a = self.with_promos(|p| p.approve_award(&id, now))?;
                self.pay_award(&a.id, now, &mut out)?;
                out.tell.push(Tell::Promos);
            }
            "void_award" => {
                let why = text(args, "why");
                let v = self.with_promos(|p| p.void_award(&id, now, &why, rng))?;
                if v.take > 0 {
                    let a = &v.award;
                    self.book.ensure_links(now)?;
                    let entries = self
                        .book
                        .money
                        .void(&a.member, &a.promo, v.take, now)
                        .map_err(|e| RuleError(e.to_string()))?;
                    self.book.chips.reclaimed += v.take as i64;
                    self.book.sync();
                    if let (Some(e), Some(i)) = (
                        entries.first(),
                        self.promos.awards.iter().position(|x| x.id == a.id),
                    ) {
                        self.promos.awards[i]
                            .other
                            .insert("voidEntry".into(), e.n.into());
                    }
                }
                out.account(&v.award.member, None);
                out.value = v
                    .offered
                    .map_or(Value::Null, |o| json!({"offered": o.id, "name": o.name}));
                out.tell.push(Tell::Promos);
            }
            "promos" => {
                let p = Promos {
                    code: self.book.club.code.clone(),
                    members: self.book.club.members.clone(),
                    promos: self.promos.clone(),
                };
                out.value = p.admin(now);
            }
            // Tournaments (#77): a new one, timed to start; cancelling one,
            // every buy-in back. `limits`: the plan's (#265, tourneysPerDay).
            "new_tourney" => {
                let s = args.get("tourney").cloned().unwrap_or_else(|| json!({}));
                let most = args
                    .get("limits")
                    .and_then(|l| l.get("tourneysPerDay"))
                    .and_then(Value::as_u64);
                let t = self.with_tourneys(|t| t.new_tourney(&s, now, most, rng))?;
                out.schedule.push(t.id.clone());
                out.value = t.id.into();
                out.tell.push(Tell::Tourneys);
            }
            "cancel_tourney" => {
                if !self.tourneys.iter().any(|t| t.id == id) {
                    return refuse("No such tournament.");
                }
                let mut step = crate::club_tourney_host::TourneyStep::default();
                self.tourney_abandon(&id, "the manager cancelled it", true, now, &mut step)?;
                for m in &step.chips_changed {
                    self.account_latest(m, &mut out);
                }
                out.stop_tourneys.push(id);
                out.tell.push(Tell::Tourneys);
            }
            // A member registering or unregistering before the start (#77):
            // the buy-in down their path, or back up it. `tid`: the tournament.
            "register" | "unregister" => {
                let tid = text(args, "tid");
                if op == "register" {
                    let buy_in = self.with_tourneys(|t| t.check_register(&tid, &id, now))?;
                    if buy_in > 0 {
                        self.book
                            .tourney_buy_in(&id, &tid, Some(buy_in as i64), now)
                            .map_err(|e| RuleError(e.message))?;
                    }
                    self.with_tourneys(|t| t.register(&tid, &id, now))?;
                } else {
                    let back = self.with_tourneys(|t| t.unregister(&tid, &id))?;
                    if back.amount > 0 {
                        self.book
                            .pay(&id, "refund", Some(back.amount as i64), &tid, now)
                            .map_err(|e| RuleError(e.message))?;
                    }
                }
                self.account_latest(&id, &mut out);
                out.tell.push(Tell::Tourneys);
            }
            // Must move (#501, #518), set for a game (`key`): on or off, its
            // main tables, and whether must-move tables open and become main
            // tables by themselves.
            "set_must_move" => {
                let s: MustMove = serde_json::from_value(args.clone())
                    .map_err(|_| RuleError("Not a valid must move.".into()))?;
                let key = text(args, "key");
                let g = self.book.club.set_must_move(&key, &s, rng)?;
                out.value = g.map_or(Value::Null, Value::from);
                out.tell.push(Tell::Tables);
            }
            // Taking a no-show off a game's list (#518).
            "list_remove" => {
                if self.book.club.leave_list(&id, &text(args, "key")) {
                    out.tell.push(Tell::Tables);
                    out.lists_to.push(id);
                }
            }
            _ => return refuse(format!("no host action {op}")),
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use serde_json::Map;

    fn rng() -> StdRng {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEED: AtomicU64 = AtomicU64::new(1);
        StdRng::seed_from_u64(SEED.fetch_add(1, Ordering::Relaxed))
    }

    fn club(ids: &[&str]) -> ClubState {
        let mut members = Map::new();
        for id in ids {
            members.insert(
                (*id).into(),
                json!({"id": id, "name": id[..1].to_uppercase() + &id[1..], "status": "member"}),
            );
        }
        let mut c: ClubState = serde_json::from_value(
            json!({"code": "ABCD-EFGH", "hostId": "HOST", "members": members}),
        )
        .unwrap();
        c.book.ensure_links(0).unwrap();
        c.book.sync();
        c
    }

    fn run(c: &mut ClubState, op: &str, args: Value) -> Result<Outcome, RuleError> {
        c.run(op, &args, 10, &mut rng())
    }

    #[test]
    fn chips_and_who_hears_about_them() {
        let mut c = club(&["ann"]);
        let o = run(
            &mut c,
            "give",
            json!({"id": "ann", "amount": 500, "note": "first"}),
        )
        .unwrap();
        assert_eq!(c.book.balance("ann"), 500);
        assert_eq!(
            o.accounts,
            vec![AccountTo {
                id: "ann".into(),
                since: Some(0)
            }]
        );
        let o = run(&mut c, "take", json!({"id": "ann", "amount": 200})).unwrap();
        assert_eq!(o.accounts[0].since, Some(1));
        assert!(
            run(&mut c, "give", json!({"id": "ann", "amount": 0}))
                .unwrap_err()
                .0
                .contains("whole number")
        );
        assert!(
            run(&mut c, "nope", json!({}))
                .unwrap_err()
                .0
                .contains("no host action nope")
        );
    }

    #[test]
    fn removing_a_member_takes_their_chips_back_first() {
        let mut c = club(&["ann"]);
        run(&mut c, "give", json!({"id": "ann", "amount": 300})).unwrap();
        let o = run(&mut c, "remove", json!({"id": "ann"})).unwrap();
        assert_eq!(c.book.balance("ann"), 0);
        assert_eq!(c.book.chips.reclaimed, 300);
        assert_eq!(o.replies[0]["body"]["status"], "removed");
        assert_eq!(o.accounts[0].since, Some(0));
        assert_eq!(o.tell, vec![Tell::Lobby]);
        assert_eq!(c.book.club.members["ann"].removed_at, Some(10));
    }

    #[test]
    fn settings_are_checked_before_any_is_set() {
        let mut c = club(&[]);
        assert_eq!(
            run(
                &mut c,
                "settings",
                json!({"sitOutMins": 15, "stayHands": 999})
            )
            .unwrap_err()
            .0,
            "Not a valid stayHands."
        );
        assert_eq!(c.book.club.sit_out_mins, None);
        run(
            &mut c,
            "settings",
            json!({"sitOutMins": 15, "keepDays": 30, "codeJoin": true}),
        )
        .unwrap();
        assert_eq!(c.book.club.sit_out_mins, Some(15.into()));
        assert_eq!(c.keep_days, Some(30.into()));
        assert_eq!(c.book.club.code_join, Some(true));
    }

    #[test]
    fn payout_tables_are_logged() {
        let mut c = club(&[]);
        let o = run(
            &mut c,
            "add_payout_table",
            json!({"table": {"name": "Main", "rows": [{"from": 1, "places": [10000]}]}}),
        )
        .unwrap();
        let id = o.value.as_str().unwrap().to_string();
        run(
            &mut c,
            "edit_payout_table",
            json!({"id": id, "table": {"name": "Big"}}),
        )
        .unwrap();
        run(&mut c, "delete_payout_table", json!({"id": id})).unwrap();
        let lines: Vec<&str> = c
            .book
            .log
            .iter()
            .map(|l| l["text"].as_str().unwrap())
            .collect();
        assert_eq!(
            lines,
            vec![
                "Added the payout table Main",
                "Renamed the payout table Main to Big",
                "Deleted the payout table Big"
            ]
        );
    }

    #[test]
    fn approving_a_held_award_pays_it_and_voiding_takes_it_back() {
        let mut c = club(&["ann"]);
        let o = run(
            &mut c,
            "new_promo",
            json!({"promo": {"kind": "bonus", "games": ["nlhe"], "every": "day", "tiers": [{"category": "quads", "amount": 1000}], "budget": 1}}),
        )
        .unwrap();
        assert!(o.arm_promos);
        // A quads hand over the budget: held for the manager.
        let rec = json!({"id": "h1", "table": "t1", "hand": 1, "game": "nlhe", "at": 10, "pot": 10, "showdown": true,
            "board": ["Kh", "Kd", "2s", "7c", "9d"], "seats": [{"i": 0, "name": "Ann", "cards": ["Ks", "Kc"], "inHand": true},
            {"i": 1, "inHand": true}, {"i": 2, "inHand": true}, {"i": 3, "inHand": true}]});
        let players = [json!({"seat": 0, "client_id": "ann"})];
        let mut r = rng();
        c.with_promos(|p| {
            p.process_hand(&rec, &players, 10, &mut r);
            Ok(())
        })
        .unwrap();
        // Over the budget, a bonus isn't held: it's not paid. Hold one by hand.
        let held = crate::club_promos::Award {
            id: "a1".into(),
            promo: c.promos.list[0].id.clone(),
            member: "ann".into(),
            name: "Ann".into(),
            amount: 1000,
            status: "held".into(),
            period: "0".into(),
            kind: "high_hand".into(),
            at: 10,
            ..Default::default()
        };
        c.promos.awards.push(held);
        let o = run(&mut c, "approve_award", json!({"id": "a1"})).unwrap();
        assert_eq!(c.book.balance("ann"), 1000);
        assert_eq!(c.book.chips.issued, 1000);
        assert_eq!(o.accounts[0].id, "ann");
        assert!(
            c.promos
                .awards
                .iter()
                .find(|a| a.id == "a1")
                .unwrap()
                .entry
                .is_some()
        );
        run(&mut c, "void_award", json!({"id": "a1", "why": "misdeal"})).unwrap();
        assert_eq!(c.book.balance("ann"), 0);
        assert_eq!(c.book.chips.reclaimed, 1000);
    }

    #[test]
    fn tournaments_registering_and_cancelling() {
        let mut c = club(&["ann", "bob"]);
        for id in ["ann", "bob"] {
            run(&mut c, "give", json!({"id": id, "amount": 1000})).unwrap();
        }
        let s = json!({"name": "Sunday", "buyIn": 100, "stack": 1500, "opensAt": 0, "startsAt": 1000,
                       "structure": [{"sb": 10, "bb": 20, "minutes": 10}]});
        // The plan's limit on tournaments a day.
        let o = run(
            &mut c,
            "new_tourney",
            json!({"tourney": s, "limits": {"tourneysPerDay": 1}}),
        )
        .unwrap();
        let tid = o.value.as_str().unwrap().to_string();
        assert_eq!(o.schedule, std::slice::from_ref(&tid));
        assert_eq!(o.tell, [Tell::Tourneys]);
        assert!(
            run(
                &mut c,
                "new_tourney",
                json!({"tourney": s, "limits": {"tourneysPerDay": 1}})
            )
            .is_err()
        );
        // Registering takes the buy-in; unregistering gives it back.
        for id in ["ann", "bob"] {
            let o = run(&mut c, "register", json!({"tid": tid, "id": id})).unwrap();
            assert_eq!(o.accounts[0].id, id);
            assert!(o.accounts[0].since.is_some());
        }
        assert_eq!(c.book.balance("ann"), 900);
        run(&mut c, "unregister", json!({"tid": tid, "id": "bob"})).unwrap();
        assert_eq!(c.book.balance("bob"), 1000);
        // Cancelling refunds, logs, and stops it.
        let o = run(&mut c, "cancel_tourney", json!({"id": tid})).unwrap();
        assert_eq!(o.stop_tourneys, std::slice::from_ref(&tid));
        assert_eq!(
            o.accounts.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            ["ann"]
        );
        assert_eq!(c.book.balance("ann"), 1000);
        assert!(run(&mut c, "cancel_tourney", json!({"id": "nope"})).is_err());
    }

    #[test]
    fn must_move_is_set_per_game_and_no_shows_come_off_lists() {
        let mut c = club(&["ann"]);
        c.book.club.tables = serde_json::from_value(json!([
            {"id": "t1", "name": "Main", "game": "nlhe", "sb": 1, "bb": 2, "minBB": 20, "maxBB": 100, "seats": 6, "status": "open"},
            {"id": "t2", "name": "Side", "game": "nlhe", "sb": 1, "bb": 2, "minBB": 20, "maxBB": 100, "seats": 6, "status": "open", "createdAt": 1}
        ]))
        .unwrap();
        let key = "nlhe:1/2:6";
        let o = run(
            &mut c,
            "set_must_move",
            json!({"key": key, "on": true, "mains": 1, "autoOpen": true, "min": 4}),
        )
        .unwrap();
        assert!(o.value.is_string());
        assert_eq!(o.tell, [Tell::Tables]);
        let g = &c.book.club.groups[0];
        assert_eq!((g.mains, g.min, g.auto_open), (1, 4, true));
        assert!(
            run(
                &mut c,
                "set_must_move",
                json!({"key": key, "on": true, "mains": 1, "min": 9})
            )
            .is_err()
        );
        run(&mut c, "set_must_move", json!({"key": key, "on": false})).unwrap();
        assert!(c.book.club.groups.is_empty());
        c.book.club.join_list("ann", key, 5).unwrap();
        let o = run(&mut c, "list_remove", json!({"key": key, "id": "ann"})).unwrap();
        assert_eq!(o.lists_to, ["ann"]);
        assert!(c.book.club.lists.is_empty());
    }
}
