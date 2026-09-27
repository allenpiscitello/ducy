pub mod deck;
pub mod error;
pub mod games;
pub mod ranking;

#[cfg(test)]
pub(crate) mod test_util {
    use crate::deck::{Card, Deck};

    pub fn deck_from_cards(val: &str) -> Deck {
        let card_strs = val.split(" ");
        let cards: Vec<Card> = card_strs.map(|x| Card::parse(x).unwrap()).collect();
        let mut deck = Deck::empty();
        deck.insert_cards(cards.iter());
        deck
    }
}
