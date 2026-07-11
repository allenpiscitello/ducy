use crate::{
    deck::{Deck, Rank},
    ranking::hand_rank::StandardHandRanks,
};

pub enum RankOrder {
    AceIsHigh,
    AceIsLow
}

impl RankOrder {

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
            RankOrder::AceIsHigh => return_val,
            RankOrder::AceIsLow => return_val + 1,
        }
    }

    pub fn cmp(&self, a: Rank, b: Rank) -> std::cmp::Ordering {
        self.get_score(&a).cmp(&self.get_score(&b))
    }
}

pub struct StandardHandRanker {}

impl StandardHandRanker {
    pub fn get_rank(deck: &Deck) -> StandardHandRanks {
        if let Some(sf) = Self::get_best_straight_flush(deck) {
            StandardHandRanks::StraightFlush { sf }
        } else {
            let rank_count = deck.get_rank_count();
            let best_quads = rank_count.find_highest_with_n(&vec![], 4);

            if let Some(quad) = best_quads {
                if let Some(kicker) = rank_count.find_highest_with_n(&vec![quad], 1) {
                    return StandardHandRanks::FourOfAKind { q: quad, c: kicker };
                }
            }
            let best_trips = rank_count.find_highest_with_n(&vec![], 3);

            if let Some(trip) = best_trips {
                if let Some(pair) = rank_count.find_highest_with_n(&vec![trip], 2) {
                    return StandardHandRanks::FullHouse { t: trip, p: pair };
                }
            }
            if let Some(flush_ranks) = Self::get_flush(deck) {
                return StandardHandRanks::Flush {
                    c1: flush_ranks[0],
                    c2: flush_ranks[1],
                    c3: flush_ranks[2],
                    c4: flush_ranks[3],
                    c5: flush_ranks[4],
                };
            }
            if let Some(s) = Self::get_straight(deck) {
                return StandardHandRanks::Straight { s };
            }
            if let Some(trip) = best_trips {
                if let Some(c1) = rank_count.find_highest_with_n(&vec![trip], 1) {
                    if let Some(c2) = rank_count.find_highest_with_n(&vec![trip, c1], 1) {
                        return StandardHandRanks::ThreeOfAKind { t: trip, c1, c2 };
                    }
                }
            }
            if let Some(best_pair) = rank_count.find_highest_with_n(&vec![], 2) {
                if let Some(second_best_pair) = rank_count.find_highest_with_n(&vec![best_pair], 2)
                {
                    if let Some(c) =
                        rank_count.find_highest_with_n(&vec![best_pair, second_best_pair], 1)
                    {
                        return StandardHandRanks::TwoPair {
                            p1: best_pair,
                            p2: second_best_pair,
                            c1: c,
                        };
                    }
                } else if let Some(c1) = rank_count.find_highest_with_n(&vec![best_pair], 1) {
                    if let Some(c2) = rank_count.find_highest_with_n(&vec![best_pair, c1], 1) {
                        if let Some(c3) =
                            rank_count.find_highest_with_n(&vec![best_pair, c1, c2], 1)
                        {
                            return StandardHandRanks::OnePair {
                                p: best_pair,
                                c1,
                                c2,
                                c3,
                            };
                        }
                    }
                }
            }

            if let Some(highest_cards) = deck.get_combined_rank_bitfield().get_highest_five() {
                return StandardHandRanks::HighCard {
                    c1: highest_cards[0],
                    c2: highest_cards[1],
                    c3: highest_cards[2],
                    c4: highest_cards[3],
                    c5: highest_cards[4],
                };
            }
            panic!();
        }
    }

    fn get_straight(deck: &Deck) -> Option<Rank> {
        deck.get_combined_rank_bitfield().get_straight()  
     }


    fn get_flush(deck: &Deck) -> Option<[Rank; 5]> {
        let mut best: Option<[Rank; 5]> = None;
        for bits in deck.get_single_suit_ranks() {
            if bits.num_unique_ranks() >= 5 {
                match (best, bits.get_highest_five()) {
                    (Some(existing), Some(newest)) => {
                        for i in 0..5 {
                            match
                             RankOrder::AceIsHigh.cmp(newest[i], existing[i])        {
                                std::cmp::Ordering::Less => continue,
                                std::cmp::Ordering::Equal => {},
                                std::cmp::Ordering::Greater => best = Some(existing),
                              }
                        }
                    }
                    (None, Some(existing)) => best = Some(existing),
                    (_, None) => {}
                }
            }
        }
        best
    }

    fn get_best_straight_flush(deck: &Deck) -> Option<Rank> {
        let mut found: Option<Rank> = None;
        for single_suit_rank in deck.get_single_suit_ranks() {
            match (single_suit_rank.get_straight(), found) {
                (Some(straight), Some(found_val)) => {
                    match RankOrder::AceIsHigh.cmp(straight, found_val) {
                        std::cmp::Ordering::Greater => found = Some(straight),
                        _ => {}
                    }
                }
                (Some(straight), None) => found = Some(straight),
                (None, _) => {}
            }
        }
        found
    }
}

#[cfg(test)]
mod test {
    use crate::{
        deck::*,
        ranking::hand_rank::StandardHandRanks,
        ranking::standard_hand_ranker::StandardHandRanker,
    };

    macro_rules! assert_rank {
        ($hand:expr, $rank:expr) => {
            let hand = deck_from_cards($hand);
            assert_eq!(StandardHandRanker::get_rank(&hand), $rank);
        };
    }

    pub fn deck_from_cards(val: &str) -> Deck {
        let card_strs = val.split(" ");
        let cards: Vec<Card> = card_strs.map(|x| Card::parse(x).unwrap()).collect();
        let mut deck = Deck::empty();
        deck.insert_cards(cards.iter());
        deck
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
        assert_rank!("As 2s 3h 4c 5d", StandardHandRanks::Straight { s: Rank::Five });
        assert_rank!("As 2s 3h 4c 5d 6d", StandardHandRanks::Straight { s: Rank::Six });
        assert_rank!("6s 2s 3s 4s 5s", StandardHandRanks::StraightFlush { sf: Rank::Six });
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
}
