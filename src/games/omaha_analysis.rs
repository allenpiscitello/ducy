//! Omaha starting-hand analysis (4, 5 or 6 cards): the features that make a
//! hand play well, recognisable kinds of hands ("tags"), how often it
//! blocks the nut flush, and a ranking by percentile from a model fitted to
//! engine measurements ([`omaha_rank_model`](super::omaha_rank_model)).
//!
//! These came from ducy.cards' trainer, so the page, the bots and the PLO
//! GTO work (card abstraction, #128) can share one copy.
//!
//! ```
//! use ducy::deck::Deck;
//! use ducy::games::omaha_analysis::{hand_tags, percentile};
//!
//! let hand: Vec<_> = Deck::parse("As Ad Kh Qh").unwrap().iter(true).collect();
//! assert!(percentile(&hand).unwrap() < 1.0); // AAKQ double-suited is a top hand
//! assert!(hand_tags(&hand).contains(&"aa"));
//! ```

use crate::deck::{Card, Suit};
use crate::games::omaha_rank_model::{OmahaRankModel, PLO4, PLO5, PLO6};
use crate::ranking::standard_hand_ranker::RankOrder;

/// The features [`hand_features`] returns, in order.
pub const FEATURE_NAMES: [&str; 43] = [
    "bias",
    "high_card_sum",
    "aces",
    "kings",
    "queens",
    "broadways",
    "low_cards",
    "pairs",
    "top_pair_rank",
    "second_pair_rank",
    "pair_aa",
    "pair_kk",
    "pair_qq",
    "pair_jj_tt",
    "trips_or_more",
    "suits_with_2",
    "suits_with_3plus",
    "rainbow",
    "suited_ace",
    "suited_king",
    "suited_low_only",
    "straights_2",
    "straights_3",
    "straights_4",
    "max_in_straight",
    "connectors",
    "one_gaps",
    "two_gaps",
    "rundown_4",
    "danglers",
    "aa_suited",
    "pair_suited_with_side",
    "double_suited_connected",
    "broadway_rundown",
    // Interactions: a big pair is only as good as what comes with it.
    "aa_danglers",
    "aa_rainbow",
    "aa_double_suited",
    "aa_side_connected",
    "kk_qq_double_suited",
    "kk_qq_danglers",
    "pair_side_connected",
    "two_danglers_plus",
    "no_pair_no_suit",
];

/// Each tag [`hand_tags`] can give, and a plural description that completes
/// "you play too many ___".
pub const TAG_LABELS: [(&str, &str); 13] = [
    ("aa", "aces (AAxx)"),
    ("big_pair", "kings or queens (KKxx, QQxx)"),
    ("small_pair", "hands built around a small pair"),
    ("double_paired", "double-paired hands"),
    ("dead_cards", "hands with trips or quads (dead cards)"),
    ("double_suited", "double-suited hands"),
    ("rainbow", "rainbow hands with no flush draw"),
    ("suited_ace", "hands with a suited ace"),
    (
        "weak_suited",
        "weak suited hands (suited, but no ace or king in the suit)",
    ),
    ("rundown", "rundowns (four ranks in a row)"),
    ("broadway", "broadway-heavy hands"),
    ("low", "hands full of low cards"),
    ("danglers", "hands with a dangling card"),
];

/// 2 to 14 (ace high).
fn value(c: &Card) -> i32 {
    RankOrder::AceIsHigh.get_score(&c.rank()) as i32 + 2
}

fn suit_index(s: Suit) -> usize {
    match s {
        Suit::Spades => 0,
        Suit::Hearts => 1,
        Suit::Diamonds => 2,
        Suit::Clubs => 3,
    }
}

/// Rank and suit counts.
struct Counts {
    ranks: Vec<i32>,
    suits: Vec<usize>,
    rank_count: [u32; 15],
    suit_count: [u32; 4],
}

impl Counts {
    fn of(cards: &[Card]) -> Self {
        let ranks: Vec<i32> = cards.iter().map(value).collect();
        let suits: Vec<usize> = cards.iter().map(|c| suit_index(c.suit())).collect();
        let mut rank_count = [0u32; 15];
        let mut suit_count = [0u32; 4];
        for &r in &ranks {
            rank_count[r as usize] += 1;
        }
        for &s in &suits {
            suit_count[s] += 1;
        }
        Self {
            ranks,
            suits,
            rank_count,
            suit_count,
        }
    }

    /// Ranks with exactly two cards, highest first.
    fn pairs(&self) -> Vec<i32> {
        (2..=14)
            .rev()
            .filter(|&r| self.rank_count[r as usize] == 2)
            .collect()
    }

    /// Distinct ranks, highest first.
    fn distinct(&self) -> Vec<i32> {
        (2..=14)
            .rev()
            .filter(|&r| self.rank_count[r as usize] > 0)
            .collect()
    }

    /// Whether card `i` is within three ranks of another card (ace counting
    /// low against 2-4 too).
    fn near(&self, i: usize) -> bool {
        let r = self.ranks[i];
        self.ranks.iter().enumerate().any(|(j, &r2)| {
            j != i
                && r2 != r
                && ((r2 - r).abs() <= 3 || (r == 14 && r2 <= 4) || (r2 == 14 && r <= 4))
        })
    }

    /// The longest run of consecutive distinct ranks.
    fn best_run(distinct: &[i32]) -> u32 {
        let (mut run, mut best) = (1, 1);
        for w in distinct.windows(2) {
            run = if w[0] - w[1] == 1 { run + 1 } else { 1 };
            best = best.max(run);
        }
        best
    }
}

/// The hand's feature vector, in [`FEATURE_NAMES`] order.
pub fn hand_features(cards: &[Card]) -> Vec<f64> {
    let n = cards.len() as f64;
    let k = Counts::of(cards);
    let rc = |r: usize| k.rank_count[r];
    let pairs = k.pairs();
    let trips_or_more: u32 = k
        .rank_count
        .iter()
        .filter(|&&c| c >= 3)
        .map(|c| c - 2)
        .sum();

    // Suit structure: suits with exactly two cards are the strongest flush
    // draws; three or more of a suit cost outs.
    let present: Vec<u32> = k.suit_count.iter().copied().filter(|&c| c > 0).collect();
    let suits_with_2 = present.iter().filter(|&&c| c == 2).count() as f64;
    let suits_with_3 = present.iter().filter(|&&c| c >= 3).count() as f64;
    let rainbow = if present.len() == cards.len() {
        1.0
    } else {
        0.0
    };
    let (mut suited_ace, mut suited_king, mut suited_low_only) = (0.0, 0.0, 0.0);
    for s in 0..4 {
        if k.suit_count[s] < 2 {
            continue;
        }
        let rs: Vec<i32> = (0..cards.len())
            .filter(|&i| k.suits[i] == s)
            .map(|i| k.ranks[i])
            .collect();
        if rs.contains(&14) {
            suited_ace += 1.0;
        } else if rs.contains(&13) {
            suited_king += 1.0;
        } else if rs.iter().max().copied().unwrap_or(0) <= 9 {
            suited_low_only += 1.0;
        }
    }

    // Straight potential from distinct ranks (ace counts high and low).
    let mut mask = 0u32;
    for &r in &k.ranks {
        mask |= 1 << r;
        if r == 14 {
            mask |= 1 << 1;
        }
    }
    let (mut s2, mut s3, mut s4, mut max_in) = (0.0, 0.0, 0.0, 0u32);
    for low in 1..=10 {
        let straight: u32 = (low..low + 5).map(|r| 1u32 << r).sum();
        let hits = (mask & straight).count_ones();
        if hits >= 2 {
            s2 += 1.0;
        }
        if hits >= 3 {
            s3 += 1.0;
        }
        if hits >= 4 {
            s4 += 1.0;
        }
        max_in = max_in.max(hits);
    }
    let distinct = k.distinct();
    let (mut connectors, mut one_gaps, mut two_gaps) = (0.0, 0.0, 0.0);
    for i in 0..distinct.len() {
        for j in i + 1..distinct.len() {
            match distinct[i] - distinct[j] {
                1 => connectors += 1.0,
                2 => one_gaps += 1.0,
                3 => two_gaps += 1.0,
                _ => {}
            }
        }
    }
    // Ace-low connections (A with 2-4).
    if distinct.contains(&14) {
        for &r in &distinct {
            match r {
                2 => connectors += 1.0,
                3 => one_gaps += 1.0,
                4 => two_gaps += 1.0,
                _ => {}
            }
        }
    }
    let best_run = Counts::best_run(&distinct);

    // Danglers: cards that aren't paired, suited with another card, or within
    // three ranks of another card.
    let danglers = (0..cards.len())
        .filter(|&i| rc(k.ranks[i] as usize) <= 1 && k.suit_count[k.suits[i]] <= 1 && !k.near(i))
        .count() as f64;

    let aa = if rc(14) == 2 { 1.0 } else { 0.0 };
    let aa_suited = if rc(14) == 2
        && (0..cards.len()).any(|i| k.ranks[i] == 14 && k.suit_count[k.suits[i]] >= 2)
    {
        1.0
    } else {
        0.0
    };
    let pair_suited_with_side = if pairs
        .iter()
        .any(|&p| (0..cards.len()).any(|i| k.ranks[i] == p && k.suit_count[k.suits[i]] >= 2))
    {
        1.0
    } else {
        0.0
    };
    let broadways = k.ranks.iter().filter(|&&r| r >= 10).count() as f64;
    let kkqq = if rc(13) == 2 || rc(12) == 2 { 1.0 } else { 0.0 };

    // Side cards of the top pair: how many of them sit within three ranks of
    // each other (they make straights together instead of dangling).
    let side: Vec<i32> = match pairs.first() {
        Some(&top) => distinct.iter().copied().filter(|&r| r != top).collect(),
        None => Vec::new(),
    };
    let mut side_connected = 0.0f64;
    for i in 0..side.len() {
        for j in i + 1..side.len() {
            if (side[i] - side[j]).abs() <= 3 {
                side_connected += 1.0;
            }
        }
    }
    let double_suited = if suits_with_2 >= 2.0 { 1.0 } else { 0.0 };
    let b = |x: bool| if x { 1.0 } else { 0.0 };
    let pair_rank = |i: usize| pairs.get(i).map_or(0.0, |&p| (p - 2) as f64 / 12.0);

    vec![
        1.0,
        k.ranks.iter().fold(0.0, |a, &r| a + (r - 2) as f64 / 12.0) / n,
        rc(14) as f64,
        rc(13) as f64,
        rc(12) as f64,
        broadways,
        k.ranks.iter().filter(|&&r| r <= 5).count() as f64,
        pairs.len() as f64,
        pair_rank(0),
        pair_rank(1),
        b(rc(14) == 2),
        b(rc(13) == 2),
        b(rc(12) == 2),
        b(rc(11) == 2 || rc(10) == 2),
        trips_or_more as f64,
        suits_with_2,
        suits_with_3,
        rainbow,
        suited_ace,
        suited_king,
        suited_low_only,
        s2,
        s3,
        s4,
        max_in as f64,
        connectors,
        one_gaps,
        two_gaps,
        b(best_run >= 4),
        danglers,
        aa_suited,
        pair_suited_with_side,
        b(suits_with_2 >= 2.0 && max_in >= 3),
        b(distinct.iter().filter(|&&r| r >= 10).count() >= 4),
        aa * danglers,
        aa * rainbow,
        aa * double_suited,
        aa * side_connected.min(2.0),
        kkqq * double_suited,
        kkqq * danglers,
        if pairs.is_empty() {
            0.0
        } else {
            side_connected.min(3.0)
        },
        b(danglers >= 2.0),
        b(pairs.is_empty() && suits_with_2 == 0.0 && suits_with_3 == 0.0),
    ]
}

/// The recognisable kinds the hand is ([`TAG_LABELS`] ids), for spotting
/// patterns in mistakes. A hand can have several.
pub fn hand_tags(cards: &[Card]) -> Vec<&'static str> {
    let k = Counts::of(cards);
    let rc = |r: usize| k.rank_count[r];
    let pairs = k.pairs();
    let mut tags = Vec::new();

    if rc(14) == 2 {
        tags.push("aa");
    } else if rc(13) == 2 || rc(12) == 2 {
        tags.push("big_pair");
    } else if pairs.first().is_some_and(|&p| p <= 9) {
        tags.push("small_pair");
    }
    if pairs.len() >= 2 {
        tags.push("double_paired");
    }
    if k.rank_count.iter().any(|&c| c >= 3) {
        tags.push("dead_cards");
    }

    let suited: Vec<usize> = (0..4).filter(|&s| k.suit_count[s] >= 2).collect();
    if suited.len() >= 2 {
        tags.push("double_suited");
    }
    if suited.is_empty() {
        tags.push("rainbow");
    }
    let tops: Vec<i32> = suited
        .iter()
        .map(|&s| {
            (0..cards.len())
                .filter(|&i| k.suits[i] == s)
                .map(|i| k.ranks[i])
                .max()
                .unwrap()
        })
        .collect();
    if tops.contains(&14) {
        tags.push("suited_ace");
    } else if !suited.is_empty() && tops.iter().all(|&t| t <= 12) {
        tags.push("weak_suited");
    }

    if Counts::best_run(&k.distinct()) >= 4 {
        tags.push("rundown");
    }
    if k.ranks.iter().filter(|&&r| r >= 10).count() >= 3 {
        tags.push("broadway");
    }
    if k.ranks.iter().filter(|&&r| r <= 6).count() > cards.len().div_ceil(2) {
        tags.push("low");
    }
    if (0..cards.len())
        .any(|i| rc(k.ranks[i] as usize) == 1 && k.suit_count[k.suits[i]] == 1 && !k.near(i))
    {
        tags.push("danglers");
    }
    tags
}

/// `n` choose `k` for `k` up to 5.
fn choose(n: i64, k: i64) -> f64 {
    if !(0..=5).contains(&k) || n < k {
        return 0.0;
    }
    (0..k)
        .fold(1.0, |acc, i| acc * (n - i) as f64 / (i + 1) as f64)
        .round()
}

/// The chance, over every five-card board from the cards you don't hold,
/// that the board shows a possible flush (three or more of a suit) while you
/// hold that suit's nut card. Equity already accounts for card removal, but
/// not for knowing no one else can have the nut flush.
pub fn nut_flush_block(cards: &[Card]) -> f64 {
    let deck = 52 - cards.len() as i64;
    let boards = choose(deck, 5);
    let mut p = 0.0;
    // Three suited cards on a five-card board can only happen in one suit,
    // so the per-suit chances add up.
    for suit in [Suit::Spades, Suit::Hearts, Suit::Diamonds, Suit::Clubs] {
        let mine: Vec<i32> = cards
            .iter()
            .filter(|c| c.suit() == suit)
            .map(value)
            .collect();
        let Some(&top) = mine.iter().max() else {
            continue;
        };
        let above = 14 - top as i64; // ranks above your best card of the suit: all must be on the board
        let left = 13 - mine.len() as i64; // cards of the suit you don't hold
        let mut ways = 0.0;
        for j in above.max(3)..=5 {
            ways += choose(left - above, j - above) * choose(deck - left, 5 - j);
        }
        p += ways / boards;
    }
    p
}

fn model(cards: usize) -> Option<&'static OmahaRankModel> {
    match cards {
        4 => Some(&PLO4),
        5 => Some(&PLO5),
        6 => Some(&PLO6),
        _ => None,
    }
}

/// The hand's score under the fitted model for its size (higher is better),
/// or none for a hand that isn't 4, 5 or 6 cards.
pub fn score(cards: &[Card]) -> Option<f64> {
    let m = model(cards.len())?;
    let x = hand_features(cards);
    let mut s = 0.0;
    for (w, f) in m.weights.iter().zip(&x) {
        s += w * f;
    }
    // Holding the nut card of a suit that may flush.
    let (weight, mean, sd, score_sd) = m.blocker;
    if weight != 0.0 {
        s += weight * score_sd / sd * (nut_flush_block(cards) - mean);
    }
    Some(s)
}

/// The hand's percentile among all hands of its size: 0 is the best hand,
/// 100 the worst, so 12.5 is in the top 12.5%. None for a hand that isn't 4,
/// 5 or 6 cards.
pub fn percentile(cards: &[Card]) -> Option<f64> {
    let cut = model(cards.len())?.cutoffs; // descending scores
    let steps = cut.len() - 1;
    let s = score(cards)?;
    if s >= cut[0] {
        return Some(0.0);
    }
    if s <= cut[steps] {
        return Some(100.0);
    }
    let (mut lo, mut hi) = (0, steps); // cut[lo] >= s > cut[hi]
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if cut[mid] >= s {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let span = cut[lo] - cut[hi];
    let frac = if span > 0.0 {
        (cut[lo] - s) / span
    } else {
        0.0
    };
    Some((lo as f64 + frac) / steps as f64 * 100.0)
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::deck::Deck;

    fn cards(s: &str) -> Vec<Card> {
        s.split(',')
            .map(|c| Card::parse(c.trim()).unwrap())
            .collect()
    }

    /// Everything matches ducy.cards' JavaScript on a fixed set of hands
    /// (omaha_analysis_fixture.json, made from it).
    #[test]
    fn matches_the_page() {
        let f: serde_json::Value =
            serde_json::from_str(include_str!("omaha_analysis_fixture.json")).unwrap();
        let names: Vec<&str> = f["features"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(names, FEATURE_NAMES);
        let close = |a: f64, b: f64| (a - b).abs() <= 1e-9 * a.abs().max(1.0);
        for case in f["cases"].as_array().unwrap() {
            let text: Vec<&str> = case["cards"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            let hand = cards(&text.join(","));
            let want: Vec<f64> = case["features"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect();
            let got = hand_features(&hand);
            for (i, (g, w)) in got.iter().zip(&want).enumerate() {
                assert!(close(*g, *w), "{text:?} {}: {g} vs {w}", FEATURE_NAMES[i]);
            }
            let tags: Vec<&str> = case["tags"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            assert_eq!(hand_tags(&hand), tags, "{text:?}");
            assert!(
                close(nut_flush_block(&hand), case["block"].as_f64().unwrap()),
                "{text:?} block"
            );
            assert!(
                close(score(&hand).unwrap(), case["score"].as_f64().unwrap()),
                "{text:?} score"
            );
            assert!(
                close(
                    percentile(&hand).unwrap(),
                    case["percentile"].as_f64().unwrap()
                ),
                "{text:?} percentile"
            );
        }
    }

    #[test]
    fn ranks_and_tags_make_sense() {
        let aakq = cards("As, Ad, Kh, Qh");
        let trash = cards("7c, 2d, 9h, 3s");
        assert!(percentile(&aakq).unwrap() < percentile(&trash).unwrap());
        assert_eq!(percentile(&cards("As, Ad")), None, "only 4-6 cards");
        assert!(hand_tags(&trash).contains(&"rainbow"));
        assert!(
            nut_flush_block(
                &Deck::parse("As 2s 3d 4d")
                    .unwrap()
                    .iter(true)
                    .collect::<Vec<_>>()
            ) > 0.0
        );
    }
}
