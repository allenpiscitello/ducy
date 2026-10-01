use std::fmt::Display;

use crate::{
    deck::{Deck, Rank, RankSet},
    ranking::standard_hand_ranker::RankOrder,
};

/// Marker trait for types that represent a hand's ranking.
pub trait HandRanking {}

/// Standard poker hand rankings from high card through straight flush.
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum StandardHandRanks {
    /// Five unpaired, unconnected cards.
    HighCard {
        /// Highest kicker.
        c1: Rank,
        /// Second kicker.
        c2: Rank,
        /// Third kicker.
        c3: Rank,
        /// Fourth kicker.
        c4: Rank,
        /// Fifth kicker.
        c5: Rank,
    },
    /// Two cards of the same rank.
    OnePair {
        /// Pair rank.
        p: Rank,
        /// First kicker.
        c1: Rank,
        /// Second kicker.
        c2: Rank,
        /// Third kicker.
        c3: Rank,
    },
    /// Two distinct pairs.
    TwoPair {
        /// Higher pair rank.
        p1: Rank,
        /// Lower pair rank.
        p2: Rank,
        /// Kicker.
        c1: Rank,
    },
    /// Three cards of the same rank.
    ThreeOfAKind {
        /// Trips rank.
        t: Rank,
        /// First kicker.
        c1: Rank,
        /// Second kicker.
        c2: Rank,
    },
    /// Five consecutive ranks.
    Straight {
        /// High card of the straight.
        s: Rank,
    },
    /// Five cards of the same suit.
    Flush {
        /// Highest card.
        c1: Rank,
        /// Second card.
        c2: Rank,
        /// Third card.
        c3: Rank,
        /// Fourth card.
        c4: Rank,
        /// Fifth card.
        c5: Rank,
    },
    /// Three of a kind plus a pair.
    FullHouse {
        /// Trips rank.
        t: Rank,
        /// Pair rank.
        p: Rank,
    },
    /// Four cards of the same rank.
    FourOfAKind {
        /// Quads rank.
        q: Rank,
        /// Kicker.
        c: Rank,
    },
    /// Five consecutive cards of the same suit.
    StraightFlush {
        /// High card of the straight flush.
        sf: Rank,
    },
}

impl HandRanking for StandardHandRanks {}

impl Ord for StandardHandRanks {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.get_score().cmp(&other.get_score())
    }
}

impl PartialOrd for StandardHandRanks {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Display for StandardHandRanks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StandardHandRanks::HighCard { c1, c2, c3, c4, c5 } => {
                write!(f, "High Card {} {} {} {} {}", c1, c2, c3, c4, c5)
            }
            StandardHandRanks::OnePair { p, c1, c2, c3 } => {
                write!(f, "Pair of {p}, {c1} {c2} {c3}")
            }
            StandardHandRanks::TwoPair { p1, p2, c1 } => write!(f, "Two Pair {p1} over {p2}, {c1}"),
            StandardHandRanks::ThreeOfAKind { t, c1, c2 } => {
                write!(f, "Three of a Kind {t}, {c1} {c2}")
            }
            StandardHandRanks::Straight { s } => write!(f, "Straight {s} high"),
            StandardHandRanks::Flush { c1, c2, c3, c4, c5 } => {
                write!(f, "Flush {c1} {c2} {c3} {c4} {c5}")
            }
            StandardHandRanks::FullHouse { t, p } => write!(f, "Full House {t} full of {p}"),
            StandardHandRanks::FourOfAKind { q, c } => write!(f, "Four of a Kind {q}, {c}"),
            StandardHandRanks::StraightFlush { sf } => write!(f, "Straight Flush {sf} high"),
        }
    }
}

const FIVE_OPTIONS: u32 = 13 * 13 * 13 * 13 * 13;
const FOUR_OPTIONS: u32 = 13 * 13 * 13 * 13;
const THREE_OPTIONS: u32 = 13 * 13 * 13;
const TWO_OPTIONS: u32 = 13 * 13;
const ONE_OPTION: u32 = 13;

const ONE_PAIR_BASE: u32 = FIVE_OPTIONS;
const TWO_PAIR_BASE: u32 = ONE_PAIR_BASE + FOUR_OPTIONS;
const TRIP_BASE: u32 = TWO_PAIR_BASE + THREE_OPTIONS;
const STRAIGHT_BASE: u32 = TRIP_BASE + THREE_OPTIONS;
const FLUSH_BASE: u32 = STRAIGHT_BASE + ONE_OPTION;
const FULL_HOUSE_BASE: u32 = FLUSH_BASE + FIVE_OPTIONS;
const FOUR_OF_KIND_BASE: u32 = FULL_HOUSE_BASE + TWO_OPTIONS;
const STRAIGHT_FLUSH_BASE: u32 = FOUR_OF_KIND_BASE + TWO_OPTIONS;

impl StandardHandRanks {
    /// Returns a numeric score for comparison; higher is better.
    pub fn get_score(&self) -> u32 {
        match self {
            StandardHandRanks::HighCard { c1, c2, c3, c4, c5 } => {
                Self::get_score_from_ranks(&[c1, c2, c3, c4, c5])
            }
            StandardHandRanks::OnePair { p, c1, c2, c3 } => {
                Self::get_score_from_ranks(&[p, c1, c2, c3]) + ONE_PAIR_BASE
            }
            StandardHandRanks::TwoPair { p1, p2, c1 } => {
                Self::get_score_from_ranks(&[p1, p2, c1]) + TWO_PAIR_BASE
            }

            StandardHandRanks::ThreeOfAKind { t, c1, c2 } => {
                Self::get_score_from_ranks(&[t, c1, c2]) + TRIP_BASE
            }
            StandardHandRanks::Straight { s } => RankOrder::AceIsHigh.get_score(s) + STRAIGHT_BASE,
            StandardHandRanks::Flush { c1, c2, c3, c4, c5 } => {
                Self::get_score_from_ranks(&[c1, c2, c3, c4, c5]) + FLUSH_BASE
            }
            StandardHandRanks::FullHouse { t, p } => {
                Self::get_score_from_ranks(&[t, p]) + FULL_HOUSE_BASE
            }
            StandardHandRanks::FourOfAKind { q, c } => {
                Self::get_score_from_ranks(&[q, c]) + FOUR_OF_KIND_BASE
            }
            StandardHandRanks::StraightFlush { sf } => {
                RankOrder::AceIsHigh.get_score(sf) + STRAIGHT_FLUSH_BASE
            }
        }
    }

    fn get_score_from_ranks(values: &[&Rank]) -> u32 {
        let mut val = 0;
        for rank in values {
            val = val * 13 + RankOrder::AceIsHigh.get_score(rank)
        }
        val
    }
}

/// Evaluates a set of cards to determine the best standard poker hand.
pub struct StandardHandRanker {}

impl StandardHandRanker {
    /// Returns the best hand ranking from the given cards.
    pub fn get_rank(deck: &Deck) -> StandardHandRanks {
        Self::get_rank_at_least(deck, None).unwrap()
    }

    /// Returns the best hand ranking if it meets or exceeds the minimum, or `None`.
    pub fn get_rank_at_least(
        deck: &Deck,
        must_be_at_least: Option<StandardHandRanks>,
    ) -> Option<StandardHandRanks> {
        Self::get_rank_at_least_inner(deck, must_be_at_least, true, true)
    }

    /// Like `get_rank_at_least`, but with structural hints that skip impossible categories.
    ///
    /// - `flush_possible`: false skips straight-flush and flush checks.
    /// - `quads_fh_possible`: false skips four-of-a-kind and full-house checks.
    pub fn get_rank_at_least_with_hints(
        deck: &Deck,
        must_be_at_least: Option<StandardHandRanks>,
        flush_possible: bool,
        quads_fh_possible: bool,
    ) -> Option<StandardHandRanks> {
        Self::get_rank_at_least_inner(deck, must_be_at_least, flush_possible, quads_fh_possible)
    }

    fn get_rank_at_least_inner(
        deck: &Deck,
        must_be_at_least: Option<StandardHandRanks>,
        flush_possible: bool,
        quads_fh_possible: bool,
    ) -> Option<StandardHandRanks> {
        let rank_to_beat = must_be_at_least.map(|x| x.get_score()).unwrap_or(0);

        if flush_possible {
            if let Some(sf) = Self::get_best_straight_flush(deck) {
                return Some(StandardHandRanks::StraightFlush { sf });
            }
        }
        if rank_to_beat >= STRAIGHT_FLUSH_BASE {
            return None;
        }

        let rank_count = deck.get_rank_count();

        if quads_fh_possible {
            let best_quads = rank_count.find_highest_with_n(&[], 4);
            if let Some(quad) = best_quads
                && let Some(kicker) = rank_count.find_highest_with_n(&[quad], 1)
            {
                return Some(StandardHandRanks::FourOfAKind { q: quad, c: kicker });
            }
        }
        if rank_to_beat >= FOUR_OF_KIND_BASE {
            return None;
        }

        let best_trips = rank_count.find_highest_with_n(&[], 3);

        if quads_fh_possible
            && let Some(trip) = best_trips
            && let Some(pair) = rank_count.find_highest_with_n(&[trip], 2)
        {
            return Some(StandardHandRanks::FullHouse { t: trip, p: pair });
        }
        if rank_to_beat >= FULL_HOUSE_BASE {
            return None;
        }

        if flush_possible {
            if let Some(flush_ranks) = Self::get_flush(deck) {
                return Some(StandardHandRanks::Flush {
                    c1: flush_ranks[0],
                    c2: flush_ranks[1],
                    c3: flush_ranks[2],
                    c4: flush_ranks[3],
                    c5: flush_ranks[4],
                });
            }
        }
        if rank_to_beat >= FLUSH_BASE {
            return None;
        }
        if let Some(s) = Self::get_straight(deck) {
            return Some(StandardHandRanks::Straight { s });
        }
        if rank_to_beat >= STRAIGHT_BASE {
            return None;
        }
        if let Some(trip) = best_trips
            && let Some(c1) = rank_count.find_highest_with_n(&[trip], 1)
            && let Some(c2) = rank_count.find_highest_with_n(&[trip, c1], 1)
        {
            return Some(StandardHandRanks::ThreeOfAKind { t: trip, c1, c2 });
        }

        if rank_to_beat >= TRIP_BASE {
            return None;
        }
        if let Some(best_pair) = rank_count.find_highest_with_n(&[], 2) {
            if let Some(second_best_pair) = rank_count.find_highest_with_n(&[best_pair], 2)
                && let Some(c) = rank_count.find_highest_with_n(&[best_pair, second_best_pair], 1)
            {
                return Some(StandardHandRanks::TwoPair {
                    p1: best_pair,
                    p2: second_best_pair,
                    c1: c,
                });
            }

            if rank_to_beat >= TWO_PAIR_BASE {
                return None;
            }
            if let Some(c1) = rank_count.find_highest_with_n(&[best_pair], 1)
                && let Some(c2) = rank_count.find_highest_with_n(&[best_pair, c1], 1)
                && let Some(c3) = rank_count.find_highest_with_n(&[best_pair, c1, c2], 1)
            {
                return Some(StandardHandRanks::OnePair {
                    p: best_pair,
                    c1,
                    c2,
                    c3,
                });
            }
        }
        if rank_to_beat >= ONE_PAIR_BASE {
            return None;
        }
        if let Some(highest_cards) = deck
            .get_combined_ranks()
            .get_highest_five(&RankOrder::AceIsHigh)
        {
            return Some(StandardHandRanks::HighCard {
                c1: highest_cards[0],
                c2: highest_cards[1],
                c3: highest_cards[2],
                c4: highest_cards[3],
                c5: highest_cards[4],
            });
        }
        None
    }

    /// Fast scoring path for equity calculations: returns a u32 score directly,
    /// bypassing enum construction. Higher scores beat lower scores.
    pub fn fast_score_at_least(
        deck: &Deck,
        score_to_beat: u32,
        flush_possible: bool,
        quads_fh_possible: bool,
    ) -> Option<u32> {
        if flush_possible {
            if let Some(sf) = Self::get_best_straight_flush(deck) {
                return Some(rank_score(sf) + STRAIGHT_FLUSH_BASE);
            }
        }
        if score_to_beat >= STRAIGHT_FLUSH_BASE {
            return None;
        }

        let combined_ranks = deck.get_combined_ranks();
        let num_unique = combined_ranks.num_unique_ranks();

        // Each early exit compares against the base of the category above the
        // one being checked, so hands that tie the current best still return.
        if num_unique == 5 {
            if score_to_beat >= FULL_HOUSE_BASE {
                return None;
            }
            if flush_possible {
                if let Some(flush_ranks) = Self::get_flush(deck) {
                    return Some(
                        rank_score_5(
                            flush_ranks[0],
                            flush_ranks[1],
                            flush_ranks[2],
                            flush_ranks[3],
                            flush_ranks[4],
                        ) + FLUSH_BASE,
                    );
                }
            }
            if score_to_beat >= FLUSH_BASE {
                return None;
            }
            if let Some(s) = Self::get_straight_from_rank_bitfield(&combined_ranks) {
                return Some(rank_score(s) + STRAIGHT_BASE);
            }
            if score_to_beat >= ONE_PAIR_BASE {
                return None;
            }
            if let Some(hc) = combined_ranks.get_highest_five(&RankOrder::AceIsHigh) {
                return Some(rank_score_5(hc[0], hc[1], hc[2], hc[3], hc[4]));
            }
            return None;
        }

        if num_unique == 4 {
            if score_to_beat >= TWO_PAIR_BASE {
                return None;
            }
            let cards = u64::from(*deck);
            let s0 = cards & 0x3FFE;
            let s1 = (cards >> 16) & 0x3FFE;
            let s2 = (cards >> 32) & 0x3FFE;
            let s3 = (cards >> 48) & 0x3FFE;
            let paired = (s0 & s1) | (s0 & s2) | (s0 & s3) | (s1 & s2) | (s1 & s3) | (s2 & s3);
            let pair_bit = 63 - paired.leading_zeros();
            let combined = s0 | s1 | s2 | s3;
            let mut kickers = combined ^ (1u64 << pair_bit);
            let k1 = 63 - kickers.leading_zeros();
            kickers ^= 1u64 << k1;
            let k2 = 63 - kickers.leading_zeros();
            kickers ^= 1u64 << k2;
            let k3 = 63 - kickers.leading_zeros();
            let p = pair_bit - 1;
            return Some(((p * 13 + (k1 - 1)) * 13 + (k2 - 1)) * 13 + (k3 - 1) + ONE_PAIR_BASE);
        }

        let rank_count = deck.get_rank_count();

        if quads_fh_possible
            && let Some(quad) = rank_count.find_highest_with_n(&[], 4)
            && let Some(kicker) = rank_count.find_highest_with_n(&[quad], 1)
        {
            return Some(rank_score_2(quad, kicker) + FOUR_OF_KIND_BASE);
        }
        if score_to_beat >= FOUR_OF_KIND_BASE {
            return None;
        }

        let best_trips = rank_count.find_highest_with_n(&[], 3);

        if quads_fh_possible
            && let Some(trip) = best_trips
            && let Some(pair) = rank_count.find_highest_with_n(&[trip], 2)
        {
            return Some(rank_score_2(trip, pair) + FULL_HOUSE_BASE);
        }
        if score_to_beat >= FULL_HOUSE_BASE {
            return None;
        }

        if flush_possible {
            if let Some(flush_ranks) = Self::get_flush(deck) {
                return Some(
                    rank_score_5(
                        flush_ranks[0],
                        flush_ranks[1],
                        flush_ranks[2],
                        flush_ranks[3],
                        flush_ranks[4],
                    ) + FLUSH_BASE,
                );
            }
        }
        if score_to_beat >= FLUSH_BASE {
            return None;
        }
        if let Some(s) = Self::get_straight_from_rank_bitfield(&combined_ranks) {
            return Some(rank_score(s) + STRAIGHT_BASE);
        }
        if score_to_beat >= STRAIGHT_BASE {
            return None;
        }
        if let Some(trip) = best_trips
            && let Some(c1) = rank_count.find_highest_with_n(&[trip], 1)
            && let Some(c2) = rank_count.find_highest_with_n(&[trip, c1], 1)
        {
            return Some(rank_score_3(trip, c1, c2) + TRIP_BASE);
        }
        if score_to_beat >= TRIP_BASE {
            return None;
        }
        if let Some(best_pair) = rank_count.find_highest_with_n(&[], 2) {
            if let Some(second_best_pair) = rank_count.find_highest_with_n(&[best_pair], 2)
                && let Some(c) = rank_count.find_highest_with_n(&[best_pair, second_best_pair], 1)
            {
                return Some(rank_score_3(best_pair, second_best_pair, c) + TWO_PAIR_BASE);
            }
            if score_to_beat >= TWO_PAIR_BASE {
                return None;
            }
            if let Some(c1) = rank_count.find_highest_with_n(&[best_pair], 1)
                && let Some(c2) = rank_count.find_highest_with_n(&[best_pair, c1], 1)
                && let Some(c3) = rank_count.find_highest_with_n(&[best_pair, c1, c2], 1)
            {
                return Some(rank_score_4(best_pair, c1, c2, c3) + ONE_PAIR_BASE);
            }
        }
        if score_to_beat >= ONE_PAIR_BASE {
            return None;
        }
        if let Some(hc) = combined_ranks.get_highest_five(&RankOrder::AceIsHigh) {
            return Some(rank_score_5(hc[0], hc[1], hc[2], hc[3], hc[4]));
        }
        None
    }

    fn get_straight(deck: &Deck) -> Option<Rank> {
        let combined_ranks = deck.get_combined_ranks();
        Self::get_straight_from_rank_bitfield(&combined_ranks)
    }

    fn get_straight_from_rank_bitfield(rank_bitfield: &RankSet) -> Option<Rank> {
        rank_bitfield.matches_pattern(0b11111, 5)
    }

    fn get_flush(deck: &Deck) -> Option<[Rank; 5]> {
        let mut best: Option<[Rank; 5]> = None;
        for (bits, _) in deck.get_single_suit_ranks() {
            if bits.num_unique_ranks() >= 5 {
                match (best, bits.get_highest_five(&RankOrder::AceIsHigh)) {
                    (Some(existing), Some(newest)) => {
                        for i in 0..5 {
                            match RankOrder::AceIsHigh.cmp(newest[i], existing[i]) {
                                std::cmp::Ordering::Greater => {
                                    best = Some(newest);
                                    break;
                                }
                                std::cmp::Ordering::Less => break,
                                std::cmp::Ordering::Equal => {}
                            }
                        }
                    }
                    (None, Some(newest)) => best = Some(newest),
                    (_, None) => {}
                }
            }
        }
        best
    }

    fn get_best_straight_flush(deck: &Deck) -> Option<Rank> {
        let mut found: Option<Rank> = None;
        for (single_suit_rank, _) in deck.get_single_suit_ranks() {
            match (
                Self::get_straight_from_rank_bitfield(&single_suit_rank),
                found,
            ) {
                (Some(straight), Some(found_val)) => {
                    if RankOrder::AceIsHigh.cmp(straight, found_val) == std::cmp::Ordering::Greater
                    {
                        found = Some(straight)
                    }
                }
                (Some(straight), None) => found = Some(straight),
                (None, _) => {}
            }
        }
        found
    }
}

#[inline(always)]
fn rs(rank: Rank) -> u32 {
    rank as u32
}

#[inline(always)]
fn rank_score(r: Rank) -> u32 {
    rs(r)
}

#[inline(always)]
fn rank_score_2(r1: Rank, r2: Rank) -> u32 {
    rs(r1) * 13 + rs(r2)
}

#[inline(always)]
fn rank_score_3(r1: Rank, r2: Rank, r3: Rank) -> u32 {
    (rs(r1) * 13 + rs(r2)) * 13 + rs(r3)
}

#[inline(always)]
fn rank_score_4(r1: Rank, r2: Rank, r3: Rank, r4: Rank) -> u32 {
    ((rs(r1) * 13 + rs(r2)) * 13 + rs(r3)) * 13 + rs(r4)
}

#[inline(always)]
fn rank_score_5(r1: Rank, r2: Rank, r3: Rank, r4: Rank, r5: Rank) -> u32 {
    (((rs(r1) * 13 + rs(r2)) * 13 + rs(r3)) * 13 + rs(r4)) * 13 + rs(r5)
}

// Rank masks below use one bit per rank: bit 1 = Two ... bit 13 = Ace, so a
// rank's score (Two = 0 ... Ace = 12) is its bit index minus one.

#[inline(always)]
fn top_bit(bits: u64) -> u64 {
    1u64 << (63 - bits.leading_zeros())
}

#[inline(always)]
fn bit_score(bit: u64) -> u32 {
    62 - bit.leading_zeros()
}

/// Base-13 score of the highest `n` ranks in `bits`, highest first.
#[inline(always)]
fn top_ranks_score(mut bits: u64, n: u32) -> u32 {
    let mut score = 0;
    for _ in 0..n {
        let bit = top_bit(bits);
        score = score * 13 + bit_score(bit);
        bits ^= bit;
    }
    score
}

/// Score of the top card of the highest straight in `ranks`, counting the
/// ace as low too.
#[inline(always)]
fn straight_top(ranks: u64) -> Option<u32> {
    let r = ranks | ((ranks >> 13) & 1);
    let runs = r & (r >> 1) & (r >> 2) & (r >> 3) & (r >> 4);
    // Bit i of `runs` means ranks i..=i+4 are present; the top card is at
    // bit i + 4, whose score is i + 3. The wheel is i = 0 (ace at bit 0).
    (runs != 0).then(|| 63 - runs.leading_zeros() + 3)
}

impl StandardHandRanker {
    /// Scores the best 5-card hand from 5 to 7 cards. Returns the same value
    /// as `get_rank(deck).get_score()` without building a `StandardHandRanks`.
    pub fn score(deck: &Deck) -> u32 {
        debug_assert!((5..=7).contains(&deck.num_cards()));
        let c = u64::from(*deck);
        let (s0, s1, s2, s3) = (
            c & 0x3FFE,
            (c >> 16) & 0x3FFE,
            (c >> 32) & 0x3FFE,
            (c >> 48) & 0x3FFE,
        );
        let all = s0 | s1 | s2 | s3;

        let flush = [s0, s1, s2, s3].into_iter().find(|s| s.count_ones() >= 5);
        if let Some(top) = flush.and_then(straight_top) {
            return STRAIGHT_FLUSH_BASE + top;
        }

        let quads = s0 & s1 & s2 & s3;
        if quads != 0 {
            let q = top_bit(quads);
            return FOUR_OF_KIND_BASE + bit_score(q) * 13 + top_ranks_score(all ^ q, 1);
        }

        let two_plus = (s0 & s1) | (s0 & s2) | (s0 & s3) | (s1 & s2) | (s1 & s3) | (s2 & s3);
        let three_plus = (s0 & s1 & s2) | (s0 & s1 & s3) | (s0 & s2 & s3) | (s1 & s2 & s3);
        let trips = (three_plus != 0).then(|| top_bit(three_plus));
        if let Some(t) = trips {
            let others = two_plus ^ t;
            if others != 0 {
                return FULL_HOUSE_BASE + bit_score(t) * 13 + top_ranks_score(others, 1);
            }
        }

        if let Some(f) = flush {
            return FLUSH_BASE + top_ranks_score(f, 5);
        }
        if let Some(top) = straight_top(all) {
            return STRAIGHT_BASE + top;
        }
        if let Some(t) = trips {
            return TRIP_BASE + bit_score(t) * 169 + top_ranks_score(all ^ t, 2);
        }
        if two_plus != 0 {
            let p1 = top_bit(two_plus);
            let rest = two_plus ^ p1;
            if rest != 0 {
                let p2 = top_bit(rest);
                return TWO_PAIR_BASE
                    + (bit_score(p1) * 13 + bit_score(p2)) * 13
                    + top_ranks_score(all ^ p1 ^ p2, 1);
            }
            return ONE_PAIR_BASE + bit_score(p1) * 2197 + top_ranks_score(all ^ p1, 3);
        }
        top_ranks_score(all, 5)
    }
}

#[cfg(test)]
mod test {

    use crate::{
        deck::{Deck, Rank},
        ranking::hand_rank::{StandardHandRanker, StandardHandRanks},
    };

    #[test]
    pub fn test_score_matches_get_rank_for_5_6_7_cards() {
        for deck in Deck::all_cards().enumerate_combinations(5).step_by(5) {
            assert_eq!(
                StandardHandRanker::score(&deck),
                StandardHandRanker::get_rank(&deck).get_score(),
                "{deck:?}"
            );
        }
        let mut dealer = crate::games::CardDealer::new(Deck::all_cards());
        for n in [6, 7] {
            for _ in 0..300_000 {
                dealer.reset();
                let deck = dealer.deal(n);
                assert_eq!(
                    StandardHandRanker::score(&deck),
                    StandardHandRanker::get_rank(&deck).get_score(),
                    "{deck:?}"
                );
            }
        }
    }

    #[test]
    pub fn test_score_seven_card_edge_cases() {
        for (cards, expected) in [
            ("Ah 2c 3d 4s 5h Kd Kc", "Straight 5 high"),
            ("Ah Kh Qh Jh Th 9h 8h", "Straight Flush A high"),
            ("9s 9h 9d 9c Ks Kh 2c", "Four of a Kind 9, K"),
            ("7s 7h 7d 5c 5s 5h 2c", "Full House 7 full of 5"),
            ("Qs Qh Jd Jc 3s 3h Ac", "Two Pair Q over J, A"),
            ("2s 3s 4s 5s 9s 6s Kc", "Straight Flush 6 high"),
            ("2s 3s 4s 5s 9s 6d Kc", "Flush 9 5 4 3 2"),
        ] {
            let deck = Deck::parse(cards).unwrap();
            let rank = StandardHandRanker::get_rank(&deck);
            assert_eq!(
                StandardHandRanker::score(&deck),
                rank.get_score(),
                "{cards}"
            );
            assert!(
                rank.to_string()
                    .starts_with(expected.split(',').next().unwrap()),
                "{cards}: {rank}"
            );
        }
    }

    #[test]
    pub fn test_fast_score_matches_get_rank_and_keeps_ties() {
        for deck in Deck::all_cards().enumerate_combinations(5).step_by(7) {
            let score = StandardHandRanker::get_rank(&deck).get_score();
            let fast =
                |to_beat| StandardHandRanker::fast_score_at_least(&deck, to_beat, true, true);
            assert_eq!(fast(0), Some(score), "{deck:?}");
            assert_eq!(fast(score), Some(score), "tie dropped for {deck:?}");
            assert!(fast(score + 1).is_none_or(|s| s == score), "{deck:?}");
        }
    }

    #[test]
    pub fn test_rank() {
        let high_card_lowest = StandardHandRanks::HighCard {
            c1: Rank::Seven,
            c2: Rank::Five,
            c3: Rank::Four,
            c4: Rank::Three,
            c5: Rank::Two,
        };
        let high_card_highest = StandardHandRanks::HighCard {
            c1: Rank::Ace,
            c2: Rank::King,
            c3: Rank::Queen,
            c4: Rank::Jack,
            c5: Rank::Nine,
        };

        let one_pair_lowest = StandardHandRanks::OnePair {
            p: Rank::Two,
            c1: Rank::Five,
            c2: Rank::Four,
            c3: Rank::Three,
        };

        let one_pair_highest = StandardHandRanks::OnePair {
            p: Rank::Ace,
            c1: Rank::King,
            c2: Rank::Queen,
            c3: Rank::Jack,
        };

        let two_pair_lowest = StandardHandRanks::TwoPair {
            p1: Rank::Three,
            p2: Rank::Two,
            c1: Rank::Four,
        };

        let two_pair_highest = StandardHandRanks::TwoPair {
            p1: Rank::Ace,
            p2: Rank::King,
            c1: Rank::Queen,
        };

        let trip_lowest = StandardHandRanks::ThreeOfAKind {
            t: Rank::Two,
            c1: Rank::Four,
            c2: Rank::Three,
        };

        let trip_highest = StandardHandRanks::ThreeOfAKind {
            t: Rank::Ace,
            c1: Rank::King,
            c2: Rank::Queen,
        };

        let straight_lowest = StandardHandRanks::Straight { s: Rank::Five };

        let straight_highest = StandardHandRanks::Straight { s: Rank::Ace };

        let flush_lowest = StandardHandRanks::Flush {
            c1: Rank::Seven,
            c2: Rank::Six,
            c3: Rank::Five,
            c4: Rank::Four,
            c5: Rank::Three,
        };

        let flush_highest = StandardHandRanks::Flush {
            c1: Rank::Ace,
            c2: Rank::King,
            c3: Rank::Queen,
            c4: Rank::Jack,
            c5: Rank::Nine,
        };

        let full_house_lowest = StandardHandRanks::FullHouse {
            t: Rank::Two,
            p: Rank::Three,
        };

        let full_house_highest = StandardHandRanks::FullHouse {
            t: Rank::Ace,
            p: Rank::King,
        };

        let quads_lowest = StandardHandRanks::FourOfAKind {
            q: Rank::Two,
            c: Rank::Three,
        };

        let quads_highest = StandardHandRanks::FourOfAKind {
            q: Rank::Ace,
            c: Rank::King,
        };

        let sf_lowest = StandardHandRanks::StraightFlush { sf: Rank::Five };
        let sf_highest = StandardHandRanks::StraightFlush { sf: Rank::Ace };

        let mut all_hands = vec![
            one_pair_highest,
            one_pair_lowest,
            two_pair_highest,
            two_pair_lowest,
            high_card_highest,
            high_card_lowest,
            trip_lowest,
            trip_highest,
            straight_highest,
            straight_lowest,
            flush_highest,
            flush_lowest,
            full_house_highest,
            full_house_lowest,
            quads_highest,
            quads_lowest,
            sf_lowest,
            sf_highest,
        ];

        all_hands.sort();

        assert_eq!(
            all_hands,
            [
                high_card_lowest,
                high_card_highest,
                one_pair_lowest,
                one_pair_highest,
                two_pair_lowest,
                two_pair_highest,
                trip_lowest,
                trip_highest,
                straight_lowest,
                straight_highest,
                flush_lowest,
                flush_highest,
                full_house_lowest,
                full_house_highest,
                quads_lowest,
                quads_highest,
                sf_lowest,
                sf_highest
            ]
        )
    }
}
