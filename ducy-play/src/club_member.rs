//! The member's side of a club (ducy-web #456): what a member's page makes
//! of each message from the club's host, and the check that their own copy
//! of the ledger adds up to their balance.
//!
//! The page keeps a membership, `{code, hostId, invite, name, status,
//! hostSeq, …}`, and checks each envelope's signature itself (the browser's
//! own crypto). [`from_host`] says whether an envelope is worth checking:
//! from the club's host, newer than the last, and from the key the first
//! message pinned. Once its signature checks out, [`take_host_message`]
//! takes it in: the host's key is pinned, and each kind of message is
//! checked and cut to size before it's kept.

use serde_json::{Map, Value, json};

use crate::club_rules::{
    LOBBY_KEEP, MAX_EVENT_TEXT, MAX_EVENTS, MAX_INFO_TEXT, MAX_PLAYER_NAME, chat_name, clean_chat,
    cut, info_text,
};

/// The longest club name a member keeps.
pub const MAX_CLUB_NAME: usize = 50;
/// The most people a lobby lists.
const LOBBY_ONLINE: usize = 500;

const B58: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// A player ID: 8 Base58 digits.
pub fn is_player_id(s: &str) -> bool {
    s.chars().count() == 8 && s.chars().all(|c| B58.contains(c))
}

/// A number as JavaScript's `Number(v) || 0` reads it.
fn number(v: Option<&Value>) -> f64 {
    let n = match v {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) if s.trim().is_empty() => 0.0,
        Some(Value::String(s)) => s.trim().parse().unwrap_or(0.0),
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        _ => 0.0,
    };
    if n.is_finite() { n } else { 0.0 }
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// Text as JavaScript's `String(v || '')` gives it.
fn text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(v) if truthy(Some(v)) => match v {
            Value::Number(n) => n.to_string(),
            Value::Bool(_) => "true".into(),
            _ => "[object Object]".into(),
        },
        _ => String::new(),
    }
}

/// An array, or empty.
fn list(v: Option<&Value>) -> Value {
    v.filter(|x| x.is_array())
        .cloned()
        .unwrap_or_else(|| json!([]))
}

/// Whether envelope `env` is worth checking: from the club's host, newer
/// than the last message taken, and from the key the first one pinned.
pub fn from_host(mem: &Map<String, Value>, env: &Value) -> bool {
    let from = env.get("from").and_then(Value::as_str);
    let seq = number(env.get("seq"));
    if from.is_none() || from != mem.get("hostId").and_then(Value::as_str) {
        return false;
    }
    if seq <= number(mem.get("hostSeq")) {
        return false;
    }
    match mem.get("hostPub").filter(|p| !p.is_null()) {
        Some(p) => env.get("pub") == Some(p),
        None => true,
    }
}

/// The lobby as a member keeps it: who's online and the chat, checked and
/// cut to size.
fn read_lobby(b: &Value) -> Value {
    let arr = |k: &str| {
        b.get(k)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let online: Vec<Value> = arr("online")
        .into_iter()
        .take(LOBBY_ONLINE)
        .filter(|x| x.get("name").is_some_and(Value::is_string))
        .map(|x| {
            json!({
                "id": text(x.get("id")), "name": chat_name(&text(x.get("name"))),
                "role": if x.get("role").and_then(Value::as_str) == Some("host") { "host" } else { "" },
                "table": cut(&clean_chat(&text(x.get("table"))), 50),
            })
        })
        .collect();
    let chat_all = arr("chat");
    let chat: Vec<Value> = chat_all[chat_all.len().saturating_sub(LOBBY_KEEP)..]
        .iter()
        .filter(|x| x.get("text").is_some_and(Value::is_string))
        .map(|x| {
            json!({
                "id": text(x.get("id")), "name": chat_name(&text(x.get("name"))),
                "text": clean_chat(&text(x.get("text"))), "at": number(x.get("at")),
                "host": truthy(x.get("host")),
            })
        })
        .filter(|x| x["text"] != "")
        .collect();
    json!({"online": online, "chat": chat})
}

/// The club info as a member keeps it, cut to size, keeping when each
/// section changed.
fn read_info(x: Option<&Value>) -> Value {
    let Some(x) = x.filter(|x| x.is_object()) else {
        return Value::Null;
    };
    let t = |k: &str| info_text(&text(x.get(k)), MAX_INFO_TEXT);
    let safe = |v: Option<&Value>| {
        v.and_then(Value::as_f64)
            .filter(|n| n.fract() == 0.0 && *n > 0.0 && *n <= 9_007_199_254_740_991.0)
            .map(|n| n as u64)
    };
    let mut events: Vec<(u64, String)> = x
        .get("events")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    let at = safe(e.get("at"))?;
                    let s = info_text(&text(e.get("text")), MAX_EVENT_TEXT).replace('\n', " ");
                    (!s.is_empty()).then_some((at, s))
                })
                .collect()
        })
        .unwrap_or_default();
    events.sort_by_key(|e| e.0);
    events.truncate(MAX_EVENTS);
    let mut at = Map::new();
    for k in ["about", "events", "chips", "rules"] {
        if let Some(n) = safe(x.get("at").and_then(|a| a.get(k))) {
            at.insert(k.into(), n.into());
        }
    }
    json!({
        "about": t("about"), "chips": t("chips"), "rules": t("rules"),
        "events": events.into_iter().map(|(at, text)| json!({"at": at, "text": text})).collect::<Vec<_>>(),
        "at": at,
    })
}

/// Whether a member's own entries add up: on each link, each entry's
/// balance is the one before plus its amount, and the links' balances add
/// up to `balance`.
pub fn check_own_ledger(entries: &[Value], balance: i64) -> bool {
    let mut sorted: Vec<&Value> = entries.iter().collect();
    sorted.sort_by(|a, b| number(a.get("n")).total_cmp(&number(b.get("n"))));
    let mut links: Map<String, Value> = Map::new();
    for e in sorted {
        let Some(amount) = e.get("amount").and_then(Value::as_i64) else {
            return false;
        };
        let backer = text(e.get("backer"));
        let b = links.get(&backer).and_then(Value::as_i64).unwrap_or(0) + amount;
        if Some(b) != e.get("balance").and_then(Value::as_i64) {
            return false;
        }
        links.insert(backer, b.into());
    }
    links.values().filter_map(Value::as_i64).sum::<i64>() == balance
}

/// Takes in envelope `env` from the host, its signature checked (see
/// [`from_host`]): pins the host's key, and keeps what the message says.
/// Whether the membership changed.
pub fn take_host_message(mem: &mut Map<String, Value>, env: &Value) -> bool {
    mem.insert(
        "hostPub".into(),
        env.get("pub").cloned().unwrap_or(Value::Null),
    );
    let b = env
        .get("body")
        .filter(|b| b.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    if b.get("club") != mem.get("code") {
        return false;
    }
    let seq = env.get("seq").cloned().unwrap_or(Value::Null);
    let kind = b.get("type").and_then(Value::as_str).unwrap_or("");
    let mut set = |k: &str, v: Value| {
        mem.insert(k.into(), v);
    };
    // The club moved to an always-on room on the relay (#98), signed by the
    // host it had: from now on it signs as `hostId`.
    if kind == "host_moved" {
        let to = b.get("hostId").and_then(Value::as_str).unwrap_or("");
        if !is_player_id(to) || Some(to) == mem.get("hostId").and_then(Value::as_str) {
            return false;
        }
        mem.insert("hostId".into(), to.into());
        mem.insert("hostPub".into(), Value::Null);
        mem.insert("hostSeq".into(), 0.into());
        mem.insert("hostKey".into(), Value::Null);
        return true;
    }
    let known = [
        "status",
        "tables",
        "help",
        "lobby",
        "tourneys",
        "promos",
        "owner_view",
        "owner_result",
        "info",
        "promo_event",
        "group",
        "lists",
        "list",
        "tourney_error",
        "host_key",
        "table_error",
        "agent_role",
        "agent",
        "admin_result",
        "account",
    ];
    if !known.contains(&kind) {
        return false;
    }
    set("hostSeq", seq.clone());
    let short = |v: Option<&Value>, n: usize| -> Value { cut(&text(v), n).into() };
    match kind {
        "status" => {
            if truthy(b.get("name")) {
                set("name", short(b.get("name"), MAX_CLUB_NAME));
            }
            if b.get("status").and_then(Value::as_str) != Some("unknown") {
                set("status", b.get("status").cloned().unwrap_or(Value::Null));
            }
            // The name you asked for is someone else's here (#450).
            if truthy(b.get("nameTaken")) {
                set("nameTaken", short(b.get("nameTaken"), MAX_PLAYER_NAME));
            } else {
                mem.remove("nameTaken");
            }
        }
        "tables" => {
            set("tables", list(b.get("tables")));
            set("rathole", number(b.get("rathole")).into());
            // No backup of the club proved yet (#395).
            set(
                "temporary",
                (b.get("temporary") == Some(&Value::Bool(true))).into(),
            );
            // The games offered that aren't running, to sign up for.
            set("menu", list(b.get("menu")));
            // How many wait on each game's list (#518).
            set(
                "listCounts",
                b.get("lists")
                    .filter(|l| l.is_object())
                    .cloned()
                    .unwrap_or_else(|| json!({})),
            );
        }
        // The waitlists you're on (#518): each with its count and your
        // place, a seat offered, or when your place goes.
        "lists" => set("lists", list(b.get("lists"))),
        // A seat offered (offer), an offer missed (missed), off a list
        // (off), or a list refused (refused).
        "list" => {
            let k = b.get("kind").and_then(Value::as_str).unwrap_or("");
            let k = if ["offer", "missed", "off", "refused"].contains(&k) {
                k
            } else {
                "off"
            };
            set(
                "listEvent",
                json!({"kind": k, "key": short(b.get("key"), 60), "table": text(b.get("table")),
                       "name": short(b.get("name"), 60), "until": number(b.get("until")),
                       "reason": short(b.get("reason"), 200)}),
            );
        }
        "help" => {
            set("staff", list(b.get("staff")));
            set("help", list(b.get("mine")));
        }
        "lobby" => set("lobby", read_lobby(&b)),
        "tourneys" => set("tourneys", list(b.get("tourneys"))),
        "promos" => set(
            "promos",
            json!({"promos": list(b.get("promos")), "recent": list(b.get("recent"))}),
        ),
        // The owner's view of the whole club, and how their action went (#93).
        "owner_view" => {
            let Some(data) = b.get("data").filter(|d| d.is_object() || d.is_array()) else {
                return false;
            };
            let or = |k: &str, d: Value| b.get(k).filter(|v| truthy(Some(v))).cloned().unwrap_or(d);
            set(
                "mgr",
                json!({
                    "data": data, "seated": or("seated", json!({})), "running": or("running", json!([])),
                    "online": or("online", json!([])), "tourneys": or("tourneys", json!({})),
                    "plan": or("plan", Value::Null), "owner": b.get("owner") == Some(&Value::Bool(true)),
                    "seen": or("seen", json!({})),
                }),
            );
        }
        "owner_result" => set(
            "mgrResult",
            json!({"id": b.get("id"), "error": text(b.get("error")), "value": b.get("value").cloned().unwrap_or(Value::Null)}),
        ),
        "info" => set("info", read_info(b.get("info"))),
        "promo_event" => set(
            "promoEvent",
            json!({
                "kind": if b.get("kind").and_then(Value::as_str) == Some("award") { "award" } else { "lead" },
                "text": short(b.get("text"), 200), "at": seq,
            }),
        ),
        // Grouped tables (#501): "We're starting a new table. Join?" (ask),
        // the table it opened for you (open), or the question closed.
        "group" => {
            let k = b.get("kind").and_then(Value::as_str).unwrap_or("");
            let k = if ["ask", "open", "closed"].contains(&k) {
                k
            } else {
                "closed"
            };
            let group = text(b.get("group"));
            let name = short(b.get("name"), 60);
            set(
                "groupAsk",
                if k == "ask" {
                    json!({"group": group, "name": name, "until": number(b.get("until"))})
                } else {
                    Value::Null
                },
            );
            set(
                "groupEvent",
                json!({"kind": k, "group": group, "table": text(b.get("table")), "name": name,
                       "reason": short(b.get("reason"), 200)}),
            );
        }
        "tourney_error" => set(
            "tourneyError",
            json!({"tourney": b.get("tourney"), "reason": text(b.get("reason"))}),
        ),
        // The host's key for this session's encrypted table updates.
        "host_key" => set(
            "hostKey",
            b.get("key")
                .filter(|k| k.is_string())
                .cloned()
                .unwrap_or(Value::Null),
        ),
        "table_error" => set(
            "tableError",
            json!({"table": b.get("table"), "reason": text(b.get("reason"))}),
        ),
        "agent_role" => {
            let agent = b
                .get("agent")
                .filter(|a| truthy(Some(a)))
                .cloned()
                .unwrap_or(Value::Null);
            if agent.as_str() != Some("yes") {
                set("agentView", Value::Null);
            }
            set("agent", agent);
        }
        "agent" => {
            let g = |k: &str| b.get(k).cloned().unwrap_or(Value::Null);
            let owed = b
                .get("owed")
                .filter(|o| !o.is_null())
                .cloned()
                .unwrap_or_else(|| g("creditUsed"));
            set(
                "agentView",
                json!({
                    "own": g("own"), "shares": g("shares"), "total": g("total"), "limit": g("limit"),
                    "creditUsed": g("creditUsed"), "room": g("room"), "owed": owed, "invite": g("invite"),
                    "recruits": b.get("recruits").filter(|v| truthy(Some(v))).cloned().unwrap_or(json!([])),
                    "players": b.get("players").filter(|v| truthy(Some(v))).cloned().unwrap_or(json!([])),
                }),
            );
        }
        "admin_result" => set("adminNote", text(b.get("error")).into()),
        "account" => take_account(mem, &b, env),
        _ => return false,
    }
    true
}

/// An account message: the balance, credit and request, and the new
/// entries, with the signed message that brought them kept as a receipt.
fn take_account(mem: &mut Map<String, Value>, b: &Value, env: &Value) {
    let balance = number(b.get("balance"));
    mem.insert("balance".into(), balance.into());
    // A credit player's limit and what they may sit with (#404).
    mem.insert("credit".into(), number(b.get("credit")).into());
    let available = match b.get("available") {
        None | Some(Value::Null) => balance.max(0.0),
        a => number(a),
    };
    mem.insert("available".into(), available.into());
    mem.insert(
        "request".into(),
        b.get("request")
            .filter(|r| truthy(Some(r)))
            .cloned()
            .unwrap_or(Value::Null),
    );
    // A cancel sent while the host was away has reached them (#188).
    mem.remove("cancelQueued");
    if let Some(a) = b.get("answer") {
        mem.insert(
            "answer".into(),
            if truthy(Some(a)) {
                a.clone()
            } else {
                Value::Null
            },
        );
    }
    let entries_of = |m: &Map<String, Value>, k: &str| -> Vec<Value> {
        m.get(k)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    // The club moved to the money model's ledger (#454): the copy starts
    // again from its entries, and the older one is kept with its receipts.
    if b.get("ledger").and_then(Value::as_str) == Some("links")
        && mem.get("ledger").and_then(Value::as_str) != Some("links")
    {
        let old = entries_of(mem, "entries");
        if !old.is_empty() {
            let receipts = entries_of(mem, "receipts");
            mem.insert(
                "before".into(),
                json!({"entries": old, "receipts": receipts}),
            );
        }
        mem.insert("entries".into(), json!([]));
        mem.insert("receipts".into(), json!([]));
        mem.insert("ledger".into(), "links".into());
    }
    let mut entries = entries_of(mem, "entries");
    let have: Vec<i64> = entries
        .iter()
        .filter_map(|e| e.get("n").and_then(Value::as_i64))
        .collect();
    let fresh: Vec<Value> = b
        .get("entries")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|e| {
                    e.get("n")
                        .and_then(Value::as_i64)
                        .is_some_and(|n| n.abs() <= 9_007_199_254_740_991 && !have.contains(&n))
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let got = !fresh.is_empty();
    entries.extend(fresh);
    entries.sort_by_key(|e| e.get("n").and_then(Value::as_i64).unwrap_or(0));
    // Keep the signed messages that brought entries: they prove the balance.
    if got {
        let mut receipts = entries_of(mem, "receipts");
        receipts.push(env.clone());
        mem.insert("receipts".into(), receipts.into());
    }
    let ok = check_own_ledger(&entries, balance as i64);
    mem.insert("entries".into(), entries.into());
    mem.insert("ledgerOk".into(), ok.into());
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOST: &str = "HoSTabcd";

    fn mem() -> Map<String, Value> {
        json!({"code": "ABCD-EFGH", "hostId": HOST, "invite": "x", "name": "", "status": "new", "hostSeq": 0})
            .as_object()
            .unwrap()
            .clone()
    }

    fn env(seq: u64, body: Value) -> Value {
        json!({"from": HOST, "pub": "KEY1", "seq": seq, "body": body})
    }

    #[test]
    fn only_the_hosts_newer_messages_from_its_first_key() {
        let mut m = mem();
        assert!(from_host(&m, &env(5, json!({}))));
        assert!(!from_host(&m, &json!({"from": "someone1", "seq": 5})));
        let e = env(
            5,
            json!({"type": "status", "club": "ABCD-EFGH", "name": "Friday", "status": "member"}),
        );
        assert!(take_host_message(&mut m, &e));
        assert_eq!(
            (m["name"].clone(), m["status"].clone(), m["hostPub"].clone()),
            (json!("Friday"), json!("member"), json!("KEY1"))
        );
        // Replayed, or from another key with the same ID: not even checked.
        assert!(!from_host(&m, &env(5, json!({}))));
        let mut other = env(6, json!({}));
        other["pub"] = "KEY2".into();
        assert!(!from_host(&m, &other));
        // Another club's message isn't taken.
        assert!(!take_host_message(
            &mut m,
            &env(6, json!({"type": "status", "club": "ZZZZ-ZZZZ"}))
        ));
        // The club moves to an always-on room: it signs as its new ID.
        assert!(take_host_message(
            &mut m,
            &env(
                7,
                json!({"type": "host_moved", "club": "ABCD-EFGH", "hostId": "NewHost1"})
            )
        ));
        assert_eq!(
            (
                m["hostId"].clone(),
                m["hostSeq"].clone(),
                m["hostPub"].clone()
            ),
            (json!("NewHost1"), json!(0), Value::Null)
        );
        assert!(!take_host_message(
            &mut m,
            &env(
                8,
                json!({"type": "host_moved", "club": "ABCD-EFGH", "hostId": "bad id"})
            )
        ));
    }

    #[test]
    fn messages_are_checked_and_cut_to_size() {
        let mut m = mem();
        let long = "x".repeat(300);
        take_host_message(
            &mut m,
            &env(
                1,
                json!({"type": "group", "club": "ABCD-EFGH", "kind": "odd", "reason": long}),
            ),
        );
        assert_eq!(m["groupEvent"]["kind"], "closed");
        assert_eq!(m["groupEvent"]["reason"].as_str().unwrap().len(), 200);
        assert_eq!(m["groupAsk"], Value::Null);
        // Waitlists (#518): yours, and a seat offered.
        take_host_message(
            &mut m,
            &env(
                1,
                json!({"type": "lists", "club": "ABCD-EFGH", "lists": [{"key": "nlhe:1/2:9", "count": 3, "place": 2}]}),
            ),
        );
        assert_eq!(m["lists"][0]["place"], 2);
        take_host_message(
            &mut m,
            &env(
                1,
                json!({"type": "list", "club": "ABCD-EFGH", "kind": "offer", "key": "nlhe:1/2:9", "table": "t1", "name": "Main", "until": 5}),
            ),
        );
        assert_eq!(
            m["listEvent"],
            json!({"kind": "offer", "key": "nlhe:1/2:9", "table": "t1", "name": "Main", "until": 5.0, "reason": ""})
        );
        take_host_message(
            &mut m,
            &env(
                2,
                json!({"type": "lobby", "club": "ABCD-EFGH",
            "online": [{"id": "a", "name": "Ann", "role": "host"}, {"id": "b"}, null, {"name": "x".repeat(40), "role": "boss"}],
            "chat": [{"id": "1", "name": "", "text": "  hi  there ", "at": 5}, {"text": "   "}]}),
            ),
        );
        let online = m["lobby"]["online"].as_array().unwrap();
        assert_eq!(online.len(), 2);
        assert_eq!(
            (online[0]["role"].clone(), online[1]["role"].clone()),
            (json!("host"), json!(""))
        );
        assert_eq!(online[1]["name"].as_str().unwrap().len(), 20);
        assert_eq!(
            m["lobby"]["chat"],
            json!([{"id": "1", "name": "A player", "text": "hi there", "at": 5.0, "host": false}])
        );
        take_host_message(
            &mut m,
            &env(
                3,
                json!({"type": "info", "club": "ABCD-EFGH",
            "info": {"about": "Hi\r\n\r\n\r\nthere", "events": [{"at": 20, "text": "B"}, {"at": 10, "text": "A"}, {"at": -1, "text": "no"}], "at": {"about": 9, "rules": "x", "bogus": 1}}}),
            ),
        );
        assert_eq!(m["info"]["about"], "Hi\n\nthere");
        assert_eq!(
            m["info"]["events"],
            json!([{"at": 10, "text": "A"}, {"at": 20, "text": "B"}])
        );
        // The host's times are kept, and nothing else.
        assert_eq!(m["info"]["at"], json!({"about": 9}));
        take_host_message(
            &mut m,
            &env(
                4,
                json!({"type": "info", "club": "ABCD-EFGH", "info": "nope"}),
            ),
        );
        assert_eq!(m["info"], Value::Null);
        assert!(!take_host_message(
            &mut m,
            &env(4, json!({"type": "nope", "club": "ABCD-EFGH"}))
        ));
    }

    #[test]
    fn the_account_keeps_new_entries_with_receipts_and_checks_them() {
        let mut m = mem();
        let e1 = json!({"n": 1, "backer": "club", "amount": 500, "balance": 500});
        let e2 = json!({"n": 2, "backer": "club", "amount": -200, "balance": 300});
        let a = env(
            1,
            json!({"type": "account", "club": "ABCD-EFGH", "balance": 500, "ledger": "links", "entries": [e1]}),
        );
        take_host_message(&mut m, &a);
        assert_eq!(
            (m["ledgerOk"].clone(), m["available"].clone()),
            (json!(true), json!(500.0))
        );
        // The same entry again adds nothing, and brings no receipt.
        let b = env(
            2,
            json!({"type": "account", "club": "ABCD-EFGH", "balance": 300, "ledger": "links", "entries": [e1, e2]}),
        );
        take_host_message(&mut m, &b);
        assert_eq!(m["entries"].as_array().unwrap().len(), 2);
        assert_eq!(m["receipts"].as_array().unwrap().len(), 2);
        assert_eq!(m["ledgerOk"], true);
        // A balance the entries don't add up to is caught.
        take_host_message(
            &mut m,
            &env(
                3,
                json!({"type": "account", "club": "ABCD-EFGH", "balance": 999, "ledger": "links"}),
            ),
        );
        assert_eq!(m["ledgerOk"], false);
        assert!(!check_own_ledger(
            &[json!({"n": 1, "backer": "club", "amount": 5, "balance": 6})],
            6
        ));
    }
}
