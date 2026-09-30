use std::fmt::Display;
use std::ops::{BitOr, BitOrAssign, Sub, SubAssign};
use std::str::FromStr;

use strum_macros::EnumIter;

use crate::error::DucyError;
use crate::ranking::standard_hand_ranker::RankOrder;

/// Weighted hand ranges for equity calculations.
pub mod range;

/// Represents the rank of a playing card (Two through Ace).
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Rank {
    /// 2
    Two,
    /// 3
    Three,
    /// 4
    Four,
    /// 5
    Five,
    /// 6
    Six,
    /// 7
    Seven,
    /// 8
    Eight,
    /// 9
    Nine,
    /// 10
    Ten,
    /// Jack
    Jack,
    /// Queen
    Queen,
    /// King
    King,
    /// Ace
    Ace,
}

impl Rank {
    /// Tries to create a `Rank` from a character representation.
    /// Returns `Ok(Rank)` if successful, or `Err(String)` if the character is invalid.
    /// Characters representing ranks are: '2'-'9', 'T' (Ten), 'J' (Jack), 'Q' (Queen), 'K' (King), 'A' (Ace).
    pub fn try_from_char(val: &char) -> Result<Self, DucyError> {
        match val.to_ascii_lowercase() {
            'a' => Ok(Rank::Ace),
            '2' => Ok(Rank::Two),
            '3' => Ok(Rank::Three),
            '4' => Ok(Rank::Four),
            '5' => Ok(Rank::Five),
            '6' => Ok(Rank::Six),
            '7' => Ok(Rank::Seven),
            '8' => Ok(Rank::Eight),
            '9' => Ok(Rank::Nine),
            't' => Ok(Rank::Ten),
            'j' => Ok(Rank::Jack),
            'q' => Ok(Rank::Queen),
            'k' => Ok(Rank::King),
            _ => Err(DucyError::InvalidRank),
        }
    }
}

impl Display for Rank {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Rank::Ace => write!(f, "A"),
            Rank::Two => write!(f, "2"),
            Rank::Three => write!(f, "3"),
            Rank::Four => write!(f, "4"),
            Rank::Five => write!(f, "5"),
            Rank::Six => write!(f, "6"),
            Rank::Seven => write!(f, "7"),
            Rank::Eight => write!(f, "8"),
            Rank::Nine => write!(f, "9"),
            Rank::Ten => write!(f, "T"),
            Rank::Jack => write!(f, "J"),
            Rank::Queen => write!(f, "Q"),
            Rank::King => write!(f, "K"),
        }
    }
}

/// Represents the suit of a playing card (Clubs, Diamonds, Hearts, Spades).
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Suit {
    /// Clubs (c)
    Clubs,
    /// Diamonds (d)
    Diamonds,
    /// Hearts (h)
    Hearts,
    /// Spades (s)
    Spades,
}

impl Suit {
    /// Tries to create a `Suit` from a character representation.
    /// Returns `Ok(Suit)` if successful, or `Err(String)` if the character is invalid.
    /// Characters representing suits are: 'c' (Clubs), 'd' (Diamonds), 'h' (Hearts), 's' (Spades).
    pub fn try_from_char(val: &char) -> Result<Self, DucyError> {
        match val.to_ascii_lowercase() {
            'c' => Ok(Suit::Clubs),
            'd' => Ok(Suit::Diamonds),
            'h' => Ok(Suit::Hearts),
            's' => Ok(Suit::Spades),
            _ => Err(DucyError::InvalidSuit),
        }
    }
}

impl Display for Suit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Suit::Hearts => write!(f, "h"),
            Suit::Diamonds => write!(f, "d"),
            Suit::Clubs => write!(f, "c"),
            Suit::Spades => write!(f, "s"),
        }
    }
}

/// Represents a playing card with a rank and suit.
/// Provides methods to create, parse, and display cards.
#[derive(Clone, Copy, Eq, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(into = "String", try_from = "String"))]
pub struct Card {
    val: Deck,
    suit: Suit,
    rank: Rank,
}

impl Card {
    /// Creates a new `Card` with the specified rank and suit.
    ///
    /// # Arguments
    ///
    /// * `rank` - The rank of the card.
    /// * `suit` - The suit of the card.
    ///
    /// # Returns
    ///
    /// A `Card` instance with the specified rank and suit.
    pub fn new(rank: Rank, suit: Suit) -> Card {
        Card {
            val: Deck::get_card(&rank, &suit),
            rank,
            suit,
        }
    }

    /// Creates a `Card` instance from a `Deck` value, rank, and suit without performing any checks.
    ///
    /// # Arguments
    ///
    /// * `val` - The `Deck` value representing the card.
    /// * `rank` - The rank of the card.
    /// * `suit` - The suit of the card.
    ///
    /// # Returns
    ///
    /// A `Card` instance with the specified `Deck` value, rank, and suit.
    fn from_deck_unchecked(val: Deck, rank: Rank, suit: Suit) -> Self {
        Self { val, rank, suit }
    }

    /// Returns the `Deck` value associated with this card.
    ///
    /// # Returns
    ///
    /// The `Deck` value representing this card.
    pub fn get_deck(&self) -> Deck {
        self.val
    }

    /// Parses a card from a two-character string like `"Ah"` (Ace of Hearts).
    pub fn parse(val: &str) -> Result<Self, DucyError> {
        let trimmed = val.trim();
        if trimmed.len() < 2 {
            return Err(DucyError::InvalidCard);
        }
        let mut chars = val.chars();
        let rank: char = chars.next().ok_or(DucyError::InvalidCard)?;
        let suit = chars.next().ok_or(DucyError::InvalidCard)?;
        Ok(Self::new(
            Rank::try_from_char(&rank)?,
            Suit::try_from_char(&suit)?,
        ))
    }
    /// Returns an iterator over all possible `Card` instances.
    ///
    /// # Returns
    ///
    /// An iterator that yields every possible card in a standard deck.
    pub fn values() -> impl Iterator<Item = Card> {
        CardIterator::new()
    }
}

impl Display for Card {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.rank(), self.suit())
    }
}

impl FromStr for Card {
    type Err = DucyError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Card {
    /// Returns the rank of the card.
    pub fn rank(&self) -> Rank {
        self.rank
    }

    /// Returns the suit of the card.
    pub fn suit(&self) -> Suit {
        self.suit
    }
}

impl From<Card> for String {
    fn from(card: Card) -> Self {
        card.to_string()
    }
}

impl TryFrom<String> for Card {
    type Error = DucyError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Card::parse(&s)
    }
}

/// Represents a deck of playing cards.
/// Able to hold any subset of playing cards, up to one of each card.
/// Provides methods to create, parse, and display decks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(into = "String", try_from = "String"))]
pub struct Deck {
    cards: u64,
}

impl Display for Deck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let card_iterator = DeckCardIterator::new(*self, true);
        let cards_str: Vec<String> = card_iterator.map(|x| format!("{}", x)).collect();
        let cards_as_str = cards_str.join(" ");
        write!(f, "{}", cards_as_str)
    }
}

impl FromStr for Deck {
    type Err = DucyError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl From<Deck> for String {
    fn from(deck: Deck) -> Self {
        deck.to_string()
    }
}

impl TryFrom<String> for Deck {
    type Error = DucyError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Deck::parse(&s)
    }
}

const SINGLE_SUIT_BITFIELD: u64 = 0b11111111111111;
const SINGLE_SUIT_HIGH_ACE_BITFIELD: u64 = 0b11111111111110;
const SINGLE_SUIT_LOW_ACE_BITFIELD: u64 = 0b01111111111111;
const ALL_CARDS_BITFIELD: u64 = SINGLE_SUIT_BITFIELD
    | SINGLE_SUIT_BITFIELD << 16
    | SINGLE_SUIT_BITFIELD << 32
    | SINGLE_SUIT_BITFIELD << 48;
const ALL_CARDS_NO_LOW_ACES_BITFIELD: u64 = SINGLE_SUIT_HIGH_ACE_BITFIELD
    | SINGLE_SUIT_HIGH_ACE_BITFIELD << 16
    | SINGLE_SUIT_HIGH_ACE_BITFIELD << 32
    | SINGLE_SUIT_HIGH_ACE_BITFIELD << 48;
const SINGLE_RANK_BITFIELD: u64 = 0b00010000000000000;
const SINGLE_RANK_FILTER: u64 = SINGLE_RANK_BITFIELD
    | SINGLE_RANK_BITFIELD << 16
    | SINGLE_RANK_BITFIELD << 32
    | SINGLE_RANK_BITFIELD << 48;

const RANKS: [Rank; 14] = [
    Rank::Ace,
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

const RANK_BITS: [u64; 13] = [
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

const SUITS: [Suit; 4] = [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades];

impl Deck {
    /// Creates an empty deck with no cards.
    pub fn empty() -> Self {
        Self { cards: 0 }
    }

    /// Creates a deck containing all possible cards.
    pub fn all_cards() -> Self {
        Self {
            cards: ALL_CARDS_BITFIELD,
        }
    }

    /// Parses a string representation of a deck and returns a `Deck` instance if successful.
    /// # Arguments
    ///
    /// * `val` - A string representation of the deck, with cards separated by spaces.
    ///
    /// # Returns
    ///
    /// A `Result` containing the `Deck` instance if parsing is successful, or an error message if parsing fails.
    pub fn parse(val: &str) -> Result<Self, DucyError> {
        let owned_cards: Vec<Card> = val
            .split(' ')
            .map(Card::parse)
            .collect::<Result<Vec<Card>, DucyError>>()?;

        let mut empty = Self::empty();
        empty.insert_cards(owned_cards.iter());
        Ok(empty)
    }

    /// Creates a `Deck` containing a single specified card.
    ///
    /// # Arguments
    ///
    /// * `rank` - The rank of the card.
    /// * `suit` - The suit of the card.
    ///
    /// # Returns
    ///
    /// A `Deck` instance containing only the specified card.
    pub fn get_card(rank: &Rank, suit: &Suit) -> Self {
        Self {
            cards: Self::get_bits_for_card(rank, suit),
        }
    }

    /// Checks if the deck contains the specified card.
    pub fn has_card(&self, card: &Card) -> bool {
        card.get_deck().cards & self.cards > 0
    }

    /// Checks if the deck contains the specified cards.
    pub fn has_cards(&self, deck: &Deck) -> bool {
        deck.cards & self.cards == deck.cards
    }

    /// Inserts multiple cards into the deck.
    ///
    /// # Arguments
    ///
    /// * `cards` - An iterator over the cards to be inserted into the deck.
    pub fn insert_cards<'a>(&mut self, cards: impl Iterator<Item = &'a Card>) {
        for card in cards {
            self.cards |= card.get_deck().cards
        }
    }

    /// Removes multiple cards from the deck.
    ///
    /// # Arguments
    ///
    /// * `cards` - An iterator over the cards to be removed from the deck.
    ///
    /// # Returns
    ///
    /// None. The deck is modified in place.
    pub fn remove_cards(&mut self, cards: impl Iterator<Item = Card>) {
        for card in cards {
            self.cards &= !card.get_deck().cards;
        }
    }

    /// Removes the nth card from the deck with checking bounds.
    /// # Arguments
    ///
    /// * `index` - The index of the card to be removed.
    ///
    /// # Returns
    ///
    /// A `Result` containing the removed `Card` if successful, or an error message if the index is out of bounds.
    fn try_remove_nth_card(&mut self, index: usize) -> Result<Card, DucyError> {
        let num_cards = self.num_cards();
        if index >= num_cards as usize {
            return Err(DucyError::NotEnoughCards);
        }
        Ok(self.remove_nth_card_unchecked(index))
    }

    /// Returns an iterator over the cards in the deck.
    ///
    /// # Arguments
    ///
    /// * `rank_first` - If true, the iterator will prioritize ranks over suits.
    pub fn iter(self, rank_first: bool) -> impl Iterator<Item = Card> {
        DeckCardIterator::new(self, rank_first)
    }

    /// Returns the nth card from the deck if it exists.
    ///
    /// # Arguments
    ///
    /// * `index` - The index of the card to retrieve.
    ///
    /// # Returns
    ///
    /// An `Option` containing the `Card` if it exists, or `None` if the index is out of bounds.
    ///
    pub fn try_get_nth_card(&self, index: usize) -> Option<Card> {
        let num_cards = self.num_cards();
        if index >= num_cards as usize {
            None
        } else {
            Some(self.get_nth_card_unchecked(index))
        }
    }

    /// Attempts to remove a specified number of random cards from the deck.
    ///
    /// # Arguments
    ///
    /// * `number_to_remove` - The number of random cards to remove from the deck.
    ///
    /// # Returns
    ///
    /// A `Result` containing a vector of the removed `Card`s if successful, or an error message if there are not enough cards in the deck.
    pub fn try_remove_random_cards(
        &mut self,
        number_to_remove: u32,
    ) -> Result<Vec<Card>, DucyError> {
        let mut num_cards = self.num_cards();
        let mut cards = vec![];

        if number_to_remove > num_cards {
            return Err(DucyError::NotEnoughCards);
        }
        for _ in 0..number_to_remove {
            let index = rand::random_range(0..num_cards);
            cards.push(self.try_remove_nth_card(index as usize)?);
            num_cards -= 1;
        }
        Ok(cards)
    }

    /// Returns `true` if the deck is empty, `false` otherwise.
    pub fn is_empty(&self) -> bool {
        self.cards == 0
    }

    /// Returns the number of cards currently in the deck.
    pub fn num_cards(&self) -> u32 {
        let all_ranks = Self::get_without_low_aces(self.cards);
        u64::count_ones(all_ranks)
    }

    /// Returns `true` if all cards in the deck belong to a single suit.
    pub fn is_single_suit(&self) -> bool {
        self.single_suit_index().is_some()
    }

    /// Returns the suit index (0-3) if all cards share one suit, or `None`.
    pub fn single_suit_index(&self) -> Option<usize> {
        let no_low = Self::get_without_low_aces(self.cards);
        if no_low == 0 {
            return None;
        }
        for i in 0..4 {
            let mask = SINGLE_SUIT_HIGH_ACE_BITFIELD << (16 * i);
            if no_low & mask == no_low {
                return Some(i);
            }
        }
        None
    }

    /// Returns `true` if all cards belong to the suit at the given index (0-3).
    pub fn all_in_suit_index(&self, suit_index: usize) -> bool {
        let no_low = Self::get_without_low_aces(self.cards);
        if no_low == 0 {
            return false;
        }
        let mask = SINGLE_SUIT_HIGH_ACE_BITFIELD << (16 * suit_index);
        no_low & mask == no_low
    }

    /// Returns `true` if any rank appears more than once.
    pub fn has_rank_pair(&self) -> bool {
        self.get_combined_ranks().num_unique_ranks() < self.num_cards()
    }

    /// Returns an iterator over for each suit in the deck that returns a set of which ranks are present in each suit.  
    pub fn get_single_suit_ranks(&self) -> impl Iterator<Item = (RankSet, Suit)> {
        SingleSuitRankIterator {
            deck: *self,
            suit_index: 0,
        }
    }

    /// Returns which ranks are present for the specified suit index.
    /// Used to obtain a combined view of all the ranks in the deck, regardless of suit.
    pub fn get_combined_ranks(&self) -> RankSet {
        RankSet {
            ranks: self.get_combined_ranks_bits(),
        }
    }

    /// Returns the count of each rank in the deck as a `RankCount` object.
    pub fn get_rank_count(&self) -> RankCount {
        RankCount {
            rank_counts: self.get_rank_counts(),
        }
    }

    /// Returns an iterator over all possible combinations of the specified number of cards from the deck.
    /// # Arguments
    ///
    /// * `num_cards` - The number of cards to include in each combination.
    ///
    /// # Returns
    ///
    /// An iterator over all possible combinations of the specified number of cards from the deck.
    pub fn enumerate_combinations(self, num_cards: usize) -> impl Iterator<Item = Deck> {
        DeckIterator::new(self, num_cards)
    }

    fn get_single_suit_card_ranks(&self, i: usize) -> u64 {
        self.cards >> (16 * i) & SINGLE_SUIT_BITFIELD
    }

    fn get_combined_ranks_bits(&self) -> u64 {
        // combine the deck for all suits
        let mut ranks_combined = 0;
        for i in 0..4 {
            ranks_combined |= self.get_single_suit_card_ranks(i);
        }
        ranks_combined
    }

    fn get_nth_card_unchecked(&self, index: usize) -> Card {
        self.iter(true)
            .nth(index)
            .expect("index out of bounds in get_nth_card_unchecked")
    }

    fn get_rank_counts(&self) -> [u32; 13] {
        let mut counts = [0; 13];
        (0..13).for_each(|i| {
            let filtered = self.cards & (SINGLE_RANK_FILTER >> i);
            counts[i] = filtered.count_ones();
        });
        counts
    }

    fn get_without_low_aces(cards: u64) -> u64 {
        cards & ALL_CARDS_NO_LOW_ACES_BITFIELD
    }

    fn remove_nth_card_unchecked(&mut self, index: usize) -> Card {
        let card = self.get_nth_card_unchecked(index);
        self.remove_cards([card].into_iter());
        card
    }

    fn get_bits_for_card(rank: &Rank, suit: &Suit) -> u64 {
        let rank_bits: u64 = match rank {
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
        let suit_shift = match suit {
            Suit::Clubs => 0,
            Suit::Diamonds => 16,
            Suit::Hearts => 32,
            Suit::Spades => 48,
        };
        rank_bits << suit_shift
    }
}

struct DeckCardIterator {
    suit_index: usize,
    rank_index: usize,
    rank_first: bool,
    deck: Deck,
}

impl DeckCardIterator {
    fn new(deck: Deck, rank_first: bool) -> Self {
        Self {
            suit_index: 0,
            rank_index: 0,
            rank_first,
            deck,
        }
    }
}

impl Iterator for DeckCardIterator {
    type Item = Card;

    fn next(&mut self) -> Option<Self::Item> {
        if self.suit_index >= 4 || self.rank_index >= 13 {
            None
        } else {
            let bit_to_check = RANK_BITS[12 - self.rank_index] << (16 * self.suit_index);
            let card = if self.deck.cards & bit_to_check == bit_to_check {
                Some(Card::from_deck_unchecked(
                    Deck {
                        cards: bit_to_check,
                    },
                    RANKS[13 - self.rank_index],
                    SUITS[self.suit_index],
                ))
            } else {
                None
            };

            if self.rank_first {
                self.suit_index = (self.suit_index + 1) % 4;
                if self.suit_index == 0 {
                    self.rank_index += 1;
                }
            } else {
                self.rank_index = (self.rank_index + 1) % 13;
                if self.rank_index == 0 {
                    self.suit_index += 1;
                }
            }

            if card.is_some() { card } else { self.next() }
        }
    }
}

struct SingleSuitRankIterator {
    deck: Deck,
    suit_index: usize,
}

impl Iterator for SingleSuitRankIterator {
    type Item = (RankSet, Suit);

    fn next(&mut self) -> Option<Self::Item> {
        if self.suit_index == 4 {
            None
        } else {
            let ranks = self.deck.get_single_suit_card_ranks(self.suit_index);
            let suit = SUITS[self.suit_index];
            self.suit_index += 1;
            Some((RankSet { ranks }, suit))
        }
    }
}

/// A bitfield representing which ranks are present in a suit or combined view.
pub struct RankSet {
    ranks: u64,
}

impl RankSet {
    /// Returns the number of unique ranks present.
    pub fn num_unique_ranks(&self) -> u32 {
        (self.ranks & SINGLE_SUIT_LOW_ACE_BITFIELD).count_ones()
    }

    /// Returns the highest five ranks present, if there are at least five unique ranks.
    pub fn get_highest_five(&self, rank_order: &RankOrder) -> Option<[Rank; 5]> {
        let all_ranks = self.ranks
            & match rank_order {
                RankOrder::AceIsHigh => SINGLE_SUIT_HIGH_ACE_BITFIELD,
                RankOrder::AceIsLow => SINGLE_SUIT_LOW_ACE_BITFIELD,
            };

        if u64::count_ones(all_ranks) < 5 {
            None
        } else {
            let mut result = [Rank::Two; 5];
            let mut found = 0;
            for i in 1..14 {
                let target = 0b1 << (14 - i);
                if target & all_ranks == target {
                    result[found] = RANKS[14 - i];
                    found += 1;
                    if found == 5 {
                        break;
                    }
                }
            }
            Some(result)
        }
    }

    /// Checks if the rank set matches a specific bit pattern.
    ///
    /// `required_on_bits` specifies which bits must be set.
    /// `field_length` specifies the length of the bit field to check.
    /// Returns the highest rank that matches the pattern, if any.
    pub fn matches_pattern(&self, required_on_bits: u64, field_length: usize) -> Option<Rank> {
        for i in 0..(15 - field_length) {
            let must_match = required_on_bits << (14 - field_length - i);
            if must_match & self.ranks == must_match {
                return Some(RANKS[13 - i]);
            }
        }
        None
    }
}

/// Counts of each rank present in a deck.
pub struct RankCount {
    rank_counts: [u32; 13],
}

impl RankCount {
    /// Returns the highest rank that appears at least `target_count` times, excluding specified ranks.
    pub fn find_highest_with_n(
        &self,
        ranks_to_exclude: &[Rank],
        target_count: u32,
    ) -> Option<Rank> {
        for (index, val) in self.rank_counts.iter().enumerate() {
            if ranks_to_exclude.contains(&RANKS[13 - index]) {
                continue;
            } else if *val >= target_count {
                return Some(RANKS[13 - index]);
            }
        }
        None
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

impl BitOrAssign for Deck {
    fn bitor_assign(&mut self, rhs: Self) {
        self.cards |= rhs.cards
    }
}

impl BitOrAssign<Card> for Deck {
    fn bitor_assign(&mut self, rhs: Card) {
        self.cards |= rhs.val.cards
    }
}

impl Sub for Deck {
    type Output = Deck;

    fn sub(self, rhs: Self) -> Self::Output {
        let cards = self.cards & !(rhs.cards);
        Deck { cards }
    }
}

impl Sub for &Deck {
    type Output = Deck;

    fn sub(self, rhs: Self) -> Self::Output {
        let cards = self.cards & !(rhs.cards);
        Deck { cards }
    }
}

impl SubAssign for Deck {
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}
impl<'a> SubAssign<&'a Deck> for Deck {
    fn sub_assign(&mut self, rhs: &'a Deck) {
        self.cards &= !rhs.cards;
    }
}

impl From<u64> for Deck {
    fn from(value: u64) -> Self {
        Deck { cards: value }
    }
}

impl From<Deck> for u64 {
    fn from(val: Deck) -> Self {
        val.cards
    }
}

struct CardIterator {
    inner: DeckCardIterator,
}

impl CardIterator {
    pub fn new() -> Self {
        Self {
            inner: DeckCardIterator::new(Deck::all_cards(), true),
        }
    }
}

impl Iterator for CardIterator {
    type Item = Card;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

struct DeckIterator {
    card_bits: [u64; 52],
    indices: [usize; 5],
    n: usize,
    k: usize,
    done: bool,
}

impl DeckIterator {
    fn new(deck: Deck, size: usize) -> Self {
        debug_assert!(size <= 5);
        let mut card_bits = [0u64; 52];
        let mut n = 0;
        for suit_idx in 0..4usize {
            let shift = suit_idx * 16;
            for rank_bits in &RANK_BITS {
                let bits = rank_bits << shift;
                if deck.cards & bits == bits {
                    card_bits[n] = bits;
                    n += 1;
                }
            }
        }
        let done = size == 0 || size > n;
        let mut indices = [0usize; 5];
        for (i, idx) in indices.iter_mut().enumerate().take(size.min(5)) {
            *idx = i;
        }
        Self {
            card_bits,
            indices,
            n,
            k: size,
            done,
        }
    }
}

impl Iterator for DeckIterator {
    type Item = Deck;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let mut bits = 0u64;
        for idx in 0..self.k {
            bits |= self.card_bits[self.indices[idx]];
        }
        let mut i = self.k;
        while i > 0 {
            i -= 1;
            if self.indices[i] != i + self.n - self.k {
                break;
            }
            if i == 0 {
                self.done = true;
                return Some(Deck::from(bits));
            }
        }
        self.indices[i] += 1;
        for j in (i + 1)..self.k {
            self.indices[j] = self.indices[j - 1] + 1;
        }
        Some(Deck::from(bits))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test_util::deck_from_cards;

    #[test]
    pub fn test_has_card() {
        let mut empty = Deck::empty();
        let mut full = Deck::all_cards();

        for card in Card::values() {
            assert!(!empty.has_card(&card));
            assert!(full.has_card(&card))
        }

        let two_clubs = Card::new(Rank::Two, Suit::Clubs);
        let three_clubs = Card::new(Rank::Three, Suit::Clubs);

        empty.insert_cards([two_clubs].iter());
        full.remove_cards([three_clubs].into_iter());
        assert!(empty.has_card(&two_clubs));
        assert!(!empty.has_card(&three_clubs));
        assert!(!full.has_card(&three_clubs));
        assert!(full.has_card(&two_clubs));

        let card = empty.try_remove_nth_card(0).unwrap();
        assert_eq!(card, two_clubs);
        assert!(!empty.has_card(&two_clubs));

        println!("{}", full);
        let card = full.try_remove_nth_card(3).unwrap();
        assert_eq!(card, Card::parse("As").unwrap());
        assert_eq!(full.num_cards(), 50);
        let card = full.try_remove_nth_card(10).unwrap();
        assert_eq!(card, Card::parse("Qs").unwrap());
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
                unreachable!();
            }
        }
    }

    #[test]
    pub fn create_card_works() {
        let card = Card::new(Rank::Ace, Suit::Spades);
        assert_eq!(Rank::Ace, card.rank());
        assert_eq!(Suit::Spades, card.suit());
    }

    #[test]
    pub fn test_display_for_card() {
        let card = Card::new(Rank::Ace, Suit::Spades);
        assert_eq!(format!("{card}"), "As");
    }

    #[test]
    pub fn test_from_str() -> Result<(), DucyError> {
        for i in 0..52 {
            let card = Deck::all_cards().try_get_nth_card(i).unwrap();
            let display: String = format!("{}", card);
            let other_card = Card::parse(&display)?;
            assert_eq!(other_card, card);
        }

        assert_eq!(Card::parse("As")?, Card::new(Rank::Ace, Suit::Spades));

        Ok(())
    }

    #[test]
    pub fn test_card_fromstr_trait() {
        let card: Card = "As".parse().unwrap();
        assert_eq!(card, Card::new(Rank::Ace, Suit::Spades));
        assert!("xx".parse::<Card>().is_err());
    }

    #[test]
    pub fn test_deck_fromstr_trait() {
        let deck: Deck = "As Kh".parse().unwrap();
        assert_eq!(deck, Deck::parse("As Kh").unwrap());
        assert_eq!(deck.num_cards(), 2);
    }

    #[test]
    pub fn test_parse_invalid_rank() {
        assert_eq!(
            Rank::try_from_char(&'x'),
            Err(crate::error::DucyError::InvalidRank)
        );
        assert_eq!(
            Rank::try_from_char(&'1'),
            Err(crate::error::DucyError::InvalidRank)
        );
    }

    #[test]
    pub fn test_parse_invalid_suit() {
        assert_eq!(
            Suit::try_from_char(&'x'),
            Err(crate::error::DucyError::InvalidSuit)
        );
        assert_eq!(
            Suit::try_from_char(&'a'),
            Err(crate::error::DucyError::InvalidSuit)
        );
    }

    #[test]
    pub fn test_parse_invalid_card() {
        assert!(Card::parse("").is_err());
        assert!(Card::parse("A").is_err());
        assert!(Card::parse("Xc").is_err());
        assert!(Card::parse("Ax").is_err());
    }

    #[test]
    pub fn test_parse_invalid_deck() {
        assert!(Deck::parse("As Xc").is_err());
        assert!(Deck::parse("not_cards").is_err());
    }

    #[test]
    pub fn test_deck_num_cards() {
        assert_eq!(Deck::empty().num_cards(), 0);
        assert_eq!(Deck::all_cards().num_cards(), 52);
        assert_eq!(deck_from_cards("As Ks Qs").num_cards(), 3);
    }

    #[test]
    pub fn test_deck_is_empty() {
        assert!(Deck::empty().is_empty());
        assert!(!Deck::all_cards().is_empty());
    }

    #[test]
    pub fn test_deck_subtraction() {
        let full = Deck::all_cards();
        let aces = deck_from_cards("As Ah Ad Ac");
        let remaining = full - aces;
        assert_eq!(remaining.num_cards(), 48);
        assert!(!remaining.has_cards(&aces));
    }

    #[test]
    pub fn test_deck_bitor() {
        let d1 = deck_from_cards("As Ks");
        let d2 = deck_from_cards("Qs Js");
        let combined = d1 | d2;
        assert_eq!(combined.num_cards(), 4);
        assert!(combined.has_cards(&d1));
        assert!(combined.has_cards(&d2));
    }

    #[test]
    pub fn test_try_remove_nth_card_out_of_bounds() {
        let mut deck = deck_from_cards("As Ks");
        assert!(deck.try_remove_nth_card(5).is_err());
    }

    #[test]
    pub fn test_try_remove_random_cards() {
        let mut deck = Deck::all_cards();
        let removed = deck.try_remove_random_cards(5).unwrap();
        assert_eq!(removed.len(), 5);
        assert_eq!(deck.num_cards(), 47);
        for card in &removed {
            assert!(!deck.has_card(card));
        }
    }

    #[test]
    pub fn test_try_remove_random_cards_too_many() {
        let mut deck = deck_from_cards("As Ks");
        assert!(deck.try_remove_random_cards(3).is_err());
    }

    #[test]
    pub fn test_remove_cards_does_not_add() {
        let mut deck = deck_from_cards("As Ks");
        let three_clubs = Card::new(Rank::Three, Suit::Clubs);
        assert!(!deck.has_card(&three_clubs));
        deck.remove_cards([three_clubs].into_iter());
        assert!(!deck.has_card(&three_clubs));
        assert_eq!(deck.num_cards(), 2);
    }

    #[test]
    pub fn test_try_get_nth_card_at_boundary() {
        let deck = deck_from_cards("As Ks");
        assert!(deck.try_get_nth_card(0).is_some());
        assert!(deck.try_get_nth_card(1).is_some());
        assert!(deck.try_get_nth_card(2).is_none());
    }

    #[cfg(feature = "serde")]
    #[test]
    pub fn test_serde_card_roundtrip() {
        let card = Card::new(Rank::Ace, Suit::Spades);
        let json = serde_json::to_string(&card).unwrap();
        assert_eq!(json, "\"As\"");
        let deserialized: Card = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, card);
    }

    #[cfg(feature = "serde")]
    #[test]
    pub fn test_serde_deck_roundtrip() {
        let deck = deck_from_cards("As Kh Qd");
        let json = serde_json::to_string(&deck).unwrap();
        let deserialized: Deck = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, deck);
    }

    #[cfg(feature = "serde")]
    #[test]
    pub fn test_serde_rank_roundtrip() {
        let rank = Rank::Ace;
        let json = serde_json::to_string(&rank).unwrap();
        let deserialized: Rank = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, rank);
    }
}
