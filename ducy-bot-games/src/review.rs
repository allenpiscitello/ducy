//! Hand reviews against the GTO bot: totals for the reviews ducy-wasm's
//! `BotTable.reviewLastHand()` gives after each hand, and a ladder game's
//! review (ducy-web #284): its costliest decisions, the places to learn from.
//!
//! A review lists the person's decisions, each with a grade:
//! - `fine`: the bot's own play, a play it mixes in, or close in value;
//! - `inaccuracy`: gives up a little (from 0.25 bb); `mistake`: from 1 bb;
//!   `blunder`: from 4 bb;
//! - `deviation`: something the bot (almost) never does preflop, where the
//!   cost isn't valued;
//! - `unknown`: the hand left the bot's betting tree, nothing to compare;
//!
//! and `bb_lost`, the big blinds it's charged (0 for the bot's own plays).
//! Reviews and decisions are kept as their JSON: the page shows the rest.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::num;

/// Totals over hands' reviews.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Totals {
    /// Hands reviewed.
    pub hands: u32,
    /// Decisions in them.
    pub decisions: u32,
    /// Big blinds the decisions gave up.
    pub bb_lost: f64,
    /// Decisions graded fine.
    pub fine: u32,
    /// Inaccuracies.
    pub inaccuracies: u32,
    /// Mistakes.
    pub mistakes: u32,
    /// Blunders.
    pub blunders: u32,
    /// Off-strategy plays.
    pub deviations: u32,
    /// Not graded.
    pub unknown: u32,
    /// Fine, but the less common side of the bot's mix.
    pub variants: u32,
    /// Big blinds those mixed plays gave up against the best play (not
    /// charged).
    pub mix_loss: f64,
}

/// What an uncharged play still gave up against the best play.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MixCost {
    /// Big blinds.
    pub loss: f64,
    /// The best play.
    pub best: Value,
    /// Within the estimate's error (2 standard errors).
    pub close: bool,
}

fn grade(d: &Value) -> &str {
    d.get("grade").and_then(Value::as_str).unwrap_or("")
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Number(n)) => n.as_f64().is_some_and(|x| x != 0.0),
        Some(_) => true,
    }
}

/// A play the bot mixes in isn't graded down, but in this spot it can still
/// be worth less than the best play, and that's worth seeing: for a play
/// graded fine and not charged that's worth less than the best.
pub fn mix_cost(d: &Value) -> Option<MixCost> {
    let loss = num(d.get("ev_loss"));
    let best = d.get("best");
    if grade(d) != "fine"
        || num(d.get("bb_lost")) > 0.0
        || loss <= 0.0
        || !truthy(best)
        || best == d.get("action")
    {
        return None;
    }
    Some(MixCost {
        loss,
        best: best.cloned().unwrap_or(Value::Null),
        close: loss <= 2.0 * num(d.get("stderr")),
    })
}

/// Whether a fine decision took the less common side of the bot's mix.
pub fn is_variant(d: &Value) -> bool {
    grade(d) == "fine"
        && d.get("note")
            .and_then(Value::as_str)
            .is_some_and(|n| n.starts_with("a less common"))
}

fn decisions(review: &Value) -> &[Value] {
    review
        .get("decisions")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

impl Totals {
    /// Adds one hand's review.
    pub fn add_review(&mut self, review: &Value) {
        self.hands += 1;
        for d in decisions(review) {
            self.decisions += 1;
            self.bb_lost += num(d.get("bb_lost"));
            match grade(d) {
                "fine" => self.fine += 1,
                "inaccuracy" => self.inaccuracies += 1,
                "mistake" => self.mistakes += 1,
                "blunder" => self.blunders += 1,
                "deviation" => self.deviations += 1,
                "unknown" => self.unknown += 1,
                _ => {}
            }
            if is_variant(d) {
                self.variants += 1;
            }
            if let Some(c) = mix_cost(d).filter(|c| !c.close) {
                self.mix_loss += c.loss;
            }
        }
    }

    /// Adds another set of totals.
    pub fn add(&mut self, b: &Totals) {
        self.hands += b.hands;
        self.decisions += b.decisions;
        self.bb_lost += b.bb_lost;
        self.fine += b.fine;
        self.inaccuracies += b.inaccuracies;
        self.mistakes += b.mistakes;
        self.blunders += b.blunders;
        self.deviations += b.deviations;
        self.unknown += b.unknown;
        self.variants += b.variants;
        self.mix_loss += b.mix_loss;
    }

    /// Big blinds lost per 100 hands, or none with no hands.
    pub fn per100(&self) -> Option<f64> {
        (self.hands > 0).then(|| 100.0 * self.bb_lost / self.hands as f64)
    }
}

/// The decision that cost the most in a review, by its place.
pub fn worst_index(review: &Value) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (i, d) in decisions(review).iter().enumerate() {
        let lost = num(d.get("bb_lost"));
        if lost > 0.0 && best.is_none_or(|(_, b)| lost > b) {
            best = Some((i, lost));
        }
    }
    best.map(|(i, _)| i)
}

/// Decisions kept for a ladder game's review.
pub const LESSONS_KEPT: usize = 5;

/// One of a game's costliest decisions, with its hand.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Lesson {
    /// The ladder game's hand number.
    #[serde(default)]
    pub hand: u32,
    /// The decision's place in the hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<usize>,
    /// The hand's hole cards.
    #[serde(default)]
    pub hole: Value,
    /// Its board.
    #[serde(default)]
    pub board: Value,
    /// Its result (bb).
    #[serde(default)]
    pub result: Value,
    /// Its depth note.
    #[serde(default)]
    pub note: Value,
    /// The decision, as the review gave it.
    #[serde(default)]
    pub d: Value,
}

/// A ladder game's review.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameReview {
    /// Its totals.
    pub totals: Totals,
    /// Its costliest decisions, worst first.
    pub lessons: Vec<Lesson>,
}

// Worth learning from: a decision that gave up big blinds, or a preflop play
// the GTO bot (almost) never makes, whose cost isn't valued.
fn is_lesson(d: &Value) -> bool {
    num(d.get("bb_lost")) > 0.0 || grade(d) == "deviation"
}

// Costliest first; off-strategy plays (not valued) after every charged one;
// then the earlier hand.
fn by_cost(a: &Lesson, b: &Lesson) -> Ordering {
    let cost = |x: &Lesson| num(x.d.get("bb_lost"));
    cost(b)
        .partial_cmp(&cost(a))
        .unwrap_or(Ordering::Equal)
        .then(a.hand.cmp(&b.hand))
}

/// Adds one hand's review to a game's review, as hand number `hand` of the
/// game.
pub fn add_hand(g: Option<GameReview>, review: &Value, hand: u32) -> GameReview {
    let mut g = g.unwrap_or_default();
    g.totals.add_review(review);
    let note = review.get("note").filter(|n| truthy(Some(n))).cloned();
    let field = |k: &str| review.get(k).cloned().unwrap_or(Value::Null);
    for (at, d) in decisions(review).iter().enumerate() {
        if is_lesson(d) {
            g.lessons.push(Lesson {
                hand,
                at: Some(at),
                hole: field("hole"),
                board: field("board"),
                result: field("result"),
                note: note.clone().unwrap_or(Value::Null),
                d: d.clone(),
            });
        }
    }
    g.lessons.sort_by(by_cost);
    g.lessons.truncate(LESSONS_KEPT);
    g
}

/// A lesson in a hand's list.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HandItem {
    /// The decision.
    pub d: Value,
    /// Its place in the game's list.
    pub i: usize,
    /// Its place in the hand.
    pub at: usize,
}

/// A hand's lessons.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HandLessons {
    /// The hand number.
    pub hand: u32,
    /// Its hole cards.
    pub hole: Value,
    /// Its board.
    pub board: Value,
    /// Its result.
    pub result: Value,
    /// Its depth note.
    pub note: Value,
    /// Its lessons, in the order they were played.
    pub items: Vec<HandItem>,
}

/// The lessons by hand, to show each hand once: hands in the order of their
/// costliest lesson, each with its lessons in the order they were played.
pub fn lessons_by_hand(lessons: &[Lesson]) -> Vec<HandLessons> {
    let mut hands: Vec<HandLessons> = Vec::new();
    for (i, x) in lessons.iter().enumerate() {
        let item = HandItem {
            d: x.d.clone(),
            i,
            at: x.at.unwrap_or(i),
        };
        match hands.iter_mut().find(|h| h.hand == x.hand) {
            Some(h) => h.items.push(item),
            None => hands.push(HandLessons {
                hand: x.hand,
                hole: x.hole.clone(),
                board: x.board.clone(),
                result: x.result.clone(),
                note: x.note.clone(),
                items: vec![item],
            }),
        }
    }
    for h in &mut hands {
        h.items.sort_by_key(|x| x.at);
    }
    hands
}

/// The big blinds a game's decisions gave up, in all and for the lessons
/// shown: how much of the loss the list accounts for.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Share {
    /// In all.
    pub all: f64,
    /// For the lessons shown.
    pub shown: f64,
}

/// How much of a game's loss its lessons account for.
pub fn lessons_share(g: Option<&GameReview>) -> Share {
    Share {
        all: g.map_or(0.0, |g| g.totals.bb_lost),
        shown: g.map_or(0.0, |g| {
            g.lessons.iter().map(|x| num(x.d.get("bb_lost"))).sum()
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dec(grade: &str, bb_lost: f64, action: &str) -> Value {
        json!({"street": "flop", "board": "Ks 9d 4h", "pot": 6, "to_call": 3, "action": action, "gto": [],
            "main": "fold", "best": "fold", "bb_lost": bb_lost, "stderr": 0, "grade": grade})
    }
    fn hand(decisions: &[Value]) -> Value {
        let lost: f64 = decisions
            .iter()
            .map(|d| d["bb_lost"].as_f64().unwrap())
            .sum();
        json!({"hole": "Ah Kd", "board": "Ks 9d 4h 2c 7s", "decisions": decisions, "bb_lost": lost, "result": -12.5, "note": null})
    }

    #[test]
    fn a_review_adds_its_decisions_grades_and_bb_lost() {
        let mixed = json!({"grade": "fine", "bb_lost": 0, "action": "call", "best": "raise to 9bb", "ev_loss": 0.6, "stderr": 0.05, "note": "a less common choice: …"});
        let mut close = mixed.clone();
        close["ev_loss"] = json!(0.05);
        let mut charged = mixed.clone();
        charged["grade"] = json!("mistake");
        charged["bb_lost"] = json!(1.2);
        charged["ev_loss"] = json!(1.2);
        let mut t = Totals::default();
        t.add_review(&json!({"decisions": [mixed, close, charged]}));
        assert_eq!(
            (t.hands, t.decisions, t.fine, t.mistakes, t.variants),
            (1, 3, 2, 1, 2)
        );
        assert!(
            (t.mix_loss - 0.6).abs() < 1e-9,
            "only the uncharged one, outside the error"
        );
        assert!((t.bb_lost - 1.2).abs() < 1e-9);
        let mut both = Totals::default();
        both.add(&t);
        both.add(&t);
        assert!((both.mix_loss - 1.2).abs() < 1e-9);
        assert_eq!(both.hands, 2);
        assert!((both.per100().unwrap() - 120.0).abs() < 1e-9);
        assert_eq!(Totals::default().per100(), None);
        let mut empty = Totals::default();
        empty.add_review(&json!({"decisions": [], "bb_lost": 0}));
        assert_eq!((empty.hands, empty.decisions), (1, 0));
    }

    #[test]
    fn an_uncharged_mixed_play_still_shows_what_it_gave_up() {
        let d = json!({"grade": "fine", "bb_lost": 0, "action": "call", "best": "raise to 9bb", "ev_loss": 0.6, "stderr": 0.05});
        assert_eq!(
            mix_cost(&d),
            Some(MixCost {
                loss: 0.6,
                best: json!("raise to 9bb"),
                close: false
            })
        );
        let with = |k: &str, v: Value| {
            let mut x = d.clone();
            x[k] = v;
            x
        };
        assert!(mix_cost(&with("ev_loss", json!(0.08))).unwrap().close);
        assert_eq!(mix_cost(&with("best", json!("call"))), None);
        assert_eq!(mix_cost(&with("bb_lost", json!(0.6))), None);
        assert_eq!(mix_cost(&with("ev_loss", Value::Null)), None);
        assert_eq!(mix_cost(&with("ev_loss", json!(0))), None);
        assert_eq!(mix_cost(&with("grade", json!("mistake"))), None);
    }

    #[test]
    fn the_worst_decision_is_the_costliest() {
        let r = hand(&[
            dec("fine", 0.0, "call"),
            dec("blunder", 5.0, "call"),
            dec("mistake", 2.0, "call"),
        ]);
        assert_eq!(worst_index(&r), Some(1));
        assert_eq!(worst_index(&json!({"decisions": [{"bb_lost": 0}]})), None);
    }

    #[test]
    fn a_game_review_adds_up_every_hand_and_keeps_what_cost_something() {
        let g = add_hand(
            None,
            &hand(&[
                dec("fine", 0.0, "call 3bb"),
                dec("mistake", 1.5, "call 3bb"),
            ]),
            1,
        );
        let g = add_hand(Some(g), &hand(&[dec("fine", 0.0, "call 3bb")]), 2);
        let g = add_hand(
            Some(g),
            &hand(&[
                dec("blunder", 6.0, "call 3bb"),
                dec("deviation", 0.0, "raise to 25bb"),
            ]),
            3,
        );
        assert_eq!(
            (g.totals.hands, g.totals.decisions, g.totals.blunders),
            (3, 5, 1)
        );
        assert_eq!(g.totals.bb_lost, 7.5);
        let got: Vec<(u32, &str)> = g.lessons.iter().map(|x| (x.hand, grade(&x.d))).collect();
        assert_eq!(got, [(3, "blunder"), (1, "mistake"), (3, "deviation")]);
        assert_eq!(g.lessons[0].hole, "Ah Kd");
        assert_eq!(g.lessons[0].result, -12.5);
        assert_eq!(
            lessons_share(Some(&g)),
            Share {
                all: 7.5,
                shown: 7.5
            }
        );
    }

    #[test]
    fn only_the_costliest_few_are_kept_the_earlier_hand_first_on_a_tie() {
        let mut g = GameReview::default();
        for i in 1..=8u32 {
            let lost = if i % 2 == 1 { 0.5 } else { 0.3 * i as f64 };
            g = add_hand(Some(g), &hand(&[dec("inaccuracy", lost, "call 3bb")]), i);
        }
        assert_eq!(g.lessons.len(), LESSONS_KEPT);
        assert_eq!(
            g.lessons.iter().map(|x| x.hand).collect::<Vec<_>>(),
            [8, 6, 4, 2, 1]
        );
        let s = lessons_share(Some(&g));
        assert!(s.shown < s.all);
    }

    #[test]
    fn lessons_are_shown_by_hand_costliest_hand_first_in_the_order_played() {
        let g = add_hand(
            None,
            &hand(&[
                dec("mistake", 2.0, "call 2bb"),
                dec("fine", 0.0, "call"),
                dec("blunder", 5.0, "call 9bb"),
            ]),
            1,
        );
        let g = add_hand(Some(g), &hand(&[dec("blunder", 4.0, "call 3bb")]), 2);
        let by = lessons_by_hand(&g.lessons);
        assert_eq!(by.iter().map(|h| h.hand).collect::<Vec<_>>(), [1, 2]);
        let items: Vec<(&str, usize, usize)> = by[0]
            .items
            .iter()
            .map(|x| (x.d["action"].as_str().unwrap(), x.i, x.at))
            .collect();
        assert_eq!(items, [("call 2bb", 2, 0), ("call 9bb", 0, 2)]);
    }

    #[test]
    fn a_hand_with_nothing_to_learn_from_adds_to_the_totals_only() {
        let g = add_hand(
            Some(GameReview::default()),
            &hand(&[dec("fine", 0.0, "call")]),
            1,
        );
        assert_eq!(g.totals.hands, 1);
        assert!(g.lessons.is_empty());
        assert_eq!(
            lessons_share(Some(&g)),
            Share {
                all: 0.0,
                shown: 0.0
            }
        );
        assert_eq!(
            lessons_share(None),
            Share {
                all: 0.0,
                shown: 0.0
            }
        );
    }

    #[test]
    fn a_saved_review_reads_back_with_missing_totals_and_places() {
        let g: GameReview = serde_json::from_value(json!({"totals": {"hands": 2, "bbLost": 1.5}, "lessons": [{"hand": 1, "d": {"bb_lost": 1.5}}]})).unwrap();
        assert_eq!((g.totals.hands, g.totals.fine), (2, 0));
        assert_eq!(lessons_by_hand(&g.lessons)[0].items[0].at, 0);
    }
}
