use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::{
    deck::{Card, Deck, Suit},
    error::DucyError,
    games::{
        GameEquityEvaluation, GameEvaluation, GameState, GameWinner,
        flop_game::{FlopGame, FlopGameState},
    },
    ranking::hand_rank::{StandardHandRanker, StandardHandRanks},
};

pub struct OmahaGameState {
    flop_game_state: FlopGameState,
}

impl OmahaGameState {
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
pub enum BoardTone {
    Monotone { suit: Suit },
    FourFlush { suit: Suit },
    ThreeFlush { suit: Suit },
    Rainbow {},
}

pub enum BoardPairs {
    NoPairs,
    OnePair,
    Trips,
    Quads,
    TwoPairs,
    FullHouse,
}

pub enum BoardTexture {
    Todo,
}

pub trait OmahaBoardAnalysis {
    fn board_tone(&self) -> BoardTone;
    fn board_pairs(&self) -> BoardPairs;
    fn board_texture(&self) -> BoardTexture;
    fn board_straight_flushes(&self) -> BoardTexture;
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

    fn board_pairs(&self) -> BoardPairs {
        let _ranks = self.get_community_cards().get_rank_count();
        todo!()
    }

    fn board_texture(&self) -> BoardTexture {
        todo!()
    }

    fn board_straight_flushes(&self) -> BoardTexture {
        todo!()
    }
}

pub struct OmahaGameEvaluation {}

impl OmahaGameEvaluation {
    fn add_winner(winners: &mut Vec<usize>, index: usize) {
        winners.push(index);
    }

    fn assign_winner(
        winning_hands: &mut Option<StandardHandRanks>,
        winners: &mut Vec<usize>,
        index: usize,
        winning_hand: StandardHandRanks,
    ) {
        *winning_hands = Some(winning_hand);
        *winners = vec![index];
    }
}

impl GameEvaluation<OmahaGameState, StandardHandRanks> for OmahaGameEvaluation {
    fn evaluate_winners(&self, game_state: &OmahaGameState) -> Vec<GameWinner<StandardHandRanks>> {
        let mut best_hand: Option<StandardHandRanks> = None;
        let mut winners = vec![];
        for (i, player) in game_state.get_player_hole_cards().enumerate() {
            for community_cards_of_3 in game_state.get_community_cards().enumerate_combinations(3) {
                for player_cards_group_of_2 in player.enumerate_combinations(2) {
                    let combined_deck = community_cards_of_3 | player_cards_group_of_2;
                    if let Some(rank) =
                        StandardHandRanker::get_rank_at_least(&combined_deck, best_hand)
                    {
                        match best_hand {
                            Some(best_hand_val) => {
                                match StandardHandRanks::cmp(&best_hand_val, &rank) {
                                    std::cmp::Ordering::Less => {
                                        Self::assign_winner(&mut best_hand, &mut winners, i, rank);
                                    }
                                    std::cmp::Ordering::Equal => Self::add_winner(&mut winners, i),
                                    std::cmp::Ordering::Greater => {}
                                }
                            }
                            None => Self::assign_winner(&mut best_hand, &mut winners, i, rank),
                        }
                    }
                }
            }
        }
        let winner_count = winners.len();
        if let Some(best_hand) = best_hand
            && winner_count > 0
        {
            let pot_distribution = dec!(1.0) / Decimal::from(winner_count);
            winners
                .iter()
                .map(|x| GameWinner::new(*x, pot_distribution, best_hand))
                .collect()
        } else {
            vec![]
        }
    }
}

impl GameEquityEvaluation<OmahaGameState, StandardHandRanks, OmahaGameEvaluation>
    for OmahaGameEvaluation
{
    fn evaluate_equity(&self, game_state: &OmahaGameState) -> Vec<Decimal> {
        let mut winner_equity: Vec<Decimal> = game_state
            .get_player_hole_cards()
            .map(|_| dec!(0))
            .collect();
        let mut hand_count = 0;
        for runout in game_state.get_final_states() {
            let winners = OmahaGameEvaluation {}.evaluate_winners(&runout);
            let num_winners = winners.len();
            let equity = if num_winners > 0 {
                dec!(1.0) / Decimal::from(num_winners)
            } else {
                dec!(0)
            };
            for winner in winners {
                winner_equity[winner.player_index] += equity;
            }
            hand_count += 1;
        }
        winner_equity
            .iter()
            .map(|x| x / Decimal::from(hand_count))
            .collect()
    }
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
