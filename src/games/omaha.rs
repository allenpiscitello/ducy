use rust_decimal::Decimal;

use crate::{
    deck::{Card, Deck, Suit},
    error::DucyError,
    games::{
        EquityShares, FastWinnerTracker, GameEquityEvaluation, GameEvaluation, GameState,
        GameWinner, WinnerTracker,
        flop_game::{FlopGame, FlopGameState},
    },
    ranking::hand_rank::{StandardHandRanker, StandardHandRanks},
};

/// Omaha game state (configurable number of hole cards per player).
pub struct OmahaGameState {
    pub(crate) flop_game_state: FlopGameState,
}

impl OmahaGameState {
    /// Hole cards per player.
    pub fn cards_per_player(&self) -> usize {
        self.flop_game_state.cards_per_player()
    }

    /// Creates a new Omaha game state with the given number of hole cards per player.
    pub fn new(cards_per_player: u32) -> Self {
        Self {
            flop_game_state: FlopGameState::new(cards_per_player),
        }
    }
}

impl FlopGame for OmahaGameState {
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

impl GameState for OmahaGameState {}

#[derive(Debug, PartialEq, Eq)]
/// Describes the flush texture of the community cards.
pub enum BoardTone {
    /// All five cards share one suit.
    Monotone {
        /// The dominant suit.
        suit: Suit,
    },
    /// Four cards share one suit.
    FourFlush {
        /// The dominant suit.
        suit: Suit,
    },
    /// Three cards share one suit.
    ThreeFlush {
        /// The dominant suit.
        suit: Suit,
    },
    /// No suit has three or more cards.
    Rainbow {},
}

/// Board analysis methods for Omaha games.
pub trait OmahaBoardAnalysis {
    /// Returns the flush texture of the community cards.
    fn board_tone(&self) -> BoardTone;
}

impl OmahaBoardAnalysis for OmahaGameState {
    fn board_tone(&self) -> BoardTone {
        for (ranks, suit) in self.get_community_cards().get_single_suit_ranks() {
            match ranks.num_unique_ranks() {
                5 => return BoardTone::Monotone { suit },
                4 => return BoardTone::FourFlush { suit },
                3 => return BoardTone::ThreeFlush { suit },
                _ => {}
            }
        }
        BoardTone::Rainbow {}
    }
}

/// Evaluates Omaha hands to determine winners (brute-force over all 2-from-hand + 3-from-board combos).
pub struct OmahaGameEvaluation {}

impl GameEvaluation<OmahaGameState, StandardHandRanks> for OmahaGameEvaluation {
    fn evaluate_winners(&self, game_state: &OmahaGameState) -> Vec<GameWinner<StandardHandRanks>> {
        let mut tracker = WinnerTracker::new();
        for (i, player) in game_state.get_player_hole_cards().enumerate() {
            for community_cards_of_3 in game_state.get_community_cards().enumerate_combinations(3) {
                let board_suit = community_cards_of_3.single_suit_index();
                let board_paired = community_cards_of_3.has_rank_pair();
                for player_cards_group_of_2 in player.enumerate_combinations(2) {
                    let flush_possible =
                        board_suit.is_some_and(|s| player_cards_group_of_2.all_in_suit_index(s));
                    let combined_deck = community_cards_of_3 | player_cards_group_of_2;
                    if let Some(rank) = StandardHandRanker::get_rank_at_least_with_hints(
                        &combined_deck,
                        tracker.best_hand(),
                        flush_possible,
                        board_paired,
                    ) {
                        tracker.consider(i, rank);
                    }
                }
            }
        }
        tracker.into_results()
    }
}

impl GameEquityEvaluation<OmahaGameState, StandardHandRanks, OmahaGameEvaluation>
    for OmahaGameEvaluation
{
    fn evaluate_equity(&self, game_state: &OmahaGameState) -> Vec<Decimal> {
        let runouts = game_state
            .flop_game_state
            .enumerate_runout_community_cards()
            .collect();
        shares_over_runouts(game_state.flop_game_state.hole_cards(), runouts).equity()
    }
}

impl OmahaGameEvaluation {
    /// Estimates equity from `samples` random runouts instead of enumerating all of them.
    pub fn sample_equity(&self, game_state: &OmahaGameState, samples: usize) -> Vec<Decimal> {
        self.sample_equity_seeded(game_state, samples, None)
    }

    /// Like `sample_equity`; a `seed` makes the result reproducible.
    pub fn sample_equity_seeded(
        &self,
        game_state: &OmahaGameState,
        samples: usize,
        seed: Option<u64>,
    ) -> Vec<Decimal> {
        let runouts = game_state
            .flop_game_state
            .sample_runout_community_cards(samples, seed);
        shares_over_runouts(game_state.flop_game_state.hole_cards(), runouts).equity()
    }

    /// Number of runouts exact equity enumerates.
    pub fn runout_count(&self, game_state: &OmahaGameState) -> u64 {
        game_state.flop_game_state.runout_count()
    }

    /// Equity totals for runouts `start..start + count` of the exact
    /// enumeration. Merging every chunk gives the same result as
    /// `evaluate_equity`, so chunks can run on separate threads or workers.
    pub fn evaluate_equity_chunk(
        &self,
        game_state: &OmahaGameState,
        start: u64,
        count: u64,
    ) -> EquityShares {
        let runouts = game_state.flop_game_state.runout_chunk(start, count);
        shares_over_runouts(game_state.flop_game_state.hole_cards(), runouts)
    }
}

fn shares_over_runouts(hole_cards: &[Deck], runouts: Vec<Deck>) -> EquityShares {
    let player_combos: Vec<_> = hole_cards.iter().map(hole_card_combos).collect();
    crate::games::accumulate_shares(runouts, hole_cards.len(), |community, shares| {
        high_winners(community, &player_combos).distribute(shares);
    })
}

/// Each 2-card combo from a player's hole cards, with its suit if suited.
pub(crate) type HoleCombos = Vec<(Deck, Option<usize>)>;

pub(crate) fn hole_card_combos(hole_cards: &Deck) -> HoleCombos {
    hole_cards
        .enumerate_combinations(2)
        .map(|d| (d, d.single_suit_index()))
        .collect()
}

/// Finds the players with the best Omaha high hand on a complete 5-card board.
pub(crate) fn high_winners(community: &Deck, player_combos: &[HoleCombos]) -> FastWinnerTracker {
    let mut tracker = FastWinnerTracker::new();
    for community_cards_of_3 in community.enumerate_combinations(3) {
        let board_suit = community_cards_of_3.single_suit_index();
        let board_paired = community_cards_of_3.has_rank_pair();
        for (i, combos) in player_combos.iter().enumerate() {
            for &(player_deck, player_suit) in combos {
                let flush_possible = board_suit.is_some() && board_suit == player_suit;
                let combined_deck = community_cards_of_3 | player_deck;
                if let Some(score) = StandardHandRanker::fast_score_at_least(
                    &combined_deck,
                    tracker.best_score(),
                    flush_possible,
                    board_paired,
                ) {
                    tracker.consider(i, score);
                }
            }
        }
    }
    tracker
}

#[cfg(test)]
mod test {
    use rust_decimal_macros::dec;

    use crate::{
        deck::{Card, Deck, Rank, Suit},
        games::{
            GameEquityEvaluation, GameEvaluation, GameWinner,
            flop_game::FlopGame,
            omaha::{BoardTone, OmahaBoardAnalysis, OmahaGameEvaluation, OmahaGameState},
        },
        ranking::hand_rank::StandardHandRanks,
    };

    #[test]
    pub fn test_omaha_hand() {
        let mut state = OmahaGameState::new(4);
        assert_eq!(state.get_community_cards(), Deck::empty());

        state
            .add_player(Deck::parse("As Ac Jc Ts").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("9h 8h 7d 6d").unwrap())
            .unwrap();

        state.set_flop(Deck::parse("Jh Th Qd").unwrap()).unwrap();

        let evaluator = OmahaGameEvaluation {};

        assert_eq!(state.board_tone(), BoardTone::Rainbow {});

        let winners = evaluator.evaluate_winners(&state);

        assert_eq!(winners.len(), 1);
        assert_eq!(
            winners[0],
            GameWinner {
                player_index: 1,
                pot_amount: dec!(1),
                winning_hand: StandardHandRanks::Straight { s: Rank::Queen }
            }
        );

        state.set_turn(Card::parse("Jd").unwrap()).unwrap();
        assert_eq!(state.board_tone(), BoardTone::Rainbow {});

        let winners = evaluator.evaluate_winners(&state);
        assert_eq!(winners.len(), 1);
        assert_eq!(
            winners[0],
            GameWinner {
                player_index: 0,
                pot_amount: dec!(1),
                winning_hand: StandardHandRanks::FullHouse {
                    t: Rank::Jack,
                    p: Rank::Ten
                }
            }
        );

        let equity = evaluator.evaluate_equity(&state);
        assert_eq!(equity.len(), 2);
        assert_eq!(equity[0], dec!(38) / dec!(40));
        assert_eq!(equity[1], dec!(2) / dec!(40));

        state.set_river(Card::parse("Qh").unwrap()).unwrap();

        assert_eq!(
            state.board_tone(),
            BoardTone::ThreeFlush { suit: Suit::Hearts }
        );

        let winners = evaluator.evaluate_winners(&state);
        assert_eq!(winners.len(), 1);
        assert_eq!(
            winners[0],
            GameWinner {
                player_index: 1,
                pot_amount: dec!(1),
                winning_hand: StandardHandRanks::StraightFlush { sf: Rank::Queen }
            }
        );
    }

    #[test]
    pub fn test_tie_split_evenly_when_one_player_ties_with_several_combos() {
        let mut state = OmahaGameState::new(4);
        state
            .add_player(Deck::parse("Qs Ks Qd Kd").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("Qh Kh 4c 5d").unwrap())
            .unwrap();
        state.set_flop(Deck::parse("9h Tc Jd").unwrap()).unwrap();
        state.set_turn(Card::parse("2s").unwrap()).unwrap();
        state.set_river(Card::parse("3c").unwrap()).unwrap();

        let evaluator = OmahaGameEvaluation {};
        let winners = evaluator.evaluate_winners(&state);
        assert_eq!(winners.len(), 2);
        assert!(winners.iter().all(|w| w.pot_amount == dec!(0.5)));

        let equity = evaluator.evaluate_equity(&state);
        assert_eq!(equity, vec![dec!(0.5), dec!(0.5)]);
    }

    #[test]
    pub fn test_dead_cards_never_dealt() {
        let mut state = OmahaGameState::new(4);
        state
            .add_player(Deck::parse("As Ac Jc Ts").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("9h 8h 7d 6d").unwrap())
            .unwrap();
        state.set_flop(Deck::parse("Jh Th Qd").unwrap()).unwrap();
        let dead = Deck::parse("Kh 2c").unwrap();
        state.add_dead_cards(dead).unwrap();
        assert!(state.add_dead_cards(dead).is_err());

        let runouts: Vec<Deck> = state
            .flop_game_state
            .enumerate_runout_community_cards()
            .collect();
        assert_eq!(runouts.len(), 39 * 38 / 2);
        assert!(runouts.iter().all(|r| u64::from(*r) & u64::from(dead) == 0));
        let sampled = state
            .flop_game_state
            .sample_runout_community_cards(500, None);
        assert!(sampled.iter().all(|r| u64::from(*r) & u64::from(dead) == 0));
    }

    #[test]
    pub fn test_equity_chunks_merge_to_exact() {
        let mut state = OmahaGameState::new(4);
        state
            .add_player(Deck::parse("As Ac Jc Ts").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("9h 8h 7d 6d").unwrap())
            .unwrap();
        state.set_flop(Deck::parse("Jh Th Qd").unwrap()).unwrap();

        let eval = OmahaGameEvaluation {};
        let total = eval.runout_count(&state);
        assert_eq!(total, 41 * 40 / 2);
        let mut merged = crate::games::EquityShares::default();
        for start in (0..total).step_by(301) {
            merged.merge(&eval.evaluate_equity_chunk(&state, start, 301));
        }
        assert_eq!(merged.runouts, total);
        assert_eq!(merged.equity(), eval.evaluate_equity(&state));
    }

    #[test]
    pub fn test_sample_equity_close_to_exact() {
        let mut state = OmahaGameState::new(4);
        state
            .add_player(Deck::parse("As Ac Jc Ts").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("9h 8h 7d 6d").unwrap())
            .unwrap();
        state.set_flop(Deck::parse("Jh Th Qd").unwrap()).unwrap();

        let evaluator = OmahaGameEvaluation {};
        let exact = evaluator.evaluate_equity(&state);
        let sampled = evaluator.sample_equity(&state, 20_000);
        for (e, s) in exact.iter().zip(&sampled) {
            assert!((e - s).abs() < dec!(0.02), "{exact:?} vs {sampled:?}");
        }

        let seeded = evaluator.sample_equity_seeded(&state, 2_000, Some(42));
        assert_eq!(
            seeded,
            evaluator.sample_equity_seeded(&state, 2_000, Some(42))
        );
        assert_ne!(
            seeded,
            evaluator.sample_equity_seeded(&state, 2_000, Some(43))
        );
    }

    #[test]
    pub fn test_board_tone_monotone() {
        let mut state = OmahaGameState::new(4);
        state
            .add_player(Deck::parse("As Ac 2d 3d").unwrap())
            .unwrap();
        state.set_flop(Deck::parse("2h 3h 4h").unwrap()).unwrap();
        state.set_turn(Card::parse("5h").unwrap()).unwrap();
        state.set_river(Card::parse("6h").unwrap()).unwrap();
        assert_eq!(
            state.board_tone(),
            BoardTone::Monotone { suit: Suit::Hearts }
        );
    }

    #[test]
    pub fn test_board_tone_four_flush() {
        let mut state = OmahaGameState::new(4);
        state
            .add_player(Deck::parse("As Ac 2d 3d").unwrap())
            .unwrap();
        state.set_flop(Deck::parse("2h 3h 4h").unwrap()).unwrap();
        state.set_turn(Card::parse("5h").unwrap()).unwrap();
        state.set_river(Card::parse("6c").unwrap()).unwrap();
        assert_eq!(
            state.board_tone(),
            BoardTone::FourFlush { suit: Suit::Hearts }
        );
    }
}
