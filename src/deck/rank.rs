use std::fmt::Display;

use strum_macros::EnumIter;
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, Hash, PartialOrd, Ord)]
pub enum Rank {
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
    Ace,
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

    pub fn try_from_str(val: &char) -> Result<Self, String> {
        match val {
            'A' => Ok(Rank::Ace),
            '2' => Ok(Rank::Two),
            '3' => Ok(Rank::Three),
            '4' => Ok(Rank::Four),
            '5' => Ok(Rank::Five),
            '6' => Ok(Rank::Six),
            '7' => Ok(Rank::Seven),
            '8' => Ok(Rank::Eight),
            '9' => Ok(Rank::Nine),
            'T' => Ok(Rank::Ten),
            'J' => Ok(Rank::Jack),
            'Q' => Ok(Rank::Queen),
            'K' => Ok(Rank::King),
            _ => Err("Invalid value".to_owned()),
        }
    }

    pub fn get_score(&self) -> u32 {
        match self {
            Rank::Two => 1,
            Rank::Three => 2,
            Rank::Four => 3,
            Rank::Five => 4,
            Rank::Six => 5,
            Rank::Seven => 6,
            Rank::Eight => 7,
            Rank::Nine => 8,
            Rank::Ten => 9,
            Rank::Jack => 10,
            Rank::Queen => 11,
            Rank::King => 12,
            Rank::Ace => 13,
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
    pub fn new() -> Self {
        Self { ranks: 0 }
    }

    pub fn from_rank(rank: &Rank) -> Self {
        let mut return_val = RankBitfield::new();
        return_val.add_ranks(&[rank.clone()]);
        return_val
    }

    pub fn add_ranks(&mut self, ranks: &[Rank]) {
        for rank in ranks {
            self.ranks |= match rank {
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
            }
        }
    }

    pub fn get_straight(&self) -> Option<Rank> {
        let result = self.matches_pattern(0b11111, 5);
        result.map(|x| Rank::try_from_usize(x + 2).unwrap())
    }

    pub fn get_highest_five(&self) -> Option<[Rank; 5]> {
        // we get rid of the low aces since aces are always high
        let ranks_to_check = self.ranks & 0b11111111111110;
        let count = u32::count_ones(ranks_to_check);
        if count < 5 {
            None
        } else {
            let mut return_val = vec![];
            for i in 1..13 {
                let target = 0b1 << 15 - i;
                if target & self.ranks == target {
                    return_val.push(Rank::try_from_usize(15 - i - 1).unwrap());
                    if return_val.len() == 5 {
                        break;
                    }
                }
            }
            let slice = return_val.try_into().unwrap();
            Some(slice)
        }
    }

    fn matches_pattern(&self, required_on_bits: u32, field_length: usize) -> Option<usize> {
        for i in 0..(15 - field_length) {
            let must_match = required_on_bits << (14 - field_length - i);
            if must_match & self.ranks == must_match {
                return Some(15 - field_length - i);
            }
        }
        return None;
    }

    // pub fn get_connectedness(&self) -> Connectedness {
    //     let connected_bitfield: u32 = 0b0011100;
    //     if self.matches_pattern(connected_bitfield, 7) {
    //         return Connectedness::FullyConnected;
    //     }

    //     if self.matches_pattern(0b011010, 6) {
    //         return Connectedness::OneGap;
    //     }
    //     if self.matches_pattern(0b010110, 6) {
    //         return Connectedness::OneGap;
    //     }
    //     if self.matches_pattern(0b001110, 6) {
    //         return Connectedness::OneGap;
    //     }
    //     if self.matches_pattern(0b011100, 6) {
    //         return Connectedness::OneGap;
    //     }
    //     if self.matches_pattern(0b11001, 5) {
    //         return Connectedness::TwoGap;
    //     }
    //     if self.matches_pattern(0b10011, 5) {
    //         return Connectedness::TwoGap;
    //     }
    //     if self.matches_pattern(0b10101, 5) {
    //         return Connectedness::TwoGap;
    //     }
    //     if self.matches_pattern(0b00011000, 8) {
    //         return Connectedness::SixCardDraw;
    //     }
    //     if self.matches_pattern(0b0011000, 8) {
    //         return Connectedness::FiveCardDraw;
    //     }
    //     if self.matches_pattern(0b0001100, 8) {
    //         return Connectedness::FiveCardDraw;
    //     }
    //     Connectedness::Disconnected
    // }
}

// Connectedness properties:
// How many straights are possible now?
// How many draws are possible in the next card?
// #[derive(Debug, Eq, PartialEq)]
// pub enum Connectedness {
//     Disconnected,   // No straights possible now or even on next card
//     FiveCardDraw,   // 5 cards make a straight next hand
//     SixCardDraw,    // 6 cards make a straight on next card
//     TwoGap,         // Straight is possible with 1 combo
//     OneGap,         // Straight is possible with 2 combos
//     FullyConnected, // Straight is possible with 3 combos
// }

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
