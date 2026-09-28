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

impl std::fmt::Display for RazzRanks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Razz Low")
    }
}

pub struct RazzRanker;

impl RazzRanker {
    pub fn get_rank(deck: &Deck) -> RazzRanks {
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

    pub fn get_best_from(deck: &Deck) -> RazzRanks {
        if deck.num_cards() <= 5 {
            return Self::get_rank(deck);
        }
        let mut best: Option<RazzRanks> = None;
        for combo in deck.enumerate_combinations(5) {
            let rank = Self::get_rank(&combo);
            match &best {
                Some(b) if rank > *b => best = Some(rank),
                None => best = Some(rank),
                _ => {}
            }
        }
        best.unwrap()
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
    fn test_ace_is_low() {
        let ace_low = deck_from_cards("Ac 2d 3h 4s 6c");
        let no_ace = deck_from_cards("2c 3d 4h 5s 7c");
        assert!(RazzRanker::get_rank(&ace_low) > RazzRanker::get_rank(&no_ace));
    }
}
