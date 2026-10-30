//! A club table's seats (ducy-web #456): what the club makes of each result
//! from a table's runner. Chips people left with go back to their balance,
//! top-ups the table handed back too; the ledger takes the engine's stacks
//! between hands; anyone the manager is removing stands up, and anyone
//! moving to another table leaves for it; the hit-and-run count goes to
//! each player as it changes; and chip requests are answered from the
//! club's book.
//!
//! The caller (ducy-web's club-host.js) runs the table: the runner, its
//! clock, saving and sending. It keeps a [`Seating`] for each table, hands it
//! in with the club and the result, and carries out the [`TableStep`] that
//! comes back: messages for players, events for the runner, and what to save
//! and tell.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::club_book::Limits;
use crate::club_host::ClubState;
use crate::club_rules::{RuleError, Table, refuse};

/// What the club keeps on a running table between the runner's results.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Seating {
    /// Who's seated, as the runner last said.
    pub players: Vec<Player>,
    /// Whether the hand shown is over.
    pub complete: bool,
    /// The hand shown.
    pub hand_number: u64,
    /// How many are in to deal to.
    pub players_in: u64,
    /// The last finished hand recorded for the history.
    pub recorded: u64,
    /// The hand the club was last saved at, if any.
    pub saved: Option<u64>,
    /// The hands each player was last told they owe the table.
    pub stay: Map<String, Value>,
    /// Seats whose chip request is being answered.
    pub asking: Vec<u32>,
    /// Members being stood up (removed, or idle).
    pub removed: Vec<String>,
    /// Members leaving for a seat held at another table.
    pub leaving: Vec<String>,
    /// Members with no seat in the ledger leaving for another table, by the
    /// table they're going to.
    pub moves: Map<String, Value>,
}

impl Default for Seating {
    fn default() -> Self {
        Self {
            players: vec![],
            complete: true,
            hand_number: 0,
            players_in: 0,
            recorded: 0,
            saved: None,
            stay: Map::new(),
            asking: vec![],
            removed: vec![],
            leaving: vec![],
            moves: Map::new(),
        }
    }
}

/// Someone seated, as the runner says.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Player {
    /// Their member id.
    pub client_id: String,
    /// Their seat.
    pub seat: u32,
    /// Everything else the runner says (chips, pending, …).
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

impl Player {
    fn chips(&self) -> u64 {
        count(self.other.get("chips"))
    }
}

impl Seating {
    /// Hands finished: the one shown counts once it's over.
    pub fn completed(&self) -> u64 {
        if self.complete {
            self.hand_number
        } else {
            self.hand_number.saturating_sub(1)
        }
    }

    fn player(&self, id: &str) -> Option<&Player> {
        self.players.iter().find(|p| p.client_id == id)
    }
}

/// What the caller does after a step.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableStep {
    /// Messages to send, in order: `{to, update}`, for this table.
    pub sends: Vec<Value>,
    /// Events for the table's runner, in order. A `cmd` is missing
    /// `online`, which the caller knows.
    pub events: Vec<Value>,
    /// Members whose balance changed: send them their account.
    pub chips_changed: Vec<String>,
    /// A hand is in play.
    pub on_play: bool,
    /// A hand just finished: record it.
    pub on_hand: bool,
    /// Save the club.
    pub save: bool,
    /// Seat counts or tables changed: tell members.
    pub tables_changed: bool,
    /// Reserved seats changed: re-arm the timer for the next to lapse.
    pub arm_holds: bool,
    /// What an op returns.
    pub value: Value,
}

impl TableStep {
    fn send(&mut self, to: &str, update: Value) {
        self.sends.push(json!({"to": to, "update": update}));
    }

    fn chips(&mut self, id: &str) {
        if !self.chips_changed.iter().any(|x| x == id) {
            self.chips_changed.push(id.into());
        }
    }
}

/// A count of chips or hands from the runner: a whole number, or a numeric
/// string; anything else is 0.
fn count(v: Option<&Value>) -> u64 {
    match v {
        Some(Value::Number(n)) => n
            .as_u64()
            .or_else(|| n.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64))
            .unwrap_or(0),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

fn list(v: &Value, k: &str) -> Vec<Value> {
    v.get(k)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// What a player is told when they try to leave too soon.
pub fn stay_text(n: u64) -> String {
    format!(
        "You’re up at this table, so you play {n} more {} before you can leave. Sitting out doesn’t stop the count: you’re dealt in and fold, posting your blinds, until it’s done.",
        if n == 1 { "hand" } else { "hands" }
    )
}

impl ClubState {
    fn seat_of(&self, id: &str) -> Option<&Map<String, Value>> {
        self.book.club.members.get(id)?.seat()
    }

    /// Member `id`'s seat, if it's at table `tid`.
    fn seat_at(&self, id: &str, tid: &str) -> Option<&Map<String, Value>> {
        self.seat_of(id)
            .filter(|s| s.get("table").and_then(Value::as_str) == Some(tid))
    }

    /// Whether member `id` is being removed from table `tid`.
    fn removing(&self, id: &str, tid: &str) -> bool {
        self.seat_at(id, tid)
            .is_some_and(|s| truthy(s.get("removing")))
    }

    /// Member `id`'s move from table `tid`: {to, by?}.
    fn move_of(&self, id: &str, tid: &str) -> Option<Map<String, Value>> {
        self.seat_at(id, tid)?.get("moving")?.as_object().cloned()
    }

    fn table_of(&self, tid: &str) -> Result<Table, RuleError> {
        match self.book.club.table(tid) {
            Some(t) => Ok(t.clone()),
            None => refuse("No such table."),
        }
    }

    fn member_name(&self, id: &str) -> String {
        self.book
            .club
            .members
            .get(id)
            .map_or(id.to_string(), |m| m.name.clone())
    }

    /// Whether `by` is the manager.
    fn manager(&self, by: &str) -> bool {
        by == self.book.club.host_id || self.book.club.owner_id.as_deref() == Some(by)
    }

    /// Why member `id`, with `chips` at their table, couldn't buy in at
    /// table `to` once they'd left it (their chips back in their balance,
    /// credit included, against its minimum and the rathole rule), or "".
    pub fn too_short(&self, id: &str, to: &str, chips: u64, now: u64) -> String {
        let (Some(_), Some(t)) = (self.book.club.members.get(id), self.book.club.table(to)) else {
            return String::new();
        };
        let could =
            (self.book.balance(id) + chips as i64 + self.book.credit(id) as i64).max(0) as u64;
        let least = t.limits().0.max(self.book.rathole_min(id, to, now));
        if could < least {
            format!("Not enough for {} · Staying here", t.name)
        } else {
            String::new()
        }
    }

    /// Between hands the engine's count is the truth: each seated member's
    /// stack, and when they first got ahead (the hit-and-run rule).
    fn sync_stacks(&mut self, players: &[Player], hand: u64) {
        let mut seated = vec![];
        for p in players {
            let seat = self
                .book
                .club
                .members
                .get_mut(&p.client_id)
                .and_then(|m| m.other.get_mut("seat"))
                .and_then(Value::as_object_mut);
            if let Some(seat) = seat {
                seat.insert("stack".into(), p.chips().into());
                seated.push(p.client_id.clone());
            }
        }
        if !seated.is_empty() {
            self.book.club.note_stacks(&seated, hand);
        }
    }

    /// The runner stands member `id` up: now between hands, else folding at
    /// their turn and gone when the hand ends. Once per removal. `why`:
    /// {idle: true} when they sat idle too long.
    pub fn stand_up(
        &self,
        s: &mut Seating,
        tid: &str,
        id: &str,
        why: &Map<String, Value>,
        out: &mut TableStep,
    ) {
        let Some(seat) = s.player(id).map(|p| p.seat) else {
            return;
        };
        if s.removed.iter().any(|x| x == id) {
            return;
        }
        s.removed.push(id.into());
        // Mid-hand the engine sends them nothing more: their page leaves
        // now, rather than wait on a table that stopped for them. So does
        // someone with no chips there (no seat in the ledger, so no result to
        // wait for).
        if !s.complete || self.seat_of(id).is_none() {
            let mut u = json!({"type": "removed", "name": self.book.club.table(tid).map_or("", |t| t.name.as_str())});
            for (k, v) in why {
                u[k] = v.clone();
            }
            out.send(id, u);
        }
        out.events.push(json!({"type": "remove", "seat": seat}));
    }

    /// Member `id` leaves table `tid` for the seat held for them, once. Up by
    /// then and owing hands (and not moved by a manager), or with too little
    /// to buy in there, they stay, and the seat is given up.
    pub fn leave_to_move(
        &mut self,
        s: &mut Seating,
        tid: &str,
        id: &str,
        now: u64,
        out: &mut TableStep,
    ) {
        let Some(chips) = s.player(id).map(Player::chips) else {
            return;
        };
        if s.leaving.iter().any(|x| x == id) {
            return;
        }
        if let Some(mv) = self.move_of(id, tid) {
            let to = mv
                .get("to")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let stay = self.book.club.table(tid).map_or(0, |t| t.stay_hands);
            let owed = if truthy(mv.get("by")) || stay == 0 {
                0
            } else {
                self.book.club.hands_to_stay(id, tid, s.completed())
            };
            let short = if owed == 0 {
                self.too_short(id, &to, chips, now)
            } else {
                String::new()
            };
            if owed > 0 || !short.is_empty() {
                if let Some(seat) = self
                    .book
                    .club
                    .members
                    .get_mut(id)
                    .and_then(|m| m.other.get_mut("seat"))
                    .and_then(Value::as_object_mut)
                {
                    seat.remove("moving");
                }
                self.book.club.drop_reservation(&to, id);
                out.save = true;
                out.arm_holds = true;
                out.tables_changed = true;
                let refused = if owed > 0 { stay_text(owed) } else { short };
                out.send(
                    id,
                    json!({"type": "reserve", "table": to, "refused": refused}),
                );
                out.events.push(json!({"type": "state"}));
                return;
            }
        }
        s.leaving.push(id.into());
        out.events
            .push(json!({"type": "cmd", "id": id, "cmd": {"type": "leave"}}));
    }

    /// Member `id` was removed from table `t` with `chips`: the log, and
    /// their page (if it hasn't left yet). `how`: {by, idle?}; `put`, what
    /// they brought.
    #[allow(clippy::too_many_arguments)]
    fn removed(
        &mut self,
        t: &Table,
        id: &str,
        how: &Value,
        chips: u64,
        put: Option<u64>,
        now: u64,
        out: &mut TableStep,
    ) {
        let name = self.member_name(id);
        let by = how
            .get("by")
            .and_then(Value::as_str)
            .filter(|b| !b.is_empty());
        let idle = truthy(how.get("idle"));
        let who = match by {
            Some(b) if !self.manager(b) => self.member_name(b),
            _ => "The manager".into(),
        };
        let text = if idle {
            format!("{name} was idle, so they left {}", t.name)
        } else {
            format!("{who} removed {name} from {}", t.name)
        };
        let actor = by.map_or(self.book.club.host_id.clone(), str::to_string);
        self.book.log_action(&actor, &text, now, None);
        let mut u = json!({"type": "removed", "name": t.name, "chips": chips});
        if let Some(put) = put {
            u["net"] = (chips as i64 - put as i64).into();
        }
        if idle {
            u["idle"] = true.into();
        }
        out.send(id, u);
    }

    /// Member `id` left their table with `chips` for the seat held at table
    /// `to`: their page sits them there. Their `hold_ms` to sit down start now.
    fn moved_out(
        &mut self,
        id: &str,
        to: &str,
        chips: u64,
        now: u64,
        hold_ms: u64,
        out: &mut TableStep,
    ) {
        let held = self.book.club.hold_from(to, id, now, hold_ms);
        out.arm_holds = true;
        let name = self.book.club.table(to).map_or("", |d| d.name.as_str());
        let mut u = json!({"type": "moved", "to": to, "name": name, "chips": chips});
        if held {
            u["until"] = (now + hold_ms).into();
        }
        out.send(id, u);
    }

    /// Takes in a result from table `tid`'s runner (`res`: players,
    /// departed, refunds, state, playersIn, chipRequests). `hold_ms`: how
    /// long someone who left for a held seat has to sit down there.
    pub fn table_result(
        &mut self,
        s: &mut Seating,
        tid: &str,
        res: &Value,
        now: u64,
        hold_ms: u64,
    ) -> Result<TableStep, RuleError> {
        let t = self.table_of(tid)?;
        let mut out = TableStep::default();
        let before = s.players.len();
        s.players = serde_json::from_value(res.get("players").cloned().unwrap_or(json!([])))
            .map_err(|e| RuleError(e.to_string()))?;
        let mut changed = false;
        // Chips people left with go back to their club balance.
        for d in list(res, "departed") {
            let id = d
                .get("client_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let chips = count(d.get("chips"));
            s.removed.retain(|x| *x != id);
            let here = self.seat_at(&id, tid).cloned();
            let removing = here
                .as_ref()
                .and_then(|x| x.get("removing"))
                .filter(|v| truthy(Some(v)))
                .cloned();
            let put = self
                .seat_of(&id)
                .and_then(|x| x.get("put"))
                .and_then(Value::as_u64);
            // Leaving for a seat held at another table (#425).
            let to = here
                .as_ref()
                .and_then(|x| x.get("moving"))
                .and_then(|m| m.get("to"))
                .and_then(Value::as_str)
                .filter(|x| !x.is_empty())
                .map(str::to_string)
                .or_else(|| s.moves.get(&id).and_then(Value::as_str).map(str::to_string));
            s.moves.remove(&id);
            s.leaving.retain(|x| *x != id);
            if self
                .book
                .left_table(&id, chips, now)
                .map_err(|e| RuleError(e.message))?
                .is_some()
            {
                changed = true;
                out.chips(&id);
            }
            if let Some(how) = removing {
                self.removed(&t, &id, &how, chips, put, now, &mut out);
            } else if let Some(to) = to {
                self.moved_out(&id, &to, chips, now, hold_ms, &mut out);
                changed = true;
            }
        }
        // Top-ups the table capped at its maximum: the rest goes back.
        for f in list(res, "refunds") {
            let id = f.get("client_id").and_then(Value::as_str).unwrap_or("");
            let chips = count(f.get("chips"));
            if self.seat_of(id).is_none() || chips == 0 {
                continue;
            }
            self.book
                .refund_top_up(id, Some(chips as i64), now)
                .map_err(|e| RuleError(e.message))?;
            changed = true;
            out.chips(id);
        }
        let state = res.get("state").cloned().unwrap_or(Value::Null);
        s.complete = truthy(state.get("complete"));
        s.hand_number = count(state.get("hand_number"));
        out.on_play = !s.complete;
        // A hand just finished: for the host's hand history.
        if s.complete && s.hand_number > 0 && s.hand_number != s.recorded {
            s.recorded = s.hand_number;
            out.on_hand = true;
        }
        s.players_in = count(res.get("playersIn"));
        let players = s.players.clone();
        if s.complete {
            self.sync_stacks(&players, s.hand_number);
            // Anyone the manager is removing stands up now (#414).
            for p in &players {
                if self.removing(&p.client_id, tid) {
                    self.stand_up(s, tid, &p.client_id, &Map::new(), &mut out);
                }
            }
            // And anyone moving to another table leaves now (#425).
            for p in &players {
                if !self.removing(&p.client_id, tid) && self.move_of(&p.client_id, tid).is_some() {
                    self.leave_to_move(s, tid, &p.client_id, now, &mut out);
                }
            }
        }
        // The hit-and-run rule: each player hears how many hands they still
        // owe the table whenever that changes.
        if t.stay_hands > 0 {
            let done = s.completed();
            for p in &players {
                // Being removed: the manager overrides the rule.
                if self.removing(&p.client_id, tid) {
                    continue;
                }
                let left = self.book.club.hands_to_stay(&p.client_id, tid, done);
                let was = s.stay.get(&p.client_id).map(|v| count(Some(v)));
                if was == Some(left) {
                    continue;
                }
                s.stay.insert(p.client_id.clone(), left.into());
                out.send(&p.client_id, json!({"type": "stay", "hands": left}));
                // Owing hands, sitting out or dropping off blinds them off
                // rather than taking them out of the count.
                if (was.unwrap_or(0) > 0) != (left > 0) {
                    out.events
                        .push(json!({"type": "mustPlay", "id": p.client_id, "on": left > 0}));
                }
            }
        }
        // Chip requests: approved if the club's book allows them.
        let (min, max) = t.limits();
        for q in list(res, "chipRequests") {
            let seat = count(q.get("seat")) as u32;
            if s.asking.contains(&seat) {
                continue;
            }
            s.asking.push(seat);
            let who = players
                .iter()
                .find(|p| p.seat == seat)
                .map(|p| p.client_id.clone());
            let amount = q.get("amount").and_then(|a| {
                a.as_i64()
                    .or_else(|| a.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64))
            });
            let answer = match &who {
                None => Err(("No one in that seat.".to_string(), false)),
                Some(id) => self
                    .book
                    .chips_for_table(id, tid, amount, Limits { min, max }, now)
                    .map_err(|e| (e.message, e.short)),
            };
            out.events
                .push(json!({"type": "chips", "seat": seat, "ok": answer.is_ok()}));
            match (answer, who) {
                (Ok(_), Some(id)) => {
                    changed = true;
                    out.chips(&id);
                }
                (Err((why, ask)), Some(id)) => {
                    let mut u = json!({"type": "rejected", "reason": why, "chips": true});
                    // Too little available: the panel offers to ask the manager.
                    if ask {
                        u["ask"] = true.into();
                    }
                    out.send(&id, u);
                }
                _ => {}
            }
        }
        // Save when chips moved or a hand ended, not on every tick.
        if changed || (s.complete && s.saved != Some(s.hand_number)) {
            s.saved = Some(s.hand_number);
            out.save = true;
        }
        if before != s.players.len() {
            out.tables_changed = true;
        }
        Ok(out)
    }

    /// Runs `op` for table `tid` with `args`: `result` ({res, holdMs}),
    /// `standUp` ({id, why}), `leaveToMove` ({id}) or `tooShort` ({id, to,
    /// chips}, its value the reason or "").
    pub fn seats(
        &mut self,
        s: &mut Seating,
        tid: &str,
        op: &str,
        args: &Value,
        now: u64,
    ) -> Result<TableStep, RuleError> {
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        match op {
            "result" => {
                let hold = args.get("holdMs").and_then(Value::as_u64).unwrap_or(60_000);
                self.table_result(s, tid, args.get("res").unwrap_or(&Value::Null), now, hold)
            }
            "standUp" => {
                let mut out = TableStep::default();
                let why = args
                    .get("why")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                self.stand_up(s, tid, &id, &why, &mut out);
                Ok(out)
            }
            "leaveToMove" => {
                let mut out = TableStep::default();
                self.leave_to_move(s, tid, &id, now, &mut out);
                Ok(out)
            }
            "tooShort" => {
                let to = args.get("to").and_then(Value::as_str).unwrap_or("");
                Ok(TableStep {
                    value: self
                        .too_short(&id, to, count(args.get("chips")), now)
                        .into(),
                    ..TableStep::default()
                })
            }
            _ => refuse(format!("no table op {op}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: &str = "t1";

    // A club with members `ids`, 1000 chips each, and two tables: t1 (the
    // hit-and-run rule at `stay` hands) and t2, both buying in for 40-200.
    fn club(ids: &[&str], stay: u32) -> ClubState {
        let mut members = Map::new();
        for id in ids {
            members.insert(
                (*id).into(),
                json!({"id": id, "name": id[..1].to_uppercase() + &id[1..], "status": "member"}),
            );
        }
        let table = |id: &str, name: &str, stay: u32| {
            json!({"id": id, "name": name, "game": "nlhe", "sb": 1, "bb": 2, "minBB": 20, "maxBB": 100,
                   "seats": 6, "stayHands": stay, "status": "open"})
        };
        let mut c: ClubState = serde_json::from_value(json!({
            "code": "ABCD-EFGH", "hostId": "HOST", "ratholeHours": 2, "members": members,
            "tables": [table("t1", "Main", stay), table("t2", "Side", 0)],
        }))
        .unwrap();
        for id in ids {
            c.book.issue(id, Some(1000), "", 0).unwrap();
        }
        c
    }

    fn sit(c: &mut ClubState, id: &str, chips: i64) {
        c.book
            .chips_for_table(id, T, Some(chips), Limits { min: 40, max: 200 }, 1)
            .unwrap();
    }

    fn player(id: &str, seat: u32, chips: impl Into<Value>) -> Value {
        let chips: Value = chips.into();
        json!({"client_id": id, "seat": seat, "chips": chips})
    }

    fn result(c: &mut ClubState, s: &mut Seating, res: Value) -> TableStep {
        c.table_result(s, T, &res, 100, 60_000).unwrap()
    }

    fn seat(c: &ClubState, id: &str) -> Map<String, Value> {
        c.seat_of(id).cloned().unwrap_or_default()
    }

    fn set_seat(c: &mut ClubState, id: &str, k: &str, v: Value) {
        let m = c.book.club.members.get_mut(id).unwrap();
        m.other["seat"][k] = v;
    }

    #[test]
    fn leaving_cashes_out_and_a_hand_over_syncs_the_stacks() {
        let mut c = club(&["ann", "bob"], 0);
        sit(&mut c, "ann", 100);
        sit(&mut c, "bob", 100);
        let mut s = Seating::default();
        let o = result(
            &mut c,
            &mut s,
            json!({"players": [player("ann", 0, 100), player("bob", 1, 100)], "state": {"complete": false, "hand_number": 1}, "playersIn": 2}),
        );
        assert!(o.on_play && !o.on_hand && o.tables_changed);
        // The hand ends: Ann won 30; the ledger takes the engine's count.
        let o = result(
            &mut c,
            &mut s,
            json!({"players": [player("ann", 0, "130"), player("bob", 1, 70)], "state": {"complete": true, "hand_number": 1}, "playersIn": 2}),
        );
        assert!(o.on_hand && o.save && !o.tables_changed);
        assert_eq!(seat(&c, "ann")["stack"], 130);
        assert_eq!(seat(&c, "ann")["upAt"], 1);
        // The same result again: nothing to record or save.
        let o = result(
            &mut c,
            &mut s,
            json!({"players": [player("ann", 0, 130), player("bob", 1, 70)], "state": {"complete": true, "hand_number": 1}}),
        );
        assert!(!o.on_hand && !o.save);
        // Bob leaves with 70: back to his balance, and the rathole rule starts.
        let o = result(
            &mut c,
            &mut s,
            json!({"players": [player("ann", 0, 130)], "departed": [{"client_id": "bob", "chips": 70}], "state": {"complete": true, "hand_number": 1}}),
        );
        assert_eq!(o.chips_changed, vec!["bob".to_string()]);
        assert!(o.save && o.tables_changed);
        assert_eq!(c.book.balance("bob"), 970);
        assert!(c.seat_of("bob").is_none());
        assert_eq!(c.book.rathole_min("bob", T, 200), 70);
    }

    #[test]
    fn chip_requests_and_refunds() {
        let mut c = club(&["ann", "bob"], 0);
        c.book.reclaim("bob", Some(900), "", 0).unwrap();
        let mut s = Seating::default();
        let o = result(
            &mut c,
            &mut s,
            json!({"players": [player("ann", 0, 0), player("bob", 1, 0)], "state": {"complete": true, "hand_number": 0},
                   "chipRequests": [{"seat": 0, "amount": 150}, {"seat": 1, "amount": 150}, {"seat": 4, "amount": 50}]}),
        );
        assert_eq!(
            o.events,
            vec![
                json!({"type": "chips", "seat": 0, "ok": true}),
                json!({"type": "chips", "seat": 1, "ok": false}),
                json!({"type": "chips", "seat": 4, "ok": false}),
            ]
        );
        assert_eq!(c.book.balance("ann"), 850);
        assert_eq!(o.chips_changed, vec!["ann".to_string()]);
        // Too little: Bob's page offers to ask the manager.
        assert_eq!(o.sends.len(), 1);
        assert_eq!(o.sends[0]["to"], "bob");
        assert_eq!(o.sends[0]["update"]["ask"], true);
        assert_eq!(
            o.sends[0]["update"]["reason"],
            "You have 1 chips in the club."
        );
        // Still being answered: not asked again.
        let o = result(
            &mut c,
            &mut s,
            json!({"players": [player("ann", 0, 150)], "state": {"complete": true}, "chipRequests": [{"seat": 0, "amount": 10}]}),
        );
        assert!(o.events.is_empty());
        // The table handed back 20 of a top-up.
        let o = result(
            &mut c,
            &mut s,
            json!({"players": [player("ann", 0, 150)], "state": {"complete": true}, "refunds": [{"client_id": "ann", "chips": 20}]}),
        );
        assert_eq!(c.book.balance("ann"), 870);
        assert_eq!(o.chips_changed, vec!["ann".to_string()]);
    }

    #[test]
    fn the_hit_and_run_count_is_told_as_it_changes() {
        let mut c = club(&["ann"], 3);
        sit(&mut c, "ann", 100);
        let mut s = Seating::default();
        let hand = |n: u64, chips: u64| json!({"players": [player("ann", 0, chips)], "state": {"complete": true, "hand_number": n}});
        // Even: nothing owed, said once.
        let o = result(&mut c, &mut s, hand(1, 100));
        assert_eq!(
            o.sends,
            vec![json!({"to": "ann", "update": {"type": "stay", "hands": 0}})]
        );
        assert!(o.events.is_empty());
        let o = result(&mut c, &mut s, hand(1, 100));
        assert!(o.sends.is_empty());
        // Up after hand 2: three hands to play, and the blinds hold her.
        let o = result(&mut c, &mut s, hand(2, 150));
        assert_eq!(
            o.sends,
            vec![json!({"to": "ann", "update": {"type": "stay", "hands": 3}})]
        );
        assert_eq!(
            o.events,
            vec![json!({"type": "mustPlay", "id": "ann", "on": true})]
        );
        let o = result(&mut c, &mut s, hand(3, 150));
        assert_eq!(o.sends[0]["update"]["hands"], 2);
        assert!(o.events.is_empty());
        let o = result(&mut c, &mut s, hand(5, 150));
        assert_eq!(o.sends[0]["update"]["hands"], 0);
        assert_eq!(
            o.events,
            vec![json!({"type": "mustPlay", "id": "ann", "on": false})]
        );
    }

    #[test]
    fn a_removal_stands_them_up_after_the_hand() {
        let mut c = club(&["ann", "bob"], 3);
        sit(&mut c, "ann", 100);
        sit(&mut c, "bob", 100);
        set_seat(&mut c, "ann", "removing", json!({"by": "bob", "at": 5}));
        let mut s = Seating::default();
        let both = |done: bool| json!({"players": [player("ann", 0, 160), player("bob", 1, 40)], "state": {"complete": done, "hand_number": 1}});
        // In the hand: she plays it out.
        let o = result(&mut c, &mut s, both(false));
        assert!(o.events.is_empty());
        // Over: the runner stands her up, once; the rule doesn't hold her.
        let o = result(&mut c, &mut s, both(true));
        assert_eq!(o.events, vec![json!({"type": "remove", "seat": 0})]);
        assert!(o.sends.is_empty());
        let o = result(&mut c, &mut s, both(true));
        assert!(o.events.is_empty());
        // She's gone with 160: logged, and her page hears what she won.
        let o = result(
            &mut c,
            &mut s,
            json!({"players": [player("bob", 1, 40)], "departed": [{"client_id": "ann", "chips": 160}], "state": {"complete": true, "hand_number": 1}}),
        );
        assert_eq!(
            o.sends[0],
            json!({"to": "ann", "update": {"type": "removed", "name": "Main", "chips": 160, "net": 60}})
        );
        assert_eq!(
            c.book.log.last().unwrap()["text"],
            "Bob removed Ann from Main"
        );
        assert!(s.removed.is_empty());
        // Idle, standing up mid-hand: her page leaves now.
        let mut s = Seating {
            complete: false,
            ..Seating::default()
        };
        s.players = serde_json::from_value(json!([player("bob", 1, 40)])).unwrap();
        let mut out = TableStep::default();
        let why = json!({"idle": true}).as_object().cloned().unwrap();
        c.stand_up(&mut s, T, "bob", &why, &mut out);
        assert_eq!(
            out.sends,
            vec![json!({"to": "bob", "update": {"type": "removed", "name": "Main", "idle": true}})]
        );
    }

    #[test]
    fn moving_to_another_table() {
        let mut c = club(&["ann", "bob"], 3);
        sit(&mut c, "ann", 100);
        sit(&mut c, "bob", 100);
        c.book
            .club
            .reserve_seat("ann", "t2", 0, Some(T), None, 1, 60_000)
            .unwrap();
        c.book
            .club
            .reserve_seat("bob", "t2", 0, Some(T), None, 1, 60_000)
            .unwrap();
        set_seat(&mut c, "ann", "moving", json!({"to": "t2"}));
        set_seat(&mut c, "bob", "moving", json!({"to": "t2"}));
        // Bob is up and owes hands: he stays, and his seat there goes.
        let mut s = Seating::default();
        let o = result(
            &mut c,
            &mut s,
            json!({"players": [player("ann", 0, 100), player("bob", 1, 150)], "state": {"complete": true, "hand_number": 1}}),
        );
        assert!(
            o.events
                .contains(&json!({"type": "cmd", "id": "ann", "cmd": {"type": "leave"}}))
        );
        assert!(o.events.contains(&json!({"type": "state"})));
        let refused = o
            .sends
            .iter()
            .find(|m| m["update"]["type"] == "reserve")
            .unwrap();
        assert_eq!(refused["to"], "bob");
        assert!(
            refused["update"]["refused"]
                .as_str()
                .unwrap()
                .starts_with("You’re up at this table, so you play 3 more hands")
        );
        assert!(c.move_of("bob", T).is_none());
        assert_eq!(c.book.club.held_seats("t2", 2, ""), 1);
        assert!(o.save && o.arm_holds && o.tables_changed);
        // Ann leaves once, and her seat there is held from now.
        let o = result(
            &mut c,
            &mut s,
            json!({"players": [player("ann", 0, 100), player("bob", 1, 150)], "state": {"complete": true, "hand_number": 1}}),
        );
        assert!(!o.events.iter().any(|e| e["type"] == "cmd"));
        let o = result(
            &mut c,
            &mut s,
            json!({"players": [player("bob", 1, 150)], "departed": [{"client_id": "ann", "chips": 100}], "state": {"complete": true, "hand_number": 1}}),
        );
        assert_eq!(
            o.sends[0],
            json!({"to": "ann", "update": {"type": "moved", "to": "t2", "name": "Side", "chips": 100, "until": 60_100}})
        );
        assert!(o.arm_holds && s.leaving.is_empty());
        // Too little to buy in at a table: they'd stay.
        assert_eq!(c.too_short("bob", "t2", 0, 100), "");
        c.book.club.tables[1].min_bb = 1000;
        assert_eq!(
            c.too_short("bob", "t2", 150, 100),
            "Not enough for Side · Staying here"
        );
    }

    #[test]
    fn ops() {
        let mut c = club(&["ann"], 0);
        let mut s = Seating::default();
        let o = c
            .seats(
                &mut s,
                T,
                "tooShort",
                &json!({"id": "ann", "to": "t2", "chips": 0}),
                1,
            )
            .unwrap();
        assert_eq!(o.value, "");
        assert!(
            c.seats(&mut s, T, "nope", &json!({}), 1)
                .unwrap_err()
                .0
                .contains("no table op")
        );
        assert!(
            c.seats(&mut s, "t9", "result", &json!({"res": {}}), 1)
                .unwrap_err()
                .0
                .contains("No such table")
        );
        // A Seating from the page, with fields it doesn't know of yet.
        let s: Seating = serde_json::from_value(json!({"players": [], "handNumber": 3})).unwrap();
        assert!(s.complete && s.hand_number == 3);
    }
}
