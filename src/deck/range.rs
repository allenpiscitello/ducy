use std::{collections::HashMap};

use crate::deck::Deck;
use rust_decimal::Decimal;

pub struct RangeItem {
    deck: Deck,
    weight: Decimal,
}

impl RangeItem {
    pub fn get_deck(&self) -> Deck { self.deck}
    pub fn get_weight(&self) -> Decimal { self.weight }
}

pub trait Range {
    fn iter(&self) -> impl Iterator<Item = RangeItem>;
}


pub struct RangeBase { 
    weights: HashMap<Deck, Decimal>
}
impl RangeBase {
    pub(crate) fn new() -> Self {
        Self { weights: HashMap::new() }
    }

    pub fn add_deck_weight(&mut self, deck: Deck, weight: Decimal) {
        self.weights.insert(deck, weight);
    }
}

impl Range for RangeBase {
    fn iter(&self) -> impl Iterator<Item = RangeItem> {
        self.weights.iter().map(|(deck, weight)| RangeItem { deck: *deck, weight: *weight})      
    }
}