use crate::{deck::{Card, Deck}};


#[derive(Clone)]
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
        // TODO: need to have flop set
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
    
    fn get_final_states<'a>(&'a self) -> impl Iterator<Item = Self> +'a {
        // TODO: This can be optimized by not caring about which card is flop/turn/river and do way fewer evaluations
    
        if self.flop.is_empty() {
            let flop_iterator = self.remaining_cards_in_deck.enumerate_combinations(3);
            FlopGameStateIterator::Flop { iterator: FlopIterator { base_state: self.clone(), flop_iterator: Box::new(flop_iterator), turn_iterator: None } } 
        } else if self.turn.is_none() {
            let turn_iterator = self.remaining_cards_in_deck.iter(true);
            FlopGameStateIterator::Turn { iterator: TurnIterator {
                base_state: self.clone(),
                turn_iterator: Box::new(turn_iterator),
                river_iterator: None,
                }
            } 
        }
        else if self.river.is_none() {
            let river_iterator = self.remaining_cards_in_deck.iter(true);
            FlopGameStateIterator::River { 
                iterator: RiverIterator { 
                    base_state: self.clone(),
                    river_iterator: Box::new(river_iterator),
                }
            }
        }
        else {
            FlopGameStateIterator::Complete { game_state: self.clone(), iterated: false }
        }
    }
}

enum FlopGameStateIterator {
    Flop {
        iterator: FlopIterator,
    },
    Turn { 
       iterator: TurnIterator
    },
    River {
        iterator: RiverIterator
    },
    Complete {
        game_state: FlopGameState,
        iterated: bool,
    }
    
}

impl Iterator for FlopGameStateIterator {
    type Item = FlopGameState;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            FlopGameStateIterator::Flop { iterator } => iterator.next(),
            FlopGameStateIterator::Turn { iterator } => iterator.next(),
            FlopGameStateIterator::River { iterator } => iterator.next(),
            FlopGameStateIterator::Complete { game_state,iterated  } => {
                if *iterated { None } else { 
                    *iterated = true;
                    Some(game_state.clone())}
                },
        }
    }
}


struct FlopIterator {
    base_state: FlopGameState,
    flop_iterator: Box<dyn Iterator<Item=Deck>>,
    turn_iterator: Option<TurnIterator>,
}


impl Iterator for FlopIterator {
    type Item = FlopGameState;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(turn_iterator) = &mut self.turn_iterator {
            match turn_iterator.next() {
                Some(state) => Some(state),
                None => {
                    self.turn_iterator = None;
                    self.next()
                }
            }
        } else {
            match self.flop_iterator.next() {
                Some(flop) => {
                    let mut game_state = self.base_state.clone();
                    game_state.set_flop(flop).unwrap();
                    self.turn_iterator = 
                    Some(TurnIterator {
                        base_state: game_state.clone(),
                        turn_iterator: Box::new(game_state.remaining_cards_in_deck.iter(true)),
                        river_iterator: None, 
                    });
                    self.next()
                }
                None => None
            }
            
        }
    }
}

struct TurnIterator {
    base_state: FlopGameState,
    turn_iterator: Box<dyn Iterator<Item=Card>>,
    river_iterator: Option<RiverIterator>,
}

impl Iterator for TurnIterator {
    type Item = FlopGameState;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(river_iterator) = &mut self.river_iterator {
            match river_iterator.next() {
                Some(state) => Some(state),
                None => {
                    self.river_iterator = None;
                    self.next()
                }
            }
        } else {
            match self.turn_iterator.next() {
                Some(turn) => {
                    let mut game_state = self.base_state.clone();
                    game_state.set_turn(turn).unwrap();
                    self.river_iterator = 
                    Some(RiverIterator {
                        base_state: game_state.clone(),
                        river_iterator: Box::new(game_state.remaining_cards_in_deck.iter(true))
                    });
                    self.next()
                }
                None => None
            }
            
        }
    }
}


struct RiverIterator {
    base_state: FlopGameState,
    river_iterator: Box<dyn Iterator<Item=Card>>,
}

impl Iterator for RiverIterator {
    type Item = FlopGameState;

    fn next(&mut self) -> Option<Self::Item> {
        let mut new_state = self.base_state.clone();
        if let Some(river) = self.river_iterator.next() {
            new_state.set_river(river).unwrap();
            Some(new_state)
        }
        else {
            None
        }
    }
}


pub trait FlopGame {
    fn get_community_cards(&self) -> Deck;
    fn add_player(&mut self, cards: Deck) -> Result<(), String>;
    
    fn set_flop(&mut self, cards: Deck) -> Result<(), String>;

    fn set_turn(&mut self, card: Card) -> Result<(), String>;

    fn set_river(&mut self, card: Card) -> Result<(), String>;

    fn get_player_hole_cards(&self) -> impl Iterator<Item = &Deck>;

    fn get_final_states<'a>(&'a self) -> impl Iterator<Item = Self> +'a;
}