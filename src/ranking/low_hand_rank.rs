use crate::deck::{Deck, Rank};
use crate::ranking::hand_rank::HandRanking;
use crate::ranking::standard_hand_ranker::RankOrder;

/// Ace-to-five lowball hand rankings for 8-or-better qualification.
///
/// A hand qualifies only if it contains five unpaired cards all ranked
/// 8 or lower (with ace counting as low). Straights and flushes are
/// ignored. The best possible low is A-2-3-4-5 (the "wheel").
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum LowHandRanks {
    /// No qualifying low hand (fewer than 5 unpaired cards <= 8).
    NoLow,
    /// A qualifying low, ranked from highest to lowest card.
    Low {
        c1: Rank,
        c2: Rank,
        c3: Rank,
        c4: Rank,
        c5: Rank,
    },
}

impl HandRanking for LowHandRanks {}

impl Ord for LowHandRanks {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.get_score().cmp(&other.get_score())
    }
}

impl PartialOrd for LowHandRanks {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl LowHandRanks {
    fn get_score(&self) -> (i32, i32, i32, i32, i32) {
        let order = RankOrder::AceIsLow;
        match self {
            LowHandRanks::NoLow => (i32::MIN, 0, 0, 0, 0),
            LowHandRanks::Low { c1, c2, c3, c4, c5 } => {
                // Lower cards = better low. Negate so Ord gives us better = greater.
                (
                    -(order.get_score(c1) as i32),
                    -(order.get_score(c2) as i32),
                    -(order.get_score(c3) as i32),
                    -(order.get_score(c4) as i32),
                    -(order.get_score(c5) as i32),
                )
            }
        }
    }
}

impl std::fmt::Display for LowHandRanks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LowHandRanks::NoLow => write!(f, "No Low"),
            LowHandRanks::Low { c1, c2, c3, c4, c5 } => {
                write!(f, "Low: {}-{}-{}-{}-{}", c1, c2, c3, c4, c5)
            }
        }
    }
}

pub struct LowHandRanker;

impl LowHandRanker {
    /// Evaluates the best qualifying low hand from a 5-card combination.
    /// Returns `LowHandRanks::Low` if all 5 cards are 8-or-lower with no
    /// pairs, or `LowHandRanks::NoLow` otherwise.
    pub fn get_rank(deck: &Deck) -> LowHandRanks {
        let combined = deck.get_combined_ranks();
        if combined.num_unique_ranks() < 5 {
            return LowHandRanks::NoLow;
        }

        let order = RankOrder::AceIsLow;
        let mut ranks: Vec<(u32, Rank)> = Vec::new();

        for card in deck.iter(false) {
            let rank = card.rank();
            let score = order.get_score(&rank);
            if score > 7 {
                return LowHandRanks::NoLow;
            }
            if !ranks.iter().any(|(_, r)| *r == rank) {
                ranks.push((score, rank));
            }
        }

        if ranks.len() < 5 {
            return LowHandRanks::NoLow;
        }

        ranks.sort_by_key(|a| std::cmp::Reverse(a.0));

        LowHandRanks::Low {
            c1: ranks[0].1,
            c2: ranks[1].1,
            c3: ranks[2].1,
            c4: ranks[3].1,
            c5: ranks[4].1,
        }
    }

    /// Evaluates the best qualifying low from a 5-card hand,
    /// returning `None` if it doesn't beat `min_hand`.
    pub fn get_rank_at_least(deck: &Deck, min_hand: Option<LowHandRanks>) -> Option<LowHandRanks> {
        let rank = Self::get_rank(deck);
        match rank {
            LowHandRanks::NoLow => None,
            _ => {
                if let Some(min) = min_hand {
                    if rank >= min { Some(rank) } else { None }
                } else {
                    Some(rank)
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test_util::deck_from_cards;

    #[test]
    fn test_wheel_is_best_low() {
        let hand = deck_from_cards("As 2h 3d 4c 5s");
        let rank = LowHandRanker::get_rank(&hand);
        assert_eq!(
            rank,
            LowHandRanks::Low {
                c1: Rank::Five,
                c2: Rank::Four,
                c3: Rank::Three,
                c4: Rank::Two,
                c5: Rank::Ace,
            }
        );
    }

    #[test]
    fn test_eight_high_low() {
        let hand = deck_from_cards("As 2h 3d 4c 8s");
        let rank = LowHandRanker::get_rank(&hand);
        assert_eq!(
            rank,
            LowHandRanks::Low {
                c1: Rank::Eight,
                c2: Rank::Four,
                c3: Rank::Three,
                c4: Rank::Two,
                c5: Rank::Ace,
            }
        );
    }

    #[test]
    fn test_no_low_nine_high() {
        let hand = deck_from_cards("As 2h 3d 4c 9s");
        assert_eq!(LowHandRanker::get_rank(&hand), LowHandRanks::NoLow);
    }

    #[test]
    fn test_no_low_pair() {
        let hand = deck_from_cards("As Ah 3d 4c 5s");
        assert_eq!(LowHandRanker::get_rank(&hand), LowHandRanks::NoLow);
    }

    #[test]
    fn test_wheel_beats_six_low() {
        let wheel = deck_from_cards("As 2h 3d 4c 5s");
        let six_low = deck_from_cards("As 2h 3d 4c 6s");
        let wheel_rank = LowHandRanker::get_rank(&wheel);
        let six_rank = LowHandRanker::get_rank(&six_low);
        assert!(wheel_rank > six_rank);
    }

    #[test]
    fn test_six_four_beats_six_five() {
        let six_four = deck_from_cards("As 2h 3d 4c 6s");
        let six_five = deck_from_cards("As 2h 3d 5c 6s");
        let rank_64 = LowHandRanker::get_rank(&six_four);
        let rank_65 = LowHandRanker::get_rank(&six_five);
        assert!(rank_64 > rank_65);
    }

    #[test]
    fn test_no_low_is_worst() {
        let low = deck_from_cards("As 2h 3d 7c 8s");
        let no_low = LowHandRanks::NoLow;
        let low_rank = LowHandRanker::get_rank(&low);
        assert!(low_rank > no_low);
    }

    #[test]
    fn test_straight_doesnt_disqualify() {
        let hand = deck_from_cards("4s 5h 6d 7c 8s");
        let rank = LowHandRanker::get_rank(&hand);
        assert!(rank != LowHandRanks::NoLow);
    }

    #[test]
    fn test_flush_doesnt_disqualify() {
        let hand = deck_from_cards("As 2s 3s 4s 8s");
        let rank = LowHandRanker::get_rank(&hand);
        assert!(rank != LowHandRanks::NoLow);
    }
}
