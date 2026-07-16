use crate::{
    deck::Rank};

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
            RankOrder::AceIsHigh => return_val - 1 ,
            RankOrder::AceIsLow => return_val,
        }
    }

    pub fn cmp(&self, a: Rank, b: Rank) -> std::cmp::Ordering {
        self.get_score(&a).cmp(&self.get_score(&b))
    }
}

#[cfg(test)]
mod test {
    use crate::{
        deck::*,
        ranking::hand_rank::StandardHandRanks,
        ranking::hand_rank::StandardHandRanker,
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
