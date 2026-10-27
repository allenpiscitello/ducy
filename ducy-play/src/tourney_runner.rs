//! One club tournament's loop: a [`TournamentHost`] and everything that
//! happens there with time. The level clock with its breaks, the next hands,
//! absent players' moves, the turn clocks, and which tables have just
//! finished a hand. It has no timers and no I/O: every [`TourneyEvent`]
//! comes with the time, and the [`TourneyStep`] it returns says what
//! happened and when to wake it next. The club keeps registration,
//! knockouts, prizes, saving and the transport.
//!
//! Ported from ducy-web's `tourney-runner.js` (ducy-web #458): the same
//! events in, the same steps out.

use std::collections::HashMap;

use crate::{
    Command, Outgoing, PlayError, Standing, TableView, TournamentEvents, TournamentHost, Update,
};

/// Absent players' moves come after this long.
pub const BOT_PACE_MS: u64 = 700;
/// The next hands are dealt this long after the tables can.
pub const NEXT_HAND_MS: u64 = 2500;

/// One table of a tournament, as the club sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TableInfo {
    /// The table.
    pub id: usize,
    /// People at it.
    pub players: usize,
    /// A hand is being played.
    #[cfg_attr(feature = "serde", serde(rename = "inHand"))]
    pub in_hand: bool,
    /// Time left for whoever is acting, if a person is on the clock.
    #[cfg_attr(feature = "serde", serde(rename = "turnMsLeft"))]
    pub turn_ms_left: Option<u64>,
}

/// An engine result: what happened, the standings and tables, and what can
/// happen next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TourneyResult {
    /// Goes up whenever anything changes.
    pub seq: u64,
    /// Each player's update.
    pub out: Vec<Outgoing>,
    /// Busted players with places, moves, broken tables, the winner, a new level.
    pub events: TournamentEvents,
    /// Everyone still in.
    pub standings: Vec<Standing>,
    /// The tables.
    pub tables: Vec<TableInfo>,
    /// The blind level (from 0).
    pub level: usize,
    /// Players still in.
    pub players_left: usize,
    /// Every chip in play.
    pub total_chips: u64,
    /// Hand for hand (on the bubble).
    pub hand_for_hand: bool,
    /// One player has every chip.
    pub over: bool,
    /// An absent player should act next.
    pub auto_to_act: bool,
    /// Tables can deal their next hands.
    pub can_deal: bool,
}

impl TourneyResult {
    /// The result of a call on `host` that sent `out`, at `now`.
    pub fn of(host: &mut TournamentHost, out: Vec<Outgoing>, now: u64) -> Self {
        let events = host.take_events();
        let t = host.tournament();
        let tables = t
            .table_ids()
            .into_iter()
            .map(|id| TableInfo {
                id,
                players: t.players_at(id),
                in_hand: t.table(id).is_some_and(|x| x.in_hand()),
                turn_ms_left: host.turn_ms_left(id, now),
            })
            .collect();
        let t = host.tournament();
        Self {
            seq: host.seq(),
            out,
            events,
            standings: host.standings(),
            tables,
            level: t.level(),
            players_left: t.players_left(),
            total_chips: t.total_chips(),
            hand_for_hand: t.hand_for_hand(),
            over: t.is_over(),
            auto_to_act: host.auto_to_act(),
            can_deal: host.can_deal(),
        }
    }
}

/// What happens to a tournament, with the time it happens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TourneyEvent {
    /// The level clock starts: at the config's step, with its time left
    /// (resuming), else at the first level.
    Begin,
    /// A player's table command (`None`: one that can't be read, refused).
    Cmd {
        /// Who.
        id: String,
        /// What.
        cmd: Option<Command>,
    },
    /// A player dropped: blinded off until they're back.
    Disconnected {
        /// Who.
        id: String,
    },
    /// A late registration or re-entry, seated now.
    AddEntry {
        /// Who.
        id: String,
        /// Their name.
        name: String,
    },
    /// The tournament as it is.
    State,
    /// The time the last step asked for came.
    Wake,
}

/// What a step did, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TourneyAction {
    /// An engine result.
    Engine(Box<TourneyResult>),
    /// A table finished a hand: its spectator view.
    Hand {
        /// The table.
        table: usize,
        /// Its view (no one's cards until shown down).
        view: Box<TableView>,
    },
}

/// What an event did, the level clock, and when to wake the runner next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TourneyStep {
    /// What happened, in order.
    pub actions: Vec<TourneyAction>,
    /// When to send [`TourneyEvent::Wake`]; `None`: no need.
    pub wake: Option<u64>,
    /// The structure's step being played (a level or a break).
    pub step: usize,
    /// When it ends; `None`: the last level, which stays.
    pub step_ends_at: Option<u64>,
    /// It's a break.
    pub on_break: bool,
    /// The blind level being played.
    pub level: usize,
}

/// One step of a tournament's structure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StructureStep {
    /// How long it lasts.
    pub minutes: u64,
    /// A break, not a level.
    pub is_break: bool,
}

/// How a runner starts.
#[derive(Clone, Debug)]
pub struct TourneyConfig {
    /// The tournament's id, for messages.
    pub id: String,
    /// Its levels and breaks (a level's blinds are the engine's).
    pub structure: Vec<StructureStep>,
    /// Replaces every step's minutes (tests).
    pub level_ms: Option<u64>,
    /// See [`NEXT_HAND_MS`].
    pub next_hand_ms: u64,
    /// See [`BOT_PACE_MS`].
    pub bot_pace_ms: u64,
    /// Where a restored one's clock was: the step…
    pub step: usize,
    /// …its time left…
    pub step_left_ms: Option<u64>,
    /// …and the level.
    pub level: usize,
    /// Report finished hands.
    pub hands: bool,
}

impl Default for TourneyConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            structure: Vec::new(),
            level_ms: None,
            next_hand_ms: NEXT_HAND_MS,
            bot_pace_ms: BOT_PACE_MS,
            step: 0,
            step_left_ms: None,
            level: 0,
            hands: false,
        }
    }
}

#[derive(Default)]
struct Due {
    level: Option<u64>,
    tick: Option<u64>,
    advance: Option<u64>,
    deal: Option<u64>,
}

/// One club tournament's loop. See the module docs.
pub struct TourneyRunner {
    host: TournamentHost,
    config: TourneyConfig,
    due: Due,
    now: u64,
    actions: Vec<TourneyAction>,
    cur: usize,
    on_break: bool,
    level: usize,
    step_ends_at: Option<u64>,
    over: bool,
    seen: HashMap<usize, u64>,
}

impl TourneyRunner {
    /// Runs `host` (a new tournament, or one restored).
    pub fn new(host: TournamentHost, config: TourneyConfig) -> Self {
        Self {
            host,
            cur: config.step,
            level: config.level,
            config,
            due: Due::default(),
            now: 0,
            actions: Vec::new(),
            on_break: false,
            step_ends_at: None,
            over: false,
            seen: HashMap::new(),
        }
    }

    /// The tournament.
    pub fn host(&self) -> &TournamentHost {
        &self.host
    }

    /// Takes in `ev` at time `now`. Fails only for a late entry the
    /// tournament can't take.
    pub fn event(&mut self, ev: TourneyEvent, now: u64) -> Result<TourneyStep, PlayError> {
        self.now = now;
        self.actions.clear();
        match ev {
            TourneyEvent::Begin => self.start_step(self.config.step, self.config.step_left_ms),
            TourneyEvent::Cmd { id, cmd } => {
                let out = match cmd {
                    Some(c) => self.host.handle(&id, c, now),
                    None => vec![Outgoing {
                        to: id,
                        update: Update::Rejected {
                            reason: "bad message".to_string(),
                        },
                    }],
                };
                self.took(out);
            }
            TourneyEvent::Disconnected { id } => {
                let out = self.host.disconnected(&id, now);
                self.took(out);
            }
            TourneyEvent::AddEntry { id, name } => {
                let out = self.host.add_entry(&id, &name, now)?;
                self.took(out);
            }
            TourneyEvent::State => self.took(Vec::new()),
            TourneyEvent::Wake => self.wake(),
        }
        let d = &self.due;
        Ok(TourneyStep {
            actions: std::mem::take(&mut self.actions),
            wake: [d.level, d.tick, d.advance, d.deal]
                .into_iter()
                .flatten()
                .min(),
            step: self.cur,
            step_ends_at: self.step_ends_at,
            on_break: self.on_break,
            level: self.level,
        })
    }

    fn took(&mut self, out: Vec<Outgoing>) {
        let res = TourneyResult::of(&mut self.host, out, self.now);
        if let Some(l) = res.events.level {
            self.level = l;
        }
        self.level = res.level;
        self.over = res.over || res.events.winner.is_some();
        let done: Vec<usize> = if self.config.hands {
            res.tables
                .iter()
                .filter(|x| !x.in_hand)
                .map(|x| x.id)
                .collect()
        } else {
            Vec::new()
        };
        let (auto, deal) = (res.auto_to_act, res.can_deal);
        let left = res.tables.iter().filter_map(|x| x.turn_ms_left).min();
        self.actions.push(TourneyAction::Engine(Box::new(res)));
        // Tables between hands: each one's last hand, once.
        for table in done {
            self.hand_done(table);
        }
        self.pace(auto, deal, left);
    }

    fn hand_done(&mut self, table: usize) {
        let Some(view) = self.host.table_view(table) else {
            return;
        };
        let n = view.hand_number;
        if !view.complete || n == 0 || self.seen.get(&table) == Some(&n) {
            return;
        }
        self.seen.insert(table, n);
        self.actions.push(TourneyAction::Hand {
            table,
            view: Box::new(view),
        });
    }

    /// What happens next: an absent player's move, the next hands (not
    /// during a break), and the turn clocks.
    fn pace(&mut self, auto: bool, deal: bool, left: Option<u64>) {
        let now = self.now;
        let d = &mut self.due;
        (d.advance, d.deal, d.tick) = (None, None, None);
        if self.over {
            d.level = None;
            return;
        }
        if auto {
            d.advance = Some(now + self.config.bot_pace_ms);
        }
        if deal && !self.on_break {
            d.deal = Some(now + self.config.next_hand_ms);
        }
        d.tick = left.map(|ms| now + ms + 50);
    }

    /// The level clock: step `i` of the structure (a level, or a break)
    /// starts now, or, resuming, goes on with `left_ms` to go. The last level
    /// stays once it's reached.
    fn start_step(&mut self, i: usize, left_ms: Option<u64>) {
        let steps = &self.config.structure;
        let Some(last) = steps.len().checked_sub(1) else {
            self.took(Vec::new());
            return;
        };
        self.cur = i.min(last);
        let s = steps[self.cur];
        self.on_break = s.is_break;
        let level = steps[..=self.cur]
            .iter()
            .filter(|x| !x.is_break)
            .count()
            .saturating_sub(1);
        let ms = left_ms
            .or(self.config.level_ms)
            .unwrap_or(s.minutes * 60_000);
        self.step_ends_at = (i < steps.len()).then_some(self.now + ms);
        self.due.level = (i < last || s.is_break).then_some(self.now + ms);
        if !self.on_break && level != self.level {
            self.host.set_level(level);
        }
        self.took(Vec::new());
    }

    fn wake(&mut self) {
        let now = self.now;
        let due = |slot: &mut Option<u64>| -> bool {
            if slot.is_some_and(|t| t <= now) {
                *slot = None;
                true
            } else {
                false
            }
        };
        if !self.over && due(&mut self.due.level) {
            self.start_step(self.cur + 1, None);
        }
        if !self.over && due(&mut self.due.tick) {
            let out = self.host.tick(now);
            self.took(out);
        }
        if !self.over
            && due(&mut self.due.advance)
            && let Ok(out) = self.host.advance(now)
        {
            self.took(out);
        }
        if !self.over
            && due(&mut self.due.deal)
            && let Ok(out) = self.host.new_hands(now)
        {
            self.took(out);
        }
    }
}
