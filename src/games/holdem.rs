
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::{deck::{Card, Deck}, games::{GameEquityEvaluation, GameEvaluation, GameState, GameWinner, flop_game::{FlopGame, FlopGameState}}, ranking::hand_rank::{StandardHandRanker, StandardHandRanks}};


pub struct HoldemGameState {
    flop_game_state: FlopGameState,
}

impl HoldemGameState {
    pub fn new() -> Self {
        Self { 
            flop_game_state: FlopGameState::new(2)
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

    fn add_player(&mut self, cards: Deck) -> Result<(), String> {
        self.flop_game_state.add_player(cards)
    }
    
    fn set_flop(&mut self, cards: Deck) -> Result<(), String> {
        self.flop_game_state.set_flop(cards)
    }
    
    fn set_turn(&mut self, card: Card) -> Result<(), String> {
        self.flop_game_state.set_turn(card)
    }
    
    fn set_river(&mut self, card: Card) -> Result<(), String> {
        self.flop_game_state.set_river(card)
    }
    
    fn get_player_hole_cards(&self) -> impl Iterator<Item = &Deck> {
        self.flop_game_state.get_player_hole_cards()
    }
    
    fn get_final_states<'a>(&'a self) -> impl Iterator<Item = Self> +'a {
        self.flop_game_state.get_final_states().map(|x| Self { flop_game_state: x })
    }
}


impl GameState for HoldemGameState {}

pub struct HoldemGameEvaluation {
}

impl HoldemGameEvaluation {
    fn add_winner(winners: &mut Vec<usize>, index: usize) {
        winners.push(index);
    }

    fn assign_winner(winning_hands: &mut Option<StandardHandRanks>, winners: &mut Vec<usize>, index: usize, winning_hand: StandardHandRanks) {
        *winning_hands = Some(winning_hand);
        *winners = vec![index];
    }
}

impl GameEvaluation<HoldemGameState, StandardHandRanks> for HoldemGameEvaluation {
    fn evaluate_winners(&self, game_state: &HoldemGameState) -> Vec<GameWinner<StandardHandRanks>> {
        let mut best_hand: Option<StandardHandRanks> = None;
        let mut winners = vec![];
        for (i, player) in game_state.get_player_hole_cards().enumerate() {
            let combined_deck = *player | game_state.get_community_cards();
            if let Some(rank) = StandardHandRanker::get_rank_at_least(&combined_deck, best_hand) {
                match best_hand {
                    Some(best_hand_val) => {
                        match StandardHandRanks::cmp(&best_hand_val, &rank) {
                            std::cmp::Ordering::Less => {
                                Self::assign_winner(&mut best_hand, &mut winners, i, rank);
                            }
                            std::cmp::Ordering::Equal => Self::add_winner(&mut winners, i),
                            std::cmp::Ordering::Greater => {},
                        }
                    },
                    None => {
                        Self::assign_winner(&mut best_hand, &mut winners, i, rank)
                    }
                }
            }
        }
        let winner_count = winners.len();
        if let Some(best_hand) = best_hand && winner_count > 0 {
            let pot_distribution = dec!(1.0) / Decimal::from(winner_count);
            winners.iter().map(|x| GameWinner::new(*x, pot_distribution, best_hand)).collect()
        } else { 
            vec![]
        }
    } 

}

impl GameEquityEvaluation<HoldemGameState, StandardHandRanks, HoldemGameEvaluation> for HoldemGameEvaluation {
    fn evaluate_equity(&self, game_state: &HoldemGameState) -> Vec<Decimal> {
        let mut winner_equity: Vec<Decimal> = game_state.get_player_hole_cards().map(|_| dec!(0)).collect();
        let mut hand_count = 0;
        for runout in game_state.get_final_states() {
            let winners = HoldemGameEvaluation{}.evaluate_winners(&runout);
            let num_winners = winners.len();
            let equity = if num_winners > 0 {
                dec!(1.0) / Decimal::from(num_winners)
            } else { dec!(0) };
            for winner in winners {
                winner_equity[winner.player_index] += equity;
            } 
            hand_count += 1;           
        }
        winner_equity.iter().map(|x| x / Decimal::from(hand_count)).collect()
    }
}


#[cfg(test)]
mod test {
    use rust_decimal_macros::dec;

use crate::{deck::{Card, Deck, Rank}, games::{GameEquityEvaluation, GameEvaluation, GameWinner, flop_game::FlopGame, holdem::{HoldemGameEvaluation, HoldemGameState}}, ranking::hand_rank::StandardHandRanks};

    
    #[test]
    pub fn test_holdem_hand() {
        let mut holdem_hand = HoldemGameState::new();
        holdem_hand.add_player(Deck::parse("As Ac").unwrap()).unwrap();
        holdem_hand.add_player(Deck::parse("ks kd").unwrap()).unwrap();


        let hand_evaluation = HoldemGameEvaluation{};
        // let equities = hand_evaluation.evaluate_equity(&holdem_hand);

        // assert_eq!(equities.len(), 2);
        // assert_eq!(equities[0], dec!(28063310)/ dec!(34246080));
        // assert_eq!(equities[1], dec!(6182770)/ dec!(34246080));


        holdem_hand.set_flop(Deck::parse("kc qd js").unwrap()).unwrap();

        let winners = hand_evaluation.evaluate_winners(&holdem_hand);

        assert_eq!(winners.len(), 1);
        assert_eq!(winners[0], GameWinner { player_index: 1, pot_amount: dec!(1), winning_hand: StandardHandRanks::ThreeOfAKind { t: Rank::King, c1: Rank::Queen, c2: Rank::Jack } });


        let equities = hand_evaluation.evaluate_equity(&holdem_hand);

        assert_eq!(equities.len(), 2);
        assert_eq!(equities[0], dec!(418)/ dec!(1980));
        assert_eq!(equities[1], dec!(1562)/ dec!(1980));

        holdem_hand.set_turn(Card::parse("Tc").unwrap()).unwrap();
    
        let winners = hand_evaluation.evaluate_winners(&holdem_hand);

        assert_eq!(winners.len(), 1);
        assert_eq!(winners[0], GameWinner { player_index: 0, pot_amount: dec!(1), winning_hand: StandardHandRanks::Straight { s: Rank::Ace } });


        let equities = hand_evaluation.evaluate_equity(&holdem_hand);

        assert_eq!(equities.len(), 2);
        assert_eq!(equities[0], dec!(33)/ dec!(44));
        assert_eq!(equities[1], dec!(11)/ dec!(44));

        holdem_hand.set_river(Card::parse("Ad").unwrap()).unwrap();
    
        let winners = hand_evaluation.evaluate_winners(&holdem_hand);

        assert_eq!(winners.len(), 2);
        assert_eq!(winners[0], GameWinner { player_index: 0, pot_amount: dec!(0.5), winning_hand: StandardHandRanks::Straight { s: Rank::Ace } });
        assert_eq!(winners[1], GameWinner { player_index: 1, pot_amount: dec!(0.5), winning_hand: StandardHandRanks::Straight { s: Rank::Ace } });

        let equities = hand_evaluation.evaluate_equity(&holdem_hand);

        assert_eq!(equities.len(), 2);
        assert_eq!(equities[0], dec!(1)/ dec!(2));
        assert_eq!(equities[1], dec!(1)/ dec!(2));

    }
}