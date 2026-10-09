//! Hosting a [`Tournament`] for people on other devices (#142), as
//! [`TableHost`](crate::TableHost) hosts one table: feed it each player's
//! [`Command`]s and send each [`Outgoing`] it returns to its recipient.
//!
//! - **Views:** each player gets their own seat's view of the table they're
//!   at now (no one else's unshown cards), and acts with the same commands
//!   as at a club table. A player moved to another table gets a view of it
//!   at once.
//! - **Absent players are blinded off:** everyone starts absent until they
//!   join, and a player who disconnects, leaves or times out twice in a row
//!   is absent again. They're still dealt in, post blinds and antes, and
//!   check or fold when it's their turn ([`TournamentHost::advance`]).
//! - **The clock:** times are milliseconds from any clock the caller
//!   chooses. [`TournamentHost::tick`] runs every table's turn clock; the
//!   caller paces the next hands ([`TournamentHost::new_hands`]) and raises
//!   the blinds ([`TournamentHost::set_level`]).
//! - **Results:** each finished hand is taken in at once (eliminations with
//!   their places, table moves, broken tables, the winner), kept for
//!   [`TournamentHost::take_events`].
//! - **Secure deals** at every table ([`Tournament::with_secure_deals`]).

use std::collections::{HashMap, HashSet};

use crate::{
    Command, Entrant, Finish, Move, Outgoing, PlayError, Standing, TableView, Tournament,
    TournamentConfig, Update, host::parse_action,
};

/// What happened since the last [`TournamentHost::take_events`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TournamentEvents {
    /// Players knocked out, in the order they went out (worst place first).
    pub busted: Vec<Finish>,
    /// Players moved to other tables, in order.
    pub moves: Vec<Move>,
    /// Tables closed.
    pub broken: Vec<usize>,
    /// Set once one player has all the chips.
    pub winner: Option<Finish>,
    /// The blind level, when it changed.
    pub level: Option<usize>,
}

/// Runs every table of a [`Tournament`] for remote players.
pub struct TournamentHost {
    t: Tournament,
    turn_ms: u64,
    seq: u64,
    /// People in the tournament (bots aren't hosted here), and those connected.
    people: HashSet<String>,
    connected: HashSet<String>,
    /// Turns in a row each person ran out of time.
    timeouts: HashMap<String, u32>,
    /// Per table id: the turn on the clock (hand number, events so far) and
    /// when it runs out.
    turns: HashMap<usize, (u64, usize)>,
    deadlines: HashMap<usize, u64>,
    events: TournamentEvents,
}

impl TournamentHost {
    /// A tournament for `players` ((id, name) pairs), seated at random, with
    /// `turn_ms` to act (0 for no clock). Everyone is absent until they join.
    pub fn new(
        config: TournamentConfig,
        players: Vec<(String, String)>,
        turn_ms: u64,
    ) -> Result<Self, PlayError> {
        let entrants = players
            .iter()
            .map(|(id, name)| Entrant::person(id.clone(), name.clone()))
            .collect();
        let t = Tournament::new(config, entrants)?.with_secure_deals();
        let mut h = Self {
            t,
            turn_ms,
            seq: 0,
            people: players.into_iter().map(|(id, _)| id).collect(),
            connected: HashSet::new(),
            timeouts: HashMap::new(),
            turns: HashMap::new(),
            deadlines: HashMap::new(),
            events: TournamentEvents::default(),
        };
        let ids: Vec<String> = h.people.iter().cloned().collect();
        for id in ids {
            h.t.set_absent(&id, true)?;
        }
        Ok(h)
    }

    /// The tournament itself, to read (standings, tables, finishes).
    pub fn tournament(&self) -> &Tournament {
        &self.t
    }

    /// The current state number; it goes up whenever anything changes.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Everyone still in, biggest stack first.
    pub fn standings(&self) -> Vec<Standing> {
        self.t.standings()
    }

    /// What happened since the last call: eliminations, moves, broken
    /// tables, the winner and level changes.
    pub fn take_events(&mut self) -> TournamentEvents {
        std::mem::take(&mut self.events)
    }

    /// A late registration or a re-entry for `id` (out, or new). They're
    /// absent until they join, unless they're connected now.
    pub fn add_entry(
        &mut self,
        id: &str,
        name: &str,
        now: u64,
    ) -> Result<Vec<Outgoing>, PlayError> {
        self.t.add_entry(Entrant::person(id, name))?;
        self.people.insert(id.to_string());
        self.t.set_absent(id, !self.connected.contains(id))?;
        Ok(self.changed(now))
    }

    /// A message from player `id`.
    pub fn handle(&mut self, id: &str, command: Command, now: u64) -> Vec<Outgoing> {
        let reject = |reason: &str| {
            vec![Outgoing {
                to: id.to_string(),
                update: Update::Rejected {
                    reason: reason.to_string(),
                },
            }]
        };
        if !self.people.contains(id) {
            return reject("not in this tournament");
        }
        let Some(at) = self.t.find(id) else {
            return reject("you're out of the tournament");
        };
        match command {
            // Here (again): no longer blinded off.
            Command::Join { .. } | Command::SitIn { .. } => {
                self.connected.insert(id.to_string());
                self.timeouts.remove(id);
                let _ = self.t.set_absent(id, false);
                let mut out = vec![Outgoing {
                    to: id.to_string(),
                    update: Update::Welcome { seat: at.seat },
                }];
                out.extend(self.changed(now));
                out
            }
            // Nobody leaves a tournament: they're blinded off instead.
            Command::SitOut | Command::Leave => {
                let _ = self.t.set_absent(id, true);
                self.changed(now)
            }
            Command::RequestChips { .. } => reject("chips can't be bought in a tournament"),
            Command::Act { seq, kind, amount } => {
                if seq != self.seq {
                    return reject("that was for an earlier state");
                }
                let Some(action) = parse_action(&kind, amount) else {
                    return reject("not an action");
                };
                let table = self.t.table_mut(at.table).expect("live table");
                if table.to_act() != Some(at.seat) {
                    return reject("not your turn");
                }
                if let Err(e) = table.act(at.seat, action) {
                    return reject(&e.to_string());
                }
                self.timeouts.remove(id);
                self.changed(now)
            }
        }
    }

    /// Player `id`'s connection dropped: they're blinded off until they're back.
    pub fn disconnected(&mut self, id: &str, now: u64) -> Vec<Outgoing> {
        self.connected.remove(id);
        if self.t.set_absent(id, true).is_err() {
            return Vec::new();
        }
        self.changed(now)
    }

    /// Whether a seat somewhere plays without waiting for a person (an
    /// absent player's turn): call [`Self::advance`].
    pub fn auto_to_act(&self) -> bool {
        self.t
            .table_ids()
            .into_iter()
            .any(|id| self.t.table(id).is_some_and(|t| t.auto_to_act()))
    }

    /// Absent players check or fold, one turn at each table where it's theirs.
    pub fn advance(&mut self, now: u64) -> Result<Vec<Outgoing>, PlayError> {
        let mut any = false;
        for id in self.t.table_ids() {
            if let Some(table) = self.t.table_mut(id) {
                any |= table.advance()?;
            }
        }
        Ok(if any { self.changed(now) } else { Vec::new() })
    }

    /// Runs the turn clocks: a person out of time checks or folds, and after
    /// two timeouts in a row they're absent until they're back.
    pub fn tick(&mut self, now: u64) -> Vec<Outgoing> {
        let due: Vec<usize> = self
            .deadlines
            .iter()
            .filter(|&(_, &d)| now >= d)
            .map(|(&id, _)| id)
            .collect();
        if due.is_empty() {
            return Vec::new();
        }
        for id in due {
            self.deadlines.remove(&id);
            let Some(table) = self.t.table_mut(id) else {
                continue;
            };
            let Some(seat) = table.to_act() else { continue };
            if table.act_default(seat).is_err() {
                continue;
            }
            let who = table.seat(seat).id.clone();
            let n = self.timeouts.entry(who.clone()).or_insert(0);
            *n += 1;
            if *n >= 2 {
                let _ = self.t.set_absent(&who, true);
            }
        }
        self.changed(now)
    }

    /// Deals the next hand at every table that may deal now (see
    /// [`Tournament::can_deal`]).
    pub fn new_hands(&mut self, now: u64) -> Result<Vec<Outgoing>, PlayError> {
        let ids: Vec<usize> = self
            .t
            .table_ids()
            .into_iter()
            .filter(|&id| self.t.can_deal(id))
            .collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        for id in ids {
            self.t.new_hand(id)?;
        }
        Ok(self.changed(now))
    }

    /// Whether some table could deal its next hand now.
    pub fn can_deal(&self) -> bool {
        self.t.table_ids().into_iter().any(|id| self.t.can_deal(id))
    }

    /// Moves to blind level `level`; each table uses it from its next hand.
    pub fn set_level(&mut self, level: usize) {
        self.t.set_level(level);
        self.events.level = Some(self.t.level());
    }

    /// Time left for whoever is acting at table `id`, if a person is on the clock.
    pub fn turn_ms_left(&self, id: usize, now: u64) -> Option<u64> {
        self.deadlines.get(&id).map(|d| d.saturating_sub(now))
    }

    /// Player `id`'s view of the table they're at now (they're seat 0), if
    /// they're still in.
    pub fn view_for(&self, id: &str) -> Option<TableView> {
        let at = self.t.find(id)?;
        Some(self.t.table(at.table)?.view(at.seat))
    }

    /// A spectator's view of table `id`: no one's cards until they're shown
    /// down, and nothing to act on.
    pub fn table_view(&self, id: usize) -> Option<TableView> {
        let mut view = self.t.table(id)?.view(0);
        view.legal = None;
        let s = &mut view.seats[0];
        if !(view.showdown && !s.folded) {
            s.cards = None;
        }
        Some(view)
    }

    /// Something changed: take in finished hands (eliminations, moves),
    /// restart the clock where the turn moved, and send every connected
    /// player still in their view.
    fn changed(&mut self, now: u64) -> Vec<Outgoing> {
        for id in self.t.table_ids() {
            let done = self
                .t
                .table(id)
                .and_then(|t| t.hand())
                .is_some_and(|h| h.is_complete());
            // Already taken in: finish_hand refuses a hand twice.
            if done && let Ok(report) = self.t.finish_hand(id) {
                self.events.busted.extend(report.busted.into_iter().rev());
                self.events.moves.extend(report.moves);
                self.events.broken.extend(report.broken);
                if report.winner.is_some() {
                    self.events.winner = report.winner;
                }
            }
        }
        // Clocks, for the tables still in play.
        let live: HashSet<usize> = self.t.table_ids().into_iter().collect();
        self.turns.retain(|id, _| live.contains(id));
        self.deadlines.retain(|id, _| live.contains(id));
        for &id in &live {
            let table = self.t.table(id).expect("live table");
            let turn = table
                .hand()
                .filter(|h| !h.is_complete())
                .map(|h| (table.hand_number(), h.events().len()));
            if turn != self.turns.get(&id).copied() {
                match turn {
                    Some(k) => self.turns.insert(id, k),
                    None => self.turns.remove(&id),
                };
                let person = table.to_act().is_some() && !table.auto_to_act();
                if person && self.turn_ms > 0 {
                    self.deadlines.insert(id, now + self.turn_ms);
                } else {
                    self.deadlines.remove(&id);
                }
            }
        }
        self.seq += 1;
        let mut ids: Vec<&String> = self.connected.iter().collect();
        ids.sort();
        ids.into_iter()
            .filter_map(|id| {
                let at = self.t.find(id)?;
                let view = self.t.table(at.table)?.view(at.seat);
                Some(Outgoing {
                    to: id.clone(),
                    update: Update::State {
                        seq: self.seq,
                        view: Box::new(view),
                        turn_ms_left: self.turn_ms_left(at.table, now),
                        chips: None,
                        me: None,
                    },
                })
            })
            .collect()
    }
}
