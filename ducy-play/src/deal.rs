use ducy::deck::{Card, Deck};
use rand::{RngExt, SeedableRng, rngs::StdRng};

use crate::{error::PlayError, rules::Variant};

/// Where a hand's cards come from (#135).
///
/// A [`Deal`] knows every card up front, as tables have always worked. A
/// dealer for the trustless shuffle (#134) doesn't let the engine see them:
/// a player's hole cards are known only to that player until they show them
/// at showdown ([`crate::Hand::reveal`]), and the board arrives street by
/// street as the host unlocks it ([`crate::Hand::deal_board`]); meanwhile the
/// hand waits ([`crate::Hand::awaiting`]).
pub trait Dealer {
    /// How many players are dealt in.
    fn players(&self) -> usize;
    /// Player `player`'s hole cards, or `None` if only that player may see
    /// them.
    fn hole_cards(&self, player: usize) -> Option<Deck>;
    /// The whole board, if it's known up front; otherwise it's dealt street
    /// by street with [`crate::Hand::deal_board`].
    fn board(&self) -> Option<[Card; 5]>;
}

impl Dealer for Deal {
    fn players(&self) -> usize {
        self.hole_cards.len()
    }

    fn hole_cards(&self, player: usize) -> Option<Deck> {
        self.hole_cards.get(player).copied()
    }

    fn board(&self) -> Option<[Card; 5]> {
        Some(self.board)
    }
}

/// A deal the engine never sees: every player's hole cards stay with that
/// player until showdown, and the board is dealt street by street. The cards
/// themselves come from outside, e.g. a trustless shuffle (#134).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HiddenDeal {
    /// How many players are dealt in.
    pub players: usize,
}

impl Dealer for HiddenDeal {
    fn players(&self) -> usize {
        self.players
    }

    fn hole_cards(&self, _player: usize) -> Option<Deck> {
        None
    }

    fn board(&self) -> Option<[Card; 5]> {
        None
    }
}

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
        Self::from_order(variant, players, cards)
    }

    /// The deal for a provably fair seed agreed by the players and the host
    /// ([`crate::fair_deal`]): anyone with the seed gets the same cards.
    pub fn from_seed(variant: Variant, players: usize, seed: &[u8; 32]) -> Result<Self, PlayError> {
        if players * variant.hole_cards() + 5 > 52 {
            return Err(PlayError::NotEnoughCards);
        }
        Self::from_order(variant, players, crate::fair_deal::shuffled_deck(seed))
    }

    /// Deals from a shuffled deck: each player's hole cards in turn, then the
    /// board.
    fn from_order(variant: Variant, players: usize, cards: Vec<Card>) -> Result<Self, PlayError> {
        let per_player = variant.hole_cards();
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
