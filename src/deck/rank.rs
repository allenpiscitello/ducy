use std::fmt::Display;

use strum_macros::EnumIter;
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, Hash)]
pub enum Rank {
    Ace,
    Two,
    Three,
    Four,
    Five,
    Six,
    Seven,
    Eight,
    Nine,
    Ten,
    Jack,
    Queen,
    King,
}

impl Rank {
    pub fn try_from_usize(val: usize) -> Result<Self, String> {
        match val {
            0 => Ok(Rank::Two),
            1 => Ok(Rank::Three),
            2 => Ok(Rank::Four),
            3 => Ok(Rank::Five),
            4 => Ok(Rank::Six),
            5 => Ok(Rank::Seven),
            6 => Ok(Rank::Eight),
            7 => Ok(Rank::Nine),
            8 => Ok(Rank::Ten),
            9 => Ok(Rank::Jack),
            10 => Ok(Rank::Queen),
            11 => Ok(Rank::King),
            12 => Ok(Rank::Ace),
            _ => Err("Invalid value".to_owned()),
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

#[derive(Eq, PartialEq, Debug)]
pub struct RankBitfield {
    pub ranks: u32,
}

impl RankBitfield {
    pub fn from_rank(rank: &Rank) -> RankBitfield {
        Self {
            ranks: match rank {
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
            },
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    pub fn test_rank_try_from_usize_works() {
        for i in 0..13 {
            let rank = Rank::try_from_usize(i);
            assert!(rank.is_ok())
        }
    }

    #[test]
    pub fn test_rank_get_bitfield() {
        assert_eq!(
            RankBitfield {
                ranks: 0b10000000000001
            },
            RankBitfield::from_rank(&Rank::Ace)
        );
        assert_eq!(
            RankBitfield { ranks: 0b10 },
            RankBitfield::from_rank(&Rank::Two)
        );
        assert_eq!(
            RankBitfield { ranks: 0b100 },
            RankBitfield::from_rank(&Rank::Three)
        );
    }
}
