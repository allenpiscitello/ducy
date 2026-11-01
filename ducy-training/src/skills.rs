//! The range trainer's skills (ducy-web #225, #464): a game (hole cards) and
//! a top-% target, each scored 0 to 100 and mastered at [`MASTERY`], with
//! the pacing of the climb.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// A skill scores 0 to 100, and this or more masters it.
pub const MASTERY: u32 = 90;

/// Points for a correct answer at score `score`: the climb slows as it
/// rises, and from 80 up every point is one more right answer.
pub fn gain_for(score: u32) -> u32 {
    match score {
        0..40 => 4,
        40..60 => 3,
        60..80 => 2,
        _ => 1,
    }
}

/// Correct answers in a row needed to go from `from` to mastery.
pub fn answers_to_master(from: u32) -> u32 {
    let (mut s, mut n) = (from, 0);
    while s < MASTERY {
        s = (s + gain_for(s)).min(MASTERY);
        n += 1;
    }
    n
}

/// The progress key of a skill: a game (hole cards) and a top-% target.
pub fn skill_key(cards: u32, pct: u32) -> String {
    if cards == 2 {
        format!("holdem:{pct}")
    } else {
        format!("omaha:{cards}:{pct}")
    }
}

/// A skill: hole cards and a top-% target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skill {
    /// Hole cards: 2 for Hold'em, 4 to 6 for Omaha.
    pub cards: u32,
    /// The top-% target.
    pub pct: u32,
}

/// The skill a progress key is for (all hands, not a drill), or None.
pub fn parse_skill_key(key: &str) -> Option<Skill> {
    let parts: Vec<&str> = key.split(':').collect();
    let n = |s: &str| {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
    };
    match parts.as_slice() {
        ["holdem", p] => Some(Skill {
            cards: 2,
            pct: n(p)?,
        }),
        ["omaha", c, p] => Some(Skill {
            cards: n(c)?,
            pct: n(p)?,
        }),
        _ => None,
    }
}

/// A skill's progress record, as the page keeps it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Record {
    /// Its score now.
    pub score: f64,
    /// The best it reached.
    pub best: f64,
    /// Answers given.
    pub answered: f64,
}

/// A skill's state: new, started or mastered, with its scores.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct State {
    /// "new", "started" or "mastered".
    pub status: &'static str,
    /// Its score now.
    pub score: f64,
    /// The best it reached.
    pub best: f64,
    /// Answers given.
    pub answered: f64,
}

/// A skill's state from its progress record (or none).
pub fn skill_state(rec: Option<&Record>) -> State {
    let Some(r) = rec.filter(|r| r.answered != 0.0 || r.score != 0.0 || r.best != 0.0) else {
        return State {
            status: "new",
            score: 0.0,
            best: 0.0,
            answered: 0.0,
        };
    };
    let best = r.best.max(r.score);
    State {
        status: if best >= f64::from(MASTERY) {
            "mastered"
        } else {
            "started"
        },
        score: r.score,
        best,
        answered: r.answered,
    }
}

/// A cell of the skill table: a game at a target, and its state.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Cell {
    /// Hole cards.
    pub cards: u32,
    /// The target.
    pub pct: u32,
    /// Its state.
    #[serde(flatten)]
    pub state: State,
}

/// A row of the skill table: one target.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Row {
    /// The target.
    pub pct: u32,
    /// A cell per game.
    pub cells: Vec<Cell>,
}

/// Every skill for `games` and the standard `targets`, plus any other target
/// already practised: rows by target, a cell per game.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Table {
    /// The rows, by target.
    pub rows: Vec<Row>,
    /// How many are mastered.
    pub mastered: u32,
    /// How many are started (mastered too).
    pub started: u32,
    /// How many there are.
    pub total: u32,
}

/// The skill table (see [`Table`]) from the page's `progress`.
pub fn skill_table(progress: &BTreeMap<String, Record>, games: &[u32], targets: &[u32]) -> Table {
    let mut pcts: BTreeSet<u32> = targets.iter().copied().collect();
    for (key, rec) in progress {
        if let Some(s) = parse_skill_key(key)
            && games.contains(&s.cards)
            && skill_state(Some(rec)).status != "new"
        {
            pcts.insert(s.pct);
        }
    }
    let (mut mastered, mut started, mut total) = (0, 0, 0);
    let rows = pcts
        .into_iter()
        .map(|pct| Row {
            pct,
            cells: games
                .iter()
                .map(|&cards| {
                    let state = skill_state(progress.get(&skill_key(cards, pct)));
                    total += 1;
                    if state.status == "mastered" {
                        mastered += 1;
                    }
                    if state.status != "new" {
                        started += 1;
                    }
                    Cell { cards, pct, state }
                })
                .collect(),
        })
        .collect();
    Table {
        rows,
        mastered,
        started,
        total,
    }
}

/// The skill to move on to after mastering `pct` in the game with `cards`
/// hole cards: the next standard target of that game not mastered yet (going
/// up, then from the start), then the other games' in order. None once every
/// one is mastered.
pub fn next_skill_after(
    progress: &BTreeMap<String, Record>,
    games: &[u32],
    targets: &[u32],
    cards: u32,
    pct: u32,
) -> Option<Skill> {
    let done = |c: u32, p: u32| skill_state(progress.get(&skill_key(c, p))).status == "mastered";
    let order_games = std::iter::once(cards).chain(games.iter().copied().filter(|&x| x != cards));
    for c in order_games {
        let order: Vec<u32> = if c == cards {
            targets
                .iter()
                .copied()
                .filter(|&p| p > pct)
                .chain(targets.iter().copied().filter(|&p| p < pct))
                .collect()
        } else {
            targets.to_vec()
        };
        if let Some(p) = order.into_iter().find(|&t| !done(c, t)) {
            return Some(Skill { cards: c, pct: p });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pacing_to_mastery() {
        assert_eq!(
            (gain_for(0), gain_for(45), gain_for(70), gain_for(85)),
            (4, 3, 2, 1)
        );
        assert_eq!(answers_to_master(0), 36);
        assert_eq!(answers_to_master(90), 0);
    }

    #[test]
    fn keys_states_and_the_table() {
        assert_eq!(skill_key(2, 20), "holdem:20");
        assert_eq!(skill_key(4, 15), "omaha:4:15");
        assert_eq!(
            parse_skill_key("omaha:5:30"),
            Some(Skill { cards: 5, pct: 30 })
        );
        assert_eq!(parse_skill_key("holdem:x"), None);
        assert_eq!(parse_skill_key("drill:holdem:20"), None);
        let rec = |score: f64, best: f64| Record {
            score,
            best,
            answered: 3.0,
        };
        let mut p = BTreeMap::new();
        p.insert("holdem:20".to_string(), rec(95.0, 0.0));
        p.insert("holdem:33".to_string(), rec(40.0, 50.0));
        assert_eq!(skill_state(None).status, "new");
        assert_eq!(skill_state(p.get("holdem:20")).status, "mastered");
        assert_eq!(skill_state(p.get("holdem:33")).best, 50.0);
        let t = skill_table(&p, &[2, 4], &[10, 20]);
        assert_eq!(
            t.rows.iter().map(|r| r.pct).collect::<Vec<_>>(),
            [10, 20, 33]
        );
        assert_eq!((t.mastered, t.started, t.total), (1, 2, 6));
        // After mastering Hold'em 10: Hold'em 30 (20 is done), then back round.
        assert_eq!(
            next_skill_after(&p, &[2, 4], &[10, 20, 30], 2, 10),
            Some(Skill { cards: 2, pct: 30 })
        );
        assert_eq!(
            next_skill_after(&p, &[2, 4], &[20], 2, 20),
            Some(Skill { cards: 4, pct: 20 })
        );
    }
}
