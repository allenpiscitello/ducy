//! A table where one person plays Hold'em against ducy-play's personality bots.
//!
//! The page drives it one action at a time: [`BotTable::advance`] makes the
//! next bot act, and [`BotTable::act`] applies the person's action, so the UI
//! can show each decision as it happens. [`BotTable::state`] returns what the
//! person may see: their own cards, public chip counts, and other players'
//! cards only once they're shown down.

use ducy::deck::{Card, Deck};
use ducy_play::{
    Action, Bot, Deal, Event, Hand, LegalActions, Personality, PersonalityBot, Pot, Street,
    TableRules,
};
use serde::Serialize;
use wasm_bindgen::prelude::*;

fn cards(deck: Deck) -> Vec<String> {
    deck.iter(true).map(|c| c.to_string()).collect()
}

fn card_strings(board: &[Card]) -> Vec<String> {
    board.iter().map(|c| c.to_string()).collect()
}

fn err(e: impl std::fmt::Debug) -> JsError {
    JsError::new(&format!("{e:?}"))
}

#[derive(Serialize)]
struct PersonalityInfo {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    catchphrase: &'static str,
}

/// Every bot personality, for the page's seat picker.
#[wasm_bindgen(js_name = botPersonalities)]
pub fn bot_personalities() -> Result<JsValue, JsError> {
    let list: Vec<PersonalityInfo> = Personality::ALL
        .iter()
        .map(|p| PersonalityInfo {
            id: p.id(),
            name: p.name(),
            description: p.description(),
            catchphrase: p.catchphrase(),
        })
        .collect();
    serde_wasm_bindgen::to_value(&list).map_err(err)
}

#[derive(Serialize)]
struct SeatState {
    name: String,
    /// Personality id, or "you" for the person's seat.
    id: String,
    stack: u64,
    street_bet: u64,
    folded: bool,
    all_in: bool,
    /// The person's own cards always; a bot's only once shown down.
    cards: Option<Vec<String>>,
    /// Chips won this hand, once it's over.
    won: u64,
    /// Net result this hand, once it's over.
    net: i64,
}

#[derive(Serialize)]
struct TableState {
    hand_number: u64,
    street: Option<Street>,
    board: Vec<String>,
    pot: u64,
    current_bet: u64,
    button: usize,
    hero: usize,
    big_blind: u64,
    /// The seat whose turn it is, if the hand is still going.
    to_act: Option<usize>,
    /// What the person may do, when it's their turn.
    legal: Option<LegalActions>,
    seats: Vec<SeatState>,
    events: Vec<Event>,
    complete: bool,
    showdown: bool,
    pots: Vec<Pot>,
}

/// A Hold'em table: the person in seat 0 and up to eight bots.
#[wasm_bindgen]
pub struct BotTable {
    rules: TableRules,
    names: Vec<String>,
    ids: Vec<String>,
    bots: Vec<Option<PersonalityBot>>,
    stacks: Vec<u64>,
    buy_in: u64,
    button: usize,
    hand: Option<Hand>,
    hand_number: u64,
    seed: u64,
}

#[wasm_bindgen]
impl BotTable {
    /// `bots` are personality ids (see `botPersonalities`), one per bot seat;
    /// the person sits in seat 0. Everyone starts with `buy_in` chips, and a
    /// player who goes broke is topped back up before the next hand.
    #[wasm_bindgen(constructor)]
    pub fn new(
        bots: Vec<String>,
        buy_in: u64,
        small_blind: u64,
        big_blind: u64,
        seed: u64,
    ) -> Result<BotTable, JsError> {
        if bots.is_empty() || bots.len() + 1 > ducy_play::MAX_PLAYERS {
            return Err(JsError::new("between 1 and 8 bots"));
        }
        if small_blind == 0 || big_blind < small_blind || buy_in < big_blind {
            return Err(JsError::new("invalid blinds or buy-in"));
        }
        let mut names = vec!["You".to_string()];
        let mut ids = vec!["you".to_string()];
        let mut seats = vec![None];
        for (i, id) in bots.iter().enumerate() {
            let p = Personality::from_name(id)
                .ok_or_else(|| JsError::new(&format!("unknown bot {id}")))?;
            names.push(p.name().to_string());
            ids.push(p.id().to_string());
            seats.push(Some(p.bot(Some(seed.wrapping_add(i as u64 + 1)))));
        }
        let n = names.len();
        Ok(BotTable {
            rules: TableRules::no_limit_holdem(small_blind, big_blind),
            names,
            ids,
            bots: seats,
            stacks: vec![buy_in; n],
            buy_in,
            button: n - 1,
            hand: None,
            hand_number: 0,
            seed,
        })
    }

    /// Deals the next hand (moving the button and topping up broke players)
    /// and returns the state.
    #[wasm_bindgen(js_name = newHand)]
    pub fn new_hand(&mut self) -> Result<JsValue, JsError> {
        if let Some(h) = &self.hand {
            match h.result() {
                Some(r) => self.stacks = r.final_stacks.clone(),
                None => return Err(JsError::new("the current hand isn't over")),
            }
        }
        for s in &mut self.stacks {
            if *s == 0 {
                *s = self.buy_in;
            }
        }
        self.button = (self.button + 1) % self.stacks.len();
        self.hand_number += 1;
        let deal = Deal::random(
            self.rules.variant,
            self.stacks.len(),
            Some(self.seed.wrapping_add(self.hand_number * 7919)),
        )
        .map_err(err)?;
        self.hand = Some(Hand::new(self.rules, &self.stacks, self.button, deal).map_err(err)?);
        self.state()
    }

    /// Lets the next bot act if it's a bot's turn; returns the state either way.
    pub fn advance(&mut self) -> Result<JsValue, JsError> {
        let hand = self
            .hand
            .as_mut()
            .ok_or_else(|| JsError::new("no hand dealt"))?;
        if let Some(seat) = hand.to_act()
            && let Some(bot) = self.bots[seat].as_mut()
        {
            let obs = hand
                .observation(seat)
                .ok_or_else(|| JsError::new("no observation"))?;
            let action = bot.act(&obs);
            let ok = action.is_some_and(|a| hand.act(a).is_ok());
            if !ok {
                hand.act(ducy_play::fallback_action(&obs.legal))
                    .map_err(err)?;
            }
            self.finish_if_over();
        }
        self.state()
    }

    /// Applies the person's action: "fold", "check", "call", "bet", "raise"
    /// or "allin". `amount` is the street total for a bet or raise.
    pub fn act(&mut self, kind: &str, amount: u64) -> Result<JsValue, JsError> {
        let hand = self
            .hand
            .as_mut()
            .ok_or_else(|| JsError::new("no hand dealt"))?;
        if hand.to_act() != Some(0) {
            return Err(JsError::new("it isn't your turn"));
        }
        let action = match kind {
            "fold" => Action::Fold,
            "check" => Action::Check,
            "call" => Action::Call,
            "bet" => Action::Bet(amount),
            "raise" => Action::Raise(amount),
            "allin" => Action::AllIn,
            _ => return Err(JsError::new(&format!("unknown action {kind}"))),
        };
        hand.act(action).map_err(err)?;
        self.finish_if_over();
        self.state()
    }

    /// Whether it's a bot's turn (the page calls `advance` while this is true).
    #[wasm_bindgen(js_name = botToAct)]
    pub fn bot_to_act(&self) -> bool {
        self.hand
            .as_ref()
            .and_then(|h| h.to_act())
            .is_some_and(|s| self.bots[s].is_some())
    }

    /// What the person may see right now.
    pub fn state(&self) -> Result<JsValue, JsError> {
        let hand = self
            .hand
            .as_ref()
            .ok_or_else(|| JsError::new("no hand dealt"))?;
        let result = hand.result();
        let showdown = result.is_some_and(|r| r.showdown);
        let seats = (0..hand.num_seats())
            .map(|s| {
                let shown = s == 0 || (showdown && !hand.has_folded(s));
                SeatState {
                    name: self.names[s].clone(),
                    id: self.ids[s].clone(),
                    stack: result.map_or(hand.stack(s), |r| r.final_stacks[s]),
                    street_bet: if result.is_some() {
                        0
                    } else {
                        hand.street_bet(s)
                    },
                    folded: hand.has_folded(s),
                    all_in: hand.is_all_in(s),
                    cards: shown.then(|| cards(hand.deal().hole_cards()[s])),
                    won: result.map_or(0, |r| r.payouts[s]),
                    net: result.map_or(0, |r| r.net[s]),
                }
            })
            .collect();
        let to_act = hand.to_act();
        let state = TableState {
            hand_number: self.hand_number,
            street: (!hand.is_complete()).then(|| hand.street()),
            board: card_strings(hand.board()),
            pot: hand.pot(),
            current_bet: hand.current_bet(),
            button: hand.button(),
            hero: 0,
            big_blind: self.rules.big_blind,
            to_act,
            legal: hand.legal_actions().filter(|l| l.seat == 0),
            seats,
            events: hand.events().to_vec(),
            complete: hand.is_complete(),
            showdown,
            pots: result.map_or_else(Vec::new, |r| r.pots.clone()),
        };
        serde_wasm_bindgen::to_value(&state).map_err(err)
    }
}

impl BotTable {
    /// Tells the bots how a finished hand went, so they can adapt.
    fn finish_if_over(&mut self) {
        let Some(hand) = &self.hand else { return };
        if !hand.is_complete() {
            return;
        }
        for (seat, bot) in self.bots.iter_mut().enumerate() {
            if let (Some(bot), Some(summary)) = (bot.as_mut(), hand.summary(seat)) {
                bot.hand_over(&summary);
            }
        }
    }
}
