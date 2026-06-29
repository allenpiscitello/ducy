use std::fmt::Display;

use crate::deck::deck::{CardIterator, Deck};
use crate::deck::rank::Rank;
use crate::deck::suit::Suit;

#[derive(Clone, Copy, Eq, PartialEq, Debug)]
pub struct Card {
    val: Deck,
    suit: Suit,
    rank: Rank,
}

impl Card {
    pub fn new(rank: Rank, suit: Suit) -> Card {
        Card {
            val: Deck::get_card(&rank, &suit),
            rank,
            suit,
        }
    }

    pub(crate) fn from_deck(val: Deck, rank: Rank, suit: Suit) -> Self {
        Self { val, rank, suit }
    }

    pub fn get_deck(&self) -> Deck {
        self.val.clone()
    }

    pub fn parse(val: &str) -> Result<Self, String> {
        let trimmed = val.trim();
        if trimmed.len() < 2 {
            return Err("Invalid Value".to_owned());
        }
        let mut chars = val.chars();
        let rank: char = chars.next().ok_or("Invalid value".to_owned())?;
        let suit = chars.next().ok_or("Invalid value".to_owned())?;
        Ok(Self::new(
            Rank::try_from_char(&rank)?,
            Suit::try_from_char(&suit)?,
        ))
    }

    pub fn values() -> impl Iterator<Item = Card> {
        CardIterator::new()
    }
}

impl Display for Card {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.rank(), self.suit())
    }
}

impl Card {
    pub fn rank(&self) -> Rank {
        self.rank
    }

    pub fn suit(&self) -> Suit {
        self.suit
    }
}

#[cfg(test)]
pub mod test {
    use super::*;

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
    pub fn test_from_str() -> Result<(), String> {
        for i in 0..52 {
            let card = Deck::all_cards().try_get_nth_card(i).unwrap();
            let display: String = format!("{}", card);
            let other_card = Card::parse(&display)?;
            assert_eq!(other_card, card);
        }

        assert_eq!(
            Card::parse("As")?,
            Card::new(Rank::Ace, Suit::Spades)
        );

        Ok(())
    }
}
