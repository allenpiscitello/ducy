use rust_decimal::Decimal;

use crate::ranking::hand_rank::HandRanking;

pub mod holdem;
pub mod omaha;


pub trait GameState {

}

pub struct GameWinner<H: HandRanking> {
    player_index: usize, 
    pot_amount: Decimal,
    winning_hand: H,
}

pub trait GameEvaluation<GS: GameState, H: HandRanking>  {
    fn evaluate_winners(&self, game_state: &GS) -> Vec<GameWinner<H>>; 
}