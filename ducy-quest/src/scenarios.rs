//! Scenarios (ducy-web #365): things that happen around the poker. The shared
//! machinery (#355): an inbox of offers and news, a ledger of the money that
//! changes hands; and the first scenario, staking players.
//!
//! Staking players: a generic player you've sat with (never a Regular) asks
//! you to back them for a game week. Two hidden traits lean their outcome
//! but never decide it: **skill**, from their style and their own noise, as
//! bb/100; and **honesty**, regs leaning honest and tourists less so. About
//! a game week later the stake comes back: busted, some of it, a small or a
//! big win, or nothing at all (they won and stiffed you). Someone who
//! stiffed you may turn up at your table again, and you can confront them.
//!
//! Repeatable: chances come from the world's seed.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::dice::js_num;
use crate::people::person_of;
use crate::quest::Quest;
use crate::roll;
use crate::text::dollars;
use crate::world::js_round;

const DAY: f64 = 1440.0;

/// Staking's tuning.
pub mod stake {
    /// Chance someone you played with asks, when you leave a table.
    pub const ASK: f64 = 0.3;
    /// More often at low stakes.
    pub const ASK_LOW: f64 = 1.5;
    /// And late at night.
    pub const ASK_LATE: f64 = 1.3;
    /// Hands with you before they'd ask.
    pub const MIN_HANDS: f64 = 12.0;
    /// What they ask for, in their buy-ins (100 BB) at the stakes you played.
    pub const BUY_INS: [f64; 4] = [3.0, 5.0, 8.0, 10.0];
    /// At most this share of your bankroll.
    pub const MAX_SHARE: f64 = 0.3;
    /// Your share of their profit.
    pub const SHARES: [f64; 4] = [0.4, 0.5, 0.5, 0.6];
    /// A game week, give or take two days.
    pub const DAYS: f64 = 7.0;
    /// Give or take.
    pub const SPREAD: f64 = 2.0;
    /// Hands they play in a game day.
    pub const HANDS_PER_DAY: f64 = 220.0;
    /// How much results swing, bb/100 per 100 hands.
    pub const SD: f64 = 95.0;
    /// An offer you don't answer goes away after this many game hours.
    pub const OFFER_HOURS: f64 = 12.0;
    /// How often a counter is taken.
    pub const COUNTER_TAKEN: f64 = 0.5;
}

/// Confronting someone who stiffed you.
pub mod confronting {
    /// Paying up: at least this.
    pub const PAY_BASE: f64 = 0.25;
    /// More the more honest they are.
    pub const PAY_HONEST: f64 = 0.5;
}

// Win rate in bb/100 by style, before each player's own noise.
fn skill_base(family: &str) -> f64 {
    match family {
        "grinder" => 6.0,
        "rock" => 1.0,
        "station" => -9.0,
        _ => -12.0,
    }
}
const SKILL_NOISE: f64 = 6.0;
const HONESTY_NOISE: f64 = 0.25;

/// A generic player's hidden win rate, bb/100.
pub fn skill_of(id: &str) -> f64 {
    let Some(p) = person_of(id) else { return 0.0 };
    skill_base(&p.family) + (roll!(p.seed; "skill") * 2.0 - 1.0) * SKILL_NOISE
}

/// A generic player's hidden honesty, 0 to 1.
pub fn honesty_of(id: &str, reg: bool) -> f64 {
    let Some(p) = person_of(id) else { return 0.5 };
    let base = if reg { 0.75 } else { 0.5 };
    (base + (roll!(p.seed; "honesty") * 2.0 - 1.0) * HONESTY_NOISE).clamp(0.0, 1.0)
}

/// An offer in a message.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Offer {
    /// `stake`, `backer` or `backer-settle`.
    pub kind: String,
    /// Who asks (a stake).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// A backer's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The blinds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stakes: Option<[f64; 2]>,
    /// The money.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<f64>,
    /// The share of the profit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share: Option<f64>,
    /// A room's reg.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reg: Option<bool>,
    /// Asked before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub again: Option<bool>,
    /// What's owed (settling with a backer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owed: Option<f64>,
    /// The profit or loss.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net: Option<f64>,
    /// When it goes away.
    pub expires: f64,
    /// How it was answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered: Option<String>,
    /// Anything else the page keeps.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A message in the inbox.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Message {
    /// `m1`, `m2`, …
    #[serde(default)]
    pub id: String,
    /// Read.
    #[serde(default)]
    pub read: bool,
    /// What it is.
    pub kind: String,
    /// What it says.
    pub text: String,
    /// The game minute.
    pub at: f64,
    /// An offer to answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offer: Option<Offer>,
    /// How a stake went.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// Anything else the page keeps.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Message {
    fn new(kind: &str, at: f64, text: String) -> Self {
        Message {
            kind: kind.to_string(),
            at,
            text,
            ..Message::default()
        }
    }
    fn open(&self, minutes: f64) -> bool {
        self.offer
            .as_ref()
            .is_some_and(|o| o.answered.is_none() && o.expires > minutes)
    }
}

/// Money from (negative) or to (positive) the bankroll.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LedgerEntry {
    /// The game minute.
    pub at: f64,
    /// What for.
    pub kind: String,
    /// How much.
    pub amount: f64,
    /// Who (a stake).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// A backer's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// How a stake went.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// A backer stiffed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stiffed: Option<bool>,
}

/// A player you've staked.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stake {
    /// Their bot id.
    pub id: String,
    /// The blinds they play.
    pub stakes: [f64; 2],
    /// The money.
    pub amount: f64,
    /// Your share of their profit.
    pub share: f64,
    /// A room's reg.
    #[serde(default)]
    pub reg: bool,
    /// When it started.
    pub start: f64,
    /// When it comes back.
    pub end: f64,
    /// Its number, among your stakes.
    pub n: f64,
    /// How it went.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// What came back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returned: Option<f64>,
    /// Their profit or loss.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profit: Option<f64>,
    /// When it was settled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settled: Option<f64>,
    /// You confronted them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confronted: Option<bool>,
}

impl Stake {
    fn stiffed_open(&self, id: &str) -> bool {
        self.id == id && self.result.as_deref() == Some("stiffed") && self.confronted != Some(true)
    }
}

/// Adds a message to the inbox (the newest 60 kept).
pub fn post(q: &Quest, msg: Message) -> Quest {
    let n = q.inbox_n.unwrap_or(0.0) + 1.0;
    let mut inbox = vec![Message {
        id: format!("m{}", js_num(n)),
        read: false,
        ..msg
    }];
    inbox.extend(q.inbox.clone().unwrap_or_default());
    inbox.truncate(60);
    Quest {
        inbox: Some(inbox),
        inbox_n: Some(n),
        ..q.clone()
    }
}

/// Marks every message read.
pub fn mark_read(q: &Quest) -> Quest {
    let inbox = q.inbox.as_deref().unwrap_or_default();
    if !inbox.iter().any(|m| !m.read) {
        return q.clone();
    }
    Quest {
        inbox: Some(
            inbox
                .iter()
                .map(|m| Message {
                    read: true,
                    ..m.clone()
                })
                .collect(),
        ),
        ..q.clone()
    }
}

/// Unread messages.
pub fn unread(q: &Quest) -> usize {
    q.inbox
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter(|m| !m.read)
        .count()
}

/// Open offers waiting for an answer at game time `minutes`.
pub fn open_offers(q: &Quest, minutes: f64) -> Vec<Message> {
    q.inbox
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter(|m| m.open(minutes))
        .cloned()
        .collect()
}

/// A ledger entry (the newest 500 kept).
pub fn book(q: &Quest, entry: LedgerEntry) -> Quest {
    let mut ledger = q.ledger.clone().unwrap_or_default();
    ledger.push(entry);
    if ledger.len() > 500 {
        ledger.drain(..ledger.len() - 500);
    }
    Quest {
        ledger: Some(ledger),
        ..q.clone()
    }
}

/// Staking's totals.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct StakingTotals {
    /// Stakes settled.
    pub count: usize,
    /// What they took.
    pub staked: f64,
    /// What came back.
    pub returned: f64,
    /// Returned less staked.
    pub net: f64,
    /// Net over staked.
    pub roi: Option<f64>,
    /// Still out.
    pub outstanding: f64,
}

/// Staking's totals.
pub fn staking_totals(q: &Quest) -> StakingTotals {
    let all = q.stakes.as_deref().unwrap_or_default();
    let done: Vec<&Stake> = all.iter().filter(|s| s.result.is_some()).collect();
    let staked: f64 = done.iter().fold(0.0, |a, s| a + s.amount);
    let returned: f64 = done.iter().fold(0.0, |a, s| a + s.returned.unwrap_or(0.0));
    let outstanding = all
        .iter()
        .filter(|s| s.result.is_none())
        .fold(0.0, |a, s| a + s.amount);
    StakingTotals {
        count: done.len(),
        staked,
        returned,
        net: returned - staked,
        roi: (staked != 0.0).then(|| (returned - staked) / staked),
        outstanding,
    }
}

fn buy_in_at(stakes: [f64; 2]) -> f64 {
    100.0 * stakes[1]
}
fn late(minutes: f64) -> bool {
    let h = ((minutes % DAY) / 60.0).floor();
    h >= 22.0 || h < 4.0
}

/// A player from the session.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Played {
    /// Their bot id.
    pub id: String,
    /// Hands with you.
    #[serde(default)]
    pub hands: f64,
}

/// Maybe someone you played with asks you to stake them, as you leave a
/// table: `people` the generic players this session, `stakes` where you
/// played, `minutes` the game time, `is_regular` whether they're a room's
/// reg, `n` a counter that makes each chance its own.
pub fn maybe_ask_stake(
    q: &Quest,
    people: &[Played],
    stakes: [f64; 2],
    minutes: f64,
    is_regular: &dyn Fn(&str) -> bool,
    n: f64,
) -> Quest {
    let all = q.stakes.as_deref().unwrap_or_default();
    let owed: HashSet<&str> = all
        .iter()
        .filter(|s| s.result.as_deref() == Some("stiffed"))
        .map(|s| s.id.as_str())
        .collect();
    let busy: HashSet<&str> = all
        .iter()
        .filter(|s| s.result.is_none())
        .map(|s| s.id.as_str())
        .collect();
    let open = open_offers(q, minutes);
    let asked: HashSet<&str> = open
        .iter()
        .filter_map(|m| m.offer.as_ref()?.id.as_deref())
        .collect();
    let eligible: Vec<&Played> = people
        .iter()
        .filter(|p| {
            p.hands >= stake::MIN_HANDS
                && person_of(&p.id).is_some()
                && !owed.contains(p.id.as_str())
                && !busy.contains(p.id.as_str())
                && !asked.contains(p.id.as_str())
        })
        .collect();
    if eligible.is_empty() {
        return q.clone();
    }
    let low = stakes[1] <= 500.0;
    let chance = stake::ASK
        * if low { stake::ASK_LOW } else { 1.0 }
        * if late(minutes) { stake::ASK_LATE } else { 1.0 };
    if roll!(q.seed; "stake-ask", minutes, n) >= chance {
        return q.clone();
    }
    let who =
        eligible[(roll!(q.seed; "stake-who", minutes, n) * eligible.len() as f64).floor() as usize];
    // Someone who paid you well before asks for more.
    let paid = all
        .iter()
        .filter(|s| s.id == who.id && s.result.is_some() && s.returned.unwrap_or(0.0) > s.amount)
        .count() as f64;
    let buy_ins = stake::BUY_INS
        [(roll!(q.seed; "stake-size", minutes, n) * stake::BUY_INS.len() as f64).floor() as usize]
        + 2.0 * paid;
    let cap = (q.bankroll * stake::MAX_SHARE).floor();
    let amount = (buy_ins * buy_in_at(stakes)).min(cap);
    if amount < buy_in_at(stakes) {
        return q.clone();
    }
    let share = stake::SHARES
        [(roll!(q.seed; "stake-share", minutes, n) * stake::SHARES.len() as f64).floor() as usize];
    let name = person_of(&who.id).map(|p| p.name).unwrap_or_default();
    // A round sum: whole $50s.
    let amount = 5000f64.max(js_round(amount / 5000.0) * 5000.0);
    let offer = Offer {
        kind: "stake".into(),
        id: Some(who.id.clone()),
        stakes: Some(stakes),
        amount: Some(amount),
        share: Some(share),
        reg: Some(is_regular(&who.id)),
        expires: minutes + stake::OFFER_HOURS * 60.0,
        again: Some(paid > 0.0),
        ..Offer::default()
    };
    let text = format!(
        "{name} catches you on your way out: “Could you stake me ${} for a week at {}/{}? I’ll give you {}% of what I win.”",
        dollars(amount),
        js_num(stakes[0] / 100.0),
        js_num(stakes[1] / 100.0),
        js_num(js_round(share * 100.0))
    );
    post(
        q,
        Message {
            offer: Some(offer),
            ..Message::new("stake-ask", minutes, text)
        },
    )
}

/// Half a stake, for a counter offer: whole $50s.
pub fn half_of(amount: f64) -> f64 {
    5000f64.max(js_round(amount / 10000.0) * 5000.0)
}

// The offer in message `msg_id` answered `how`, and the message read.
pub(crate) fn answered(q: &Quest, msg_id: &str, how: &str) -> Quest {
    let inbox = q
        .inbox
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|m| {
            if m.id != msg_id {
                return m.clone();
            }
            let mut m = m.clone();
            m.read = true;
            if let Some(o) = &mut m.offer {
                o.answered = Some(how.to_string());
            }
            m
        })
        .collect();
    Quest {
        inbox: Some(inbox),
        ..q.clone()
    }
}
pub(crate) fn message<'a>(q: &'a Quest, msg_id: &str) -> Option<&'a Message> {
    q.inbox
        .as_deref()
        .unwrap_or_default()
        .iter()
        .find(|m| m.id == msg_id)
}

/// Answers a stake offer (message `msg_id`): `accept`, `decline`, or a
/// counter, `more` (a bigger share of the profit) or `less` (half the
/// money). Accepting moves the money from the bankroll into the stake.
pub fn answer_stake(q: &Quest, msg_id: &str, answer: &str, minutes: f64) -> Quest {
    let Some(o) = message(q, msg_id).and_then(|m| m.offer.clone()) else {
        return q.clone();
    };
    if o.answered.is_some() || o.expires <= minutes {
        return q.clone();
    }
    let id = o.id.clone().unwrap_or_default();
    let name = person_of(&id).map_or_else(|| "They".to_string(), |p| p.name);
    let mut amount = o.amount.unwrap_or(0.0);
    let mut share = o.share.unwrap_or(0.0);
    let note;
    let mut q = q.clone();
    if answer == "decline" {
        note = format!("You passed on staking {name}.");
    } else {
        if answer == "more" || answer == "less" {
            // A counter: they take it or walk.
            if roll!(q.seed; "stake-counter", id, minutes.floor()) >= stake::COUNTER_TAKEN {
                let q = answered(&q, msg_id, "walked");
                return post(
                    &q,
                    Message::new(
                        "stake-news",
                        minutes,
                        format!("{name} wouldn’t take that and found someone else."),
                    ),
                );
            }
            if answer == "more" {
                share = 0.8f64.min(share + 0.15);
            } else {
                amount = half_of(amount);
            }
        }
        if q.bankroll < amount {
            return q;
        }
        let end = minutes
            + (stake::DAYS
                + (roll!(q.seed; "stake-days", id, minutes.floor()) * 2.0 - 1.0) * stake::SPREAD)
                * DAY;
        let mut stakes = q.stakes.clone().unwrap_or_default();
        let n = stakes.len() as f64;
        stakes.push(Stake {
            id: id.clone(),
            stakes: o.stakes.unwrap_or_default(),
            amount,
            share,
            reg: o.reg.unwrap_or(false),
            start: minutes,
            end: js_round(end),
            n,
            ..Stake::default()
        });
        q = Quest {
            bankroll: q.bankroll - amount,
            stakes: Some(stakes),
            ..q
        };
        q = book(
            &q,
            LedgerEntry {
                at: minutes,
                kind: "stake".into(),
                amount: -amount,
                id: Some(id.clone()),
                ..LedgerEntry::default()
            },
        );
        note = format!(
            "You staked {name} ${} for {}% of the profit. You’ll hear back in about a week.",
            dollars(amount),
            js_num(js_round(share * 100.0))
        );
    }
    let q = answered(&q, msg_id, answer);
    post(&q, Message::new("stake-news", minutes, note))
}

/// How a stake went.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Outcome {
    /// `busted`, `down`, `small`, `big` or `stiffed`.
    pub result: &'static str,
    /// What came back.
    pub returned: f64,
    /// Their profit or loss.
    pub profit: f64,
}

/// How a stake went, once its week is over. Leans on the player's skill
/// and honesty; the swings are real.
pub fn stake_outcome(seed: u64, s: &Stake) -> Outcome {
    let hands = 200f64.max(((s.end - s.start) / DAY) * stake::HANDS_PER_DAY);
    let bb = s.stakes[1];
    let mean = (skill_of(&s.id) * hands) / 100.0;
    let sd = stake::SD * (hands / 100.0).sqrt();
    // A normal draw from two seeded rolls (Box-Muller).
    let u1 = 1e-9f64.max(roll!(seed; "stake-run", s.id, s.n));
    let u2 = roll!(seed; "stake-run2", s.id, s.n);
    let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
    let net = js_round((mean + z * sd) * bb);
    let last = s.amount + net;
    let honest = roll!(seed; "stake-honest", s.id, s.n) < honesty_of(&s.id, s.reg);
    let out = |result, returned, profit| Outcome {
        result,
        returned,
        profit,
    };
    if last <= s.amount * 0.05 {
        return out("busted", 0.0, -s.amount);
    }
    if last < s.amount {
        return if honest {
            out("down", js_round(last), last - s.amount)
        } else {
            out("stiffed", 0.0, last - s.amount)
        };
    }
    let profit = last - s.amount;
    if !honest {
        return out("stiffed", 0.0, profit);
    }
    let returned = s.amount + js_round(profit * s.share);
    out(
        if profit >= s.amount { "big" } else { "small" },
        returned,
        profit,
    )
}

/// Settles every stake whose week is over by game time `minutes`.
pub fn resolve_stakes(q: &Quest, minutes: f64) -> Quest {
    let all = q.stakes.as_deref().unwrap_or_default();
    let mut settled_now = Vec::new();
    let stakes: Vec<Stake> = all
        .iter()
        .map(|s| {
            if s.result.is_some() || s.end > minutes {
                return s.clone();
            }
            let o = stake_outcome(q.seed, s);
            let done = Stake {
                result: Some(o.result.to_string()),
                returned: Some(o.returned),
                profit: Some(o.profit),
                settled: Some(s.end),
                ..s.clone()
            };
            settled_now.push(done.clone());
            done
        })
        .collect();
    if settled_now.is_empty() {
        return q.clone();
    }
    let mut out = Quest {
        stakes: Some(stakes),
        ..q.clone()
    };
    for s in settled_now {
        let name = person_of(&s.id).map_or_else(|| "Someone you staked".to_string(), |p| p.name);
        let (amount, returned) = (dollars(s.amount), s.returned.unwrap_or(0.0));
        let result = s.result.clone().unwrap_or_default();
        let text = match result.as_str() {
            "busted" => format!("{name} lost the whole ${amount} you put up. That’s poker."),
            "down" => format!(
                "{name} had a losing week and handed back ${} of your ${amount}.",
                dollars(returned)
            ),
            "small" => format!(
                "{name} won a little: you got your ${amount} back plus ${}.",
                dollars(returned - s.amount)
            ),
            "big" => format!(
                "{name} had a great week and paid you ${} on your ${amount} stake.",
                dollars(returned)
            ),
            _ => format!(
                "You haven’t heard from {name} since you staked them ${amount}. Word is they had a decent week."
            ),
        };
        if returned != 0.0 {
            out.bankroll += returned;
        }
        let at = s.settled.unwrap_or(s.end);
        out = book(
            &out,
            LedgerEntry {
                at,
                kind: "stake-back".into(),
                amount: returned,
                id: Some(s.id.clone()),
                result: Some(result.clone()),
                ..LedgerEntry::default()
            },
        );
        out = post(
            &out,
            Message {
                result: Some(result),
                ..Message::new("stake-result", at, text)
            },
        );
    }
    out
}

/// What `id` owes you on stakes they stiffed you on, not yet confronted.
pub fn owed_by(q: &Quest, id: &str) -> f64 {
    q.stakes
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter(|s| s.stiffed_open(id))
        .fold(0.0, |a, s| {
            let profit = s.profit.unwrap_or(0.0);
            a + if profit > 0.0 {
                s.amount + js_round(profit * s.share)
            } else {
                0f64.max(s.amount + profit)
            }
        })
}

/// Confronts `id` about what they owe: they pay it, or some of it, or
/// nothing, leaning on their honesty.
pub fn confront(q: &Quest, id: &str, minutes: f64) -> Quest {
    let owed = owed_by(q, id);
    if owed == 0.0 {
        return q.clone();
    }
    let all = q.stakes.as_deref().unwrap_or_default();
    let Some(first) = all.iter().position(|s| s.stiffed_open(id)) else {
        return q.clone();
    };
    let chance = confronting::PAY_BASE + confronting::PAY_HONEST * honesty_of(id, all[first].reg);
    let r = roll!(q.seed; "confront", id, minutes.floor());
    let name = person_of(id).map_or_else(|| "They".to_string(), |p| p.name);
    let paid = if r < chance * 0.6 {
        owed
    } else if r < chance {
        js_round(owed / 2.0 / 100.0) * 100.0
    } else {
        0.0
    };
    // What they pay is counted on the first stake they owe on; every one of them is settled.
    let stakes = all
        .iter()
        .enumerate()
        .map(|(i, s)| {
            if !s.stiffed_open(id) {
                return s.clone();
            }
            Stake {
                confronted: Some(true),
                returned: Some(s.returned.unwrap_or(0.0) + if i == first { paid } else { 0.0 }),
                ..s.clone()
            }
        })
        .collect();
    let out = Quest {
        bankroll: q.bankroll + paid,
        stakes: Some(stakes),
        ..q.clone()
    };
    let out = book(
        &out,
        LedgerEntry {
            at: minutes,
            kind: "stake-back".into(),
            amount: paid,
            id: Some(id.to_string()),
            result: Some("confronted".into()),
            ..LedgerEntry::default()
        },
    );
    let text = if paid == owed {
        format!(
            "You asked {name} about the stake. Embarrassed, they paid the ${} they owed.",
            dollars(paid)
        )
    } else if paid != 0.0 {
        format!(
            "You asked {name} about the stake. They paid ${} and promised the rest “soon”.",
            dollars(paid)
        )
    } else {
        format!(
            "You asked {name} about the stake. They shrugged and said they’d had a bad week. You won’t see that money."
        )
    };
    post(&out, Message::new("stake-news", minutes, text))
}
