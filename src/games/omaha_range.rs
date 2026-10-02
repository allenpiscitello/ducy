use rust_decimal::Decimal;

use crate::{
    deck::{
        Card, Deck, Rank, Suit,
        range::{Range, RangeBase, RangeItem},
    },
    error::DucyError,
};

/// Suit and pair conditions a hand must meet, written after `$` in a term.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Qualifier {
    /// `$ds`: at least two suits with two or more cards each.
    DoubleSuited,
    /// `$ss`: exactly one suit with two or more cards.
    SingleSuited,
    /// `$r`: every card a different suit.
    Rainbow,
    /// `$np`: no two cards share a rank.
    NoPair,
    /// `$1p`: exactly one pair and no trips.
    OnePair,
    /// `$2p`: two pairs.
    TwoPair,
}

impl Qualifier {
    fn parse(s: &str) -> Result<Self, DucyError> {
        match s {
            "ds" => Ok(Self::DoubleSuited),
            "ss" => Ok(Self::SingleSuited),
            "r" => Ok(Self::Rainbow),
            "np" => Ok(Self::NoPair),
            "1p" => Ok(Self::OnePair),
            "2p" => Ok(Self::TwoPair),
            _ => Err(DucyError::InvalidRange),
        }
    }

    fn matches(self, ranks: &[u8; 13], suits: &[u8; 4]) -> bool {
        let suited = suits.iter().filter(|&&c| c >= 2).count();
        let pairs = ranks.iter().filter(|&&c| c == 2).count();
        let trips = ranks.iter().any(|&c| c >= 3);
        match self {
            Self::DoubleSuited => suited >= 2,
            Self::SingleSuited => suited == 1,
            Self::Rainbow => suited == 0,
            Self::NoPair => pairs == 0 && !trips,
            Self::OnePair => pairs == 1 && !trips,
            Self::TwoPair => pairs >= 2,
        }
    }
}

/// One parsed range term, e.g. `AAxx$ds`.
struct Term {
    cards: Deck,
    ranks: [u8; 13],
    qualifiers: Vec<Qualifier>,
}

impl Term {
    fn parse(term: &str, cards_per_player: usize) -> Result<Self, DucyError> {
        let mut parts = term.split('$');
        let body: Vec<char> = parts.next().unwrap_or_default().chars().collect();
        let qualifiers: Vec<Qualifier> = parts.map(Qualifier::parse).collect::<Result<_, _>>()?;

        let mut cards = Deck::empty();
        let mut ranks = [0u8; 13];
        let mut tokens = 0;
        let mut i = 0;
        while i < body.len() {
            let c = body[i];
            if c == 'x' || c == '*' {
                i += 1;
            } else {
                let rank = Rank::try_from_char(&c)?;
                match body
                    .get(i + 1)
                    .filter(|s| s.is_ascii_lowercase() && **s != 'x')
                {
                    Some(s) => {
                        let card = Card::new(rank, Suit::try_from_char(s)?);
                        if cards.has_card(&card) {
                            return Err(DucyError::InvalidRange);
                        }
                        cards |= card;
                        i += 2;
                    }
                    None => {
                        ranks[rank as usize] += 1;
                        i += 1;
                    }
                }
            }
            tokens += 1;
        }
        if (tokens == 0 && qualifiers.is_empty()) || tokens > cards_per_player {
            return Err(DucyError::InvalidRange);
        }
        Ok(Self {
            cards,
            ranks,
            qualifiers,
        })
    }

    fn matches(&self, hand: Deck) -> bool {
        if !hand.has_cards(&self.cards) {
            return false;
        }
        let mut ranks = [0u8; 13];
        let mut suits = [0u8; 4];
        let mut rest_ranks = [0u8; 13];
        for card in hand.iter(false) {
            ranks[card.rank() as usize] += 1;
            suits[card.suit() as usize] += 1;
            if !self.cards.has_card(&card) {
                rest_ranks[card.rank() as usize] += 1;
            }
        }
        rest_ranks
            .iter()
            .zip(&self.ranks)
            .all(|(have, need)| have >= need)
            && self.qualifiers.iter().all(|q| q.matches(&ranks, &suits))
    }
}

/// Number of `k`-card hands from a 52-card deck.
fn total_hands(k: usize) -> u64 {
    (0..k as u64).fold(1, |acc, i| acc * (52 - i) / (i + 1))
}

/// An Omaha starting-hand range, for any hole card count (4 for PLO,
/// 5 for PLO5, ...).
///
/// A range is a union of terms separated by commas or whitespace. Each term
/// lists up to `cards_per_player` cards; missing cards are wildcards.
/// - Rank: `A`, `K`, ..., `2`. Repeated ranks need that many cards of the rank
///   (`AA` is any hand with at least two aces).
/// - Specific card: `As`, `Td`.
/// - Wildcard: `x` or `*`.
/// - Conditions after `$`, all of which must hold:
///   `$ds` double-suited, `$ss` single-suited, `$r` rainbow,
///   `$np` no pair, `$1p` exactly one pair, `$2p` two pair.
///   `$ds`, `$ss` and `$r` split every hand into three disjoint classes.
///
/// Examples: `AAxx$ds`, `KK$ss`, `AsKs`, `JT98$np`, `$2p`.
pub struct OmahaRange {
    cards_per_player: usize,
    range_base: RangeBase,
}

impl OmahaRange {
    /// Creates an empty range for hands of `cards_per_player` cards.
    pub fn new(cards_per_player: usize) -> Self {
        Self {
            cards_per_player,
            range_base: RangeBase::new(),
        }
    }

    /// Parses terms separated by commas or whitespace, each with weight 1.
    pub fn parse(ranges: &str, cards_per_player: usize) -> Result<Self, DucyError> {
        let mut range = Self::new(cards_per_player);
        for part in ranges
            .split([',', ' ', '\t', '\n'])
            .filter(|p| !p.is_empty())
        {
            range.add(part, Decimal::ONE)?;
        }
        Ok(range)
    }

    /// Adds every hand matching `term` with the given weight, replacing the
    /// weight of hands already in the range.
    pub fn add(&mut self, term: &str, weight: Decimal) -> Result<(), DucyError> {
        let term = Term::parse(term.trim(), self.cards_per_player)?;
        for hand in Deck::all_cards().enumerate_combinations(self.cards_per_player) {
            if term.matches(hand) {
                self.range_base.add_deck_weight(hand, weight);
            }
        }
        Ok(())
    }

    /// Number of distinct hands in the range with a positive weight.
    pub fn combos(&self) -> u64 {
        self.iter()
            .filter(|item| item.get_weight() > Decimal::ZERO)
            .count() as u64
    }

    /// Number of possible starting hands, e.g. 270,725 for 4 cards.
    pub fn total_hands(&self) -> u64 {
        total_hands(self.cards_per_player)
    }

    /// Fraction of all starting hands in the range (0 to 1), ignoring weights.
    pub fn coverage(&self) -> f64 {
        self.combos() as f64 / self.total_hands() as f64
    }

    /// Fraction of all starting hands in the range, counting each hand by
    /// its weight (a hand at weight 0.5 counts as half).
    pub fn weighted_coverage(&self) -> f64 {
        let total: Decimal = self.iter().map(|item| item.get_weight()).sum();
        f64::try_from(total).unwrap_or(0.0) / self.total_hands() as f64
    }
}

impl Range for OmahaRange {
    fn iter(&self) -> impl Iterator<Item = RangeItem> {
        self.range_base.iter()
    }
}

#[cfg(test)]
mod test {
    use rust_decimal_macros::dec;

    use super::*;

    fn count(range: &str) -> u64 {
        OmahaRange::parse(range, 4).unwrap().combos()
    }

    #[test]
    fn test_total_hands() {
        assert_eq!(total_hands(4), 270_725);
        assert_eq!(total_hands(5), 2_598_960);
        assert_eq!(count("xxxx"), 270_725);
        assert_eq!(count("$ds"), count("xxxx$ds"));
    }

    #[test]
    fn test_rank_counts() {
        // Two, three or four aces: 6*C(48,2) + 4*48 + 1.
        assert_eq!(count("AA"), 6 * 1128 + 4 * 48 + 1);
        assert_eq!(count("AAAA"), 1);
        assert_eq!(count("AAKK"), 36);
        // Double-suited only when the kings share the aces' two suits.
        assert_eq!(count("AAKK$ds"), 6);
    }

    #[test]
    fn test_suit_classes_partition() {
        let (ds, ss, r) = (count("$ds"), count("$ss"), count("$r"));
        assert_eq!(ds + ss + r, 270_725);
        // 2-2 suit split: C(4,2) suit pairs * C(13,2)^2.
        assert_eq!(ds, 6 * 78 * 78);
        assert_eq!(r, 13u64.pow(4));
        assert_eq!(count("$ds, $ss, $r"), 270_725);
    }

    #[test]
    fn test_pair_classes() {
        // No pair: 13C4 rank sets * 4^4 suits.
        assert_eq!(count("$np"), 715 * 256);
        // Two pair: 13C2 rank pairs * 6 * 6.
        assert_eq!(count("$2p"), 78 * 36);
        assert_eq!(count("$2p$np"), 0);
        assert_eq!(count("$np") + count("$1p") + count("$2p"), {
            // Everything except hands with trips or quads.
            270_725 - (13 * 4 * 48 + 13)
        });
    }

    #[test]
    fn test_specific_cards() {
        // As Ks plus any two of the other 50.
        assert_eq!(count("AsKs"), 50 * 49 / 2);
        // A specific card next to a rank: the rank must come from another card.
        // C(51,3) - C(48,3): the other three cards hold at least one ace.
        assert_eq!(count("AsA"), 20_825 - 17_296);
        assert!(OmahaRange::parse("AsAs", 4).is_err());
    }

    #[test]
    fn test_coverage_and_union() {
        let range = OmahaRange::parse("AAxx, KKxx", 4).unwrap();
        let both = count("AAKK");
        assert_eq!(range.combos(), count("AA") + count("KK") - both);
        assert!((range.coverage() - range.combos() as f64 / 270_725.0).abs() < 1e-12);

        let mut weighted = OmahaRange::new(4);
        weighted.add("AAAA", dec!(0.5)).unwrap();
        weighted.add("KKKK", dec!(1)).unwrap();
        assert!((weighted.weighted_coverage() - 1.5 / 270_725.0).abs() < 1e-15);
    }

    #[test]
    fn test_invalid() {
        for bad in ["AAKKQ", "Z", "AA$xx", "AAx$", "$"] {
            assert!(OmahaRange::parse(bad, 4).is_err(), "{bad}");
        }
        assert!(OmahaRange::new(4).add("", Decimal::ONE).is_err());
        assert_eq!(OmahaRange::parse("", 4).unwrap().combos(), 0);
    }

    #[test]
    fn test_five_card() {
        let range = OmahaRange::parse("AA$ds", 5).unwrap();
        assert_eq!(range.total_hands(), 2_598_960);
        assert!(range.combos() > 0);
    }
}
