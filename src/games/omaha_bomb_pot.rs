use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::{
    deck::{Card, Deck},
    error::DucyError,
    games::{
        CardDealer, EQUITY_SCALE, GameState, GameWinner, WinnerTracker,
        omaha::{HoleCombos, high_winners, hole_card_combos},
        omaha_range::{OmahaRange, RangeSampler},
    },
    ranking::hand_rank::{StandardHandRanker, StandardHandRanks},
};

const MAX_PLAYERS: usize = 10;

/// Redraws allowed per sample when range hands collide before giving up.
const MAX_CONFLICT_REDRAWS: u32 = 100_000;

/// How [`OmahaBombPotGameEvaluation::sample_seats_seeded`] deals a seat whose
/// cards are unknown.
#[derive(Clone, Copy)]
pub enum SeatHand<'a> {
    /// Any hand, uniformly at random.
    Random,
    /// A hand from the range, in proportion to its weight. Use this for
    /// realistic opponents, e.g. hands likely to see the flop.
    Range(&'a OmahaRange),
}

/// Omaha bomb pot game state with multiple boards sharing the same hole cards.
pub struct OmahaBombPotGameState {
    num_boards: usize,
    num_hole_cards_per_player: u32,
    hole_cards: Vec<Deck>,
    boards: Vec<BoardState>,
    remaining_cards: Deck,
}

#[derive(Clone)]
struct BoardState {
    flop: Deck,
    turn: Option<Card>,
    river: Option<Card>,
    community_cards: Deck,
}

impl BoardState {
    fn new() -> Self {
        Self {
            flop: Deck::empty(),
            turn: None,
            river: None,
            community_cards: Deck::empty(),
        }
    }

    fn cards_needed(&self) -> u32 {
        if self.flop.is_empty() {
            5
        } else if self.turn.is_none() {
            2
        } else if self.river.is_none() {
            1
        } else {
            0
        }
    }
}

/// Per-board winner results for a bomb pot.
pub struct BombPotResult {
    /// The winners on each board, by board index (more than one on a tie);
    /// each board is worth an equal share of the pot.
    pub board_winners: Vec<Vec<GameWinner<StandardHandRanks>>>,
}

impl OmahaBombPotGameState {
    /// Creates a new bomb pot state with the given number of boards and hole cards per player.
    pub fn new(num_boards: usize, cards_per_player: u32) -> Self {
        Self {
            num_boards,
            num_hole_cards_per_player: cards_per_player,
            hole_cards: Vec::new(),
            boards: (0..num_boards).map(|_| BoardState::new()).collect(),
            remaining_cards: Deck::all_cards(),
        }
    }

    /// Adds the next player's hole cards. Fails with
    /// [`IncorrectCardCount`](DucyError::IncorrectCardCount) if it isn't the
    /// game's number of hole cards, or
    /// [`CardsNotAvailable`](DucyError::CardsNotAvailable) if a card was already dealt.
    pub fn add_player(&mut self, cards: Deck) -> Result<(), DucyError> {
        if cards.num_cards() != self.num_hole_cards_per_player {
            return Err(DucyError::IncorrectCardCount);
        }
        if !self.remaining_cards.has_cards(&cards) {
            return Err(DucyError::CardsNotAvailable);
        }
        self.remaining_cards -= cards;
        self.hole_cards.push(cards);
        Ok(())
    }

    /// Sets board `board_index`'s flop (3 cards). Fails with
    /// [`InvalidBoardIndex`](DucyError::InvalidBoardIndex),
    /// [`IncorrectCardCount`](DucyError::IncorrectCardCount) or
    /// [`CardsNotAvailable`](DucyError::CardsNotAvailable).
    pub fn set_flop(&mut self, board_index: usize, cards: Deck) -> Result<(), DucyError> {
        if board_index >= self.num_boards {
            return Err(DucyError::InvalidBoardIndex);
        }
        if cards.num_cards() != 3 {
            return Err(DucyError::IncorrectCardCount);
        }
        if !self.remaining_cards.has_cards(&cards) {
            return Err(DucyError::CardsNotAvailable);
        }
        self.remaining_cards -= cards;
        self.boards[board_index].flop = cards;
        self.boards[board_index].community_cards |= cards;
        Ok(())
    }

    /// Sets board `board_index`'s turn; its flop must be set first
    /// ([`FlopNotSet`](DucyError::FlopNotSet)).
    pub fn set_turn(&mut self, board_index: usize, card: Card) -> Result<(), DucyError> {
        if board_index >= self.num_boards {
            return Err(DucyError::InvalidBoardIndex);
        }
        if self.boards[board_index].flop.is_empty() {
            return Err(DucyError::FlopNotSet);
        }
        if !self.remaining_cards.has_card(&card) {
            return Err(DucyError::CardsNotAvailable);
        }
        self.remaining_cards.remove_cards([card].into_iter());
        self.boards[board_index].turn = Some(card);
        self.boards[board_index].community_cards |= card;
        Ok(())
    }

    /// Sets board `board_index`'s river; its turn must be set first
    /// ([`TurnNotSet`](DucyError::TurnNotSet)).
    pub fn set_river(&mut self, board_index: usize, card: Card) -> Result<(), DucyError> {
        if board_index >= self.num_boards {
            return Err(DucyError::InvalidBoardIndex);
        }
        if self.boards[board_index].turn.is_none() {
            return Err(DucyError::TurnNotSet);
        }
        if !self.remaining_cards.has_card(&card) {
            return Err(DucyError::CardsNotAvailable);
        }
        self.remaining_cards.remove_cards([card].into_iter());
        self.boards[board_index].river = Some(card);
        self.boards[board_index].community_cards |= card;
        Ok(())
    }

    /// Removes cards known to be out of play (dead cards) from the remaining deck.
    pub fn add_dead_cards(&mut self, cards: Deck) -> Result<(), DucyError> {
        if !self.remaining_cards.has_cards(&cards) {
            return Err(DucyError::CardsNotAvailable);
        }
        self.remaining_cards -= cards;
        Ok(())
    }

    /// Board `board_index`'s cards so far. Panics if there's no such board.
    pub fn get_board_community_cards(&self, board_index: usize) -> Deck {
        self.boards[board_index].community_cards
    }

    /// Each player's hole cards, in the order they were added.
    pub fn get_player_hole_cards(&self) -> &[Deck] {
        &self.hole_cards
    }

    /// How many boards are run.
    pub fn num_boards(&self) -> usize {
        self.num_boards
    }
}

impl GameState for OmahaBombPotGameState {}

/// Evaluates Omaha bomb pot hands across multiple boards.
pub struct OmahaBombPotGameEvaluation {}

impl OmahaBombPotGameEvaluation {
    /// Evaluates winners for each board independently. Pot is split equally among boards.
    pub fn evaluate_winners(&self, game_state: &OmahaBombPotGameState) -> BombPotResult {
        let board_share = dec!(1) / Decimal::from(game_state.num_boards as u64);
        let mut board_winners = Vec::with_capacity(game_state.num_boards);

        for board in &game_state.boards {
            let community = board.community_cards;
            let mut tracker = WinnerTracker::new();
            for community_cards_of_3 in community.enumerate_combinations(3) {
                let board_suit = community_cards_of_3.single_suit_index();
                let board_paired = community_cards_of_3.has_rank_pair();
                for (i, player) in game_state.hole_cards.iter().enumerate() {
                    for player_cards_of_2 in player.enumerate_combinations(2) {
                        let flush_possible =
                            board_suit.is_some_and(|s| player_cards_of_2.all_in_suit_index(s));
                        let combined = community_cards_of_3 | player_cards_of_2;
                        if let Some(rank) = StandardHandRanker::get_rank_at_least_with_hints(
                            &combined,
                            tracker.best_hand(),
                            flush_possible,
                            board_paired,
                        ) {
                            tracker.consider(i, rank);
                        }
                    }
                }
            }
            let mut winners = tracker.into_results();
            for w in &mut winners {
                w.pot_amount *= board_share;
            }
            board_winners.push(winners);
        }

        BombPotResult { board_winners }
    }

    /// Evaluates equity across all possible runouts for all boards.
    /// Each board's runout is enumerated independently; the pot is split
    /// equally among boards.
    pub fn evaluate_equity(&self, game_state: &OmahaBombPotGameState) -> Vec<Decimal> {
        let num_players = game_state.hole_cards.len();
        let num_boards = game_state.num_boards;

        if num_boards == 0 || num_players == 0 {
            return vec![Decimal::ZERO; num_players];
        }

        let board_weight = Decimal::from(1) / Decimal::from(num_boards as u64);
        let mut total_equity = vec![Decimal::ZERO; num_players];

        let player_combos: Vec<_> = game_state.hole_cards.iter().map(hole_card_combos).collect();

        for board in &game_state.boards {
            let cards_needed = board.cards_needed();
            let base_community = board.community_cards;

            let runouts: Vec<Deck> = if cards_needed == 0 {
                vec![base_community]
            } else {
                game_state
                    .remaining_cards
                    .enumerate_combinations(cards_needed as usize)
                    .map(|c| base_community | c)
                    .collect()
            };

            let board_equity =
                crate::games::accumulate_equity(runouts, num_players, |community, shares| {
                    high_winners(community, &player_combos).distribute(shares);
                });

            for (i, eq) in board_equity.iter().enumerate() {
                total_equity[i] += eq * board_weight;
            }
        }

        total_equity
    }

    /// Monte Carlo sampling that deals every unknown card at random: missing
    /// board cards, plus full hands for players whose cards are unknown.
    ///
    /// `random_seats` are positions in the final player order that get random
    /// hands; the players added to `game_state` fill the other positions in
    /// order. Results are indexed by final player order.
    pub fn sample(
        &self,
        game_state: &OmahaBombPotGameState,
        random_seats: &[usize],
        samples: usize,
    ) -> Result<BombPotSamples, DucyError> {
        self.sample_seeded(game_state, random_seats, samples, None)
    }

    /// Like `sample`; a `seed` makes the deals reproducible.
    pub fn sample_seeded(
        &self,
        game_state: &OmahaBombPotGameState,
        random_seats: &[usize],
        samples: usize,
        seed: Option<u64>,
    ) -> Result<BombPotSamples, DucyError> {
        let seats: Vec<(usize, SeatHand)> = random_seats
            .iter()
            .map(|&seat| (seat, SeatHand::Random))
            .collect();
        self.sample_seats_seeded(game_state, &seats, samples, seed)
    }

    /// Like `sample_seeded`, but each unknown seat is dealt either a random
    /// hand or a hand from an [`OmahaRange`] (weighted, never sharing a card
    /// with known cards or other seats). Errors with `InvalidRange` if a
    /// range has the wrong hole-card count, has no hand that fits the known
    /// cards, or the ranges keep colliding.
    pub fn sample_seats_seeded(
        &self,
        game_state: &OmahaBombPotGameState,
        seats: &[(usize, SeatHand)],
        samples: usize,
        seed: Option<u64>,
    ) -> Result<BombPotSamples, DucyError> {
        let num_players = game_state.hole_cards.len() + seats.len();
        if num_players > MAX_PLAYERS {
            return Err(DucyError::TooManyPlayers);
        }
        let mut is_random = vec![false; num_players];
        for &(seat, _) in seats {
            if seat >= num_players || is_random[seat] {
                return Err(DucyError::TooManyPlayers);
            }
            is_random[seat] = true;
        }
        let random_seats: Vec<usize> = seats
            .iter()
            .filter(|(_, hand)| matches!(hand, SeatHand::Random))
            .map(|&(seat, _)| seat)
            .collect();
        let range_seats: Vec<(usize, RangeSampler)> = seats
            .iter()
            .filter_map(|&(seat, hand)| match hand {
                SeatHand::Random => None,
                SeatHand::Range(range) => Some((seat, range)),
            })
            .map(|(seat, range)| {
                if range.cards_per_player() != game_state.num_hole_cards_per_player as usize {
                    return Err(DucyError::InvalidRange);
                }
                Ok((seat, RangeSampler::new(range, game_state.remaining_cards)?))
            })
            .collect::<Result<_, _>>()?;

        let cards_per_player = game_state.num_hole_cards_per_player as usize;
        let board_cards_needed: Vec<usize> = game_state
            .boards
            .iter()
            .map(|b| b.cards_needed() as usize)
            .collect();
        let mut dealer = CardDealer::maybe_seeded(game_state.remaining_cards, seed);
        let cards_needed =
            seats.len() * cards_per_player + board_cards_needed.iter().sum::<usize>();
        if cards_needed > dealer.available() {
            return Err(DucyError::NotEnoughCards);
        }

        let mut known = game_state.hole_cards.iter();
        let mut player_combos: Vec<HoleCombos> = is_random
            .iter()
            .map(|&r| {
                if r {
                    Vec::new()
                } else {
                    hole_card_combos(known.next().unwrap())
                }
            })
            .collect();

        let num_boards = game_state.num_boards;
        let mut shares = vec![0u64; num_players];
        let mut result = BombPotSamples {
            samples: samples as u64,
            equity_sum: vec![0.0; num_players],
            board_wins: vec![vec![0; num_players]; num_boards],
            scoops: vec![0; num_players],
            scooped: vec![0; num_players],
        };

        for _ in 0..samples {
            dealer.reset();
            let mut redraws = 0;
            let mut used = 'draw: loop {
                let mut used = Deck::empty();
                for (seat, sampler) in &range_seats {
                    let hand = sampler.draw(&mut dealer);
                    if u64::from(used) & u64::from(hand) != 0 {
                        redraws += 1;
                        if redraws > MAX_CONFLICT_REDRAWS {
                            return Err(DucyError::InvalidRange);
                        }
                        continue 'draw;
                    }
                    used |= hand;
                    player_combos[*seat] = hole_card_combos(&hand);
                }
                break used;
            };
            for &seat in &random_seats {
                let hand = dealer.deal_excluding(cards_per_player, used);
                used |= hand;
                player_combos[seat] = hole_card_combos(&hand);
            }

            let mut scooper = None;
            for (b, board) in game_state.boards.iter().enumerate() {
                let community =
                    board.community_cards | dealer.deal_excluding(board_cards_needed[b], used);
                let tracker = high_winners(&community, &player_combos);
                tracker.distribute(&mut shares);
                for &w in tracker.winners() {
                    result.board_wins[b][w] += 1;
                }
                let sole = match tracker.winners() {
                    [w] => Some(*w),
                    _ => None,
                };
                scooper = if b == 0 || scooper == sole {
                    sole
                } else {
                    None
                };
            }
            if let Some(p) = scooper {
                result.scoops[p] += 1;
                for (i, s) in result.scooped.iter_mut().enumerate() {
                    if i != p {
                        *s += 1;
                    }
                }
            }
        }

        let scale = (EQUITY_SCALE * num_boards as u64) as f64;
        for (sum, &s) in result.equity_sum.iter_mut().zip(&shares) {
            *sum = s as f64 / scale;
        }
        Ok(result)
    }
}

/// Aggregated Monte Carlo results for a bomb pot, indexed by player.
pub struct BombPotSamples {
    /// Number of samples taken.
    pub samples: u64,
    /// Sum across samples of the fraction of the pot each player won.
    pub equity_sum: Vec<f64>,
    /// `board_wins[board][player]`: samples where the player won or tied that board.
    pub board_wins: Vec<Vec<u64>>,
    /// Samples where the player alone won every board.
    pub scoops: Vec<u64>,
    /// Samples where a single opponent alone won every board.
    pub scooped: Vec<u64>,
}

#[cfg(test)]
mod test {
    use rust_decimal_macros::dec;

    use crate::deck::{Card, Deck};
    use crate::error::DucyError;
    use crate::games::omaha_bomb_pot::{
        OmahaBombPotGameEvaluation, OmahaBombPotGameState, SeatHand,
    };
    use crate::games::omaha_range::OmahaRange;

    fn complete_board(state: &mut OmahaBombPotGameState, b: usize, cards: &str) {
        let cards: Vec<&str> = cards.split(' ').collect();
        state
            .set_flop(b, Deck::parse(&cards[..3].join(" ")).unwrap())
            .unwrap();
        state.set_turn(b, Card::parse(cards[3]).unwrap()).unwrap();
        state.set_river(b, Card::parse(cards[4]).unwrap()).unwrap();
    }

    #[test]
    fn test_sample_complete_boards_split() {
        let mut state = OmahaBombPotGameState::new(2, 4);
        state
            .add_player(Deck::parse("As Ac Jc Ts").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("9h 8h 7d 6d").unwrap())
            .unwrap();
        complete_board(&mut state, 0, "Ad 2h 3c 5s Kd");
        complete_board(&mut state, 1, "Th Jh Qd 4d 2d");

        let r = OmahaBombPotGameEvaluation {}
            .sample(&state, &[], 10)
            .unwrap();
        assert_eq!(r.equity_sum, vec![5.0, 5.0]);
        assert_eq!(r.board_wins, vec![vec![10, 0], vec![0, 10]]);
        assert_eq!(r.scoops, vec![0, 0]);
        assert_eq!(r.scooped, vec![0, 0]);
    }

    #[test]
    fn test_sample_complete_boards_scoop() {
        let mut state = OmahaBombPotGameState::new(2, 4);
        state
            .add_player(Deck::parse("As Ac Kc Kd").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("2h 3h 4d 5d").unwrap())
            .unwrap();
        complete_board(&mut state, 0, "Ah Ad Qs Jh Ts");
        complete_board(&mut state, 1, "Ks Kh Qd 9c 8c");

        let r = OmahaBombPotGameEvaluation {}
            .sample(&state, &[], 10)
            .unwrap();
        assert_eq!(r.equity_sum, vec![10.0, 0.0]);
        assert_eq!(r.scoops, vec![10, 0]);
        assert_eq!(r.scooped, vec![0, 10]);
    }

    #[test]
    fn test_sample_with_random_player() {
        let mut state = OmahaBombPotGameState::new(2, 4);
        state
            .add_player(Deck::parse("As Ac Kc Kd").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("2h 3h 4d 5d").unwrap())
            .unwrap();
        state.set_flop(0, Deck::parse("7s 8s 9d").unwrap()).unwrap();

        let eval = OmahaBombPotGameEvaluation {};
        let r = eval.sample(&state, &[1], 2_000).unwrap();
        assert_eq!(r.equity_sum.len(), 3);
        assert!((r.equity_sum.iter().sum::<f64>() - 2_000.0).abs() < 1e-6);
        for board in &r.board_wins {
            assert!(board.iter().sum::<u64>() >= 2_000);
        }
        assert!(r.equity_sum.iter().all(|&e| e > 0.0));

        let seeded = |seed| eval.sample_seeded(&state, &[1], 500, Some(seed)).unwrap();
        let (a, b, c) = (seeded(1), seeded(1), seeded(2));
        assert_eq!(a.equity_sum, b.equity_sum);
        assert_eq!(a.board_wins, b.board_wins);
        assert_ne!(a.equity_sum, c.equity_sum);

        assert!(eval.sample(&state, &[3], 1).is_err());
        assert!(eval.sample(&state, &[1, 1], 1).is_err());
    }

    fn heads_up(hero: &str) -> OmahaBombPotGameState {
        let mut state = OmahaBombPotGameState::new(1, 4);
        state.add_player(Deck::parse(hero).unwrap()).unwrap();
        state
    }

    #[test]
    fn test_range_seat_matches_known_hand() {
        let eval = OmahaBombPotGameEvaluation {};
        let villain = OmahaRange::parse("AsAhKsKh", 4).unwrap();
        let ranged = eval
            .sample_seats_seeded(
                &heads_up("Qd Qc Jd Jc"),
                &[(1, SeatHand::Range(&villain))],
                20_000,
                Some(1),
            )
            .unwrap();

        let mut known = heads_up("Qd Qc Jd Jc");
        known
            .add_player(Deck::parse("As Ah Ks Kh").unwrap())
            .unwrap();
        let fixed = eval.sample_seeded(&known, &[], 20_000, Some(2)).unwrap();
        for (a, b) in ranged.equity_sum.iter().zip(&fixed.equity_sum) {
            assert!((a - b).abs() / 20_000.0 < 0.02, "{a} vs {b}");
        }
    }

    #[test]
    fn test_range_seats() {
        let eval = OmahaBombPotGameEvaluation {};
        let state = heads_up("As Ac 7d 2h");
        // Only Ah Ad remain, so every villain hand holds both.
        let aces = OmahaRange::parse("AA", 4).unwrap();
        let r = eval
            .sample_seats_seeded(&state, &[(1, SeatHand::Range(&aces))], 2_000, Some(3))
            .unwrap();
        assert!((r.equity_sum.iter().sum::<f64>() - 2_000.0).abs() < 1e-6);

        // Range and random seats together, reproducible with a seed.
        let rundowns = OmahaRange::parse("$rd0-1$!r", 4).unwrap();
        let seats = [
            (1, SeatHand::Range(&rundowns)),
            (2, SeatHand::Random),
            (3, SeatHand::Range(&aces)),
        ];
        let run = |seed| {
            eval.sample_seats_seeded(&state, &seats, 500, Some(seed))
                .unwrap()
        };
        let (a, b) = (run(4), run(4));
        assert_eq!(a.equity_sum, b.equity_sum);
        assert_eq!(a.equity_sum.len(), 4);
        // Both remaining aces can't go to two players at once.
        assert!(matches!(
            eval.sample_seats_seeded(
                &state,
                &[(1, SeatHand::Range(&aces)), (2, SeatHand::Range(&aces))],
                10,
                Some(5)
            ),
            Err(DucyError::InvalidRange)
        ));
    }

    #[test]
    fn test_range_seat_errors() {
        let eval = OmahaBombPotGameEvaluation {};
        let all_aces = heads_up("As Ac Ad Ah");
        let aces = OmahaRange::parse("AA", 4).unwrap();
        assert!(matches!(
            eval.sample_seats_seeded(&all_aces, &[(1, SeatHand::Range(&aces))], 10, None),
            Err(DucyError::InvalidRange)
        ));
        let plo5 = OmahaRange::parse("KK", 5).unwrap();
        assert!(matches!(
            eval.sample_seats_seeded(&all_aces, &[(1, SeatHand::Range(&plo5))], 10, None),
            Err(DucyError::InvalidRange)
        ));
    }

    #[test]
    fn test_bomb_pot_two_boards_different_winners() {
        let mut state = OmahaBombPotGameState::new(2, 4);

        state
            .add_player(Deck::parse("As Ac Jc Ts").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("9h 8h 7d 6d").unwrap())
            .unwrap();

        // Board 1: Player 0 wins with aces
        state.set_flop(0, Deck::parse("Ad 2h 3c").unwrap()).unwrap();
        state
            .set_turn(0, crate::deck::Card::parse("5s").unwrap())
            .unwrap();
        state
            .set_river(0, crate::deck::Card::parse("Kd").unwrap())
            .unwrap();

        // Board 2: Player 1 wins with straight
        state.set_flop(1, Deck::parse("Th Jh Qd").unwrap()).unwrap();
        state
            .set_turn(1, crate::deck::Card::parse("4d").unwrap())
            .unwrap();
        state
            .set_river(1, crate::deck::Card::parse("2d").unwrap())
            .unwrap();

        let evaluator = OmahaBombPotGameEvaluation {};
        let result = evaluator.evaluate_winners(&state);

        assert_eq!(result.board_winners.len(), 2);

        // Board 1: Player 0 wins
        assert_eq!(result.board_winners[0].len(), 1);
        assert_eq!(result.board_winners[0][0].player_index(), 0);
        assert_eq!(result.board_winners[0][0].pot_amount(), dec!(0.5));

        // Board 2: Player 1 wins
        assert_eq!(result.board_winners[1].len(), 1);
        assert_eq!(result.board_winners[1][0].player_index(), 1);
        assert_eq!(result.board_winners[1][0].pot_amount(), dec!(0.5));
    }

    #[test]
    fn test_bomb_pot_same_winner_scoops() {
        let mut state = OmahaBombPotGameState::new(2, 4);

        state
            .add_player(Deck::parse("As Ac Kc Kd").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("2h 3h 4d 5d").unwrap())
            .unwrap();

        // Board 1: high cards favor player 0
        state.set_flop(0, Deck::parse("Ah Ad Qs").unwrap()).unwrap();
        state
            .set_turn(0, crate::deck::Card::parse("Jh").unwrap())
            .unwrap();
        state
            .set_river(0, crate::deck::Card::parse("Ts").unwrap())
            .unwrap();

        // Board 2: also favors player 0
        state.set_flop(1, Deck::parse("Ks Kh Qd").unwrap()).unwrap();
        state
            .set_turn(1, crate::deck::Card::parse("9c").unwrap())
            .unwrap();
        state
            .set_river(1, crate::deck::Card::parse("8c").unwrap())
            .unwrap();

        let evaluator = OmahaBombPotGameEvaluation {};
        let result = evaluator.evaluate_winners(&state);

        // Player 0 wins both boards
        assert_eq!(result.board_winners[0][0].player_index(), 0);
        assert_eq!(result.board_winners[1][0].player_index(), 0);
        // Each board is half the pot
        assert_eq!(result.board_winners[0][0].pot_amount(), dec!(0.5));
        assert_eq!(result.board_winners[1][0].pot_amount(), dec!(0.5));
    }

    #[test]
    fn test_bomb_pot_equity_complete_boards() {
        let mut state = OmahaBombPotGameState::new(2, 4);

        state
            .add_player(Deck::parse("As Ac Jc Ts").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("9h 8h 7d 6d").unwrap())
            .unwrap();

        // Board 1: Player 0 dominates
        state.set_flop(0, Deck::parse("Ad 2h 3c").unwrap()).unwrap();
        state
            .set_turn(0, crate::deck::Card::parse("5s").unwrap())
            .unwrap();
        state
            .set_river(0, crate::deck::Card::parse("Kd").unwrap())
            .unwrap();

        // Board 2: Player 1 dominates
        state.set_flop(1, Deck::parse("Th Jh Qd").unwrap()).unwrap();
        state
            .set_turn(1, crate::deck::Card::parse("4d").unwrap())
            .unwrap();
        state
            .set_river(1, crate::deck::Card::parse("2d").unwrap())
            .unwrap();

        let evaluator = OmahaBombPotGameEvaluation {};
        let equity = evaluator.evaluate_equity(&state);

        assert_eq!(equity.len(), 2);
        // Each player wins one board, so equity should be 50/50
        assert_eq!(equity[0] + equity[1], dec!(1));
        assert_eq!(equity[0], dec!(0.5));
        assert_eq!(equity[1], dec!(0.5));
    }

    #[test]
    fn test_bomb_pot_shared_remaining_deck() {
        // Cards dealt to boards reduce the remaining deck for both
        let mut state = OmahaBombPotGameState::new(2, 4);

        state
            .add_player(Deck::parse("As Ac 2c 3c").unwrap())
            .unwrap();
        state
            .add_player(Deck::parse("Ks Kd 4d 5d").unwrap())
            .unwrap();

        state.set_flop(0, Deck::parse("6h 7h 8h").unwrap()).unwrap();

        // Board 2 cannot reuse cards from board 1
        let result = state.set_flop(1, Deck::parse("6h 9s Ts").unwrap());
        assert!(result.is_err());

        // But different cards work
        state.set_flop(1, Deck::parse("9s Ts Js").unwrap()).unwrap();
    }
}
