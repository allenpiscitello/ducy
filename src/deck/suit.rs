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
    pub fn try_from_char(val: &char) -> Result<Self, String> {
        match val.to_ascii_lowercase() {
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
