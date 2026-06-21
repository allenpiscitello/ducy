use crate::deck::rank::Rank;

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum HandRank {
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

impl Ord for HandRank {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.get_score().cmp(&other.get_score())
    }
}

impl PartialOrd for HandRank {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
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

impl HandRank {
    fn get_score(&self) -> u32 {
        match self {
            HandRank::HighCard { c1, c2, c3, c4, c5 } => {
                Self::get_score_from_ranks(&[c1, c2, c3, c4, c5])
            }
            HandRank::OnePair { p, c1, c2, c3 } => {
                Self::get_score_from_ranks(&[p, c1, c2, c3]) + ONE_PAIR_BASE
            }
            HandRank::TwoPair { p1, p2, c1 } => {
                Self::get_score_from_ranks(&[p1, p2, c1]) + TWO_PAIR_BASE
            }

            HandRank::ThreeOfAKind { t, c1, c2 } => {
                Self::get_score_from_ranks(&[t, c1, c2]) + TRIP_BASE
            }
            HandRank::Straight { s } => s.get_score() + STRAIGHT_BASE,
            HandRank::Flush { c1, c2, c3, c4, c5 } => {
                Self::get_score_from_ranks(&[c1, c2, c3, c4, c5]) + FLUSH_BASE
            }
            HandRank::FullHouse { t, p } => Self::get_score_from_ranks(&[t, p]) + FULL_HOUSE_BASE,
            HandRank::FourOfAKind { q, c } => {
                Self::get_score_from_ranks(&[q, c]) + FOUR_OF_KIND_BASE
            }
            HandRank::StraightFlush { sf } => sf.get_score() + STRAIGHT_FLUSH_BASE,
        }
    }

    fn get_score_from_ranks(values: &[&Rank]) -> u32 {
        let mut val = 0;
        for rank in values {
            val = val * 13 + rank.get_score() - 1
        }
        val
    }
}

#[cfg(test)]
mod test {

    use crate::{deck::rank::Rank, ranks::hand_rank::HandRank};

    #[test]
    pub fn test_rank() {
        let high_card_lowest = HandRank::HighCard {
            c1: Rank::Seven,
            c2: Rank::Five,
            c3: Rank::Four,
            c4: Rank::Three,
            c5: Rank::Two,
        };
        let high_card_highest = HandRank::HighCard {
            c1: Rank::Ace,
            c2: Rank::King,
            c3: Rank::Queen,
            c4: Rank::Jack,
            c5: Rank::Nine,
        };

        let one_pair_lowest = HandRank::OnePair {
            p: Rank::Two,
            c1: Rank::Five,
            c2: Rank::Four,
            c3: Rank::Three,
        };

        let one_pair_highest = HandRank::OnePair {
            p: Rank::Ace,
            c1: Rank::King,
            c2: Rank::Queen,
            c3: Rank::Jack,
        };

        let two_pair_lowest = HandRank::TwoPair {
            p1: Rank::Three,
            p2: Rank::Two,
            c1: Rank::Four,
        };

        let two_pair_highest = HandRank::TwoPair {
            p1: Rank::Ace,
            p2: Rank::King,
            c1: Rank::Queen,
        };

        let trip_lowest = HandRank::ThreeOfAKind {
            t: Rank::Two,
            c1: Rank::Four,
            c2: Rank::Three,
        };

        let trip_highest = HandRank::ThreeOfAKind {
            t: Rank::Ace,
            c1: Rank::King,
            c2: Rank::Queen,
        };

        let straight_lowest = HandRank::Straight { s: Rank::Five };

        let straight_highest = HandRank::Straight { s: Rank::Ace };

        let flush_lowest = HandRank::Flush {
            c1: Rank::Seven,
            c2: Rank::Six,
            c3: Rank::Five,
            c4: Rank::Four,
            c5: Rank::Three,
        };

        let flush_highest = HandRank::Flush {
            c1: Rank::Ace,
            c2: Rank::King,
            c3: Rank::Queen,
            c4: Rank::Jack,
            c5: Rank::Nine,
        };

        let full_house_lowest = HandRank::FullHouse {
            t: Rank::Two,
            p: Rank::Three,
        };

        let full_house_highest = HandRank::FullHouse {
            t: Rank::Ace,
            p: Rank::King,
        };

        let quads_lowest = HandRank::FourOfAKind {
            q: Rank::Two,
            c: Rank::Three,
        };

        let quads_highest = HandRank::FourOfAKind {
            q: Rank::Ace,
            c: Rank::King,
        };

        let sf_lowest = HandRank::StraightFlush { sf: Rank::Five };
        let sf_highest = HandRank::StraightFlush { sf: Rank::Ace };

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
