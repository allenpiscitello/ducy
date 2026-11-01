//! One-off private games (ducy-web #457, from its #394 and #403): a
//! tournament started from "Start a game" rather than a club's, played for
//! points, and a cash game's ledger, with their standings and who owes whom.
//!
//! The tournament itself is a club tournament ([`crate::tourneys`]) in a
//! stand-in club that lives only in the host's tab. These are the parts of
//! its own: names, the buy-in and prizes in points (`t.points = {buyIn,
//! prizes}`), what players see, the results and standings. Points are only
//! for settling up afterwards: no club chips move.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::rules::{Member, RuleError, refuse};
use crate::tourneys::{MAX_ENTRIES, Tourney, TourneyStatus};

/// The most points in a buy-in or a prize.
pub const MAX_POINTS: u64 = 1_000_000_000;
/// The host's own id in the stand-in club.
pub const HOST_ID: &str = "host";
/// The longest name.
pub const MAX_NAME: usize = 20;

/// A name as typed: trimmed, spaces run together, at most 20 characters.
pub fn clean_name(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_NAME)
        .collect()
}

/// `name` as nobody in `taken` has it (whatever the case): "Ann", then
/// "Ann 2", and so on.
pub fn unique_name(taken: &[String], name: &str) -> String {
    let taken: Vec<String> = taken.iter().map(|t| t.to_lowercase()).collect();
    if !taken.contains(&name.to_lowercase()) {
        return name.into();
    }
    (2..)
        .map(|n: usize| {
            let keep = MAX_NAME - n.to_string().len() - 1;
            format!("{} {n}", name.chars().take(keep).collect::<String>())
        })
        .find(|s| !taken.contains(&s.to_lowercase()))
        .expect("some number is free")
}

fn points(v: &Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| {
            v.as_f64()
                .filter(|f| f.fract() == 0.0 && *f >= 0.0)
                .map(|f| f as u64)
        })
        .filter(|&n| n <= MAX_POINTS)
}

/// A buy-in in points: a whole number, 0 or more (missing is 0).
pub fn check_buy_in(v: &Value) -> Result<u64, RuleError> {
    if v.is_null() {
        return Ok(0);
    }
    points(v).map_or_else(
        || refuse("The buy-in is a whole number of points, 0 or more."),
        Ok,
    )
}

/// The host's prizes: whole points by place from first, any amounts in any
/// order.
pub fn check_prizes(list: &Value) -> Result<Vec<u64>, RuleError> {
    let bad = || refuse("Prizes are whole numbers of points, 0 or more.");
    let Some(l) = list
        .as_array()
        .filter(|l| !l.is_empty() && l.len() as u64 <= MAX_ENTRIES)
    else {
        return bad();
    };
    l.iter()
        .map(points)
        .collect::<Option<Vec<_>>>()
        .map_or_else(bad, Ok)
}

/// A paid place and its share, in basis points.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Place {
    /// The place, from 1.
    pub place: u64,
    /// Its share.
    pub share: u64,
}

/// The paid places now: no more than the entries.
pub fn paid_places(t: &Tourney) -> Vec<Place> {
    t.payout_shares(&[])
        .into_iter()
        .take(t.entry_count().max(1) as usize)
        .enumerate()
        .map(|(i, share)| Place {
            place: i as u64 + 1,
            share,
        })
        .collect()
}

fn buy_in_of(t: &Tourney) -> u64 {
    t.other
        .get("points")
        .and_then(|p| p.get("buyIn"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

fn host_prizes(t: &Tourney) -> Option<Vec<u64>> {
    let list = t.other.get("points")?.get("prizes")?.as_array()?;
    Some(list.iter().map(|x| x.as_u64().unwrap_or(0)).collect())
}

/// The prizes the buy-ins make, split by the payout shares (first place
/// first); the rounding goes to first, so they add up to the entries.
pub fn auto_prizes(t: &Tourney) -> Vec<u64> {
    let total = t.entry_count() * buy_in_of(t);
    let mut out: Vec<u64> = paid_places(t)
        .iter()
        .map(|p| (u128::from(total) * u128::from(p.share) / 10_000) as u64)
        .collect();
    let given: u64 = out.iter().sum();
    out[0] += total - given;
    out
}

/// The prizes in force: the host's, or the split of the buy-ins.
pub fn prize_list(t: &Tourney) -> Vec<u64> {
    host_prizes(t).unwrap_or_else(|| auto_prizes(t))
}

/// The prizes' total against the entries': `added` below 0 when the
/// prizes pay out less than came in.
pub fn prize_totals(t: &Tourney) -> Value {
    let prizes: u64 = prize_list(t).iter().sum();
    let entries = t.entry_count() * buy_in_of(t);
    json!({"prizes": prizes, "entries": entries, "added": prizes as i64 - entries as i64})
}

/// What every player sees of the tournament (the lobby, how it's going, the
/// results), with `live` from the host while it runs.
pub fn lobby_body(
    code: &str,
    members: &BTreeMap<String, Member>,
    t: &Tourney,
    live: Option<&Value>,
) -> Value {
    let name = |id: &str| members.get(id).map_or("?".to_string(), |m| m.name.clone());
    let mut v = json!({
        "type": "t_lobby", "code": code, "host": name(HOST_ID),
        "id": t.id, "name": t.name, "game": t.game, "stack": t.stack, "tableSize": t.table_size, "structure": t.structure,
        "status": t.status, "why": t.why.clone().unwrap_or_default(), "startedAt": t.started_at, "endedAt": t.ended_at,
        "players": t.entries.iter().map(|e| json!({"id": e.member, "name": name(&e.member), "place": e.place})).collect::<Vec<_>>(),
        "count": t.player_count(), "paid": paid_places(t),
        "buyIn": buy_in_of(t), "prizes": prize_list(t), "custom": host_prizes(t).is_some(), "totals": prize_totals(t),
    });
    if let Some(l) = live.filter(|l| !l.is_null()) {
        v["live"] = l.clone();
    }
    v
}

/// The finishing places so far, best first: {place, id, name, share,
/// amount} (share and amount 0 out of the money; amount in points).
pub fn results(body: &Value) -> Vec<Value> {
    let shares: BTreeMap<u64, u64> = body["paid"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| Some((p["place"].as_u64()?, p["share"].as_u64()?)))
        .collect();
    let prizes = body["prizes"].as_array();
    let mut placed: Vec<&Value> = body["players"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["place"].as_u64().is_some())
        .collect();
    placed.sort_by_key(|p| p["place"].as_u64());
    placed
        .iter()
        .map(|p| {
            let place = p["place"].as_u64().unwrap_or(0);
            let amount = prizes
                .and_then(|l| l.get(place as usize - 1))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            json!({"place": place, "id": p["id"], "name": p["name"], "share": shares.get(&place).copied().unwrap_or(0), "amount": amount})
        })
        .collect()
}

/// Names in order as a reader sorts them: ignoring case first.
fn by_name(a: &str, b: &str) -> Ordering {
    a.to_lowercase()
        .cmp(&b.to_lowercase())
        .then_with(|| a.cmp(b))
}

/// The standings at the end, the biggest winner first: {id, name, buyIn,
/// won, added, net}. What the prizes pay beyond the entries comes from the
/// host (`added`, below 0 when they pay less), in their row, whether or not
/// they played. None before the end.
pub fn tourney_standings(body: &Value) -> Vec<Value> {
    if body["status"] != "finished" {
        return vec![];
    }
    let won: BTreeMap<String, i64> = results(body)
        .iter()
        .filter_map(|r| Some((r["id"].as_str()?.to_string(), r["amount"].as_i64()?)))
        .collect();
    let buy_in = body["buyIn"].as_i64().unwrap_or(0);
    // (id, name, buyIn, won, added)
    let mut rows: Vec<(String, String, i64, i64, i64)> = body["players"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            let id = p["id"].as_str().unwrap_or("").to_string();
            let w = won.get(&id).copied().unwrap_or(0);
            (
                id,
                p["name"].as_str().unwrap_or("").to_string(),
                buy_in,
                w,
                0,
            )
        })
        .collect();
    let added: i64 = rows.iter().map(|r| r.3 - r.2).sum();
    if added != 0 {
        match rows.iter_mut().find(|r| r.0 == HOST_ID) {
            Some(host) => host.4 = added,
            None => rows.push((
                HOST_ID.into(),
                body["host"].as_str().unwrap_or("").into(),
                0,
                0,
                added,
            )),
        }
    }
    let mut out: Vec<(Value, i64, String)> = rows
        .into_iter()
        .map(|(id, name, b, w, a)| {
            let net = w - b - a;
            (
                json!({"id": id, "name": name, "buyIn": b, "won": w, "added": a, "net": net}),
                net,
                name,
            )
        })
        .collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| by_name(&a.2, &b.2)));
    out.into_iter().map(|r| r.0).collect()
}

/// Who owes whom, from each row's `net`: the biggest loser pays the
/// biggest winner first, so there are as few payments as this simple way
/// makes. Nets that don't add up to 0 (a pot in play) leave the
/// difference unpaid.
pub fn settle_up(rows: &[Value]) -> Vec<Value> {
    let side = |sign: i64| {
        let mut v: Vec<(&Value, i64)> = rows
            .iter()
            .filter_map(|r| {
                let n = r["net"].as_i64()? * sign;
                (n > 0).then_some((r, n))
            })
            .collect();
        v.sort_by_key(|a| std::cmp::Reverse(a.1));
        v
    };
    let (mut owe, mut get) = (side(-1), side(1));
    let mut out = vec![];
    let (mut i, mut j) = (0, 0);
    while i < owe.len() && j < get.len() {
        let amount = owe[i].1.min(get[j].1);
        out.push(json!({"from": owe[i].0["id"], "fromName": owe[i].0["name"], "to": get[j].0["id"], "toName": get[j].0["name"], "amount": amount}));
        owe[i].1 -= amount;
        get[j].1 -= amount;
        if owe[i].1 == 0 {
            i += 1;
        }
        if get[j].1 == 0 {
            j += 1;
        }
    }
    out
}

/// One player in a cash game's ledger, in chips (hundredths).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LedgerPlayer {
    /// Their id.
    pub id: String,
    /// Their name.
    pub name: String,
    /// Everything the host gave them, less what was handed back.
    pub given: i64,
    /// What they left the table with.
    pub out: i64,
    /// What they have at the table now.
    pub at: i64,
}

/// A one-off cash game's ledger.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ledger {
    /// The players, by id.
    pub players: BTreeMap<String, LedgerPlayer>,
}

/// Someone at the table now, for [`Ledger::at_table`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Seated {
    /// Their id.
    pub id: String,
    /// Their name.
    pub name: String,
    /// Their chips.
    pub chips: i64,
}

impl Ledger {
    fn player(&mut self, id: &str, name: &str) -> &mut LedgerPlayer {
        let p = self
            .players
            .entry(id.into())
            .or_insert_with(|| LedgerPlayer {
                id: id.into(),
                name: if name.is_empty() {
                    "?".into()
                } else {
                    name.into()
                },
                ..Default::default()
            });
        if !name.is_empty() {
            p.name = name.into();
        }
        p
    }

    /// The host gave `id` `amount` chips: theirs at once.
    pub fn give(&mut self, id: &str, name: &str, amount: i64) {
        if amount <= 0 {
            return;
        }
        let p = self.player(id, name);
        p.given += amount;
        p.at += amount;
    }

    /// Chips approved that didn't fit under the table's maximum: handed
    /// back, so never given.
    pub fn hand_back(&mut self, id: &str, amount: i64) {
        if amount <= 0 {
            return;
        }
        if let Some(p) = self.players.get_mut(id) {
            p.given -= amount;
            p.at = (p.at - amount).max(0);
        }
    }

    /// `id` left the table with `chips`.
    pub fn cash_out(&mut self, id: &str, chips: i64) {
        if let Some(p) = self.players.get_mut(id) {
            p.out += chips.max(0);
            p.at = 0;
        }
    }

    /// Everyone's chips at the table now (between hands); anyone not listed
    /// has none there.
    pub fn at_table(&mut self, seated: &[Seated]) {
        for p in self.players.values_mut() {
            p.at = seated.iter().find(|s| s.id == p.id).map_or(0, |s| s.chips);
        }
        for s in seated {
            if !self.players.contains_key(&s.id) && s.chips > 0 {
                self.player(&s.id, &s.name).at = s.chips;
            }
        }
        for s in seated {
            if let Some(p) = self.players.get_mut(&s.id)
                && !s.name.is_empty()
            {
                p.name = s.name.clone();
            }
        }
    }

    /// The standings, the biggest winner first: {id, name, given, now, net}
    /// (now: at the table plus cashed out).
    pub fn standings(&self) -> Vec<Value> {
        let mut rows: Vec<&LedgerPlayer> = self.players.values().collect();
        let net = |p: &LedgerPlayer| p.at + p.out - p.given;
        rows.sort_by(|a, b| net(b).cmp(&net(a)).then_with(|| by_name(&a.name, &b.name)));
        rows.iter()
            .map(|p| json!({"id": p.id, "name": p.name, "given": p.given, "now": p.at + p.out, "net": net(p)}))
            .collect()
    }
}

/// The stand-in club's side of a one-off tournament (#456): making it,
/// players registering and leaving before the start, and the host's prizes.
impl crate::host::ClubState {
    fn the_tourney(&mut self) -> Result<&mut Tourney, RuleError> {
        match self.tourneys.first_mut() {
            Some(t) => Ok(t),
            None => refuse("No tournament here."),
        }
    }

    /// The tournament, from the host's setup `s`: {name, game, stack,
    /// tableSize, structure, buyIn (points)}, and `payout`: a list of
    /// shares, a payout table {rows}, or null for the defaults. It starts
    /// when the host starts it; anyone may register till then, the host
    /// too. Nothing is kept if the settings won't do.
    pub fn private_new(
        &mut self,
        s: &Value,
        payout: &Value,
        now: u64,
        rng: &mut rand::rngs::StdRng,
    ) -> Result<Tourney, RuleError> {
        if !self.tourneys.is_empty() {
            return refuse("This game has a tournament already.");
        }
        let buy_in = check_buy_in(s.get("buyIn").unwrap_or(&Value::Null))?;
        let host = self
            .book
            .club
            .members
            .get(HOST_ID)
            .map_or("Host".into(), |m| m.name.clone());
        let name = s
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map_or(format!("{host}’s tournament"), str::to_string);
        let spec = json!({
            "name": name, "game": s.get("game"), "buyIn": 0, "added": 0, "stack": s.get("stack"),
            "structure": s.get("structure"), "tableSize": s.get("tableSize"),
            "opensAt": now, "startsAt": now, "minPlayers": 2, "maxPlayers": MAX_ENTRIES,
        });
        let made = self.with_tourneys(|ts| {
            let t = ts.new_tourney(&spec, now, None, rng)?;
            if !payout.is_null() {
                ts.set_payout(&t.id, payout)?;
            }
            Ok(t.id)
        });
        let id = match made {
            Ok(id) => id,
            Err(e) => {
                self.tourneys.clear();
                return Err(e);
            }
        };
        let t = self.the_tourney()?;
        t.other
            .insert("points".into(), json!({"buyIn": buy_in, "prizes": null}));
        debug_assert_eq!(t.id, id);
        Ok(t.clone())
    }

    /// Player `id` registers as `name` before the start. Asking again (a
    /// reconnect) keeps their entry and takes the new name, made unique.
    pub fn private_join(&mut self, id: &str, name: &str, now: u64) -> Result<Member, RuleError> {
        let clean = clean_name(name);
        if clean.is_empty() {
            return refuse("Enter your name: it’s what the others see.");
        }
        let t = self.the_tourney()?;
        let (tid, scheduled) = (t.id.clone(), t.status == TourneyStatus::Scheduled);
        let entered = t.entries.iter().any(|e| e.member == id);
        let taken: Vec<String> = self
            .book
            .club
            .members
            .values()
            .filter(|m| m.id != id)
            .map(|m| m.name.clone())
            .collect();
        let name = unique_name(&taken, &clean);
        if entered && let Some(m) = self.book.club.members.get_mut(id) {
            if scheduled {
                m.name = name;
            }
            return Ok(m.clone());
        }
        if !scheduled {
            return refuse("This tournament has started: registration is closed.");
        }
        let m: Member = serde_json::from_value(
            json!({"id": id, "name": name, "status": "member", "balance": 0}),
        )
        .map_err(|e| RuleError(e.to_string()))?;
        self.book.club.members.insert(id.into(), m.clone());
        if let Err(e) = self.with_tourneys(|ts| ts.register(&tid, id, now)) {
            self.book.club.members.remove(id);
            return Err(e);
        }
        Ok(m)
    }

    /// Player `id` unregisters (or the host takes them off) before the
    /// start; the host keeps their name and goes on running it. Whether
    /// they were registered.
    pub fn private_leave(&mut self, id: &str) -> Result<bool, RuleError> {
        let Some(t) = self.tourneys.first() else {
            return Ok(false);
        };
        if t.status != TourneyStatus::Scheduled || !t.entries.iter().any(|e| e.member == id) {
            return Ok(false);
        }
        let tid = t.id.clone();
        self.with_tourneys(|ts| ts.unregister(&tid, id))?;
        if id != HOST_ID {
            self.book.club.members.remove(id);
        }
        Ok(true)
    }

    /// The host sets the prizes, whole points by place from first; null
    /// goes back to the split of the buy-ins. Until the end.
    pub fn private_prizes(&mut self, list: &Value) -> Result<Tourney, RuleError> {
        let prizes = if list.is_null() {
            Value::Null
        } else {
            json!(check_prizes(list)?)
        };
        let t = self.the_tourney()?;
        if matches!(t.status, TourneyStatus::Finished | TourneyStatus::Cancelled) {
            return refuse("It’s over already.");
        }
        let p = t
            .other
            .entry("points")
            .or_insert_with(|| json!({"buyIn": 0, "prizes": null}));
        p["prizes"] = prizes;
        Ok(t.clone())
    }

    /// Runs one-off op `op` with `args`: "new" {setup, payout}, "join" {id,
    /// name}, "leave" {id}, "prizes" {list}.
    pub fn private_tourney(
        &mut self,
        op: &str,
        args: &Value,
        now: u64,
        rng: &mut rand::rngs::StdRng,
    ) -> Result<Value, RuleError> {
        let s = |k: &str| {
            args.get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        let null = Value::Null;
        let v = |x: Result<Value, serde_json::Error>| x.map_err(|e| RuleError(e.to_string()));
        match op {
            "new" => v(serde_json::to_value(self.private_new(
                args.get("setup").unwrap_or(&null),
                args.get("payout").unwrap_or(&null),
                now,
                rng,
            )?)),
            "join" => v(serde_json::to_value(self.private_join(
                &s("id"),
                &s("name"),
                now,
            )?)),
            "leave" => Ok(self.private_leave(&s("id"))?.into()),
            "prizes" => v(serde_json::to_value(
                self.private_prizes(args.get("list").unwrap_or(&null))?,
            )),
            _ => refuse(format!("no one-off op {op}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cash_games_ledger_and_who_owes_whom() {
        let mut l = Ledger::default();
        l.give("host", "Ann", 20000);
        l.give("b", "Bo", 10000);
        l.give("c", "Cy", 10000);
        l.give("c", "Cy", 0);
        let seat = |id: &str, name: &str, chips| Seated {
            id: id.into(),
            name: name.into(),
            chips,
        };
        l.at_table(&[
            seat("host", "Ann", 18000),
            seat("b", "Bo", 22000),
            seat("c", "Cy", 0),
        ]);
        l.give("c", "Cy", 10000);
        l.give("b", "Bo", 5000);
        l.hand_back("b", 3000);
        l.at_table(&[
            seat("host", "Ann", 15000),
            seat("b", "Bo", 26000),
            seat("c", "Cy", 11000),
        ]);
        l.cash_out("c", 11000);
        l.at_table(&[seat("host", "Ann", 15000), seat("b", "Bobby", 26000)]);
        let rows = l.standings();
        let got: Vec<(String, i64, i64, i64)> = rows
            .iter()
            .map(|r| {
                (
                    r["name"].as_str().unwrap().into(),
                    r["given"].as_i64().unwrap(),
                    r["now"].as_i64().unwrap(),
                    r["net"].as_i64().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                ("Bobby".into(), 12000, 26000, 14000),
                ("Ann".into(), 20000, 15000, -5000),
                ("Cy".into(), 20000, 11000, -9000)
            ]
        );
        let pay = settle_up(&rows);
        let got: Vec<(&str, &str, i64)> = pay
            .iter()
            .map(|p| {
                (
                    p["fromName"].as_str().unwrap(),
                    p["toName"].as_str().unwrap(),
                    p["amount"].as_i64().unwrap(),
                )
            })
            .collect();
        assert_eq!(got, vec![("Cy", "Bobby", 9000), ("Ann", "Bobby", 5000)]);
    }

    #[test]
    fn settle_up_leaves_everyone_square() {
        let rows: Vec<Value> = [
            ("a", 5000),
            ("b", 2500),
            ("c", -4000),
            ("d", -2000),
            ("e", -1500),
            ("f", 0),
        ]
        .iter()
        .map(|(id, net)| json!({"id": id, "name": id.to_uppercase(), "net": net}))
        .collect();
        let pay = settle_up(&rows);
        let mut after: BTreeMap<String, i64> = rows
            .iter()
            .map(|r| (r["id"].as_str().unwrap().into(), r["net"].as_i64().unwrap()))
            .collect();
        for p in &pay {
            *after.get_mut(p["from"].as_str().unwrap()).unwrap() += p["amount"].as_i64().unwrap();
            *after.get_mut(p["to"].as_str().unwrap()).unwrap() -= p["amount"].as_i64().unwrap();
        }
        assert!(after.values().all(|&v| v == 0));
        assert!(pay.len() <= 4);
        let short = settle_up(&[
            json!({"id": "a", "name": "A", "net": 300}),
            json!({"id": "b", "name": "B", "net": -200}),
        ]);
        assert_eq!(short[0]["amount"], 200);
    }

    #[test]
    fn names_unique_whatever_the_case() {
        assert_eq!(clean_name("  Ann   Lee "), "Ann Lee");
        let taken = vec!["ann".to_string(), "Ann 2".to_string()];
        assert_eq!(unique_name(&taken, "Ann"), "Ann 3");
        assert_eq!(unique_name(&taken, "Bo"), "Bo");
        let long = "Abcdefghijklmnopqrst";
        assert_eq!(
            unique_name(&[long.to_lowercase()], long),
            "Abcdefghijklmnopqr 2"
        );
    }

    fn tourney(
        entries: usize,
        buy_in: u64,
        places: Option<Vec<u64>>,
        prizes: Option<Vec<u64>>,
    ) -> Tourney {
        let mut t: Tourney = serde_json::from_value(json!({
            "id": "t", "status": "scheduled",
            "entries": (0..entries).map(|i| json!({"n": i + 1, "member": format!("g{i}"), "at": 1, "place": null})).collect::<Vec<_>>(),
            "points": {"buyIn": buy_in, "prizes": prizes},
        }))
        .unwrap();
        if let Some(p) = places {
            t.payout = Some(crate::tourneys::Payout {
                rows: Some(vec![crate::tourneys::PayoutRow { from: 1, places: p }]),
                places: None,
            });
        }
        t
    }

    #[test]
    fn prizes_in_points_follow_the_entries_until_the_host_sets_them() {
        let t = tourney(10, 100, None, None);
        let auto = auto_prizes(&t);
        assert_eq!(auto.len(), 3);
        assert_eq!(auto.iter().sum::<u64>(), 1000);
        assert_eq!(
            prize_totals(&t),
            json!({"prizes": 1000, "entries": 1000, "added": 0})
        );
        assert_eq!(
            prize_list(&tourney(2, 100, Some(vec![6500, 3500]), None)),
            vec![130, 70]
        );
        assert_eq!(
            prize_list(&tourney(3, 7, Some(vec![5000, 3000, 2000]), None)),
            vec![11, 6, 4]
        );
        assert_eq!(prize_list(&tourney(4, 0, None, None)), vec![0, 0]);
        let own = tourney(10, 100, None, Some(vec![700, 400, 100]));
        assert_eq!(
            prize_totals(&own),
            json!({"prizes": 1200, "entries": 1000, "added": 200})
        );
        for bad in [
            json!([]),
            json!([-1]),
            json!([1.5]),
            json!(["10"]),
            json!("x"),
            json!([1e10]),
        ] {
            assert!(check_prizes(&bad).is_err(), "{bad}");
        }
        assert!(check_buy_in(&json!(-1)).is_err());
        assert!(check_buy_in(&json!(1.5)).is_err());
        assert_eq!(check_buy_in(&Value::Null), Ok(0));
    }

    #[test]
    fn standings_put_the_overlay_on_the_host() {
        let mut members = BTreeMap::new();
        for (id, name) in [("host", "Ann"), ("g0", "P0"), ("g1", "P1"), ("g2", "P2")] {
            members.insert(
                id.to_string(),
                serde_json::from_value::<Member>(
                    json!({"id": id, "name": name, "status": "member"}),
                )
                .unwrap(),
            );
        }
        let mut t = tourney(3, 100, None, Some(vec![300, 200]));
        assert_eq!(
            tourney_standings(&lobby_body("C", &members, &t, None)),
            Vec::<Value>::new()
        );
        for (i, e) in t.entries.iter_mut().enumerate() {
            e.place = Some(i as u64 + 1);
        }
        t.status = crate::tourneys::TourneyStatus::Finished;
        let b = lobby_body("C", &members, &t, None);
        let r: Vec<(String, i64)> = results(&b)
            .iter()
            .map(|r| {
                (
                    r["name"].as_str().unwrap().into(),
                    r["amount"].as_i64().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            r,
            vec![("P0".into(), 300), ("P1".into(), 200), ("P2".into(), 0)]
        );
        let rows = tourney_standings(&b);
        let got: Vec<(String, i64, i64, i64, i64)> = rows
            .iter()
            .map(|r| {
                (
                    r["name"].as_str().unwrap().into(),
                    r["buyIn"].as_i64().unwrap(),
                    r["won"].as_i64().unwrap(),
                    r["added"].as_i64().unwrap(),
                    r["net"].as_i64().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                ("P0".into(), 100, 300, 0, 200),
                ("P1".into(), 100, 200, 0, 100),
                ("P2".into(), 100, 0, 0, -100),
                ("Ann".into(), 0, 0, 200, -200)
            ]
        );
        let pay = settle_up(&rows);
        assert_eq!(
            (
                pay[0]["fromName"].clone(),
                pay[0]["toName"].clone(),
                pay[0]["amount"].clone()
            ),
            (json!("Ann"), json!("P0"), json!(200))
        );
    }

    // The stand-in club, with the host as its first member.
    fn stand_in() -> crate::host::ClubState {
        serde_json::from_value(json!({
            "code": "ABCD-EFGH", "hostId": HOST_ID, "name": "Private game",
            "members": {HOST_ID: {"id": HOST_ID, "name": "Ann", "status": "member", "balance": 0}},
        }))
        .unwrap()
    }

    fn rng() -> rand::rngs::StdRng {
        use rand::SeedableRng;
        rand::rngs::StdRng::seed_from_u64(3)
    }

    const LEVELS: &str = r#"[{"sb": 10, "bb": 20, "minutes": 10}]"#;

    #[test]
    fn a_one_off_tournament_is_made_joined_and_left() {
        let mut c = stand_in();
        let setup = json!({"game": "nlhe", "stack": 1500, "tableSize": 6, "buyIn": 20,
                           "structure": serde_json::from_str::<Value>(LEVELS).unwrap()});
        // Settings that won't do leave nothing behind.
        assert!(
            c.private_new(&setup, &json!([5000]), 0, &mut rng())
                .is_err()
        );
        assert!(c.tourneys.is_empty());
        let t = c
            .private_new(&setup, &json!([7000, 3000]), 0, &mut rng())
            .unwrap();
        assert_eq!(t.name, "Ann’s tournament");
        assert_eq!(t.other["points"], json!({"buyIn": 20, "prizes": null}));
        assert!(
            c.private_new(&setup, &Value::Null, 0, &mut rng())
                .unwrap_err()
                .0
                .contains("already")
        );
        // Names are cleaned and made unique; asking again renames.
        assert!(c.private_join("b", "   ", 1).is_err());
        assert_eq!(c.private_join("b", "  ann  ", 1).unwrap().name, "ann 2");
        assert_eq!(c.private_join("b", "Bo", 2).unwrap().name, "Bo");
        assert_eq!(c.tourneys[0].entries.len(), 1);
        c.private_join(HOST_ID, "Ann", 2).unwrap();
        // Leaving: the player goes; the host stays a member.
        assert!(c.private_leave("b").unwrap());
        assert!(!c.book.club.members.contains_key("b"));
        assert!(c.private_leave(HOST_ID).unwrap());
        assert!(c.book.club.members.contains_key(HOST_ID));
        assert!(!c.private_leave("nobody").unwrap());
        // Prizes, until the end.
        let t = c.private_prizes(&json!([30, 10])).unwrap();
        assert_eq!(t.other["points"]["prizes"], json!([30, 10]));
        assert!(c.private_prizes(&json!(["x"])).is_err());
        c.tourneys[0].status = TourneyStatus::Running;
        assert!(
            c.private_join("d", "Di", 3)
                .unwrap_err()
                .0
                .contains("started")
        );
        c.tourneys[0].status = TourneyStatus::Finished;
        assert!(
            c.private_prizes(&Value::Null)
                .unwrap_err()
                .0
                .contains("over")
        );
    }
}
