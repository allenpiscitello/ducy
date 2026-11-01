//! Study while you travel or wait for a seat: quick questions on the numbers
//! players use at the table, pot odds, outs and combinations. Question `n`
//! of a world is always the same, so a redrawn page shows the question you
//! were on.

use serde::Serialize;

use crate::roll;
use crate::text::{grouped, pct};
use crate::world::js_round;

fn dollars(n: f64) -> String {
    format!("${}", grouped(n))
}

// The chance of hitting one of `outs` cards by the river: from the flop (two
// cards to come) or the turn (one).
fn hit_chance(outs: f64, cards: u32) -> f64 {
    if cards == 2 {
        1.0 - ((47.0 - outs) * (46.0 - outs)) / (47.0 * 46.0)
    } else {
        outs / 46.0
    }
}

// Four choices around `right` (a share, 0 to 1), all different when rounded.
fn choices_around(right: f64, wrong: &[f64]) -> Vec<f64> {
    let mut seen = vec![pct(right)];
    let mut out = vec![right];
    for &w in wrong {
        if out.len() == 4 {
            break;
        }
        if w > 0.0 && w < 1.0 && !seen.contains(&pct(w)) {
            seen.push(pct(w));
            out.push(w);
        }
    }
    out
}

const DRAWS: [(&str, f64); 6] = [
    ("a flush draw", 9.0),
    ("an open-ended straight draw", 8.0),
    ("a gutshot straight draw", 4.0),
    ("a flush draw and a gutshot", 12.0),
    ("two overcards", 6.0),
    ("an open-ended straight and flush draw", 15.0),
];
const BETS: [(f64, f64, &str); 5] = [
    (1.0, 3.0, "a third of the pot"),
    (1.0, 2.0, "half the pot"),
    (2.0, 3.0, "two thirds of the pot"),
    (3.0, 4.0, "three quarters of the pot"),
    (1.0, 1.0, "the pot"),
];
const HANDS: [(&str, u32); 4] = [
    ("a pocket pair, like 8♠8♥", 6),
    ("a suited hand, like A♠K♠", 4),
    ("an offsuit hand, like A♠K♥", 12),
    ("a hand like AK, suited or not", 16),
];

/// A study question.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Question {
    /// `odds`, `outs`, `turn` or `combos`.
    pub kind: &'static str,
    /// The question.
    pub text: String,
    /// The choices.
    pub choices: Vec<String>,
    /// The right one's index.
    pub answer: usize,
    /// Why.
    pub why: String,
}

fn at<T: Copy>(list: &[T], r: f64) -> T {
    list[(r * list.len() as f64).floor() as usize]
}

/// Question `n` for world `seed`.
pub fn question(seed: u64, n: u64) -> Question {
    let r = |key: &str| roll!(seed; "study", n, key);
    let kind = at(&["odds", "outs", "turn", "combos"], r("kind"));
    let (text, why, labels);
    if kind == "odds" {
        let pot = 20.0 * (2.0 + (r("pot") * 14.0).floor());
        let (num, den, size) = at(&BETS, r("bet"));
        let bet = js_round((pot * num) / den);
        let right = bet / (pot + 2.0 * bet);
        let values = choices_around(
            right,
            &[
                bet / (pot + bet),
                bet / pot,
                right * 1.5,
                right / 1.5,
                right + 0.1,
            ],
        );
        text = format!(
            "The pot is {} and your opponent bets {} ({size}). How often do you need to win to call?",
            dollars(pot),
            dollars(bet)
        );
        why = format!(
            "You call {} to win a pot of {} with your call in it: {} ÷ {} is {}.",
            dollars(bet),
            dollars(pot + 2.0 * bet),
            dollars(bet),
            dollars(pot + 2.0 * bet),
            pct(right)
        );
        labels = values.into_iter().map(pct).collect::<Vec<_>>();
    } else if kind == "outs" || kind == "turn" {
        let (name, outs) = at(&DRAWS, r("draw"));
        let cards = if kind == "outs" { 2 } else { 1 };
        let right = hit_chance(outs, cards);
        let values = choices_around(
            right,
            &[
                hit_chance(outs, 3 - cards),
                outs / 52.0,
                right * 1.6,
                right / 1.6,
                right + 0.12,
            ],
        );
        if cards == 2 {
            text = format!(
                "On the flop you have {name} ({outs} outs). How often do you make your hand by the river, if you see both cards?"
            );
            why = format!(
                "With {outs} outs and two cards to come it's {}. A quick rule: outs × 4, about {}%.",
                pct(right),
                (outs * 4.0).min(100.0)
            );
        } else {
            text = format!(
                "On the turn you have {name} ({outs} outs). How often do you make your hand on the river?"
            );
            why = format!(
                "With {outs} outs among 46 unseen cards it's {}. A quick rule: outs × 2, about {}%.",
                pct(right),
                outs * 2.0
            );
        }
        labels = values.into_iter().map(pct).collect();
    } else {
        let (hand, combos) = at(&HANDS, r("hand"));
        let mut options: Vec<u32> = Vec::new();
        for c in [combos, 4, 6, 12, 16, 8, 24] {
            if !options.contains(&c) {
                options.push(c);
            }
        }
        options.truncate(4);
        labels = options.iter().map(u32::to_string).collect();
        text = format!("How many ways can you be dealt {hand}?");
        why = "A pocket pair has 6 combinations, a suited hand 4 and an offsuit hand 12, so a hand like AK has 16 in all.".to_string();
    }
    // Shuffle the choices, the right one first before.
    let mut order: Vec<usize> = (0..labels.len()).collect();
    order.sort_by(|&a, &b| {
        roll!(seed; "study", n, "order", a).total_cmp(&roll!(seed; "study", n, "order", b))
    });
    Question {
        kind,
        text,
        choices: order.iter().map(|&i| labels[i].clone()).collect(),
        answer: order.iter().position(|&i| i == 0).unwrap_or(0),
        why,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_question_has_its_answer_among_four() {
        for n in 0..40 {
            let q = question(9, n);
            assert!(q.choices.len() >= 2 && q.choices.len() <= 4, "{q:?}");
            assert!(q.answer < q.choices.len());
            assert_eq!(q, question(9, n));
        }
    }
}
