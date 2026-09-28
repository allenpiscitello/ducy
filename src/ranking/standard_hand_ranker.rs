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
    pub fn test_ace_low_straight() {
        assert_rank!(
            "As 2s 3h 4c 5d",
            StandardHandRanks::Straight { s: Rank::Five }
        );
    }

    #[test]
    pub fn test_ace_high_straight_broadway() {
        assert_rank!(
            "Ts Js Qh Kc Ad",
            StandardHandRanks::Straight { s: Rank::Ace }
        );
    }

    #[test]
    pub fn test_ace_low_straight_flush() {
        assert_rank!(
            "As 2s 3s 4s 5s",
            StandardHandRanks::StraightFlush { sf: Rank::Five }
        );
    }

    #[test]
    pub fn test_royal_flush() {
        assert_rank!(
            "Ts Js Qs Ks As",
            StandardHandRanks::StraightFlush { sf: Rank::Ace }
        );
    }

    #[test]
    pub fn test_best_flush_from_six_suited() {
        assert_rank!(
            "2s 4s 6s 8s Ts Qs",
            StandardHandRanks::Flush {
                c1: Rank::Queen,
                c2: Rank::Ten,
                c3: Rank::Eight,
                c4: Rank::Six,
                c5: Rank::Four,
            }
        );
    }

    #[test]
    pub fn test_best_flush_from_seven_suited() {
        assert_rank!(
            "2s 3s 5s 7s 9s Js Ks",
            StandardHandRanks::Flush {
                c1: Rank::King,
                c2: Rank::Jack,
                c3: Rank::Nine,
                c4: Rank::Seven,
                c5: Rank::Five,
            }
        );
    }

    #[test]
    pub fn test_full_house_with_two_trips() {
        assert_rank!(
            "3c 3d 3h Kc Kd Kh 2s",
            StandardHandRanks::FullHouse {
                t: Rank::King,
                p: Rank::Three,
            }
        );
    }

    #[test]
    pub fn test_full_house_with_two_pairs_and_trips() {
        assert_rank!(
            "Ac Ad Ah 5c 5d 9c 9d",
            StandardHandRanks::FullHouse {
                t: Rank::Ace,
                p: Rank::Nine,
            }
        );
    }

    #[test]
    pub fn test_kicker_comparison_one_pair() {
        let hand_a = deck_from_cards("As Ah Kc Qd Jh");
        let hand_b = deck_from_cards("As Ah Kc Qd Th");
        let rank_a = StandardHandRanker::get_rank(&hand_a);
        let rank_b = StandardHandRanker::get_rank(&hand_b);
        assert!(rank_a > rank_b);
    }

    #[test]
    pub fn test_identical_hands_are_equal() {
        let hand_a = deck_from_cards("As Kc Qd Jh 9s");
        let hand_b = deck_from_cards("Ah Kd Qs Jc 9d");
        let rank_a = StandardHandRanker::get_rank(&hand_a);
        let rank_b = StandardHandRanker::get_rank(&hand_b);
        assert_eq!(rank_a, rank_b);
    }

    #[test]
    pub fn test_straight_beats_trips() {
        let trips = deck_from_cards("Ac Ad Ah 5c 3d");
        let straight = deck_from_cards("5c 6d 7h 8s 9c");
        let rank_trips = StandardHandRanker::get_rank(&trips);
        let rank_straight = StandardHandRanker::get_rank(&straight);
        assert!(rank_straight > rank_trips);
    }

    #[test]
    pub fn test_flush_beats_straight() {
        let straight = deck_from_cards("5c 6d 7h 8s 9c");
        let flush = deck_from_cards("2s 4s 6s 8s Ts");
        let rank_straight = StandardHandRanker::get_rank(&straight);
        let rank_flush = StandardHandRanker::get_rank(&flush);
        assert!(rank_flush > rank_straight);
    }

    #[test]
    pub fn test_seven_card_best_hand_selection() {
        assert_rank!(
            "2c 5d 8h Ts Qd Ac Ah",
            StandardHandRanks::OnePair {
                p: Rank::Ace,
                c1: Rank::Queen,
                c2: Rank::Ten,
                c3: Rank::Eight,
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
