use wasm_bindgen::prelude::*;

mod play;
pub use play::{BotTable, MultiTable};

use ducy::deck::{Card, Deck};
use ducy::games::flop_game::FlopGame;
use ducy::games::holdem::{HoldemGameEvaluation, HoldemGameState, HoldemRange};
use ducy::games::omaha::{OmahaGameEvaluation, OmahaGameState};
use ducy::games::omaha_bomb_pot::{OmahaBombPotGameEvaluation, OmahaBombPotGameState, SeatHand};
use ducy::games::omaha_hilo::{OmahaHiLoGameEvaluation, OmahaHiLoGameState};
use ducy::games::{GameEquityEvaluation, GameEvaluation};
use ducy::games::{omaha_range, omaha_range_equity};

fn to_js_err(e: ducy::error::DucyError) -> JsError {
    JsError::new(&e.to_string())
}

/// What range text covers, for a range picker: `{ok: true, classes,
/// combos}`, or `{ok: false, error: {term, start, message}}` naming the term
/// that didn't parse and its byte offset.
#[derive(serde::Serialize)]
#[serde(untagged)]
enum RangeAnswer<T> {
    Ok {
        ok: bool,
        #[serde(flatten)]
        report: T,
    },
    Err {
        ok: bool,
        error: ducy::games::holdem::RangeTextError,
    },
}

impl<T> RangeAnswer<T> {
    fn from(r: Result<T, ducy::games::holdem::RangeTextError>) -> Self {
        match r {
            Ok(report) => RangeAnswer::Ok { ok: true, report },
            Err(error) => RangeAnswer::Err { ok: false, error },
        }
    }
}

/// Hold'em range text (`QQ+, AKs, A5s-A2s, KQo, AhKh`, ...) as ducy reads
/// it: the hand classes it covers (strongest first) and its combos, leaving
/// out combos that use a card in `dead` (e.g. "As Kd"). See
/// `ducy::games::holdem::HoldemRange` for the notation.
#[wasm_bindgen(js_name = holdemRangeReport)]
pub fn holdem_range_report(text: &str, dead: Option<String>) -> Result<JsValue, JsError> {
    let dead = match dead.as_deref().map(str::trim) {
        Some(d) if !d.is_empty() => Deck::parse(d).map_err(to_js_err)?,
        _ => Deck::empty(),
    };
    plain_js(&RangeAnswer::from(HoldemRange::report(text, dead)))
}

/// A value as plain JS objects (not Maps), as JSON would give it.
fn plain_js(value: &impl serde::Serialize) -> Result<JsValue, JsError> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|e| JsError::new(&e.to_string()))
}

fn hand_cards(cards: &str) -> Result<Vec<Card>, JsError> {
    cards
        .split([',', ' '])
        .filter(|c| !c.is_empty())
        .map(|c| Card::parse(c).map_err(to_js_err))
        .collect()
}

#[derive(serde::Serialize)]
struct OmahaHandInfo {
    /// Feature values, in `features` order.
    values: Vec<f64>,
    features: Vec<&'static str>,
    tags: Vec<&'static str>,
    nut_flush_block: f64,
    score: Option<f64>,
    /// 0 (best) to 100 (worst) among hands of the same size.
    percentile: Option<f64>,
}

/// Omaha starting-hand analysis for 4-6 cards like "As Ad Kh Qh"
/// (`ducy::games::omaha_analysis`): feature values and names, tags,
/// nut-flush blocking, and the fitted model's score and percentile (0 best).
#[wasm_bindgen(js_name = omahaHand)]
pub fn omaha_hand(cards: &str) -> Result<JsValue, JsError> {
    use ducy::games::omaha_analysis as oa;
    let hand = hand_cards(cards)?;
    if !(4..=6).contains(&hand.len()) {
        return Err(JsError::new("an Omaha hand has 4, 5 or 6 cards"));
    }
    let info = OmahaHandInfo {
        values: oa::hand_features(&hand),
        features: oa::FEATURE_NAMES.to_vec(),
        tags: oa::hand_tags(&hand),
        nut_flush_block: oa::nut_flush_block(&hand),
        score: oa::score(&hand),
        percentile: oa::percentile(&hand),
    };
    serde_wasm_bindgen::to_value(&info).map_err(|e| JsError::new(&e.to_string()))
}

/// Each Omaha hand tag and its description ("you play too many ___").
#[wasm_bindgen(js_name = omahaTagLabels)]
pub fn omaha_tag_labels() -> Result<JsValue, JsError> {
    use serde::Serialize;
    let labels: std::collections::BTreeMap<&str, &str> = ducy::games::omaha_analysis::TAG_LABELS
        .iter()
        .copied()
        .collect();
    labels
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|e| JsError::new(&e.to_string()))
}

#[derive(serde::Serialize)]
struct OmahaRangeReport {
    combos: f64,
    total: f64,
    coverage: f64,
}

/// Omaha range text (`AAxx$ds, $rd0-1`, ...) for hands of `cards_per_player`
/// cards: `{ok: true, combos, total, coverage}`, or the term that didn't parse
/// as for `holdemRangeReport`.
#[wasm_bindgen(js_name = omahaRangeReport)]
pub fn omaha_range_report(text: &str, cards_per_player: usize) -> Result<JsValue, JsError> {
    let mut range = omaha_range::OmahaRange::new(cards_per_player);
    let mut answer = Ok(());
    for (start, term) in range_terms(text) {
        if range.add(term, rust_decimal::Decimal::ONE).is_err() {
            answer = Err(ducy::games::holdem::RangeTextError {
                term: term.to_string(),
                start,
                message: format!(
                    "“{term}” isn’t an Omaha range term (e.g. AAxx$ds, $rd0-1, KK$ss)."
                ),
            });
            break;
        }
    }
    let answer = answer.map(|()| OmahaRangeReport {
        combos: range.combos() as f64,
        total: range.total_hands() as f64,
        coverage: range.coverage(),
    });
    plain_js(&RangeAnswer::from(answer))
}

/// The terms of range text (separated by commas or whitespace), each with its byte offset.
fn range_terms(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, ch) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ',')))
    {
        if matches!(ch, ',' | ' ' | '\t' | '\n' | '\r') {
            if let Some(s) = start.take() {
                out.push((s, &text[s..i]));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    out
}

/// JS numbers are f64; seeds are whole numbers up to 2^53.
fn to_seed(seed: Option<f64>) -> Option<u64> {
    seed.map(|s| s as u64)
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
    /// Passing a `seed` makes the result reproducible.
    pub fn sample_range_equity(
        &self,
        ranges: Vec<String>,
        samples: usize,
        seed: Option<f64>,
    ) -> Result<JsValue, JsError> {
        let ranges = parse_ranges(&ranges)?;
        let r = self
            .eval
            .sample_range_equity_seeded(&self.state, &ranges, samples, to_seed(seed))
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

    /// Average equity over `samples` random runouts. Passing a `seed` makes
    /// the result reproducible.
    pub fn sample_equity(&self, samples: usize, seed: Option<f64>) -> Vec<f64> {
        self.eval
            .sample_equity_seeded(&self.state, samples, to_seed(seed))
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

    /// Average equity over `samples` random runouts. Passing a `seed` makes
    /// the result reproducible.
    pub fn sample_equity(&self, samples: usize, seed: Option<f64>) -> Vec<f64> {
        self.eval
            .sample_equity_seeded(&self.state, samples, to_seed(seed))
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
    range_seats: Vec<(usize, omaha_range::OmahaRange)>,
}

#[wasm_bindgen]
impl OmahaBombPotGame {
    #[wasm_bindgen(constructor)]
    pub fn new(num_boards: usize, cards_per_player: u32) -> Self {
        Self {
            state: OmahaBombPotGameState::new(num_boards, cards_per_player),
            eval: OmahaBombPotGameEvaluation {},
            range_seats: Vec::new(),
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

    /// Deals seat `seat` (a position in the final player order) a hand
    /// from `range` in every later `sample` call, instead of a random hand,
    /// e.g. to sample against hands likely to see the flop. Replaces any
    /// range already set for that seat.
    pub fn set_seat_range(&mut self, seat: usize, range: &OmahaRange) {
        self.range_seats.retain(|(s, _)| *s != seat);
        self.range_seats.push((seat, range.range.clone()));
    }

    /// Removes every range set with `set_seat_range`.
    pub fn clear_seat_ranges(&mut self) {
        self.range_seats.clear();
    }

    /// Runs `samples` Monte Carlo deals. `random_seats` are the final player
    /// positions dealt random hands; added players fill the rest in order.
    /// Seats set with `set_seat_range` are dealt from their range.
    /// Returns sums over samples, so batches can be added together.
    /// Passing a `seed` makes the result reproducible.
    pub fn sample(
        &self,
        samples: usize,
        random_seats: Vec<usize>,
        seed: Option<f64>,
    ) -> Result<JsValue, JsError> {
        let seats: Vec<(usize, SeatHand)> = random_seats
            .iter()
            .map(|&seat| (seat, SeatHand::Random))
            .chain(
                self.range_seats
                    .iter()
                    .map(|(seat, range)| (*seat, SeatHand::Range(range))),
            )
            .collect();
        let r = self
            .eval
            .sample_seats_seeded(&self.state, &seats, samples, to_seed(seed))
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

fn winner_results<H: std::fmt::Display + ducy::ranking::hand_rank::HandRanking>(
    winners: Vec<ducy::games::GameWinner<H>>,
) -> Vec<WinnerResult> {
    winners
        .into_iter()
        .map(|w| WinnerResult {
            player_index: w.player_index(),
            pot_share: w.pot_amount().try_into().unwrap_or(0.0),
            hand: w.winning_hand().to_string(),
        })
        .collect()
}

/// Bindings for games where every player holds a complete hand and there are
/// no community cards (stud and draw games after the last card or draw).
macro_rules! dealt_game_bindings {
    ($js:ident, $state:ty, $eval:expr, $doc:literal) => {
        #[doc = $doc]
        #[wasm_bindgen]
        pub struct $js {
            state: $state,
        }

        impl Default for $js {
            fn default() -> Self {
                Self::new()
            }
        }

        #[wasm_bindgen]
        impl $js {
            #[wasm_bindgen(constructor)]
            pub fn new() -> Self {
                Self {
                    state: <$state>::new(),
                }
            }

            pub fn add_player(&mut self, cards: &str) -> Result<(), JsError> {
                let deck = Deck::parse(cards).map_err(to_js_err)?;
                self.state.add_player(deck).map_err(to_js_err)
            }

            /// Winners as `[{ player_index, pot_share, hand }]`.
            pub fn evaluate_winners(&self) -> Result<JsValue, JsError> {
                let results = winner_results($eval.evaluate_winners(&self.state));
                serde_wasm_bindgen::to_value(&results).map_err(|e| JsError::new(&e.to_string()))
            }
        }
    };
}

equity_chunk_bindings!(HoldemGame);
equity_chunk_bindings!(OmahaGame);
equity_chunk_bindings!(OmahaHiLoGame);

dealt_game_bindings!(
    RazzGame,
    ducy::games::razz::RazzGameState,
    ducy::games::razz::RazzGameEvaluation,
    "Razz: 7 cards per player, best ace-to-five low."
);
dealt_game_bindings!(
    BadugiGame,
    ducy::games::badugi::BadugiGameState,
    ducy::games::badugi::BadugiGameEvaluation,
    "Badugi: 4 cards per player."
);
dealt_game_bindings!(
    DeuceToSevenGame,
    ducy::games::deuce_to_seven::DeuceToSevenGameState,
    ducy::games::deuce_to_seven::DeuceToSevenGameEvaluation,
    "2-7 Triple Draw: 5 cards per player, deuce-to-seven low."
);
dealt_game_bindings!(
    StudGame,
    ducy::games::stud::StudGameState,
    ducy::games::stud::StudGameEvaluation,
    "Seven-Card Stud: 7 cards per player, best high hand."
);
dealt_game_bindings!(
    SingleDrawA5Game,
    ducy::games::single_draw_a5::SingleDrawA5GameState,
    ducy::games::single_draw_a5::SingleDrawA5GameEvaluation,
    "Single Draw A-5 Lowball: 5 cards per player."
);
dealt_game_bindings!(
    SingleDraw27Game,
    ducy::games::single_draw_27::SingleDraw27GameState,
    ducy::games::single_draw_27::SingleDraw27GameEvaluation,
    "Single Draw 2-7 Lowball: 5 cards per player."
);

#[derive(serde::Serialize)]
struct HiLoWinners {
    high: Vec<WinnerResult>,
    low: Vec<WinnerResult>,
}

/// Seven-Card Stud Hi-Lo 8-or-Better: 7 cards per player, split pot.
#[wasm_bindgen]
pub struct StudHiLoGame {
    state: ducy::games::stud_hilo::StudHiLoGameState,
}

impl Default for StudHiLoGame {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl StudHiLoGame {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            state: ducy::games::stud_hilo::StudHiLoGameState::new(),
        }
    }

    pub fn add_player(&mut self, cards: &str) -> Result<(), JsError> {
        let deck = Deck::parse(cards).map_err(to_js_err)?;
        self.state.add_player(deck).map_err(to_js_err)
    }

    /// `{ high: [...], low: [...] }`; `low` is empty when no low qualifies
    /// (high then scoops).
    pub fn evaluate_winners(&self) -> Result<JsValue, JsError> {
        let r = ducy::games::stud_hilo::StudHiLoGameEvaluation.evaluate_winners(&self.state);
        let result = HiLoWinners {
            high: winner_results(r.high_winners),
            low: winner_results(r.low_winners),
        };
        serde_wasm_bindgen::to_value(&result).map_err(|e| JsError::new(&e.to_string()))
    }
}

/// Omaha starting-hand range (PPT-style terms such as `AAxx$ds`, `$rd0-1`,
/// `TT$3rd`, `$ts`), for measuring what share of all starting hands it
/// covers. The full syntax is documented on `ducy::games::omaha_range::OmahaRange`.
#[wasm_bindgen]
pub struct OmahaRange {
    range: omaha_range::OmahaRange,
}

#[wasm_bindgen]
impl OmahaRange {
    /// An empty range for hands of `cards_per_player` cards (4 for PLO).
    #[wasm_bindgen(constructor)]
    pub fn new(cards_per_player: usize) -> Self {
        Self {
            range: omaha_range::OmahaRange::new(cards_per_player),
        }
    }

    /// Adds every hand matching `terms` (separated by commas or spaces).
    /// `weight` defaults to 1; a hand already in the range takes the new weight.
    pub fn add(&mut self, terms: &str, weight: Option<f64>) -> Result<(), JsError> {
        let weight = rust_decimal::Decimal::try_from(weight.unwrap_or(1.0))
            .map_err(|e| JsError::new(&e.to_string()))?;
        for term in terms
            .split([',', ' ', '\t', '\n'])
            .filter(|t| !t.is_empty())
        {
            self.range.add(term, weight).map_err(to_js_err)?;
        }
        Ok(())
    }

    /// Number of distinct hands in the range.
    pub fn combos(&self) -> f64 {
        self.range.combos() as f64
    }

    /// Number of possible starting hands (270,725 for 4 cards).
    pub fn total_hands(&self) -> f64 {
        self.range.total_hands() as f64
    }

    /// Share of all starting hands in the range, 0 to 1.
    pub fn coverage(&self) -> f64 {
        self.range.coverage()
    }

    /// Like `coverage`, counting each hand by its weight.
    pub fn weighted_coverage(&self) -> f64 {
        self.range.weighted_coverage()
    }
}

thread_local! {
    // Prepared Omaha ranges by (hole cards, text). Building one can take a
    // moment (an explicit hand list for narrow ranges), and the site samples
    // in many small batches, so each worker keeps recent ranges around.
    static OMAHA_RANGES: std::cell::RefCell<std::collections::HashMap<(usize, String), std::rc::Rc<omaha_range_equity::OmahaRangeSampler>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

const OMAHA_RANGE_CACHE: usize = 32;

fn omaha_ranges(
    ranges: &[String],
    cards: usize,
) -> Result<Vec<std::rc::Rc<omaha_range_equity::OmahaRangeSampler>>, JsError> {
    OMAHA_RANGES.with(|cache| {
        let mut cache = cache.borrow_mut();
        ranges
            .iter()
            .map(|text| {
                let key = (cards, text.clone());
                if let Some(r) = cache.get(&key) {
                    return Ok(r.clone());
                }
                let r = std::rc::Rc::new(
                    omaha_range_equity::OmahaRangeSampler::parse(text, cards).map_err(to_js_err)?,
                );
                if cache.len() >= OMAHA_RANGE_CACHE {
                    cache.clear();
                }
                cache.insert(key, r.clone());
                Ok(r)
            })
            .collect()
    })
}

fn range_result(r: ducy::games::holdem::RangeEquitySamples) -> Result<JsValue, JsError> {
    let result = RangeSampleResult {
        samples: r.samples,
        equity_sum: r.equity_sum,
        equity_sq_sum: r.equity_sq_sum,
    };
    serde_wasm_bindgen::to_value(&result).map_err(|e| JsError::new(&e.to_string()))
}

/// Omaha range equity. Players added with `add_player` come first, then one
/// player per entry in `ranges`, each in Omaha shorthand (e.g. `"AAxx$ds, $rd$ds"`).
#[wasm_bindgen]
impl OmahaGame {
    /// Runs `samples` Monte Carlo deals. Returns sums over samples
    /// (`samples`, `equity_sum`, `equity_sq_sum`) so batches can be added.
    pub fn sample_range_equity(
        &self,
        ranges: Vec<String>,
        samples: usize,
        seed: Option<f64>,
    ) -> Result<JsValue, JsError> {
        let rs = omaha_ranges(&ranges, self.state.cards_per_player())?;
        let refs: Vec<&omaha_range_equity::OmahaRangeSampler> =
            rs.iter().map(|r| r.as_ref()).collect();
        range_result(
            self.eval
                .sample_range_equity_seeded(&self.state, &refs, samples, to_seed(seed))
                .map_err(to_js_err)?,
        )
    }
}

/// Omaha Hi-Lo range equity; see `OmahaGame.sample_range_equity`.
#[wasm_bindgen]
impl OmahaHiLoGame {
    pub fn sample_range_equity(
        &self,
        ranges: Vec<String>,
        samples: usize,
        seed: Option<f64>,
    ) -> Result<JsValue, JsError> {
        let rs = omaha_ranges(&ranges, self.state.cards_per_player())?;
        let refs: Vec<&omaha_range_equity::OmahaRangeSampler> =
            rs.iter().map(|r| r.as_ref()).collect();
        range_result(
            self.eval
                .sample_range_equity_seeded(&self.state, &refs, samples, to_seed(seed))
                .map_err(to_js_err)?,
        )
    }
}

/// An Omaha shorthand range for checking what it covers, usable for any hole
/// card count including PLO6 (narrow ranges are counted exactly, broad ones
/// estimated by sampling).
#[wasm_bindgen]
pub struct OmahaRangeSampler {
    range: std::rc::Rc<omaha_range_equity::OmahaRangeSampler>,
}

#[wasm_bindgen]
impl OmahaRangeSampler {
    /// Parses `ranges` (terms separated by commas or spaces) for hands of
    /// `cards_per_player` cards. Errors on invalid syntax.
    #[wasm_bindgen(constructor)]
    pub fn new(cards_per_player: usize, ranges: &str) -> Result<OmahaRangeSampler, JsError> {
        let range = omaha_ranges(&[ranges.to_string()], cards_per_player)?.remove(0);
        Ok(Self { range })
    }

    /// Fraction of all starting hands in the range (0 to 1).
    pub fn coverage(&self) -> f64 {
        self.range.coverage().0
    }

    /// Whether `coverage` is exact (true) or a sampled estimate (false).
    pub fn coverage_exact(&self) -> bool {
        self.range.coverage().1
    }

    /// Whether a hand such as `"As Ah Ks Kh"` is in the range.
    pub fn contains(&self, hand: &str) -> Result<bool, JsError> {
        Ok(self.range.contains(Deck::parse(hand).map_err(to_js_err)?))
    }
}
