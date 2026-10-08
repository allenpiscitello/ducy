use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::{
    deck::{Card, Deck},
    error::DucyError,
    games::{
        EQUITY_SCALE, EquityShares, FastWinnerTracker, GameState, GameWinner, WinnerTracker,
        flop_game::{FlopGame, FlopGameState},
    },
    ranking::{
        hand_rank::{StandardHandRanker, StandardHandRanks},
        low_hand_rank::{LowHandRanker, LowHandRanks},
    },
};

/// Omaha Hi-Lo 8-or-Better game state (configurable number of hole cards per player, split pot).
pub struct OmahaHiLoGameState {
    pub(crate) flop_game_state: FlopGameState,
}

impl OmahaHiLoGameState {
    /// Hole cards per player.
    pub fn cards_per_player(&self) -> usize {
        self.flop_game_state.cards_per_player()
    }

    /// Creates a new Omaha Hi-Lo game state with the given number of hole cards per player.
    pub fn new(cards_per_player: u32) -> Self {
        Self {
            flop_game_state: FlopGameState::new(cards_per_player),
        }
    }
}

impl Default for OmahaHiLoGameState {
    fn default() -> Self {
        Self::new(4)
    }
}

impl FlopGame for OmahaHiLoGameState {
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

impl GameState for OmahaHiLoGameState {}

/// Result of a Hi-Lo hand evaluation: high winners and optional low winners.
pub struct HiLoResult {
    /// The best high hand (more than one on a tie).
    pub high_winners: Vec<GameWinner<StandardHandRanks>>,
    /// The best qualifying 8-or-better low; empty if no one has one.
    pub low_winners: Vec<GameWinner<LowHandRanks>>,
}

/// Evaluates Omaha Hi-Lo hands (high and low).
pub struct OmahaHiLoGameEvaluation {}

impl OmahaHiLoGameEvaluation {
    /// Evaluates a single board state, returning high and low winners with
    /// correct pot shares. If no low qualifies, high scoops the full pot.
    pub fn evaluate_winners(&self, game_state: &OmahaHiLoGameState) -> HiLoResult {
        let community = game_state.get_community_cards();

        let mut high_tracker = WinnerTracker::new();
        let mut low_tracker: WinnerTracker<LowHandRanks> = WinnerTracker::new();

        for community_cards_of_3 in community.enumerate_combinations(3) {
            let board_suit = community_cards_of_3.single_suit_index();
            let board_paired = community_cards_of_3.has_rank_pair();
            for (i, player) in game_state.get_player_hole_cards().enumerate() {
                for player_cards_of_2 in player.enumerate_combinations(2) {
                    let flush_possible =
                        board_suit.is_some_and(|s| player_cards_of_2.all_in_suit_index(s));
                    let combined = community_cards_of_3 | player_cards_of_2;
                    if let Some(rank) = StandardHandRanker::get_rank_at_least_with_hints(
                        &combined,
                        high_tracker.best_hand(),
                        flush_possible,
                        board_paired,
                    ) {
                        high_tracker.consider(i, rank);
                    }
                    if let Some(rank) =
                        LowHandRanker::get_rank_at_least(&combined, low_tracker.best_hand())
                    {
                        low_tracker.consider(i, rank);
                    }
                }
            }
        }

        let has_low = low_tracker.best_hand().is_some()
            && low_tracker.best_hand() != Some(LowHandRanks::NoLow);

        if has_low {
            let mut high_winners = high_tracker.into_results();
            let mut low_winners = low_tracker.into_results();
            for w in &mut high_winners {
                w.pot_amount *= dec!(0.5);
            }
            for w in &mut low_winners {
                w.pot_amount *= dec!(0.5);
            }
            HiLoResult {
                high_winners,
                low_winners,
            }
        } else {
            HiLoResult {
                high_winners: high_tracker.into_results(),
                low_winners: vec![],
            }
        }
    }

    /// Evaluates equity across all possible runouts, returning each player's
    /// expected pot share accounting for high/low splits and scooping.
    pub fn evaluate_equity(&self, game_state: &OmahaHiLoGameState) -> Vec<Decimal> {
        let runouts = game_state
            .flop_game_state
            .enumerate_runout_community_cards()
            .collect();
        Self::shares_over_runouts(game_state.flop_game_state.hole_cards(), runouts).equity()
    }

    /// Estimates equity from `samples` random runouts instead of enumerating all of them.
    pub fn sample_equity(&self, game_state: &OmahaHiLoGameState, samples: usize) -> Vec<Decimal> {
        self.sample_equity_seeded(game_state, samples, None)
    }

    /// Like `sample_equity`; a `seed` makes the result reproducible.
    pub fn sample_equity_seeded(
        &self,
        game_state: &OmahaHiLoGameState,
        samples: usize,
        seed: Option<u64>,
    ) -> Vec<Decimal> {
        let runouts = game_state
            .flop_game_state
            .sample_runout_community_cards(samples, seed);
        Self::shares_over_runouts(game_state.flop_game_state.hole_cards(), runouts).equity()
    }

    /// Number of runouts exact equity enumerates.
    pub fn runout_count(&self, game_state: &OmahaHiLoGameState) -> u64 {
        game_state.flop_game_state.runout_count()
    }

    /// Equity totals for runouts `start..start + count` of the exact
    /// enumeration. Merging every chunk gives the same result as
    /// `evaluate_equity`, so chunks can run on separate threads or workers.
    pub fn evaluate_equity_chunk(
        &self,
        game_state: &OmahaHiLoGameState,
        start: u64,
        count: u64,
    ) -> EquityShares {
        let runouts = game_state.flop_game_state.runout_chunk(start, count);
        Self::shares_over_runouts(game_state.flop_game_state.hole_cards(), runouts)
    }

    fn shares_over_runouts(hole_cards: &[Deck], runouts: Vec<Deck>) -> EquityShares {
        let player_combos: Vec<HiLoCombos> = hole_cards.iter().map(hilo_combos).collect();
        crate::games::accumulate_shares(runouts, hole_cards.len(), |community, shares| {
            hilo_distribute(community, &player_combos, shares);
        })
    }
}

/// Each 2-card combo of a hand with its suit (if suited) and low-rank mask.
/// The mask is kept only when the two cards are distinct low ranks, since
/// only then can they make a qualifying low.
pub(crate) type HiLoCombos = Vec<(Deck, Option<usize>, u32)>;

pub(crate) fn hilo_combos(hand: &Deck) -> HiLoCombos {
    hand.enumerate_combinations(2)
        .map(|d| {
            let low = LowHandRanker::low_rank_mask(&d);
            let low = if low.count_ones() == 2 { low } else { 0 };
            (d, d.single_suit_index(), low)
        })
        .collect()
}

/// Adds one complete board's pot shares (in `EQUITY_SCALE` units) to `shares`:
/// half to the best high and half to the best qualifying low, or the whole
/// pot to the best high when no low qualifies.
pub(crate) fn hilo_distribute(community: &Deck, player_combos: &[HiLoCombos], shares: &mut [u64]) {
    let mut high_tracker = FastWinnerTracker::new();
    let mut low_tracker = FastWinnerTracker::new();

    for community_cards_of_3 in community.enumerate_combinations(3) {
        let board_suit = community_cards_of_3.single_suit_index();
        let board_paired = community_cards_of_3.has_rank_pair();
        let board_low = LowHandRanker::low_rank_mask(&community_cards_of_3);
        let board_low = if board_low.count_ones() == 3 {
            board_low
        } else {
            0
        };
        for (i, combos) in player_combos.iter().enumerate() {
            for &(player_deck, player_suit, player_low) in combos {
                let flush_possible = board_suit.is_some() && board_suit == player_suit;
                let combined = community_cards_of_3 | player_deck;
                if let Some(score) = StandardHandRanker::fast_score_at_least(
                    &combined,
                    high_tracker.best_score(),
                    flush_possible,
                    board_paired,
                ) {
                    high_tracker.consider(i, score);
                }
                if board_low != 0 && player_low != 0 {
                    let low_score = LowHandRanker::score_from_mask(board_low | player_low);
                    if low_score != 0 {
                        low_tracker.consider(i, low_score);
                    }
                }
            }
        }
    }

    if low_tracker.best_score() > 0 {
        let half_scale = EQUITY_SCALE / 2;
        high_tracker.distribute_scaled(shares, half_scale);
        low_tracker.distribute_scaled(shares, half_scale);
    } else {
        high_tracker.distribute(shares);
    }
}

#[cfg(test)]
mod test {
    use rust_decimal_macros::dec;

    use crate::{
        deck::{Card, Deck},
        games::{
            flop_game::FlopGame,
            omaha_hilo::{OmahaHiLoGameEvaluation, OmahaHiLoGameState},
        },
    };

    #[test]
    fn test_hilo_high_scoops_no_low() {
        let mut state = OmahaHiLoGameState::new(4);
        state
            .add_player(Deck::parse("Ks Kd Qc Qd").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("Js Jd Tc Td").unwrap())
            .unwrap();
        state.set_flop(Deck::parse("Kc 9h 9d").unwrap()).unwrap();
        state.set_turn(Card::parse("Jh").unwrap()).unwrap();
        state.set_river(Card::parse("Th").unwrap()).unwrap();

        let evaluator = OmahaHiLoGameEvaluation {};
        let result = evaluator.evaluate_winners(&state);

        assert_eq!(result.high_winners.len(), 1);
        assert_eq!(result.high_winners[0].player_index(), 0);
        assert_eq!(result.high_winners[0].pot_amount(), dec!(1));
        assert!(result.low_winners.is_empty());
    }

    #[test]
    fn test_hilo_split_pot() {
        let mut state = OmahaHiLoGameState::new(4);
        // Player 0: strong high hand
        state
            .add_player(Deck::parse("Ks Kd Qc Qd").unwrap())
            .unwrap();
        // Player 1: has a low draw
        state
            .add_player(Deck::parse("As 2d 3c 4d").unwrap())
            .unwrap();
        state.set_flop(Deck::parse("5h 7c Kc").unwrap()).unwrap();
        state.set_turn(Card::parse("8h").unwrap()).unwrap();
        state.set_river(Card::parse("Jd").unwrap()).unwrap();

        let evaluator = OmahaHiLoGameEvaluation {};
        let result = evaluator.evaluate_winners(&state);

        // Player 0 should win high (trips kings)
        assert_eq!(result.high_winners.len(), 1);
        assert_eq!(result.high_winners[0].player_index(), 0);
        assert_eq!(result.high_winners[0].pot_amount(), dec!(0.5));

        // Player 1 should win low (A-2-3-5-7 or similar)
        assert_eq!(result.low_winners.len(), 1);
        assert_eq!(result.low_winners[0].player_index(), 1);
        assert_eq!(result.low_winners[0].pot_amount(), dec!(0.5));
    }

    #[test]
    fn test_hilo_scoop_both_high_and_low() {
        let mut state = OmahaHiLoGameState::new(4);
        // Player 0: A-2 with strong high potential
        state
            .add_player(Deck::parse("As 2d Kc Kd").unwrap())
            .unwrap();
        // Player 1: no low cards
        state
            .add_player(Deck::parse("Qs Qd Jc Jd").unwrap())
            .unwrap();
        state.set_flop(Deck::parse("3h 4c 5h").unwrap()).unwrap();
        state.set_turn(Card::parse("Kh").unwrap()).unwrap();
        state.set_river(Card::parse("9d").unwrap()).unwrap();

        let evaluator = OmahaHiLoGameEvaluation {};
        let result = evaluator.evaluate_winners(&state);

        // Player 0 wins high (trips kings) and low (A-2-3-4-5)
        assert_eq!(result.high_winners.len(), 1);
        assert_eq!(result.high_winners[0].player_index(), 0);
        assert_eq!(result.high_winners[0].pot_amount(), dec!(0.5));

        assert_eq!(result.low_winners.len(), 1);
        assert_eq!(result.low_winners[0].player_index(), 0);
        assert_eq!(result.low_winners[0].pot_amount(), dec!(0.5));
    }

    #[test]
    fn test_hilo_equity_matches_per_runout_winners() {
        let mut state = OmahaHiLoGameState::new(4);
        state
            .add_player(Deck::parse("As 2d Kc Kd").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("Ah 3h 4c Qs").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("2s 3s 7d 8d").unwrap())
            .unwrap();
        state.set_flop(Deck::parse("5h 6c Jd").unwrap()).unwrap();

        let evaluator = OmahaHiLoGameEvaluation {};
        let mut expected = [dec!(0); 3];
        let mut runouts = 0;
        for runout in state.get_final_states() {
            let result = evaluator.evaluate_winners(&runout);
            for w in &result.high_winners {
                expected[w.player_index()] += w.pot_amount();
            }
            for w in &result.low_winners {
                expected[w.player_index()] += w.pot_amount();
            }
            runouts += 1;
        }
        let expected: Vec<_> = expected
            .iter()
            .map(|e| e / rust_decimal::Decimal::from(runouts))
            .collect();

        let equity = evaluator.evaluate_equity(&state);
        for (e, x) in equity.iter().zip(&expected) {
            assert!((e - x).abs() < dec!(0.000001), "{equity:?} vs {expected:?}");
        }

        let sampled = evaluator.sample_equity(&state, 20_000);
        for (e, s) in equity.iter().zip(&sampled) {
            assert!((e - s).abs() < dec!(0.02), "{equity:?} vs {sampled:?}");
        }
    }

    #[test]
    fn test_hilo_equity_chunks_merge_to_exact() {
        let mut state = OmahaHiLoGameState::new(4);
        state
            .add_player(Deck::parse("As 2d Kc Kd").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("Ah 3h 4c Qs").unwrap())
            .unwrap();
        state.set_flop(Deck::parse("5h 6c Jd").unwrap()).unwrap();

        let eval = OmahaHiLoGameEvaluation {};
        let total = eval.runout_count(&state);
        let mut merged = crate::games::EquityShares::default();
        for start in (0..total).step_by(250) {
            merged.merge(&eval.evaluate_equity_chunk(&state, start, 250));
        }
        assert_eq!(merged.runouts, total);
        assert_eq!(merged.equity(), eval.evaluate_equity(&state));
    }

    #[test]
    fn test_hilo_equity_no_low_possible() {
        let mut state = OmahaHiLoGameState::new(4);
        state
            .add_player(Deck::parse("Ks Kd Qc Qd").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("Js Jd Tc Td").unwrap())
            .unwrap();
        state.set_flop(Deck::parse("Kc 9h 9d").unwrap()).unwrap();
        state.set_turn(Card::parse("Jh").unwrap()).unwrap();
        state.set_river(Card::parse("Th").unwrap()).unwrap();

        let evaluator = OmahaHiLoGameEvaluation {};
        let equities = evaluator.evaluate_equity(&state);

        assert_eq!(equities.len(), 2);
        assert_eq!(equities[0] + equities[1], dec!(1));
        assert_eq!(equities[0], dec!(1));
    }
}
