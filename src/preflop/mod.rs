//! Preflop tools: the 169 starting-hand classes, a class-vs-class all-in
//! equity table, and a heads-up push/fold equilibrium solver.

mod equity_table;
mod push_fold;

pub use equity_table::PreflopEquityTable;
pub use push_fold::{PushFoldSolution, solve_heads_up_push_fold};

use std::fmt;
use std::str::FromStr;

use strum::IntoEnumIterator;

use crate::deck::{Card, Deck, Rank, Suit};
use crate::error::DucyError;

/// One of the 169 Hold'em starting-hand classes: a pair (`TT`), a suited
/// hand (`AKs`) or an offsuit hand (`AKo`).
///
/// Index layout: with `hi >= lo` as rank indices (Two = 0 ... Ace = 12),
/// pairs are `hi * 13 + hi`, suited hands `hi * 13 + lo`, and offsuit hands
/// `lo * 13 + hi`, so the 13x13 grid matches the usual range chart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HandClass(u8);

impl HandClass {
    /// Number of starting-hand classes.
    pub const COUNT: usize = 169;

    /// Every class, in index order.
    pub fn all() -> impl Iterator<Item = HandClass> {
        (0..Self::COUNT as u8).map(HandClass)
    }

    /// The class at `index`, if it is below 169.
    pub fn from_index(index: usize) -> Option<HandClass> {
        (index < Self::COUNT).then_some(HandClass(index as u8))
    }

    /// Position in `0..169`.
    pub fn index(self) -> usize {
        self.0 as usize
    }

    fn from_ranks(hi: Rank, lo: Rank, suited: bool) -> HandClass {
        let (h, l) = (hi as u8, lo as u8);
        let (h, l) = if h >= l { (h, l) } else { (l, h) };
        if h == l || suited {
            HandClass(h * 13 + l)
        } else {
            HandClass(l * 13 + h)
        }
    }

    /// The class of a 2-card hand, or `None` if `hand` isn't exactly 2 cards.
    pub fn from_hand(hand: Deck) -> Option<HandClass> {
        let cards: Vec<Card> = hand.iter(false).collect();
        let [a, b] = cards[..] else { return None };
        Some(Self::from_ranks(a.rank(), b.rank(), a.suit() == b.suit()))
    }

    /// Higher and lower rank (equal for pairs).
    pub fn ranks(self) -> (Rank, Rank) {
        let (r, c) = (self.0 / 13, self.0 % 13);
        let rank = |i: u8| Rank::iter().nth(i as usize).unwrap();
        (rank(r.max(c)), rank(r.min(c)))
    }

    /// Whether this is a pocket pair.
    pub fn is_pair(self) -> bool {
        self.0 / 13 == self.0 % 13
    }

    /// Whether this is a suited (non-pair) hand.
    pub fn is_suited(self) -> bool {
        self.0 / 13 > self.0 % 13
    }

    /// Number of 2-card combos in this class: 6 pairs, 4 suited, 12 offsuit.
    pub fn combo_count(self) -> u32 {
        if self.is_pair() {
            6
        } else if self.is_suited() {
            4
        } else {
            12
        }
    }

    /// Every 2-card combo in this class.
    pub fn combos(self) -> Vec<Deck> {
        let (hi, lo) = self.ranks();
        let suits: Vec<Suit> = Suit::iter().collect();
        let mut out = Vec::with_capacity(self.combo_count() as usize);
        for (i, &s1) in suits.iter().enumerate() {
            for (j, &s2) in suits.iter().enumerate() {
                let keep = if self.is_pair() {
                    i < j
                } else if self.is_suited() {
                    i == j
                } else {
                    i != j
                };
                if keep {
                    let mut deck = Deck::empty();
                    deck.insert_cards([Card::new(hi, s1), Card::new(lo, s2)].iter());
                    out.push(deck);
                }
            }
        }
        out
    }
}

impl fmt::Display for HandClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (hi, lo) = self.ranks();
        if self.is_pair() {
            write!(f, "{hi}{lo}")
        } else if self.is_suited() {
            write!(f, "{hi}{lo}s")
        } else {
            write!(f, "{hi}{lo}o")
        }
    }
}

impl FromStr for HandClass {
    type Err = DucyError;

    /// Parses `"TT"`, `"AKs"` or `"AKo"` (either rank order).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let chars: Vec<char> = s.trim().chars().collect();
        let (a, b) = match chars[..] {
            [a, b] | [a, b, _] => (Rank::try_from_char(&a)?, Rank::try_from_char(&b)?),
            _ => return Err(DucyError::InvalidRange),
        };
        match (chars.get(2).map(|c| c.to_ascii_lowercase()), a == b) {
            (None, true) => Ok(Self::from_ranks(a, b, false)),
            (Some('s'), false) => Ok(Self::from_ranks(a, b, true)),
            (Some('o'), false) => Ok(Self::from_ranks(a, b, false)),
            _ => Err(DucyError::InvalidRange),
        }
    }
}

/// Number of (a, b) combo pairs from classes `a` and `b` that share no card.
pub(crate) fn compatible_pairs(a: HandClass, b: HandClass) -> u32 {
    let bs = b.combos();
    a.combos()
        .iter()
        .map(|x| {
            bs.iter()
                .filter(|y| u64::from(*x) & u64::from(**y) == 0)
                .count() as u32
        })
        .sum()
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_class_layout_and_names() {
        let names: Vec<String> = HandClass::all().map(|c| c.to_string()).collect();
        assert_eq!(names.len(), 169);
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(unique.len(), 169);
        let total: u32 = HandClass::all().map(|c| c.combo_count()).sum();
        assert_eq!(total, 1326);
        for c in HandClass::all() {
            assert_eq!(c.combos().len() as u32, c.combo_count());
            assert_eq!(c.to_string().parse::<HandClass>().unwrap(), c);
            for combo in c.combos() {
                assert_eq!(HandClass::from_hand(combo), Some(c));
            }
        }
        assert_eq!("AKs".parse::<HandClass>().unwrap().to_string(), "AKs");
        assert_eq!(
            "ka o"
                .replace(' ', "")
                .parse::<HandClass>()
                .unwrap()
                .to_string(),
            "AKo"
        );
        assert!("AAs".parse::<HandClass>().is_err());
        assert!("AK".parse::<HandClass>().is_err());
    }

    #[test]
    fn test_compatible_pairs() {
        let aa: HandClass = "AA".parse().unwrap();
        let kk: HandClass = "KK".parse().unwrap();
        let aks: HandClass = "AKs".parse().unwrap();
        assert_eq!(compatible_pairs(aa, kk), 36);
        assert_eq!(compatible_pairs(aa, aa), 6);
        // Each AA combo blocks the two suited AK combos that use its aces.
        assert_eq!(compatible_pairs(aa, aks), 6 * 2);
    }
}
