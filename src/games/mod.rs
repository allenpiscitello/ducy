use rust_decimal::Decimal;

use crate::ranking::hand_rank::HandRanking;

pub mod holdem;
pub mod omaha;
pub mod flop_game;

pub trait GameState {

}

#[derive(Eq, PartialEq, Debug)]
pub struct GameWinner<H: HandRanking> {
    player_index: usize, 
    pot_amount: Decimal,
    winning_hand: H,
}

impl<H: HandRanking> GameWinner<H> {
    pub fn new(index: usize, pot_amount: Decimal, winning_hand: H) -> Self {
        Self { 
            player_index: index,
            pot_amount,
            winning_hand
        }
    }
}

pub trait GameEvaluation<GS: GameState, H: HandRanking>  {
    fn evaluate_winners(&self, game_state: &GS) -> Vec<GameWinner<H>>; 
}

pub trait GameEquityEvaluation<GS: GameState, H: HandRanking, GE: GameEvaluation<GS, H>> {
    fn evaluate_equity(&self, game_state: &GS) -> Vec<Decimal>;
}