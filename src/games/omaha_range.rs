use rust_decimal::Decimal;

use crate::{
    deck::{
        Card, Deck, Rank, Suit,
        range::{Range, RangeBase, RangeItem},
    },
    error::DucyError,
    games::CardDealer,
};

/// A hand property written after `$` in a term.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Condition {
    /// `$ts`: three or more suits with two or more cards each.
    TripleSuited,
    /// `$ds`: exactly two suits with two or more cards each.
    DoubleSuited,
    /// `$ss`: exactly one suit with two or more cards.
    SingleSuited,
    /// `$r`: every card a different suit.
    Rainbow,
    /// `$np`: no two cards share a rank.
    NoPair,
    /// `$1p`: exactly one pair and no trips.
    OnePair,
    /// `$2p`: two or more pairs.
    TwoPair,
    /// `$rd`, `$rd1`, `$3rd`, `$3rd0-1`, ...: `len` cards of distinct ranks
    /// (`None` means the whole hand) whose span leaves between `min_gaps`
    /// and `max_gaps` missing ranks.
    Rundown {
        len: Option<usize>,
        min_gaps: usize,
        max_gaps: usize,
    },
}

/// Largest rank span a rundown may cover, so an ace never counts twice.
const MAX_SPAN: usize = 12;

impl Condition {
    fn parse(s: &str, cards_per_player: usize) -> Result<Self, DucyError> {
        match s {
            "ts" => return Ok(Self::TripleSuited),
            "ds" => return Ok(Self::DoubleSuited),
            "ss" => return Ok(Self::SingleSuited),
            "r" => return Ok(Self::Rainbow),
            "np" => return Ok(Self::NoPair),
            "1p" => return Ok(Self::OnePair),
            "2p" => return Ok(Self::TwoPair),
            _ => {}
        }
        let (len, gaps) = s.split_once("rd").ok_or(DucyError::InvalidRange)?;
        let number = |n: &str| n.parse::<usize>().map_err(|_| DucyError::InvalidRange);
        let len = if len.is_empty() {
            None
        } else {
            Some(number(len)?)
        };
        let (min_gaps, max_gaps) = match gaps.split_once('-') {
            _ if gaps.is_empty() => (0, 0),
            Some((lo, hi)) => (number(lo)?, number(hi)?),
            None => (number(gaps)?, number(gaps)?),
        };
        let cards = len.unwrap_or(cards_per_player);
        if !(2..=cards_per_player).contains(&cards)
            || min_gaps > max_gaps
            || cards - 1 + min_gaps > MAX_SPAN
        {
            return Err(DucyError::InvalidRange);
        }
        Ok(Self::Rundown {
            len,
            min_gaps,
            max_gaps,
        })
    }

    fn matches(self, ranks: &[u8; 13], suits: &[u8; 4]) -> bool {
        let suited = suits.iter().filter(|&&c| c >= 2).count();
        let pairs = ranks.iter().filter(|&&c| c == 2).count();
        let trips = ranks.iter().any(|&c| c >= 3);
        match self {
            Self::TripleSuited => suited >= 3,
            Self::DoubleSuited => suited == 2,
            Self::SingleSuited => suited == 1,
            Self::Rainbow => suited == 0,
            Self::NoPair => pairs == 0 && !trips,
            Self::OnePair => pairs == 1 && !trips,
            Self::TwoPair => pairs >= 2,
            Self::Rundown {
                len,
                min_gaps,
                max_gaps,
            } => {
                let cards: usize = ranks.iter().map(|&c| c as usize).sum();
                match len {
                    // The whole hand: every rank distinct and part of the run.
                    None => {
                        ranks.iter().all(|&c| c <= 1)
                            && (min_gaps..=max_gaps).any(|g| has_run(ranks, cards, g))
                    }
                    Some(len) => (min_gaps..=max_gaps).any(|g| has_run(ranks, len, g)),
                }
            }
        }
    }
}

/// Whether the hand holds `len` distinct ranks spanning exactly
/// `len - 1 + gaps` ranks, with the ace playing high or low.
fn has_run(ranks: &[u8; 13], len: usize, gaps: usize) -> bool {
    let span = len - 1 + gaps;
    if span > MAX_SPAN {
        return false;
    }
    // Bit 0 is a low ace, bits 1..=13 are deuce through ace.
    let mut bits = 0u16;
    for (i, &c) in ranks.iter().enumerate() {
        if c > 0 {
            bits |= 1 << (i + 1);
        }
    }
    if bits & (1 << 13) != 0 {
        bits |= 1;
    }
    (0..=13 - span).any(|low| {
        let high = low + span;
        let inside = (bits >> (low + 1)) & ((1u16 << (span - 1)) - 1);
        bits & (1 << low) != 0 && bits & (1 << high) != 0 && inside.count_ones() as usize + 2 >= len
    })
}

/// A condition, possibly negated with `!`.
#[derive(Clone, Copy, Debug)]
struct Qualifier {
    negate: bool,
    condition: Condition,
}

impl Qualifier {
    fn parse(s: &str, cards_per_player: usize) -> Result<Self, DucyError> {
        let (negate, rest) = match s.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, s),
        };
        Ok(Self {
            negate,
            condition: Condition::parse(rest, cards_per_player)?,
        })
    }

    fn matches(self, ranks: &[u8; 13], suits: &[u8; 4]) -> bool {
        self.condition.matches(ranks, suits) != self.negate
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
        let qualifiers: Vec<Qualifier> = parts
            .map(|q| Qualifier::parse(q, cards_per_player))
            .collect::<Result<_, _>>()?;

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

/// An Omaha starting-hand range for any hole-card count (4 for PLO, 5 for
/// PLO5, 6 for PLO6).
///
/// # Syntax
///
/// A range is a list of **terms** separated by commas or whitespace. A hand
/// is in the range if it matches any term. Each term is an optional **card
/// pattern** followed by any number of **conditions**, each starting with
/// `$`. A hand matches a term when it fits the card pattern and meets every
/// condition.
///
/// ## Card pattern
///
/// Up to `cards_per_player` tokens. Cards not listed are wildcards, so `AA`
/// and `AAxx` mean the same thing. Order does not matter.
///
/// | Token | Meaning | Example |
/// |---|---|---|
/// | `A` `K` `Q` `J` `T` `9`…`2` | a card of that rank; repeat a rank to need more of them (at least, not exactly) | `AA`: two or more aces |
/// | rank + suit (`c` `d` `h` `s`) | that exact card | `As`, `Td` |
/// | `x` or `*` | any card | `AAxx` |
///
/// A specific card and a bare rank never use the same card: `AsA` needs the
/// ace of spades plus another ace.
///
/// ## Conditions
///
/// | Condition | Meaning |
/// |---|---|
/// | `$ts` | triple-suited: three suits each with two or more cards (PLO6, e.g. `AsKs QhJh 9d8d`) |
/// | `$ds` | double-suited: exactly two suits with two or more cards each |
/// | `$ss` | single-suited: exactly one suit with two or more cards |
/// | `$r` | rainbow: no two cards share a suit |
/// | `$np` | no pair |
/// | `$1p` | exactly one pair, no trips |
/// | `$2p` | two or more pairs |
/// | `$rd` | rundown: every card a different rank, all consecutive (`JT98`) |
/// | `$rdG` | rundown with exactly `G` gaps (`$rd1`: `T986`, `T976`; `$rd2`: `T975`, `T865`) |
/// | `$rdG-H` | rundown with `G` to `H` gaps (`$rd0-1`: `JT98` or `T986`) |
/// | `$Nrd`, `$NrdG`, `$NrdG-H` | contains an `N`-card rundown, with the same gap forms (`$3rd`: `TT98`, `KT98`) |
/// | `$!cond` | the condition does **not** hold (`$3rd$!rd`) |
///
/// Notes:
/// - Suit conditions split every hand into disjoint classes: `$ts` (only
///   possible with 6+ cards), `$ds`, `$ss` and `$r`. So do `$np`, `$1p`,
///   `$2p` together with hands holding trips.
/// - Gaps are the missing ranks inside the run: the run's high rank minus its
///   low rank, minus (cards − 1). `T986` spans ten to six, four steps for
///   four cards, so it has one gap.
/// - The ace plays high or low in rundowns: `A234` and `AKQJ` are both
///   rundowns, `KA23` is not.
/// - `$rd` forms describe the whole hand, so the hand can't contain a pair.
///   `$Nrd` forms only need some `N` cards of different ranks to form the
///   run; the other cards can be anything. `TT98` matches `$3rd$1p` (a
///   three-card rundown plus a pair of tens) but not `$rd`. Every hand that
///   matches `$rd` also matches `$3rd`; use `$3rd$!rd` to exclude full
///   rundowns.
///
/// ## Weights
///
/// [`Self::parse`] gives every hand weight 1. [`Self::add`] takes a weight;
/// a hand already in the range takes the weight of the latest term that
/// matches it.
///
/// ## Examples
///
/// | Range | Hands |
/// |---|---|
/// | `AAxx$ds` | aces, double-suited |
/// | `AsKs` | the ace and king of spades plus any two cards |
/// | `KK$!r` | kings with at least one suited pair |
/// | `$rd$ds` | double-suited four-card rundowns |
/// | `$rd0-1$np$ss` | single-suited rundowns with at most one gap |
/// | `TT$3rd` | a pair of tens alongside a three-card rundown, e.g. `TT98` |
/// | `$2p, $rd` | any two-pair hand or any rundown |
/// | `$ts` | triple-suited PLO6 hands |
#[derive(Clone)]
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

    /// Hole cards per hand.
    pub fn cards_per_player(&self) -> usize {
        self.cards_per_player
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

/// Draws hands from an [`OmahaRange`] in proportion to their weights,
/// skipping hands that use cards outside `available`.
pub(crate) struct RangeSampler {
    combos: Vec<Deck>,
    cumulative: Vec<f64>,
}

impl RangeSampler {
    /// Errors with `InvalidRange` if no hand with positive weight fits
    /// `available`.
    pub(crate) fn new(range: &OmahaRange, available: Deck) -> Result<Self, DucyError> {
        let mut combos: Vec<(Deck, f64)> = range
            .iter()
            .map(|item| {
                (
                    item.get_deck(),
                    f64::try_from(item.get_weight()).unwrap_or(0.0),
                )
            })
            .filter(|&(deck, w)| w > 0.0 && available.has_cards(&deck))
            .collect();
        if combos.is_empty() {
            return Err(DucyError::InvalidRange);
        }
        // The range is a hash map; sort so seeded draws are reproducible.
        combos.sort_unstable_by_key(|&(deck, _)| u64::from(deck));
        let cumulative = combos
            .iter()
            .scan(0.0, |total, &(_, w)| {
                *total += w;
                Some(*total)
            })
            .collect();
        Ok(Self {
            combos: combos.into_iter().map(|(deck, _)| deck).collect(),
            cumulative,
        })
    }

    pub(crate) fn draw(&self, dealer: &mut CardDealer) -> Deck {
        let total = self.cumulative[self.cumulative.len() - 1];
        let x = dealer.random_f64(total);
        let i = self
            .cumulative
            .partition_point(|&c| c <= x)
            .min(self.combos.len() - 1);
        self.combos[i]
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
        for bad in [
            "AAKKQ", "Z", "AA$xx", "AAx$", "$", "$5rd", "$1rd", "$rd2-1", "$rdx", "$rd-1", "$!",
            "$!!r",
        ] {
            assert!(OmahaRange::parse(bad, 4).is_err(), "{bad}");
        }
        assert!(OmahaRange::new(4).add("", Decimal::ONE).is_err());
        assert_eq!(OmahaRange::parse("", 4).unwrap().combos(), 0);
    }

    fn is_match(term: &str, hand: &str) -> bool {
        let hand = Deck::parse(hand).unwrap();
        Term::parse(term, hand.num_cards() as usize)
            .unwrap()
            .matches(hand)
    }

    #[test]
    fn test_rundowns() {
        assert!(is_match("$rd", "Jh Td 9c 8s"));
        assert!(is_match("$rd", "As 2d 3c 4h"));
        assert!(is_match("$rd", "As Kd Qc Jh"));
        assert!(!is_match("$rd", "Ks Ad 2c 3h"));
        assert!(!is_match("$rd", "Th 9d 8c 6s"));

        // Gaps.
        assert!(is_match("$rd1", "Th 9d 8c 6s"));
        assert!(!is_match("$rd1", "Jh Td 9c 8s"));
        assert!(is_match("$rd2", "Th 9d 7c 5s"));
        assert!(is_match("$rd2", "Th 8d 6c 5s"));
        assert!(is_match("$rd0-1", "Jh Td 9c 8s"));
        assert!(is_match("$rd0-1", "Th 9d 8c 6s"));
        assert!(is_match("$rd1", "Th 9d 7c 6s"));
        assert!(!is_match("$rd0-1", "Th 9d 7c 5s"));

        // Partial rundowns.
        assert!(is_match("$3rd", "Th Td 9c 8s"));
        assert!(is_match("$3rd$1p", "Th Td 9c 8s"));
        assert!(is_match("TT$3rd", "Th Td 9c 8s"));
        assert!(!is_match("$rd", "Th Td 9c 8s"));
        assert!(is_match("$3rd", "Kh Td 9c 8s"));
        assert!(is_match("$3rd", "Jh Td 9c 8s"));
        assert!(!is_match("$3rd$!rd", "Jh Td 9c 8s"));
        assert!(is_match("$3rd1", "Kh Td 9c 7s"));
        assert!(!is_match("$3rd", "Kh Td 8c 6s"));
        assert!(is_match("$2rd", "Kh Qd 8c 2s"));

        // Rank windows: 11 for no gaps, 10 windows * 3 gap spots for one gap.
        assert_eq!(count("$rd"), 11 * 256);
        assert_eq!(count("$rd1"), 30 * 256);
        assert_eq!(count("$rd0-1"), count("$rd") + count("$rd1"));
        assert!(count("$rd") + count("$rd1") + count("$rd2") <= count("$np"));
    }

    #[test]
    fn test_rundown_five_card() {
        assert!(is_match("$rd", "Qh Jh Td 9c 8s"));
        assert!(is_match("$rd1", "Qh Jh Td 9c 7s"));
        assert!(is_match("$4rd", "Qh Jh Td 9c 2s"));
        assert!(!is_match("$rd", "Qh Jh Td 9c 2s"));
    }

    #[test]
    fn test_negation() {
        assert_eq!(count("$!r"), 270_725 - count("$r"));
        assert_eq!(count("KK$!r"), count("KK$ss") + count("KK$ds"));
    }

    #[test]
    fn test_triple_suited() {
        assert!(is_match("$ts", "As Ks Qh Jh 9d 8d"));
        assert!(!is_match("$ds", "As Ks Qh Jh 9d 8d"));
        assert!(is_match("$ds", "As Ks Qh Jh 9d 8c"));
        assert!(is_match("$ds", "As Ks Qs Jh Th 8c"));
        assert!(!is_match("$ts", "As Ks Qs Jh Th 8c"));
        assert_eq!(count("$ts"), 0);
    }

    #[test]
    fn test_five_card() {
        let range = OmahaRange::parse("AA$ds", 5).unwrap();
        assert_eq!(range.total_hands(), 2_598_960);
        assert!(range.combos() > 0);
    }
}
