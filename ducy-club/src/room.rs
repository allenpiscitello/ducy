//! An always-on club's room on the relay (ducy-web #93, #463): the checks on
//! requests to create a club there or move one in, what anyone may learn
//! from a club's code, what the super admin sees of it, its plan from its
//! registry record, and its play clock.
//!
//! The room (ducy-web's relay/src/club-room.js) keeps the storage, the
//! signatures (ducy-identity) and the club server; this decides.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::registry::{FREE_LIMITS, Record};

/// How old a create or import request may be (its seq is at least the time
/// it was signed).
pub const CREATE_WINDOW_MS: u64 = 5 * 60_000;
/// The longest pause in play that still counts as play.
pub const PLAY_GAP_MS: u64 = 60_000;
/// The most play time kept before it's reported.
pub const REPORT_MS: u64 = 5 * 60_000;
/// The most of a club's name kept from a request.
const MAX_NAME: usize = 50;

fn text(v: Option<&Value>) -> &str {
    v.and_then(Value::as_str).unwrap_or("")
}

fn recent(env: &Value, now: u64) -> Result<(), String> {
    let seq = env.get("seq").and_then(Value::as_f64);
    if seq.is_none_or(|s| (now as f64 - s).abs() > CREATE_WINDOW_MS as f64) {
        return Err("The request is too old; try again.".into());
    }
    Ok(())
}

/// A signed request (its signature checked) to create a club in room
/// `code`: `{type: "create_club", club, name}`. Its name, or why not.
pub fn check_create(env: &Value, code: &str, now: u64) -> Result<String, String> {
    let body = env.get("body").cloned().unwrap_or_default();
    if text(body.get("type")) != "create_club" || text(body.get("club")) != code {
        return Err("Not a request for this club.".into());
    }
    recent(env, now)?;
    Ok(text(body.get("name")).chars().take(MAX_NAME).collect())
}

/// A signed request (its signature checked) from a club's host to move it
/// into room `code`: `{type: "import_club", club, data, senders}`. The club's
/// name, or why not: it must be this club, from its host, with no one at a
/// table.
pub fn check_import(env: &Value, code: &str, now: u64) -> Result<String, String> {
    let body = env.get("body").cloned().unwrap_or_default();
    let data = body.get("data").cloned().unwrap_or_default();
    if text(body.get("type")) != "import_club"
        || text(body.get("club")) != code
        || text(data.get("code")) != code
    {
        return Err("Not a request for this club.".into());
    }
    recent(env, now)?;
    if text(data.get("hostId")) != text(env.get("from")) {
        return Err("Only the club’s host can move it.".into());
    }
    let members = data.get("members").and_then(Value::as_object);
    if members.is_none() || data.get("chips").is_none_or(Value::is_null) {
        return Err("That isn’t a club.".into());
    }
    if members.is_some_and(|m| {
        m.values()
            .any(|x| x.get("seat").is_some_and(|s| !s.is_null()))
    }) {
        return Err("People are still at tables. Close the tables first.".into());
    }
    Ok(text(data.get("name")).chars().take(MAX_NAME).collect())
}

/// What anyone may learn from a club's code: that it's a club; its name and
/// host ID only when it lets people ask to join with the code; and with an
/// `invite` token that's the club's (or an agent's), its About.
pub fn code_info(club: &Value, invite: &str) -> Value {
    let about = club
        .get("info")
        .and_then(|i| i.get("about"))
        .and_then(Value::as_str)
        .filter(|a| !a.is_empty());
    let invited = !invite.is_empty()
        && (text(club.get("invite")) == invite
            || club
                .get("members")
                .and_then(Value::as_object)
                .is_some_and(|ms| {
                    ms.values().any(|m| {
                        text(m.get("agent")) == "yes" && text(m.get("agentInvite")) == invite
                    })
                }));
    let mut out = json!({"kind": "club", "ask": false});
    if let (true, Some(a)) = (invited, about) {
        out["about"] = a.chars().take(2000).collect::<String>().into();
    }
    if club.get("codeJoin") == Some(&Value::Bool(true)) {
        out["ask"] = true.into();
        out["name"] = text(club.get("name")).into();
        out["hostId"] = club.get("hostId").cloned().unwrap_or(Value::Null);
    }
    out
}

/// A club server's plan and limits from its registry record: `{limits,
/// plan}`; an older club with no record keeps the free limits.
pub fn plan_of(record: Option<&Record>) -> Value {
    match record {
        None => json!({"limits": FREE_LIMITS, "plan": null}),
        Some(r) => json!({
            "limits": r.limits,
            "plan": {"plan": r.plan, "expires": r.expires, "temporary": r.temporary,
                     "provedAt": r.proved_at, "removesAt": r.removes_at()},
        }),
    }
}

/// A finished hand without its cards: no one's hole cards, board or events.
pub fn hand_no_cards(h: &Value) -> Value {
    let mut h = h.clone();
    if let Some(o) = h.as_object_mut() {
        o.insert("board".into(), json!([]));
        o.remove("board2");
        o.insert("events".into(), json!([]));
        o.insert("hidden".into(), true.into());
        if let Some(seats) = o.get_mut("seats").and_then(Value::as_array_mut) {
            for s in seats {
                s["cards"] = Value::Null;
            }
        }
    }
    h
}

fn members(club: &Value) -> Vec<&Value> {
    club.get("members")
        .and_then(Value::as_object)
        .map(|m| m.values().collect())
        .unwrap_or_default()
}

/// What the super admin sees of the people a club knows: [{id, name,
/// status, role, agent, balance, lastSeen}], the owner too. `seen`: when
/// each was last heard from.
pub fn admin_people(club: &Value, seen: &Map<String, Value>) -> Value {
    let last = |id: &str| seen.get(id).cloned().unwrap_or(Value::Null);
    let mut people: Vec<Value> = members(club)
        .iter()
        .map(|m| {
            let id = text(m.get("id"));
            json!({"id": id, "name": m.get("name"), "status": m.get("status"),
                   "role": m.get("role").cloned().unwrap_or(Value::Null),
                   "agent": m.get("agent").cloned().unwrap_or(Value::Null),
                   "balance": m.get("balance").cloned().unwrap_or(0.into()), "lastSeen": last(id)})
        })
        .collect();
    let owner = text(club.get("ownerId"));
    if !owner.is_empty() && club.get("members").and_then(|m| m.get(owner)).is_none() {
        people.push(json!({"id": owner, "name": "", "status": null, "role": "owner", "balance": null, "lastSeen": last(owner)}));
    }
    json!({"name": club.get("name"), "people": people})
}

/// What the super admin sees of player `id`'s part in a club: their member
/// record, ledger entries, chip requests, help requests and log lines.
pub fn admin_person(club: &Value, seen: &Map<String, Value>, id: &str) -> Value {
    let m = club.get("members").and_then(|ms| ms.get(id)).cloned();
    let last = seen.get(id).cloned().unwrap_or(Value::Null);
    let list = |v: Option<&Value>| v.and_then(Value::as_array).cloned().unwrap_or_default();
    let name = m
        .as_ref()
        .map(|m| text(m.get("name")).to_string())
        .unwrap_or_default();
    let member = m.map(|mut m| {
        m["lastSeen"] = last.clone();
        m
    });
    json!({
        "member": member,
        "lastSeen": last,
        "ledger": list(club.get("money").and_then(|x| x.get("ledger"))).into_iter()
            .filter(|e| text(e.get("member")) == id || text(e.get("backer")) == id).collect::<Vec<_>>(),
        "requests": list(club.get("requestLog")).into_iter().filter(|r| text(r.get("member")) == id).collect::<Vec<_>>(),
        "help": list(club.get("help")).into_iter().filter(|h| text(h.get("from")) == id).collect::<Vec<_>>(),
        "log": list(club.get("log")).into_iter()
            .filter(|e| text(e.get("by")) == id || (!name.is_empty() && text(e.get("text")).contains(&name)))
            .collect::<Vec<_>>(),
    })
}

/// Everything a club stores, read-only, for the super admin: the lobby chat
/// and finished hands' cards only with `reveal`. `hands`: the newest
/// finished hands, newest first.
pub fn admin_full(club: &Value, seen: &Map<String, Value>, reveal: bool, hands: &[Value]) -> Value {
    let last = |id: &str| seen.get(id).cloned().unwrap_or(Value::Null);
    let name_of = |id: &str| {
        club.get("members")
            .and_then(|m| m.get(id))
            .map_or("", |m| text(m.get("name")))
            .to_string()
    };
    let mut out = club.as_object().cloned().unwrap_or_default();
    for k in ["lobby", "members", "chips", "money", "promos"] {
        out.remove(k);
    }
    let chips = club.get("chips");
    let num = |v: Option<&Value>| v.cloned().filter(|x| !x.is_null()).unwrap_or(0.into());
    let money = club.get("money");
    let ledger: Vec<Value> = money
        .and_then(|m| m.get("ledger"))
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .map(|e| {
                    let mut e = e.clone();
                    let backer = text(e.get("backer")).to_string();
                    e["name"] = name_of(text(e.get("member"))).into();
                    e["backerName"] = if backer == "club" {
                        "Club".into()
                    } else {
                        name_of(&backer).into()
                    };
                    e
                })
                .collect()
        })
        .unwrap_or_default();
    let lobby = club
        .get("lobby")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let promos = club.get("promos").filter(|p| p.is_object()).map(|p| {
        let mut p = p.clone();
        if !reveal {
            p.as_object_mut().map(|o| o.remove("hands"));
        }
        p
    });
    out.insert(
        "members".into(),
        members(club)
            .into_iter()
            .map(|m| {
                let mut m = m.clone();
                m["lastSeen"] = last(text(m.get("id")));
                m
            })
            .collect::<Vec<_>>()
            .into(),
    );
    out.insert("ownerSeen".into(), last(text(club.get("ownerId"))));
    out.insert(
        "chips".into(),
        json!({"issued": num(chips.and_then(|c| c.get("issued"))), "reclaimed": num(chips.and_then(|c| c.get("reclaimed")))}),
    );
    out.insert(
        "links".into(),
        money
            .and_then(|m| m.get("links"))
            .cloned()
            .unwrap_or(json!([])),
    );
    out.insert("ledger".into(), ledger.into());
    out.insert("promos".into(), promos.unwrap_or(Value::Null));
    out.insert(
        "hands".into(),
        hands
            .iter()
            .map(|h| if reveal { h.clone() } else { hand_no_cards(h) })
            .collect::<Vec<_>>()
            .into(),
    );
    out.insert("revealed".into(), reveal.into());
    out.insert("lobbyCount".into(), lobby.len().into());
    out.insert(
        "lobby".into(),
        if reveal { lobby.into() } else { Value::Null },
    );
    Value::Object(out)
}

/// A club's play time: [`PlayClock::playing`] each time a hand is in play;
/// gaps up to [`PLAY_GAP_MS`] count, longer ones end a stretch. A report
/// (minutes, by when) comes at most every [`REPORT_MS`] of play, at the end
/// of a stretch, and at a new day (UTC).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PlayClock {
    /// Play counted, not yet reported, in ms.
    pub played: u64,
    /// When a hand was last in play.
    pub last_play: u64,
}

/// Play time to report: `minutes`, by time `at`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Usage {
    /// Minutes played.
    pub minutes: f64,
    /// When the last of it was.
    pub at: u64,
}

impl PlayClock {
    /// What's counted so far, to report, and the count starts again.
    pub fn report(&mut self) -> Option<Usage> {
        if self.played == 0 {
            return None;
        }
        let u = Usage {
            minutes: self.played as f64 / 60_000.0,
            at: self.last_play,
        };
        self.played = 0;
        Some(u)
    }

    /// A hand is in play at `now`: what to report, if anything.
    pub fn playing(&mut self, now: u64) -> Option<Usage> {
        let day = |t: u64| t / 86_400_000;
        let mut out = None;
        if now.saturating_sub(self.last_play) <= PLAY_GAP_MS && day(now) == day(self.last_play) {
            self.played += now - self.last_play;
        } else {
            out = self.report();
        }
        self.last_play = now;
        if self.played >= REPORT_MS {
            out = self.report().or(out);
        }
        out
    }

    /// The stretch has ended by `now` (no play for [`PLAY_GAP_MS`]): what to
    /// report; None while it goes on.
    pub fn check(&mut self, now: u64) -> Option<Usage> {
        if now.saturating_sub(self.last_play) > PLAY_GAP_MS {
            self.report()
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_import_requests() {
        let env = |body: Value, seq: u64| json!({"from": "HOST", "seq": seq, "body": body});
        let c = json!({"type": "create_club", "club": "AAAA", "name": "x".repeat(60)});
        assert_eq!(
            check_create(&env(c.clone(), 1000), "AAAA", 2000)
                .unwrap()
                .len(),
            50
        );
        assert!(
            check_create(&env(c.clone(), 0), "AAAA", CREATE_WINDOW_MS + 1)
                .unwrap_err()
                .contains("too old")
        );
        assert!(
            check_create(&env(c, 1000), "BBBB", 1000)
                .unwrap_err()
                .contains("Not a request")
        );
        let data = |seat: bool| {
            json!({"code": "AAAA", "hostId": "HOST", "name": "Owls", "chips": {},
                                       "members": {"a": {"seat": if seat { json!({"table": "t"}) } else { Value::Null }}}})
        };
        let i = |d: Value| json!({"type": "import_club", "club": "AAAA", "data": d});
        assert_eq!(
            check_import(&env(i(data(false)), 5), "AAAA", 5).unwrap(),
            "Owls"
        );
        assert!(
            check_import(&env(i(data(true)), 5), "AAAA", 5)
                .unwrap_err()
                .contains("still at tables")
        );
        let mut other = env(i(data(false)), 5);
        other["from"] = "NOPE".into();
        assert!(
            check_import(&other, "AAAA", 5)
                .unwrap_err()
                .contains("Only the club’s host")
        );
    }

    #[test]
    fn what_a_code_tells() {
        let club = json!({"name": "Owls", "hostId": "H", "invite": "inv", "info": {"about": "Fridays"},
                          "members": {"a": {"agent": "yes", "agentInvite": "ag"}}});
        assert_eq!(code_info(&club, ""), json!({"kind": "club", "ask": false}));
        assert_eq!(code_info(&club, "ag")["about"], "Fridays");
        let mut open = club.clone();
        open["codeJoin"] = true.into();
        assert_eq!(
            code_info(&open, "nope"),
            json!({"kind": "club", "ask": true, "name": "Owls", "hostId": "H"})
        );
    }

    #[test]
    fn what_the_admin_sees() {
        let club = json!({"name": "Owls", "ownerId": "own", "lobby": [1, 2],
            "members": {"a": {"id": "a", "name": "Ann", "status": "member", "balance": 5}},
            "money": {"ledger": [{"member": "a", "backer": "club"}], "links": []},
            "log": [{"by": "x", "text": "Ann sat"}], "promos": {"hands": [1]}});
        let seen: Map<String, Value> = serde_json::from_value(json!({"a": 9})).unwrap();
        let p = admin_people(&club, &seen);
        assert_eq!(p["people"][0]["lastSeen"], 9);
        assert_eq!(p["people"][1]["role"], "owner");
        let one = admin_person(&club, &seen, "a");
        assert_eq!(
            (
                one["ledger"].as_array().unwrap().len(),
                one["log"].as_array().unwrap().len()
            ),
            (1, 1)
        );
        let hand = json!({"board": ["Ah"], "events": [1], "seats": [{"cards": ["Kd"]}]});
        let full = admin_full(&club, &seen, false, std::slice::from_ref(&hand));
        assert_eq!(
            (full["lobby"].clone(), full["lobbyCount"].clone()),
            (Value::Null, json!(2))
        );
        assert_eq!(full["hands"][0]["seats"][0]["cards"], Value::Null);
        assert_eq!(full["ledger"][0]["backerName"], "Club");
        assert!(full["promos"].get("hands").is_none());
        assert_eq!(
            admin_full(&club, &seen, true, &[hand])["hands"][0]["board"][0],
            "Ah"
        );
    }

    #[test]
    fn the_play_clock_counts_stretches_of_play() {
        let mut c = PlayClock::default();
        assert_eq!(c.playing(1_000_000), None);
        assert_eq!(c.playing(1_030_000), None);
        // A long gap ends the stretch: its 30 s are reported.
        let u = c.playing(1_200_000).unwrap();
        assert_eq!((u.minutes, u.at), (0.5, 1_030_000));
        // Five minutes of play is reported as it goes.
        let mut out = None;
        for i in 1..=10 {
            out = c.playing(1_200_000 + i * 30_000).or(out);
        }
        assert_eq!(out.unwrap().minutes, 5.0);
        assert_eq!(c.check(1_500_000 + 10_000), None);
        c.playing(1_520_000);
        assert_eq!(
            c.check(1_520_000 + PLAY_GAP_MS + 1).unwrap().minutes,
            20_000.0 / 60_000.0
        );
        assert_eq!(plan_of(None)["limits"]["tables"], 2);
    }
}
