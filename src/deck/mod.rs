use std::fmt::Display;

use crate::deck::card::Card;
use crate::deck::rank::Rank;
use crate::deck::suit::Suit;
use crate::ranks::hand_rank::HandRank;

pub mod card;
pub mod rank;
pub mod suit;

#[derive(Debug)]
pub struct Deck {
    cards: u64,
}

impl Display for Deck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut cards = vec![];
        for i in 0..4 {
            for j in 0..16 {
                if self.cards & 0b1 << (16 * i + j) > 0 {
                    let card = Card::try_from_usize(i * 16 + j).unwrap();
                    cards.push(card);
                }
            }
        }
        let cards_str: Vec<String> = cards.iter().map(|x| format!("{}", *x)).collect();
        let cards_as_str = cards_str.join(" ");
        write!(f, "{}", cards_as_str)
    }
}

const SINGLE_SUIT_BITFIELD: u64 = 0b11111111111111;
const SINGLE_SUIT_HIGH_ACE_BITFIELD: u64 = 0b11111111111110;
const ALL_CARDS_BITFIELD: u64 = SINGLE_SUIT_BITFIELD
    | SINGLE_SUIT_BITFIELD << 16
    | SINGLE_SUIT_BITFIELD << 32
    | SINGLE_SUIT_BITFIELD << 48;
const ALL_CARDS_NO_LOW_ACES_BITFIELD: u64 = SINGLE_SUIT_HIGH_ACE_BITFIELD
    | SINGLE_SUIT_HIGH_ACE_BITFIELD
    | 16 + SINGLE_SUIT_HIGH_ACE_BITFIELD << 32
    | SINGLE_SUIT_HIGH_ACE_BITFIELD << 48;
const SINGLE_RANK_BITFIELD: u64 = 0b00010000000000000;
const SINGLE_RANK_FILTER: u64 = SINGLE_RANK_BITFIELD
    | SINGLE_RANK_BITFIELD << 16
    | SINGLE_RANK_BITFIELD << 32
    | SINGLE_RANK_BITFIELD << 48;

const RANKS: [Rank; 13] = [
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

const SUITS: [Suit; 4] = [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades];

impl Deck {
    pub fn empty() -> Self {
        Self { cards: 0 }
    }
    pub fn all_cards() -> Self {
        Self {
            cards: ALL_CARDS_BITFIELD,
        }
    }

    fn get_bits_for_card(card: &Card) -> u64 {
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
        bits
    }

    pub fn has_card(&self, card: Card) -> bool {
        let target = Self::get_bits_for_card(&card);
        target == target & self.cards
    }

    pub fn insert_cards(&mut self, cards: &[Card]) {
        for card in cards {
            self.cards |= Self::get_bits_for_card(card);
        }
    }

    pub fn remove_cards(&mut self, cards: &[Card]) {
        for card in cards {
            self.cards ^= Self::get_bits_for_card(card);
        }
    }

    pub fn num_cards(&self) -> u32 {
        let all_ranks = Self::get_without_low_aces(self.cards);

        u64::count_ones(all_ranks)
    }

    fn get_single_suit_card_ranks(&self, i: usize) -> u64 {
        self.cards >> (16 * i) & SINGLE_SUIT_BITFIELD
    }

    fn get_straight_internal(ranks: u64) -> Option<Rank> {
        let result = Self::matches_pattern(ranks, 0b11111, 5);
        result.map(|x| RANKS[x + 2])
    }

    fn get_combined_ranks(&self) -> u64 {
        // combine the deck for all suits
        let mut ranks_combined = 0;
        for i in 0..4 {
            ranks_combined |= self.get_single_suit_card_ranks(i);
        }
        ranks_combined
    }

    fn get_straight(&self) -> Option<Rank> {
        Self::get_straight_internal(self.get_combined_ranks())
    }

    fn get_flush(&self) -> Option<[Rank; 5]> {
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

    fn get_straight_flush(&self) -> Option<Rank> {
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

    fn get_highest_five(cards: u64) -> Option<[Rank; 5]> {
        let all_ranks = Self::get_without_low_aces(cards);

        let count = u64::count_ones(all_ranks);
        if count < 5 {
            None
        } else {
            let mut return_val = vec![];
            for i in 1..14 {
                let target = 0b1 << (14 - i);
                if target & cards == target {
                    return_val.push(RANKS[13 - i]);
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
            let best_quads = Self::find_highest_with_n(&rank_counts, &vec![], 4);

            if let Some(quad_index) = best_quads {
                if let Some(kicker_index) =
                    Self::find_highest_with_n(&rank_counts, &vec![quad_index], 1)
                {
                    return HandRank::FourOfAKind {
                        q: RANKS[13 - quad_index - 1],
                        c: RANKS[13 - kicker_index - 1],
                    };
                }
            }
            let best_trips = Self::find_highest_with_n(&rank_counts, &vec![], 3);

            if let Some(trip_index) = best_trips {
                if let Some(pair_index) =
                    Self::find_highest_with_n(&rank_counts, &vec![trip_index], 2)
                {
                    return HandRank::FullHouse {
                        t: RANKS[13 - trip_index - 1],
                        p: RANKS[13 - pair_index - 1],
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
                if let Some(trip_index) = best_trips {
                    if let Some(c1_index) =
                        Self::find_highest_with_n(&rank_counts, &vec![trip_index], 1)
                    {
                        if let Some(c2_index) =
                            Self::find_highest_with_n(&rank_counts, &vec![trip_index, c1_index], 1)
                        {
                            return HandRank::ThreeOfAKind {
                                t: RANKS[13 - trip_index - 1],
                                c1: RANKS[13 - c1_index - 1],
                                c2: RANKS[13 - c2_index - 1],
                            };
                        }
                    }
                }
                let best_pair = Self::find_highest_with_n(&rank_counts, &vec![], 2);
                if let Some(pair1_index) = best_pair {
                    if let Some(pair2_index) =
                        Self::find_highest_with_n(&rank_counts, &vec![pair1_index], 2)
                    {
                        if let Some(c1_index) = Self::find_highest_with_n(
                            &rank_counts,
                            &vec![pair1_index, pair2_index],
                            1,
                        ) {
                            return HandRank::TwoPair {
                                p1: RANKS[13 - pair1_index - 1],
                                p2: RANKS[13 - pair2_index - 1],
                                c1: RANKS[13 - c1_index - 1],
                            };
                        }
                    }
                }
                if let Some(pair_index) = best_pair {
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
                                    p: RANKS[13 - pair_index - 1],
                                    c1: RANKS[13 - c1_index - 1],
                                    c2: RANKS[13 - c2_index - 1],
                                    c3: RANKS[13 - c3_index - 1],
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

    fn get_without_low_aces(cards: u64) -> u64 {
        cards & ALL_CARDS_NO_LOW_ACES_BITFIELD
    }
}

#[cfg(test)]
mod test {
    use crate::deck::Deck;
    use crate::deck::card::Card;
    use crate::deck::rank::Rank;
    use crate::deck::suit::Suit;
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

    pub fn hand_ranker_from_cards(val: &str) -> Deck {
        let card_strs = val.split(" ");
        let cards: Vec<Card> = card_strs.map(|x| Card::try_from_str(x).unwrap()).collect();
        let mut hand_ranker = Deck::empty();
        hand_ranker.insert_cards(&cards);
        hand_ranker
    }

    #[test]
    pub fn get_hand_ranks() {
        assert_rank!(
            "3c 4c 5c 3d 4d",
            HandRank::TwoPair {
                p1: Rank::Four,
                p2: Rank::Three,
                c1: Rank::Five
            }
        );
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

        let test_deck = Deck { cards: 393230 };
        println!("{}", test_deck.to_string());
        test_deck.get_rank();
    }

    #[test]
    pub fn has_card() {
        let mut empty = Deck::empty();
        let mut full = Deck::all_cards();

        for card in Card::all_cards() {
            assert!(!empty.has_card(card));
            assert!(full.has_card(card))
        }

        let two_clubs = Card::new(Rank::Two, Suit::Clubs);
        let three_clubs = Card::new(Rank::Three, Suit::Clubs);

        empty.insert_cards(&[two_clubs.clone()]);
        full.remove_cards(&[three_clubs.clone()]);
        assert!(empty.has_card(two_clubs));
        assert!(!empty.has_card(three_clubs));
        assert!(!full.has_card(three_clubs));
        assert!(full.has_card(two_clubs));
    }
}
