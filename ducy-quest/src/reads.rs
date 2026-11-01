//! Reading players (ducy-web #306): guess a stranger's style from what you
//! can see, tag them, and check it against how they play.
//!
//! - **Tags:** rock, calling station, maniac, grinder or not sure yet, kept
//!   with the player in the save.
//! - **After a session:** each tagged player's real style, and whether the
//!   read was right; for the others, what their hands suggested.
//! - **Over time:** how often a first read (made before [`READ_HANDS`] hands
//!   with that player) and a later one were right.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::people::{FAMILIES, person_of};
use crate::quest::Quest;
use crate::text::pct;

/// What a player can be tagged.
pub const TAGS: [&str; 5] = ["rock", "station", "maniac", "grinder", "unsure"];
/// Hands with a player before a read counts as a later one, not a first.
pub const READ_HANDS: f64 = 20.0;

/// A tag's name.
pub fn tag_name(tag: &str) -> Option<&'static str> {
    Some(match tag {
        "rock" => "Rock",
        "station" => "Calling station",
        "maniac" => "Maniac",
        "grinder" => "Grinder",
        "unsure" => "Not sure yet",
        _ => return None,
    })
}

/// Your read on a player.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Read {
    /// The tag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Hands with them when tagged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<f64>,
    /// The session's first read, made early.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<String>,
    /// Anything else the page keeps.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Reads made and right.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Tally {
    /// Made.
    pub n: f64,
    /// Right.
    pub right: f64,
}

/// How often first and later reads were right.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReadStats {
    /// Reads made before [`READ_HANDS`] hands.
    #[serde(default)]
    pub first: Tally,
    /// Reads after.
    #[serde(default)]
    pub later: Tally,
}

/// Tags generic player `id` with `tag` after `hands` hands with them this
/// session. The first real tag (not `unsure`) made before [`READ_HANDS`]
/// hands is kept as the session's first read.
pub fn tag_player(q: &Quest, id: &str, tag: &str, hands: f64) -> Quest {
    if !TAGS.contains(&tag) || person_of(id).is_none() {
        return q.clone();
    }
    let mut reads = q.reads.clone().unwrap_or_default();
    let old = read_of(&reads, id);
    let first = old
        .first
        .clone()
        .or_else(|| (tag != "unsure" && hands < READ_HANDS).then(|| tag.to_string()));
    let read = Read {
        tag: Some(tag.to_string()),
        at: Some(hands),
        first,
        ..old
    };
    reads.insert(id.to_string(), to_value(read));
    Quest {
        reads: Some(reads),
        ..q.clone()
    }
}

// The read on `id` in a save's reads (anything else there reads as none).
fn read_of(reads: &Map<String, Value>, id: &str) -> Read {
    reads
        .get(id)
        .and_then(|v| Read::deserialize(v).ok())
        .unwrap_or_default()
}
fn to_value(r: Read) -> Value {
    serde_json::to_value(r).unwrap_or(Value::Null)
}

/// The tag on generic player `id`.
pub fn tag_of<'a>(q: &'a Quest, id: &str) -> Option<&'a str> {
    q.reads.as_ref()?.get(id)?.get("tag")?.as_str()
}

/// A rate: `v` none until there's something to count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
pub struct Rate {
    /// The rate.
    #[serde(default)]
    pub v: Option<f64>,
}

/// A player's rates (the page's play stats).
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
pub struct Rates {
    /// Hands seen.
    #[serde(default)]
    pub hands: f64,
    /// Voluntarily put money in.
    #[serde(default)]
    pub vpip: Rate,
    /// Raised pre-flop.
    #[serde(default)]
    pub pfr: Rate,
    /// Aggression factor.
    #[serde(default)]
    pub af: Rate,
}

/// A style guessed from a player's rates.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Guess {
    /// The family.
    pub family: &'static str,
    /// Why.
    pub why: String,
}

/// A style guess from a player's rates, or none with fewer than `min_hands`.
pub fn suggest_family(r: Option<&Rates>, min_hands: f64) -> Option<Guess> {
    let r = r?;
    let vpip = r.vpip.v?;
    if r.hands < min_hands {
        return None;
    }
    let pfr = r.pfr.v.unwrap_or(0.0);
    let af = r.af.v.unwrap_or(1.0);
    let (family, why) = if vpip < 0.16 {
        ("rock", format!("played only {} of hands", pct(vpip)))
    } else if vpip >= 0.3 && (pfr >= 0.22 || af >= 2.5) {
        (
            "maniac",
            format!("played {} of hands and raised {}", pct(vpip), pct(pfr)),
        )
    } else if vpip >= 0.3 {
        (
            "station",
            format!("played {} of hands and rarely raised", pct(vpip)),
        )
    } else {
        (
            "grinder",
            format!("played {} of hands and raised {}", pct(vpip), pct(pfr)),
        )
    };
    Some(Guess { family, why })
}

/// A player you sat with this session.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct SatWith {
    /// Their bot id.
    pub id: String,
    /// Hands with them.
    #[serde(default)]
    pub hands: f64,
    /// Their rates.
    #[serde(default)]
    pub rates: Option<Rates>,
}

/// A row of the session's review.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReviewRow {
    /// Their bot id.
    pub id: String,
    /// Their name.
    pub name: String,
    /// Their real style.
    pub family: String,
    /// Your tag (not `unsure`).
    pub tag: Option<String>,
    /// Whether it was right.
    pub right: Option<bool>,
    /// For the untagged: what their hands suggested.
    pub guess: Option<Guess>,
}

/// The session's review of the generic players you sat with: the rows, and
/// the quest with this session's reads added to its accuracy and each
/// player's first read cleared for next time.
pub fn review_reads(q: &Quest, people: &[SatWith]) -> (Vec<ReviewRow>, Quest) {
    let stats = q.read_stats.unwrap_or_default();
    let (mut first, mut later) = (stats.first, stats.later);
    let mut reads = q.reads.clone().unwrap_or_default();
    let mut rows = Vec::new();
    for s in people {
        let Some(p) = person_of(&s.id) else { continue };
        let read = read_of(&reads, &s.id);
        let tag = read.tag.clone().filter(|t| t != "unsure");
        if let Some(f) = &read.first {
            first.n += 1.0;
            first.right += if *f == p.family { 1.0 } else { 0.0 };
        }
        if let Some(t) = tag.as_ref().filter(|_| s.hands >= READ_HANDS) {
            later.n += 1.0;
            later.right += if *t == p.family { 1.0 } else { 0.0 };
        }
        if read.first.is_some() {
            reads.insert(
                s.id.clone(),
                to_value(Read {
                    first: None,
                    ..read
                }),
            );
        }
        rows.push(ReviewRow {
            id: s.id.clone(),
            name: p.name,
            right: tag.as_ref().map(|t| *t == p.family),
            guess: if tag.is_some() {
                None
            } else {
                suggest_family(s.rates.as_ref(), 15.0)
            },
            family: p.family,
            tag,
        });
    }
    let q = Quest {
        reads: Some(reads),
        read_stats: Some(ReadStats { first, later }),
        ..q.clone()
    };
    (rows, q)
}

/// `7 of 10 (70%)`, or `none yet`.
pub fn accuracy_text(s: Option<&Tally>) -> String {
    match s {
        Some(s) if s.n > 0.0 => format!(
            "{} of {} ({}%)",
            s.right,
            s.n,
            crate::world::js_round(100.0 * s.right / s.n)
        ),
        _ => "none yet".to_string(),
    }
}

/// How long a generic player takes to act, in ms, from draws `a` and `b` in
/// `0..1`: maniacs fast, stations quick, grinders steady, rocks slow. Now
/// and then anyone takes an odd time, so it's a lean, never a rule.
pub fn think_ms(family: &str, a: f64, b: f64) -> f64 {
    if a < 0.15 {
        return 400.0 + b * 1200.0;
    }
    match family {
        "maniac" => 350.0 + b * 300.0,
        "station" => 450.0 + b * 350.0,
        "grinder" => 850.0 + b * 200.0,
        "rock" => 1100.0 + b * 800.0,
        _ => 650.0,
    }
}

const _: () = assert!(FAMILIES.len() + 1 == TAGS.len());

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quest::new_quest;

    #[test]
    fn a_first_read_is_kept_and_reviewed() {
        let id = crate::people::bot_id("rock", 12345);
        let q = tag_player(&new_quest(0.0, 1), &id, "rock", 3.0);
        assert_eq!(tag_of(&q, &id), Some("rock"));
        assert_eq!(q.reads.as_ref().unwrap()[&id]["first"], "rock");
        let (rows, q) = review_reads(
            &q,
            &[SatWith {
                id: id.clone(),
                hands: 30.0,
                rates: None,
            }],
        );
        assert_eq!(rows[0].right, Some(true));
        assert_eq!(q.read_stats.unwrap().first, Tally { n: 1.0, right: 1.0 });
        assert_eq!(q.read_stats.unwrap().later, Tally { n: 1.0, right: 1.0 });
        assert!(q.reads.as_ref().unwrap()[&id].get("first").is_none());
        assert_eq!(
            accuracy_text(Some(&Tally {
                n: 10.0,
                right: 7.0
            })),
            "7 of 10 (70%)"
        );
        // A Regular can't be tagged.
        assert_eq!(tag_player(&q, "doug_poker", "rock", 0.0), q);
    }
}
