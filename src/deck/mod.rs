use strum::IntoEnumIterator;

use crate::deck::card::{Cardlike, SimpleCard};
use crate::deck::rank::Rank;
use crate::deck::suit::Suit;
use crate::ranks::hand_rank::HandRank;

pub mod card;
pub mod rank;
pub mod suit;

pub trait Decklike {
    type Card: Cardlike + Copy + Clone;

    fn get_next_card(&mut self) -> Option<Self::Card>;

    fn cards_remaining(&self) -> u8;
}

pub struct StandardDeck {
    cards: Vec<SimpleCard>,
}

impl StandardDeck {
    pub fn new() -> Self {
        let mut cards = vec![];
        for suit in Suit::iter() {
            for rank in Rank::iter() {
                cards.push(SimpleCard::new(rank, suit));
            }
        }
        Self { cards }
    }

    #[cfg(test)]
    fn seed_deck(&mut self, _seed: i64) {}
}

impl Decklike for StandardDeck {
    type Card = SimpleCard;

    fn get_next_card(&mut self) -> Option<Self::Card> {
        // TODO: Make this random in the future
        if self.cards.len() > 0 {
            Some(self.cards.remove(0))
        } else {
            None
        }
    }

    fn cards_remaining(&self) -> u8 {
        self.cards.len() as u8
    }
}

pub struct DeckBitfield {
    cards: u64,
}

const SINGLE_SUIT_BITFIELD: u64 = 0b11111111111111;
const SINGLE_SUIT_HIGH_ACE_BITFIELD: u64 = 0b11111111111110;
const SINGLE_RANK_BITFIELD: u64 = 0b00010000000000000;
const SINGLE_RANK_FILTER: u64 = SINGLE_RANK_BITFIELD
    | SINGLE_RANK_BITFIELD << 16
    | SINGLE_RANK_BITFIELD << 32
    | SINGLE_RANK_BITFIELD << 48;
impl DeckBitfield {
    pub fn new() -> Self {
        Self { cards: 0 }
    }

    pub fn insert_cards(&mut self, cards: &[SimpleCard]) {
        for card in cards {
            let rank_bits: u64 = match card.rank() {
                Rank::Ace => 1 << 13 | 1,
                Rank::Two => 1 << 1,
                Rank::Three => 1 << 2,
                Rank::Four => 1 << 3,
                Rank::Five => 1 << 4,
                Rank::Six => 1 << 5,
                Rank::Seven => 1 << 6,
                Rank::Eight => 1 << 7,
                Rank::Nine => 1 << 8,
                Rank::Ten => 1 << 9,
                Rank::Jack => 1 << 10,
                Rank::Queen => 1 << 11,
                Rank::King => 1 << 12,
            };
            let suit_shift = match card.suit() {
                Suit::Clubs => 0,
                Suit::Diamonds => 16,
                Suit::Hearts => 32,
                Suit::Spades => 48,
            };
            let bits = rank_bits << suit_shift;
            self.cards |= bits;
        }
    }

    fn get_single_suit_card_ranks(&self, i: usize) -> u64 {
        self.cards >> (16 * i) & SINGLE_SUIT_BITFIELD
    }

    fn get_straight_internal(ranks: u64) -> Option<Rank> {
        let result = Self::matches_pattern(ranks, 0b11111, 5);
        result.map(|x| Rank::try_from_usize(x + 2).unwrap())
    }

    fn get_combined_ranks(&self) -> u64 {
        // combine the deck for all suits
        let mut ranks_combined = 0;
        for i in 0..4 {
            ranks_combined |= self.get_single_suit_card_ranks(i);
        }
        ranks_combined
    }

    pub fn get_straight(&self) -> Option<Rank> {
        Self::get_straight_internal(self.get_combined_ranks())
    }

    pub fn get_flush(&self) -> Option<[Rank; 5]> {
        let mut best: Option<[Rank; 5]> = None;
        for i in 0..4 {
            let bits = self.get_single_suit_card_ranks(i);
            if bits.count_ones() >= 5 {
                match (best, Self::get_highest_five(bits)) {
                    (Some(existing), Some(newest)) => {
                        for i in 0..5 {
                            if newest[i] > existing[i] {
                                best = Some(existing)
                            }
                            if newest[i] < existing[i] {
                                continue;
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

    fn matches_pattern(
        bits_to_check: u64,
        required_on_bits: u64,
        field_length: usize,
    ) -> Option<usize> {
        for i in 0..(15 - field_length) {
            let must_match = required_on_bits << (14 - field_length - i);
            if must_match & bits_to_check == must_match {
                return Some(15 - field_length - i);
            }
        }
        return None;
    }

    pub fn get_straight_flush(&self) -> Option<Rank> {
        let mut found: Option<Rank> = None;
        for i in 0..4 {
            let value_to_check = self.get_single_suit_card_ranks(i);
            match (Self::get_straight_internal(value_to_check), found) {
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

    pub fn get_highest_five(cards: u64) -> Option<[Rank; 5]> {
        // we get rid of the low aces since aces are always high
        let mut all_ranks = 0;
        for i in 0..4 {
            all_ranks |= (cards >> 16 * i) & SINGLE_SUIT_HIGH_ACE_BITFIELD;
        }

        let count = u64::count_ones(all_ranks);
        if count < 5 {
            None
        } else {
            let mut return_val = vec![];
            for i in 1..14 {
                let target = 0b1 << (14 - i);
                if target & cards == target {
                    return_val.push(Rank::try_from_usize(13 - i).unwrap());
                    if return_val.len() == 5 {
                        break;
                    }
                }
            }
            let slice = return_val.try_into().unwrap();
            Some(slice)
        }
    }

    fn get_rank_counts(&self) -> Vec<u32> {
        let mut counts = vec![];
        for i in 0..13 {
            let filtered = self.cards & (SINGLE_RANK_FILTER >> i);
            counts.push(filtered.count_ones());
        }
        counts
    }

    fn find_highest_with_n(
        ranks: &Vec<u32>,
        indexes_to_exclude: &Vec<usize>,
        target_count: u32,
    ) -> Option<usize> {
        for (index, val) in ranks.iter().enumerate() {
            if indexes_to_exclude.contains(&index) {
                continue;
            } else {
                if *val >= target_count {
                    return Some(index);
                }
            }
        }
        return None;
    }

    pub fn get_rank(&self) -> HandRank {
        let rank_counts = self.get_rank_counts();
        if let Some(sf) = self.get_straight_flush() {
            HandRank::StraightFlush { sf }
        } else {
            if let Some(quad_index) = Self::find_highest_with_n(&rank_counts, &vec![], 4) {
                if let Some(kicker_index) =
                    Self::find_highest_with_n(&rank_counts, &vec![quad_index], 1)
                {
                    return HandRank::FourOfAKind {
                        q: Rank::try_from_usize(13 - quad_index - 1).unwrap(),
                        c: Rank::try_from_usize(13 - kicker_index - 1).unwrap(),
                    };
                }
            }
            if let Some(trip_index) = Self::find_highest_with_n(&rank_counts, &vec![], 3) {
                if let Some(pair_index) =
                    Self::find_highest_with_n(&rank_counts, &vec![trip_index], 2)
                {
                    return HandRank::FullHouse {
                        t: Rank::try_from_usize(13 - trip_index - 1).unwrap(),
                        p: Rank::try_from_usize(13 - pair_index - 1).unwrap(),
                    };
                }
            }
            if let Some(flush_ranks) = self.get_flush() {
                HandRank::Flush {
                    c1: flush_ranks[0],
                    c2: flush_ranks[1],
                    c3: flush_ranks[2],
                    c4: flush_ranks[3],
                    c5: flush_ranks[4],
                }
            } else if let Some(s) = self.get_straight() {
                HandRank::Straight { s }
            } else {
                if let Some(trip_index) = Self::find_highest_with_n(&rank_counts, &vec![], 3) {
                    if let Some(c1_index) =
                        Self::find_highest_with_n(&rank_counts, &vec![trip_index], 1)
                    {
                        if let Some(c2_index) =
                            Self::find_highest_with_n(&rank_counts, &vec![trip_index, c1_index], 1)
                        {
                            return HandRank::ThreeOfAKind {
                                t: Rank::try_from_usize(13 - trip_index - 1).unwrap(),
                                c1: Rank::try_from_usize(13 - c1_index - 1).unwrap(),
                                c2: Rank::try_from_usize(13 - c2_index - 1).unwrap(),
                            };
                        }
                    }
                }
                if let Some(pair1_index) = Self::find_highest_with_n(&rank_counts, &vec![], 2) {
                    if let Some(pair2_index) =
                        Self::find_highest_with_n(&rank_counts, &vec![pair1_index], 2)
                    {
                        if let Some(c1_index) = Self::find_highest_with_n(
                            &rank_counts,
                            &vec![pair1_index, pair2_index],
                            2,
                        ) {
                            return HandRank::TwoPair {
                                p1: Rank::try_from_usize(13 - pair1_index - 1).unwrap(),
                                p2: Rank::try_from_usize(13 - pair2_index - 1).unwrap(),
                                c1: Rank::try_from_usize(13 - c1_index - 1).unwrap(),
                            };
                        }
                    }
                }
                if let Some(pair_index) = Self::find_highest_with_n(&rank_counts, &vec![], 2) {
                    if let Some(c1_index) =
                        Self::find_highest_with_n(&rank_counts, &vec![pair_index], 1)
                    {
                        if let Some(c2_index) =
                            Self::find_highest_with_n(&rank_counts, &vec![pair_index, c1_index], 1)
                        {
                            if let Some(c3_index) = Self::find_highest_with_n(
                                &rank_counts,
                                &vec![pair_index, c1_index, c2_index],
                                1,
                            ) {
                                return HandRank::OnePair {
                                    p: Rank::try_from_usize(13 - pair_index - 1).unwrap(),
                                    c1: Rank::try_from_usize(13 - c1_index - 1).unwrap(),
                                    c2: Rank::try_from_usize(13 - c2_index - 1).unwrap(),
                                    c3: Rank::try_from_usize(13 - c3_index - 1).unwrap(),
                                };
                            }
                        }
                    }
                }
                let combined_ranks = self.get_combined_ranks();
                let ranks = Self::get_highest_five(combined_ranks).unwrap();
                HandRank::HighCard {
                    c1: ranks[0],
                    c2: ranks[1],
                    c3: ranks[2],
                    c4: ranks[3],
                    c5: ranks[4],
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use crate::deck::DeckBitfield;
    use crate::deck::card::SimpleCard;
    use crate::deck::rank::Rank;
    use crate::deck::suit::Suit;
    use crate::deck::{Decklike, StandardDeck, card::Cardlike};
    use crate::ranks::hand_rank::HandRank;

    macro_rules! assert_straight_and_straight_flush {
        ($hand:expr, $straight:expr, $straight_flush:expr) => {
            let hand = hand_ranker_from_cards($hand);
            assert_eq!(hand.get_straight(), $straight);
            assert_eq!(hand.get_straight_flush(), $straight_flush);
        };
    }

    macro_rules! assert_rank {
        ($hand:expr, $rank:expr) => {
            let hand = hand_ranker_from_cards($hand);
            assert_eq!(hand.get_rank(), $rank);
        };
    }

    #[test]
    pub fn deal_a_flop() -> Result<(), String> {
        let mut deck = StandardDeck::new();
        deck.seed_deck(1);
        assert_eq!(52, deck.cards_remaining());

        test_card(&mut deck, Suit::Clubs, Rank::Two);
        test_card(&mut deck, Suit::Clubs, Rank::Three);
        test_card(&mut deck, Suit::Clubs, Rank::Four);

        Ok(())
    }

    pub fn test_card(deck: &mut StandardDeck, suit: Suit, rank: Rank) {
        let card = deck.get_next_card().unwrap();
        assert_eq!(suit, card.suit());
        assert_eq!(rank, card.rank());
    }

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
        let flush_ranks = hand.get_flush();
        assert_eq!(
            flush_ranks,
            Some([Rank::Ace, Rank::King, Rank::Queen, Rank::Ten, Rank::Nine])
        );
    }

    pub fn hand_ranker_from_cards(val: &str) -> DeckBitfield {
        let card_strs = val.split(" ");
        let cards: Vec<SimpleCard> = card_strs
            .map(|x| SimpleCard::try_from_str(x).unwrap())
            .collect();
        let mut hand_ranker = DeckBitfield::new();
        hand_ranker.insert_cards(&cards);
        hand_ranker
    }

    #[test]
    pub fn get_hand_ranks() {
        assert_rank!(
            "As 2s 3h 4c 6d",
            HandRank::HighCard {
                c1: Rank::Ace,
                c2: Rank::Six,
                c3: Rank::Four,
                c4: Rank::Three,
                c5: Rank::Two,
            }
        );
        assert_rank!(
            "As 2s 3s 4s 6s",
            HandRank::Flush {
                c1: Rank::Ace,
                c2: Rank::Six,
                c3: Rank::Four,
                c4: Rank::Three,
                c5: Rank::Two,
            }
        );
        assert_rank!("As 2s 3h 4c 5d", HandRank::Straight { s: Rank::Five });
        assert_rank!("As 2s 3h 4c 5d 6d", HandRank::Straight { s: Rank::Six });
        assert_rank!("6s 2s 3s 4s 5s", HandRank::StraightFlush { sf: Rank::Six });
        assert_rank!(
            "6d 6c 6h 6s 5s",
            HandRank::FourOfAKind {
                q: Rank::Six,
                c: Rank::Five
            }
        );
        assert_rank!(
            "6s 2s 2h 4s 5s",
            HandRank::OnePair {
                p: Rank::Two,
                c1: Rank::Six,
                c2: Rank::Five,
                c3: Rank::Four
            }
        );
    }
}
