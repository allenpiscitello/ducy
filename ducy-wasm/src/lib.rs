use wasm_bindgen::prelude::*;

use ducy::deck::{Card, Deck};
use ducy::games::flop_game::FlopGame;
use ducy::games::holdem::{HoldemGameEvaluation, HoldemGameState, HoldemRange};
use ducy::games::omaha::{OmahaGameEvaluation, OmahaGameState};
use ducy::games::omaha_bomb_pot::{OmahaBombPotGameEvaluation, OmahaBombPotGameState};
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

    pub fn add_dead_cards(&mut self, cards: &str) -> Result<(), JsError> {
        let deck = Deck::parse(cards).map_err(to_js_err)?;
        self.state.add_dead_cards(deck).map_err(to_js_err)
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

fn parse_ranges(ranges: &[String]) -> Result<Vec<HoldemRange>, JsError> {
    ranges
        .iter()
        .map(|r| HoldemRange::parse(r).map_err(to_js_err))
        .collect()
}

#[derive(serde::Serialize)]
struct RangeSampleResult {
    samples: u64,
    equity_sum: Vec<f64>,
    equity_sq_sum: Vec<f64>,
}

/// Range-vs-range equity. Players added with `add_player` come first, then one
/// player per entry in `ranges` (e.g. `"QQ+, AKs"`).
#[wasm_bindgen]
impl HoldemGame {
    /// Exact equity over every combo combination and runout.
    pub fn range_equity(&self, ranges: Vec<String>) -> Result<Vec<f64>, JsError> {
        let ranges = parse_ranges(&ranges)?;
        Ok(self
            .eval
            .range_equity(&self.state, &ranges)
            .map_err(to_js_err)?
            .into_iter()
            .map(|d| d.try_into().unwrap_or(0.0))
            .collect())
    }

    /// Upper bound on hand evaluations `range_equity` would run, to decide
    /// between exact and sampled.
    pub fn exact_range_evaluations(&self, ranges: Vec<String>) -> Result<f64, JsError> {
        let ranges = parse_ranges(&ranges)?;
        self.eval
            .exact_range_evaluations(&self.state, &ranges)
            .map(|n| n as f64)
            .map_err(to_js_err)
    }

    /// Runs `samples` Monte Carlo deals. Returns sums over samples
    /// (`samples`, `equity_sum`, `equity_sq_sum`) so batches can be added.
    pub fn sample_range_equity(
        &self,
        ranges: Vec<String>,
        samples: usize,
    ) -> Result<JsValue, JsError> {
        let ranges = parse_ranges(&ranges)?;
        let r = self
            .eval
            .sample_range_equity(&self.state, &ranges, samples)
            .map_err(to_js_err)?;
        let result = RangeSampleResult {
            samples: r.samples,
            equity_sum: r.equity_sum,
            equity_sq_sum: r.equity_sq_sum,
        };
        serde_wasm_bindgen::to_value(&result).map_err(|e| JsError::new(&e.to_string()))
    }
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

    pub fn add_dead_cards(&mut self, cards: &str) -> Result<(), JsError> {
        let deck = Deck::parse(cards).map_err(to_js_err)?;
        self.state.add_dead_cards(deck).map_err(to_js_err)
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

    /// Average equity over `samples` random runouts.
    pub fn sample_equity(&self, samples: usize) -> Vec<f64> {
        self.eval
            .sample_equity(&self.state, samples)
            .into_iter()
            .map(|d| d.try_into().unwrap_or(0.0))
            .collect()
    }
}

#[wasm_bindgen]
pub struct OmahaHiLoGame {
    state: OmahaHiLoGameState,
    eval: OmahaHiLoGameEvaluation,
}

#[wasm_bindgen]
impl OmahaHiLoGame {
    #[wasm_bindgen(constructor)]
    pub fn new(cards_per_player: u32) -> Self {
        Self {
            state: OmahaHiLoGameState::new(cards_per_player),
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

    pub fn add_dead_cards(&mut self, cards: &str) -> Result<(), JsError> {
        let deck = Deck::parse(cards).map_err(to_js_err)?;
        self.state.add_dead_cards(deck).map_err(to_js_err)
    }

    pub fn evaluate_equity(&self) -> Vec<f64> {
        self.eval
            .evaluate_equity(&self.state)
            .into_iter()
            .map(|d| d.try_into().unwrap_or(0.0))
            .collect()
    }

    /// Average equity over `samples` random runouts.
    pub fn sample_equity(&self, samples: usize) -> Vec<f64> {
        self.eval
            .sample_equity(&self.state, samples)
            .into_iter()
            .map(|d| d.try_into().unwrap_or(0.0))
            .collect()
    }
}

#[derive(serde::Serialize)]
struct BombPotSampleResult {
    samples: u64,
    equity_sum: Vec<f64>,
    board_wins: Vec<Vec<u64>>,
    scoops: Vec<u64>,
    scooped: Vec<u64>,
}

#[wasm_bindgen]
pub struct OmahaBombPotGame {
    state: OmahaBombPotGameState,
    eval: OmahaBombPotGameEvaluation,
}

#[wasm_bindgen]
impl OmahaBombPotGame {
    #[wasm_bindgen(constructor)]
    pub fn new(num_boards: usize, cards_per_player: u32) -> Self {
        Self {
            state: OmahaBombPotGameState::new(num_boards, cards_per_player),
            eval: OmahaBombPotGameEvaluation {},
        }
    }

    pub fn add_player(&mut self, cards: &str) -> Result<(), JsError> {
        let deck = Deck::parse(cards).map_err(to_js_err)?;
        self.state.add_player(deck).map_err(to_js_err)
    }

    pub fn set_flop(&mut self, board_index: usize, cards: &str) -> Result<(), JsError> {
        let deck = Deck::parse(cards).map_err(to_js_err)?;
        self.state.set_flop(board_index, deck).map_err(to_js_err)
    }

    pub fn set_turn(&mut self, board_index: usize, card: &str) -> Result<(), JsError> {
        let c = Card::parse(card).map_err(to_js_err)?;
        self.state.set_turn(board_index, c).map_err(to_js_err)
    }

    pub fn set_river(&mut self, board_index: usize, card: &str) -> Result<(), JsError> {
        let c = Card::parse(card).map_err(to_js_err)?;
        self.state.set_river(board_index, c).map_err(to_js_err)
    }

    pub fn add_dead_cards(&mut self, cards: &str) -> Result<(), JsError> {
        let deck = Deck::parse(cards).map_err(to_js_err)?;
        self.state.add_dead_cards(deck).map_err(to_js_err)
    }

    pub fn evaluate_equity(&self) -> Vec<f64> {
        self.eval
            .evaluate_equity(&self.state)
            .into_iter()
            .map(|d| d.try_into().unwrap_or(0.0))
            .collect()
    }

    /// Runs `samples` Monte Carlo deals. `random_seats` are the final player
    /// positions dealt random hands; added players fill the rest in order.
    /// Returns sums over samples, so batches can be added together.
    pub fn sample(&self, samples: usize, random_seats: Vec<usize>) -> Result<JsValue, JsError> {
        let r = self
            .eval
            .sample(&self.state, &random_seats, samples)
            .map_err(to_js_err)?;
        let result = BombPotSampleResult {
            samples: r.samples,
            equity_sum: r.equity_sum,
            board_wins: r.board_wins,
            scoops: r.scoops,
            scooped: r.scooped,
        };
        serde_wasm_bindgen::to_value(&result).map_err(|e| JsError::new(&e.to_string()))
    }

    pub fn evaluate_winners(&self) -> Result<JsValue, JsError> {
        let result = self.eval.evaluate_winners(&self.state);
        let board_results: Vec<Vec<WinnerResult>> = result
            .board_winners
            .into_iter()
            .map(|winners| {
                winners
                    .into_iter()
                    .map(|w| WinnerResult {
                        player_index: w.player_index(),
                        pot_share: w.pot_amount().try_into().unwrap_or(0.0),
                        hand: w.winning_hand().to_string(),
                    })
                    .collect()
            })
            .collect();
        serde_wasm_bindgen::to_value(&board_results).map_err(|e| JsError::new(&e.to_string()))
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

#[derive(serde::Serialize)]
struct EquityChunkResult {
    runouts: u64,
    equity_sum: Vec<f64>,
}

/// Exact equity split into chunks of runouts, e.g. one chunk per Web Worker.
/// Sum `equity_sum` and `runouts` across all chunks, then divide.
macro_rules! equity_chunk_bindings {
    ($game:ty) => {
        #[wasm_bindgen]
        impl $game {
            /// Number of runouts exact equity enumerates.
            pub fn runout_count(&self) -> f64 {
                self.eval.runout_count(&self.state) as f64
            }

            /// Equity totals for runouts `start..start + count`
            /// (`{ runouts, equity_sum }`).
            pub fn equity_chunk(&self, start: f64, count: f64) -> Result<JsValue, JsError> {
                let r = self
                    .eval
                    .evaluate_equity_chunk(&self.state, start as u64, count as u64);
                let result = EquityChunkResult {
                    runouts: r.runouts,
                    equity_sum: r.equity_sum(),
                };
                serde_wasm_bindgen::to_value(&result).map_err(|e| JsError::new(&e.to_string()))
            }
        }
    };
}

equity_chunk_bindings!(HoldemGame);
equity_chunk_bindings!(OmahaGame);
equity_chunk_bindings!(OmahaHiLoGame);
