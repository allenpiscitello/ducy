use crate::deck::Deck;
use crate::ranking::hand_rank::HandRanking;
use crate::ranking::standard_hand_ranker::RankOrder;

/// Ace-to-five low hand ranking for Razz.
///
/// Aces are low, straights and flushes do not count against you.
/// The best hand is A-2-3-4-5 (the wheel). Pairs count against you.
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub struct RazzRanks {
    score: (u32, u32, u32, u32, u32, u32),
}

impl HandRanking for RazzRanks {}

impl Ord for RazzRanks {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.score.cmp(&self.score)
    }
}

impl PartialOrd for RazzRanks {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Rank character for an ace-low score (A = 0, 2 = 1, ... K = 12).
pub(crate) fn ace_low_char(score: u32) -> char {
    b"A23456789TJQK"[score as usize] as char
}

impl std::fmt::Display for RazzRanks {
    /// e.g. "Low 7-5-4-3-A" or "One Pair K-K-4-3-A".
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (kind, c1, c2, c3, c4, c5) = self.score;
        let label = match kind {
            0 => "Low",
            1 => "One Pair",
            2 => "Two Pair",
            3 => "Three of a Kind",
            4 => "Full House",
            _ => "Four of a Kind",
        };
        let cards: Vec<String> = [c1, c2, c3, c4, c5]
            .iter()
            .map(|&s| ace_low_char(s).to_string())
            .collect();
        write!(f, "{label} {}", cards.join("-"))
    }
}

pub struct RazzRanker;

impl RazzRanker {
    /// Best ace-to-five low from any number of cards (5-card subsets are
    /// compared when more than five are given).
    pub fn get_rank(deck: &Deck) -> RazzRanks {
        if deck.num_cards() <= 5 {
            return Self::rank_five(deck);
        }
        deck.enumerate_combinations(5)
            .map(|combo| Self::rank_five(&combo))
            .max()
            .unwrap()
    }

    fn rank_five(deck: &Deck) -> RazzRanks {
        let ace_low = RankOrder::AceIsLow;
        let mut scores: Vec<u32> = deck
            .iter(false)
            .map(|c| ace_low.get_score(&c.rank()))
            .collect();
        scores.sort();

        let mut rank_freq = [0u32; 14];
        for card in deck.iter(false) {
            rank_freq[ace_low.get_score(&card.rank()) as usize] += 1;
        }

        let mut pair_count = 0u32;
        let mut trip_count = 0u32;
        let mut quad_count = 0u32;
        for &freq in &rank_freq {
            match freq {
                4 => quad_count += 1,
                3 => trip_count += 1,
                2 => pair_count += 1,
                _ => {}
            }
        }

        let hand_type = if quad_count > 0 {
            5
        } else if trip_count > 0 && pair_count > 0 {
            4
        } else if trip_count > 0 {
            3
        } else if pair_count >= 2 {
            2
        } else if pair_count == 1 {
            1
        } else {
            0
        };

        let s = |i: usize| scores.get(i).copied().unwrap_or(0);
        RazzRanks {
            score: (hand_type, s(4), s(3), s(2), s(1), s(0)),
        }
    }

    /// Same as [`Self::get_rank`].
    pub fn get_best_from(deck: &Deck) -> RazzRanks {
        Self::get_rank(deck)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test_util::deck_from_cards;

    #[test]
    fn test_wheel_is_best() {
        let wheel = deck_from_cards("Ac 2d 3h 4s 5c");
        let rank = RazzRanker::get_rank(&wheel);
        assert_eq!(rank.score.0, 0);
    }

    #[test]
    fn test_wheel_beats_six_low() {
        let wheel = deck_from_cards("Ac 2d 3h 4s 5c");
        let six_low = deck_from_cards("Ac 2d 3h 4s 6c");
        assert!(RazzRanker::get_rank(&wheel) > RazzRanker::get_rank(&six_low));
    }

    #[test]
    fn test_straight_doesnt_count() {
        let wheel = deck_from_cards("Ac 2d 3h 4s 5c");
        assert_eq!(RazzRanker::get_rank(&wheel).score.0, 0);
    }

    #[test]
    fn test_flush_doesnt_count() {
        let flush = deck_from_cards("Ac 2c 3c 4c 6c");
        let offsuit = deck_from_cards("Ac 2d 3h 4s 6c");
        assert_eq!(RazzRanker::get_rank(&flush), RazzRanker::get_rank(&offsuit));
    }

    #[test]
    fn test_pair_loses_to_no_pair() {
        let no_pair = deck_from_cards("Ac 2d 3h 4s Kc");
        let pair = deck_from_cards("Ac Ad 2h 3s 4c");
        assert!(RazzRanker::get_rank(&no_pair) > RazzRanker::get_rank(&pair));
    }

    #[test]
    fn test_two_pair_loses_to_one_pair() {
        let one_pair = deck_from_cards("Ac Ad 2h 3s 4c");
        let two_pair = deck_from_cards("Ac Ad 2h 2s 3c");
        assert!(RazzRanker::get_rank(&one_pair) > RazzRanker::get_rank(&two_pair));
    }

    #[test]
    fn test_best_from_seven() {
        let hand = deck_from_cards("Ac 2d 3h 4s 5c Kd Qh");
        let best = RazzRanker::get_best_from(&hand);
        let wheel = deck_from_cards("Ac 2d 3h 4s 5c");
        assert_eq!(best, RazzRanker::get_rank(&wheel));
    }

    #[test]
    fn test_get_rank_seven_cards_avoids_pairs() {
        let hand = deck_from_cards("Ac Ad 2h 3s 4c 5d Kh");
        let wheel = deck_from_cards("Ac 2h 3s 4c 5d");
        assert_eq!(RazzRanker::get_rank(&hand), RazzRanker::get_rank(&wheel));
        let seven_low = deck_from_cards("2c 3d 4h 5s 7c 8d 9h");
        assert!(RazzRanker::get_rank(&hand) > RazzRanker::get_rank(&seven_low));
    }

    #[test]
    fn test_display_shows_cards() {
        assert_eq!(
            RazzRanker::get_rank(&deck_from_cards("Ac 3d 4h 5s 7c Kd Kh")).to_string(),
            "Low 7-5-4-3-A"
        );
        assert_eq!(
            RazzRanker::get_rank(&deck_from_cards("Ac Ad 3h 4s Kc")).to_string(),
            "One Pair K-4-3-A-A"
        );
    }

    #[test]
    fn test_ace_is_low() {
        let ace_low = deck_from_cards("Ac 2d 3h 4s 6c");
        let no_ace = deck_from_cards("2c 3d 4h 5s 7c");
        assert!(RazzRanker::get_rank(&ace_low) > RazzRanker::get_rank(&no_ace));
    }
}
