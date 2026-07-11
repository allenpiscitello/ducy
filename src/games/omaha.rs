use crate::{deck::Deck, games::GameState};

struct OmahaGameState {
    hole_cards: Vec<Deck>,
    community_cards: Deck,
    remaining_cards_in_deck: Deck,
}

impl GameState for OmahaGameState {}