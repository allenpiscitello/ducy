use crate::deck::Rank;

/// Controls whether Ace ranks high (above King) or low (below Two).
pub enum RankOrder {
    /// Ace ranks above King.
    AceIsHigh,
    /// Ace ranks below Two.
    AceIsLow,
}

impl RankOrder {
    /// Returns ranks from `low_rank` up to (but not including) `high_rank`, or all above `low_rank` if `None`.
    pub fn get_ranks_between(
        &self,
        low_rank: &Rank,
        high_rank: Option<&Rank>,
    ) -> impl Iterator<Item = Rank> {
        // high card must be higher than
        let mut cards = vec![
            Rank::Ace,
            Rank::Two,
            Rank::Three,
            Rank::Four,
            Rank::Five,
            Rank::Six,
            Rank::Seven,
            Rank::Eight,
            Rank::Nine,
            Rank::Ten,
            Rank::Jack,
            Rank::Queen,
            Rank::King,
            Rank::Ace,
        ];
        if let &RankOrder::AceIsLow = self {
            cards.truncate(13);
        } else {
            cards.remove(0);
        }

        let low_score = self.get_score(low_rank);
        cards.retain(|x| self.get_score(x) >= low_score);
        if let Some(high_rank) = high_rank {
            let high_score = self.get_score(high_rank);
            cards.retain(|x| self.get_score(x) < high_score);
        }
        cards.into_iter()
    }

    /// Returns a numeric score for the rank under this ordering.
    pub fn get_score(&self, rank: &Rank) -> u32 {
        let return_val = match rank {
            Rank::Two => 1,
            Rank::Three => 2,
            Rank::Four => 3,
            Rank::Five => 4,
            Rank::Six => 5,
            Rank::Seven => 6,
            Rank::Eight => 7,
            Rank::Nine => 8,
            Rank::Ten => 9,
            Rank::Jack => 10,
            Rank::Queen => 11,
            Rank::King => 12,
            Rank::Ace => match self {
                RankOrder::AceIsHigh => 13,
                RankOrder::AceIsLow => 0,
            },
        };
        match self {
            RankOrder::AceIsHigh => return_val - 1,
            RankOrder::AceIsLow => return_val,
        }
    }

    /// Compares two ranks under this ordering.
    pub fn cmp(&self, a: Rank, b: Rank) -> std::cmp::Ordering {
        self.get_score(&a).cmp(&self.get_score(&b))
    }
}

#[cfg(test)]
mod test {
    use crate::{
        deck::*,
        ranking::{
            hand_rank::{StandardHandRanker, StandardHandRanks},
            standard_hand_ranker::RankOrder,
        },
        test_util::deck_from_cards,
    };

    macro_rules! assert_rank {
        ($hand:expr, $rank:expr) => {
            let hand = deck_from_cards($hand);
            assert_eq!(StandardHandRanker::get_rank(&hand), $rank);
        };
    }

    #[test]
    pub fn test_ranker() {
        assert_rank!(
            "3c 4c 5c 3d 4d",
            StandardHandRanks::TwoPair {
                p1: Rank::Four,
                p2: Rank::Three,
                c1: Rank::Five
            }
        );
        assert_rank!(
            "As 2s 3h 4c 6d",
            StandardHandRanks::HighCard {
                c1: Rank::Ace,
                c2: Rank::Six,
                c3: Rank::Four,
                c4: Rank::Three,
                c5: Rank::Two,
            }
        );
        assert_rank!(
            "As 2s 3s 4s 6s",
            StandardHandRanks::Flush {
                c1: Rank::Ace,
                c2: Rank::Six,
                c3: Rank::Four,
                c4: Rank::Three,
                c5: Rank::Two,
            }
        );
        assert_rank!(
            "As 2s 3h 4c 5d",
            StandardHandRanks::Straight { s: Rank::Five }
        );
        assert_rank!(
            "As 2s 3h 4c 5d 6d",
            StandardHandRanks::Straight { s: Rank::Six }
        );
        assert_rank!(
            "6s 2s 3s 4s 5s",
            StandardHandRanks::StraightFlush { sf: Rank::Six }
        );
        assert_rank!(
            "6d 6c 6h 6s 5s",
            StandardHandRanks::FourOfAKind {
                q: Rank::Six,
                c: Rank::Five
            }
        );
        assert_rank!(
            "6d 6c 6h 5h 5s",
            StandardHandRanks::FullHouse {
                t: Rank::Six,
                p: Rank::Five
            }
        );
        assert_rank!(
            "6d 6c 6h 4h 5s",
            StandardHandRanks::ThreeOfAKind {
                t: Rank::Six,
                c1: Rank::Five,
                c2: Rank::Four,
            }
        );
        assert_rank!(
            "6s 2s 2h 4s 5s",
            StandardHandRanks::OnePair {
                p: Rank::Two,
                c1: Rank::Six,
                c2: Rank::Five,
                c3: Rank::Four
            }
        );
    }

    #[test]
    pub fn test_rank_between() {
        let ranks_between: Vec<Rank> = RankOrder::AceIsHigh
            .get_ranks_between(&Rank::Three, Some(&Rank::Seven))
            .collect();

        assert_eq!(
            ranks_between,
            [Rank::Three, Rank::Four, Rank::Five, Rank::Six]
        );
        let ranks_between: Vec<Rank> = RankOrder::AceIsHigh
            .get_ranks_between(&Rank::Queen, None)
            .collect();

        assert_eq!(ranks_between, [Rank::Queen, Rank::King, Rank::Ace]);
    }
}
