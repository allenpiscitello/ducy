
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::{deck::{Card, Deck}, games::{GameEvaluation, GameState, GameWinner}, ranking::{hand_rank::StandardHandRanks, standard_hand_ranker::StandardHandRanker}};


pub struct HoldemGameState {
    hole_cards: Vec<Deck>,
    flop: Deck,
    turn: Option<Card>,
    river: Option<Card>,
    remaining_cards_in_deck: Deck,
}


impl HoldemGameState {
    pub fn new() -> Self {
        Self { 
            hole_cards: vec![],
            flop: Deck::empty(),
            turn: None,
            river: None,
            remaining_cards_in_deck: Deck::all_cards()
        }
    }
    pub fn add_player(&mut self, cards: Deck) -> Result<(), String> {
        if cards.num_cards() != 2 {
            return Err("Incorrect number of cards".to_owned())
        }
        if !self.remaining_cards_in_deck.has_cards(&cards) {
            return Err("Cards not in deck".to_owned());
        }

        self.remaining_cards_in_deck -= cards;
        self.hole_cards.push(cards);

        Ok(())
    } 

    pub fn set_flop(&mut self, cards: Deck) -> Result<(), String> {
        if cards.num_cards() != 3 {
            return Err("Incorrect number of cards".to_owned())
        }
        if !self.remaining_cards_in_deck.has_cards(&cards) {
            return Err("Cards not in deck".to_owned());
        }
        self.remaining_cards_in_deck -= cards;
        self.flop = cards;
        
        Ok(())
        
    }

    pub fn set_turn(&mut self, card: Card) -> Result<(), String> {
        if !self.remaining_cards_in_deck.has_card(&card) {
            return Err("Card is not in deck".to_owned())
        } 
        self.remaining_cards_in_deck.remove_cards([card].into_iter());
        self.turn = Some(card);
        Ok(())   
    }

    pub fn set_river(&mut self, card: Card) -> Result<(), String> {
        if !self.remaining_cards_in_deck.has_card(&card) {
            return Err("Card is not in deck".to_owned())
        } 
        self.remaining_cards_in_deck.remove_cards([card].into_iter());
        self.river = Some(card);
        Ok(())   
    }

    pub fn get_community_cards(&self) -> Deck {
        let mut cards = self.flop.clone();
        let mut other_cards = vec![];
        if let Some(turn) = self.turn {
            other_cards.push(turn);
        }
        if let Some(river) = self.river {
            other_cards.push(river);
        }
        cards.insert_cards(other_cards.iter());
        cards
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
        for (i, player) in game_state.hole_cards.iter().enumerate() {
            let combined_deck = *player | game_state.get_community_cards();
            let rank = StandardHandRanker::get_rank(&combined_deck);
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
        let winner_count = winners.len();
        if let Some(best_hand) = best_hand && winner_count > 0 {
            let pot_distribution = dec!(1.0) / Decimal::try_from(winner_count).unwrap();
            winners.iter().map(|x| GameWinner::new(*x, pot_distribution, best_hand)).collect()
        } else { 
            vec![]
        }
    }

}

#[cfg(test)]
mod test {
    use rust_decimal_macros::dec;

use crate::{deck::{Card, Deck, Rank}, games::{GameEvaluation, GameWinner, holdem::{HoldemGameEvaluation, HoldemGameState}}, ranking::hand_rank::StandardHandRanks};

    
    #[test]
    pub fn test_holdem_hand() {
        let mut holdem_hand = HoldemGameState::new();
        holdem_hand.add_player(Deck::parse("As Ac").unwrap()).unwrap();
        holdem_hand.add_player(Deck::parse("ks kd").unwrap()).unwrap();
        holdem_hand.set_flop(Deck::parse("kc qd js").unwrap()).unwrap();
        
        let winners = HoldemGameEvaluation{}.evaluate_winners(&holdem_hand);

        assert_eq!(winners.len(), 1);
        assert_eq!(winners[0], GameWinner { player_index: 1, pot_amount: dec!(1), winning_hand: StandardHandRanks::ThreeOfAKind { t: Rank::King, c1: Rank::Queen, c2: Rank::Jack } });

        holdem_hand.set_turn(Card::parse("Tc").unwrap()).unwrap();
    
        let winners = HoldemGameEvaluation{}.evaluate_winners(&holdem_hand);

        assert_eq!(winners.len(), 1);
        assert_eq!(winners[0], GameWinner { player_index: 0, pot_amount: dec!(1), winning_hand: StandardHandRanks::Straight { s: Rank::Ace } });

        holdem_hand.set_river(Card::parse("Ad").unwrap()).unwrap();
    
        let winners = HoldemGameEvaluation{}.evaluate_winners(&holdem_hand);

        assert_eq!(winners.len(), 2);
        assert_eq!(winners[0], GameWinner { player_index: 0, pot_amount: dec!(0.5), winning_hand: StandardHandRanks::Straight { s: Rank::Ace } });
        assert_eq!(winners[1], GameWinner { player_index: 1, pot_amount: dec!(0.5), winning_hand: StandardHandRanks::Straight { s: Rank::Ace } });
     
    }

}