use crate::deck::{Deck, Rank};
use crate::ranking::hand_rank::{HandRanking, StandardHandRanker, StandardHandRanks};

/// A deuce-to-seven lowball hand's rank: aces are high, and straights and
/// flushes count against you; the best hand is 7-5-4-3-2 offsuit. Greater is
/// better.
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub struct DeuceToSevenRanks {
    pub(crate) high_rank: StandardHandRanks,
}

impl HandRanking for DeuceToSevenRanks {}

impl Ord for DeuceToSevenRanks {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.high_rank.cmp(&self.high_rank)
    }
}

impl PartialOrd for DeuceToSevenRanks {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl std::fmt::Display for DeuceToSevenRanks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "2-7 Low: {}", self.high_rank)
    }
}

/// Ranks deuce-to-seven lowball hands.
pub struct DeuceToSevenRanker;

impl DeuceToSevenRanker {
    /// The rank of exactly 5 cards.
    pub fn get_rank(deck: &Deck) -> DeuceToSevenRanks {
        let standard = StandardHandRanker::get_rank(deck);
        let high_rank = match standard {
            // A-2-3-4-5 ace-low straight: not a straight in 2-7 (ace is always high)
            StandardHandRanks::Straight { s: Rank::Five } => {
                if Self::is_flush(deck) {
                    StandardHandRanks::Flush {
                        c1: Rank::Ace,
                        c2: Rank::Five,
                        c3: Rank::Four,
                        c4: Rank::Three,
                        c5: Rank::Two,
                    }
                } else {
                    StandardHandRanks::HighCard {
                        c1: Rank::Ace,
                        c2: Rank::Five,
                        c3: Rank::Four,
                        c4: Rank::Three,
                        c5: Rank::Two,
                    }
                }
            }
            // A-2-3-4-5 suited: not a straight flush in 2-7, just a flush
            StandardHandRanks::StraightFlush { sf: Rank::Five } => StandardHandRanks::Flush {
                c1: Rank::Ace,
                c2: Rank::Five,
                c3: Rank::Four,
                c4: Rank::Three,
                c5: Rank::Two,
            },
            other => other,
        };
        DeuceToSevenRanks { high_rank }
    }

    /// The best 5-card hand from `deck` (5 or more cards).
    pub fn get_best_from(deck: &Deck) -> DeuceToSevenRanks {
        if deck.num_cards() <= 5 {
            return Self::get_rank(deck);
        }
        let mut best: Option<DeuceToSevenRanks> = None;
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

    fn is_flush(deck: &Deck) -> bool {
        for (suit_ranks, _) in deck.get_single_suit_ranks() {
            if suit_ranks.num_unique_ranks() >= 5 {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test_util::deck_from_cards;

    #[test]
    fn test_best_27_hand() {
        let nut = deck_from_cards("2c 3d 4h 5s 7c");
        let rank = DeuceToSevenRanker::get_rank(&nut);
        assert_eq!(
            rank.high_rank,
            StandardHandRanks::HighCard {
                c1: Rank::Seven,
                c2: Rank::Five,
                c3: Rank::Four,
                c4: Rank::Three,
                c5: Rank::Two,
            }
        );
    }

    #[test]
    fn test_nut_beats_eight_low() {
        let nut = deck_from_cards("2c 3d 4h 5s 7c");
        let eight = deck_from_cards("2c 3d 4h 5s 8c");
        assert!(DeuceToSevenRanker::get_rank(&nut) > DeuceToSevenRanker::get_rank(&eight));
    }

    #[test]
    fn test_pair_is_bad() {
        let no_pair = deck_from_cards("2c 3d 4h 5s 8c");
        let pair = deck_from_cards("2c 2d 4h 5s 7c");
        assert!(DeuceToSevenRanker::get_rank(&no_pair) > DeuceToSevenRanker::get_rank(&pair));
    }

    #[test]
    fn test_straight_counts_against() {
        let straight = deck_from_cards("2c 3d 4h 5s 6c");
        let seven_high = deck_from_cards("2c 3d 4h 5s 7c");
        assert!(
            DeuceToSevenRanker::get_rank(&seven_high) > DeuceToSevenRanker::get_rank(&straight)
        );
    }

    #[test]
    fn test_flush_counts_against() {
        let flush = deck_from_cards("2c 3c 4c 5c 7c");
        let offsuit = deck_from_cards("2c 3d 4h 5s 7c");
        assert!(DeuceToSevenRanker::get_rank(&offsuit) > DeuceToSevenRanker::get_rank(&flush));
    }

    #[test]
    fn test_ace_is_high_not_straight() {
        // A-2-3-4-5 offsuit: NOT a straight in 2-7, just ace-high
        let hand = deck_from_cards("Ac 2d 3h 4s 5c");
        let rank = DeuceToSevenRanker::get_rank(&hand);
        assert_eq!(
            rank.high_rank,
            StandardHandRanks::HighCard {
                c1: Rank::Ace,
                c2: Rank::Five,
                c3: Rank::Four,
                c4: Rank::Three,
                c5: Rank::Two,
            }
        );
    }

    #[test]
    fn test_ace_low_suited_is_flush_not_straight_flush() {
        // A-2-3-4-5 all clubs: flush in 2-7, not a straight flush
        let hand = deck_from_cards("Ac 2c 3c 4c 5c");
        let rank = DeuceToSevenRanker::get_rank(&hand);
        assert_eq!(
            rank.high_rank,
            StandardHandRanks::Flush {
                c1: Rank::Ace,
                c2: Rank::Five,
                c3: Rank::Four,
                c4: Rank::Three,
                c5: Rank::Two,
            }
        );
    }

    #[test]
    fn test_ace_high_is_worse_than_king_high() {
        // Ace-high is worse than king-high in 2-7 (ace is the highest card)
        let ace_high = deck_from_cards("Ac 2d 3h 4s 7c");
        let king_high = deck_from_cards("Kc 2d 3h 4s 7c");
        assert!(DeuceToSevenRanker::get_rank(&king_high) > DeuceToSevenRanker::get_rank(&ace_high));
    }

    #[test]
    fn test_real_straight_still_counts() {
        // 6-high straight still counts against you
        let straight = deck_from_cards("2c 3d 4h 5s 6c");
        assert!(matches!(
            DeuceToSevenRanker::get_rank(&straight).high_rank,
            StandardHandRanks::Straight { s: Rank::Six }
        ));
    }

    #[test]
    fn test_best_from_seven() {
        let hand = deck_from_cards("2c 3d 4h 5s 7c 8d 9h");
        let best = DeuceToSevenRanker::get_best_from(&hand);
        assert_eq!(
            best.high_rank,
            StandardHandRanks::HighCard {
                c1: Rank::Seven,
                c2: Rank::Five,
                c3: Rank::Four,
                c4: Rank::Three,
                c5: Rank::Two,
            }
        );
    }
}
