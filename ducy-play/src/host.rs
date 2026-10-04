//! Hosting a [`Table`] for people playing from other devices.
//!
//! The host keeps the whole game state; players only send [`Command`]s and
//! receive [`Update`]s built for them, so no one ever receives another
//! player's cards before they're shown down. [`TableHost`] doesn't do any
//! networking: feed it each player's commands and send each [`Outgoing`] it
//! returns to its recipient over whatever connection the app uses.
//!
//! Seat 0 is the host's own seat. Seats marked *open* are played by their bot
//! until someone joins; a person takes over at the start of the next hand,
//! and the bot takes the seat back when they leave. Times are milliseconds
//! from any clock the caller chooses, which keeps the host deterministic.

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
    /// Give the seat back to its bot.
    Leave,
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
    },
    /// Your command was refused.
    Rejected { reason: String },
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
}

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
        if open
            .iter()
            .enumerate()
            .any(|(s, &o)| o && table.seat(s).bot.is_none())
        {
            return Err(PlayError::InvalidPlayerCount);
        }
        let bot_ids = (0..table.num_seats())
            .map(|s| table.seat(s).id.clone())
            .collect();
        Ok(Self {
            table,
            open,
            players: Vec::new(),
            seq: 0,
            turn_ms,
            deadline: None,
            turn: None,
            bot_ids,
        })
    }

    pub fn table(&self) -> &Table {
        &self.table
    }

    /// The current state number; it goes up whenever anything changes.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// The host's own view (seat 0).
    pub fn host_view(&self) -> TableView {
        self.table.view(0)
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
                let p = &mut self.players[i];
                p.leaving = true;
                p.connected = false;
                let seat = p.seat;
                if self.table.in_hand() {
                    self.table.seat_mut(seat).away = true;
                } else {
                    self.release_seat(i);
                }
                self.changed(now)
            }
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
    /// left give theirs back to the bots.
    pub fn new_hand(&mut self, now: u64) -> Result<Vec<Outgoing>, PlayError> {
        if self.table.in_hand() {
            return Err(PlayError::IllegalAction);
        }
        while let Some(i) = self.players.iter().position(|p| p.leaving) {
            self.release_seat(i);
        }
        for i in 0..self.players.len() {
            if self.players[i].pending {
                self.players[i].pending = false;
                let seat = self.players[i].seat;
                self.take_seat(seat, self.players[i].name.clone());
                if !self.players[i].connected {
                    self.table.seat_mut(seat).away = true;
                }
            }
        }
        self.table.new_hand()?;
        Ok(self.changed(now))
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
        let _ = self.table.reset_stack(seat);
    }

    fn release_seat(&mut self, i: usize) {
        let p = self.players.remove(i);
        if p.pending {
            return;
        }
        let s = self.table.seat_mut(p.seat);
        if let Some(bot) = &s.bot {
            s.name = bot.name().to_string();
            s.id = self.bot_ids[p.seat].clone();
        }
        s.human = false;
        s.away = false;
        let _ = self.table.reset_stack(p.seat);
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
                Outgoing {
                    to: p.client_id.clone(),
                    update: Update::State {
                        seq: self.seq,
                        view: Box::new(view),
                        turn_ms_left,
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
