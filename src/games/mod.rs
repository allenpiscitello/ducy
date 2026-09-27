use rust_decimal::Decimal;

use crate::ranking::hand_rank::HandRanking;

pub mod flop_game;
pub mod holdem;
pub mod omaha;

pub trait GameState {}

#[derive(Eq, PartialEq, Debug)]
pub struct GameWinner<H: HandRanking> {
    pub player_index: usize,
    pub pot_amount: Decimal,
    pub winning_hand: H,
}

impl<H: HandRanking> GameWinner<H> {
    pub fn new(index: usize, pot_amount: Decimal, winning_hand: H) -> Self {
        Self {
            player_index: index,
            pot_amount,
            winning_hand,
        }
    }

    pub fn player_index(&self) -> usize {
        self.player_index
    }

    pub fn pot_amount(&self) -> Decimal {
        self.pot_amount
    }

    pub fn winning_hand(&self) -> &H {
        &self.winning_hand
    }
}

pub trait GameEvaluation<GS: GameState, H: HandRanking> {
    fn evaluate_winners(&self, game_state: &GS) -> Vec<GameWinner<H>>;
}

pub trait GameEquityEvaluation<GS: GameState, H: HandRanking, GE: GameEvaluation<GS, H>> {
    fn evaluate_equity(&self, game_state: &GS) -> Vec<Decimal>;
}
