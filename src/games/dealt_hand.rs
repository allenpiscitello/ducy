use crate::deck::Deck;
use crate::error::DucyError;
use crate::games::GameState;

const MAX_PLAYERS: usize = 10;

/// Game state for dealt-hand games where each player holds a fixed set of cards
/// with no community cards (used by draw and stud variants after all dealing is complete).
#[derive(Clone, Copy)]
pub struct DealtHandGameState {
    hole_cards: [Deck; MAX_PLAYERS],
    num_players: usize,
    cards_per_player: u32,
    remaining_cards: Deck,
}

impl GameState for DealtHandGameState {}

impl DealtHandGameState {
    pub fn new(cards_per_player: u32) -> Self {
        Self {
            hole_cards: [Deck::empty(); MAX_PLAYERS],
            num_players: 0,
            cards_per_player,
            remaining_cards: Deck::all_cards(),
        }
    }

    pub fn add_player(&mut self, cards: Deck) -> Result<(), DucyError> {
        if cards.num_cards() != self.cards_per_player {
            return Err(DucyError::IncorrectCardCount);
        }
        if !self.remaining_cards.has_cards(&cards) {
            return Err(DucyError::CardsNotAvailable);
        }
        if self.num_players >= MAX_PLAYERS {
            return Err(DucyError::TooManyPlayers);
        }
        self.remaining_cards -= cards;
        self.hole_cards[self.num_players] = cards;
        self.num_players += 1;
        Ok(())
    }

    pub fn hole_cards(&self) -> &[Deck] {
        &self.hole_cards[..self.num_players]
    }
}
