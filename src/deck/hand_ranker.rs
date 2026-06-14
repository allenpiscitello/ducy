use std::collections::HashMap;

use strum::IntoEnumIterator;

use crate::deck::{
    card::Cardlike,
    rank::{Rank, RankBitfield},
    suit::Suit,
};

pub struct HandRanker {
    ranks: RankBitfield,
    suit_ranks: HashMap<Suit, RankBitfield>,
}

impl HandRanker {
    pub fn new() -> Self {
        let mut suit_ranks = HashMap::new();
        for suit in Suit::iter() {
            suit_ranks.insert(suit, RankBitfield::new());
        }

        Self {
            ranks: RankBitfield::new(),
            suit_ranks,
        }
    }

    pub fn insert(&mut self, card: impl Cardlike) {
        let rank = card.rank();
        let suit = card.suit();
        self.ranks.add_ranks(&[rank]);
        if let Some(val) = self.suit_ranks.get_mut(&suit) {
            val.add_ranks(&[rank]);
        }
    }

    pub fn get_straight(&self) -> Option<Rank> {
        self.ranks.get_straight()
    }

    pub fn get_straight_flush(&self) -> Option<Rank> {
        let mut found: Option<Rank> = None;
        for suit in Suit::iter() {
            match (self.suit_ranks[&suit].get_straight(), found) {
                (Some(straight), Some(found_val)) => {
                    if straight > found_val {
                        found = Some(straight)
                    }
                }
                (Some(straight), None) => found = Some(straight),
                (None, _) => {}
            }
        }
        found
    }

    pub fn get_flush_ranks(&self) -> Option<[Rank; 5]> {
        let mut potential_ranks: Option<[Rank; 5]> = None;
        let mut rank_score: u32 = 0;
        for suit in Suit::iter() {
            if let Some(ranks) = self.suit_ranks[&suit].get_highest_five() {
                let mut score = 0;
                for rank in ranks {
                    score = score * 13 + rank.get_score();
                }
                if score > rank_score {
                    potential_ranks = Some(ranks);
                    rank_score = score;
                }
            }
        }
        potential_ranks
    }
}

#[cfg(test)]
pub mod test {

    macro_rules! assert_straight_and_straight_flush {
        ($hand:expr, $straight:expr, $straight_flush:expr) => {
            let hand = hand_ranker_from_cards($hand);
            assert_eq!(hand.get_straight(), $straight);
            assert_eq!(hand.get_straight_flush(), $straight_flush);
        };
    }

    use crate::deck::{card::SimpleCard, hand_ranker::HandRanker, rank::Rank};

    #[test]
    pub fn test_straight_and_straight_flush() {
        assert_straight_and_straight_flush!("As 2s 3s 4s", None, None);
        assert_straight_and_straight_flush!("As 2s 3s 4s 5s", Some(Rank::Five), Some(Rank::Five));
        assert_straight_and_straight_flush!("6s 2s 3s 4s 5s", Some(Rank::Six), Some(Rank::Six));
        assert_straight_and_straight_flush!("As 6h 2s 3s 4s 5s", Some(Rank::Six), Some(Rank::Five));
        assert_straight_and_straight_flush!("As 6h 2s 3s 4s 5s", Some(Rank::Six), Some(Rank::Five));
        assert_straight_and_straight_flush!("6s 7s 3s 4s 5s", Some(Rank::Seven), Some(Rank::Seven));
        assert_straight_and_straight_flush!("6s 7s 8h 4s 5s", Some(Rank::Eight), None);
        assert_straight_and_straight_flush!("Js Qs Kh As Ts", Some(Rank::Ace), None);
        assert_straight_and_straight_flush!("9s Js Qs Ks Ah Ts", Some(Rank::Ace), Some(Rank::King));
    }

    #[test]
    pub fn test_get_flush() {
        let hand = hand_ranker_from_cards("As Ks Qs Ts 9s 7h 4s");
        let flush_ranks = hand.get_flush_ranks();
        assert_eq!(
            flush_ranks,
            Some([Rank::Ace, Rank::King, Rank::Queen, Rank::Ten, Rank::Nine])
        );
    }

    pub fn hand_ranker_from_cards(val: &str) -> HandRanker {
        let card_strs = val.split(" ");
        let cards: Vec<SimpleCard> = card_strs
            .map(|x| SimpleCard::try_from_str(x).unwrap())
            .collect();
        let mut hand_ranker = HandRanker::new();
        for card in cards {
            hand_ranker.insert(card);
        }
        hand_ranker
    }
}
