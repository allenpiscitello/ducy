//! The range trainer's practice (ducy-web #225, #464): a top-% target in
//! Hold'em or Omaha, hands dealt for it, and the score that climbs.
//!
//! The score picks how close to the cutoff hands are dealt ([`LEVELS`]).
//! Closeness is relative to each side of the cutoff: for a hand inside the
//! range, how far in as a share of the cutoff; outside, how far out as a
//! share of the rest. Obvious hands (deep inside, or far outside) aren't
//! dealt except as occasional probes for major mistakes, and practice leans
//! toward the kinds of hands the player keeps getting wrong ([`diagnose`]).
//!
//! The dice come from a seed the page passes, a fresh one for each deal.

use std::collections::BTreeMap;

use ducy::deck::Card;
use ducy::games::omaha_analysis as oa;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::holdem::{HOLDEM_TAG_LABELS, holdem_tag};
use crate::holdem_rank::HOLDEM_PERCENTILE;
use crate::skills::{MASTERY, gain_for};

const RANKS: &str = "AKQJT98765432";
const SUITS: &str = "shdc";

/// Numbers in `0..1` from a seed (splitmix64).
pub struct Dice(u64);

impl Dice {
    /// Dice from `seed`.
    pub fn new(seed: u64) -> Self {
        Dice(seed)
    }
    /// The next number in `0..1`.
    pub fn roll(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }
    fn below(&mut self, n: usize) -> usize {
        ((self.roll() * n as f64) as usize).min(n.saturating_sub(1))
    }
}

fn rank_at(c: &str) -> usize {
    c.chars()
        .next()
        .and_then(|r| RANKS.find(r))
        .unwrap_or(RANKS.len())
}
fn suit_at(c: &str) -> usize {
    c.chars()
        .nth(1)
        .and_then(|s| SUITS.find(s))
        .unwrap_or(SUITS.len())
}

// The deck in the page's order: each suit's ranks, ace first.
fn deck() -> Vec<String> {
    SUITS
        .chars()
        .flat_map(|s| RANKS.chars().map(move |r| format!("{r}{s}")))
        .collect()
}

/// A Hold'em hand, the higher rank first, the way players read a hand.
pub fn deal_holdem(d: &mut Dice) -> Vec<String> {
    let deck = deck();
    let i = d.below(52);
    let mut j = d.below(51);
    if j >= i {
        j += 1;
    }
    let (a, b) = (deck[i].clone(), deck[j].clone());
    if rank_at(&a) <= rank_at(&b) {
        vec![a, b]
    } else {
        vec![b, a]
    }
}

/// An Omaha hand of `n` cards, sorted high to low (then by suit) for reading.
pub fn deal_omaha(n: usize, d: &mut Dice) -> Vec<String> {
    let mut deck = deck();
    for i in 0..n {
        let j = i + d.below(52 - i);
        deck.swap(i, j);
    }
    let mut hand: Vec<String> = deck.into_iter().take(n).collect();
    hand.sort_by_key(|c| (rank_at(c), suit_at(c)));
    hand
}

/// A hand of `n` hole cards: Hold'em for 2, Omaha for 4 to 6.
pub fn deal_top(n: usize, d: &mut Dice) -> Vec<String> {
    if n == 2 {
        deal_holdem(d)
    } else {
        deal_omaha(n, d)
    }
}

/// The class of two cards like `Ah`, `Kh`: `AKs`.
pub fn hand_class(c1: &str, c2: &str) -> String {
    let (a, b) = if rank_at(c1) <= rank_at(c2) {
        (c1, c2)
    } else {
        (c2, c1)
    };
    let (ra, rb) = (&a[..1], &b[..1]);
    if ra == rb {
        format!("{ra}{rb}")
    } else {
        format!(
            "{ra}{rb}{}",
            if suit_at(a) == suit_at(b) { 's' } else { 'o' }
        )
    }
}

/// Where Hold'em class `class` falls in the trainer's ranking, 0 (best) to 100.
pub fn holdem_percentile(class: &str) -> Option<f64> {
    HOLDEM_PERCENTILE
        .iter()
        .find(|(c, _)| *c == class)
        .map(|(_, p)| *p)
}

fn cards_of(cards: &[String]) -> Option<Vec<Card>> {
    cards.iter().map(|c| Card::parse(c).ok()).collect()
}

/// Where a hand falls among hands of its size, 0 (best) to 100: Hold'em by
/// the trainer's ranking, Omaha by ducy's model.
pub fn top_percentile(cards: &[String]) -> f64 {
    let p = if cards.len() == 2 {
        holdem_percentile(&hand_class(&cards[0], &cards[1]))
    } else {
        cards_of(cards).and_then(|h| oa::percentile(&h))
    };
    p.unwrap_or(100.0)
}

/// A hand's kinds, for spotting patterns in mistakes.
pub fn top_tags(cards: &[String]) -> Vec<String> {
    if cards.len() == 2 {
        let c: Vec<&str> = cards.iter().map(String::as_str).collect();
        holdem_tag(&c)
            .map(|t| vec![t.to_string()])
            .unwrap_or_default()
    } else {
        cards_of(cards)
            .map(|h| oa::hand_tags(&h).into_iter().map(str::to_string).collect())
            .unwrap_or_default()
    }
}

/// Each kind of hand at `n` hole cards and its description, in the page's
/// order (Omaha's by name).
pub fn tag_labels(n: usize) -> Vec<(&'static str, &'static str)> {
    if n == 2 {
        HOLDEM_TAG_LABELS.to_vec()
    } else {
        let mut l = crate::omaha::OMAHA_TAG_LABELS.to_vec();
        l.sort_by_key(|(t, _)| *t);
        l
    }
}

fn label_of(n: usize, tag: &str) -> String {
    tag_labels(n)
        .into_iter()
        .find(|(t, _)| *t == tag)
        .map_or_else(|| tag.to_string(), |(_, l)| l.to_string())
}

/// A hand whatever its suits: its ranks, and how many cards share each suit
/// (AKs and AKo differ; A♠K♠ and A♥K♥ don't).
pub fn hand_kind(cards: &[String]) -> String {
    let mut ranks: Vec<char> = cards.iter().filter_map(|c| c.chars().next()).collect();
    ranks.sort_by_key(|r| RANKS.find(*r).unwrap_or(RANKS.len()));
    let mut suits: Vec<(char, usize)> = Vec::new();
    for c in cards {
        let Some(s) = c.chars().nth(1) else { continue };
        match suits.iter_mut().find(|(x, _)| *x == s) {
            Some((_, n)) => *n += 1,
            None => suits.push((s, 1)),
        }
    }
    let mut counts: Vec<usize> = suits.into_iter().map(|(_, n)| n).collect();
    counts.sort_by(|a, b| b.cmp(a));
    let counts: String = counts.iter().map(usize::to_string).collect();
    format!("{}:{counts}", ranks.into_iter().collect::<String>())
}

/// A hand dealt for practice.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Dealt {
    /// The cards.
    pub cards: Vec<String>,
    /// Where it falls, 0 (best) to 100.
    pub pct: f64,
    /// An obvious hand, dealt to catch a major mistake.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub probe: bool,
    /// Dealt to work on this kind of hand.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
}

fn dealt(cards: Vec<String>) -> Dealt {
    let pct = top_percentile(&cards);
    Dealt {
        cards,
        pct,
        ..Dealt::default()
    }
}

/// Deals with `deal` until the hand isn't the same as `prev`, suits aside;
/// tries a few dozen times.
pub fn deal_not_again(mut deal: impl FnMut() -> Dealt, prev: Option<&[String]>) -> Dealt {
    let kind = prev.map(hand_kind);
    let mut h = deal();
    for _ in 0..50 {
        if kind.as_deref() != Some(hand_kind(&h.cards).as_str()) {
            break;
        }
        h = deal();
    }
    h
}

/// A practice level: from its score, how close to the cutoff hands come.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Level {
    /// Its name.
    pub name: &'static str,
    /// The score it starts at.
    pub from: u32,
    /// Relative distance from the cutoff, `[min, max)`.
    pub band: [f64; 2],
}

/// The levels, easiest first.
pub const LEVELS: [Level; 4] = [
    Level {
        name: "Easy",
        from: 0,
        band: [0.4, 1.0],
    },
    Level {
        name: "Medium",
        from: 50,
        band: [0.15, 0.4],
    },
    Level {
        name: "Hard",
        from: 80,
        band: [0.05, 0.2],
    },
    Level {
        name: "Challenge",
        from: 90,
        band: [0.0, 0.08],
    },
];
// Deeper than this into the range is an obvious play; further out, an obvious fold.
const OBVIOUS_INSIDE: f64 = 0.75;
const OBVIOUS_OUTSIDE: f64 = 0.5;
const PROBE_RATE: f64 = 0.1;
const FOCUS_RATE: f64 = 0.4;
/// A miss this close to the cutoff is a close call rather than a mistake.
pub const CLOSE_CALL: f64 = 0.2;

/// The level at `score`.
pub fn level_for(score: f64) -> Level {
    *LEVELS
        .iter()
        .rev()
        .find(|l| score >= l.from as f64)
        .unwrap_or(&LEVELS[0])
}

/// Relative distance from the cutoff: 0 at the edge, 1 at the far end of its side.
pub fn closeness(p: f64, cut: f64) -> f64 {
    if p < cut {
        (cut - p) / cut
    } else {
        (p - cut) / (100.0 - cut)
    }
}

/// An obvious play or fold.
pub fn is_obvious(p: f64, cut: f64) -> bool {
    closeness(p, cut)
        > if p < cut {
            OBVIOUS_INSIDE
        } else {
            OBVIOUS_OUTSIDE
        }
}

/// How bad a wrong answer is, 0 (right at the cutoff) to 1: the distance
/// from the target against the target itself, the same on both sides.
pub fn severity_for(pct: f64, cut: f64) -> f64 {
    1f64.min((pct - cut).abs() / cut)
}

/// Points lost for a miss: 1 for a hair-thin one up to 16; a missed probe 20.
pub fn penalty_for(severity: f64, probe: bool) -> u32 {
    if probe {
        20
    } else {
        1u32.max((16.0 * severity).round() as u32)
    }
}

/// A non-obvious hand on the given side within the band (and of `tag`).
pub fn find_in_band(
    cards: usize,
    cut: f64,
    band: [f64; 2],
    tag: Option<&str>,
    inside: bool,
    d: &mut Dice,
) -> Option<Dealt> {
    for _ in 0..8000 {
        let h = deal_top(cards, d);
        let p = top_percentile(&h);
        if (p < cut) != inside || is_obvious(p, cut) {
            continue;
        }
        let r = closeness(p, cut);
        if r < band[0] || r >= band[1] {
            continue;
        }
        if tag.is_some_and(|t| !top_tags(&h).iter().any(|x| x == t)) {
            continue;
        }
        return Some(Dealt {
            cards: h,
            pct: p,
            ..Dealt::default()
        });
    }
    None
}

/// A non-obvious hand on one side of the cutoff (`in`, `out`, or either at
/// even odds, so plays and folds stay balanced) within the band, of `tag`
/// if given; relaxing the tag, then the band, rather than giving up.
pub fn deal_in_band(
    cards: usize,
    cut: f64,
    band: [f64; 2],
    tag: Option<&str>,
    side: Option<&str>,
    d: &mut Dice,
) -> Dealt {
    let inside = match side {
        Some(s) => s == "in",
        None => d.roll() < 0.5,
    };
    if let Some(found) = find_in_band(cards, cut, band, tag, inside, d) {
        return found;
    }
    if tag.is_some() {
        return deal_in_band(cards, cut, band, None, side, d);
    }
    if band[0] > 0.0 || band[1] < 1.0 {
        return deal_in_band(cards, cut, [0.0, 1.0], None, side, d);
    }
    dealt(deal_top(cards, d))
}

/// An obvious hand, to catch a major mistake.
pub fn deal_probe(cards: usize, cut: f64, d: &mut Dice) -> Dealt {
    let inside = d.roll() < 0.5;
    for _ in 0..8000 {
        let h = deal_top(cards, d);
        let p = top_percentile(&h);
        if (p < cut) == inside && is_obvious(p, cut) {
            return Dealt {
                cards: h,
                pct: p,
                probe: true,
                focus: None,
            };
        }
    }
    dealt(deal_top(cards, d))
}

/// A drill hand: the drilled kind, on either side of the cutoff, preferring
/// the level's band and non-obvious hands but falling back rather than
/// giving up.
pub fn deal_drill(cards: usize, cut: f64, band: [f64; 2], tag: &str, d: &mut Dice) -> Dealt {
    let sides = if d.roll() < 0.5 {
        [true, false]
    } else {
        [false, true]
    };
    for b in [band, [0.0, 1.0]] {
        for inside in sides {
            if let Some(r) = find_in_band(cards, cut, b, Some(tag), inside, d) {
                return r;
            }
        }
    }
    for _ in 0..20000 {
        let h = deal_top(cards, d);
        if top_tags(&h).iter().any(|x| x == tag) {
            return dealt(h);
        }
    }
    dealt(deal_top(cards, d))
}

/// Hands kinds worth drilling need this share of their hands on each side.
const DRILL_MIN_SHARE: f64 = 0.04;
/// Hands of a kind needed before calling it weak.
const WEAK_MIN_SEEN: usize = 8;

/// Kinds worth drilling at the cutoff: ones with real decisions on both
/// sides. Exact for Hold'em, sampled for Omaha.
pub fn drill_tags(cards: usize, pct: f64, d: &mut Dice) -> Vec<String> {
    let mut count: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    let mut add = |tags: Vec<String>, inside: bool, w: f64| {
        for t in tags {
            let c = count.entry(t).or_default();
            if inside {
                c.0 += w;
            } else {
                c.1 += w;
            }
        }
    };
    if cards == 2 {
        for (cls, p) in HOLDEM_PERCENTILE {
            let b = cls.as_bytes();
            let (r1, r2) = (b[0] as char, b[1] as char);
            let h = if cls.len() == 3 && b[2] == b's' {
                [format!("{r1}s"), format!("{r2}s")]
            } else {
                [format!("{r1}s"), format!("{r2}h")]
            };
            let combos = if cls.len() == 2 {
                6.0
            } else if b[2] == b's' {
                4.0
            } else {
                12.0
            };
            add(top_tags(&h), p < pct, combos);
        }
    } else {
        for _ in 0..6000 {
            let h = deal_omaha(cards, d);
            add(top_tags(&h), top_percentile(&h) < pct, 1.0);
        }
    }
    tag_labels(cards)
        .into_iter()
        .filter(|(t, _)| {
            count
                .get(*t)
                .is_some_and(|&(i, o)| i.min(o) / (i + o) >= DRILL_MIN_SHARE)
        })
        .map(|(t, _)| t.to_string())
        .collect()
}

/// A kind of hand and how it's gone lately.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Weak {
    /// The kind.
    pub tag: String,
    /// Hands of it lately.
    pub seen: usize,
    /// The share right.
    pub acc: f64,
}

/// Recent accuracy per kind for a target's record (its `kinds`), weakest first.
pub fn weakest_kinds(rec: Option<&Value>, tags: &[String]) -> Vec<Weak> {
    let kinds = rec.and_then(|r| r.get("kinds"));
    let mut out: Vec<Weak> = tags
        .iter()
        .filter_map(|t| {
            let recent = kinds
                .and_then(|k| k.get(t))
                .and_then(|k| k.get("recent"))
                .and_then(Value::as_str)
                .unwrap_or("");
            (recent.len() >= WEAK_MIN_SEEN).then(|| Weak {
                tag: t.clone(),
                seen: recent.len(),
                acc: recent.chars().filter(|c| *c == '1').count() as f64 / recent.len() as f64,
            })
        })
        .collect();
    out.sort_by(|a, b| a.acc.total_cmp(&b.acc));
    out
}

/// An answer in a practice session.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Answer {
    /// The hand.
    pub cards: Vec<String>,
    /// Where it falls.
    pub pct: f64,
    /// Played (true) or folded.
    pub answer: bool,
    /// Right.
    pub correct: bool,
    /// How close to the cutoff.
    pub rel: f64,
    /// How bad a miss.
    pub severity: f64,
    /// Points from the cutoff.
    pub distance: f64,
    /// A missed probe.
    pub major: bool,
    /// Dealt to work on a weak spot.
    pub focus: bool,
    /// Its kinds.
    pub tags: Vec<String>,
    /// The level it was dealt at.
    pub level: String,
}

/// A kind's answers in a session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TagStats {
    /// Dealt inside the range.
    pub seen_in: u32,
    /// Dealt outside.
    pub seen_out: u32,
    /// Played though outside.
    pub over: u32,
    /// Folded though inside.
    pub under: u32,
}

/// What's said about a miss.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Feedback {
    /// The hand.
    pub cards: Vec<String>,
    /// Where it falls.
    pub pct: f64,
    /// Played.
    pub yes: bool,
    /// An obvious hand.
    pub major: bool,
    /// Close to the cutoff.
    pub close: bool,
    /// The trend it fits, when there's one to mention.
    pub pattern: Option<String>,
}

/// A practice session: the page's, with what it keeps besides.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Practice {
    /// Hole cards: 2 for Hold'em, 4 to 6 for Omaha.
    pub cards: usize,
    /// The target: top `pct`%.
    pub pct: f64,
    /// A kind drilled on its own.
    pub drill: Option<String>,
    /// The score, 0 to 100.
    pub score: f64,
    /// The best score.
    pub best: f64,
    /// Hands answered at this target, ever.
    pub total: f64,
    /// Hands answered this session.
    pub answered: f64,
    /// Right this session.
    pub correct: f64,
    /// Right in a row.
    pub streak: f64,
    /// This session's answers.
    pub history: Vec<Answer>,
    /// This session's answers by kind.
    pub tag_stats: BTreeMap<String, TagStats>,
    /// What's said about the last miss.
    pub feedback: Option<Feedback>,
    /// The last answer's change in score.
    pub delta: f64,
    /// Just mastered.
    pub mastered: bool,
    /// The hand dealt.
    pub current: Option<Dealt>,
    /// When a trend was last mentioned.
    pub last_trend_at: Option<f64>,
    /// Anything else the page keeps.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

// Patterns are only called out with a long record behind them.
const DIAG_MIN_HANDS: f64 = 20.0;
const DIAG_MIN_ERRORS: usize = 6;
const TAG_MIN_ERRORS: u32 = 3;
const TAG_MIN_RATE: f64 = 0.5;
const TREND_NOTE_GAP: f64 = 8.0;

/// A kind behind one direction of mistake.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TagMisses {
    /// The kind.
    pub tag: String,
    /// Mistakes.
    pub n: u32,
    /// Of hands.
    pub of: u32,
}

/// Which way a player's mistakes lean.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnosis {
    /// `loose` (playing hands outside the range), `tight` (folding hands
    /// inside it) or `mixed` (both).
    pub kind: &'static str,
    /// Played though outside.
    pub over: usize,
    /// Folded though inside.
    pub under: usize,
    /// Hands dealt outside.
    pub seen_out: usize,
    /// Inside.
    pub seen_in: usize,
    /// The kinds played too often.
    pub over_tags: Vec<TagMisses>,
    /// The kinds folded too often.
    pub under_tags: Vec<TagMisses>,
}

fn tags_for(pr: &Practice, over: bool) -> Vec<TagMisses> {
    let mut out: Vec<TagMisses> = pr
        .tag_stats
        .iter()
        .map(|(tag, s)| TagMisses {
            tag: tag.clone(),
            n: if over { s.over } else { s.under },
            of: if over { s.seen_out } else { s.seen_in },
        })
        .filter(|x| x.n >= TAG_MIN_ERRORS && x.n as f64 / x.of as f64 >= TAG_MIN_RATE)
        .collect();
    out.sort_by(|a, b| {
        b.n.cmp(&a.n)
            .then((b.n as f64 / b.of as f64).total_cmp(&(a.n as f64 / a.of as f64)))
    });
    out.truncate(3);
    out
}

/// Which way the session's mistakes lean, once there's a long record.
pub fn diagnose(pr: &Practice) -> Option<Diagnosis> {
    if pr.answered < DIAG_MIN_HANDS
        || pr.history.iter().filter(|x| !x.correct).count() < DIAG_MIN_ERRORS
    {
        return None;
    }
    // Judged from regular deals only: hands dealt to focus on a weak spot are
    // a biased sample and would feed back into it.
    let regular: Vec<&Answer> = pr.history.iter().filter(|x| !x.focus).collect();
    let misses: Vec<&&Answer> = regular.iter().filter(|x| !x.correct).collect();
    let over = misses.iter().filter(|x| x.answer).count();
    let under = misses.len() - over;
    let seen_out = regular.iter().filter(|x| x.pct >= pr.pct).count();
    let seen_in = regular.len() - seen_out;
    let rate = |n: usize, of: usize| if of > 0 { n as f64 / of as f64 } else { 0.0 };
    let (over_rate, under_rate) = (rate(over, seen_out), rate(under, seen_in));
    let (lo, hi) = (over_rate.min(under_rate), over_rate.max(under_rate));
    let kind = if over >= 3 && under >= 3 && lo >= 0.2 && lo >= hi / 3.0 {
        "mixed"
    } else if over >= 4 && (over_rate >= 2.0 * under_rate || under < 3) {
        // A couple of stray mistakes the other way don't hide a clear lean.
        "loose"
    } else if under >= 4 && (under_rate >= 2.0 * over_rate || over < 3) {
        "tight"
    } else if over >= 3 && under >= 3 {
        "mixed"
    } else {
        return None;
    };
    Some(Diagnosis {
        kind,
        over,
        under,
        seen_out,
        seen_in,
        over_tags: tags_for(pr, true),
        under_tags: tags_for(pr, false),
    })
}

/// A diagnosis as text: a headline and details.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DiagnosisText {
    /// The headline.
    pub headline: String,
    /// What's behind it.
    pub details: Vec<String>,
}

/// A diagnosis at `cards` hole cards and top `cut`% as text.
pub fn diagnosis_text(d: &Diagnosis, cards: usize, cut: f64) -> DiagnosisText {
    let list = |xs: &[TagMisses]| {
        xs.iter()
            .map(|x| format!("{} ({} of {})", label_of(cards, &x.tag), x.n, x.of))
            .collect::<Vec<_>>()
            .join("; ")
    };
    let cut = crate::js_number(cut);
    if d.kind == "loose" {
        return DiagnosisText {
            headline: format!(
                "You’re playing too many hands: you played {} of {} hands outside your top {cut}%.",
                d.over, d.seen_out
            ),
            details: vec![if d.over_tags.is_empty() {
                "The extra hands are spread across all kinds rather than one type, so tighten up overall.".to_string()
            } else {
                format!("Most of the extra hands: {}.", list(&d.over_tags))
            }],
        };
    }
    if d.kind == "tight" {
        return DiagnosisText {
            headline: format!(
                "You’re folding hands you should play: you folded {} of {} hands inside your top {cut}%.",
                d.under, d.seen_in
            ),
            details: vec![if d.under_tags.is_empty() {
                "The missed hands are spread across all kinds, so loosen up overall.".to_string()
            } else {
                format!("Most often missed: {}.", list(&d.under_tags))
            }],
        };
    }
    let both: Vec<String> = d
        .over_tags
        .iter()
        .filter(|o| d.under_tags.iter().any(|u| u.tag == o.tag))
        .map(|o| label_of(cards, &o.tag))
        .collect();
    let mut details = Vec::new();
    if !d.over_tags.is_empty() {
        details.push(format!(
            "Played when they’re outside the range: {}.",
            list(&d.over_tags)
        ));
    }
    if !d.under_tags.is_empty() {
        details.push(format!(
            "Folded when they’re inside the range: {}.",
            list(&d.under_tags)
        ));
    }
    if !both.is_empty() {
        details.push(format!("With {}, you’re going both ways, so look at what separates the good ones from the bad ones.", both.join(" and ")));
    }
    if details.is_empty() {
        details.push("The mix-ups aren’t concentrated in one kind of hand yet.".to_string());
    }
    DiagnosisText {
        headline: format!(
            "You’re mixing up good and bad hands: you played {} hands outside your top {cut}% and folded {} inside it.",
            d.over, d.under
        ),
        details,
    }
}

// The kind to deal more of while practising, and the side of the cutoff its
// mistakes are on: overplayed kinds are tested outside the range, overfolded
// ones inside it. A mixed player gets both sides in turn.
fn focus_for(pr: &Practice, d: &mut Dice) -> Option<(String, &'static str)> {
    let diag = diagnose(pr)?;
    let over = diag.over_tags.first().map(|x| (x.tag.clone(), "out"));
    let under = diag.under_tags.first().map(|x| (x.tag.clone(), "in"));
    match diag.kind {
        "loose" => over,
        "tight" => under,
        _ => match (over, under) {
            (Some(o), Some(u)) => Some(if d.roll() < 0.5 { o } else { u }),
            (o, u) => o.or(u),
        },
    }
}

/// The next hand to deal in a session: never the same hand twice in a row,
/// even in other suits; a drill's kind, now and then a probe, and while the
/// record shows a weak spot, often a hand of that kind.
pub fn practice_next(pr: &Practice, d: &mut Dice) -> Dealt {
    let band = level_for(pr.score).band;
    let prev = pr.current.as_ref().map(|c| c.cards.clone());
    let prev = prev.as_deref();
    if let Some(tag) = &pr.drill {
        return deal_not_again(|| deal_drill(pr.cards, pr.pct, band, tag, d), prev);
    }
    let focus = focus_for(pr, d);
    if pr.answered >= 4.0 && d.roll() < PROBE_RATE {
        return deal_not_again(|| deal_probe(pr.cards, pr.pct, d), prev);
    }
    if let Some((tag, side)) = focus.filter(|_| d.roll() < FOCUS_RATE) {
        let h = deal_not_again(
            || deal_in_band(pr.cards, pr.pct, band, Some(&tag), Some(side), d),
            prev,
        );
        return Dealt {
            focus: Some(tag),
            ..h
        };
    }
    deal_not_again(|| deal_in_band(pr.cards, pr.pct, band, None, None, d), prev)
}

/// Answers the hand dealt: `yes` plays it. The score moves (up less as it
/// climbs, down more the more obvious the hand), the session's record grows,
/// and a miss gets its feedback, with the trend it fits now and then.
/// `None` when there's nothing to answer.
pub fn practice_answer(pr: &Practice, yes: bool) -> Option<Practice> {
    if pr.feedback.is_some() || pr.mastered {
        return None;
    }
    let cur = pr.current.clone()?;
    let mut pr = pr.clone();
    let in_range = cur.pct < pr.pct;
    let correct = yes == in_range;
    let rel = closeness(cur.pct, pr.pct);
    let severity = severity_for(cur.pct, pr.pct);
    let before = pr.score;
    pr.score = if correct {
        100f64.min(pr.score + gain_for(pr.score as u32) as f64)
    } else {
        0f64.max(pr.score - penalty_for(severity, cur.probe) as f64)
    };
    pr.delta = pr.score - before;
    pr.answered += 1.0;
    pr.total += 1.0;
    pr.best = pr.best.max(pr.score);
    if correct {
        pr.correct += 1.0;
        pr.streak += 1.0;
    } else {
        pr.streak = 0.0;
    }
    let tags = top_tags(&cur.cards);
    for t in &tags {
        let s = pr.tag_stats.entry(t.clone()).or_default();
        if in_range {
            s.seen_in += 1;
        } else {
            s.seen_out += 1;
        }
        if !correct {
            if yes {
                s.over += 1;
            } else {
                s.under += 1;
            }
        }
    }
    pr.history.push(Answer {
        cards: cur.cards.clone(),
        pct: cur.pct,
        answer: yes,
        correct,
        rel,
        severity,
        distance: (cur.pct - pr.pct).abs(),
        major: !correct && cur.probe,
        focus: cur.focus.is_some(),
        tags,
        level: level_for(before).name.to_string(),
    });
    // Mastery is announced when the score reaches it, and again only after
    // dropping below and climbing back.
    if pr.score >= MASTERY as f64 && before < MASTERY as f64 {
        pr.mastered = true;
        return Some(pr);
    }
    if !correct {
        // Mention the trend only with a long record behind it, when this
        // mistake fits it, and not more often than every TREND_NOTE_GAP hands.
        let d = diagnose(&pr);
        let fits = d
            .as_ref()
            .is_some_and(|d| d.kind == "mixed" || (d.kind == "loose") == yes);
        let due = pr
            .last_trend_at
            .is_none_or(|at| pr.answered - at >= TREND_NOTE_GAP);
        let mut pattern = None;
        if let (true, true, Some(d)) = (fits, due, &d) {
            let t = diagnosis_text(d, pr.cards, pr.pct);
            pattern = Some(
                std::iter::once(t.headline)
                    .chain(t.details)
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            pr.last_trend_at = Some(pr.answered);
        }
        pr.feedback = Some(Feedback {
            cards: cur.cards,
            pct: cur.pct,
            yes,
            major: cur.probe,
            close: !cur.probe && severity < CLOSE_CALL,
            pattern,
        });
    }
    Some(pr)
}

/// Example hands just inside and just outside a cutoff, for the setup screen.
pub fn top_examples(cards: usize, pct: f64, d: &mut Dice) -> (Vec<Dealt>, Vec<Dealt>) {
    let (mut inside, mut outside) = (Vec::new(), Vec::new());
    for _ in 0..20000 {
        if inside.len() >= 3 && outside.len() >= 3 {
            break;
        }
        let h = dealt(deal_top(cards, d));
        let p = h.pct;
        if p < pct && p >= pct - 3.0 && inside.len() < 3 {
            inside.push(h);
        } else if p >= pct && p < pct + 3.0 && outside.len() < 3 {
            outside.push(h);
        }
    }
    (inside, outside)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(cards: &[&str]) -> Vec<String> {
        cards.iter().map(|c| c.to_string()).collect()
    }

    #[test]
    fn hands_and_their_kinds() {
        assert_eq!(hand_kind(&s(&["As", "Ks"])), hand_kind(&s(&["Kh", "Ah"])));
        assert_ne!(hand_kind(&s(&["As", "Ks"])), hand_kind(&s(&["As", "Kd"])));
        assert_eq!(hand_class("Kh", "Ah"), "AKs");
        assert_eq!(hand_class("7c", "7d"), "77");
        assert_eq!(top_percentile(&s(&["Ah", "As"])), 0.23);
        let mut d = Dice::new(3);
        for _ in 0..200 {
            let h = deal_omaha(4, &mut d);
            assert_eq!(h.len(), 4);
            assert!((0.0..=100.0).contains(&top_percentile(&h)));
            let h = deal_holdem(&mut d);
            assert_ne!(h[0], h[1]);
            assert!(rank_at(&h[0]) <= rank_at(&h[1]));
        }
    }

    #[test]
    fn deals_fall_in_the_level_band() {
        let mut d = Dice::new(9);
        for _ in 0..50 {
            let h = deal_in_band(2, 20.0, [0.15, 0.4], None, None, &mut d);
            let r = closeness(h.pct, 20.0);
            assert!((0.15..0.4).contains(&r), "{h:?}");
            let p = deal_probe(2, 20.0, &mut d);
            assert!(p.probe && is_obvious(p.pct, 20.0));
        }
        assert_eq!(level_for(85.0).name, "Hard");
        assert_eq!(penalty_for(0.0, false), 1);
        assert_eq!(penalty_for(1.0, true), 20);
    }

    #[test]
    fn a_session_scores_and_finds_a_lean() {
        let mut pr = Practice {
            cards: 2,
            pct: 20.0,
            ..Practice::default()
        };
        let mut d = Dice::new(1);
        // Playing every hand: loose.
        for _ in 0..40 {
            pr.current = Some(practice_next(&pr, &mut d));
            pr = practice_answer(&pr, true).unwrap();
            pr.feedback = None;
        }
        assert_eq!(pr.answered, 40.0);
        assert_eq!(diagnose(&pr).unwrap().kind, "loose");
        // At top 20% every suited ace is a play, so there's nothing to drill;
        // offsuit broadway hands fall both sides.
        let tags = drill_tags(2, 20.0, &mut d);
        assert!(
            tags.contains(&"broadway_offsuit".to_string())
                && !tags.contains(&"ace_suited".to_string()),
            "{tags:?}"
        );
    }
}
