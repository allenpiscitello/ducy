//! A table that plays hand after hand: seats, stacks, the button and bots.
//!
//! [`Table`] is the game state a host keeps. Seats are played by people or by
//! personality bots, and a seat can switch between the two between hands.
//! [`Table::view`] builds what one seat may see, with seats renumbered so
//! that seat is always seat 0, the way a poker client puts you at the bottom.

use ducy::deck::{Card, Deck};

use crate::{
    Action, BettingStructure, Bot, Deal, Event, Hand, LegalActions, PersonalityBot, PlayError, Pot,
    Street, TableRules, fallback_action,
};

/// One seat: who sits there and the bot that plays it when no one does.
pub struct TableSeat {
    /// Shown name.
    pub name: String,
    /// Personality id for a bot, or any id for a person (e.g. "you").
    pub id: String,
    /// Plays the seat when `human` is false. A seat without a bot is always
    /// played by a person.
    pub bot: Option<Box<dyn Bot>>,
    /// The bot's name, shown again when a person gives the seat back.
    pub bot_name: String,
    /// Whether a person plays this seat.
    pub human: bool,
    /// A person who isn't there: they check or fold at once.
    pub away: bool,
}

impl TableSeat {
    /// A seat for a person.
    pub fn human(name: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            id: id.into(),
            bot: None,
            bot_name: String::new(),
            human: true,
            away: false,
        }
    }

    /// A seat played by a personality bot.
    pub fn bot(id: impl Into<String>, bot: PersonalityBot) -> Self {
        let name = bot.name();
        Self::with_bot(name, id, Box::new(bot))
    }

    /// A seat played by any bot, shown as `name`.
    pub fn with_bot(name: impl Into<String>, id: impl Into<String>, bot: Box<dyn Bot>) -> Self {
        let name = name.into();
        Self {
            name: name.clone(),
            id: id.into(),
            bot: Some(bot),
            bot_name: name,
            human: false,
            away: false,
        }
    }
}

/// What a seat may see of the table: its own cards, public chip counts, and
/// other players' cards only once they're shown down. Seat numbers are
/// rotated so the viewer is seat 0 (`hero`).
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TableView {
    pub hand_number: u64,
    /// `None` once the hand is over.
    pub street: Option<Street>,
    pub board: Vec<String>,
    pub pot: u64,
    pub current_bet: u64,
    pub button: usize,
    /// Always 0: the viewer.
    pub hero: usize,
    /// The viewer's real seat number at the table.
    pub seat: usize,
    pub small_blind: u64,
    pub big_blind: u64,
    /// Hole cards per player: 2 for Hold'em, 4 to 6 for Omaha.
    pub hole_cards: usize,
    /// Pot-limit betting (otherwise no-limit).
    pub pot_limit: bool,
    /// The seat whose turn it is, if the hand is still going.
    pub to_act: Option<usize>,
    /// What the viewer may do, when it's their turn.
    pub legal: Option<LegalActions>,
    pub seats: Vec<SeatState>,
    pub events: Vec<Event>,
    pub complete: bool,
    pub showdown: bool,
    pub pots: Vec<Pot>,
}

/// One seat as a viewer sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SeatState {
    pub name: String,
    /// Personality id, or the person's id.
    pub id: String,
    /// Whether a person plays the seat.
    pub human: bool,
    /// A person who isn't there right now.
    pub away: bool,
    pub stack: u64,
    pub street_bet: u64,
    pub folded: bool,
    pub all_in: bool,
    /// The viewer's own cards always; anyone else's only once shown down.
    pub cards: Option<Vec<String>>,
    /// Chips won this hand, once it's over.
    pub won: u64,
    /// Net result this hand, once it's over.
    pub net: i64,
}

/// Hand after hand at one table. Everyone starts with `buy_in` chips, and a
/// player who goes broke is topped back up before the next hand.
pub struct Table {
    rules: TableRules,
    seats: Vec<TableSeat>,
    stacks: Vec<u64>,
    buy_in: u64,
    button: usize,
    hand: Option<Hand>,
    hand_number: u64,
    /// Whether `stacks` has taken in the finished hand's result, so a stack
    /// reset between hands isn't overwritten by it.
    synced: bool,
    seed: u64,
}

impl Table {
    /// A table of `seats` (2 to [`crate::MAX_PLAYERS`]). The first hand
    /// puts the button on seat 0.
    pub fn new(
        rules: TableRules,
        seats: Vec<TableSeat>,
        buy_in: u64,
        seed: u64,
    ) -> Result<Self, PlayError> {
        if seats.len() < 2 || seats.len() > crate::MAX_PLAYERS {
            return Err(PlayError::InvalidPlayerCount);
        }
        if rules.small_blind == 0 || rules.big_blind < rules.small_blind || buy_in < rules.big_blind
        {
            return Err(PlayError::InvalidBlinds);
        }
        if seats.iter().any(|s| !s.human && s.bot.is_none()) {
            return Err(PlayError::InvalidPlayerCount);
        }
        let n = seats.len();
        Ok(Self {
            rules,
            seats,
            stacks: vec![buy_in; n],
            buy_in,
            button: n - 1,
            hand: None,
            hand_number: 0,
            synced: false,
            seed,
        })
    }

    pub fn rules(&self) -> &TableRules {
        &self.rules
    }

    pub fn num_seats(&self) -> usize {
        self.seats.len()
    }

    pub fn seat(&self, seat: usize) -> &TableSeat {
        &self.seats[seat]
    }

    /// Change who plays a seat. Takes effect at once, so do it between hands
    /// unless the seat should be taken over mid-hand.
    pub fn seat_mut(&mut self, seat: usize) -> &mut TableSeat {
        &mut self.seats[seat]
    }

    pub fn hand(&self) -> Option<&Hand> {
        self.hand.as_ref()
    }

    pub fn hand_number(&self) -> u64 {
        self.hand_number
    }

    /// Chips behind for `seat`: in the current hand, or between hands.
    pub fn stack(&self, seat: usize) -> u64 {
        match &self.hand {
            Some(h) if !self.synced => h
                .result()
                .map_or_else(|| h.stack(seat), |r| r.final_stacks[seat]),
            _ => self.stacks[seat],
        }
    }

    /// Resets a seat's stack to the buy-in, e.g. when a new person sits down.
    /// Only between hands.
    pub fn reset_stack(&mut self, seat: usize) -> Result<(), PlayError> {
        if self.hand.as_ref().is_some_and(|h| !h.is_complete()) {
            return Err(PlayError::IllegalAction);
        }
        self.sync_stacks();
        self.stacks[seat] = self.buy_in;
        Ok(())
    }

    /// Takes in the finished hand's stacks, once.
    fn sync_stacks(&mut self) {
        if self.synced {
            return;
        }
        if let Some(r) = self.hand.as_ref().and_then(|h| h.result()) {
            self.stacks = r.final_stacks.clone();
            self.synced = true;
        }
    }

    /// Whether a hand is being played (dealt and not over).
    pub fn in_hand(&self) -> bool {
        self.hand.as_ref().is_some_and(|h| !h.is_complete())
    }

    /// Deals the next hand, moving the button and topping up broke players.
    pub fn new_hand(&mut self) -> Result<(), PlayError> {
        if self.in_hand() {
            return Err(PlayError::IllegalAction);
        }
        self.sync_stacks();
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
        )?;
        self.hand = Some(Hand::new(self.rules, &self.stacks, self.button, deal)?);
        self.synced = false;
        Ok(())
    }

    /// The seat whose turn it is, if a hand is going.
    pub fn to_act(&self) -> Option<usize> {
        self.hand.as_ref().and_then(|h| h.to_act())
    }

    /// Whether the seat to act plays without waiting for a person: a bot, or
    /// a person who is away.
    pub fn auto_to_act(&self) -> bool {
        self.to_act().is_some_and(|s| {
            let seat = &self.seats[s];
            !seat.human || seat.away
        })
    }

    /// Lets the seat to act play if it doesn't need a person: a bot decides,
    /// and an away person checks or folds. Returns whether anyone acted.
    pub fn advance(&mut self) -> Result<bool, PlayError> {
        if !self.auto_to_act() {
            return Ok(false);
        }
        let hand = self.hand.as_mut().ok_or(PlayError::HandComplete)?;
        let seat = hand.to_act().ok_or(PlayError::HandComplete)?;
        let obs = hand.observation(seat).ok_or(PlayError::HandComplete)?;
        let s = &mut self.seats[seat];
        let action = match (&mut s.bot, s.human) {
            (Some(bot), false) => bot.act(&obs),
            _ => None,
        };
        if !action.is_some_and(|a| hand.act(a).is_ok()) {
            hand.act(fallback_action(&obs.legal))?;
        }
        self.finish_if_over();
        Ok(true)
    }

    /// Plays `action` for `seat`, which must be the seat to act.
    pub fn act(&mut self, seat: usize, action: Action) -> Result<(), PlayError> {
        let hand = self.hand.as_mut().ok_or(PlayError::HandComplete)?;
        if hand.to_act() != Some(seat) {
            return Err(PlayError::IllegalAction);
        }
        hand.act(action)?;
        self.finish_if_over();
        Ok(())
    }

    /// Checks if it can, folds otherwise: for a person who ran out of time.
    pub fn act_default(&mut self, seat: usize) -> Result<(), PlayError> {
        let legal = self
            .hand
            .as_ref()
            .and_then(|h| h.legal_actions())
            .filter(|l| l.seat == seat)
            .ok_or(PlayError::IllegalAction)?;
        self.act(seat, fallback_action(&legal))
    }

    /// Tells the bots how a finished hand went, so they can adapt.
    fn finish_if_over(&mut self) {
        let Some(hand) = &self.hand else { return };
        if !hand.is_complete() {
            return;
        }
        for (seat, s) in self.seats.iter_mut().enumerate() {
            if let (Some(bot), Some(summary)) = (s.bot.as_mut(), hand.summary(seat)) {
                bot.hand_over(&summary);
            }
        }
    }

    /// What `seat` may see right now, with seats renumbered so `seat` is 0.
    /// Before the first hand there are no cards and no one to act.
    pub fn view(&self, seat: usize) -> TableView {
        let n = self.seats.len();
        let rot = |s: usize| (s + n - seat) % n;
        let order = (0..n).map(|i| (seat + i) % n);
        let Some(hand) = &self.hand else {
            return TableView {
                hand_number: 0,
                street: None,
                board: Vec::new(),
                pot: 0,
                current_bet: 0,
                button: rot(self.button),
                hero: 0,
                seat,
                small_blind: self.rules.small_blind,
                big_blind: self.rules.big_blind,
                hole_cards: self.rules.variant.hole_cards(),
                pot_limit: self.rules.structure == BettingStructure::PotLimit,
                to_act: None,
                legal: None,
                seats: order
                    .map(|s| self.seat_state(s, self.stacks[s], 0, false, false, None, 0, 0))
                    .collect(),
                events: Vec::new(),
                complete: true,
                showdown: false,
                pots: Vec::new(),
            };
        };
        let result = hand.result();
        let showdown = result.is_some_and(|r| r.showdown);
        let seats = order
            .map(|s| {
                let shown = s == seat || (showdown && !hand.has_folded(s));
                self.seat_state(
                    s,
                    result.map_or(hand.stack(s), |r| r.final_stacks[s]),
                    if result.is_some() {
                        0
                    } else {
                        hand.street_bet(s)
                    },
                    hand.has_folded(s),
                    hand.is_all_in(s),
                    shown.then(|| cards(hand.deal().hole_cards()[s])),
                    result.map_or(0, |r| r.payouts[s]),
                    result.map_or(0, |r| r.net[s]),
                )
            })
            .collect();
        TableView {
            hand_number: self.hand_number,
            street: (!hand.is_complete()).then(|| hand.street()),
            board: hand.board().iter().map(Card::to_string).collect(),
            pot: hand.pot(),
            current_bet: hand.current_bet(),
            button: rot(hand.button()),
            hero: 0,
            seat,
            small_blind: self.rules.small_blind,
            big_blind: self.rules.big_blind,
            hole_cards: self.rules.variant.hole_cards(),
            pot_limit: self.rules.structure == BettingStructure::PotLimit,
            to_act: hand.to_act().map(rot),
            legal: hand
                .legal_actions()
                .filter(|l| l.seat == seat)
                .map(|l| LegalActions { seat: 0, ..l }),
            seats,
            events: hand.events().iter().map(|e| rotate_event(e, rot)).collect(),
            complete: hand.is_complete(),
            showdown,
            pots: result.map_or_else(Vec::new, |r| {
                r.pots.iter().map(|p| rotate_pot(p, rot)).collect()
            }),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn seat_state(
        &self,
        s: usize,
        stack: u64,
        street_bet: u64,
        folded: bool,
        all_in: bool,
        cards: Option<Vec<String>>,
        won: u64,
        net: i64,
    ) -> SeatState {
        let seat = &self.seats[s];
        SeatState {
            name: seat.name.clone(),
            id: seat.id.clone(),
            human: seat.human,
            away: seat.human && seat.away,
            stack,
            street_bet,
            folded,
            all_in,
            cards,
            won,
            net,
        }
    }
}

fn cards(deck: Deck) -> Vec<String> {
    deck.iter(true).map(|c| c.to_string()).collect()
}

fn rotate_event(e: &Event, rot: impl Fn(usize) -> usize) -> Event {
    let mut e = e.clone();
    match &mut e {
        Event::Ante { seat, .. }
        | Event::SmallBlind { seat, .. }
        | Event::BigBlind { seat, .. }
        | Event::Fold { seat }
        | Event::Check { seat }
        | Event::Call { seat, .. }
        | Event::Bet { seat, .. }
        | Event::Raise { seat, .. }
        | Event::Award { seat, .. } => *seat = rot(*seat),
        Event::Board { .. } => {}
    }
    e
}

fn rotate_pot(p: &Pot, rot: impl Fn(usize) -> usize) -> Pot {
    let mut p = p.clone();
    p.eligible.iter_mut().for_each(|s| *s = rot(*s));
    p.awards.iter_mut().for_each(|a| a.seat = rot(a.seat));
    p
}
