use strum::IntoEnumIterator;

use crate::deck::card::{Cardlike, SimpleCard};
use crate::deck::rank::Rank;
use crate::deck::suit::Suit;

pub mod card;
pub mod rank;
pub mod suit;

pub trait Decklike {
    type Card: Cardlike + Copy + Clone;

    fn get_next_card(&mut self) -> Option<Self::Card>;

    fn cards_remaining(&self) -> u8;
}

pub struct StandardDeck {
    cards: Vec<SimpleCard>,
}

impl StandardDeck {
    pub fn new() -> Self {
        let mut cards = vec![];
        for suit in Suit::iter() {
            for rank in Rank::iter() {
                cards.push(SimpleCard::new(rank, suit));
            }
        }
        Self { cards }
    }

    #[cfg(test)]
    fn seed_deck(&mut self, _seed: i64) {}
}

impl Decklike for StandardDeck {
    type Card = SimpleCard;

    fn get_next_card(&mut self) -> Option<Self::Card> {
        // TODO: Make this random in the future
        if self.cards.len() > 0 {
            Some(self.cards.remove(0))
        } else {
            None
        }
    }

    fn cards_remaining(&self) -> u8 {
        self.cards.len() as u8
    }
}

#[cfg(test)]
mod test {
    use crate::deck::rank::Rank;
    use crate::deck::suit::Suit;
    use crate::deck::{Decklike, StandardDeck, card::Cardlike};

    #[test]
    pub fn deal_a_flop() -> Result<(), String> {
        let mut deck = StandardDeck::new();
        deck.seed_deck(1);
        assert_eq!(52, deck.cards_remaining());

        test_card(&mut deck, Suit::Clubs, Rank::Ace);
        test_card(&mut deck, Suit::Clubs, Rank::Two);
        test_card(&mut deck, Suit::Clubs, Rank::Three);

        Ok(())
    }

    pub fn test_card(deck: &mut StandardDeck, suit: Suit, rank: Rank) {
        let card = deck.get_next_card().unwrap();
        assert_eq!(suit, card.suit());
        assert_eq!(rank, card.rank());
    }
}
