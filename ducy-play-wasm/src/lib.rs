//! WebAssembly bindings for `ducy-play`: play a hand of Hold'em or Omaha
//! step by step from JavaScript. See [`PokerHand`].

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use ducy::deck::{Card, Deck};
use ducy_play::{
    Action, BettingStructure, Deal, Event, Hand, PlayError, RaiseRange, Street, TableRules, Variant,
};

fn play_err(e: PlayError) -> JsError {
    JsError::new(&e.to_string())
}

fn to_js<T: Serialize>(value: &T) -> Result<JsValue, JsError> {
    serde_wasm_bindgen::to_value(value).map_err(|e| JsError::new(&e.to_string()))
}

/// Setup for [`PokerHand`], passed as a plain JS object.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HandConfig {
    /// `"holdem"` or `"omaha"`.
    game: String,
    /// Omaha hole cards: 4 (default), 5 or 6.
    #[serde(default)]
    hole_cards: Option<u32>,
    /// `"no_limit"` or `"pot_limit"`. Defaults to no-limit for Hold'em and
    /// pot-limit for Omaha.
    #[serde(default)]
    betting: Option<String>,
    small_blind: u64,
    big_blind: u64,
    #[serde(default)]
    ante: u64,
    /// One stack per seat, in seat order.
    stacks: Vec<u64>,
    /// The button's seat.
    button: usize,
    /// Seed for the shuffle, for reproducible hands. Ignored with `cards`.
    #[serde(default)]
    seed: Option<u64>,
    /// Exact hole cards per seat, e.g. `["As Ah", "Kd Kh"]`. Requires `board`.
    #[serde(default)]
    cards: Option<Vec<String>>,
    /// Exact five-card board, e.g. `"2c 7d 9h Jc 3s"`. Requires `cards`.
    #[serde(default)]
    board: Option<String>,
}

impl HandConfig {
    fn rules(&self) -> Result<TableRules, JsError> {
        let variant = match self.game.as_str() {
            "holdem" => Variant::Holdem,
            "omaha" => Variant::Omaha {
                hole_cards: self.hole_cards.unwrap_or(4),
            },
            other => return Err(JsError::new(&format!("unknown game \"{other}\""))),
        };
        let structure = match self.betting.as_deref() {
            Some("no_limit") => BettingStructure::NoLimit,
            Some("pot_limit") => BettingStructure::PotLimit,
            None if variant == Variant::Holdem => BettingStructure::NoLimit,
            None => BettingStructure::PotLimit,
            Some(other) => return Err(JsError::new(&format!("unknown betting \"{other}\""))),
        };
        Ok(TableRules {
            variant,
            structure,
            small_blind: self.small_blind,
            big_blind: self.big_blind,
            ante: self.ante,
        })
    }

    fn deal(&self, variant: Variant) -> Result<Deal, JsError> {
        match (&self.cards, &self.board) {
            (None, None) => Deal::random(variant, self.stacks.len(), self.seed).map_err(play_err),
            (Some(cards), Some(board)) => {
                let hole_cards = cards
                    .iter()
                    .map(|c| Deck::parse(c).map_err(|e| JsError::new(&e.to_string())))
                    .collect::<Result<Vec<_>, _>>()?;
                let board: Vec<Card> = board
                    .split_whitespace()
                    .map(|c| Card::parse(c).map_err(|e| JsError::new(&e.to_string())))
                    .collect::<Result<_, _>>()?;
                let board: [Card; 5] = board
                    .try_into()
                    .map_err(|_| JsError::new("board needs exactly 5 cards"))?;
                Deal::new(variant, hole_cards, board).map_err(play_err)
            }
            _ => Err(JsError::new("cards and board must be given together")),
        }
    }
}

#[derive(Serialize)]
struct RangeJs {
    min_to: u64,
    max_to: u64,
}

impl From<RaiseRange> for RangeJs {
    fn from(r: RaiseRange) -> Self {
        Self {
            min_to: r.min_to,
            max_to: r.max_to,
        }
    }
}

#[derive(Serialize)]
struct LegalJs {
    seat: usize,
    can_fold: bool,
    can_check: bool,
    call: Option<u64>,
    bet: Option<RangeJs>,
    raise: Option<RangeJs>,
}

#[derive(Serialize)]
struct SeatJs {
    stack: u64,
    street_bet: u64,
    contributed: u64,
    folded: bool,
    all_in: bool,
}

#[derive(Serialize)]
struct StateJs {
    street: &'static str,
    board: Vec<String>,
    pot: u64,
    current_bet: u64,
    button: usize,
    to_act: Option<usize>,
    complete: bool,
    seats: Vec<SeatJs>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum EventJs {
    Ante {
        seat: usize,
        amount: u64,
    },
    SmallBlind {
        seat: usize,
        amount: u64,
    },
    BigBlind {
        seat: usize,
        amount: u64,
    },
    Fold {
        seat: usize,
    },
    Check {
        seat: usize,
    },
    Call {
        seat: usize,
        amount: u64,
        all_in: bool,
    },
    Bet {
        seat: usize,
        to: u64,
        all_in: bool,
    },
    Raise {
        seat: usize,
        to: u64,
        all_in: bool,
    },
    Board {
        street: &'static str,
        cards: Vec<String>,
    },
    Award {
        seat: usize,
        pot: usize,
        amount: u64,
    },
}

impl From<&Event> for EventJs {
    fn from(e: &Event) -> Self {
        match e {
            Event::Ante { seat, amount } => Self::Ante {
                seat: *seat,
                amount: *amount,
            },
            Event::SmallBlind { seat, amount } => Self::SmallBlind {
                seat: *seat,
                amount: *amount,
            },
            Event::BigBlind { seat, amount } => Self::BigBlind {
                seat: *seat,
                amount: *amount,
            },
            Event::Fold { seat } => Self::Fold { seat: *seat },
            Event::Check { seat } => Self::Check { seat: *seat },
            Event::Call {
                seat,
                amount,
                all_in,
            } => Self::Call {
                seat: *seat,
                amount: *amount,
                all_in: *all_in,
            },
            Event::Bet { seat, to, all_in } => Self::Bet {
                seat: *seat,
                to: *to,
                all_in: *all_in,
            },
            Event::Raise { seat, to, all_in } => Self::Raise {
                seat: *seat,
                to: *to,
                all_in: *all_in,
            },
            Event::Board { street, cards } => Self::Board {
                street: street_name(*street),
                cards: cards.iter().map(|c| c.to_string()).collect(),
            },
            Event::Award { seat, pot, amount } => Self::Award {
                seat: *seat,
                pot: *pot,
                amount: *amount,
            },
        }
    }
}

#[derive(Serialize)]
struct AwardJs {
    seat: usize,
    amount: u64,
}

#[derive(Serialize)]
struct PotJs {
    amount: u64,
    eligible: Vec<usize>,
    awards: Vec<AwardJs>,
    winning_hand: Option<String>,
}

#[derive(Serialize)]
struct ResultJs {
    showdown: bool,
    pots: Vec<PotJs>,
    payouts: Vec<u64>,
    final_stacks: Vec<u64>,
    net: Vec<i64>,
}

fn street_name(street: Street) -> &'static str {
    match street {
        Street::Preflop => "preflop",
        Street::Flop => "flop",
        Street::Turn => "turn",
        Street::River => "river",
    }
}

/// Converts a JS number to a whole number of chips.
fn chips(amount: Option<f64>) -> Result<u64, JsError> {
    match amount {
        Some(a)
            if a.is_finite() && a >= 0.0 && a.fract() == 0.0 && a <= 9_007_199_254_740_991.0 =>
        {
            Ok(a as u64)
        }
        Some(_) => Err(JsError::new("amount must be a whole number of chips")),
        None => Err(JsError::new("bet and raise need an amount")),
    }
}

/// One hand of Hold'em or Omaha, played action by action.
///
/// ```js
/// const hand = new PokerHand({
///   game: "holdem", small_blind: 1, big_blind: 2,
///   stacks: [200, 200, 200], button: 0, seed: 42,
/// });
/// while (!hand.is_complete()) {
///   const legal = hand.legal_actions();
///   hand.act(legal.can_check ? "check" : "call");
/// }
/// console.log(hand.result());
/// ```
///
/// Config fields: `game` (`"holdem"` | `"omaha"`), `hole_cards` (Omaha: 4, 5
/// or 6), `betting` (`"no_limit"` | `"pot_limit"`, defaulting to no-limit
/// Hold'em and pot-limit Omaha), `small_blind`, `big_blind`, `ante`,
/// `stacks`, `button`, and either `seed` (shuffled deal) or `cards` +
/// `board` (exact deal).
#[wasm_bindgen]
pub struct PokerHand {
    hand: Hand,
}

#[wasm_bindgen]
impl PokerHand {
    #[wasm_bindgen(constructor)]
    pub fn new(config: JsValue) -> Result<PokerHand, JsError> {
        let config: HandConfig =
            serde_wasm_bindgen::from_value(config).map_err(|e| JsError::new(&e.to_string()))?;
        let rules = config.rules()?;
        let deal = config.deal(rules.variant)?;
        let hand = Hand::new(rules, &config.stacks, config.button, deal).map_err(play_err)?;
        Ok(Self { hand })
    }

    /// Seat whose turn it is, or `undefined` once the hand is over.
    pub fn to_act(&self) -> Option<usize> {
        self.hand.to_act()
    }

    /// Whether the hand is over.
    pub fn is_complete(&self) -> bool {
        self.hand.is_complete()
    }

    /// `{ seat, can_fold, can_check, call?, bet?: { min_to, max_to },
    /// raise?: { min_to, max_to } }`, or `null` once the hand is over.
    /// Bet and raise amounts are totals for the street.
    pub fn legal_actions(&self) -> Result<JsValue, JsError> {
        let Some(l) = self.hand.legal_actions() else {
            return Ok(JsValue::NULL);
        };
        to_js(&LegalJs {
            seat: l.seat,
            can_fold: l.can_fold,
            can_check: l.can_check,
            call: l.call,
            bet: l.bet.map(Into::into),
            raise: l.raise.map(Into::into),
        })
    }

    /// Plays an action for the seat to act: `"fold"`, `"check"`, `"call"`,
    /// `"bet"` or `"raise"` (with `amount`, the street total), or `"all_in"`.
    pub fn act(&mut self, action: &str, amount: Option<f64>) -> Result<(), JsError> {
        let action = match action {
            "fold" => Action::Fold,
            "check" => Action::Check,
            "call" => Action::Call,
            "bet" => Action::Bet(chips(amount)?),
            "raise" => Action::Raise(chips(amount)?),
            "all_in" => Action::AllIn,
            other => return Err(JsError::new(&format!("unknown action \"{other}\""))),
        };
        self.hand.act(action).map_err(play_err)
    }

    /// The table right now: `{ street, board, pot, current_bet, button,
    /// to_act, complete, seats: [{ stack, street_bet, contributed, folded,
    /// all_in }] }`.
    pub fn state(&self) -> Result<JsValue, JsError> {
        let seats = (0..self.hand.num_seats())
            .map(|s| SeatJs {
                stack: self.hand.stack(s),
                street_bet: self.hand.street_bet(s),
                contributed: self.hand.contributed(s),
                folded: self.hand.has_folded(s),
                all_in: self.hand.is_all_in(s),
            })
            .collect();
        to_js(&StateJs {
            street: street_name(self.hand.street()),
            board: self.hand.board().iter().map(|c| c.to_string()).collect(),
            pot: self.hand.pot(),
            current_bet: self.hand.current_bet(),
            button: self.hand.button(),
            to_act: self.hand.to_act(),
            complete: self.hand.is_complete(),
            seats,
        })
    }

    /// A seat's hole cards, e.g. `"As Ah"`. The app decides who may see them.
    pub fn hole_cards(&self, seat: usize) -> Result<String, JsError> {
        self.hand
            .deal()
            .hole_cards()
            .get(seat)
            .map(|d| d.to_string())
            .ok_or_else(|| JsError::new("no such seat"))
    }

    /// Everything that has happened, in order, as objects with a `type` of
    /// `ante`, `small_blind`, `big_blind`, `fold`, `check`, `call`, `bet`,
    /// `raise`, `board` or `award`.
    pub fn events(&self) -> Result<JsValue, JsError> {
        let events: Vec<EventJs> = self.hand.events().iter().map(Into::into).collect();
        to_js(&events)
    }

    /// `{ showdown, pots: [{ amount, eligible, awards: [{ seat, amount }],
    /// winning_hand? }], payouts, final_stacks, net }` once the hand is over,
    /// otherwise `null`.
    pub fn result(&self) -> Result<JsValue, JsError> {
        let Some(r) = self.hand.result() else {
            return Ok(JsValue::NULL);
        };
        let pots = r
            .pots
            .iter()
            .map(|p| PotJs {
                amount: p.amount,
                eligible: p.eligible.clone(),
                awards: p
                    .awards
                    .iter()
                    .map(|a| AwardJs {
                        seat: a.seat,
                        amount: a.amount,
                    })
                    .collect(),
                winning_hand: p.winning_hand.clone(),
            })
            .collect();
        to_js(&ResultJs {
            showdown: r.showdown,
            pots,
            payouts: r.payouts.clone(),
            final_stacks: r.final_stacks.clone(),
            net: r.net.clone(),
        })
    }
}
