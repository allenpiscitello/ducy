use crate::deck::rank::Rank;

#[derive(PartialEq, Eq, Debug)]
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
    },
    StraightFlush {
        sf: Rank,
    },
}
