use std::fmt::Display;

use crate::{deck::Rank, ranking::standard_hand_ranker::RankOrder};


pub trait HandRanking {}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum StandardHandRanks {
    HighCard {
        c1: Rank,
        c2: Rank,
        c3: Rank,
        c4: Rank,
        c5: Rank,
    },
    OnePair {
        p: Rank,
        c1: Rank,
        c2: Rank,
        c3: Rank,
    },
    TwoPair {
        p1: Rank,
        p2: Rank,
        c1: Rank,
    },
    ThreeOfAKind {
        t: Rank,
        c1: Rank,
        c2: Rank,
    },
    Straight {
        s: Rank,
    },
    Flush {
        c1: Rank,
        c2: Rank,
        c3: Rank,
        c4: Rank,
        c5: Rank,
    },
    FullHouse {
        t: Rank,
        p: Rank,
    },
    FourOfAKind {
        q: Rank,
        c: Rank,
    },
    StraightFlush {
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
            StandardHandRanks::OnePair { p, c1, c2, c3 } => write!(f, "Pair of {p}, {c1} {c2} {c3}"),
            StandardHandRanks::TwoPair { p1, p2, c1 } => write!(f, "Two Pair {p1} over {p2}, {c1}"),
            StandardHandRanks::ThreeOfAKind { t, c1, c2 } => write!(f, "Three of a Kind {t}, {c1} {c2}"),
            StandardHandRanks::Straight { s } => write!(f, "Straight {s} high"),
            StandardHandRanks::Flush { c1, c2, c3, c4, c5 } => write!(f, "Flush {c1} {c2} {c3} {c4} {c5}"),
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
    fn get_score(&self) -> u32 {
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
            StandardHandRanks::FullHouse { t, p } => Self::get_score_from_ranks(&[t, p]) + FULL_HOUSE_BASE,
            StandardHandRanks::FourOfAKind { q, c } => {
                Self::get_score_from_ranks(&[q, c]) + FOUR_OF_KIND_BASE
            }
            StandardHandRanks::StraightFlush { sf } => RankOrder::AceIsHigh.get_score(&sf) + STRAIGHT_FLUSH_BASE,
        }
    }

    fn get_score_from_ranks(values: &[&Rank]) -> u32 {
        let mut val = 0;
        for rank in values {
            val = val * 13 + RankOrder::AceIsHigh.get_score(rank) - 1
        }
        val
    }
}

#[cfg(test)]
mod test {

    use crate::{deck::Rank, ranking::hand_rank::StandardHandRanks};

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
            one_pair_highest.clone(),
            one_pair_lowest.clone(),
            two_pair_highest.clone(),
            two_pair_lowest.clone(),
            high_card_highest.clone(),
            high_card_lowest.clone(),
            trip_lowest.clone(),
            trip_highest.clone(),
            straight_highest.clone(),
            straight_lowest.clone(),
            flush_highest.clone(),
            flush_lowest.clone(),
            full_house_highest.clone(),
            full_house_lowest.clone(),
            quads_highest.clone(),
            quads_lowest.clone(),
            sf_lowest.clone(),
            sf_highest.clone(),
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
