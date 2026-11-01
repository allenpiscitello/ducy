//! Getting staked (ducy-web #356): a backer offers to stake you at bigger
//! games, but only if you can prove you know your ranges, on a test of hands
//! from Ducybot's 6-max charts (the page draws and scores it). Pass, and the
//! stake is yours for a game week; then you pay it back with the backer's
//! cut of your profit, smaller the better you scored.
//!
//! - **Training counts:** every 6-max spot you've mastered lowers the mark
//!   you need, down to a floor.
//! - **Reputation as a horse:** paying a backer what you owe raises it;
//!   stiffing them lowers it a lot. A bad name means fewer offers and a
//!   higher mark.
//! - **A failed test:** no backer comes back for a few game days.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::dice::js_num;
use crate::quest::Quest;
use crate::roll;
use crate::scenarios::{LedgerEntry, Message, Offer, answered, book, message, post};
use crate::text::dollars;
use crate::world::js_round;

const DAY: f64 = 1440.0;
/// An offer that waits until it's answered.
pub const NEVER: f64 = 1e12;

/// Chance a backer gets in touch as you leave a table.
pub const ASK: f64 = 0.12;
/// Sessions played before one does.
pub const MIN_SESSIONS: usize = 3;
/// The stake: this many buy-ins (100 BB) at the next stakes up.
pub const BUY_INS: f64 = 10.0;
/// The backer's cut of the profit at the pass mark.
pub const CUT_AT_PASS: f64 = 0.5;
/// For a perfect score.
pub const CUT_AT_PERFECT: f64 = 0.3;
/// In steps of this.
pub const CUT_STEP: f64 = 0.05;
/// The stake runs this many game days.
pub const DAYS: f64 = 7.0;
/// An offer goes away after this many game hours.
pub const OFFER_HOURS: f64 = 24.0;
/// The test's hands.
pub const HANDS: usize = 15;
/// The mark to pass, in percent.
pub const MARK: f64 = 80.0;
/// Lowered this much per mastered trainer spot.
pub const PER_MASTERED: f64 = 2.0;
/// Down to this.
pub const FLOOR: f64 = 66.0;
/// A bad name raises the mark this much per point below zero.
pub const BAD_NAME: f64 = 5.0;
/// No backer for this long after a failed test.
pub const COOLDOWN_DAYS: f64 = 3.0;

// The ladder of stakes the backer moves you up.
const LADDER: [[f64; 2]; 6] = [
    [100.0, 200.0],
    [200.0, 500.0],
    [500.0, 1000.0],
    [1000.0, 2000.0],
    [2500.0, 5000.0],
    [5000.0, 10000.0],
];
const BACKERS: [&str; 8] = [
    "Dmitri", "Lena", "Marcus", "Priya", "Owen", "Sofia", "Kenji", "Rosa",
];

fn label(s: [f64; 2]) -> String {
    format!("{}/{}", js_num(s[0] / 100.0), js_num(s[1] / 100.0))
}
fn pc(x: f64) -> String {
    format!("{}%", js_num(js_round(x * 100.0)))
}

/// Your reputation.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rep {
    /// As a horse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horse: Option<f64>,
    /// Anything else the page keeps.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A backer's stake, while it runs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Backing {
    /// The backer.
    pub name: String,
    /// Their money.
    pub amount: f64,
    /// Their cut of the profit.
    pub share: f64,
    /// The blinds to play.
    pub stakes: [f64; 2],
    /// When it started.
    pub start: f64,
    /// When it's due.
    pub end: f64,
    /// Your bankroll before it.
    pub base: f64,
    /// The backer has asked for their money.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<bool>,
}

/// Your reputation as a horse: 0 to start, up when you pay, down when you stiff.
pub fn horse_rep(q: &Quest) -> f64 {
    q.rep.as_ref().and_then(|r| r.horse).unwrap_or(0.0)
}

/// The mark (% right) you need on the test, given `mastered` Ducybot spots
/// in the trainer and your name as a horse.
pub fn pass_mark(q: &Quest, mastered: f64) -> f64 {
    let rep = horse_rep(q);
    let base = FLOOR.max(MARK - PER_MASTERED * mastered);
    95f64.min(base + if rep < 0.0 { -rep * BAD_NAME } else { 0.0 })
}

/// The backer's cut after a passing score `pct`: the offer's cut at the
/// mark, down to [`CUT_AT_PERFECT`] at 100%, in whole steps of 5%. Below the
/// mark, the offer's cut.
pub fn cut_for(pct: f64, mark: f64, offer_cut: f64) -> f64 {
    if pct < mark {
        return offer_cut;
    }
    let t = if mark >= 100.0 {
        1.0
    } else {
        (pct - mark) / (100.0 - mark)
    };
    let cut = offer_cut - t * 0f64.max(offer_cut - CUT_AT_PERFECT);
    js_round(js_round(cut / CUT_STEP) * CUT_STEP * 100.0) / 100.0
}

/// The stakes one step up from `stakes` (or the top).
pub fn stakes_up(stakes: [f64; 2]) -> [f64; 2] {
    let i = LADDER.iter().position(|s| s[1] == stakes[1]).unwrap_or(0);
    LADDER[(LADDER.len() - 1).min(i + 1)]
}

/// Maybe a backer gets in touch as you leave a table at `stakes`. Not while
/// you're already backed, during a cooldown, or before [`MIN_SESSIONS`];
/// less often with a bad name. `n` makes each chance its own.
pub fn maybe_backer_offer(q: &Quest, stakes: [f64; 2], minutes: f64, n: f64) -> Quest {
    if q.backing().is_some()
        || q.backer_cooldown.unwrap_or(0.0) > minutes
        || q.history.len() < MIN_SESSIONS
    {
        return q.clone();
    }
    let inbox = q.inbox.as_deref().unwrap_or_default();
    if inbox.iter().any(|m| {
        m.offer
            .as_ref()
            .is_some_and(|o| o.kind == "backer" && o.answered.is_none() && o.expires > minutes)
    }) {
        return q.clone();
    }
    let rep = horse_rep(q);
    let chance = ASK
        * if rep < 0.0 {
            0.2f64.max(1.0 + rep * 0.3)
        } else {
            1.0 + 0.15 * rep.min(4.0)
        };
    if roll!(q.seed; "backer", minutes.floor(), n) >= chance {
        return q.clone();
    }
    let up = stakes_up(stakes);
    let amount = BUY_INS * 100.0 * up[1];
    let name = BACKERS[(roll!(q.seed; "backer-name", minutes.floor(), n) * BACKERS.len() as f64)
        .floor() as usize];
    let text = format!(
        "{name}, a backer, has been watching you play: “I’ll stake you ${} to play {} for a week, for a cut of the profit. First, show me you know your 6-max ranges. The better you score, the smaller my cut: {} at the pass mark, down to {}.”",
        dollars(amount),
        label(up),
        pc(CUT_AT_PASS),
        pc(CUT_AT_PERFECT)
    );
    let offer = Offer {
        kind: "backer".into(),
        name: Some(name.into()),
        stakes: Some(up),
        amount: Some(amount),
        share: Some(CUT_AT_PASS),
        expires: minutes + OFFER_HOURS * 60.0,
        ..Offer::default()
    };
    post(
        q,
        Message {
            kind: "backer-offer".into(),
            at: minutes,
            text,
            offer: Some(offer),
            ..Message::default()
        },
    )
}

/// A test's score.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Score {
    /// Hands right.
    pub right: f64,
    /// Of.
    pub of: f64,
    /// Whole percent.
    pub pct: f64,
}

fn news(q: &Quest, minutes: f64, text: String) -> Quest {
    post(
        q,
        Message {
            kind: "backer-news".into(),
            at: minutes,
            text,
            ..Message::default()
        },
    )
}

/// The test for message `msg_id` finished with score `s`: pass and the
/// stake comes in; fail and the backer walks, with a cooldown. `mastered`
/// your trainer spots mastered.
pub fn finish_test(q: &Quest, msg_id: &str, s: Score, mastered: f64, minutes: f64) -> Quest {
    let Some(o) = message(q, msg_id).and_then(|m| m.offer.clone()) else {
        return q.clone();
    };
    if o.answered.is_some() {
        return q.clone();
    }
    let mark = pass_mark(q, mastered);
    let passed = s.pct >= mark;
    let q = answered(q, msg_id, if passed { "passed" } else { "failed" });
    let name = o.name.clone().unwrap_or_default();
    if !passed {
        let q = Quest {
            backer_cooldown: Some(minutes + COOLDOWN_DAYS * DAY),
            ..q
        };
        let text = format!(
            "{name} watched you get {} of {} ({}%), short of the {}% they wanted. “Study up and I’ll think about it.”",
            js_num(s.right),
            js_num(s.of),
            js_num(s.pct),
            js_num(mark)
        );
        return news(&q, minutes, text);
    }
    let share = cut_for(s.pct, mark, o.share.unwrap_or(CUT_AT_PASS));
    let amount = o.amount.unwrap_or(0.0);
    let stakes = o.stakes.unwrap_or_default();
    let backing = Backing {
        name: name.clone(),
        amount,
        share,
        stakes,
        start: minutes,
        end: minutes + DAYS * DAY,
        base: q.bankroll,
        due: None,
    };
    let q = Quest {
        bankroll: q.bankroll + amount,
        backing: Some(Some(backing)),
        ..q
    };
    let q = book(
        &q,
        LedgerEntry {
            at: minutes,
            kind: "backed".into(),
            amount,
            name: Some(name.clone()),
            ..LedgerEntry::default()
        },
    );
    let text = format!(
        "You scored {}% ({} of {}): {name} will take {}% of the profit. They’re staking you ${} to play {}; in a week you pay it back, with their cut.",
        js_num(s.pct),
        js_num(s.right),
        js_num(s.of),
        js_num(js_round(share * 100.0)),
        dollars(amount),
        label(stakes)
    );
    news(&q, minutes, text)
}

/// What you owe a backer now: `(net, owed)`, their stake and their share of
/// a profit, or what's left of it after a loss.
pub fn owed_to_backer(q: &Quest) -> (f64, f64) {
    let Some(b) = q.backing() else {
        return (0.0, 0.0);
    };
    let net = q.bankroll - b.base - b.amount;
    (
        net,
        if net > 0.0 {
            b.amount + js_round(net * b.share)
        } else {
            0f64.max(b.amount + net)
        },
    )
}

/// The week's up (and you're not at a table): the backer asks for their
/// money, an offer to pay or to skip out on. Once only.
pub fn settle_backing(q: &Quest, minutes: f64) -> Quest {
    let Some(b) = q.backing().cloned() else {
        return q.clone();
    };
    if b.due == Some(true) || q.seat.is_some() || minutes < b.end {
        return q.clone();
    }
    let (net, owed) = owed_to_backer(q);
    let q = Quest {
        backing: Some(Some(Backing {
            due: Some(true),
            ..b.clone()
        })),
        ..q.clone()
    };
    let how = if net > 0.0 {
        format!(
            "You made ${} on their money: they want their ${} back and {}% of the profit, ${} in all.",
            dollars(net),
            dollars(b.amount),
            js_num(js_round(b.share * 100.0)),
            dollars(owed)
        )
    } else if owed != 0.0 {
        format!(
            "It went badly: ${} is left of their ${}.",
            dollars(owed),
            dollars(b.amount)
        )
    } else {
        format!("Their ${} is all gone.", dollars(b.amount))
    };
    // It waits until answered.
    let offer = Offer {
        kind: "backer-settle".into(),
        name: Some(b.name.clone()),
        owed: Some(owed),
        net: Some(net),
        expires: NEVER,
        ..Offer::default()
    };
    post(
        &q,
        Message {
            kind: "backer-settle".into(),
            at: minutes,
            text: format!("{}’s week is up. {how}", b.name),
            offer: Some(offer),
            ..Message::default()
        },
    )
}

/// Settles with the backer: `pay` (what you owe, and your name grows if
/// they made money, shrinks a little if you lost most of theirs) or `stiff`
/// (keep it, and backers will hear about it).
pub fn answer_settle(q: &Quest, msg_id: &str, answer: &str, minutes: f64) -> Quest {
    let Some(o) = message(q, msg_id).and_then(|m| m.offer.clone()) else {
        return q.clone();
    };
    let Some(b) = q.backing().cloned() else {
        return q.clone();
    };
    if o.kind != "backer-settle" || o.answered.is_some() {
        return q.clone();
    }
    let (net, owed) = owed_to_backer(q);
    let paying = answer == "pay";
    let pay = if paying { owed.min(q.bankroll) } else { 0.0 };
    let change = if !paying {
        -3.0
    } else if net > 0.0 {
        1.0
    } else if net < -b.amount / 2.0 {
        -1.0
    } else {
        0.0
    };
    let mut rep = q.rep.clone().unwrap_or_default();
    rep.horse = Some(horse_rep(q) + change);
    let q = answered(q, msg_id, answer);
    let q = Quest {
        bankroll: q.bankroll - pay,
        backing: Some(None),
        rep: Some(rep),
        ..q
    };
    let q = book(
        &q,
        LedgerEntry {
            at: minutes,
            kind: "backer-paid".into(),
            amount: -pay,
            name: Some(b.name.clone()),
            stiffed: Some(!paying),
            ..LedgerEntry::default()
        },
    );
    let name = &b.name;
    let text = if !paying {
        format!("You kept {name}’s money. Word gets around: backers will be warier of you.")
    } else if net > 0.0 {
        format!("You paid {name} ${}. They’d back you again.", dollars(pay))
    } else if pay != 0.0 {
        let why = if change < 0.0 {
            "Losing most of their money hurts your name a little."
        } else {
            "Bad weeks happen."
        };
        format!("You handed {name} back the ${} left. {why}", dollars(pay))
    } else {
        let why = if change < 0.0 {
            "Losing their money hurts your name a little."
        } else {
            ""
        };
        format!("You told {name} it’s all gone. {why}")
    };
    news(&q, minutes, text.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mark_and_the_cut() {
        let q = crate::quest::new_quest(0.0, 1);
        assert_eq!(pass_mark(&q, 0.0), 80.0);
        assert_eq!(pass_mark(&q, 10.0), 66.0);
        assert_eq!(cut_for(80.0, 80.0, CUT_AT_PASS), 0.5);
        assert_eq!(cut_for(100.0, 80.0, CUT_AT_PASS), 0.3);
        assert_eq!(cut_for(90.0, 80.0, CUT_AT_PASS), 0.4);
        assert_eq!(stakes_up([100.0, 200.0]), [200.0, 500.0]);
        assert_eq!(stakes_up([5000.0, 10000.0]), [5000.0, 10000.0]);
    }
}
