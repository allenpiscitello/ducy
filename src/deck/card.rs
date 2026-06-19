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
    val: u8,
}

impl SimpleCard {
    fn get_rank_index(rank: Rank) -> u8 {
        match rank {
            Rank::Two => 0,
            Rank::Three => 1,
            Rank::Four => 2,
            Rank::Five => 3,
            Rank::Six => 4,
            Rank::Seven => 5,
            Rank::Eight => 6,
            Rank::Nine => 7,
            Rank::Ten => 8,
            Rank::Jack => 9,
            Rank::Queen => 10,
            Rank::King => 11,
            Rank::Ace => 12,
        }
    }

    fn get_suit_index(suit: Suit) -> u8 {
        match suit {
            Suit::Clubs => 0,
            Suit::Diamonds => 1,
            Suit::Hearts => 2,
            Suit::Spades => 3,
        }
    }

    pub fn new(rank: Rank, suit: Suit) -> SimpleCard {
        return SimpleCard {
            val: SimpleCard::get_suit_index(suit) * 16 + SimpleCard::get_rank_index(rank),
        };
    }

    pub fn try_from_usize(val: usize) -> Result<Self, String> {
        if val % 16 >= 13 {
            return Err("Invalid value".to_owned());
        }
        Ok(Self { val: val as u8 })
    }

    pub fn try_from_str(val: &str) -> Result<Self, String> {
        let trimmed = val.trim();
        if trimmed.len() < 2 {
            return Err("Invalid Value".to_owned());
        }
        let mut chars = val.chars();
        let rank = chars.next().ok_or("Invalid value".to_owned())?;
        let suit = chars.next().ok_or("Invalid value".to_owned())?;
        Ok(Self::new(
            Rank::try_from_str(&rank)?,
            Suit::try_from_str(&suit)?,
        ))
    }
}

impl Display for dyn Cardlike + '_ {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.rank(), self.suit())
    }
}

impl Cardlike for SimpleCard {
    fn rank(&self) -> Rank {
        Rank::try_from_usize((self.val % 16) as usize).unwrap()
    }

    fn suit(&self) -> Suit {
        let suit_val = self.val / 16;
        Suit::try_from_usize(suit_val as usize).unwrap()
    }
}

#[cfg(test)]
pub mod test {
    use super::*;

    #[test]
    pub fn create_card_works() {
        let card = SimpleCard::new(Rank::Ace, Suit::Spades);
        assert_eq!(Rank::Ace, card.rank());
        assert_eq!(Suit::Spades, card.suit());
    }

    #[test]
    pub fn test_card_try_from_usize_works() {
        for i in 0..13 {
            let card = SimpleCard::try_from_usize(i);
            assert!(card.is_ok())
        }
        for i in 16..29 {
            let card = SimpleCard::try_from_usize(i);
            assert!(card.is_ok())
        }
        for i in 32..45 {
            let card = SimpleCard::try_from_usize(i);
            assert!(card.is_ok())
        }
        for i in 48..61 {
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
        for i in 0..64 {
            let result = SimpleCard::try_from_usize(i);
            if let Ok(card) = result {
                let display: String = format!("{}", Card(card));
                let other_card = SimpleCard::try_from_str(&display)?;
                assert_eq!(other_card, card);
            }
        }

        assert_eq!(
            SimpleCard::try_from_str("As")?,
            SimpleCard::new(Rank::Ace, Suit::Spades)
        );

        Ok(())
    }
}
