//! Monte Carlo equity for Omaha and Omaha Hi-Lo players holding shorthand
//! ranges (the [`OmahaRange`](crate::games::omaha_range::OmahaRange) syntax).
//!
//! [`OmahaRangeSampler`] parses the same terms as `OmahaRange` but never lists
//! every starting hand up front, which is impractical for PLO6 (about 20.4M
//! hands). Narrow ranges are expanded into an explicit list of hands, built
//! from each term's required cards so only candidates that can match are
//! visited. Broad ranges are drawn by rejection sampling: deal a random hand
//! and keep it with probability proportional to its weight.

use std::collections::HashSet;
use std::sync::OnceLock;

use rust_decimal::Decimal;

use crate::{
    deck::Deck,
    error::DucyError,
    games::{
        CardDealer, EQUITY_SCALE,
        flop_game::{FlopGame, FlopGameState},
        holdem::RangeEquitySamples,
        omaha::{OmahaGameEvaluation, OmahaGameState, high_winners, hole_card_combos},
        omaha_hilo::{OmahaHiLoGameEvaluation, OmahaHiLoGameState, hilo_combos, hilo_distribute},
        omaha_range::{Term, total_hands},
    },
};

const MAX_PLAYERS: usize = 10;
/// Ranges whose terms have at most this many candidate hands in total are
/// expanded into an explicit list; larger ones are rejection-sampled.
const POOL_LIMIT: u64 = 300_000;
/// Random hands tried for one rejection-sampled draw before giving up.
const MAX_REJECTIONS: u32 = 200_000;
/// Conflicting draws (range players sharing a card) allowed for one sample.
const MAX_CONFLICT_REDRAWS: u32 = 100_000;
/// Random hands used to estimate a rejection-sampled range's coverage.
const COVERAGE_SAMPLES: usize = 20_000;

fn choose(n: u64, k: u64) -> u64 {
    if k > n {
        return 0;
    }
    (0..k).fold(1u64, |acc, i| acc.saturating_mul(n - i) / (i + 1))
}

/// An Omaha shorthand range prepared for drawing random hands.
pub struct OmahaRangeSampler {
    cards_per_player: usize,
    terms: Vec<(Term, f64)>,
    /// Explicit `(hand, weight)` list, or `None` when the range is too broad
    /// and is rejection-sampled instead. Built on first use.
    pool: OnceLock<Option<Vec<(Deck, f64)>>>,
}

impl OmahaRangeSampler {
    /// An empty range for hands of `cards_per_player` cards.
    pub fn new(cards_per_player: usize) -> Self {
        Self {
            cards_per_player,
            terms: Vec::new(),
            pool: OnceLock::new(),
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

    /// Adds a term. A hand matching several terms takes the weight of the
    /// latest one, as in `OmahaRange`.
    pub fn add(&mut self, term: &str, weight: Decimal) -> Result<(), DucyError> {
        let weight = f64::try_from(weight).map_err(|_| DucyError::InvalidRange)?;
        if weight < 0.0 {
            return Err(DucyError::InvalidRange);
        }
        self.terms
            .push((Term::parse(term.trim(), self.cards_per_player)?, weight));
        self.pool = OnceLock::new();
        Ok(())
    }

    pub fn cards_per_player(&self) -> usize {
        self.cards_per_player
    }

    /// The weight of `hand` in the range (0 when it isn't in the range).
    pub fn weight(&self, hand: Deck) -> f64 {
        self.terms
            .iter()
            .rev()
            .find(|(t, _)| t.matches(hand))
            .map_or(0.0, |(_, w)| *w)
    }

    /// Whether `hand` is in the range with a positive weight.
    pub fn contains(&self, hand: Deck) -> bool {
        self.weight(hand) > 0.0
    }

    /// Fraction of all starting hands in the range, ignoring weights, and
    /// whether that figure is exact (explicit list) or a sampled estimate.
    pub fn coverage(&self) -> (f64, bool) {
        let total = total_hands(self.cards_per_player) as f64;
        if let Some(pool) = self.pool() {
            return (pool.len() as f64 / total, true);
        }
        let mut dealer = CardDealer::seeded(Deck::all_cards(), 0x5EED);
        let hits = (0..COVERAGE_SAMPLES)
            .filter(|_| {
                dealer.reset();
                self.contains(dealer.deal(self.cards_per_player))
            })
            .count();
        (hits as f64 / COVERAGE_SAMPLES as f64, false)
    }

    /// Number of hands that could match `term`: its required cards plus any
    /// completion, counted with repeats.
    fn candidate_count(&self, term: &Term) -> u64 {
        let fixed = term.cards.num_cards() as u64;
        let mut count = 1u64;
        let mut used = fixed;
        for (rank, &need) in term.ranks.iter().enumerate() {
            if need == 0 {
                continue;
            }
            let taken = term
                .cards
                .iter(false)
                .filter(|c| c.rank() as usize == rank)
                .count() as u64;
            count = count.saturating_mul(choose(4 - taken, need as u64));
            used += need as u64;
        }
        let fill = (self.cards_per_player as u64).saturating_sub(used);
        count.saturating_mul(choose(52 - used, fill))
    }

    fn pool(&self) -> Option<&Vec<(Deck, f64)>> {
        self.pool
            .get_or_init(|| {
                let total: u64 = self
                    .terms
                    .iter()
                    .map(|(t, _)| self.candidate_count(t))
                    .fold(0u64, |a, c| a.saturating_add(c));
                if total > POOL_LIMIT {
                    return None;
                }
                let mut seen = HashSet::new();
                for (term, _) in &self.terms {
                    self.term_candidates(term, &mut seen);
                }
                let mut pool: Vec<(Deck, f64)> = seen
                    .into_iter()
                    .map(Deck::from)
                    .map(|h| (h, self.weight(h)))
                    .filter(|(_, w)| *w > 0.0)
                    .collect();
                pool.sort_by_key(|(h, _)| u64::from(*h));
                Some(pool)
            })
            .as_ref()
    }

    /// Every hand holding `term`'s specific cards and required ranks, added to
    /// `out` (as deck bits) when it matches the term.
    fn term_candidates(&self, term: &Term, out: &mut HashSet<u64>) {
        let mut partials = vec![term.cards];
        let mut used = term.cards.num_cards() as usize;
        for (rank, &need) in term.ranks.iter().enumerate() {
            if need == 0 {
                continue;
            }
            let mut rank_cards = Deck::empty();
            for c in Deck::all_cards().iter(false) {
                if c.rank() as usize == rank && !term.cards.has_card(&c) {
                    rank_cards |= c;
                }
            }
            partials = partials
                .iter()
                .flat_map(|p| {
                    rank_cards
                        .enumerate_combinations(need as usize)
                        .map(move |c| *p | c)
                })
                .collect();
            used += need as usize;
        }
        let fill = self.cards_per_player.saturating_sub(used);
        for p in partials {
            if fill == 0 {
                if term.matches(p) {
                    out.insert(u64::from(p));
                }
                continue;
            }
            let mut rest = Deck::all_cards();
            rest -= p;
            for c in rest.enumerate_combinations(fill) {
                let hand = p | c;
                if term.matches(hand) {
                    out.insert(u64::from(hand));
                }
            }
        }
    }
}

/// A range restricted to the cards still available for one sampling call.
enum Drawable<'a> {
    /// Explicit hands with cumulative weights.
    Pool(Vec<Deck>, Vec<f64>),
    /// Rejection sampling against the range's terms.
    Reject(&'a OmahaRangeSampler, f64),
}

impl<'a> Drawable<'a> {
    fn new(range: &'a OmahaRangeSampler, available: Deck) -> Result<Self, DucyError> {
        if let Some(pool) = range.pool() {
            let mut hands = Vec::new();
            let mut cumulative = Vec::new();
            let mut total = 0.0;
            for &(h, w) in pool {
                if available.has_cards(&h) {
                    total += w;
                    hands.push(h);
                    cumulative.push(total);
                }
            }
            if hands.is_empty() {
                return Err(DucyError::InvalidRange);
            }
            return Ok(Self::Pool(hands, cumulative));
        }
        let max_weight = range.terms.iter().map(|(_, w)| *w).fold(0.0, f64::max);
        if max_weight <= 0.0 {
            return Err(DucyError::InvalidRange);
        }
        Ok(Self::Reject(range, max_weight))
    }

    fn draw(&self, dealer: &mut CardDealer, cards: usize) -> Result<Deck, DucyError> {
        match self {
            Self::Pool(hands, cumulative) => {
                let total = cumulative[cumulative.len() - 1];
                let x = dealer.random_f64(total);
                let i = cumulative.partition_point(|&c| c <= x).min(hands.len() - 1);
                Ok(hands[i])
            }
            Self::Reject(range, max_weight) => {
                for _ in 0..MAX_REJECTIONS {
                    dealer.reset();
                    let hand = dealer.deal(cards);
                    let w = range.weight(hand);
                    if w > 0.0 && (w >= *max_weight || dealer.random_f64(*max_weight) < w) {
                        return Ok(hand);
                    }
                }
                Err(DucyError::InvalidRange)
            }
        }
    }
}

/// Shared sampling loop: draws a hand for every range player (redrawing all of
/// them when two share a card), deals the rest of the board, and adds each
/// player's pot share from `distribute`.
fn sample_ranges<C>(
    fs: &FlopGameState,
    ranges: &[&OmahaRangeSampler],
    samples: usize,
    seed: Option<u64>,
    combos_of: impl Fn(&Deck) -> C,
    distribute: impl Fn(&Deck, &[C], &mut [u64]),
) -> Result<RangeEquitySamples, DucyError> {
    let cards = fs.cards_per_player();
    if ranges.iter().any(|r| r.cards_per_player() != cards) {
        return Err(DucyError::IncorrectCardCount);
    }
    let num_fixed = fs.hole_cards().len();
    let players = num_fixed + ranges.len();
    if players > MAX_PLAYERS {
        return Err(DucyError::TooManyPlayers);
    }
    let available = fs.remaining_cards();
    let cards_needed = fs.cards_needed();
    if (available.num_cards() as usize) < cards * ranges.len() + cards_needed {
        return Err(DucyError::NotEnoughCards);
    }
    let drawables: Vec<Drawable> = ranges
        .iter()
        .map(|r| Drawable::new(r, available))
        .collect::<Result<_, _>>()?;

    let community = fs.get_community_cards();
    let mut player_combos: Vec<C> = fs.hole_cards().iter().map(&combos_of).collect();
    let mut dealer = CardDealer::maybe_seeded(available, seed);
    let mut drawn = vec![Deck::empty(); ranges.len()];
    let mut shares = vec![0u64; players];
    let mut result = RangeEquitySamples {
        samples: samples as u64,
        equity_sum: vec![0.0; players],
        equity_sq_sum: vec![0.0; players],
    };

    for _ in 0..samples {
        let mut redraws = 0;
        let used = 'draw: loop {
            let mut used = Deck::empty();
            for (r, d) in drawables.iter().enumerate() {
                let hand = d.draw(&mut dealer, cards)?;
                if u64::from(used) & u64::from(hand) != 0 {
                    redraws += 1;
                    if redraws > MAX_CONFLICT_REDRAWS {
                        return Err(DucyError::InvalidRange);
                    }
                    continue 'draw;
                }
                used |= hand;
                drawn[r] = hand;
            }
            break used;
        };

        player_combos.truncate(num_fixed);
        player_combos.extend(drawn.iter().map(&combos_of));
        dealer.reset();
        let board = community | dealer.deal_excluding(cards_needed, used);
        shares.iter_mut().for_each(|s| *s = 0);
        distribute(&board, &player_combos, &mut shares);
        for (i, &s) in shares.iter().enumerate() {
            let share = s as f64 / EQUITY_SCALE as f64;
            result.equity_sum[i] += share;
            result.equity_sq_sum[i] += share * share;
        }
    }
    Ok(result)
}

impl OmahaGameEvaluation {
    /// Monte Carlo equity with range players. Players already added to
    /// `game_state` keep their hands and come first; `ranges` are the players
    /// after them. Each sample draws a hand from every range in proportion to
    /// its weight, redraws them all if any two share a card, then deals the
    /// rest of the board. Returns sums over samples, so calls can be added.
    ///
    /// Errors with `InvalidRange` when a range has no hand left (e.g. blocked
    /// by the board or dead cards) or the ranges can't be dealt without
    /// sharing a card, and `IncorrectCardCount` when a range is for a
    /// different number of hole cards.
    pub fn sample_range_equity(
        &self,
        game_state: &OmahaGameState,
        ranges: &[&OmahaRangeSampler],
        samples: usize,
    ) -> Result<RangeEquitySamples, DucyError> {
        self.sample_range_equity_seeded(game_state, ranges, samples, None)
    }

    /// Like `sample_range_equity`; a `seed` makes the result reproducible.
    pub fn sample_range_equity_seeded(
        &self,
        game_state: &OmahaGameState,
        ranges: &[&OmahaRangeSampler],
        samples: usize,
        seed: Option<u64>,
    ) -> Result<RangeEquitySamples, DucyError> {
        sample_ranges(
            &game_state.flop_game_state,
            ranges,
            samples,
            seed,
            hole_card_combos,
            |board, combos, shares| high_winners(board, combos).distribute(shares),
        )
    }
}

impl OmahaHiLoGameEvaluation {
    /// Omaha Hi-Lo version of [`OmahaGameEvaluation::sample_range_equity`].
    pub fn sample_range_equity(
        &self,
        game_state: &OmahaHiLoGameState,
        ranges: &[&OmahaRangeSampler],
        samples: usize,
    ) -> Result<RangeEquitySamples, DucyError> {
        self.sample_range_equity_seeded(game_state, ranges, samples, None)
    }

    /// Like `sample_range_equity`; a `seed` makes the result reproducible.
    pub fn sample_range_equity_seeded(
        &self,
        game_state: &OmahaHiLoGameState,
        ranges: &[&OmahaRangeSampler],
        samples: usize,
        seed: Option<u64>,
    ) -> Result<RangeEquitySamples, DucyError> {
        sample_ranges(
            &game_state.flop_game_state,
            ranges,
            samples,
            seed,
            hilo_combos,
            hilo_distribute,
        )
    }
}

#[cfg(test)]
mod test {
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::deck::{Card, range::Range};
    use crate::games::{GameEquityEvaluation, omaha_range::OmahaRange};

    fn turn_state(players: &[&str]) -> OmahaGameState {
        let mut s = OmahaGameState::new(4);
        for p in players {
            s.add_player(Deck::parse(p).unwrap()).unwrap();
        }
        s.set_flop(Deck::parse("Ts 7d 2c").unwrap()).unwrap();
        s.set_turn(Card::parse("9h").unwrap()).unwrap();
        s
    }

    fn exact(players: &[&str]) -> Vec<f64> {
        OmahaGameEvaluation {}
            .evaluate_equity(&turn_state(players))
            .iter()
            .map(|d| f64::try_from(*d).unwrap())
            .collect()
    }

    fn close(a: &[f64], b: &[f64], tol: f64) {
        for (x, y) in a.iter().zip(b) {
            assert!((x - y).abs() < tol, "{a:?} vs {b:?}");
        }
    }

    #[test]
    fn test_single_hand_range_matches_exact() {
        let range = OmahaRangeSampler::parse("AsAhKsKh", 4).unwrap();
        let state = turn_state(&["Jd Jc 8s 8c"]);
        let r = OmahaGameEvaluation {}
            .sample_range_equity_seeded(&state, &[&range], 20_000, Some(1))
            .unwrap();
        assert!((r.equity_sum.iter().sum::<f64>() - 20_000.0).abs() < 1e-6);
        close(&r.equity(), &exact(&["Jd Jc 8s 8c", "As Ah Ks Kh"]), 0.02);
    }

    #[test]
    fn test_weighted_two_hand_range_matches_brute_force() {
        let mut range = OmahaRangeSampler::new(4);
        range.add("AsAhKsKh", dec!(3)).unwrap();
        range.add("QdQh6s6d", dec!(1)).unwrap();
        let a = exact(&["Jd Jc 8s 8c", "As Ah Ks Kh"]);
        let b = exact(&["Jd Jc 8s 8c", "Qd Qh 6s 6d"]);
        let expected: Vec<f64> = a.iter().zip(&b).map(|(x, y)| (3.0 * x + y) / 4.0).collect();
        let state = turn_state(&["Jd Jc 8s 8c"]);
        let r = OmahaGameEvaluation {}
            .sample_range_equity_seeded(&state, &[&range], 40_000, Some(2))
            .unwrap();
        close(&r.equity(), &expected, 0.015);
    }

    #[test]
    fn test_pool_matches_omaha_range_membership() {
        for text in ["AAxx", "$rd$ds, KK$!r", "AsKs, TT$3rd"] {
            let sampler = OmahaRangeSampler::parse(text, 4).unwrap();
            let pool = sampler.pool().expect("PLO4 ranges are expanded");
            let full = OmahaRange::parse(text, 4).unwrap();
            assert_eq!(pool.len() as u64, full.combos(), "{text}");
            assert!(
                full.iter().all(|item| sampler.contains(item.get_deck())),
                "{text}"
            );
            let (cov, exact) = sampler.coverage();
            assert!(exact && (cov - full.coverage()).abs() < 1e-12);
        }
    }

    #[test]
    fn test_broad_plo6_range_is_rejection_sampled() {
        let range = OmahaRangeSampler::parse("AAxx", 6).unwrap();
        assert!(range.pool().is_none());
        // Two or more aces among six cards.
        let n = |a: u64, b: u64| choose(a, b) as f64;
        let truth = (n(4, 2) * n(48, 4) + n(4, 3) * n(48, 3) + n(48, 2)) / n(52, 6);
        let (cov, exact) = range.coverage();
        assert!(!exact && (cov - truth).abs() < 0.01, "{cov} vs {truth}");

        let mut state = OmahaGameState::new(6);
        state
            .add_player(Deck::parse("Kd Kc Qs Qh 9d 8c").unwrap())
            .unwrap();
        let r = OmahaGameEvaluation {}
            .sample_range_equity_seeded(&state, &[&range], 2_000, Some(3))
            .unwrap();
        assert!((r.equity_sum.iter().sum::<f64>() - 2_000.0).abs() < 1e-6);
    }

    #[test]
    fn test_seeded_is_reproducible() {
        let range = OmahaRangeSampler::parse("$ds", 5).unwrap();
        let mut state = OmahaGameState::new(5);
        state
            .add_player(Deck::parse("As Ah Ks Kh 7d").unwrap())
            .unwrap();
        let eval = OmahaGameEvaluation {};
        let run = |seed| {
            eval.sample_range_equity_seeded(&state, &[&range, &range], 500, Some(seed))
                .unwrap()
                .equity_sum
        };
        assert_eq!(run(7), run(7));
        assert_ne!(run(7), run(8));
    }

    #[test]
    fn test_errors() {
        let eval = OmahaGameEvaluation {};
        let state = turn_state(&["Jd Jc 8s 8c"]);
        // Every hand of the range needs a card that's on the board.
        let blocked = OmahaRangeSampler::parse("Ts", 4).unwrap();
        assert_eq!(
            eval.sample_range_equity(&state, &[&blocked], 10)
                .unwrap_err(),
            DucyError::InvalidRange
        );
        // Two players who both need the same four cards.
        let one = OmahaRangeSampler::parse("AsAhKsKh", 4).unwrap();
        assert_eq!(
            eval.sample_range_equity(&state, &[&one, &one], 10)
                .unwrap_err(),
            DucyError::InvalidRange
        );
        let plo5 = OmahaRangeSampler::parse("AAxxx", 5).unwrap();
        assert_eq!(
            eval.sample_range_equity(&state, &[&plo5], 10).unwrap_err(),
            DucyError::IncorrectCardCount
        );
        assert!(OmahaRangeSampler::parse("AA$zz", 4).is_err());
        let mut neg = OmahaRangeSampler::new(4);
        assert!(neg.add("AAxx", Decimal::NEGATIVE_ONE).is_err());
    }

    #[test]
    fn test_hilo_single_hand_range_matches_exact() {
        let mut s = OmahaHiLoGameState::new(4);
        s.add_player(Deck::parse("As 2d Kc Kd").unwrap()).unwrap();
        s.set_flop(Deck::parse("3h 7c Jd").unwrap()).unwrap();
        s.set_turn(Card::parse("5s").unwrap()).unwrap();
        let mut full = OmahaHiLoGameState::new(4);
        full.add_player(Deck::parse("As 2d Kc Kd").unwrap())
            .unwrap();
        full.add_player(Deck::parse("Ah 4c Qs Qh").unwrap())
            .unwrap();
        full.set_flop(Deck::parse("3h 7c Jd").unwrap()).unwrap();
        full.set_turn(Card::parse("5s").unwrap()).unwrap();
        let eval = OmahaHiLoGameEvaluation {};
        let want: Vec<f64> = eval
            .evaluate_equity(&full)
            .iter()
            .map(|d| f64::try_from(*d).unwrap())
            .collect();
        let range = OmahaRangeSampler::parse("Ah4cQsQh", 4).unwrap();
        let r = eval
            .sample_range_equity_seeded(&s, &[&range], 20_000, Some(4))
            .unwrap();
        close(&r.equity(), &want, 0.02);
    }
}
