//! A club's book (ducy-web #456): its rules ([`Club`]) and its money
//! ([`Money`]) together, for what needs both: chips the club or an agent
//! gives and takes back, chip requests, credit limits, agents, and the
//! activity log.
//!
//! Kept as ducy-web's JSON: the club object, with its money as `money`, its
//! totals as `chips` ({issued, reclaimed}), answered requests as
//! `requestLog` and the activity log as `log`. After every move each
//! member's mirrors (`balance`, `credit`, `room`) follow the money, as the
//! pages read them.

use rand::rngs::StdRng;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value, json};

use crate::club_money::{CLUB, Entry, EntryKind, Money, MoneyError};
use crate::club_rules::{Club, RuleError, Status, chip_text, new_invite, refuse};

/// The most chips in one move.
pub const MAX_CHIPS: u64 = 1_000_000_000_000;
/// The longest note.
pub const MAX_NOTE: usize = 80;
/// Answered requests kept.
pub const REQUEST_LOG: usize = 100;
/// Activity log entries kept.
pub const MAX_LOG: usize = 500;

/// Chips the club has issued and taken back, for conservation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Chips {
    /// Issued.
    pub issued: i64,
    /// Taken back.
    pub reclaimed: i64,
}

/// A club's rules and money.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Book {
    /// The club.
    #[serde(flatten)]
    pub club: Club,
    /// Its money, once on links.
    #[serde(default)]
    pub money: Money,
    /// Its totals.
    #[serde(default)]
    pub chips: Chips,
    /// Answered chip requests, newest last.
    #[serde(default)]
    pub request_log: Vec<Value>,
    /// What was done, newest last.
    #[serde(default)]
    pub log: Vec<Value>,
}

/// A member's account to send them, after entry `since` (none: no entries).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AccountTo {
    /// The member.
    pub id: String,
    /// Entries after this one; none for no entries.
    pub since: Option<u64>,
}

/// What an agent's command did.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct AgentDone {
    /// Status replies to send.
    pub replies: Vec<Value>,
    /// Accounts to send.
    pub accounts: Vec<AccountTo>,
    /// A player the agent let in, to send the tables.
    pub approved: Option<String>,
}

fn money_err(e: crate::club_money::MoneyError) -> RuleError {
    RuleError(e.to_string())
}

fn amount(a: Option<i64>) -> Result<u64, RuleError> {
    match a {
        Some(a) if a > 0 && a as u64 <= MAX_CHIPS => Ok(a as u64),
        _ => refuse("Enter a whole number of chips above 0."),
    }
}

fn note(s: &str) -> String {
    s.trim().chars().take(MAX_NOTE).collect()
}

fn int(v: Option<&Value>) -> Option<i64> {
    let v = v?;
    v.as_i64().or_else(|| {
        v.as_f64()
            .filter(|f| f.fract() == 0.0 && f.abs() <= 9e15)
            .map(|f| f as i64)
    })
}

impl Book {
    fn get(&self, id: &str, k: &str) -> Option<&Value> {
        self.club
            .members
            .get(id)?
            .other
            .get(k)
            .filter(|v| !v.is_null())
    }

    fn set(&mut self, id: &str, k: &str, v: Value) {
        if let Some(m) = self.club.members.get_mut(id) {
            m.other.insert(k.into(), v);
        }
    }

    /// A member's mirrored balance.
    pub fn balance(&self, id: &str) -> i64 {
        self.get(id, "balance").and_then(Value::as_i64).unwrap_or(0)
    }

    /// A member's credit limit.
    pub fn credit(&self, id: &str) -> u64 {
        self.get(id, "credit").and_then(Value::as_u64).unwrap_or(0)
    }

    /// What a member can bring to a table now.
    pub fn available(&self, id: &str) -> u64 {
        match self.get(id, "room").and_then(Value::as_u64) {
            Some(r) => r,
            None => (self.balance(id) + self.credit(id) as i64).max(0) as u64,
        }
    }

    fn name(&self, id: &str) -> String {
        self.club
            .members
            .get(id)
            .map_or(id.to_string(), |m| m.name.clone())
    }

    fn seated(&self, id: &str) -> bool {
        self.get(id, "seat").is_some()
    }

    /// A member, or why they can't be acted on.
    fn member(&self, id: &str) -> Result<(), RuleError> {
        match self.club.members.get(id) {
            Some(m) if m.status == Status::Member => Ok(()),
            _ => refuse("Not a member of this club."),
        }
    }

    /// An agent in the club.
    pub fn is_agent(&self, id: &str) -> bool {
        self.club
            .members
            .get(id)
            .is_some_and(|m| m.status == Status::Member && m.agent.as_deref() == Some("yes"))
    }

    /// Whom a member plays through, by their fields: their agent, or the club.
    fn backer_of(&self, id: &str) -> String {
        match self.club.members.get(id).and_then(|m| m.agent_of.clone()) {
            Some(a) if self.is_agent(&a) => a,
            _ => CLUB.into(),
        }
    }

    fn through(&self, id: &str) -> Option<String> {
        self.money.account(id).and_then(|a| a.through.clone())
    }

    /// Links for members who have none yet (new, or approved since).
    pub fn ensure_links(&mut self, now: u64) -> Result<(), RuleError> {
        let ids: Vec<String> = self
            .club
            .members
            .values()
            .filter(|m| m.status == Status::Member)
            .map(|m| m.id.clone())
            .collect();
        for id in ids {
            self.link(&id, 0, now)?;
        }
        Ok(())
    }

    fn link(&mut self, id: &str, depth: u32, now: u64) -> Result<(), RuleError> {
        if self.through(id).is_some() || depth > 8 {
            return Ok(());
        }
        let b = self.backer_of(id);
        if b != CLUB {
            self.link(&b, depth + 1, now)?;
        }
        if self.money.link(id, &b).is_none() {
            self.money.add_link(id, &b).map_err(money_err)?;
        }
        self.money.set_through(id, &b).map_err(money_err)?;
        let credit = self.credit(id);
        if credit > 0 {
            self.money.set_credit(id, &b, credit).map_err(money_err)?;
        }
        let _ = now;
        Ok(())
    }

    /// Each member's balance, credit and room, from the money.
    pub fn sync(&mut self) {
        let ids: Vec<String> = self.club.members.keys().cloned().collect();
        for id in ids {
            let Some(b) = self.through(&id) else { continue };
            let Some(l) = self.money.link(&id, &b).cloned() else {
                continue;
            };
            let room = self.room_in(&id);
            let m = self.club.members.get_mut(&id).expect("listed");
            m.other.insert("balance".into(), l.balance.into());
            if l.credit > 0 {
                m.other.insert("credit".into(), l.credit.into());
            } else {
                m.other.remove("credit");
            }
            m.other.insert("room".into(), room.into());
        }
    }

    /// The least room on a member's path up to the club.
    fn room_in(&self, id: &str) -> u64 {
        let mut room = u64::MAX;
        let mut at = id.to_string();
        for _ in 0..64 {
            let Some(b) = self.through(&at) else { return 0 };
            let r = self.money.link(&at, &b).map_or(0, |l| l.room());
            room = room.min(r);
            if b == CLUB {
                return room;
            }
            at = b;
        }
        0
    }

    /// Adds an entry to the activity log; `amount`: the chips it moved.
    pub fn log_action(&mut self, by: &str, text: &str, now: u64, amount: Option<i64>) {
        let mut e = json!({"at": now, "by": by, "text": text});
        if let Some(a) = amount {
            e["amount"] = a.into();
        }
        self.log.push(e);
        let n = self.log.len();
        if n > MAX_LOG {
            self.log.drain(..n - MAX_LOG);
        }
    }

    fn direct(&self, id: &str) -> Result<(), RuleError> {
        let b = self.backer_of(id);
        if b != CLUB {
            return refuse(format!(
                "{} plays under {}: their chips come from them.",
                self.name(id),
                self.name(&b)
            ));
        }
        Ok(())
    }

    /// The club gives a member chips.
    pub fn issue(
        &mut self,
        id: &str,
        amt: Option<i64>,
        why: &str,
        now: u64,
    ) -> Result<Entry, RuleError> {
        self.member(id)?;
        let amt = amount(amt)?;
        self.direct(id)?;
        self.ensure_links(now)?;
        let e = self
            .money
            .give(CLUB, id, amt, &note(why), now)
            .map_err(money_err)?;
        self.chips.issued += amt as i64;
        self.sync();
        Ok(e)
    }

    /// The club takes chips back from a member's balance (not a table's).
    pub fn reclaim(
        &mut self,
        id: &str,
        amt: Option<i64>,
        why: &str,
        now: u64,
    ) -> Result<Entry, RuleError> {
        if !self.club.members.contains_key(id) {
            return refuse("No such member.");
        }
        let amt = amount(amt)?;
        let bal = self.balance(id);
        if amt as i64 > bal {
            return refuse(format!(
                "{} only has {} chips in the club.",
                self.name(id),
                chip_text(bal.max(0) as u64)
            ));
        }
        self.direct(id)?;
        let e = self
            .money
            .take_back(CLUB, id, amt, &note(why), now)
            .map_err(money_err)?;
        self.chips.reclaimed += amt as i64;
        self.sync();
        Ok(e)
    }

    /// A backer (an agent) gives one of their players chips: it costs them
    /// nothing until the chips go on a table.
    pub fn backer_give(
        &mut self,
        backer: &str,
        id: &str,
        amt: Option<i64>,
        why: &str,
        now: u64,
    ) -> Result<Entry, RuleError> {
        self.member(id)?;
        let amt = amount(amt)?;
        self.ensure_links(now)?;
        let e = self
            .money
            .give(backer, id, amt, &note(why), now)
            .map_err(money_err)?;
        self.sync();
        Ok(e)
    }

    /// A backer takes back chips from one of their players, up to what they have.
    pub fn backer_take(
        &mut self,
        backer: &str,
        id: &str,
        amt: Option<i64>,
        why: &str,
        now: u64,
    ) -> Result<Entry, RuleError> {
        self.member(id)?;
        let amt = amount(amt)?;
        let bal = self.balance(id);
        if amt as i64 > bal {
            return refuse(format!(
                "{} has {} chips.",
                self.name(id),
                chip_text(bal.max(0) as u64)
            ));
        }
        let e = self
            .money
            .take_back(backer, id, amt, &note(why), now)
            .map_err(money_err)?;
        self.sync();
        Ok(e)
    }

    /// A member asks for chips; a new request replaces an older one.
    /// `table`: asked from that table. A credit player asking for more than
    /// is available is at their limit: the host may only raise it or say no.
    pub fn request_chips(
        &mut self,
        id: &str,
        amt: Option<i64>,
        why: &str,
        table: &str,
        now: u64,
    ) -> Result<Value, RuleError> {
        self.member(id)?;
        let amt = amount(amt)?;
        let mut r = json!({"amount": amt, "note": note(why), "at": now});
        if !table.is_empty() {
            r["table"] = table.chars().take(40).collect::<String>().into();
        }
        if self.credit(id) > 0 && amt > self.available(id) {
            r["atLimit"] = true.into();
        }
        self.set(id, "request", r.clone());
        Ok(r)
    }

    fn request(&self, id: &str) -> Option<Value> {
        self.get(id, "request").cloned()
    }

    /// Refused if a request may not be granted with chips: a credit player
    /// at their limit is never given chips past it.
    pub fn check_grant(&self, id: &str) -> Result<(), RuleError> {
        if self.request(id).is_some_and(|r| r["atLimit"] == true) {
            return refuse(format!(
                "{} is at their credit limit: raise it, or say no.",
                self.name(id)
            ));
        }
        Ok(())
    }

    /// A member cancels their request; whether there was one.
    pub fn cancel_request(&mut self, id: &str) -> bool {
        if self.request(id).is_none() {
            return false;
        }
        self.set(id, "request", Value::Null);
        true
    }

    /// A request is answered: cleared, the member told how (`answer`), and
    /// kept in the request log. `by`: who answered, when not the host;
    /// `limit`: the credit limit raised to it instead of chips given.
    #[allow(clippy::too_many_arguments)]
    pub fn answer_request(
        &mut self,
        id: &str,
        outcome: &str,
        given: u64,
        why: &str,
        by: &str,
        limit: Option<u64>,
        now: u64,
    ) -> Result<Option<Value>, RuleError> {
        self.member(id)?;
        let Some(r) = self.request(id) else {
            return Ok(None);
        };
        let mut a = json!({"asked": r["amount"], "given": given, "outcome": outcome, "note": note(why), "at": now});
        if let Some(l) = limit {
            a["limit"] = l.into();
        }
        self.set(id, "answer", a.clone());
        self.set(id, "request", Value::Null);
        let mut kept = json!({"member": id});
        for (k, v) in a.as_object().into_iter().flatten() {
            kept[k] = v.clone();
        }
        if !by.is_empty() {
            kept["by"] = by.into();
        }
        self.request_log.push(kept);
        let n = self.request_log.len();
        if n > REQUEST_LOG {
            self.request_log.drain(..n - REQUEST_LOG);
        }
        Ok(Some(a))
    }

    /// The host grants a request, for the amount asked or `amt`.
    pub fn grant_request(
        &mut self,
        id: &str,
        amt: Option<i64>,
        now: u64,
    ) -> Result<Entry, RuleError> {
        self.member(id)?;
        let Some(r) = self.request(id) else {
            return refuse("There’s no request to grant.");
        };
        self.check_grant(id)?;
        let given = amt.or_else(|| int(r.get("amount")));
        let why = r["note"].as_str().unwrap_or("").to_string();
        let e = self.issue(id, given, &why, now)?;
        self.answer_request(id, "granted", given.unwrap_or(0) as u64, "", "", None, now)?;
        Ok(e)
    }

    /// The host (or an agent, `by`) declines a request; whether there was one.
    pub fn decline_request(
        &mut self,
        id: &str,
        why: &str,
        by: &str,
        now: u64,
    ) -> Result<bool, RuleError> {
        Ok(self
            .answer_request(id, "declined", 0, why, by, None, now)?
            .is_some())
    }

    fn set_limit(&mut self, id: &str, limit: u64, now: u64) -> Result<(), RuleError> {
        self.ensure_links(now)?;
        let b = self.through(id).unwrap_or_else(|| self.backer_of(id));
        self.money.set_credit(id, &b, limit).map_err(money_err)?;
        self.sync();
        Ok(())
    }

    /// The host sets a member's credit limit (0: none). Lowering it below
    /// what they owe is allowed: they have nothing available until they pay.
    pub fn set_credit(&mut self, id: &str, limit: Option<i64>, now: u64) -> Result<(), RuleError> {
        self.member(id)?;
        let Some(limit) = limit
            .filter(|&l| l >= 0 && l as u64 <= MAX_CHIPS)
            .map(|l| l as u64)
        else {
            return refuse("The credit limit is a whole number of chips (0 for none).");
        };
        if limit == self.credit(id) {
            return Ok(());
        }
        self.set_limit(id, limit, now)?;
        let host = self.club.host_id.clone();
        let text = format!(
            "{}’s credit limit is now {}",
            self.name(id),
            chip_text(limit)
        );
        self.log_action(&host, &text, now, None);
        Ok(())
    }

    /// The host answers a credit player's request at their limit by raising
    /// it to `limit`. The request counts as granted, the extra credit as given.
    pub fn raise_credit(
        &mut self,
        id: &str,
        limit: Option<i64>,
        now: u64,
    ) -> Result<(), RuleError> {
        self.member(id)?;
        if self.request(id).is_none() {
            return refuse("There’s no request to answer.");
        }
        let was = self.credit(id);
        let Some(limit) = limit
            .filter(|&l| l >= 0 && l as u64 <= MAX_CHIPS && l as u64 > was)
            .map(|l| l as u64)
        else {
            return refuse(format!("Raise the limit above {}.", chip_text(was)));
        };
        self.set_limit(id, limit, now)?;
        self.answer_request(id, "granted", limit - was, "", "", Some(limit), now)?;
        let host = self.club.host_id.clone();
        let text = format!(
            "{}’s credit limit raised to {}",
            self.name(id),
            chip_text(limit)
        );
        self.log_action(&host, &text, now, Some((limit - was) as i64));
        Ok(())
    }

    fn role_reply(&self, id: &str) -> Value {
        let agent = self.club.members.get(id).and_then(|m| m.agent.clone());
        json!({"to": id, "body": {"type": "agent_role", "club": self.club.code, "agent": agent}})
    }

    /// The host offers a member the agent role.
    pub fn offer_agent(&mut self, id: &str) -> Result<Value, RuleError> {
        let Some(m) = self
            .club
            .members
            .get(id)
            .filter(|m| m.status == Status::Member)
        else {
            return refuse("Only members can be agents.");
        };
        if m.agent.as_deref() == Some("yes") {
            return refuse(format!("{} is already an agent.", m.name));
        }
        if m.agent_of.is_some() {
            return refuse(format!(
                "{} plays under an agent; unassign them first.",
                m.name
            ));
        }
        self.club.members.get_mut(id).expect("found").agent = Some("offered".into());
        Ok(self.role_reply(id))
    }

    /// A member answers the offer.
    pub fn answer_agent(
        &mut self,
        id: &str,
        accept: bool,
        now: u64,
        rng: &mut StdRng,
    ) -> Option<Value> {
        let m = self.club.members.get_mut(id)?;
        if m.agent.as_deref() != Some("offered") {
            return None;
        }
        if accept {
            m.agent = Some("yes".into());
            m.agent_invite = Some(new_invite(rng));
            let text = format!(
                "{} became an agent (credit limit {})",
                self.name(id),
                chip_text(self.credit(id))
            );
            self.log_action(id, &text, now, None);
        } else {
            m.agent = None;
        }
        Some(self.role_reply(id))
    }

    fn players_of(&self, id: &str) -> Vec<String> {
        self.club
            .members
            .values()
            .filter(|m| m.agent_of.as_deref() == Some(id) && m.status == Status::Member)
            .map(|m| m.id.clone())
            .collect()
    }

    fn play_through(&mut self, id: &str, backer: &str) -> Result<(), RuleError> {
        if self.money.link(id, backer).is_none() {
            self.money.add_link(id, backer).map_err(money_err)?;
        }
        self.money.set_through(id, backer).map_err(money_err)
    }

    /// The host ends the agent role: only once the agent's account isn't in
    /// credit and they've settled with their players, none seated. Their
    /// players then play through the club.
    pub fn revoke_agent(&mut self, id: &str, now: u64) -> Result<Option<Value>, RuleError> {
        let Some(was) = self.club.members.get(id).and_then(|m| m.agent.clone()) else {
            return Ok(None);
        };
        let name = self.name(id);
        if was == "yes" {
            if self.balance(id) < 0 {
                return refuse(format!("{name}'s account is in credit: settle up first."));
            }
            let open = self
                .players_of(id)
                .into_iter()
                .find(|p| self.seated(p) || self.money.link(p, id).is_some_and(|l| l.balance != 0));
            if let Some(p) = open {
                let after = if self.seated(&p) {
                    ", after they leave their table"
                } else {
                    ""
                };
                return refuse(format!(
                    "{name} and {} settle up first{after}.",
                    self.name(&p)
                ));
            }
        }
        self.ensure_links(now)?;
        let under: Vec<String> = self
            .club
            .members
            .values()
            .filter(|m| m.agent_of.as_deref() == Some(id))
            .map(|m| m.id.clone())
            .collect();
        for p in under {
            let member = {
                let m = self.club.members.get_mut(&p).expect("listed");
                m.agent_of = None;
                m.status == Status::Member
            };
            if was == "yes" && member {
                self.play_through(&p, CLUB)?;
            }
        }
        let m = self.club.members.get_mut(id).expect("found");
        m.agent = None;
        m.agent_invite = None;
        if was == "yes" {
            let host = self.club.host_id.clone();
            self.log_action(&host, &format!("{name} is no longer an agent"), now, None);
        }
        self.sync();
        Ok(Some(self.role_reply(id)))
    }

    /// The host puts a member under an agent (or none). They move only with
    /// nothing on their link and not seated.
    pub fn assign_to_agent(
        &mut self,
        player: &str,
        agent: Option<&str>,
        now: u64,
    ) -> Result<(), RuleError> {
        let Some(p) = self
            .club
            .members
            .get(player)
            .filter(|m| m.status == Status::Member)
        else {
            return refuse("No such member.");
        };
        if p.agent.is_some() {
            return refuse("Agents can’t play under another agent.");
        }
        let agent = agent.filter(|a| !a.is_empty());
        if let Some(a) = agent
            && !self.is_agent(a)
        {
            return refuse("Not an agent.");
        }
        let name = p.name.clone();
        if agent != p.agent_of.as_deref() {
            if self.seated(player) {
                return refuse(format!(
                    "{name} is at a table; they need to leave it first."
                ));
            }
            let bal = self.balance(player);
            if bal != 0 {
                return refuse(format!(
                    "{name} has {} on their account: settle it first.",
                    chip_text(bal.max(0) as u64)
                ));
            }
            self.ensure_links(now)?;
            self.play_through(player, agent.unwrap_or(CLUB))?;
        }
        self.club.members.get_mut(player).expect("found").agent_of = agent.map(str::to_string);
        let text = match agent {
            Some(a) => format!("{name} now plays under {}", self.name(a)),
            None => format!("{name} no longer plays under an agent"),
        };
        let host = self.club.host_id.clone();
        self.log_action(&host, &text, now, None);
        self.sync();
        Ok(())
    }

    fn mine(&self, by: &str, id: &str) -> Result<String, RuleError> {
        match self.club.members.get(id) {
            Some(p) if p.agent_of.as_deref() == Some(by) => Ok(p.name.clone()),
            _ => refuse("Not one of your players."),
        }
    }

    /// An agent's command (signed, through the host).
    pub fn run_agent(&mut self, by: &str, cmd: &Value, now: u64) -> Result<AgentDone, RuleError> {
        if !self.is_agent(by) {
            return refuse("You’re not an agent in this club.");
        }
        let a = self.name(by);
        let op = cmd.get("op").and_then(Value::as_str).unwrap_or("");
        let id = cmd
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let why = cmd
            .get("note")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let mut done = AgentDone::default();
        match op {
            "approve" | "decline" => {
                let ok = self.club.members.get(&id).is_some_and(|p| {
                    p.agent_of.as_deref() == Some(by) && p.status == Status::Pending
                });
                if !ok {
                    return refuse("No such request.");
                }
                let p = self.name(&id);
                let reply = if op == "approve" {
                    self.club.approve(&id, now)
                } else {
                    self.club.decline(&id, now)
                };
                done.replies.extend(reply);
                let did = if op == "approve" {
                    "approved"
                } else {
                    "declined"
                };
                self.log_action(by, &format!("{a} {did} {p}"), now, None);
                if op == "approve" {
                    done.approved = Some(id);
                    self.ensure_links(now)?;
                    self.sync();
                }
            }
            "give" | "grant" => {
                let p = self.mine(by, &id)?;
                let grant = op == "grant";
                let request = self.request(&id);
                if grant && request.is_none() {
                    return refuse("There’s no request to grant.");
                }
                let amt = if grant {
                    int(cmd.get("amount").filter(|v| !v.is_null()))
                        .or_else(|| request.as_ref().and_then(|r| int(r.get("amount"))))
                } else {
                    int(cmd.get("amount"))
                };
                if grant {
                    self.check_grant(&id)?;
                }
                let amt = amount(amt)?;
                let why = if grant {
                    request
                        .as_ref()
                        .and_then(|r| r["note"].as_str())
                        .unwrap_or("")
                        .to_string()
                } else {
                    why
                };
                let e = self.backer_give(by, &id, Some(amt as i64), &why, now)?;
                if grant {
                    self.answer_request(&id, "granted", amt, "", by, None, now)?;
                }
                let did = if grant { "granted" } else { "gave" };
                self.log_action(
                    by,
                    &format!("{a} {did} {p} {}", chip_text(amt)),
                    now,
                    Some(amt as i64),
                );
                done.accounts.push(AccountTo {
                    id,
                    since: Some(e.n - 1),
                });
            }
            "take" => {
                let p = self.mine(by, &id)?;
                let e = self.backer_take(by, &id, int(cmd.get("amount")), &why, now)?;
                let took = (-e.amount) as u64;
                self.log_action(
                    by,
                    &format!("{a} took back {} from {p}", chip_text(took)),
                    now,
                    Some(took as i64),
                );
                done.accounts.push(AccountTo {
                    id,
                    since: Some(e.n - 1),
                });
            }
            "deny" => {
                let p = self.mine(by, &id)?;
                if !self.decline_request(&id, &why, by, now)? {
                    return refuse("There’s no request to decline.");
                }
                self.log_action(by, &format!("{a} declined {p}’s chip request"), now, None);
                done.accounts.push(AccountTo { id, since: None });
            }
            _ => return refuse("Unknown command."),
        }
        Ok(done)
    }
}

/// A table move refused: why, and whether it's for want of chips (the
/// table then offers to ask the manager for more).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TableRefusal {
    /// Why, in the player's words.
    pub message: String,
    /// Too little available.
    pub short: bool,
}

impl From<RuleError> for TableRefusal {
    fn from(e: RuleError) -> Self {
        Self {
            message: e.0,
            short: false,
        }
    }
}

fn short<T>(message: String) -> Result<T, TableRefusal> {
    Err(TableRefusal {
        message,
        short: true,
    })
}

/// A table's buy-in limits, in chips.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct Limits {
    /// The least buy-in.
    pub min: u64,
    /// The most.
    pub max: u64,
}

impl Book {
    fn seat(&self, id: &str) -> Option<&serde_json::Map<String, Value>> {
        self.club.members.get(id)?.seat()
    }

    fn seat_num(&self, id: &str, k: &str) -> Option<u64> {
        self.seat(id)?.get(k)?.as_u64()
    }

    fn set_seat(&mut self, id: &str, seat: Value) {
        self.set(id, "seat", seat);
    }

    /// The least buy-in at `table` for `id` because of the rathole rule: who
    /// left with S chips comes back with at least S until the club's window
    /// has passed. 0 when the rule doesn't apply.
    pub fn rathole_min(&self, id: &str, table: &str, now: u64) -> u64 {
        let Some(left) = self.get(id, "left").and_then(|l| l.get(table)).cloned() else {
            return 0;
        };
        let hours = self
            .club
            .rathole_hours
            .as_ref()
            .and_then(Number::as_f64)
            .unwrap_or(2.0);
        let at = left["at"].as_u64().unwrap_or(0);
        if (now.saturating_sub(at) as f64) < hours * 3_600_000.0 {
            left["stack"].as_u64().unwrap_or(0)
        } else {
            0
        }
    }

    /// Refused, in the player's words, when `amt` is more than they have
    /// available.
    fn too_much(&self, id: &str, amt: u64) -> Result<(), TableRefusal> {
        let avail = self.available(id);
        if amt <= avail {
            return Ok(());
        }
        let own = (self.balance(id) + self.credit(id) as i64).max(0) as u64;
        if avail < own {
            return short(format!(
                "You have {} available: your agent can’t back more right now.",
                chip_text(avail)
            ));
        }
        if self.credit(id) == 0 {
            return short(format!(
                "You have {} chips in the club.",
                chip_text(self.balance(id).max(0) as u64)
            ));
        }
        short(format!(
            "You have {} available (your chips and credit).",
            chip_text(avail)
        ))
    }

    /// The money model's refusal, in the player's words.
    fn refusal(&self, id: &str, e: MoneyError) -> TableRefusal {
        match e {
            MoneyError::Room { .. } => TableRefusal {
                message: format!(
                    "You have {} available: your agent can’t back more right now.",
                    chip_text(self.available(id))
                ),
                short: true,
            },
            MoneyError::Overdue { .. } => TableRefusal {
                message: "What you owe is overdue: pay it back to play.".into(),
                short: false,
            },
            other => TableRefusal {
                message: other.to_string(),
                short: false,
            },
        }
    }

    fn own(entries: Vec<Entry>, id: &str) -> Option<Entry> {
        entries.into_iter().find(|e| e.member == id)
    }

    /// Sits a member at `table` with `amt` chips down their path, within
    /// the table's limits and the rathole rule.
    pub fn buy_in(
        &mut self,
        id: &str,
        table: &str,
        amt: Option<i64>,
        limits: Limits,
        now: u64,
    ) -> Result<Option<Entry>, TableRefusal> {
        self.member(id)?;
        let amt = amount(amt)?;
        if self.seat(id).is_some() {
            return Err(RuleError("Already seated at a table.".into()).into());
        }
        let floor = limits.min.max(self.rathole_min(id, table, now));
        if amt < floor {
            let msg = if floor > limits.min {
                let hours = self
                    .club
                    .rathole_hours
                    .as_ref()
                    .and_then(Number::as_f64)
                    .unwrap_or(2.0);
                format!(
                    "You left this table with {} chips less than {hours} hours ago, so buy in for at least that.",
                    chip_text(floor)
                )
            } else {
                format!("The minimum buy-in is {}.", limits.min)
            };
            return Err(RuleError(msg).into());
        }
        if amt > limits.max {
            return Err(
                RuleError(format!("The maximum buy-in is {}.", chip_text(limits.max))).into(),
            );
        }
        self.too_much(id, amt)?;
        self.ensure_links(now)?;
        let entries = self
            .money
            .buy_in(id, table, amt, now)
            .map_err(|e| self.refusal(id, e))?;
        self.set_seat(id, json!({"table": table, "stack": amt, "put": amt}));
        self.sync();
        Ok(Self::own(entries, id))
    }

    /// Adds chips to a seated member's stack between hands, so it doesn't
    /// go over `max`. None if nothing moved. More than is available is
    /// refused, not cut down.
    pub fn top_up(
        &mut self,
        id: &str,
        amt: Option<i64>,
        max: u64,
        now: u64,
    ) -> Result<Option<Entry>, TableRefusal> {
        self.member(id)?;
        let amt = amount(amt)?;
        let Some(stack) = self
            .seat_num(id, "stack")
            .or_else(|| self.seat(id).map(|_| 0))
        else {
            return Err(RuleError("Not seated.".into()).into());
        };
        let moved = amt.min(max.saturating_sub(stack));
        if moved == 0 {
            return Ok(None);
        }
        self.too_much(id, moved)?;
        let entries = self
            .money
            .top_up(id, moved, now)
            .map_err(|e| self.refusal(id, e))?;
        let mut seat = self.seat(id).cloned().unwrap_or_default();
        seat.insert("stack".into(), (stack + moved).into());
        if let Some(put) = seat.get("put").and_then(Value::as_u64) {
            seat.insert("put".into(), (put + moved).into());
        }
        self.set_seat(id, Value::Object(seat));
        self.sync();
        Ok(Self::own(entries, id))
    }

    /// The table handed back part of a top-up: those chips leave the seat
    /// and go back up the path.
    pub fn refund_top_up(
        &mut self,
        id: &str,
        amt: Option<i64>,
        now: u64,
    ) -> Result<Option<Entry>, TableRefusal> {
        self.member(id)?;
        let amt = amount(amt)?;
        let Some(mut seat) = self.seat(id).cloned() else {
            return Ok(None);
        };
        for k in ["stack", "put"] {
            if let Some(v) = seat.get(k).and_then(Value::as_u64) {
                seat.insert(k.into(), v.saturating_sub(amt).into());
            }
        }
        self.set_seat(id, Value::Object(seat));
        let entries = self
            .money
            .refund_top_up(id, amt, now)
            .map_err(|e| self.refusal(id, e))?;
        self.sync();
        Ok(Self::own(entries, id))
    }

    /// Stands a member up: their stack goes back up their path, and the
    /// rathole rule starts for that table. What they owed is paid back first.
    /// With nothing left, the cash-out is still an entry, at 0.
    pub fn cash_out(&mut self, id: &str, now: u64) -> Result<Option<Entry>, TableRefusal> {
        let Some(seat) = self.seat(id).cloned() else {
            return Err(RuleError("Not seated.".into()).into());
        };
        let table = seat
            .get("table")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let stack = seat.get("stack").and_then(Value::as_u64).unwrap_or(0);
        self.money
            .set_stack(id, stack)
            .map_err(|e| self.refusal(id, e))?;
        let entries = self
            .money
            .cash_out(id, 0, now)
            .map_err(|e| self.refusal(id, e))?;
        self.set_seat(id, Value::Null);
        let mut left = self
            .get(id, "left")
            .cloned()
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        left[table.as_str()] = json!({"stack": stack, "at": now});
        self.set(id, "left", left);
        self.sync();
        Ok(Self::own(entries, id))
    }

    /// A member at `table` asks for `amount` chips (the engine has checked
    /// the table's limits for their stack): a buy-in, or a top-up.
    pub fn chips_for_table(
        &mut self,
        id: &str,
        table: &str,
        amount: Option<i64>,
        limits: Limits,
        now: u64,
    ) -> Result<Option<Entry>, TableRefusal> {
        self.member(id)?;
        match self
            .seat(id)
            .and_then(|s| s.get("table"))
            .and_then(Value::as_str)
        {
            Some(t) if t != table => {
                Err(RuleError("Already seated at another table.".into()).into())
            }
            Some(_) => self.top_up(id, amount, u64::MAX, now),
            None => self.buy_in(id, table, amount, limits, now),
        }
    }

    /// Someone left a table with `chips` (the engine's count): back to their
    /// balance. None if they weren't seated.
    pub fn left_table(
        &mut self,
        id: &str,
        chips: u64,
        now: u64,
    ) -> Result<Option<Entry>, TableRefusal> {
        let Some(mut seat) = self.seat(id).cloned() else {
            return Ok(None);
        };
        seat.insert("stack".into(), chips.into());
        self.set_seat(id, Value::Object(seat));
        self.cash_out(id, now)
    }

    /// Everyone seated at a table but those in `keep` (or only at `only`)
    /// gets back the stack the ledger last had for them.
    pub fn cash_out_seated(
        &mut self,
        keep: &[String],
        only: Option<&str>,
        now: u64,
    ) -> Result<Vec<Entry>, TableRefusal> {
        let ids: Vec<String> = self
            .club
            .members
            .values()
            .filter(|m| {
                let t = m.seat_table();
                t.is_some_and(|t| only.map_or(!keep.iter().any(|k| k == t), |o| o == t))
            })
            .map(|m| m.id.clone())
            .collect();
        let mut out = vec![];
        for id in ids {
            out.extend(self.cash_out(&id, now)?);
        }
        Ok(out)
    }

    /// A tournament buy-in: `amt` down the member's path.
    pub fn tourney_buy_in(
        &mut self,
        id: &str,
        tourney: &str,
        amt: Option<i64>,
        now: u64,
    ) -> Result<Option<Entry>, TableRefusal> {
        self.member(id)?;
        let amt = amount(amt)?;
        let avail = self.available(id);
        if amt > avail {
            return Err(RuleError(format!(
                "The buy-in is {} chips; you have {}.",
                chip_text(amt),
                chip_text(avail)
            ))
            .into());
        }
        self.ensure_links(now)?;
        let entries = self
            .money
            .tourney_buy_in(id, tourney, amt, now)
            .map_err(|e| self.refusal(id, e))?;
        self.sync();
        Ok(Self::own(entries, id))
    }

    /// The club pays a member up their path: a prize (a tournament's or a
    /// promotion's) or a refund (a tournament buy-in back).
    pub fn pay(
        &mut self,
        id: &str,
        what: &str,
        amt: Option<i64>,
        about: &str,
        now: u64,
    ) -> Result<Option<Entry>, TableRefusal> {
        // Removed since they entered, they're paid all the same.
        if !self.club.members.contains_key(id) {
            return Err(RuleError("No such member.".into()).into());
        }
        let kind = match what {
            "prize" => EntryKind::Prize,
            "refund" => EntryKind::Refund,
            _ => return Err(RuleError("The club pays prizes and refunds.".into()).into()),
        };
        let amt = amount(amt)?;
        self.ensure_links(now)?;
        let entries = self
            .money
            .pay(id, kind, about, amt, now)
            .map_err(|e| self.refusal(id, e))?;
        self.sync();
        Ok(Self::own(entries, id))
    }

    /// A promotion prize taken back, down the path it came up.
    pub fn void_prize(
        &mut self,
        id: &str,
        amt: Option<i64>,
        promo: &str,
        now: u64,
    ) -> Result<Option<Entry>, TableRefusal> {
        self.member(id)?;
        let amt = amount(amt)?;
        let bal = self.balance(id);
        if (amt as i64) > bal {
            return Err(RuleError(format!(
                "{} has only {} chips in the club now; take what’s left from their balance first, or leave the award.",
                self.name(id),
                chip_text(bal.max(0) as u64)
            ))
            .into());
        }
        self.ensure_links(now)?;
        let entries = self
            .money
            .void(id, promo, amt, now)
            .map_err(|e| self.refusal(id, e))?;
        self.chips.reclaimed += amt as i64;
        self.sync();
        Ok(Self::own(entries, id))
    }
}

/// What a member may buy in for at table `t` (`{id, min, max}` as the host
/// tells members), by the rathole rule as their page sees it: having left
/// it within the last `rathole_hours`, they come back with what they left
/// with (their own last cash-out from it, in `entries`), up to its
/// maximum. `{min, max, rathole}`.
pub fn buy_in_range(t: &Value, entries: &[Value], rathole_hours: f64, now: u64) -> Value {
    let num = |v: Option<&Value>| v.and_then(Value::as_f64).unwrap_or(0.0);
    let (min, max) = (num(t.get("min")), num(t.get("max")));
    let left = entries.iter().rev().find(|e| {
        e.get("kind").and_then(Value::as_str) == Some("cash_out")
            && e.get("player")
                .is_none_or(|p| p.is_null() || p == &Value::Bool(false) || p == "")
            && e.get("about") == t.get("id")
    });
    let rathole = match left {
        Some(e)
            if rathole_hours > 0.0 && (now as f64 - num(e.get("at"))) < rathole_hours * 3.6e6 =>
        {
            num(e.get("amount"))
        }
        _ => 0.0,
    };
    json!({"min": max.min(min.max(rathole)), "max": max, "rathole": rathole})
}

/// The parts of the club a book changes, to write back.
pub fn written(book: &Book) -> Value {
    json!({
        "members": book.club.members, "money": book.money, "chips": book.chips,
        "requestLog": book.request_log, "log": book.log,
    })
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

    fn book(ids: &[&str]) -> Book {
        let mut members = Map::new();
        for id in ids {
            members.insert(
                (*id).into(),
                json!({"id": id, "name": id[..1].to_uppercase() + &id[1..], "status": "member"}),
            );
        }
        let mut b: Book = serde_json::from_value(
            json!({"code": "ABCD-EFGH", "hostId": "HOST", "members": members}),
        )
        .unwrap();
        b.ensure_links(0).unwrap();
        b.sync();
        b
    }

    fn err<T: std::fmt::Debug>(r: Result<T, RuleError>) -> String {
        r.unwrap_err().0
    }

    #[test]
    fn the_club_gives_and_takes_back_and_its_totals_follow() {
        let mut b = book(&["ann"]);
        b.issue("ann", Some(1000), " first chips ", 1).unwrap();
        assert_eq!(b.balance("ann"), 1000);
        assert_eq!(b.chips.issued, 1000);
        assert_eq!(
            b.money.ledger().last().unwrap().note.as_deref(),
            Some("first chips")
        );
        assert!(
            err(b.reclaim("ann", Some(2000), "", 2)).contains("Ann only has 10 chips in the club.")
        );
        b.reclaim("ann", Some(400), "", 3).unwrap();
        assert_eq!((b.balance("ann"), b.chips.reclaimed), (600, 400));
        assert!(err(b.issue("ann", Some(0), "", 4)).contains("whole number"));
        assert!(err(b.issue("bob", Some(10), "", 4)).contains("Not a member"));
    }

    #[test]
    fn requests_are_granted_declined_and_logged() {
        let mut b = book(&["ann"]);
        b.request_chips("ann", Some(500), "rebuy", "t1", 1).unwrap();
        let e = b.grant_request("ann", None, 2).unwrap();
        assert_eq!(e.amount, 500);
        assert_eq!(b.get("ann", "answer").unwrap()["outcome"], "granted");
        assert!(b.get("ann", "request").is_none());
        assert!(err(b.grant_request("ann", None, 3)).contains("no request"));
        b.request_chips("ann", Some(100), "", "", 4).unwrap();
        assert!(b.decline_request("ann", "not now", "", 5).unwrap());
        assert_eq!(b.request_log.len(), 2);
        assert_eq!(b.request_log[1]["note"], "not now");
    }

    #[test]
    fn credit_limits_and_a_request_at_the_limit() {
        let mut b = book(&["ann"]);
        b.set_credit("ann", Some(1000), 1).unwrap();
        assert_eq!((b.credit("ann"), b.available("ann")), (1000, 1000));
        assert_eq!(
            b.log.last().unwrap()["text"],
            "Ann’s credit limit is now 10"
        );
        let r = b.request_chips("ann", Some(5000), "", "", 2).unwrap();
        assert_eq!(r["atLimit"], true);
        assert!(err(b.grant_request("ann", None, 3)).contains("at their credit limit"));
        assert!(err(b.raise_credit("ann", Some(1000), 3)).contains("Raise the limit above 10."));
        b.raise_credit("ann", Some(5000), 4).unwrap();
        assert_eq!(b.credit("ann"), 5000);
        assert_eq!(b.get("ann", "answer").unwrap()["limit"], 5000);
        assert!(err(b.set_credit("ann", Some(-1), 5)).contains("whole number"));
    }

    #[test]
    fn agents_bring_and_back_their_players() {
        let mut b = book(&["al", "bo"]);
        b.offer_agent("al").unwrap();
        let reply = b.answer_agent("al", true, 1, &mut rng()).unwrap();
        assert_eq!(reply["body"]["agent"], "yes");
        assert!(b.club.members["al"].agent_invite.is_some());
        b.assign_to_agent("bo", Some("al"), 2).unwrap();
        assert_eq!(b.through("bo").as_deref(), Some("al"));
        // The club's chips go only to its own members.
        assert!(
            err(b.issue("bo", Some(100), "", 3))
                .contains("Bo plays under Al: their chips come from them.")
        );
        let done = b
            .run_agent("al", &json!({"op": "give", "id": "bo", "amount": 700}), 4)
            .unwrap();
        assert_eq!(b.balance("bo"), 700);
        assert_eq!(done.accounts[0].id, "bo");
        assert_eq!(b.log.last().unwrap()["text"], "Al gave Bo 7");
        b.run_agent("al", &json!({"op": "take", "id": "bo", "amount": 200}), 5)
            .unwrap();
        assert_eq!(b.balance("bo"), 500);
        assert!(
            err(b.run_agent("bo", &json!({"op": "give", "id": "al", "amount": 1}), 6))
                .contains("not an agent")
        );
        assert!(
            err(b.run_agent("al", &json!({"op": "give", "id": "al", "amount": 1}), 6))
                .contains("Not one of your players")
        );
        // Revoking waits for them to settle up.
        assert!(err(b.revoke_agent("al", 7)).contains("Al and Bo settle up first."));
        b.run_agent("al", &json!({"op": "take", "id": "bo", "amount": 500}), 8)
            .unwrap();
        b.revoke_agent("al", 9).unwrap();
        assert_eq!(b.through("bo").as_deref(), Some(CLUB));
        assert!(b.club.members["bo"].agent_of.is_none());
        assert_eq!(b.log.last().unwrap()["text"], "Al is no longer an agent");
    }

    const LIMITS: Limits = Limits {
        min: 100,
        max: 1000,
    };

    #[test]
    fn buy_in_play_and_cash_out_with_the_rathole_rule() {
        let mut b = book(&["ann"]);
        b.issue("ann", Some(2000), "", 1).unwrap();
        assert!(
            b.buy_in("ann", "t1", Some(50), LIMITS, 2)
                .unwrap_err()
                .message
                .contains("The minimum buy-in is 100.")
        );
        assert!(
            b.buy_in("ann", "t1", Some(5000), LIMITS, 2)
                .unwrap_err()
                .message
                .contains("maximum")
        );
        b.buy_in("ann", "t1", Some(500), LIMITS, 2).unwrap();
        assert_eq!(
            (b.balance("ann"), b.seat_num("ann", "stack")),
            (1500, Some(500))
        );
        assert!(
            b.buy_in("ann", "t2", Some(500), LIMITS, 3)
                .unwrap_err()
                .message
                .contains("Already seated")
        );
        // A top-up capped at the table's maximum.
        b.top_up("ann", Some(800), 1000, 4).unwrap();
        assert_eq!(b.seat_num("ann", "stack"), Some(1000));
        // They win, and stand up with 1,300.
        b.left_table("ann", 1300, 5).unwrap();
        assert_eq!(b.balance("ann"), 2300);
        assert!(b.seat("ann").is_none());
        // Back within the window: at least what they left with.
        assert_eq!(b.rathole_min("ann", "t1", 6), 1300);
        let e = b
            .buy_in(
                "ann",
                "t1",
                Some(500),
                Limits {
                    min: 100,
                    max: 2000,
                },
                6,
            )
            .unwrap_err();
        assert!(
            e.message
                .contains("You left this table with 13 chips less than 2 hours ago")
        );
        assert_eq!(b.rathole_min("ann", "t1", 6 + 2 * 3_600_000), 0);
    }

    #[test]
    fn short_of_chips_the_table_offers_to_ask() {
        let mut b = book(&["ann"]);
        b.issue("ann", Some(300), "", 1).unwrap();
        let e = b.buy_in("ann", "t1", Some(500), LIMITS, 2).unwrap_err();
        assert!(e.short);
        assert_eq!(e.message, "You have 3 chips in the club.");
        b.set_credit("ann", Some(100), 3).unwrap();
        let e = b.buy_in("ann", "t1", Some(500), LIMITS, 4).unwrap_err();
        assert_eq!(e.message, "You have 4 available (your chips and credit).");
        // Within credit: a buy-in on credit.
        b.buy_in("ann", "t1", Some(400), LIMITS, 5).unwrap();
        assert_eq!(b.balance("ann"), -100);
    }

    #[test]
    fn tournaments_and_prizes_go_along_the_path() {
        let mut b = book(&["ann"]);
        b.issue("ann", Some(1000), "", 1).unwrap();
        assert!(
            b.tourney_buy_in("ann", "T", Some(2000), 2)
                .unwrap_err()
                .message
                .contains("The buy-in is 20 chips; you have 10.")
        );
        b.tourney_buy_in("ann", "T", Some(400), 2).unwrap();
        assert_eq!(b.balance("ann"), 600);
        b.pay("ann", "refund", Some(400), "T", 3).unwrap();
        b.pay("ann", "prize", Some(1000), "P", 4).unwrap();
        assert_eq!(b.balance("ann"), 2000);
        assert!(
            b.void_prize("ann", Some(5000), "P", 5)
                .unwrap_err()
                .message
                .contains("Ann has only 20 chips")
        );
        b.void_prize("ann", Some(1000), "P", 5).unwrap();
        assert_eq!((b.balance("ann"), b.chips.reclaimed), (1000, 1000));
    }

    #[test]
    fn a_restart_cashes_out_everyone_but_the_tables_kept() {
        let mut b = book(&["ann", "bo"]);
        for id in ["ann", "bo"] {
            b.issue(id, Some(1000), "", 1).unwrap();
        }
        b.buy_in("ann", "t1", Some(500), LIMITS, 2).unwrap();
        b.buy_in("bo", "t2", Some(500), LIMITS, 2).unwrap();
        let out = b.cash_out_seated(&["t2".into()], None, 3).unwrap();
        assert_eq!(out.len(), 1);
        assert!(b.seat("ann").is_none() && b.seat("bo").is_some());
        b.cash_out_seated(&[], Some("t2"), 4).unwrap();
        assert!(b.seat("bo").is_none());
    }

    #[test]
    fn an_agent_approves_their_own_recruits() {
        let mut b = book(&["al"]);
        b.offer_agent("al").unwrap();
        b.answer_agent("al", true, 1, &mut rng());
        let p: crate::club_rules::Member = serde_json::from_value(
            json!({"id": "cy", "name": "Cy", "status": "pending", "agentOf": "al"}),
        )
        .unwrap();
        b.club.members.insert("cy".into(), p);
        let done = b
            .run_agent("al", &json!({"op": "approve", "id": "cy"}), 2)
            .unwrap();
        assert_eq!(done.approved.as_deref(), Some("cy"));
        assert_eq!(b.through("cy").as_deref(), Some("al"));
        assert!(
            err(b.run_agent("al", &json!({"op": "approve", "id": "cy"}), 3))
                .contains("No such request.")
        );
    }

    #[test]
    fn a_members_buy_in_range_by_the_rathole_rule() {
        let t = json!({"id": "t1", "min": 4000, "max": 20000});
        let hour = 3_600_000;
        let left = |amount: u64, at: u64| json!({"kind": "cash_out", "about": "t1", "amount": amount, "at": at});
        // Left with 300 an hour ago, the window 2 hours: back with 300.
        let r = buy_in_range(&t, &[left(30000, 0)], 2.0, hour);
        assert_eq!(
            r,
            json!({"min": 20000.0, "max": 20000.0, "rathole": 30000.0})
        );
        let r = buy_in_range(&t, &[left(9000, 0)], 2.0, hour);
        assert_eq!(r["min"], 9000.0);
        // After the window, or at another table, or someone else's move: the table's own.
        assert_eq!(
            buy_in_range(&t, &[left(9000, 0)], 2.0, 3 * hour)["min"],
            4000.0
        );
        let mut other = left(9000, 0);
        other["about"] = "t2".into();
        assert_eq!(buy_in_range(&t, &[other], 2.0, hour)["min"], 4000.0);
        let mut theirs = left(9000, 0);
        theirs["player"] = "bob".into();
        assert_eq!(buy_in_range(&t, &[theirs], 2.0, hour)["min"], 4000.0);
    }
}
