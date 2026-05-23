#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rank {
    Ace,
    Two,
    Three,
    Four,
    Five,
    Six,
    Seven,
    Eight,
    Nine,
    Ten,
    Jack,
    Queen,
    King,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Suit {
    Hearts,
    Diamonds,
    Clubs,
    Spades,
}

pub trait Cardlike {
    fn rank(&self) -> Rank;
    fn suit(&self) -> Suit;
}

pub struct SimpleCard {
    rank: Rank,
    suit: Suit,
}

impl SimpleCard {
    pub fn new(rank: Rank, suit: Suit) -> SimpleCard {
        return SimpleCard { rank, suit };
    }
}

impl Cardlike for SimpleCard {
    fn rank(&self) -> Rank {
        self.rank.clone()
    }

    fn suit(&self) -> Suit {
        self.suit.clone()
    }
}

#[cfg(test)]
pub mod test {
    use super::*;

    #[test]
    pub fn create_card_works() {
        let card = SimpleCard::new(Rank::Ace, Suit::Spades);
        assert_eq!(Rank::Ace, card.rank);
        assert_eq!(Suit::Spades, card.suit);
    }
}
