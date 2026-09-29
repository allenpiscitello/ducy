use wasm_bindgen::prelude::*;

use ducy::deck::{Card, Deck};
use ducy::games::flop_game::FlopGame;
use ducy::games::holdem::{HoldemGameEvaluation, HoldemGameState};
use ducy::games::omaha::{OmahaGameEvaluation, OmahaGameState};
use ducy::games::omaha_hilo::{OmahaHiLoGameEvaluation, OmahaHiLoGameState};
use ducy::games::{GameEquityEvaluation, GameEvaluation};

fn to_js_err(e: ducy::error::DucyError) -> JsError {
    JsError::new(&e.to_string())
}

#[wasm_bindgen]
pub struct HoldemGame {
    state: HoldemGameState,
    eval: HoldemGameEvaluation,
}

impl Default for HoldemGame {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl HoldemGame {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            state: HoldemGameState::new(),
            eval: HoldemGameEvaluation {},
        }
    }

    pub fn add_player(&mut self, cards: &str) -> Result<(), JsError> {
        let deck = Deck::parse(cards).map_err(to_js_err)?;
        self.state.add_player(deck).map_err(to_js_err)
    }

    pub fn set_flop(&mut self, cards: &str) -> Result<(), JsError> {
        let deck = Deck::parse(cards).map_err(to_js_err)?;
        self.state.set_flop(deck).map_err(to_js_err)
    }

    pub fn set_turn(&mut self, card: &str) -> Result<(), JsError> {
        let c = Card::parse(card).map_err(to_js_err)?;
        self.state.set_turn(c).map_err(to_js_err)
    }

    pub fn set_river(&mut self, card: &str) -> Result<(), JsError> {
        let c = Card::parse(card).map_err(to_js_err)?;
        self.state.set_river(c).map_err(to_js_err)
    }

    pub fn evaluate_equity(&self) -> Vec<f64> {
        self.eval
            .evaluate_equity(&self.state)
            .into_iter()
            .map(|d| d.try_into().unwrap_or(0.0))
            .collect()
    }

    pub fn evaluate_winners(&self) -> Result<JsValue, JsError> {
        let winners = self.eval.evaluate_winners(&self.state);
        let results: Vec<WinnerResult> = winners
            .into_iter()
            .map(|w| WinnerResult {
                player_index: w.player_index(),
                pot_share: w.pot_amount().try_into().unwrap_or(0.0),
                hand: w.winning_hand().to_string(),
            })
            .collect();
        serde_wasm_bindgen::to_value(&results).map_err(|e| JsError::new(&e.to_string()))
    }
}

#[derive(serde::Serialize)]
struct WinnerResult {
    player_index: usize,
    pot_share: f64,
    hand: String,
}

#[wasm_bindgen]
pub struct OmahaGame {
    state: OmahaGameState,
    eval: OmahaGameEvaluation,
}

#[wasm_bindgen]
impl OmahaGame {
    #[wasm_bindgen(constructor)]
    pub fn new(cards_per_player: u32) -> Self {
        Self {
            state: OmahaGameState::new(cards_per_player),
            eval: OmahaGameEvaluation {},
        }
    }

    pub fn add_player(&mut self, cards: &str) -> Result<(), JsError> {
        let deck = Deck::parse(cards).map_err(to_js_err)?;
        self.state.add_player(deck).map_err(to_js_err)
    }

    pub fn set_flop(&mut self, cards: &str) -> Result<(), JsError> {
        let deck = Deck::parse(cards).map_err(to_js_err)?;
        self.state.set_flop(deck).map_err(to_js_err)
    }

    pub fn set_turn(&mut self, card: &str) -> Result<(), JsError> {
        let c = Card::parse(card).map_err(to_js_err)?;
        self.state.set_turn(c).map_err(to_js_err)
    }

    pub fn set_river(&mut self, card: &str) -> Result<(), JsError> {
        let c = Card::parse(card).map_err(to_js_err)?;
        self.state.set_river(c).map_err(to_js_err)
    }

    pub fn evaluate_equity(&self) -> Vec<f64> {
        self.eval
            .evaluate_equity(&self.state)
            .into_iter()
            .map(|d| d.try_into().unwrap_or(0.0))
            .collect()
    }

    pub fn evaluate_winners(&self) -> Result<JsValue, JsError> {
        let winners = self.eval.evaluate_winners(&self.state);
        let results: Vec<WinnerResult> = winners
            .into_iter()
            .map(|w| WinnerResult {
                player_index: w.player_index(),
                pot_share: w.pot_amount().try_into().unwrap_or(0.0),
                hand: w.winning_hand().to_string(),
            })
            .collect();
        serde_wasm_bindgen::to_value(&results).map_err(|e| JsError::new(&e.to_string()))
    }
}

#[wasm_bindgen]
pub struct OmahaHiLoGame {
    state: OmahaHiLoGameState,
    eval: OmahaHiLoGameEvaluation,
}

impl Default for OmahaHiLoGame {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl OmahaHiLoGame {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            state: OmahaHiLoGameState::new(),
            eval: OmahaHiLoGameEvaluation {},
        }
    }

    pub fn add_player(&mut self, cards: &str) -> Result<(), JsError> {
        let deck = Deck::parse(cards).map_err(to_js_err)?;
        self.state.add_player(deck).map_err(to_js_err)
    }

    pub fn set_flop(&mut self, cards: &str) -> Result<(), JsError> {
        let deck = Deck::parse(cards).map_err(to_js_err)?;
        self.state.set_flop(deck).map_err(to_js_err)
    }

    pub fn set_turn(&mut self, card: &str) -> Result<(), JsError> {
        let c = Card::parse(card).map_err(to_js_err)?;
        self.state.set_turn(c).map_err(to_js_err)
    }

    pub fn set_river(&mut self, card: &str) -> Result<(), JsError> {
        let c = Card::parse(card).map_err(to_js_err)?;
        self.state.set_river(c).map_err(to_js_err)
    }

    pub fn evaluate_equity(&self) -> Vec<f64> {
        self.eval
            .evaluate_equity(&self.state)
            .into_iter()
            .map(|d| d.try_into().unwrap_or(0.0))
            .collect()
    }
}

#[wasm_bindgen]
pub struct RandomDeck {
    deck: Deck,
}

impl Default for RandomDeck {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl RandomDeck {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            deck: Deck::all_cards(),
        }
    }

    pub fn remove(&mut self, cards: &str) -> Result<(), JsError> {
        let to_remove = Deck::parse(cards).map_err(to_js_err)?;
        if !self.deck.has_cards(&to_remove) {
            return Err(JsError::new("cards not available in deck"));
        }
        self.deck -= to_remove;
        Ok(())
    }

    pub fn deal(&mut self, count: u32) -> Result<String, JsError> {
        let cards = self
            .deck
            .try_remove_random_cards(count)
            .map_err(to_js_err)?;
        Ok(cards
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(" "))
    }

    pub fn remaining(&self) -> u32 {
        self.deck.num_cards()
    }
}
