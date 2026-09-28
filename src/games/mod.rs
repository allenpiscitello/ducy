use rust_decimal::Decimal;
use rust_decimal_macros::dec;

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

pub(crate) struct WinnerTracker<H: HandRanking + Ord + Copy> {
    best_hand: Option<H>,
    winners: Vec<usize>,
}

impl<H: HandRanking + Ord + Copy> WinnerTracker<H> {
    pub fn new() -> Self {
        Self {
            best_hand: None,
            winners: vec![],
        }
    }

    pub fn best_hand(&self) -> Option<H> {
        self.best_hand
    }

    pub fn consider(&mut self, player_index: usize, hand: H) {
        match &self.best_hand {
            Some(best) => match best.cmp(&hand) {
                std::cmp::Ordering::Less => {
                    self.best_hand = Some(hand);
                    self.winners = vec![player_index];
                }
                std::cmp::Ordering::Equal => self.winners.push(player_index),
                std::cmp::Ordering::Greater => {}
            },
            None => {
                self.best_hand = Some(hand);
                self.winners = vec![player_index];
            }
        }
    }

    pub fn into_results(self) -> Vec<GameWinner<H>> {
        let winner_count = self.winners.len();
        if let Some(best_hand) = self.best_hand
            && winner_count > 0
        {
            let pot_distribution = dec!(1.0) / Decimal::from(winner_count);
            self.winners
                .iter()
                .map(|x| GameWinner::new(*x, pot_distribution, best_hand))
                .collect()
        } else {
            vec![]
        }
    }
}

pub trait GameEvaluation<GS: GameState, H: HandRanking> {
    fn evaluate_winners(&self, game_state: &GS) -> Vec<GameWinner<H>>;
}

pub trait GameEquityEvaluation<GS: GameState, H: HandRanking, GE: GameEvaluation<GS, H>> {
    fn evaluate_equity(&self, game_state: &GS) -> Vec<Decimal>;
}
