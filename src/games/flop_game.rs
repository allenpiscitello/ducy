use crate::deck::{Card, Deck};
use crate::error::DucyError;
use crate::games::CardDealer;

const MAX_PLAYERS: usize = 10;

/// Shared state for flop-based poker games (community cards, hole cards, remaining deck).
#[derive(Clone, Copy)]
pub struct FlopGameState {
    hole_cards: [Deck; MAX_PLAYERS],
    num_players: usize,
    flop: Deck,
    turn: Option<Card>,
    river: Option<Card>,
    community_cards: Deck,
    remaining_cards_in_deck: Deck,
    num_hole_cards_per_player: u32,
}

impl FlopGameState {
    /// Creates a new game state with the specified number of hole cards per player.
    pub fn new(num_hole_cards_per_player: u32) -> Self {
        Self {
            hole_cards: [Deck::empty(); MAX_PLAYERS],
            num_players: 0,
            flop: Deck::empty(),
            turn: None,
            river: None,
            community_cards: Deck::empty(),
            remaining_cards_in_deck: Deck::all_cards(),
            num_hole_cards_per_player,
        }
    }

    /// Adds community cards to the board (max 5 total).
    pub fn add_community_cards(&mut self, deck: &Deck) -> Result<(), DucyError> {
        if self.community_cards.num_cards() + deck.num_cards() > 5 {
            return Err(DucyError::TooManyCommunityCards);
        }
        self.community_cards |= *deck;
        Ok(())
    }

    pub(crate) fn remaining_cards(&self) -> Deck {
        self.remaining_cards_in_deck
    }

    pub(crate) fn cards_needed(&self) -> usize {
        if self.flop.is_empty() {
            5
        } else if self.turn.is_none() {
            2
        } else if self.river.is_none() {
            1
        } else {
            0
        }
    }

    /// Returns `samples` random complete boards drawn from the remaining deck.
    pub fn sample_runout_community_cards(&self, samples: usize) -> Vec<Deck> {
        let cards_needed = self.cards_needed();
        let mut dealer = CardDealer::new(self.remaining_cards_in_deck);
        (0..samples)
            .map(|_| {
                dealer.reset();
                self.community_cards | dealer.deal(cards_needed)
            })
            .collect()
    }

    pub fn enumerate_runout_community_cards(&self) -> impl Iterator<Item = Deck> + '_ {
        let cards_needed = self.cards_needed();

        let base_community = self.community_cards;
        let complete = if cards_needed == 0 {
            Some(base_community)
        } else {
            None
        };

        let combos: Box<dyn Iterator<Item = Deck>> = if cards_needed > 0 {
            Box::new(
                self.remaining_cards_in_deck
                    .enumerate_combinations(cards_needed),
            )
        } else {
            Box::new(std::iter::empty())
        };

        complete
            .into_iter()
            .chain(combos.map(move |c| base_community | c))
    }

    pub fn hole_cards(&self) -> &[Deck] {
        &self.hole_cards[..self.num_players]
    }
}

impl FlopGame for FlopGameState {
    fn add_player(&mut self, cards: Deck) -> Result<(), DucyError> {
        if cards.num_cards() != self.num_hole_cards_per_player {
            return Err(DucyError::IncorrectCardCount);
        }
        if !self.remaining_cards_in_deck.has_cards(&cards) {
            return Err(DucyError::CardsNotAvailable);
        }
        if self.num_players >= MAX_PLAYERS {
            return Err(DucyError::TooManyPlayers);
        }

        self.remaining_cards_in_deck -= cards;
        self.hole_cards[self.num_players] = cards;
        self.num_players += 1;

        Ok(())
    }

    fn set_flop(&mut self, cards: Deck) -> Result<(), DucyError> {
        if cards.num_cards() != 3 {
            return Err(DucyError::IncorrectCardCount);
        }
        if !self.remaining_cards_in_deck.has_cards(&cards) {
            return Err(DucyError::CardsNotAvailable);
        }
        self.remaining_cards_in_deck -= cards;
        self.flop = cards;
        self.community_cards |= cards;

        Ok(())
    }

    fn set_turn(&mut self, card: Card) -> Result<(), DucyError> {
        if self.flop.is_empty() {
            return Err(DucyError::FlopNotSet);
        }
        if !self.remaining_cards_in_deck.has_card(&card) {
            return Err(DucyError::CardsNotAvailable);
        }
        self.remaining_cards_in_deck
            .remove_cards([card].into_iter());
        self.turn = Some(card);
        self.community_cards |= card;
        Ok(())
    }

    fn set_river(&mut self, card: Card) -> Result<(), DucyError> {
        if self.turn.is_none() {
            return Err(DucyError::TurnNotSet);
        }
        if !self.remaining_cards_in_deck.has_card(&card) {
            return Err(DucyError::CardsNotAvailable);
        }
        self.remaining_cards_in_deck
            .remove_cards([card].into_iter());
        self.river = Some(card);
        self.community_cards |= card;
        Ok(())
    }

    fn add_dead_cards(&mut self, cards: Deck) -> Result<(), DucyError> {
        if !self.remaining_cards_in_deck.has_cards(&cards) {
            return Err(DucyError::CardsNotAvailable);
        }
        self.remaining_cards_in_deck -= cards;
        Ok(())
    }

    fn get_community_cards(&self) -> Deck {
        self.community_cards
    }

    fn get_player_hole_cards(&self) -> impl Iterator<Item = &Deck> {
        self.hole_cards[..self.num_players].iter()
    }

    fn get_final_states<'a>(&'a self) -> impl Iterator<Item = Self> + 'a {
        if self.flop.is_empty() {
            FlopGameStateIterator::AllCards {
                iterator: CommunityCardIterator {
                    base_state: *self,
                    iterator: Box::new(self.remaining_cards_in_deck.enumerate_combinations(5)),
                },
            }
        } else if self.turn.is_none() {
            FlopGameStateIterator::AllCards {
                iterator: CommunityCardIterator {
                    base_state: *self,
                    iterator: Box::new(self.remaining_cards_in_deck.enumerate_combinations(2)),
                },
            }
        } else if self.river.is_none() {
            FlopGameStateIterator::AllCards {
                iterator: CommunityCardIterator {
                    base_state: *self,
                    iterator: Box::new(self.remaining_cards_in_deck.enumerate_combinations(1)),
                },
            }
        } else {
            FlopGameStateIterator::Complete {
                game_state: *self,
                iterated: false,
            }
        }
    }
}

enum FlopGameStateIterator {
    AllCards {
        iterator: CommunityCardIterator,
    },
    Complete {
        game_state: FlopGameState,
        iterated: bool,
    },
}

impl Iterator for FlopGameStateIterator {
    type Item = FlopGameState;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            FlopGameStateIterator::AllCards { iterator } => iterator.next(),
            FlopGameStateIterator::Complete {
                game_state,
                iterated,
            } => {
                if *iterated {
                    None
                } else {
                    *iterated = true;
                    Some(*game_state)
                }
            }
        }
    }
}

struct CommunityCardIterator {
    base_state: FlopGameState,
    iterator: Box<dyn Iterator<Item = Deck>>,
}

impl Iterator for CommunityCardIterator {
    type Item = FlopGameState;
    fn next(&mut self) -> Option<Self::Item> {
        self.iterator.next().map(|x| {
            let mut game_state = self.base_state;
            game_state.add_community_cards(&x).unwrap();
            game_state
        })
    }
}

/// Trait for flop-based poker game variants (Hold'em, Omaha).
pub trait FlopGame {
    /// Returns the current community cards.
    fn get_community_cards(&self) -> Deck;
    /// Adds a player with the given hole cards.
    fn add_player(&mut self, cards: Deck) -> Result<(), DucyError>;
    /// Sets the three flop cards.
    fn set_flop(&mut self, cards: Deck) -> Result<(), DucyError>;
    /// Sets the turn card (requires flop to be set).
    fn set_turn(&mut self, card: Card) -> Result<(), DucyError>;
    /// Sets the river card (requires turn to be set).
    fn set_river(&mut self, card: Card) -> Result<(), DucyError>;
    /// Removes cards known to be out of play (dead cards) from the remaining deck.
    fn add_dead_cards(&mut self, cards: Deck) -> Result<(), DucyError>;
    /// Returns an iterator over each player's hole cards.
    fn get_player_hole_cards(&self) -> impl Iterator<Item = &Deck>;
    /// Returns an iterator over all possible final board states.
    fn get_final_states<'a>(&'a self) -> impl Iterator<Item = Self> + 'a;
}
