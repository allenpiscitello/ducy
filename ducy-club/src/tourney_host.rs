//! A club's running tournaments (ducy-web #456): what the club makes of
//! each one's start time and each result from its runner. At the start time
//! it starts, short of players it's cancelled with every buy-in back; a
//! tournament saved when the host stopped resumes only if it's in step with
//! the club; each result says who's knocked out, how it's going, what each
//! player sees, and at the end the prizes go into club balances.
//!
//! The caller (ducy-web's club-tourney-host.js) runs the tournament: its
//! runner, timers, saving and sending. It hands in the club and an op, and
//! carries out the [`TourneyStep`] that comes back.

use serde::Serialize;
use serde_json::{Value, json};

use crate::host::ClubState;
use crate::rules::{RuleError, refuse};
use crate::tourneys::{Tourney, TourneyStatus};

/// How many players the live standings show.
pub const LIVE_STANDINGS: usize = 20;

/// What the caller does after a step.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TourneyStep {
    /// Messages to send, in order: `{to, update}`, for the tournament's tables.
    pub sends: Vec<Value>,
    /// Events for the tournament's runner, in order.
    pub events: Vec<Value>,
    /// Members whose balance changed: send them their account.
    pub chips_changed: Vec<String>,
    /// The lobby changed (entries, places, status): save the club and tell
    /// members.
    pub changed: bool,
    /// Only the live standings moved.
    pub live_changed: bool,
    /// How it's going, for the lobby: {level, onBreak, stepEndsAt,
    /// playersLeft, handForHand, tables, standings}.
    pub live: Option<Value>,
    /// A hand is in play at one of its tables.
    pub on_play: bool,
    /// It's over (finished or cancelled): stop running it.
    pub over: bool,
    /// Its entries and players out, saved with its tables to check against
    /// on resuming: {entries, out}.
    pub counts: Value,
    /// What an op returns.
    pub value: Value,
}

impl TourneyStep {
    fn send(&mut self, to: &str, update: Value) {
        self.sends.push(json!({"to": to, "update": update}));
    }

    fn chips(&mut self, id: &str) {
        if !self.chips_changed.iter().any(|x| x == id) {
            self.chips_changed.push(id.into());
        }
    }
}

/// A count of chips from the runner: a whole number, or a numeric string;
/// anything else is 0.
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

fn list<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn text<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// The name of tournament `tid`'s tables, as members' pages know them.
pub fn tourney_table(tid: &str) -> String {
    format!("tourney:{tid}")
}

/// Players knocked out so far.
fn out_count(t: &Tourney) -> u64 {
    t.entries.iter().filter(|e| e.place.is_some()).count() as u64
}

fn counts(t: &Tourney) -> Value {
    json!({"entries": t.entries.len(), "out": out_count(t)})
}

/// Members still in, each once, in the order they entered.
fn still_in(t: &Tourney) -> Vec<String> {
    let mut ids: Vec<String> = vec![];
    for e in t.entries.iter().filter(|e| e.place.is_none()) {
        if !ids.contains(&e.member) {
            ids.push(e.member.clone());
        }
    }
    ids
}

impl ClubState {
    fn tourney(&self, tid: &str) -> Option<&Tourney> {
        self.tourneys.iter().find(|t| t.id == tid)
    }

    fn name_of(&self, id: &str) -> String {
        self.book
            .club
            .members
            .get(id)
            .map_or(id.to_string(), |m| m.name.clone())
    }

    /// Tournament `tid`'s runner set up: its config (the levels, the places
    /// paid, `seed` to shuffle by) and the players still in.
    fn tourney_create(&self, t: &Tourney, seed: u64) -> Value {
        let levels: Vec<Value> = serde_json::to_value(&t.structure)
            .ok()
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter(|s| !truthy(s.get("break")))
            .map(|s| json!({"sb": s.get("sb"), "bb": s.get("bb"), "ante": s.get("ante")}))
            .collect();
        let players: Vec<Value> = still_in(t)
            .iter()
            .map(|id| json!({"id": id, "name": self.name_of(id)}))
            .collect();
        let paid = t.prizes(&self.payout_tables).len();
        json!({
            "config": {"game": t.game, "tableSize": t.table_size, "startingStack": t.stack,
                       "levels": levels, "paid": paid, "seed": seed},
            "players": players,
        })
    }

    /// At tournament `tid`'s start time: too early, nothing (value "wait");
    /// too few registered, it's cancelled with every buy-in back
    /// ("cancelled"); else it starts, and the value is its runner's set-up
    /// ({config, players}).
    pub fn tourney_at_start(
        &mut self,
        tid: &str,
        seed: u64,
        now: u64,
    ) -> Result<TourneyStep, RuleError> {
        let mut out = TourneyStep::default();
        let Some(min) = self.tourney(tid).map(|t| t.min_players) else {
            out.value = "gone".into();
            return Ok(out);
        };
        let what = self.with_tourneys(|ts| ts.at_start(tid, now))?;
        match what {
            None => out.value = "wait".into(),
            Some("cancel") => {
                let why = format!("fewer than {min} registered");
                self.tourney_abandon(tid, &why, false, now, &mut out)?;
                out.value = "cancelled".into();
            }
            Some(_) => {
                self.with_tourneys(|ts| ts.start(tid, now))?;
                let t = self.tourney(tid).cloned().unwrap_or_default();
                out.value = self.tourney_create(&t, seed);
                out.changed = true;
            }
        }
        Ok(out)
    }

    /// Cancels tournament `tid`, running or not, with every buy-in back, and
    /// (`log`) says why in the club's log. Nothing if it's over already.
    pub fn tourney_abandon(
        &mut self,
        tid: &str,
        why: &str,
        log: bool,
        now: u64,
        out: &mut TourneyStep,
    ) -> Result<(), RuleError> {
        out.over = true;
        let Some(t) = self.tourney(tid) else {
            return Ok(());
        };
        if matches!(t.status, TourneyStatus::Cancelled | TourneyStatus::Finished) {
            return Ok(());
        }
        let name = t.name.clone();
        let mut who: Vec<String> = vec![];
        for e in &t.entries {
            if !who.contains(&e.member) {
                who.push(e.member.clone());
            }
        }
        let back = self.with_tourneys(|ts| ts.cancel(tid, why, now))?;
        for m in back {
            self.book
                .pay(&m.member, "refund", Some(m.amount as i64), tid, now)
                .map_err(|e| RuleError(e.message))?;
        }
        if log {
            let host = self.book.club.host_id.clone();
            self.book.log_action(
                &host,
                &format!("Tournament {name} was cancelled ({why}); every buy-in was refunded"),
                now,
                None,
            );
        }
        for id in who {
            out.chips(&id);
        }
        out.changed = true;
        Ok(())
    }

    /// A tournament that was running when the host stopped, and what was
    /// saved of it (`saved`: {entries, out}, or null if nothing usable): in
    /// step with the club, the value is true and the events bring back the
    /// players (those `online` join again, the rest are blinded off until
    /// they're back); otherwise it's cancelled, every buy-in back.
    pub fn tourney_resume(
        &mut self,
        tid: &str,
        saved: &Value,
        online: &[Value],
        now: u64,
    ) -> Result<TourneyStep, RuleError> {
        let mut out = TourneyStep::default();
        let Some(t) = self.tourney(tid).cloned() else {
            out.value = false.into();
            out.over = true;
            return Ok(out);
        };
        let why = if !saved.is_object() {
            Some("the club restarted, and its tables weren’t saved")
        } else if saved.get("entries").and_then(Value::as_u64) != Some(t.entries.len() as u64)
            || saved.get("out").and_then(Value::as_u64) != Some(out_count(&t))
        {
            Some("the club restarted, and its saved tables were out of step with it")
        } else {
            None
        };
        if let Some(why) = why {
            self.tourney_abandon(tid, why, true, now, &mut out)?;
            out.value = false.into();
            return Ok(out);
        }
        for id in still_in(&t) {
            if online.iter().any(|o| o.as_str() == Some(&id)) {
                let name = self.name_of(&id);
                out.events
                    .push(json!({"type": "cmd", "id": id, "cmd": {"type": "join", "name": name}}));
            } else {
                out.events.push(json!({"type": "disconnected", "id": id}));
            }
        }
        out.value = true.into();
        Ok(out)
    }

    /// A result from tournament `tid`'s runner (`res`: events {busted,
    /// level, winner}, out, tables, standings, playersLeft, handForHand),
    /// with where its clock is (`clock`: {level, onBreak, stepEndsAt}).
    pub fn tourney_result(
        &mut self,
        tid: &str,
        res: &Value,
        clock: &Value,
        now: u64,
    ) -> Result<TourneyStep, RuleError> {
        let mut out = TourneyStep::default();
        if self.tourney(tid).is_none() || !res.is_object() {
            return Ok(out);
        }
        let ev = res.get("events").cloned().unwrap_or(Value::Null);
        let mut lobby = false;
        for f in list(&ev, "busted") {
            let place = f.get("place").and_then(Value::as_u64).unwrap_or(0);
            self.with_tourneys(|ts| ts.knocked_out(tid, text(f, "id"), place))?;
            lobby = true;
        }
        if ev.get("level").is_some_and(|l| !l.is_null()) {
            lobby = true;
        }
        let mut standings: Vec<(u64, Value)> = list(res, "standings")
            .iter()
            .map(|s| {
                let stack = count(s.get("stack"));
                let table = s.get("at").and_then(|a| a.get("table")).cloned();
                (
                    stack,
                    json!({"id": s.get("id"), "name": s.get("name"), "stack": stack,
                           "table": table, "absent": truthy(s.get("absent"))}),
                )
            })
            .collect();
        standings.sort_by_key(|a| std::cmp::Reverse(a.0));
        let tables: Vec<Value> = list(res, "tables")
            .iter()
            .map(|x| json!({"id": x.get("id"), "players": x.get("players")}))
            .collect();
        out.live = Some(json!({
            "level": clock.get("level"), "onBreak": truthy(clock.get("onBreak")),
            "stepEndsAt": clock.get("stepEndsAt"), "playersLeft": res.get("playersLeft"),
            "handForHand": res.get("handForHand"), "tables": tables,
            "standings": standings.into_iter().take(LIVE_STANDINGS).map(|s| s.1).collect::<Vec<_>>(),
        }));
        for m in list(res, "out") {
            out.send(text(m, "to"), m.get("data").cloned().unwrap_or(Value::Null));
        }
        out.on_play = list(res, "tables").iter().any(|x| truthy(x.get("inHand")));
        if let Some(w) = ev
            .get("winner")
            .and_then(|w| w.get("id"))
            .and_then(Value::as_str)
        {
            self.tourney_finish(tid, w, now, &mut out)?;
        } else if lobby {
            out.changed = true;
        } else {
            out.live_changed = true;
        }
        if let Some(t) = self.tourney(tid) {
            out.counts = counts(t);
        }
        Ok(out)
    }

    /// One player has all the chips: places are final, and the prizes go
    /// into winners' club balances.
    fn tourney_finish(
        &mut self,
        tid: &str,
        winner: &str,
        now: u64,
        out: &mut TourneyStep,
    ) -> Result<(), RuleError> {
        let f = self.with_tourneys(|ts| ts.finish(tid, winner, now))?;
        for p in &f.paid {
            self.book
                .pay(&p.member, "prize", Some(p.amount as i64), tid, now)
                .map_err(|e| RuleError(e.message))?;
            out.chips(&p.member);
        }
        // What was paid beyond what was held: the added chips, issued now.
        self.book.chips.issued += f.issued;
        let name = self.tourney(tid).map_or(String::new(), |t| t.name.clone());
        let host = self.book.club.host_id.clone();
        let text = format!("{} won tournament {name}", self.name_of(winner));
        self.book.log_action(&host, &text, now, None);
        out.changed = true;
        out.over = true;
        Ok(())
    }

    /// Whether member `id`'s command for tournament `tid` goes to its
    /// runner (`running`: whether it's running here); if not, they're told
    /// why.
    pub fn tourney_check(&self, tid: &str, id: &str, running: bool) -> TourneyStep {
        let mut out = TourneyStep::default();
        let reason = match self.tourney(tid) {
            Some(t) if running && t.is_in(id) => None,
            Some(_) if running => Some("You aren’t in this tournament."),
            _ => Some("That tournament isn’t running."),
        };
        match reason {
            None => out.value = true.into(),
            Some(r) => {
                out.value = false.into();
                out.send(id, json!({"type": "rejected", "reason": r, "fatal": true}));
            }
        }
        out
    }

    /// A late registration or a re-entry (#80) while `level` is being
    /// played: the buy-in down their path, then a starting stack at the
    /// shortest table.
    pub fn tourney_enter(
        &mut self,
        tid: &str,
        id: &str,
        level: u64,
        now: u64,
    ) -> Result<TourneyStep, RuleError> {
        let mut out = TourneyStep::default();
        let buy_in = self.with_tourneys(|ts| ts.check_late_entry(tid, id, level))?;
        if buy_in > 0 {
            self.book
                .tourney_buy_in(id, tid, Some(buy_in as i64), now)
                .map_err(|e| RuleError(e.message))?;
        }
        let e = self.with_tourneys(|ts| ts.late_entry(tid, id, level, now))?;
        out.chips(id);
        out.changed = true;
        out.events
            .push(json!({"type": "addEntry", "id": id, "name": self.name_of(id)}));
        out.value = serde_json::to_value(e).unwrap_or(Value::Null);
        Ok(out)
    }

    /// Runs `op` for tournament `tid` with `args`: "atStart" {seed},
    /// "abandon" {why, log}, "resume" {saved, online}, "result" {res,
    /// clock}, "check" {id, running}, "enter" {id, level}, "playing" {id,
    /// running: [tid]} (the value: those of them member `id` is in).
    pub fn tourney_host(
        &mut self,
        tid: &str,
        op: &str,
        args: &Value,
        now: u64,
    ) -> Result<TourneyStep, RuleError> {
        let id = text(args, "id");
        match op {
            "atStart" => self.tourney_at_start(tid, count(args.get("seed")), now),
            "abandon" => {
                let mut out = TourneyStep::default();
                let log = args.get("log").is_none_or(|l| truthy(Some(l)));
                self.tourney_abandon(tid, text(args, "why"), log, now, &mut out)?;
                Ok(out)
            }
            "resume" => self.tourney_resume(
                tid,
                args.get("saved").unwrap_or(&Value::Null),
                list(args, "online"),
                now,
            ),
            "result" => self.tourney_result(
                tid,
                args.get("res").unwrap_or(&Value::Null),
                args.get("clock").unwrap_or(&Value::Null),
                now,
            ),
            "check" => Ok(self.tourney_check(tid, id, truthy(args.get("running")))),
            "enter" => {
                let level = args.get("level").and_then(Value::as_u64).unwrap_or(0);
                self.tourney_enter(tid, id, level, now)
            }
            "playing" => {
                let tids: Vec<Value> = list(args, "running")
                    .iter()
                    .filter(|r| {
                        r.as_str()
                            .and_then(|r| self.tourney(r))
                            .is_some_and(|t| t.is_in(id))
                    })
                    .cloned()
                    .collect();
                Ok(TourneyStep {
                    value: tids.into(),
                    ..TourneyStep::default()
                })
            }
            _ => refuse(format!("no tournament op {op}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    // A club with members `ids`, 1000 chips each, and a tournament T (buy-in
    // 100, 50 added, at least `min`, starting at 1000) they've all registered for.
    fn club(ids: &[&str], min: u64) -> (ClubState, String) {
        let mut members = serde_json::Map::new();
        for id in ids {
            members.insert(
                (*id).into(),
                json!({"id": id, "name": id[..1].to_uppercase() + &id[1..], "status": "member"}),
            );
        }
        let mut c: ClubState = serde_json::from_value(json!({
            "code": "ABCD-EFGH", "hostId": "HOST", "members": members, "tables": [],
        }))
        .unwrap();
        for id in ids {
            c.book.issue(id, Some(1000), "", 0).unwrap();
        }
        let s = json!({"name": "Sunday", "buyIn": 100, "added": 50, "stack": 1500,
                       "opensAt": 0, "startsAt": 1000, "minPlayers": min, "maxPlayers": 9,
                       "structure": [{"sb": 10, "bb": 20, "minutes": 10}, {"break": true, "minutes": 5}, {"sb": 20, "bb": 40, "ante": 5, "minutes": 10}]});
        let mut rng = StdRng::seed_from_u64(1);
        let tid = c
            .with_tourneys(|t| t.new_tourney(&s, 0, None, &mut rng))
            .unwrap()
            .id;
        for id in ids {
            c.book.tourney_buy_in(id, &tid, Some(100), 10).unwrap();
            c.with_tourneys(|t| t.register(&tid, id, 10)).unwrap();
        }
        (c, tid)
    }

    fn logged(c: &ClubState, s: &str) -> bool {
        c.book.log.iter().any(|l| text(l, "text").contains(s))
    }

    fn status(c: &ClubState, tid: &str) -> TourneyStatus {
        c.tourney(tid).unwrap().status
    }

    #[test]
    fn at_the_start_time_it_starts_with_its_runner_set_up() {
        let (mut c, tid) = club(&["ann", "bob", "cy"], 2);
        assert_eq!(c.tourney_at_start(&tid, 7, 999).unwrap().value, "wait");
        let o = c.tourney_at_start(&tid, 7, 1000).unwrap();
        assert!(o.changed && !o.over);
        assert_eq!(status(&c, &tid), TourneyStatus::Running);
        let cfg = &o.value["config"];
        assert_eq!(cfg["startingStack"], 1500);
        assert_eq!(cfg["seed"], 7);
        // Levels only, no breaks.
        assert_eq!(cfg["levels"].as_array().unwrap().len(), 2);
        assert_eq!(cfg["levels"][1]["ante"], 5.0);
        assert_eq!(o.value["players"][0], json!({"id": "ann", "name": "Ann"}));
        assert_eq!(o.value["players"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn short_of_players_it_is_cancelled_with_every_buy_in_back() {
        let (mut c, tid) = club(&["ann", "bob"], 3);
        assert_eq!(c.book.balance("ann"), 900);
        let o = c.tourney_at_start(&tid, 7, 1000).unwrap();
        assert_eq!(o.value, "cancelled");
        assert!(o.over && o.changed);
        assert_eq!(o.chips_changed, ["ann", "bob"]);
        assert_eq!(c.book.balance("ann"), 1000);
        assert_eq!(status(&c, &tid), TourneyStatus::Cancelled);
        // Not logged: nothing went wrong.
        assert!(!logged(&c, "cancelled"));
    }

    #[test]
    fn resuming_only_in_step_with_the_club() {
        let (mut c, tid) = club(&["ann", "bob", "cy"], 2);
        c.tourney_at_start(&tid, 7, 1000).unwrap();
        let o = c
            .tourney_resume(
                &tid,
                &json!({"entries": 3, "out": 0}),
                &[json!("bob")],
                2000,
            )
            .unwrap();
        assert_eq!(o.value, true);
        assert_eq!(
            o.events,
            [
                json!({"type": "disconnected", "id": "ann"}),
                json!({"type": "cmd", "id": "bob", "cmd": {"type": "join", "name": "Bob"}}),
                json!({"type": "disconnected", "id": "cy"}),
            ]
        );
        // Out of step: cancelled, refunded, and logged.
        let o = c
            .tourney_resume(&tid, &json!({"entries": 2, "out": 0}), &[], 2000)
            .unwrap();
        assert_eq!(o.value, false);
        assert!(o.over);
        assert_eq!(c.book.balance("cy"), 1000);
        assert!(logged(&c, "out of step"));
    }

    #[test]
    fn results_knock_players_out_and_the_winner_is_paid() {
        let (mut c, tid) = club(&["ann", "bob", "cy"], 2);
        c.tourney_at_start(&tid, 7, 1000).unwrap();
        let clock = json!({"level": 0, "onBreak": false, "stepEndsAt": 5000});
        // A hand in play: only the standings move.
        let o = c
            .tourney_result(
                &tid,
                &json!({"tables": [{"id": 1, "players": 3, "inHand": true}], "playersLeft": 3,
                        "standings": [{"id": "ann", "name": "Ann", "stack": "1400", "at": {"table": 1}},
                                      {"id": "bob", "name": "Bob", "stack": 1700, "at": {"table": 1}}],
                        "out": [{"to": "ann", "data": {"type": "state"}}]}),
                &clock,
                1100,
            )
            .unwrap();
        assert!(o.live_changed && !o.changed && o.on_play);
        let live = o.live.unwrap();
        assert_eq!(live["standings"][0]["id"], "bob");
        assert_eq!(live["standings"][1]["stack"], 1400);
        assert_eq!(o.sends, [json!({"to": "ann", "update": {"type": "state"}})]);
        // Cy is out third.
        let o = c
            .tourney_result(
                &tid,
                &json!({"events": {"busted": [{"id": "cy", "place": 3}]}}),
                &clock,
                1200,
            )
            .unwrap();
        assert!(o.changed && !o.over);
        assert_eq!(o.counts, json!({"entries": 3, "out": 1}));
        // Ann wins: 350 in prizes, the 50 added issued now.
        let issued = c.book.chips.issued;
        let o = c
            .tourney_result(
                &tid,
                &json!({"events": {"busted": [{"id": "bob", "place": 2}], "winner": {"id": "ann"}}}),
                &clock,
                1300,
            )
            .unwrap();
        assert!(o.over && o.changed);
        assert_eq!(status(&c, &tid), TourneyStatus::Finished);
        assert_eq!(c.book.chips.issued, issued + 50);
        let paid: u64 = c
            .tourney(&tid)
            .unwrap()
            .paid
            .as_ref()
            .unwrap()
            .iter()
            .map(|p| p.amount)
            .sum();
        assert_eq!(paid, 350);
        assert_eq!(c.book.balance("ann"), 900 + 350);
        assert!(o.chips_changed.contains(&"ann".to_string()));
        assert!(logged(&c, "Ann won tournament Sunday"));
    }

    #[test]
    fn commands_from_players_not_in_it_are_turned_away() {
        let (mut c, tid) = club(&["ann", "bob"], 2);
        assert_eq!(
            c.tourney_check(&tid, "ann", false).sends[0]["update"]["reason"],
            "That tournament isn’t running."
        );
        c.tourney_at_start(&tid, 7, 1000).unwrap();
        assert_eq!(c.tourney_check(&tid, "ann", true).value, true);
        c.tourney_result(
            &tid,
            &json!({"events": {"busted": [{"id": "bob", "place": 2}]}}),
            &json!({}),
            1100,
        )
        .unwrap();
        let o = c.tourney_check(&tid, "bob", true);
        assert_eq!(o.value, false);
        assert_eq!(
            o.sends[0]["update"]["reason"],
            "You aren’t in this tournament."
        );
        let o = c
            .tourney_host(
                "",
                "playing",
                &json!({"id": "ann", "running": [tid, "x"]}),
                1100,
            )
            .unwrap();
        assert_eq!(o.value, json!([tid]));
    }

    #[test]
    fn a_re_entry_takes_the_buy_in_and_adds_a_stack() {
        let (mut c, tid) = club(&["ann", "bob"], 2);
        c.tourneys[0].late_level = 2;
        c.tourneys[0].reentry = true;
        c.tourney_at_start(&tid, 7, 1000).unwrap();
        c.tourney_result(
            &tid,
            &json!({"events": {"busted": [{"id": "bob", "place": 2}]}}),
            &json!({}),
            1100,
        )
        .unwrap();
        let o = c.tourney_enter(&tid, "bob", 1, 1200).unwrap();
        assert_eq!(
            o.events,
            [json!({"type": "addEntry", "id": "bob", "name": "Bob"})]
        );
        assert_eq!(c.book.balance("bob"), 800);
        assert!(c.tourney(&tid).unwrap().is_in("bob"));
        // Closed after its last level.
        assert!(c.tourney_enter(&tid, "ann", 2, 1300).is_err());
    }
}
