//! Table talk from the bots: each Regular has a way of talking (a tone) and a
//! talkativeness, and says something now and then: hello when you sit down,
//! after a pot with you in it, after a big pot, when they offer to raise the
//! stakes, at the end of a heads-up challenge game, and back when you chat.
//! Heads-up they talk a lot more, as players do with only one person to talk
//! to.
//!
//! A bot's lines are its tone's plus its own, and a bot doesn't repeat what
//! it or anyone at the table said lately ([`line_for`]). The lines are
//! `chat_lines.json`: tones are friendly (nice, says nice hand), chatty
//! (cocky but fun), needle (trash talk), grump (complains about luck), cool
//! (says little, dry), math (talks in odds), coach (every hand a lesson) and
//! robot (the GTO bot).
//!
//! Moments a bot may speak at: `greet` (sitting down with you), `wonFromYou`
//! (took a pot you lost a lot in), `lostToYou` (you took a pot from them),
//! `bigWin` / `bigLoss` (a big pot that wasn't with you), `stakes` (offering
//! to raise the stakes), `gameWon` / `gameLost` (the end of a heads-up
//! challenge game, theirs to win or lose), `idle` (heads-up, between hands),
//! and replies to what you say in the chat: `hello`, `luck`, `nice`,
//! `thanks`, `laugh`, `gg`, `other`.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::num;

/// How a bot talks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Persona {
    /// Its tone.
    pub tone: String,
    /// How often it talks, 0 to 1.
    pub talk: f64,
    /// It tilts when losing a pot: `angry` (cusses, bleeped) or `swedish`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tilt: Option<String>,
}

type Pools = BTreeMap<String, Vec<String>>;
// Lines about who called: moment → `youCalled` / `theyCalled` → lines.
type CallPools = BTreeMap<String, BTreeMap<String, Vec<String>>>;

/// Every persona and line.
#[derive(Debug, Deserialize)]
pub struct Lines {
    /// Each Regular's persona.
    pub personas: BTreeMap<String, Persona>,
    /// The Bankroll Challenge's generic players (`g-<family>-<seed>`) talk by
    /// family, a clue to read (ducy-web #306).
    pub family_personas: BTreeMap<String, Persona>,
    /// Each tone's lines, by moment.
    pub lines: BTreeMap<String, Pools>,
    /// Each Regular's own lines, on top of their tone's, so no two sound
    /// alike. Not for losing moments while tilting.
    pub own: BTreeMap<String, Pools>,
    /// A tilting player's lines when losing, by tilt.
    pub tilt_lines: BTreeMap<String, Pools>,
    /// Lines that only fit when it's clear who called, by tone. A bot never
    /// says "good call" about its own call.
    pub call_lines: BTreeMap<String, CallPools>,
    /// And by tilt.
    pub tilt_call_lines: BTreeMap<String, CallPools>,
}

/// The personas and lines.
pub fn lines() -> &'static Lines {
    static L: OnceLock<Lines> = OnceLock::new();
    L.get_or_init(|| {
        serde_json::from_str(include_str!("chat_lines.json")).expect("chat_lines.json")
    })
}

fn default_persona() -> Persona {
    Persona {
        tone: "friendly".into(),
        talk: 0.3,
        tilt: None,
    }
}

// A generic player's family: `g-<family>-<digits>`.
fn family(id: &str) -> Option<&str> {
    let rest = id.strip_prefix("g-")?;
    let (fam, seed) = rest.split_once('-')?;
    let ok = !fam.is_empty()
        && fam.bytes().all(|b| b.is_ascii_lowercase())
        && !seed.is_empty()
        && seed.bytes().all(|b| b.is_ascii_digit());
    ok.then_some(fam)
}

// Every GTO bot talks as `gto`.
fn key(id: &str) -> &str {
    if id.starts_with("gto") { "gto" } else { id }
}

/// How `id` talks: its own persona, its family's, or a friendly one.
pub fn persona_of(id: &str) -> Persona {
    let l = lines();
    let p = match family(id) {
        Some(f) => l.family_personas.get(f),
        None => l.personas.get(key(id)),
    };
    p.cloned().unwrap_or_else(default_persona)
}

const LOSING: [&str; 3] = ["lostToYou", "bigLoss", "gameLost"];
const REPLIES: [&str; 7] = ["hello", "luck", "nice", "thanks", "laugh", "gg", "other"];
// Moments a bot always speaks at (when the chat is on), however quiet.
const ALWAYS: [&str; 2] = ["gameWon", "gameLost"];
// How much more they talk heads-up.
const HEADS_UP: f64 = 2.5;

/// Whether a bot speaks at `moment`: its talkativeness, more heads-up. A
/// reply to you is more likely than unprompted talk. On the ladder a bot
/// always says hello. `r` is a random draw in `0..1`.
pub fn speaks(id: &str, moment: &str, heads_up: bool, ladder: bool, r: f64) -> bool {
    if ALWAYS.contains(&moment) || (ladder && moment == "greet") {
        return true;
    }
    let talk = persona_of(id).talk;
    let mut p = talk * if heads_up { HEADS_UP } else { 1.0 };
    if REPLIES.contains(&moment) {
        p = p.max(0.5) * if heads_up { 2.0 } else { 1.0 };
    }
    if moment == "idle" {
        p *= 0.25;
    }
    r < p.min(0.95)
}

/// Every line a bot could say at `moment`: its tone's, its own, and with
/// `how` (`youCalled` or `theyCalled`) the lines about who called; without
/// it only lines that fit either way.
pub fn lines_for(id: &str, moment: &str, how: Option<&str>) -> Vec<String> {
    let l = lines();
    let persona = persona_of(id);
    let tilting = persona
        .tilt
        .as_ref()
        .filter(|_| LOSING.contains(&moment))
        .and_then(|t| l.tilt_lines.get(t)?.get(moment));
    let base = tilting.or_else(|| l.lines.get(&persona.tone)?.get(moment));
    let own = if tilting.is_some() {
        None
    } else {
        l.own.get(key(id)).and_then(|o| o.get(moment))
    };
    let calls = match (tilting, &persona.tilt) {
        (Some(_), Some(t)) => l.tilt_call_lines.get(t),
        _ => l.call_lines.get(&persona.tone),
    };
    let extra = how.and_then(|h| calls?.get(moment)?.get(h));
    [base, own, extra]
        .into_iter()
        .flatten()
        .flatten()
        .cloned()
        .collect()
}

/// What each bot said lately, and the table as a whole: a line isn't said
/// again while it's in either, as long as there's another to say.
pub const RECENT_BOT: usize = 10;
/// The table's.
pub const RECENT_TABLE: usize = 20;

/// A line for `moment`, one the bot (`mine`, what it said lately) and the
/// table (`table`) haven't heard lately when there is one. `r` is a random
/// draw in `0..1`; the caller keeps the recent lines.
pub fn line_for(
    id: &str,
    moment: &str,
    how: Option<&str>,
    mine: &[String],
    table: &[String],
    r: f64,
) -> Option<String> {
    let lines = lines_for(id, moment, how);
    if lines.is_empty() {
        return None;
    }
    let fresh: Vec<&String> = lines
        .iter()
        .filter(|l| !mine.contains(l) && !table.contains(l))
        .collect();
    let pool: Vec<&String> = if fresh.is_empty() {
        lines.iter().collect()
    } else {
        fresh
    };
    let i = ((r * pool.len() as f64).floor() as usize).min(pool.len() - 1);
    Some(pool[i].clone())
}

/// What a chat message from you is, for a bot's reply.
pub fn reply_moment(text: &str) -> &'static str {
    // Words only: every run of anything else is a space.
    let mut t = String::from(" ");
    let mut gap = false;
    for c in text.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == ' ' {
            t.push(c);
            gap = false;
        } else if !gap {
            t.push(' ');
            gap = true;
        }
    }
    t.push(' ');
    let has = |words: &[&str]| words.iter().any(|w| t.contains(&format!(" {w} ")));
    let hello = [
        "hi", "hello", "hey", "yo", "sup", "evening", "morning", "howdy",
    ];
    // "Hey, nice to meet you" is a hello, whatever follows it.
    if hello.iter().any(|w| t.starts_with(&format!(" {w} "))) {
        return "hello";
    }
    if has(&["gg", "good game"]) {
        return "gg";
    }
    if has(&["gl", "good luck", "glhf"]) {
        return "luck";
    }
    if has(&["nh", "nice hand", "wp", "well played", "nice", "good hand"]) {
        return "nice";
    }
    if has(&["ty", "thanks", "thank you", "thx"]) {
        return "thanks";
    }
    if has(&["lol", "haha", "lmao", "ha"]) {
        return "laugh";
    }
    if has(&hello) {
        return "hello";
    }
    "other"
}

/// Who called whom at the end of a hand, between you (seat 0) and bot
/// `seat`, from the hand's events: `youCalled` when you called the bot's last
/// bet or raise, `theyCalled` when the bot called yours.
pub fn call_shape(events: &[Value], seat: i64) -> Option<&'static str> {
    let mut bettor: Option<i64> = None;
    let mut callers: Vec<i64> = Vec::new();
    for e in events {
        let s = e.get("seat").and_then(Value::as_i64);
        match e.get("type").and_then(Value::as_str) {
            Some("bet" | "raise") => {
                bettor = s;
                callers.clear();
            }
            Some("call") if bettor.is_some() => callers.extend(s),
            _ => {}
        }
    }
    if bettor == Some(seat) && callers.contains(&0) {
        return Some("youCalled");
    }
    if bettor == Some(0) && callers.contains(&seat) {
        return Some("theyCalled");
    }
    None
}

/// Something for a bot to say after a hand.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Moment {
    /// The bot's seat.
    pub seat: usize,
    /// The moment.
    pub moment: &'static str,
    /// Who called, when it's clear (a pot between you and that bot).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub how: Option<&'static str>,
}

/// What a finished hand gives the bots to talk about: at most one, from the
/// seats' results (`net`, in chips; `empty` and `sitting_out` seats left out)
/// and the big blind. Seat 0 is you. With the hand's `events`, a pot between
/// you and that bot also says who called.
pub fn hand_moments(seats: &[Value], big_blind: f64, events: Option<&[Value]>) -> Vec<Moment> {
    let bb = big_blind.max(1.0);
    let net: Vec<f64> = seats.iter().map(|s| num(s.get("net"))).collect();
    let Some(&you) = net.first() else {
        return Vec::new();
    };
    let here = |s: &Value, k| s.get(k).and_then(Value::as_bool).unwrap_or(false);
    let bots: Vec<(usize, f64)> = seats
        .iter()
        .enumerate()
        .filter(|&(i, s)| i > 0 && !here(s, "empty") && !here(s, "sitting_out"))
        .map(|(i, _)| (i, net[i]))
        .collect();
    let Some(&first) = bots.first() else {
        return Vec::new();
    };
    let top = bots
        .iter()
        .fold(first, |a, &b| if b.1 > a.1 { b } else { a });
    let low = bots
        .iter()
        .fold(first, |a, &b| if b.1 < a.1 { b } else { a });
    let with_how = |seat: usize, moment| {
        let how = call_shape(events.unwrap_or(&[]), seat as i64);
        vec![Moment { seat, moment, how }]
    };
    let plain = |seat, moment| {
        vec![Moment {
            seat,
            moment,
            how: None,
        }]
    };
    if you <= -10.0 * bb && top.1 > 0.0 {
        return with_how(top.0, "wonFromYou");
    }
    if you >= 10.0 * bb && low.1 < 0.0 {
        return with_how(low.0, "lostToYou");
    }
    if top.1 >= 30.0 * bb {
        return plain(top.0, "bigWin");
    }
    if low.1 <= -30.0 * bb {
        return plain(low.0, "bigLoss");
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use serde_json::json;

    const MOMENTS: [&str; 16] = [
        "greet",
        "wonFromYou",
        "lostToYou",
        "bigWin",
        "bigLoss",
        "stakes",
        "gameWon",
        "gameLost",
        "idle",
        "hello",
        "luck",
        "nice",
        "thanks",
        "laugh",
        "gg",
        "other",
    ];
    const OFTEN: [&str; 7] = [
        "greet",
        "wonFromYou",
        "lostToYou",
        "bigWin",
        "bigLoss",
        "idle",
        "other",
    ];

    fn pool<'a>(m: &'a BTreeMap<String, Pools>, k: &str, moment: &str) -> &'a [String] {
        m.get(k)
            .and_then(|p| p.get(moment))
            .map_or(&[], Vec::as_slice)
    }

    #[test]
    fn every_tone_has_lines_for_every_moment_and_plenty_for_the_common_ones() {
        let l = lines();
        for (tone, lines) in &l.lines {
            for m in MOMENTS {
                assert!(lines.get(m).is_some_and(|x| !x.is_empty()), "{tone} {m}");
            }
            for m in OFTEN {
                assert!(lines[m].len() >= 6, "{tone} {m}: {}", lines[m].len());
            }
        }
        for p in l.personas.values().chain(l.family_personas.values()) {
            assert!(l.lines.contains_key(&p.tone), "{}", p.tone);
            if let Some(t) = &p.tilt {
                assert!(l.tilt_lines.contains_key(t), "{t}");
            }
        }
    }

    #[test]
    fn every_regular_has_lines_of_their_own_and_no_line_repeats_in_a_pool() {
        let l = lines();
        for id in l.personas.keys().filter(|id| *id != "gto") {
            for m in ["greet", "idle", "other"] {
                assert!(!pool(&l.own, id, m).is_empty(), "{id} {m}");
            }
        }
        for id in l.personas.keys() {
            for m in MOMENTS {
                for how in [None, Some("youCalled"), Some("theyCalled")] {
                    let p = lines_for(id, m, how);
                    assert_eq!(
                        p.iter().collect::<HashSet<_>>().len(),
                        p.len(),
                        "{id} {m} {how:?}: a repeated line"
                    );
                }
            }
        }
    }

    #[test]
    fn a_bot_doesnt_repeat_itself_or_what_the_table_just_heard() {
        let mut said: Vec<String> = Vec::new();
        let mut r: u64 = 0;
        for _ in 0..8 {
            r = (r * 9301 + 49297) % 233280;
            let line = line_for(
                "jen_silly",
                "greet",
                None,
                &said,
                &said,
                r as f64 / 233280.0,
            )
            .unwrap();
            said.push(line);
        }
        assert_eq!(
            said.iter().collect::<HashSet<_>>().len(),
            said.len(),
            "{said:?}"
        );
        let a = line_for("jen_silly", "lostToYou", None, &[], &[], 0.0).unwrap();
        let b = line_for(
            "andrew_favorable",
            "lostToYou",
            None,
            &[],
            std::slice::from_ref(&a),
            0.0,
        )
        .unwrap();
        assert_ne!(a, b);
        // With nothing fresh left, a line comes up again.
        let all = lines_for("rampart", "stakes", None);
        assert!(line_for("rampart", "stakes", None, &all, &[], 0.5).is_some());
        assert_eq!(line_for("rampart", "nothing", None, &[], &[], 0.5), None);
    }

    #[test]
    fn some_regulars_are_nice_and_some_trash_talk() {
        assert_eq!(persona_of("phil_bigmouth").tone, "needle");
        assert_eq!(persona_of("jen_silly").tone, "friendly");
        assert_eq!(persona_of("bungleman").tone, "needle");
        assert_eq!(persona_of("milk_king").tone, "friendly");
        assert_eq!(persona_of("gto-plo5").tone, "robot");
        assert_eq!(persona_of("gto-plo6").tone, "robot");
        assert_eq!(persona_of("someone_new").tone, "friendly");
        assert_eq!(persona_of("g-maniac-12").tone, "needle");
        assert_eq!(persona_of("g-rock-3").talk, 0.04);
        assert_eq!(persona_of("g-unknown-3"), default_persona());
        assert_eq!(
            persona_of("g-maniac-x").tone,
            "friendly",
            "not a generic player's id"
        );
    }

    #[test]
    fn danny_and_tommy_tilt_when_they_lose_a_pot() {
        let l = lines();
        assert_eq!(
            lines_for("danny_smallball", "greet", None)[0],
            pool(&l.lines, "friendly", "greet")[0]
        );
        for m in LOSING {
            assert_eq!(
                lines_for("danny_smallball", m, None),
                pool(&l.tilt_lines, "angry", m)
            );
            for line in lines_for("tommy_sweeden", m, None) {
                assert!(line.contains("Kuksugare"), "{m}: {line}");
            }
        }
        assert!(
            pool(&l.lines, "friendly", "lostToYou")
                .contains(&lines_for("jen_silly", "lostToYou", None)[0])
        );
    }

    #[test]
    fn lines_come_from_the_bots_own_tone_and_its_own_lines() {
        let l = lines();
        for id in ["phil_bigmouth", "jen_silly", "milk_king"] {
            for m in MOMENTS {
                let p = lines_for(id, m, None);
                let tone = persona_of(id).tone;
                for line in pool(&l.lines, &tone, m).iter().chain(pool(&l.own, id, m)) {
                    assert!(p.contains(line), "{id} {m}: {line}");
                }
            }
        }
        assert!(
            !lines_for("jen_silly", "greet", None).contains(&pool(&l.own, "milk_king", "greet")[0])
        );
    }

    #[test]
    fn they_talk_more_heads_up_and_always_at_the_end_of_a_challenge_game() {
        let rate = |id, heads_up| {
            (0..1000)
                .filter(|&i| speaks(id, "wonFromYou", heads_up, false, i as f64 / 1000.0))
                .count()
        };
        assert!(rate("tommy_sweeden", true) > rate("tommy_sweeden", false) * 2);
        assert!(rate("phil_bigmouth", false) > rate("tommy_sweeden", false));
        assert!(speaks("tommy_sweeden", "gameWon", false, false, 0.999));
        assert!(speaks("tommy_sweeden", "greet", false, true, 0.999));
        assert!(!speaks("tommy_sweeden", "greet", false, false, 0.999));
    }

    #[test]
    fn your_chat_is_read_for_a_reply() {
        assert_eq!(reply_moment("gl hf"), "luck");
        assert_eq!(reply_moment("Nice hand!"), "nice");
        assert_eq!(reply_moment("gg wp"), "gg");
        assert_eq!(reply_moment("hey there"), "hello");
        assert_eq!(reply_moment("hey, nice to meet you"), "hello");
        assert_eq!(reply_moment("ok nice one"), "nice");
        assert_eq!(reply_moment("lol"), "laugh");
        assert_eq!(reply_moment("what time is it"), "other");
        assert_eq!(
            reply_moment("good, game"),
            "other",
            "the comma keeps the words apart"
        );
    }

    #[test]
    fn a_hand_gives_the_bot_it_concerned_something_to_say() {
        let seats = |nets: &[i64]| nets.iter().map(|n| json!({"net": n})).collect::<Vec<_>>();
        let m = |seat, moment| Moment {
            seat,
            moment,
            how: None,
        };
        assert_eq!(
            hand_moments(&seats(&[-6000, -200, 6200]), 200.0, None),
            [m(2, "wonFromYou")]
        );
        assert_eq!(
            hand_moments(&seats(&[4000, -4000, 0]), 200.0, None),
            [m(1, "lostToYou")]
        );
        assert_eq!(
            hand_moments(&seats(&[0, 8000, -8000]), 200.0, None),
            [m(1, "bigWin")]
        );
        assert_eq!(hand_moments(&seats(&[-200, 300, -100]), 200.0, None), []);
        let mut away = seats(&[0, 8000, -8000]);
        away[1]["sitting_out"] = json!(true);
        assert_eq!(hand_moments(&away, 200.0, None), [m(2, "bigLoss")]);
    }

    #[test]
    fn a_bot_never_says_good_call_about_its_own_call() {
        let seats = |nets: &[i64]| nets.iter().map(|n| json!({"net": n})).collect::<Vec<_>>();
        let you_bet: Vec<Value> = serde_json::from_value(json!([
            {"type": "bet", "seat": 1, "to": 400}, {"type": "call", "seat": 0}, {"type": "board"},
            {"type": "bet", "seat": 0, "to": 2000}, {"type": "call", "seat": 1},
        ]))
        .unwrap();
        assert_eq!(call_shape(&you_bet, 1), Some("theyCalled"));
        assert_eq!(
            hand_moments(&seats(&[4000, -4000, 0]), 200.0, Some(&you_bet)),
            [Moment {
                seat: 1,
                moment: "lostToYou",
                how: Some("theyCalled")
            }]
        );
        let they_bet: Vec<Value> = serde_json::from_value(
            json!([{"type": "bet", "seat": 1, "to": 2000}, {"type": "call", "seat": 0}]),
        )
        .unwrap();
        assert_eq!(call_shape(&they_bet, 1), Some("youCalled"));
        let checks: Vec<Value> = serde_json::from_value(
            json!([{"type": "check", "seat": 0}, {"type": "check", "seat": 1}]),
        )
        .unwrap();
        assert_eq!(call_shape(&checks, 1), None);
        assert_eq!(call_shape(&[], 1), None);
        let wrong = [
            "good call",
            "nice call",
            "correct call",
            "hero call. i",
            "you called",
            "keep calling",
            "you can fold",
            "paid me off",
            "paying me off",
            "thanks for the call",
            "learn to fold",
            "for the call",
            "appreciate the call",
        ];
        for id in lines().personas.keys() {
            for moment in ["lostToYou", "wonFromYou"] {
                for line in lines_for(id, moment, Some("theyCalled")) {
                    let low = line.to_lowercase();
                    assert!(
                        !wrong.iter().any(|w| low.contains(w)),
                        "{id} {moment}: {line}"
                    );
                }
                for plain in lines_for(id, moment, None) {
                    let low = plain.to_lowercase();
                    let call_word = low
                        .match_indices("call")
                        .any(|(i, _)| i == 0 || !low.as_bytes()[i - 1].is_ascii_alphanumeric());
                    assert!(!call_word, "{id} {moment} without how: {plain}");
                }
            }
        }
        assert!(
            lines_for("jen_silly", "lostToYou", Some("youCalled"))
                .contains(&"Good call.".to_string())
        );
    }
}
