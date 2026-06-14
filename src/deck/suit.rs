use std::fmt::Display;

use strum_macros::EnumIter;
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, Hash)]
pub enum Suit {
    Clubs,
    Diamonds,
    Hearts,
    Spades,
}

impl Suit {
    pub fn try_from_usize(val: usize) -> Result<Self, String> {
        match val {
            0 => Ok(Suit::Clubs),
            1 => Ok(Suit::Diamonds),
            2 => Ok(Suit::Hearts),
            3 => Ok(Suit::Spades),
            _ => Err("Invalid value".to_owned()),
        }
    }

    pub fn try_from_str(val: &char) -> Result<Self, String> {
        match val {
            'c' => Ok(Suit::Clubs),
            'd' => Ok(Suit::Diamonds),
            'h' => Ok(Suit::Hearts),
            's' => Ok(Suit::Spades),
            _ => Err("Invalid value".to_owned()),
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

#[cfg(test)]
pub mod test {
    use super::*;

    #[test]
    pub fn test_suit_try_from_usize_works() {
        for i in 0..4 {
            let suit = Suit::try_from_usize(i);
            assert!(suit.is_ok())
        }
    }
}
