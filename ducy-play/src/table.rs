//! A table that plays hand after hand: seats, stacks, the button and bots.
//!
//! [`Table`] is the game state a host keeps. Seats are played by people or by
//! personality bots, and a seat can switch between the two between hands. A
//! seat can also be empty, and a person can sit out: each hand is dealt only
//! to the seats in play, so a table of friends can wait for people to join.
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
    /// A person who isn't dealt in from the next hand on.
    pub sitting_out: bool,
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
            sitting_out: false,
        }
    }

    /// A seat with no one in it: it isn't dealt in until someone sits down.
    pub fn empty() -> Self {
        Self {
            name: String::new(),
            id: String::new(),
            bot: None,
            bot_name: String::new(),
            human: false,
            away: false,
            sitting_out: false,
        }
    }

    /// Whether no one sits here: no person and no bot.
    pub fn is_empty(&self) -> bool {
        !self.human && self.bot.is_none()
    }

    /// Whether the next hand deals this seat in.
    pub fn plays(&self) -> bool {
        if self.human {
            !self.sitting_out
        } else {
            self.bot.is_some()
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
            sitting_out: false,
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
    /// No one sits here.
    #[cfg_attr(feature = "serde", serde(default))]
    pub empty: bool,
    /// A person who sits here but isn't in this hand (or, between hands,
    /// won't be dealt in).
    #[cfg_attr(feature = "serde", serde(default))]
    pub sitting_out: bool,
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
/// player who goes broke is topped back up before the next hand (unless
/// [`Table::set_top_up`] turns that off: then chips carry over and a seat
/// without chips sits out until it gets some). Each hand is
/// dealt to the seats in play ([`TableSeat::plays`]); the hand numbers its
/// players 0, 1, … in seat order, and the table maps them back to seats.
pub struct Table {
    rules: TableRules,
    seats: Vec<TableSeat>,
    stacks: Vec<u64>,
    buy_in: u64,
    button: usize,
    hand: Option<Hand>,
    /// The seat of each of the current hand's players.
    dealt: Vec<usize>,
    hand_number: u64,
    /// Whether `stacks` has taken in the finished hand's result, so a stack
    /// reset between hands isn't overwritten by it.
    synced: bool,
    /// Whether a seat out of chips gets the buy-in again.
    top_up: bool,
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
        let n = seats.len();
        Ok(Self {
            rules,
            seats,
            stacks: vec![buy_in; n],
            buy_in,
            button: n - 1,
            hand: None,
            dealt: Vec::new(),
            hand_number: 0,
            synced: false,
            top_up: true,
            seed,
        })
    }

    pub fn rules(&self) -> &TableRules {
        &self.rules
    }

    /// Changes the game or the blinds from the next hand on, e.g. for a game
    /// rotation or a tournament's blind levels. Only between hands.
    pub fn set_rules(&mut self, rules: TableRules) -> Result<(), PlayError> {
        if self.in_hand() {
            return Err(PlayError::IllegalAction);
        }
        rules.validate()?;
        self.rules = rules;
        Ok(())
    }

    /// The button's seat: in the current (or last) hand, or before the first
    /// hand the seat it moves on from.
    pub fn button(&self) -> usize {
        self.button
    }

    /// The seat that would post the big blind if the next hand were dealt
    /// now, or `None` if fewer than two seats would be dealt in.
    pub fn next_big_blind(&self) -> Option<usize> {
        let n = self.seats.len();
        let dealt: Vec<usize> = (0..n).filter(|&s| self.will_play(s)).collect();
        if dealt.len() < 2 {
            return None;
        }
        let next_after = |from: usize| {
            (1..=n)
                .map(|k| (from + k) % n)
                .find(|s| dealt.contains(s))
                .expect("two players are in")
        };
        let button = next_after(self.button);
        let small_blind = if dealt.len() == 2 {
            button
        } else {
            next_after(button)
        };
        Some(next_after(small_blind))
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

    /// The current (or last) hand. Its players are numbered 0, 1, … among
    /// the seats dealt in; see [`Self::hand_index`].
    pub fn hand(&self) -> Option<&Hand> {
        self.hand.as_ref()
    }

    /// `seat`'s player number in the current hand, if it was dealt in.
    pub fn hand_index(&self, seat: usize) -> Option<usize> {
        self.dealt.iter().position(|&s| s == seat)
    }

    /// The seats dealt into the current hand, in order.
    pub fn dealt(&self) -> &[usize] {
        &self.dealt
    }

    /// The finished hand's summary for `seat` (its seat numbers are the
    /// hand's player numbers), if `seat` played it.
    pub fn summary(&self, seat: usize) -> Option<crate::HandSummary> {
        self.hand.as_ref()?.summary(self.hand_index(seat)?)
    }

    pub fn hand_number(&self) -> u64 {
        self.hand_number
    }

    /// Chips behind for `seat`: in the current hand, or between hands.
    pub fn stack(&self, seat: usize) -> u64 {
        match (&self.hand, self.hand_index(seat)) {
            (Some(h), Some(i)) if !self.synced => {
                h.result().map_or_else(|| h.stack(i), |r| r.final_stacks[i])
            }
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
            for (i, &s) in self.dealt.iter().enumerate() {
                self.stacks[s] = r.final_stacks[i];
            }
            self.synced = true;
        }
    }

    /// Whether a hand is being played (dealt and not over).
    pub fn in_hand(&self) -> bool {
        self.hand.as_ref().is_some_and(|h| !h.is_complete())
    }

    /// Whether a seat out of chips gets the buy-in again before the next
    /// hand (on by default). Off, chips carry over: a seat with none isn't
    /// dealt in until [`Self::add_chips`] gives it some.
    pub fn set_top_up(&mut self, on: bool) {
        self.top_up = on;
    }

    /// Adds chips to a seat that isn't in a hand being played.
    pub fn add_chips(&mut self, seat: usize, amount: u64) -> Result<(), PlayError> {
        if self.in_hand() && self.hand_index(seat).is_some() {
            return Err(PlayError::IllegalAction);
        }
        self.sync_stacks();
        self.stacks[seat] += amount;
        Ok(())
    }

    /// Sets the chips of a seat that isn't in a hand being played.
    pub fn set_stack(&mut self, seat: usize, chips: u64) -> Result<(), PlayError> {
        if self.in_hand() && self.hand_index(seat).is_some() {
            return Err(PlayError::IllegalAction);
        }
        self.sync_stacks();
        self.stacks[seat] = chips;
        Ok(())
    }

    /// Whether `seat` would be dealt into the next hand: someone plays it
    /// and it has chips (or will be topped up).
    fn will_play(&self, seat: usize) -> bool {
        self.seats[seat].plays() && (self.top_up || self.stack(seat) > 0)
    }

    /// How many seats the next hand would deal in.
    pub fn players_in(&self) -> usize {
        (0..self.seats.len()).filter(|&s| self.will_play(s)).count()
    }

    /// Deals the next hand to the seats in play, moving the button to the
    /// next of them and topping up broke players. Fails with fewer than two
    /// players.
    pub fn new_hand(&mut self) -> Result<(), PlayError> {
        if self.in_hand() {
            return Err(PlayError::IllegalAction);
        }
        let n = self.seats.len();
        self.sync_stacks();
        let dealt: Vec<usize> = (0..n).filter(|&s| self.will_play(s)).collect();
        if dealt.len() < 2 {
            return Err(PlayError::InvalidPlayerCount);
        }
        if self.top_up {
            for s in &mut self.stacks {
                if *s == 0 {
                    *s = self.buy_in;
                }
            }
        }
        self.button = (1..=n)
            .map(|k| (self.button + k) % n)
            .find(|s| dealt.contains(s))
            .expect("two players are in");
        self.hand_number += 1;
        let deal = Deal::random(
            self.rules.variant,
            dealt.len(),
            Some(self.seed.wrapping_add(self.hand_number * 7919)),
        )?;
        let stacks: Vec<u64> = dealt.iter().map(|&s| self.stacks[s]).collect();
        let button = dealt.iter().position(|&s| s == self.button).expect("dealt");
        self.hand = Some(Hand::new(self.rules, &stacks, button, deal)?);
        self.dealt = dealt;
        self.synced = false;
        Ok(())
    }

    /// The seat whose turn it is, if a hand is going.
    pub fn to_act(&self) -> Option<usize> {
        self.hand
            .as_ref()
            .and_then(|h| h.to_act())
            .map(|i| self.dealt[i])
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
        let i = hand.to_act().ok_or(PlayError::HandComplete)?;
        let obs = hand.observation(i).ok_or(PlayError::HandComplete)?;
        let s = &mut self.seats[self.dealt[i]];
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
        let i = self.hand_index(seat);
        let hand = self.hand.as_mut().ok_or(PlayError::HandComplete)?;
        if i.is_none() || hand.to_act() != i {
            return Err(PlayError::IllegalAction);
        }
        hand.act(action)?;
        self.finish_if_over();
        Ok(())
    }

    /// Checks if it can, folds otherwise: for a person who ran out of time.
    pub fn act_default(&mut self, seat: usize) -> Result<(), PlayError> {
        let i = self.hand_index(seat);
        let legal = self
            .hand
            .as_ref()
            .and_then(|h| h.legal_actions())
            .filter(|l| Some(l.seat) == i)
            .ok_or(PlayError::IllegalAction)?;
        self.act(seat, fallback_action(&legal))
    }

    /// Tells the bots how a finished hand went, so they can adapt.
    fn finish_if_over(&mut self) {
        let Some(hand) = &self.hand else { return };
        if !hand.is_complete() {
            return;
        }
        for (i, &seat) in self.dealt.iter().enumerate() {
            if let (Some(bot), Some(summary)) = (self.seats[seat].bot.as_mut(), hand.summary(i)) {
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
                    .map(|s| {
                        // Someone who won't be dealt in (sitting out, or no chips).
                        let out = self.seats[s].human && !self.will_play(s);
                        self.seat_state(s, self.stacks[s], 0, false, false, None, 0, 0, out)
                    })
                    .collect(),
                events: Vec::new(),
                complete: true,
                showdown: false,
                pots: Vec::new(),
            };
        };
        let result = hand.result();
        let showdown = result.is_some_and(|r| r.showdown);
        // Hand players to rotated seats.
        let seat_of = |i: usize| rot(self.dealt[i]);
        let seats = order
            .map(|s| {
                let Some(i) = self.hand_index(s) else {
                    // Not in this hand: an empty seat, or a person sitting
                    // out (or waiting for the next hand).
                    let stack = self.stack(s);
                    return self.seat_state(
                        s,
                        stack,
                        0,
                        false,
                        false,
                        None,
                        0,
                        0,
                        self.seats[s].human,
                    );
                };
                let shown = s == seat || (showdown && !hand.has_folded(i));
                self.seat_state(
                    s,
                    result.map_or(hand.stack(i), |r| r.final_stacks[i]),
                    if result.is_some() {
                        0
                    } else {
                        hand.street_bet(i)
                    },
                    hand.has_folded(i),
                    hand.is_all_in(i),
                    shown.then(|| cards(hand.deal().hole_cards()[i])),
                    result.map_or(0, |r| r.payouts[i]),
                    result.map_or(0, |r| r.net[i]),
                    false,
                )
            })
            .collect();
        let me = self.hand_index(seat);
        TableView {
            hand_number: self.hand_number,
            street: (!hand.is_complete()).then(|| hand.street()),
            board: hand.board().iter().map(Card::to_string).collect(),
            pot: hand.pot(),
            current_bet: hand.current_bet(),
            button: seat_of(hand.button()),
            hero: 0,
            seat,
            small_blind: self.rules.small_blind,
            big_blind: self.rules.big_blind,
            hole_cards: self.rules.variant.hole_cards(),
            pot_limit: self.rules.structure == BettingStructure::PotLimit,
            to_act: hand.to_act().map(seat_of),
            legal: hand
                .legal_actions()
                .filter(|l| Some(l.seat) == me)
                .map(|l| LegalActions { seat: 0, ..l }),
            seats,
            events: hand
                .events()
                .iter()
                .map(|e| rotate_event(e, seat_of))
                .collect(),
            complete: hand.is_complete(),
            showdown,
            pots: result.map_or_else(Vec::new, |r| {
                r.pots.iter().map(|p| rotate_pot(p, seat_of)).collect()
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
        sitting_out: bool,
    ) -> SeatState {
        let seat = &self.seats[s];
        SeatState {
            name: seat.name.clone(),
            id: seat.id.clone(),
            human: seat.human,
            away: seat.human && seat.away,
            empty: seat.is_empty(),
            sitting_out,
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
