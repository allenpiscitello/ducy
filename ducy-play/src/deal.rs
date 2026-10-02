use ducy::deck::{Card, Deck};
use rand::{RngExt, SeedableRng, rngs::StdRng};

use crate::{error::PlayError, rules::Variant};

/// Every card a hand can use: each player's hole cards and the full board.
/// Board cards are revealed street by street as the hand is played.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deal {
    hole_cards: Vec<Deck>,
    board: [Card; 5],
}

impl Deal {
    /// A deal with exact cards, e.g. to replay a hand or test a spot.
    /// `hole_cards[i]` belongs to seat `i`; `board` is flop, turn, river in order.
    pub fn new(
        variant: Variant,
        hole_cards: Vec<Deck>,
        board: [Card; 5],
    ) -> Result<Self, PlayError> {
        let mut seen = Deck::empty();
        for hand in &hole_cards {
            if hand.num_cards() as usize != variant.hole_cards()
                || u64::from(seen) & u64::from(*hand) != 0
            {
                return Err(PlayError::InvalidDeal);
            }
            seen |= *hand;
        }
        for card in board {
            if seen.has_card(&card) {
                return Err(PlayError::InvalidDeal);
            }
            seen |= card;
        }
        Ok(Self { hole_cards, board })
    }

    /// A shuffled deal for `players` players. Passing a `seed` makes it
    /// reproducible.
    pub fn random(variant: Variant, players: usize, seed: Option<u64>) -> Result<Self, PlayError> {
        let per_player = variant.hole_cards();
        if players * per_player + 5 > 52 {
            return Err(PlayError::NotEnoughCards);
        }
        let mut rng: StdRng = match seed {
            Some(seed) => StdRng::seed_from_u64(seed),
            None => rand::make_rng(),
        };
        let mut cards: Vec<Card> = Deck::all_cards().iter(false).collect();
        for i in 0..cards.len() {
            let j = rng.random_range(i..cards.len());
            cards.swap(i, j);
        }
        let mut next = cards.into_iter();
        let hole_cards = (0..players)
            .map(|_| {
                let mut hand = Deck::empty();
                for card in next.by_ref().take(per_player) {
                    hand |= card;
                }
                hand
            })
            .collect();
        let board: Vec<Card> = next.take(5).collect();
        let board = board.try_into().map_err(|_| PlayError::NotEnoughCards)?;
        Ok(Self { hole_cards, board })
    }

    /// Hole cards for each seat.
    pub fn hole_cards(&self) -> &[Deck] {
        &self.hole_cards
    }

    /// The full five-card board.
    pub fn board(&self) -> &[Card; 5] {
        &self.board
    }
}
