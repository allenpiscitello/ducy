use crate::{deck::{Card, Deck}};


pub struct FlopGameState {
    hole_cards: Vec<Deck>,
    flop: Deck,
    turn: Option<Card>,
    river: Option<Card>,
    remaining_cards_in_deck: Deck,
    num_hole_cards_per_player: u32,
}

impl FlopGameState {
    pub fn new(num_hole_cards_per_player: u32) -> Self {
        Self { 
            hole_cards: vec![],
            flop: Deck::empty(),
            turn: None,
            river: None,
            remaining_cards_in_deck: Deck::all_cards(),
            num_hole_cards_per_player,
        }
    }
}

impl FlopGame for FlopGameState {
    
    fn add_player(&mut self, cards: Deck) -> Result<(), String> {
        if cards.num_cards() != self.num_hole_cards_per_player {
            return Err("Incorrect number of cards".to_owned())
        }
        if !self.remaining_cards_in_deck.has_cards(&cards) {
            return Err("Cards not in deck".to_owned());
        }

        self.remaining_cards_in_deck -= cards;
        self.hole_cards.push(cards);

        Ok(())
    } 

    fn set_flop(&mut self, cards: Deck) -> Result<(), String> {
        if cards.num_cards() != 3 {
            return Err("Incorrect number of cards".to_owned())
        }
        if !self.remaining_cards_in_deck.has_cards(&cards) {
            return Err("Cards not in deck".to_owned());
        }
        self.remaining_cards_in_deck -= cards;
        self.flop = cards;
        
        Ok(())
        
    }

    fn set_turn(&mut self, card: Card) -> Result<(), String> {
        if !self.remaining_cards_in_deck.has_card(&card) {
            return Err("Card is not in deck".to_owned())
        } 
        self.remaining_cards_in_deck.remove_cards([card].into_iter());
        self.turn = Some(card);
        Ok(())   
    }

    fn set_river(&mut self, card: Card) -> Result<(), String> {
        if !self.remaining_cards_in_deck.has_card(&card) {
            return Err("Card is not in deck".to_owned())
        } 
        self.remaining_cards_in_deck.remove_cards([card].into_iter());
        self.river = Some(card);
        Ok(())   
    }

    fn get_community_cards(&self) -> Deck {
        let mut cards = self.flop;
        let mut other_cards = vec![];
        if let Some(turn) = self.turn {
            other_cards.push(turn);
        }
        if let Some(river) = self.river {
            other_cards.push(river);
        }
        cards.insert_cards(other_cards.iter());
        cards
    }
    
    fn get_player_hole_cards(&self) -> impl Iterator<Item = &Deck> {
        self.hole_cards.iter()
    }
}

pub trait FlopGame {
    fn get_community_cards(&self) -> Deck;
    fn add_player(&mut self, cards: Deck) -> Result<(), String>;
    
    fn set_flop(&mut self, cards: Deck) -> Result<(), String>;

    fn set_turn(&mut self, card: Card) -> Result<(), String>;

    fn set_river(&mut self, card: Card) -> Result<(), String>;

    fn get_player_hole_cards(&self) -> impl Iterator<Item = &Deck>;
}