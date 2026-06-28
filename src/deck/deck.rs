use std::fmt::Display;
use std::ops::BitOr;

use numerica::combinatorics::CombinationIterator;

use crate::deck::card::Card;
use crate::deck::rank::Rank;
use crate::deck::suit::Suit;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deck {
    cards: u64,
}

impl Display for Deck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut cards = vec![];
        for j in 0..13 {
            for i in 0..4 {
                let bits_to_check = RANK_BITS[j] << (16 * i);
                if self.cards & bits_to_check == bits_to_check {
                    let card = Card::from_deck(
                        Self {
                            cards: bits_to_check,
                        },
                        RANKS[j],
                        SUITS[i],
                    );
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
    | SINGLE_SUIT_HIGH_ACE_BITFIELD << 16
    | SINGLE_SUIT_HIGH_ACE_BITFIELD << 32
    | SINGLE_SUIT_HIGH_ACE_BITFIELD << 48;
const SINGLE_RANK_BITFIELD: u64 = 0b00010000000000000;
const SINGLE_RANK_FILTER: u64 = SINGLE_RANK_BITFIELD
    | SINGLE_RANK_BITFIELD << 16
    | SINGLE_RANK_BITFIELD << 32
    | SINGLE_RANK_BITFIELD << 48;

pub(crate) const RANKS: [Rank; 13] = [
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

pub(crate) const RANK_BITS: [u64; 13] = [
    0b1 << 1,
    0b1 << 2,
    0b1 << 3,
    0b1 << 4,
    0b1 << 5,
    0b1 << 6,
    0b1 << 7,
    0b1 << 8,
    0b1 << 9,
    0b1 << 10,
    0b1 << 11,
    0b1 << 12,
    0b1 << 13 | 0b1,
];

pub(crate) const SUITS: [Suit; 4] = [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades];

impl Deck {
    pub fn empty() -> Self {
        Self { cards: 0 }
    }
    pub fn all_cards() -> Self {
        Self {
            cards: ALL_CARDS_BITFIELD,
        }
    }

    pub fn get_card(rank: &Rank, suit: &Suit) -> Self {
        Self {
            cards: Self::get_bits_for_card(rank, suit),
        }
    }

    fn get_bits_for_card(rank: &Rank, suit: &Suit) -> u64 {
        let rank_bits: u64 = match rank {
            &Rank::Ace => 1 << 13 | 1,
            &Rank::Two => 1 << 1,
            &Rank::Three => 1 << 2,
            &Rank::Four => 1 << 3,
            &Rank::Five => 1 << 4,
            &Rank::Six => 1 << 5,
            &Rank::Seven => 1 << 6,
            &Rank::Eight => 1 << 7,
            &Rank::Nine => 1 << 8,
            &Rank::Ten => 1 << 9,
            &Rank::Jack => 1 << 10,
            &Rank::Queen => 1 << 11,
            &Rank::King => 1 << 12,
        };
        let suit_shift = match suit {
            &Suit::Clubs => 0,
            &Suit::Diamonds => 16,
            &Suit::Hearts => 32,
            &Suit::Spades => 48,
        };
        let bits = rank_bits << suit_shift;
        bits
    }

    pub fn has_card(&self, card: Card) -> bool {
        card.get_deck().cards & self.cards > 0
    }

    pub fn insert_cards(&mut self, cards: &[Card]) {
        for card in cards {
            self.cards |= card.get_deck().cards
        }
    }

    pub fn remove_cards(&mut self, cards: &[Card]) {
        for card in cards {
            self.cards ^= card.get_deck().cards
        }
    }

    fn remove_nth_card_unchecked(&mut self, index: usize) -> Card {
        let card = self.get_nth_card_unchecked(index);
        self.remove_cards(&[card]);
        card
    }

    pub fn try_remove_nth_card(&mut self, index: usize) -> Result<Card, String> {
        let num_cards = self.num_cards();
        if index >= num_cards as usize {
            return Err("Too many cards".to_owned());
        }
        Ok(self.remove_nth_card_unchecked(index))
    }

    pub fn get_nth_card_unchecked(&self, index: usize) -> Card {
        let mut count: usize = index;
        for suit in 0..4 {
            for rank in 0..13 {
                let bits_for_card = Self::get_bits_for_card(&RANKS[rank], &SUITS[suit]);
                if self.cards & bits_for_card == bits_for_card {
                    if count == 0 {
                        return Card::from_deck(
                            Deck {
                                cards: bits_for_card,
                            },
                            RANKS[rank].clone(),
                            SUITS[suit].clone(),
                        );
                    } else {
                        count -= 1;
                    }
                }
            }
        }
        panic!()
    }

    pub fn try_remove_random_cards(&mut self, number_to_remove: u32) -> Result<Vec<Card>, String> {
        let mut num_cards = self.num_cards();
        let mut cards = vec![];

        if number_to_remove > num_cards {
            return Err("Too many cards to remove".to_owned());
        }
        for _ in 0..number_to_remove {
            let index = rand::random_range(0..num_cards);
            cards.push(self.try_remove_nth_card(index as usize)?);
            num_cards -= 1;
        }
        Ok(cards)
    }

    pub fn is_empty(&self) -> bool {
        self.cards == 0
    }

    pub fn num_cards(&self) -> u32 {
        let all_ranks = Self::get_without_low_aces(self.cards);
        u64::count_ones(all_ranks)
    }

    pub fn get_single_suit_ranks(&self) -> impl Iterator<Item = RankBitfield> {
        SingleSuitRankIterator {
            deck: self.clone(),
            suit_index: 0,
        }
    }

    fn get_single_suit_card_ranks(&self, i: usize) -> u64 {
        self.cards >> (16 * i) & SINGLE_SUIT_BITFIELD
    }

    pub fn get_combined_rank_bitfield(&self) -> RankBitfield {
        RankBitfield {
            ranks: self.get_combined_ranks(),
        }
    }

    fn get_combined_ranks(&self) -> u64 {
        // combine the deck for all suits
        let mut ranks_combined = 0;
        for i in 0..4 {
            ranks_combined |= self.get_single_suit_card_ranks(i);
        }
        ranks_combined
    }

    pub fn get_rank_count(&self) -> RankCount {
        RankCount {
            rank_counts: self.get_rank_counts(),
        }
    }

    fn get_rank_counts(&self) -> [u32; 13] {
        let mut counts = [0; 13];
        for i in 0..13 {
            let filtered = self.cards & (SINGLE_RANK_FILTER >> i);
            counts[i] = filtered.count_ones();
        }
        counts
    }

    fn get_without_low_aces(cards: u64) -> u64 {
        cards & ALL_CARDS_NO_LOW_ACES_BITFIELD
    }

    pub fn enumerate_combinations(self, num_cards: usize) -> impl Iterator<Item = Deck> {
        DeckIterator::new(self, num_cards)
    }
}

struct SingleSuitRankIterator {
    deck: Deck,
    suit_index: usize,
}

impl Iterator for SingleSuitRankIterator {
    type Item = RankBitfield;

    fn next(&mut self) -> Option<Self::Item> {
        if self.suit_index == 4 {
            None
        } else {
            let ranks = self.deck.get_single_suit_card_ranks(self.suit_index);
            self.suit_index += 1;
            Some(RankBitfield { ranks })
        }
    }
}

pub struct RankBitfield {
    ranks: u64,
}

impl RankBitfield {
    pub fn get_straight(&self) -> Option<Rank> {
        let result = Self::matches_pattern(self.ranks, 0b11111, 5);
        result.map(|x| RANKS[x + 2])
    }

    pub fn count_ones(&self) -> u32 {
        self.ranks.count_ones()
    }

    pub fn get_highest_five(&self) -> Option<[Rank; 5]> {
        let all_ranks = self.ranks & SINGLE_SUIT_HIGH_ACE_BITFIELD;

        let count = u64::count_ones(all_ranks);
        if count < 5 {
            None
        } else {
            let mut return_val = vec![];
            for i in 1..14 {
                let target = 0b1 << (14 - i);
                if target & all_ranks == target {
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
}

pub struct RankCount {
    rank_counts: [u32; 13],
}

impl RankCount {
    pub fn find_highest_with_n(
        &self,
        ranks_to_exclude: &Vec<Rank>,
        target_count: u32,
    ) -> Option<Rank> {
        for (index, val) in self.rank_counts.iter().enumerate() {
            if ranks_to_exclude.contains(&RANKS[12 - index]) {
                continue;
            } else {
                if *val >= target_count {
                    return Some(RANKS[12 - index]);
                }
            }
        }
        return None;
    }
}

impl BitOr for Deck {
    type Output = Deck;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self {
            cards: self.cards | rhs.cards,
        }
    }
}

struct DeckIterator {
    deck: Deck,
    iterator: CombinationIterator,
}

impl DeckIterator {
    fn new(deck: Deck, size: usize) -> Self {
        let num_cards = deck.num_cards();
        let iterator = numerica::combinatorics::CombinationIterator::new(num_cards as usize, size);
        Self {
            deck: deck,
            iterator,
        }
    }
}

impl Iterator for DeckIterator {
    type Item = Deck;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(vals) = self.iterator.next() {
            let mut deck = Deck::empty();
            let cards: Vec<Card> = vals
                .iter()
                .map(|i| self.deck.get_nth_card_unchecked(*i))
                .collect::<Vec<Card>>();
            deck.insert_cards(&cards);
            Some(deck)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod test {
    use crate::deck::card::Card;
    use crate::deck::deck::Deck;
    use crate::deck::rank::Rank;
    use crate::deck::suit::Suit;

    pub fn deck_from_cards(val: &str) -> Deck {
        let card_strs = val.split(" ");
        let cards: Vec<Card> = card_strs.map(|x| Card::try_from_str(x).unwrap()).collect();
        let mut deck = Deck::empty();
        deck.insert_cards(&cards);
        deck
    }

    #[test]
    pub fn test_has_card() {
        let mut empty = Deck::empty();
        let mut full = Deck::all_cards();

        for card in Card::values() {
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

        let card = empty.try_remove_nth_card(0).unwrap();
        assert_eq!(card, two_clubs);
        assert!(!empty.has_card(two_clubs));

        let card = full.try_remove_nth_card(50).unwrap();
        assert_eq!(card, Card::try_from_str("As").unwrap());
        assert_eq!(full.num_cards(), 50);
        let card = full.try_remove_nth_card(48).unwrap();
        assert_eq!(card, Card::try_from_str("Qs").unwrap());
        assert_eq!(full.num_cards(), 49);
    }

    #[test]
    pub fn test_deck_display() {
        let deck = Deck::all_cards();
        println!("{}", deck);
    }

    #[test]
    pub fn test_iterator() {
        let deck = deck_from_cards("As Ks Qs");

        let mut subdeck_1 = deck_from_cards("As Ks");
        let mut subdeck_2 = deck_from_cards("As Qs");
        let mut subdeck_3 = deck_from_cards("Ks Qs");

        for subdeck in deck.enumerate_combinations(2) {
            assert_eq!(subdeck.num_cards(), 2);
            if subdeck_1 == subdeck {
                subdeck_1.try_remove_nth_card(0).unwrap();
                subdeck_1.try_remove_nth_card(0).unwrap();
            } else if subdeck_2 == subdeck {
                subdeck_2.try_remove_nth_card(0).unwrap();
                subdeck_2.try_remove_nth_card(0).unwrap();
            } else if subdeck_3 == subdeck {
                subdeck_3.try_remove_nth_card(0).unwrap();
                subdeck_3.try_remove_nth_card(0).unwrap();
            } else {
                assert!(false)
            }
        }
    }
}
