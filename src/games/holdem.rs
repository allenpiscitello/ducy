
use crate::{deck::Deck, games::{GameEvaluation, GameState}, ranking::hand_rank::StandardHandRanks};


pub struct HoldemGameState {
    hole_cards: Vec<Deck>,
    community_cards: Deck,
    remaining_cards_in_deck: Deck,
}


impl GameState for HoldemGameState {}

pub struct HoldemGameEvaluation {}

impl GameEvaluation<HoldemGameState, StandardHandRanks> for HoldemGameEvaluation {
    fn evaluate_winners(&self, game_state: &HoldemGameState) -> Vec<super::GameWinner<StandardHandRanks>> {
        todo!()
    }
}

