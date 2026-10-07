//! Hosting a [`Table`] for people playing from other devices.
//!
//! The host keeps the whole game state; players only send [`Command`]s and
//! receive [`Update`]s built for them, so no one ever receives another
//! player's cards before they're shown down. [`TableHost`] doesn't do any
//! networking: feed it each player's commands and send each [`Outgoing`] it
//! returns to its recipient over whatever connection the app uses.
//!
//! Seat 0 is the host's own seat. Seats marked *open* are for people: an
//! open seat with a bot is played by the bot until someone joins, and an
//! empty one ([`crate::TableSeat::empty`]) isn't dealt in. A person takes the
//! seat at the start of the next hand, and gives it back (to its bot, or
//! empty) when they leave or the host removes them. A person who's away or
//! disconnected sits out, and one who stays disconnected for
//! [`DROP_AFTER_HANDS`] hands is removed. Times are milliseconds from any
//! clock the caller chooses, which keeps the host deterministic.
//!
//! With a bank ([`TableHost::with_bank`]), chips are real: they carry over
//! from hand to hand, people sit down with none, and they ask the host for
//! chips ([`Command::RequestChips`]), who approves or denies each request.
//! A stack after a request must be between the table's minimum and maximum
//! buy-in. Without one, every seat is topped back up to the buy-in.

use crate::{Action, PlayError, Table, TableView};

/// A message from a player to the host.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "type", rename_all = "snake_case"))]
pub enum Command {
    /// Take an open seat, or get your seat back after reconnecting with the
    /// same `client_id`.
    Join { name: String },
    /// Play an action: "fold", "check", "call", "bet", "raise" or "allin",
    /// with `amount` the street total for a bet or raise. `seq` is the
    /// state the player saw, so a stale or repeated action is rejected.
    Act {
        seq: u64,
        kind: String,
        #[cfg_attr(feature = "serde", serde(default))]
        amount: u64,
    },
    /// Back after being away (timed out twice).
    SitIn,
    /// Give the seat back (to its bot, or empty).
    Leave,
    /// Ask the host for `amount` more chips (0 withdraws the request). The
    /// stack after it must be within the table's buy-in limits.
    RequestChips { amount: u64 },
}

/// A message from the host to one player.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "type", rename_all = "snake_case"))]
pub enum Update {
    /// You have a seat. You play from the next hand if one is going.
    Welcome { seat: usize },
    /// The table as you see it (you are seat 0), as of state `seq`.
    State {
        seq: u64,
        view: Box<TableView>,
        /// Time left for whoever is acting, if a person is on the clock.
        turn_ms_left: Option<u64>,
        /// Your chips and requests, at a table with a bank.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        chips: Option<ChipsView>,
    },
    /// Your command was refused.
    Rejected { reason: String },
}

/// What a player knows about chips at a table with a bank, sent with each
/// state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ChipsView {
    /// The smallest and largest stack a request may bring you to.
    pub min: u64,
    pub max: u64,
    /// Chips you've asked for and the host hasn't answered.
    pub requested: u64,
    /// Chips approved, added before the next hand.
    pub approved: u64,
}

/// A request for chips, for the host to approve or deny.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ChipRequest {
    pub seat: usize,
    pub name: String,
    pub amount: u64,
    /// Chips the player has now.
    pub stack: u64,
}

/// Someone who gave their seat back at a table with a bank, and the chips
/// they took with them: their stack plus any chips approved but not yet
/// added. The app returns these to wherever the player's chips came from.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Departure {
    pub client_id: String,
    pub seat: usize,
    pub chips: u64,
}

/// A person at the table and the chips that are theirs: their stack (none
/// yet if they sit down at the next hand) plus chips approved but not yet
/// added. Between hands this is everything they'd leave with; during a hand
/// it leaves out what they've put in the pot.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SeatedPlayer {
    pub client_id: String,
    pub seat: usize,
    pub chips: u64,
    /// Sits down at the next hand.
    pub pending: bool,
    /// Gives the seat back once this hand is over.
    pub leaving: bool,
}

/// An update and the client it's for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outgoing {
    pub to: String,
    pub update: Update,
}

struct Player {
    client_id: String,
    name: String,
    seat: usize,
    connected: bool,
    /// Turns in a row that ran out of time.
    timeouts: u32,
    /// Plays from the next hand (joined mid-hand).
    pending: bool,
    /// Gave the seat back; the bot returns at the next hand.
    leaving: bool,
    /// Hands dealt in a row while disconnected.
    missed: u32,
    /// Chips asked for, not yet answered.
    requested: u64,
    /// Chips approved during a hand they're in, added before the next.
    approved: u64,
}

/// A table's buy-in limits, in chips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Bank {
    min: u64,
    max: u64,
}

/// Hands a disconnected person can miss before they're removed.
pub const DROP_AFTER_HANDS: u32 = 3;

/// Runs a table for the host (seat 0) and remote players.
pub struct TableHost {
    table: Table,
    open: Vec<bool>,
    players: Vec<Player>,
    seq: u64,
    turn_ms: u64,
    deadline: Option<u64>,
    /// Which turn the deadline belongs to: (hand number, events so far).
    turn: Option<(u64, usize)>,
    /// Each seat's id when its bot plays it.
    bot_ids: Vec<String>,
    bank: Option<Bank>,
    /// The host's own chips approved during a hand they're in.
    host_approved: u64,
    /// Whether seat 0 is the host's (see [`Self::without_host`]).
    has_host: bool,
    /// People who left with chips since [`Self::take_departures`].
    departed: Vec<Departure>,
}

/// Most characters kept from a player's name.
pub const MAX_NAME: usize = 20;

impl TableHost {
    /// Hosts `table`, where seat 0 is the host and `open[s]` says whether
    /// people may take seat `s`. `turn_ms` is how long a person has to act
    /// (0 for no limit).
    pub fn new(table: Table, open: Vec<bool>, turn_ms: u64) -> Result<Self, PlayError> {
        if open.len() != table.num_seats() || open[0] {
            return Err(PlayError::InvalidPlayerCount);
        }
        // A closed seat needs someone to play it.
        if (1..open.len()).any(|s| !open[s] && table.seat(s).is_empty()) {
            return Err(PlayError::InvalidPlayerCount);
        }
        Ok(Self::build(table, open, turn_ms, true))
    }

    /// Hosts `table` with no seat for the host, e.g. a club table the host
    /// runs without playing: every seat is open to people and starts empty,
    /// and chips are real, within `min` to `max` (see [`Self::with_bank`]).
    /// The host only watches: [`Self::host_view`] shows no cards that
    /// haven't been shown down, and the host can't act or take chips.
    pub fn without_host(table: Table, turn_ms: u64, min: u64, max: u64) -> Result<Self, PlayError> {
        if (0..table.num_seats()).any(|s| !table.seat(s).is_empty()) {
            return Err(PlayError::InvalidPlayerCount);
        }
        let open = vec![true; table.num_seats()];
        Self::build(table, open, turn_ms, false).with_bank(min, max)
    }

    fn build(table: Table, open: Vec<bool>, turn_ms: u64, has_host: bool) -> Self {
        let bot_ids = (0..table.num_seats())
            .map(|s| table.seat(s).id.clone())
            .collect();
        Self {
            table,
            open,
            players: Vec::new(),
            seq: 0,
            turn_ms,
            deadline: None,
            turn: None,
            bot_ids,
            bank: None,
            host_approved: 0,
            has_host,
            departed: Vec::new(),
        }
    }

    /// Makes chips real: stacks carry over, people sit down with no chips,
    /// and they ask the host for chips (see [`Command::RequestChips`]). A
    /// stack after a request must be from `min` to `max` chips. The host
    /// keeps the stack the table was made with.
    pub fn with_bank(mut self, min: u64, max: u64) -> Result<Self, PlayError> {
        if min == 0 || max < min {
            return Err(PlayError::InvalidSetup);
        }
        self.table.set_top_up(false);
        // Empty seats hold no chips (seat 0 too, without a host).
        let first = usize::from(self.has_host);
        for seat in first..self.table.num_seats() {
            if self.table.seat(seat).is_empty() {
                self.table.set_stack(seat, 0)?;
            }
        }
        self.bank = Some(Bank { min, max });
        Ok(self)
    }

    /// The buy-in limits, with a bank.
    pub fn buy_in_limits(&self) -> Option<(u64, u64)> {
        self.bank.map(|b| (b.min, b.max))
    }

    /// Requests for chips waiting for the host, oldest seat first.
    pub fn chip_requests(&self) -> Vec<ChipRequest> {
        let mut out: Vec<ChipRequest> = self
            .players
            .iter()
            .filter(|p| p.requested > 0 && !p.leaving)
            .map(|p| ChipRequest {
                seat: p.seat,
                name: p.name.clone(),
                amount: p.requested,
                stack: self.table.stack(p.seat) + p.approved,
            })
            .collect();
        out.sort_by_key(|r| r.seat);
        out
    }

    /// Approves the request from `seat`: the chips are added now, or before
    /// the next hand if the player is in this one.
    pub fn approve_chips(&mut self, seat: usize, now: u64) -> Vec<Outgoing> {
        let Some(i) = self.players.iter().position(|p| p.seat == seat) else {
            return Vec::new();
        };
        let amount = std::mem::take(&mut self.players[i].requested);
        if amount == 0 {
            return Vec::new();
        }
        // In this hand, or not seated until the next one: added then.
        if self.playing(seat) || self.players[i].pending {
            self.players[i].approved += amount;
        } else {
            let _ = self.table.add_chips(seat, amount);
        }
        self.changed(now)
    }

    /// Denies the request from `seat`.
    pub fn deny_chips(&mut self, seat: usize, now: u64) -> Vec<Outgoing> {
        let Some(p) = self.players.iter_mut().find(|p| p.seat == seat) else {
            return Vec::new();
        };
        if p.requested == 0 {
            return Vec::new();
        }
        p.requested = 0;
        self.changed(now)
    }

    /// The host adds chips to their own seat, within the limits: now, or
    /// before the next hand if they're in this one.
    pub fn host_chips(&mut self, amount: u64, now: u64) -> Result<Vec<Outgoing>, PlayError> {
        if !self.has_host {
            return Err(PlayError::IllegalAction);
        }
        let stack = self.table.stack(0) + self.host_approved;
        self.check_request(stack, amount)?;
        if self.playing(0) {
            self.host_approved += amount;
        } else {
            self.table.add_chips(0, amount)?;
        }
        Ok(self.changed(now))
    }

    /// Chips the host has had approved for the next hand.
    pub fn host_pending_chips(&self) -> u64 {
        self.host_approved
    }

    /// Whether `seat` is in the hand being played.
    fn playing(&self, seat: usize) -> bool {
        self.table.in_hand() && self.table.hand_index(seat).is_some()
    }

    /// Whether a stack of `stack` may ask for `amount` more.
    fn check_request(&self, stack: u64, amount: u64) -> Result<(), PlayError> {
        let bank = self.bank.ok_or(PlayError::IllegalAction)?;
        let after = stack + amount;
        if amount == 0 || after < bank.min || after > bank.max {
            return Err(PlayError::IllegalAction);
        }
        Ok(())
    }

    pub fn table(&self) -> &Table {
        &self.table
    }

    /// The current state number; it goes up whenever anything changes.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// The host's own view (seat 0). Without a host seat, a spectator's view
    /// from seat 0: no one's cards until they're shown down, and nothing to
    /// act on.
    pub fn host_view(&self) -> TableView {
        let mut view = self.table.view(0);
        if !self.has_host {
            view.legal = None;
            let s = &mut view.seats[0];
            if !(view.showdown && !s.folded) {
                s.cards = None;
            }
        }
        view
    }

    /// People at the table and their chips (see [`SeatedPlayer`]).
    pub fn seated(&self) -> Vec<SeatedPlayer> {
        self.players
            .iter()
            .map(|p| SeatedPlayer {
                client_id: p.client_id.clone(),
                seat: p.seat,
                chips: if p.pending {
                    0
                } else {
                    self.table.stack(p.seat)
                } + p.approved,
                pending: p.pending,
                leaving: p.leaving,
            })
            .collect()
    }

    /// People who left with chips since the last call, oldest first.
    pub fn take_departures(&mut self) -> Vec<Departure> {
        std::mem::take(&mut self.departed)
    }

    /// Time left for the person acting, if any.
    pub fn turn_ms_left(&self, now: u64) -> Option<u64> {
        self.deadline.map(|d| d.saturating_sub(now))
    }

    /// Whether the seat to act plays without waiting for a person, so the
    /// host should call [`Self::advance`] (after a short pause, for pacing).
    pub fn auto_to_act(&self) -> bool {
        self.table.auto_to_act()
    }

    /// Seats people can still take.
    pub fn open_seats(&self) -> usize {
        (0..self.open.len()).filter(|&s| self.seat_free(s)).count()
    }

    fn seat_free(&self, s: usize) -> bool {
        self.open[s] && !self.players.iter().any(|p| p.seat == s)
    }

    /// Handles a command from `client_id`.
    pub fn handle(&mut self, client_id: &str, command: Command, now: u64) -> Vec<Outgoing> {
        let reject = |reason: &str| {
            vec![Outgoing {
                to: client_id.to_string(),
                update: Update::Rejected {
                    reason: reason.to_string(),
                },
            }]
        };
        let idx = self.players.iter().position(|p| p.client_id == client_id);
        match (command, idx) {
            (Command::Join { .. }, Some(i)) => {
                // Reconnecting: same seat, back in action.
                let p = &mut self.players[i];
                p.connected = true;
                p.timeouts = 0;
                let seat = p.seat;
                let pending = p.pending;
                if !pending {
                    self.table.seat_mut(seat).away = false;
                }
                self.welcome_and_update(client_id, seat, now)
            }
            (Command::Join { name }, None) => {
                let name: String = name.trim().chars().take(MAX_NAME).collect();
                // Someone who closed their tab comes back on a new connection:
                // a disconnected player with the same name gets their seat back.
                if let Some(i) = self.players.iter().position(|p| {
                    !p.connected
                        && !p.leaving
                        && !name.is_empty()
                        && p.name.eq_ignore_ascii_case(&name)
                }) {
                    self.players[i].client_id = client_id.to_string();
                    return self.handle(client_id, Command::Join { name }, now);
                }
                let Some(seat) = (0..self.open.len()).find(|&s| self.seat_free(s)) else {
                    return reject("the table is full");
                };
                let name = if name.is_empty() {
                    format!("Player {seat}")
                } else {
                    name
                };
                let pending = self.table.in_hand();
                self.players.push(Player {
                    client_id: client_id.to_string(),
                    name: name.clone(),
                    seat,
                    connected: true,
                    timeouts: 0,
                    pending,
                    leaving: false,
                    missed: 0,
                    requested: 0,
                    approved: 0,
                });
                if !pending {
                    self.take_seat(seat, name);
                }
                self.welcome_and_update(client_id, seat, now)
            }
            (_, None) => reject("join the table first"),
            (Command::Act { seq, kind, amount }, Some(i)) => {
                let p = &self.players[i];
                if p.pending || p.leaving {
                    return reject("you're not in this hand");
                }
                if seq != self.seq {
                    return reject("stale action");
                }
                let Some(action) = parse_action(&kind, amount) else {
                    return reject("unknown action");
                };
                let seat = p.seat;
                match self.table.act(seat, action) {
                    Ok(()) => {
                        self.players[i].timeouts = 0;
                        self.changed(now)
                    }
                    Err(_) => reject("illegal action"),
                }
            }
            (Command::SitIn, Some(i)) => {
                let p = &mut self.players[i];
                p.timeouts = 0;
                let seat = p.seat;
                if !p.pending && !p.leaving {
                    self.table.seat_mut(seat).away = false;
                }
                self.changed(now)
            }
            (Command::Leave, Some(i)) => {
                self.players[i].connected = false;
                self.remove_player(i);
                self.changed(now)
            }
            (Command::RequestChips { amount }, Some(i)) => {
                let Some(bank) = self.bank else {
                    return reject("this table has no chip requests");
                };
                let p = &self.players[i];
                if amount > 0 {
                    let stack = self.table.stack(p.seat) + p.approved;
                    if self.check_request(stack, amount).is_err() {
                        let reason = if stack >= bank.max {
                            format!("you already have the most allowed ({} chips)", bank.max)
                        } else {
                            format!(
                                "ask for {} to {} chips",
                                bank.min.saturating_sub(stack).max(1),
                                bank.max - stack
                            )
                        };
                        return reject(&reason);
                    }
                }
                self.players[i].requested = amount;
                self.changed(now)
            }
        }
    }

    /// The host removes whoever sits in `seat`: they fold out of a hand in
    /// progress and the seat is free from the next hand. They get no more
    /// updates (they can join again).
    pub fn remove(&mut self, seat: usize, now: u64) -> Vec<Outgoing> {
        let Some(i) = self.players.iter().position(|p| p.seat == seat) else {
            return Vec::new();
        };
        self.players[i].connected = false;
        self.remove_player(i);
        self.changed(now)
    }

    /// Gives player `i`'s seat back: now between hands, or once this hand is
    /// over (folding at their turn meanwhile).
    fn remove_player(&mut self, i: usize) {
        let p = &mut self.players[i];
        p.leaving = true;
        let seat = p.seat;
        if self.table.in_hand() && !p.pending {
            self.table.seat_mut(seat).away = true;
        } else {
            self.release_seat(i);
        }
    }

    /// A player's connection dropped: their seat checks or folds until they
    /// come back.
    pub fn disconnected(&mut self, client_id: &str, now: u64) -> Vec<Outgoing> {
        let Some(p) = self.players.iter_mut().find(|p| p.client_id == client_id) else {
            return Vec::new();
        };
        p.connected = false;
        let seat = p.seat;
        if !p.pending {
            self.table.seat_mut(seat).away = true;
        }
        self.changed(now)
    }

    /// The host's own action (seat 0).
    pub fn host_act(
        &mut self,
        kind: &str,
        amount: u64,
        now: u64,
    ) -> Result<Vec<Outgoing>, PlayError> {
        // Without a host seat, seat 0 is someone else's.
        if !self.has_host {
            return Err(PlayError::IllegalAction);
        }
        let action = parse_action(kind, amount).ok_or(PlayError::IllegalAction)?;
        self.table.act(0, action)?;
        Ok(self.changed(now))
    }

    /// Lets a bot or an away player act. Returns no updates if it's a
    /// person's turn.
    pub fn advance(&mut self, now: u64) -> Result<Vec<Outgoing>, PlayError> {
        if self.table.advance()? {
            Ok(self.changed(now))
        } else {
            Ok(Vec::new())
        }
    }

    /// Deals the next hand: people who joined take their seats, people who
    /// left give theirs back, people who are away sit out, and someone
    /// disconnected for [`DROP_AFTER_HANDS`] hands is removed. Fails, dealing
    /// nothing, with fewer than two players in.
    pub fn new_hand(&mut self, now: u64) -> Result<Vec<Outgoing>, PlayError> {
        if self.table.in_hand() {
            return Err(PlayError::IllegalAction);
        }
        while let Some(i) = self
            .players
            .iter()
            .position(|p| p.leaving || p.missed >= DROP_AFTER_HANDS)
        {
            self.release_seat(i);
        }
        // Chips approved during the last hand.
        let host_chips = std::mem::take(&mut self.host_approved);
        if host_chips > 0 {
            let _ = self.table.add_chips(0, host_chips);
        }
        for i in 0..self.players.len() {
            let seat = self.players[i].seat;
            // (Someone sitting down gets theirs in take_seat.)
            if !self.players[i].pending {
                let chips = std::mem::take(&mut self.players[i].approved);
                if chips > 0 {
                    let _ = self.table.add_chips(seat, chips);
                }
            }
            if self.players[i].pending {
                self.players[i].pending = false;
                self.take_seat(seat, self.players[i].name.clone());
                if !self.players[i].connected {
                    self.table.seat_mut(seat).away = true;
                }
            }
            // Away (disconnected, or timed out twice): not dealt in.
            let away = self.table.seat(seat).away;
            self.table.seat_mut(seat).sitting_out = away;
        }
        self.table.new_hand()?;
        for p in &mut self.players {
            p.missed = if p.connected { 0 } else { p.missed + 1 };
        }
        Ok(self.changed(now))
    }

    /// People seated, the host included, and how many of them would be dealt
    /// into the next hand.
    pub fn players_in(&self) -> usize {
        self.table.players_in()
    }

    /// Runs the turn clock: a person out of time checks or folds, and after
    /// two timeouts in a row they're away until they sit back in.
    pub fn tick(&mut self, now: u64) -> Vec<Outgoing> {
        let Some(deadline) = self.deadline else {
            return Vec::new();
        };
        if now < deadline {
            return Vec::new();
        }
        let Some(seat) = self.table.to_act() else {
            return Vec::new();
        };
        if self.table.act_default(seat).is_err() {
            return Vec::new();
        }
        if let Some(p) = self.players.iter_mut().find(|p| p.seat == seat) {
            p.timeouts += 1;
            if p.timeouts >= 2 {
                self.table.seat_mut(seat).away = true;
            }
        }
        self.changed(now)
    }

    fn take_seat(&mut self, seat: usize, name: String) {
        let s = self.table.seat_mut(seat);
        s.name = name;
        s.id = format!("player{seat}");
        s.human = true;
        s.away = false;
        s.sitting_out = false;
        // With a bank, people sit down with no chips and ask for some.
        if self.bank.is_some() {
            let _ = self.table.set_stack(seat, 0);
            let i = self.players.iter().position(|p| p.seat == seat);
            if let Some(chips) = i.map(|i| std::mem::take(&mut self.players[i].approved)) {
                let _ = self.table.add_chips(seat, chips);
            }
        } else {
            let _ = self.table.reset_stack(seat);
        }
    }

    fn release_seat(&mut self, i: usize) {
        let p = self.players.remove(i);
        if self.bank.is_some() {
            let stack = if p.pending {
                0
            } else {
                self.table.stack(p.seat)
            };
            self.departed.push(Departure {
                client_id: p.client_id.clone(),
                seat: p.seat,
                chips: stack + p.approved,
            });
        }
        if p.pending {
            return;
        }
        let s = self.table.seat_mut(p.seat);
        if s.bot.is_some() {
            s.name = s.bot_name.clone();
            s.id = self.bot_ids[p.seat].clone();
        } else {
            // Back to an empty seat.
            s.name.clear();
            s.id.clear();
        }
        s.human = false;
        s.away = false;
        s.sitting_out = false;
        // Their chips leave with them.
        if self.bank.is_some() {
            let _ = self.table.set_stack(p.seat, 0);
        } else {
            let _ = self.table.reset_stack(p.seat);
        }
    }

    fn welcome_and_update(&mut self, client_id: &str, seat: usize, now: u64) -> Vec<Outgoing> {
        let mut out = vec![Outgoing {
            to: client_id.to_string(),
            update: Update::Welcome { seat },
        }];
        out.extend(self.changed(now));
        out
    }

    /// Something changed: restart the clock if the turn moved, and send
    /// every connected player their view.
    fn changed(&mut self, now: u64) -> Vec<Outgoing> {
        // Someone who left (or was removed) during a hand is gone once it's
        // over.
        if !self.table.in_hand() {
            while let Some(i) = self.players.iter().position(|p| p.leaving) {
                self.release_seat(i);
            }
        }
        self.seq += 1;
        let turn = self
            .table
            .hand()
            .filter(|h| !h.is_complete())
            .map(|h| (self.table.hand_number(), h.events().len()));
        if turn != self.turn {
            self.turn = turn;
            let person = self.table.to_act().is_some() && !self.table.auto_to_act();
            self.deadline = (person && self.turn_ms > 0).then(|| now + self.turn_ms);
        }
        self.updates(now)
    }

    /// Every connected player's current view.
    pub fn updates(&self, now: u64) -> Vec<Outgoing> {
        let turn_ms_left = self.turn_ms_left(now);
        self.players
            .iter()
            .filter(|p| p.connected)
            .map(|p| {
                let mut view = self.table.view(p.seat);
                if p.pending {
                    // The bot still plays this hand; its cards stay hidden.
                    view.seats[0].cards = None;
                    view.legal = None;
                }
                let chips = self.bank.map(|b| ChipsView {
                    min: b.min,
                    max: b.max,
                    requested: p.requested,
                    approved: p.approved,
                });
                Outgoing {
                    to: p.client_id.clone(),
                    update: Update::State {
                        seq: self.seq,
                        view: Box::new(view),
                        turn_ms_left,
                        chips,
                    },
                }
            })
            .collect()
    }
}

fn parse_action(kind: &str, amount: u64) -> Option<Action> {
    Some(match kind {
        "fold" => Action::Fold,
        "check" => Action::Check,
        "call" => Action::Call,
        "bet" => Action::Bet(amount),
        "raise" => Action::Raise(amount),
        "allin" => Action::AllIn,
        _ => return None,
    })
}
