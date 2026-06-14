use std::fmt::Display;

use crate::deck::rank::Rank;
use crate::deck::suit::Suit;

pub trait Cardlike {
    fn rank(&self) -> Rank;
    fn suit(&self) -> Suit;
}

pub struct Card<T: Cardlike>(pub T);

impl<T: Cardlike> Display for Card<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.0.rank(), self.0.suit())
    }
}

#[derive(Clone, Copy, Eq, PartialEq, Debug)]
pub struct SimpleCard {
    rank: Rank,
    suit: Suit,
}

impl SimpleCard {
    pub fn new(rank: Rank, suit: Suit) -> SimpleCard {
        return SimpleCard { rank, suit };
    }

    pub fn try_from_usize(val: usize) -> Result<Self, String> {
        Ok(Self {
            rank: Rank::try_from_usize(val % 13)?,
            suit: Suit::try_from_usize(val / 13)?,
        })
    }

    pub fn try_from_str(val: &str) -> Result<Self, String> {
        let trimmed = val.trim();
        if trimmed.len() < 2 {
            return Err("Invalid Value".to_owned());
        }
        let mut chars = val.chars();
        let rank = chars.next().ok_or("Invalid value".to_owned())?;
        let suit = chars.next().ok_or("Invalid value".to_owned())?;
        Ok(Self {
            rank: Rank::try_from_str(&rank)?,
            suit: Suit::try_from_str(&suit)?,
        })
    }
}

impl Display for dyn Cardlike + '_ {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.rank(), self.suit())
    }
}

impl Cardlike for SimpleCard {
    fn rank(&self) -> Rank {
        self.rank
    }

    fn suit(&self) -> Suit {
        self.suit
    }
}

#[cfg(test)]
pub mod test {
    use super::*;

    #[test]
    pub fn create_card_works() {
        let card = SimpleCard::new(Rank::Ace, Suit::Spades);
        assert_eq!(Rank::Ace, card.rank);
        assert_eq!(Suit::Spades, card.suit);
    }

    #[test]
    pub fn test_card_try_from_usize_works() {
        for i in 0..52 {
            let card = SimpleCard::try_from_usize(i);
            assert!(card.is_ok())
        }
    }

    #[test]
    pub fn test_display_for_card() {
        let card = Card(SimpleCard::new(Rank::Ace, Suit::Spades));
        assert_eq!(format!("{card}"), "As");
    }

    #[test]
    pub fn test_from_str() -> Result<(), String> {
        for i in 0..52 {
            let card = SimpleCard::try_from_usize(i)?;
            let display: String = format!("{}", Card(card));
            let other_card = SimpleCard::try_from_str(&display)?;
            assert_eq!(other_card, card);
        }

        assert_eq!(
            SimpleCard::try_from_str("As")?,
            SimpleCard::new(Rank::Ace, Suit::Spades)
        );

        Ok(())
    }
}
