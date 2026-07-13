use crate::{deck::{Card, Deck}, games::{GameState, flop_game::{FlopGame, FlopGameState}}};

pub struct OmahaGameState {
    flop_game_state: FlopGameState
}


impl OmahaGameState {
   pub fn new(cards_per_player: u32) -> Self { 
    Self { flop_game_state: FlopGameState::new(cards_per_player)}
   }
}

impl FlopGame for OmahaGameState {
     fn get_community_cards(&self) -> Deck {
        self.flop_game_state.get_community_cards()
    }

    fn add_player(&mut self, cards: Deck) -> Result<(), String> {
        self.flop_game_state.add_player(cards)
    }
    
    fn set_flop(&mut self, cards: Deck) -> Result<(), String> {
        self.flop_game_state.set_flop(cards)
    }
    
    fn set_turn(&mut self, card: Card) -> Result<(), String> {
        self.flop_game_state.set_turn(card)
    }
    
    fn set_river(&mut self, card: Card) -> Result<(), String> {
        self.flop_game_state.set_river(card)
    }
    
    fn get_player_hole_cards(&self) -> impl Iterator<Item = &Deck> {
        self.flop_game_state.get_player_hole_cards()
    }
}


impl GameState for OmahaGameState {}


#[cfg(test)]
mod test {
    use crate::{deck::Deck, games::{flop_game::FlopGame, omaha::OmahaGameState}};

    
    #[test]
    pub fn test_omaha_hand() {
       let state = OmahaGameState::new(4);
       assert_eq!(state.get_community_cards(), Deck::empty());
    }
}