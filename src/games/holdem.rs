use std::cmp::Ordering;

use rust_decimal::Decimal;
use strum::IntoEnumIterator;

use crate::{
    deck::{
        Card, Deck, Rank, Suit,
        range::{Range, RangeBase},
    },
    error::DucyError,
    games::{
        CardDealer, EquityShares, FastWinnerTracker, GameEquityEvaluation, GameEvaluation,
        GameState, GameWinner, WinnerTracker,
        flop_game::{FlopGame, FlopGameState},
    },
    ranking::{
        hand_rank::{StandardHandRanker, StandardHandRanks},
        standard_hand_ranker::RankOrder,
    },
};

const MAX_PLAYERS: usize = 10;

/// Texas Hold'em game state (2 hole cards per player).
pub struct HoldemGameState {
    pub(crate) flop_game_state: FlopGameState,
}

impl HoldemGameState {
    /// Creates a new Hold'em game state.
    pub fn new() -> Self {
        Self {
            flop_game_state: FlopGameState::new(2),
        }
    }
}

impl Default for HoldemGameState {
    fn default() -> Self {
        Self::new()
    }
}

impl FlopGame for HoldemGameState {
    fn get_community_cards(&self) -> Deck {
        self.flop_game_state.get_community_cards()
    }

    fn add_player(&mut self, cards: Deck) -> Result<(), DucyError> {
        self.flop_game_state.add_player(cards)
    }

    fn set_flop(&mut self, cards: Deck) -> Result<(), DucyError> {
        self.flop_game_state.set_flop(cards)
    }

    fn set_turn(&mut self, card: Card) -> Result<(), DucyError> {
        self.flop_game_state.set_turn(card)
    }

    fn set_river(&mut self, card: Card) -> Result<(), DucyError> {
        self.flop_game_state.set_river(card)
    }

    fn add_dead_cards(&mut self, cards: Deck) -> Result<(), DucyError> {
        self.flop_game_state.add_dead_cards(cards)
    }

    fn get_player_hole_cards(&self) -> impl Iterator<Item = &Deck> {
        self.flop_game_state.get_player_hole_cards()
    }

    fn get_final_states<'a>(&'a self) -> impl Iterator<Item = Self> + 'a {
        self.flop_game_state
            .get_final_states()
            .map(|x| Self { flop_game_state: x })
    }
}

impl GameState for HoldemGameState {}

/// Evaluates Hold'em hands to determine winners.
pub struct HoldemGameEvaluation {}

impl GameEvaluation<HoldemGameState, StandardHandRanks> for HoldemGameEvaluation {
    fn evaluate_winners(&self, game_state: &HoldemGameState) -> Vec<GameWinner<StandardHandRanks>> {
        let mut tracker = WinnerTracker::new();
        for (i, player) in game_state.get_player_hole_cards().enumerate() {
            let combined_deck = *player | game_state.get_community_cards();
            if let Some(rank) =
                StandardHandRanker::get_rank_at_least(&combined_deck, tracker.best_hand())
            {
                tracker.consider(i, rank);
            }
        }
        tracker.into_results()
    }
}

impl GameEquityEvaluation<HoldemGameState, StandardHandRanks, HoldemGameEvaluation>
    for HoldemGameEvaluation
{
    fn evaluate_equity(&self, game_state: &HoldemGameState) -> Vec<Decimal> {
        let runouts = game_state
            .flop_game_state
            .enumerate_runout_community_cards()
            .collect();
        holdem_shares(game_state.flop_game_state.hole_cards(), runouts).equity()
    }
}

impl HoldemGameEvaluation {
    /// Number of runouts exact equity enumerates.
    pub fn runout_count(&self, game_state: &HoldemGameState) -> u64 {
        game_state.flop_game_state.runout_count()
    }

    /// Equity totals for runouts `start..start + count` of the exact
    /// enumeration. Merging every chunk gives the same result as
    /// `evaluate_equity`, so chunks can run on separate threads or workers.
    pub fn evaluate_equity_chunk(
        &self,
        game_state: &HoldemGameState,
        start: u64,
        count: u64,
    ) -> EquityShares {
        let runouts = game_state.flop_game_state.runout_chunk(start, count);
        holdem_shares(game_state.flop_game_state.hole_cards(), runouts)
    }
}

fn holdem_shares(hole_cards: &[Deck], runouts: Vec<Deck>) -> EquityShares {
    crate::games::accumulate_shares(runouts, hole_cards.len(), |community, shares| {
        holdem_winners(community, hole_cards).distribute(shares);
    })
}

fn holdem_winners(community: &Deck, hands: &[Deck]) -> FastWinnerTracker {
    let mut tracker = FastWinnerTracker::new();
    for (i, hand) in hands.iter().enumerate() {
        tracker.consider(i, StandardHandRanker::score(&(*hand | *community)));
    }
    tracker
}

/// Summed Monte Carlo results for range-vs-range equity, indexed by player.
#[derive(Debug, Clone)]
pub struct RangeEquitySamples {
    /// Number of samples taken.
    pub samples: u64,
    /// Sum across samples of each player's pot share.
    pub equity_sum: Vec<f64>,
    /// Sum across samples of each player's squared pot share (for standard error).
    pub equity_sq_sum: Vec<f64>,
}

impl RangeEquitySamples {
    /// Estimated equity for each player.
    pub fn equity(&self) -> Vec<f64> {
        let n = self.samples.max(1) as f64;
        self.equity_sum.iter().map(|s| s / n).collect()
    }

    /// Standard error of each player's equity estimate.
    pub fn standard_error(&self) -> Vec<f64> {
        let n = self.samples.max(1) as f64;
        self.equity_sum
            .iter()
            .zip(&self.equity_sq_sum)
            .map(|(s, sq)| {
                let mean = s / n;
                ((sq / n - mean * mean).max(0.0) / n).sqrt()
            })
            .collect()
    }
}

/// Weighted 2-card combos a range player can hold given the known cards.
type RangeCombos = Vec<(Deck, Decimal)>;

// Consecutive conflicting draws allowed before a sample is declared
// impossible (every combination of combos shares a card).
const MAX_CONFLICT_REDRAWS: u32 = 100_000;

impl HoldemGameEvaluation {
    /// Filters each range to combos that use only cards still in the deck
    /// (not on the board, dead, or held by a fixed player) with positive weight.
    fn range_combos(
        game_state: &HoldemGameState,
        ranges: &[HoldemRange],
    ) -> Result<Vec<RangeCombos>, DucyError> {
        let fs = &game_state.flop_game_state;
        if fs.hole_cards().len() + ranges.len() > MAX_PLAYERS {
            return Err(DucyError::TooManyPlayers);
        }
        let available = fs.remaining_cards();
        if (available.num_cards() as usize) < 2 * ranges.len() + fs.cards_needed() {
            return Err(DucyError::NotEnoughCards);
        }
        ranges
            .iter()
            .map(|range| {
                let mut combos: RangeCombos = range
                    .iter()
                    .filter(|c| {
                        c.get_weight() > Decimal::ZERO && available.has_cards(&c.get_deck())
                    })
                    .map(|c| (c.get_deck(), c.get_weight()))
                    .collect();
                if combos.is_empty() {
                    return Err(DucyError::InvalidRange);
                }
                combos.sort_by_key(|c| u64::from(c.0));
                Ok(combos)
            })
            .collect()
    }

    /// Exact range-vs-range equity: every non-conflicting combination of range
    /// combos (weighted by the product of their weights) across every runout.
    ///
    /// Players already added to `game_state` keep their hands and come first;
    /// `ranges` are the players after them. Use [`Self::exact_range_evaluations`]
    /// to check the cost first, since it grows with range sizes times runouts.
    pub fn range_equity(
        &self,
        game_state: &HoldemGameState,
        ranges: &[HoldemRange],
    ) -> Result<Vec<Decimal>, DucyError> {
        let combos = Self::range_combos(game_state, ranges)?;
        let fs = &game_state.flop_game_state;
        let mut hands: Vec<Deck> = fs.hole_cards().to_vec();
        let num_fixed = hands.len();
        hands.resize(num_fixed + ranges.len(), Deck::empty());

        let mut acc = ExactRangeAcc {
            community: fs.get_community_cards(),
            available: fs.remaining_cards(),
            cards_needed: fs.cards_needed(),
            totals: vec![Decimal::ZERO; hands.len()],
            total_weight: Decimal::ZERO,
        };
        acc.visit(&combos, &mut hands, num_fixed, Deck::empty(), Decimal::ONE);

        if acc.total_weight.is_zero() {
            return Err(DucyError::InvalidRange);
        }
        Ok(acc.totals.iter().map(|t| t / acc.total_weight).collect())
    }

    /// Upper bound on the hand evaluations [`Self::range_equity`] would run:
    /// the product of range sizes times the number of runouts, times players.
    pub fn exact_range_evaluations(
        &self,
        game_state: &HoldemGameState,
        ranges: &[HoldemRange],
    ) -> Result<u64, DucyError> {
        let combos = Self::range_combos(game_state, ranges)?;
        let fs = &game_state.flop_game_state;
        let deck_left = fs.remaining_cards().num_cards() as u64 - 2 * ranges.len() as u64;
        let needed = fs.cards_needed() as u64;
        let runouts = (0..needed).fold(1u64, |acc, i| acc * (deck_left - i) / (i + 1));
        let players = (fs.hole_cards().len() + ranges.len()) as u64;
        Ok(combos
            .iter()
            .fold(runouts.saturating_mul(players), |acc, c| {
                acc.saturating_mul(c.len() as u64)
            }))
    }

    /// Monte Carlo range-vs-range equity. Each sample draws a combo from every
    /// range in proportion to its weight, redraws all of them if any two share
    /// a card (so the result matches [`Self::range_equity`]'s weighting), then
    /// deals the rest of the board from the remaining cards.
    ///
    /// Players already added to `game_state` come first, then `ranges`.
    /// Returns sums over samples, so results from several calls can be added.
    /// Errors with `InvalidRange` if a range has no usable combos or the
    /// ranges can't be dealt without sharing a card.
    pub fn sample_range_equity(
        &self,
        game_state: &HoldemGameState,
        ranges: &[HoldemRange],
        samples: usize,
    ) -> Result<RangeEquitySamples, DucyError> {
        self.sample_range_equity_seeded(game_state, ranges, samples, None)
    }

    /// Like `sample_range_equity`; a `seed` makes both the weighted combo
    /// draws and the boards reproducible.
    pub fn sample_range_equity_seeded(
        &self,
        game_state: &HoldemGameState,
        ranges: &[HoldemRange],
        samples: usize,
        seed: Option<u64>,
    ) -> Result<RangeEquitySamples, DucyError> {
        let combos = Self::range_combos(game_state, ranges)?;
        let cumulative: Vec<Vec<f64>> = combos
            .iter()
            .map(|cs| {
                cs.iter()
                    .scan(0.0, |total, (_, w)| {
                        *total += f64::try_from(*w).unwrap_or(0.0);
                        Some(*total)
                    })
                    .collect()
            })
            .collect();

        let fs = &game_state.flop_game_state;
        let community = fs.get_community_cards();
        let cards_needed = fs.cards_needed();
        let mut hands: Vec<Deck> = fs.hole_cards().to_vec();
        let num_fixed = hands.len();
        hands.resize(num_fixed + ranges.len(), Deck::empty());

        let mut dealer = CardDealer::maybe_seeded(fs.remaining_cards(), seed);
        let mut result = RangeEquitySamples {
            samples: samples as u64,
            equity_sum: vec![0.0; hands.len()],
            equity_sq_sum: vec![0.0; hands.len()],
        };

        for _ in 0..samples {
            let mut redraws = 0;
            let used = 'draw: loop {
                let mut used = Deck::empty();
                for (r, cum) in cumulative.iter().enumerate() {
                    let total = cum[cum.len() - 1];
                    let x = dealer.random_f64(total);
                    let idx = cum.partition_point(|&c| c <= x).min(cum.len() - 1);
                    let combo = combos[r][idx].0;
                    if u64::from(used) & u64::from(combo) != 0 {
                        redraws += 1;
                        if redraws > MAX_CONFLICT_REDRAWS {
                            return Err(DucyError::InvalidRange);
                        }
                        continue 'draw;
                    }
                    used |= combo;
                    hands[num_fixed + r] = combo;
                }
                break used;
            };

            dealer.reset();
            let board = community | dealer.deal_excluding(cards_needed, used);
            let tracker = holdem_winners(&board, &hands);
            let share = 1.0 / tracker.winners().len() as f64;
            for &w in tracker.winners() {
                result.equity_sum[w] += share;
                result.equity_sq_sum[w] += share * share;
            }
        }
        Ok(result)
    }
}

struct ExactRangeAcc {
    community: Deck,
    available: Deck,
    cards_needed: usize,
    totals: Vec<Decimal>,
    total_weight: Decimal,
}

impl ExactRangeAcc {
    fn visit(
        &mut self,
        combos: &[RangeCombos],
        hands: &mut [Deck],
        next: usize,
        used: Deck,
        weight: Decimal,
    ) {
        let Some((first, rest)) = combos.split_first() else {
            let mut remaining = self.available;
            remaining -= used;
            let community = self.community;
            let runouts: Vec<Deck> = if self.cards_needed == 0 {
                vec![community]
            } else {
                remaining
                    .enumerate_combinations(self.cards_needed)
                    .map(|c| community | c)
                    .collect()
            };
            let hands: &[Deck] = hands;
            let equity = crate::games::accumulate_equity(runouts, hands.len(), |board, shares| {
                holdem_winners(board, hands).distribute(shares);
            });
            for (t, e) in self.totals.iter_mut().zip(equity) {
                *t += e * weight;
            }
            self.total_weight += weight;
            return;
        };
        for &(combo, w) in first {
            if u64::from(used) & u64::from(combo) != 0 {
                continue;
            }
            hands[next] = combo;
            let mut now_used = used;
            now_used |= combo;
            self.visit(rest, hands, next + 1, now_used, weight * w);
        }
    }
}

/// A Hold'em hand range supporting standard poker range notation.
///
/// Supported patterns:
/// - Pair: `TT`, `TT+`, `77-TT`
/// - Offsuit: `AKo`, `AQo+`, `A8o-ATo`
/// - Suited: `AKs`, `AJs+`, `A8s-ATs`
/// - Mixed (both suited and offsuit): `AK`, `AQ+`
/// - Specific combo: `AhKs`
pub struct HoldemRange {
    range_base: RangeBase,
}

impl HoldemRange {
    /// Creates an empty range.
    pub fn new() -> Self {
        Self {
            range_base: RangeBase::new(),
        }
    }

    fn add_pair_combos(&mut self, rank: Rank, weight: Decimal) {
        let suits: Vec<Suit> = Suit::iter().collect();
        for i in 0..suits.len() {
            for j in (i + 1)..suits.len() {
                let mut deck = Deck::empty();
                deck.insert_cards([Card::new(rank, suits[i]), Card::new(rank, suits[j])].iter());
                self.range_base.add_deck_weight(deck, weight);
            }
        }
    }

    fn add_pair_range(
        &mut self,
        low_rank: Rank,
        high_rank: Rank,
        weight: Decimal,
    ) -> Result<(), DucyError> {
        let (low, high) = if RankOrder::AceIsHigh.cmp(low_rank, high_rank) == Ordering::Greater {
            (high_rank, low_rank)
        } else {
            (low_rank, high_rank)
        };
        for rank in RankOrder::AceIsHigh.get_ranks_between(&low, None) {
            self.add_pair_combos(rank, weight);
            if rank == high {
                break;
            }
        }
        Ok(())
    }

    fn add_offsuit_combo(&mut self, first_rank: Rank, second_rank: Rank, weight: Decimal) {
        for s1 in Suit::iter() {
            for s2 in Suit::iter() {
                if s1 != s2 {
                    let mut deck = Deck::empty();
                    deck.insert_cards(
                        [Card::new(first_rank, s1), Card::new(second_rank, s2)].iter(),
                    );
                    self.range_base.add_deck_weight(deck, weight);
                }
            }
        }
    }

    fn add_suited_combo(&mut self, first_rank: Rank, second_rank: Rank, weight: Decimal) {
        for s in Suit::iter() {
            let mut deck = Deck::empty();
            deck.insert_cards([Card::new(first_rank, s), Card::new(second_rank, s)].iter());
            self.range_base.add_deck_weight(deck, weight);
        }
    }

    fn add_offsuit_range(
        &mut self,
        high_rank: Rank,
        low_rank: Rank,
        weight: Decimal,
    ) -> Result<(), DucyError> {
        if RankOrder::AceIsHigh.cmp(high_rank, low_rank) != Ordering::Greater {
            return Err(DucyError::InvalidRange);
        }
        for rank in RankOrder::AceIsHigh.get_ranks_between(&low_rank, Some(&high_rank)) {
            self.add_offsuit_combo(high_rank, rank, weight);
        }
        Ok(())
    }

    fn add_suited_range(
        &mut self,
        high_rank: Rank,
        low_rank: Rank,
        weight: Decimal,
    ) -> Result<(), DucyError> {
        if RankOrder::AceIsHigh.cmp(high_rank, low_rank) != Ordering::Greater {
            return Err(DucyError::InvalidRange);
        }
        for rank in RankOrder::AceIsHigh.get_ranks_between(&low_rank, Some(&high_rank)) {
            self.add_suited_combo(high_rank, rank, weight);
        }
        Ok(())
    }

    fn parse_dash_range(
        &mut self,
        left: &str,
        right: &str,
        weight: Decimal,
    ) -> Result<(), DucyError> {
        // One side of the dash: two ranks, high first, and an optional
        // "s" or "o".
        let side = |s: &str| -> Result<(Rank, Rank, Option<char>), DucyError> {
            let c: Vec<char> = s.chars().collect();
            let suffix = match c.len() {
                2 => None,
                3 => match c[2].to_ascii_lowercase() {
                    x @ ('s' | 'o') => Some(x),
                    _ => return Err(DucyError::InvalidRange),
                },
                _ => return Err(DucyError::InvalidRange),
            };
            let (a, b) = (Rank::try_from_char(&c[0])?, Rank::try_from_char(&c[1])?);
            Ok(if RankOrder::AceIsHigh.cmp(a, b) == Ordering::Less {
                (b, a, suffix)
            } else {
                (a, b, suffix)
            })
        };
        let (lh, ll, ls) = side(left)?;
        let (rh, rl, rs) = side(right)?;

        // Pairs: "77-TT".
        if lh == ll || rh == rl {
            if lh != ll || rh != rl || ls.is_some() || rs.is_some() {
                return Err(DucyError::InvalidRange);
            }
            return self.add_pair_range(lh, rh, weight);
        }
        // The same top card, kickers from one to the other: "A8s-ATs",
        // "A8o-ATo", "A8-AT" (both), and "A5s-A2" (the suffix given once).
        let suffix = match (ls, rs) {
            (Some(a), Some(b)) if a != b => return Err(DucyError::InvalidRange),
            (a, b) => a.or(b),
        };
        if lh != rh {
            return Err(DucyError::InvalidRange);
        }
        let high = lh;
        let (low_start, low_end) = if RankOrder::AceIsHigh.cmp(ll, rl) == Ordering::Less {
            (ll, rl)
        } else {
            (rl, ll)
        };
        for rank in RankOrder::AceIsHigh.get_ranks_between(&low_start, None) {
            if RankOrder::AceIsHigh.cmp(rank, high) != Ordering::Less {
                break;
            }
            if suffix != Some('s') {
                self.add_offsuit_combo(high, rank, weight);
            }
            if suffix != Some('o') {
                self.add_suited_combo(high, rank, weight);
            }
            if rank == low_end {
                break;
            }
        }
        Ok(())
    }

    /// Parses a list of range patterns separated by commas or whitespace,
    /// e.g. `"QQ+, AKs, A5s-A2s"`, each with weight 1.
    pub fn parse(ranges: &str) -> Result<Self, DucyError> {
        let mut range = Self::new();
        for part in ranges
            .split([',', ' ', '\t', '\n'])
            .filter(|p| !p.is_empty())
        {
            range.add(part, Decimal::ONE)?;
        }
        Ok(range)
    }

    /// Adds hands matching a range string with the given weight.
    pub fn add(&mut self, range: &str, weight: Decimal) -> Result<(), DucyError> {
        let range = range.trim();

        // Specific combo: "AhKs" (rank-suit-rank-suit)
        if range.len() == 4 && !range.ends_with('+') {
            let chars: Vec<char> = range.chars().collect();
            if let (Ok(r1), Ok(s1), Ok(r2), Ok(s2)) = (
                Rank::try_from_char(&chars[0]),
                Suit::try_from_char(&chars[1]),
                Rank::try_from_char(&chars[2]),
                Suit::try_from_char(&chars[3]),
            ) {
                let card1 = Card::new(r1, s1);
                let card2 = Card::new(r2, s2);
                if card1 == card2 {
                    return Err(DucyError::InvalidRange);
                }
                let mut deck = Deck::empty();
                deck.insert_cards([card1, card2].iter());
                self.range_base.add_deck_weight(deck, weight);
                return Ok(());
            }
        }

        // Dash ranges: "77-TT", "A8s-ATs"
        if let Some(dash_pos) = range.find('-') {
            return self.parse_dash_range(&range[..dash_pos], &range[dash_pos + 1..], weight);
        }

        let has_plus = range.ends_with('+');
        let base = if has_plus {
            &range[..range.len() - 1]
        } else {
            range
        };
        let base_chars: Vec<char> = base.chars().collect();

        match base_chars.len() {
            2 => {
                let r1 = Rank::try_from_char(&base_chars[0])?;
                let r2 = Rank::try_from_char(&base_chars[1])?;

                if r1 == r2 {
                    if has_plus {
                        self.add_pair_range(r1, Rank::Ace, weight)
                    } else {
                        self.add_pair_combos(r1, weight);
                        Ok(())
                    }
                } else {
                    let (high, low) = if RankOrder::AceIsHigh.cmp(r1, r2) == Ordering::Greater {
                        (r1, r2)
                    } else {
                        (r2, r1)
                    };
                    if has_plus {
                        self.add_offsuit_range(high, low, weight)?;
                        self.add_suited_range(high, low, weight)
                    } else {
                        self.add_offsuit_combo(high, low, weight);
                        self.add_suited_combo(high, low, weight);
                        Ok(())
                    }
                }
            }
            3 => {
                let r1 = Rank::try_from_char(&base_chars[0])?;
                let r2 = Rank::try_from_char(&base_chars[1])?;
                let suffix = base_chars[2].to_ascii_lowercase();

                if r1 == r2 {
                    return Err(DucyError::InvalidRange);
                }

                let (high, low) = if RankOrder::AceIsHigh.cmp(r1, r2) == Ordering::Greater {
                    (r1, r2)
                } else {
                    (r2, r1)
                };

                match suffix {
                    'o' => {
                        if has_plus {
                            self.add_offsuit_range(high, low, weight)
                        } else {
                            self.add_offsuit_combo(high, low, weight);
                            Ok(())
                        }
                    }
                    's' => {
                        if has_plus {
                            self.add_suited_range(high, low, weight)
                        } else {
                            self.add_suited_combo(high, low, weight);
                            Ok(())
                        }
                    }
                    _ => Err(DucyError::InvalidRange),
                }
            }
            _ => Err(DucyError::InvalidRange),
        }
    }
}

impl Default for HoldemRange {
    fn default() -> Self {
        Self::new()
    }
}

/// What a range's text covers, for showing it: the hand classes it touches
/// ("AA", "AKs", "AKo"), strongest first, and its two-card combos.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RangeReport {
    /// Classes with at least one combo in the range, in grid order: by top
    /// card, then kicker, suited before offsuit.
    pub classes: Vec<String>,
    /// Combos in the range, not counting any that use a dead card.
    pub combos: u64,
}

/// Why range text didn't parse: the term at fault and where it starts.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RangeTextError {
    /// The term, as written.
    pub term: String,
    /// Its byte offset in the text.
    pub start: usize,
    /// A message to show, naming the term.
    pub message: String,
}

impl std::fmt::Display for RangeTextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// The class of a two-card hand: "AA", "AKs" or "AKo".
pub fn hand_class(a: Card, b: Card) -> String {
    let (hi, lo) = if RankOrder::AceIsHigh.cmp(a.rank(), b.rank()) == Ordering::Less {
        (b, a)
    } else {
        (a, b)
    };
    if hi.rank() == lo.rank() {
        format!("{}{}", hi.rank(), lo.rank())
    } else {
        let s = if hi.suit() == lo.suit() { 's' } else { 'o' };
        format!("{}{}{s}", hi.rank(), lo.rank())
    }
}

impl HoldemRange {
    /// Parses range text like [`HoldemRange::parse`], and reports what it
    /// covers, leaving out combos that use a card in `dead`. A term that
    /// doesn't parse is named, with where it starts, so a page can point
    /// at it.
    pub fn report(text: &str, dead: Deck) -> Result<RangeReport, RangeTextError> {
        let mut range = Self::new();
        let mut start = None;
        for (i, ch) in text
            .char_indices()
            .chain(std::iter::once((text.len(), ',')))
        {
            if matches!(ch, ',' | ' ' | '\t' | '\n' | '\r') {
                if let Some(s) = start.take() {
                    let term = &text[s..i];
                    range.add(term, Decimal::ONE).map_err(|_| RangeTextError {
                        term: term.to_string(),
                        start: s,
                        message: format!(
                            "“{term}” isn’t a hand or range (e.g. QQ+, AKs, A5s-A2s, KQo, AhKh)."
                        ),
                    })?;
                }
            } else if start.is_none() {
                start = Some(i);
            }
        }
        let dead = u64::from(dead);
        let mut combos = 0;
        let mut classes = std::collections::HashSet::new();
        for item in range.iter() {
            let deck = item.get_deck();
            if u64::from(deck) & dead != 0 {
                continue;
            }
            let cards: Vec<Card> = deck.iter(true).collect();
            if let [a, b] = cards[..] {
                combos += 1;
                classes.insert(hand_class(a, b));
            }
        }
        let order = |c: &String| {
            let ch: Vec<char> = c.chars().collect();
            let r = |x: &char| RankOrder::AceIsHigh.get_score(&Rank::try_from_char(x).unwrap());
            let kind = match ch.get(2) {
                None => 0,
                Some('s') => 1,
                _ => 2,
            };
            (
                std::cmp::Reverse(r(&ch[0])),
                std::cmp::Reverse(r(&ch[1])),
                kind,
            )
        };
        let mut classes: Vec<String> = classes.into_iter().collect();
        classes.sort_by_key(order);
        Ok(RangeReport { classes, combos })
    }
}

impl Range for HoldemRange {
    fn iter(&self) -> impl Iterator<Item = crate::deck::range::RangeItem> {
        self.range_base.iter()
    }
}

#[cfg(test)]
mod test {
    use rust_decimal::Decimal;

    use crate::{
        deck::{Card, Deck, Rank, range::Range},
        error::DucyError,
        games::{
            GameEquityEvaluation, GameEvaluation, GameWinner,
            flop_game::FlopGame,
            holdem::{HoldemGameEvaluation, HoldemGameState, HoldemRange},
        },
        ranking::hand_rank::StandardHandRanks,
    };
    use rust_decimal_macros::dec;

    #[test]
    pub fn test_holdem_hand() {
        let mut holdem_hand = HoldemGameState::new();
        holdem_hand
            .add_player(Deck::parse("As Ac").unwrap())
            .unwrap();
        holdem_hand
            .add_player(Deck::parse("ks kd").unwrap())
            .unwrap();

        let hand_evaluation = HoldemGameEvaluation {};

        holdem_hand
            .set_flop(Deck::parse("kc qd js").unwrap())
            .unwrap();

        let winners = hand_evaluation.evaluate_winners(&holdem_hand);

        assert_eq!(winners.len(), 1);
        assert_eq!(
            winners[0],
            GameWinner {
                player_index: 1,
                pot_amount: dec!(1),
                winning_hand: StandardHandRanks::ThreeOfAKind {
                    t: Rank::King,
                    c1: Rank::Queen,
                    c2: Rank::Jack
                }
            }
        );

        let equities = hand_evaluation.evaluate_equity(&holdem_hand);

        assert_eq!(equities.len(), 2);
        assert_eq!(equities[0], dec!(418) / dec!(1980));
        assert_eq!(equities[1], dec!(1562) / dec!(1980));

        holdem_hand.set_turn(Card::parse("Tc").unwrap()).unwrap();

        let winners = hand_evaluation.evaluate_winners(&holdem_hand);

        assert_eq!(winners.len(), 1);
        assert_eq!(
            winners[0],
            GameWinner {
                player_index: 0,
                pot_amount: dec!(1),
                winning_hand: StandardHandRanks::Straight { s: Rank::Ace }
            }
        );

        let equities = hand_evaluation.evaluate_equity(&holdem_hand);

        assert_eq!(equities.len(), 2);
        assert_eq!(equities[0], dec!(33) / dec!(44));
        assert_eq!(equities[1], dec!(11) / dec!(44));

        holdem_hand.set_river(Card::parse("Ad").unwrap()).unwrap();

        let winners = hand_evaluation.evaluate_winners(&holdem_hand);

        assert_eq!(winners.len(), 2);
        assert_eq!(
            winners[0],
            GameWinner {
                player_index: 0,
                pot_amount: dec!(0.5),
                winning_hand: StandardHandRanks::Straight { s: Rank::Ace }
            }
        );
        assert_eq!(
            winners[1],
            GameWinner {
                player_index: 1,
                pot_amount: dec!(0.5),
                winning_hand: StandardHandRanks::Straight { s: Rank::Ace }
            }
        );

        let equities = hand_evaluation.evaluate_equity(&holdem_hand);

        assert_eq!(equities.len(), 2);
        assert_eq!(equities[0], dec!(1) / dec!(2));
        assert_eq!(equities[1], dec!(1) / dec!(2));
    }

    #[test]
    pub fn range_tests() {
        let mut range = HoldemRange::new();
        range.add("AQo+", dec!(1)).unwrap();
        let mut range_1 = vec![
            "As Kc", "As Kd", "As Kh", "Ac Ks", "Ac Kh", "Ac Kd", "Ad Ks", "Ad Kc", "Ad Kh",
            "Ah Ks", "Ah Kd", "Ah Kc", "As Qc", "As Qd", "As Qh", "Ac Qs", "Ac Qh", "Ac Qd",
            "Ad Qs", "Ad Qc", "Ad Qh", "Ah Qs", "Ah Qd", "Ah Qc",
        ];

        range_1.sort();

        let mut actual_range_items: Vec<String> =
            range.iter().map(|x| x.get_deck().to_string()).collect();
        actual_range_items.sort();

        assert_eq!(actual_range_items, range_1);

        range.add("AJs+", dec!(1)).unwrap();

        let mut range_2 = vec![
            "As Ks", "Ac Kc", "Ad Kd", "Ah Kh", "As Qs", "Ah Qh", "Ac Qc", "Ad Qd", "Ac Jc",
            "Ad Jd", "Ah Jh", "As Js",
        ];

        range_1.append(&mut range_2);
        range_1.sort();

        let mut actual_range_items: Vec<String> =
            range.iter().map(|x| x.get_deck().to_string()).collect();
        actual_range_items.sort();

        assert_eq!(actual_range_items, range_1);
    }

    #[test]
    pub fn test_pair_range() {
        let mut range = HoldemRange::new();
        range.add("TT", dec!(1)).unwrap();
        assert_eq!(range.iter().count(), 6); // C(4,2) = 6 combos for one pair
    }

    #[test]
    pub fn test_pair_plus_range() {
        let mut range = HoldemRange::new();
        range.add("QQ+", dec!(1)).unwrap();
        // QQ, KK, AA = 3 pairs * 6 combos = 18
        assert_eq!(range.iter().count(), 18);
    }

    #[test]
    pub fn test_pair_dash_range() {
        let mut range = HoldemRange::new();
        range.add("77-TT", dec!(1)).unwrap();
        // 77, 88, 99, TT = 4 pairs * 6 combos = 24
        assert_eq!(range.iter().count(), 24);
    }

    #[test]
    pub fn test_offsuit_exact() {
        let mut range = HoldemRange::new();
        range.add("AKo", dec!(1)).unwrap();
        assert_eq!(range.iter().count(), 12); // 4*3 = 12 offsuit combos
    }

    #[test]
    pub fn test_suited_exact() {
        let mut range = HoldemRange::new();
        range.add("AKs", dec!(1)).unwrap();
        assert_eq!(range.iter().count(), 4); // 4 suited combos
    }

    #[test]
    pub fn test_mixed_exact() {
        let mut range = HoldemRange::new();
        range.add("AK", dec!(1)).unwrap();
        assert_eq!(range.iter().count(), 16); // 12 offsuit + 4 suited
    }

    #[test]
    pub fn test_mixed_plus() {
        let mut range = HoldemRange::new();
        range.add("AQ+", dec!(1)).unwrap();
        // AQ (16) + AK (16) = 32
        assert_eq!(range.iter().count(), 32);
    }

    #[test]
    pub fn test_specific_combo() {
        let mut range = HoldemRange::new();
        range.add("AhKs", dec!(1)).unwrap();
        assert_eq!(range.iter().count(), 1);
        let item = range.iter().next().unwrap();
        assert_eq!(item.get_deck().to_string(), "Ah Ks");
    }

    #[test]
    pub fn test_suited_dash_range() {
        let mut range = HoldemRange::new();
        range.add("A8s-ATs", dec!(1)).unwrap();
        // A8s, A9s, ATs = 3 * 4 = 12
        assert_eq!(range.iter().count(), 12);
    }

    #[test]
    pub fn test_offsuit_dash_range() {
        let mut range = HoldemRange::new();
        range.add("K9o-KJo", dec!(1)).unwrap();
        // K9o, KTo, KJo = 3 * 12 = 36
        assert_eq!(range.iter().count(), 36);
    }

    #[test]
    pub fn test_offsuit_range_plus_fixed_high_card() {
        let mut range = HoldemRange::new();
        range.add("KTo+", dec!(1)).unwrap();
        // KT, KJ, KQ = 3 combos * 12 = 36
        assert_eq!(range.iter().count(), 36);
    }

    #[test]
    pub fn test_invalid_range_same_card() {
        let mut range = HoldemRange::new();
        assert!(range.add("AsAs", dec!(1)).is_err());
    }

    #[test]
    pub fn test_invalid_range_garbage() {
        let mut range = HoldemRange::new();
        assert!(range.add("xyz", dec!(1)).is_err());
    }

    #[test]
    pub fn test_invalid_pair_with_suit_suffix() {
        let mut range = HoldemRange::new();
        assert!(range.add("TTs", dec!(1)).is_err());
    }

    fn flop_state(players: &[&str], flop: &str) -> HoldemGameState {
        let mut state = HoldemGameState::new();
        for p in players {
            state.add_player(Deck::parse(p).unwrap()).unwrap();
        }
        state.set_flop(Deck::parse(flop).unwrap()).unwrap();
        state
    }

    fn assert_close(a: &[Decimal], b: &[Decimal], tol: Decimal) {
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b) {
            assert!((x - y).abs() < tol, "{a:?} vs {b:?}");
        }
    }

    /// Weighted average of evaluate_equity over every non-conflicting combo pair.
    fn manual_range_equity(
        fixed: &[&str],
        flop: &str,
        r0: &HoldemRange,
        r1: &HoldemRange,
    ) -> Vec<Decimal> {
        let base = flop_state(fixed, flop);
        let available = base.flop_game_state.remaining_cards();
        let n = fixed.len() + 2;
        let mut totals = vec![Decimal::ZERO; n];
        let mut weight = Decimal::ZERO;
        for a in r0.iter() {
            for b in r1.iter() {
                let (da, db) = (a.get_deck(), b.get_deck());
                if !available.has_cards(&da)
                    || !available.has_cards(&db)
                    || u64::from(da) & u64::from(db) != 0
                {
                    continue;
                }
                let mut state = flop_state(fixed, flop);
                state.add_player(da).unwrap();
                state.add_player(db).unwrap();
                let w = a.get_weight() * b.get_weight();
                for (t, e) in totals
                    .iter_mut()
                    .zip(HoldemGameEvaluation {}.evaluate_equity(&state))
                {
                    *t += e * w;
                }
                weight += w;
            }
        }
        totals.iter().map(|t| t / weight).collect()
    }

    #[test]
    pub fn test_range_equity_single_combos_match_evaluate_equity() {
        let state = flop_state(&[], "Kc Qd Js");
        let ranges = [
            HoldemRange::parse("AsAc").unwrap(),
            HoldemRange::parse("KsKd").unwrap(),
        ];
        let eq = HoldemGameEvaluation {}
            .range_equity(&state, &ranges)
            .unwrap();
        assert_close(
            &eq,
            &[dec!(418) / dec!(1980), dec!(1562) / dec!(1980)],
            dec!(1e-20),
        );
    }

    #[test]
    pub fn test_range_equity_matches_manual_weighted_average() {
        let r0 = HoldemRange::parse("QQ").unwrap();
        let r1 = HoldemRange::parse("AKs").unwrap();
        let state = flop_state(&[], "Kc Qd Js");
        let eq = HoldemGameEvaluation {}
            .range_equity(
                &state,
                &[
                    HoldemRange::parse("QQ").unwrap(),
                    HoldemRange::parse("AKs").unwrap(),
                ],
            )
            .unwrap();
        assert_close(
            &eq,
            &manual_range_equity(&[], "Kc Qd Js", &r0, &r1),
            dec!(1e-20),
        );
    }

    #[test]
    pub fn test_range_equity_with_fixed_player_and_weights() {
        let mut r0 = HoldemRange::new();
        r0.add("AsAh", dec!(3)).unwrap();
        r0.add("KsKh", dec!(1)).unwrap();
        r0.add("QsQh", dec!(0)).unwrap();
        let r1 = HoldemRange::parse("JJ").unwrap();
        let fixed = ["Tc Td"];
        let state = flop_state(&fixed, "2c 7d 9h");

        let mut r0_copy = HoldemRange::new();
        r0_copy.add("AsAh", dec!(3)).unwrap();
        r0_copy.add("KsKh", dec!(1)).unwrap();
        let expected = manual_range_equity(&fixed, "2c 7d 9h", &r0_copy, &r1);

        let eq = HoldemGameEvaluation {}
            .range_equity(&state, &[r0, r1])
            .unwrap();
        assert_eq!(eq.len(), 3);
        assert_close(&eq, &expected, dec!(1e-20));
    }

    #[test]
    pub fn test_sample_range_equity_close_to_exact() {
        let mut state = flop_state(&[], "Kc 8d 3s");
        state.set_turn(Card::parse("2h").unwrap()).unwrap();
        let ranges = || {
            [
                HoldemRange::parse("QQ+, AK").unwrap(),
                HoldemRange::parse("88-TT, KQs").unwrap(),
            ]
        };
        let eval = HoldemGameEvaluation {};

        let exact = eval.range_equity(&state, &ranges()).unwrap();
        let sampled = eval.sample_range_equity(&state, &ranges(), 20_000).unwrap();
        assert_eq!(sampled.samples, 20_000);
        assert!((sampled.equity_sum.iter().sum::<f64>() - 20_000.0).abs() < 1e-6);
        for ((e, s), se) in exact
            .iter()
            .zip(sampled.equity())
            .zip(sampled.standard_error())
        {
            let e = f64::try_from(*e).unwrap();
            assert!(se > 0.0 && se < 0.01, "se {se}");
            assert!(
                (e - s).abs() < 5.0 * se + 1e-3,
                "exact {e} vs sampled {s} (se {se})"
            );
        }
    }

    #[test]
    pub fn test_sample_range_equity_seeded_weighted() {
        let state = flop_state(&["Tc Td"], "2c 7d 9h");
        let ranges = || {
            let mut r0 = HoldemRange::new();
            r0.add("AsAh", dec!(3)).unwrap();
            r0.add("KsKh", dec!(1)).unwrap();
            [r0, HoldemRange::parse("JJ").unwrap()]
        };
        let eval = HoldemGameEvaluation {};

        let run = |seed| {
            eval.sample_range_equity_seeded(&state, &ranges(), 20_000, seed)
                .unwrap()
        };
        let a = run(Some(7));
        assert_eq!(a.equity_sum, run(Some(7)).equity_sum);
        assert_ne!(a.equity_sum, run(Some(8)).equity_sum);

        // Weighted draws converge on the weighted exact result.
        let exact = eval.range_equity(&state, &ranges()).unwrap();
        for ((e, s), se) in exact.iter().zip(a.equity()).zip(a.standard_error()) {
            let e = f64::try_from(*e).unwrap();
            assert!(
                (e - s).abs() < 5.0 * se + 1e-3,
                "exact {e} vs sampled {s} (se {se})"
            );
        }
    }

    #[test]
    pub fn test_range_errors() {
        let eval = HoldemGameEvaluation {};
        let state = flop_state(&[], "As 7d 2c");

        let unusable = [
            HoldemRange::parse("AsAh").unwrap(),
            HoldemRange::parse("KK").unwrap(),
        ];
        assert_eq!(
            eval.range_equity(&state, &unusable),
            Err(DucyError::InvalidRange)
        );
        assert!(eval.sample_range_equity(&state, &unusable, 10).is_err());

        let clash = [
            HoldemRange::parse("KsKh").unwrap(),
            HoldemRange::parse("KsKh").unwrap(),
        ];
        assert_eq!(
            eval.range_equity(&state, &clash),
            Err(DucyError::InvalidRange)
        );
        assert!(matches!(
            eval.sample_range_equity(&state, &clash, 10),
            Err(DucyError::InvalidRange)
        ));

        let too_many: Vec<_> = (0..11)
            .map(|_| HoldemRange::parse("22+").unwrap())
            .collect();
        assert!(matches!(
            eval.exact_range_evaluations(&state, &too_many),
            Err(DucyError::TooManyPlayers)
        ));
    }

    #[test]
    pub fn test_dead_cards_removed_from_ranges() {
        let mut state = flop_state(&[], "2c 7d 9h");
        state.set_turn(Card::parse("3s").unwrap()).unwrap();
        let eval = HoldemGameEvaluation {};
        let ranges = || {
            [
                HoldemRange::parse("AA").unwrap(),
                HoldemRange::parse("KsKh").unwrap(),
            ]
        };
        // 44 runouts * 2 players * 6 AA combos * 1 KK combo
        assert_eq!(
            eval.exact_range_evaluations(&state, &ranges()),
            Ok(44 * 2 * 6)
        );
        state.add_dead_cards(Deck::parse("Ah").unwrap()).unwrap();
        // One fewer card left, and AA loses the 3 combos containing Ah.
        assert_eq!(
            eval.exact_range_evaluations(&state, &ranges()),
            Ok(43 * 2 * 3)
        );
    }

    #[test]
    pub fn test_equity_chunks_merge_to_exact() {
        let state = flop_state(&["As Ac", "Ks Kd", "7h 6h"], "Kc Qd Js");
        let eval = HoldemGameEvaluation {};
        let total = eval.runout_count(&state);
        assert_eq!(total, 43 * 42 / 2);

        let mut merged = crate::games::EquityShares::default();
        let mut start = 0;
        while start < total {
            merged.merge(&eval.evaluate_equity_chunk(&state, start, 137));
            start += 137;
        }
        assert_eq!(merged.runouts, total);
        assert_eq!(merged.equity(), eval.evaluate_equity(&state));
        assert_eq!(eval.evaluate_equity_chunk(&state, total, 10).runouts, 0);
    }

    #[test]
    pub fn test_parse_range_list() {
        let range = HoldemRange::parse("QQ+, AKs A5s-A2s").unwrap();
        assert_eq!(range.iter().count(), 18 + 4 + 16);
        assert!(HoldemRange::parse("QQ+, nope").is_err());
    }

    #[test]
    pub fn test_deal_excluding_never_deals_excluded() {
        let exclude = Deck::parse("As Ks Qs Js Ts 9s 8s").unwrap();
        let mut dealer = crate::games::CardDealer::new(Deck::all_cards());
        for _ in 0..1_000 {
            dealer.reset();
            let hand = dealer.deal_excluding(5, exclude);
            assert_eq!(hand.num_cards(), 5);
            assert_eq!(u64::from(hand) & u64::from(exclude), 0);
        }
    }

    #[test]
    pub fn test_add_player_wrong_card_count() {
        let mut game = HoldemGameState::new();
        assert!(game.add_player(Deck::parse("As Ks Qs").unwrap()).is_err());
    }

    #[test]
    pub fn test_add_player_duplicate_cards() {
        let mut game = HoldemGameState::new();
        game.add_player(Deck::parse("As Ks").unwrap()).unwrap();
        assert!(game.add_player(Deck::parse("As Qd").unwrap()).is_err());
    }

    #[test]
    pub fn test_set_flop_wrong_card_count() {
        let mut game = HoldemGameState::new();
        game.add_player(Deck::parse("As Ks").unwrap()).unwrap();
        assert!(game.set_flop(Deck::parse("Qd Jd").unwrap()).is_err());
    }

    #[test]
    pub fn test_set_turn_before_flop() {
        let mut game = HoldemGameState::new();
        game.add_player(Deck::parse("As Ks").unwrap()).unwrap();
        assert!(game.set_turn(Card::parse("2c").unwrap()).is_err());
    }

    #[test]
    pub fn test_set_river_before_turn() {
        let mut game = HoldemGameState::new();
        game.add_player(Deck::parse("As Ks").unwrap()).unwrap();
        game.set_flop(Deck::parse("Qd Jd Td").unwrap()).unwrap();
        assert!(game.set_river(Card::parse("2c").unwrap()).is_err());
    }

    #[test]
    pub fn test_set_turn_card_not_in_deck() {
        let mut game = HoldemGameState::new();
        game.add_player(Deck::parse("As Ks").unwrap()).unwrap();
        game.set_flop(Deck::parse("Qd Jd Td").unwrap()).unwrap();
        assert!(game.set_turn(Card::parse("As").unwrap()).is_err());
    }

    fn report(text: &str) -> super::RangeReport {
        HoldemRange::report(text, Deck::empty()).unwrap()
    }

    #[test]
    pub fn test_range_report() {
        let r = report("AKo, AA, AKs");
        assert_eq!(r.classes, ["AA", "AKs", "AKo"]);
        assert_eq!(r.combos, 6 + 4 + 12);
        // Overlapping terms count each combo once.
        assert_eq!(report("AK, AKs, AKo, AsKs").combos, 16);
        // Dead cards remove the combos that use them.
        let dead = HoldemRange::report("AA, AKs, AKo", Deck::parse("As").unwrap()).unwrap();
        assert_eq!(dead.combos, 3 + 3 + 9);
        // A specific combo is its class.
        assert_eq!(report("AhKh").classes, ["AKs"]);
        assert_eq!(report("QQ+").classes, ["AA", "KK", "QQ"]);
    }

    #[test]
    pub fn test_range_dash_forms() {
        // The same as the page's grammar: no suffix covers both, a suffix on
        // one side applies to the whole range, either order.
        assert_eq!(
            report("A8-AT").classes,
            ["ATs", "ATo", "A9s", "A9o", "A8s", "A8o"]
        );
        assert_eq!(report("A5s-A2").classes, ["A5s", "A4s", "A3s", "A2s"]);
        assert_eq!(report("ATo-A8o").classes, report("A8o-ATo").classes);
        assert_eq!(report("TT-77").classes, ["TT", "99", "88", "77"]);
        for bad in ["A8s-ATo", "A8-KT", "77-AK", "A8x-AT"] {
            assert!(HoldemRange::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    pub fn test_range_report_errors() {
        let e = HoldemRange::report("QQ+, AKs, AKx, 22", Deck::empty()).unwrap_err();
        assert_eq!((e.term.as_str(), e.start), ("AKx", 10));
        assert!(e.message.contains("AKx"));
        let e = HoldemRange::report("  zz", Deck::empty()).unwrap_err();
        assert_eq!((e.term.as_str(), e.start), ("zz", 2));
        assert_eq!(report("").combos, 0);
    }
}
