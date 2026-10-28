//! One club table's loop: a [`TableHost`] and everything that happens at it
//! with time. The turn clock, the next hand, away players' moves, fair deals,
//! idle seats, the wait for people after a restart, and the clock that stops
//! while no one can be reached. It has no timers and no I/O: every
//! [`Event`] comes with the time, and the [`Step`] it returns says what
//! happened and when to wake it next. The club keeps the money, the members
//! and the transport.
//!
//! Ported from ducy-web's `table-runner.js` (ducy-web #452): the same events
//! in, the same steps out.

use std::collections::{HashMap, HashSet};

use crate::fair_deal::{Bytes32, Reveal, hand_seed};
use crate::{
    ChipRequest, Command, Departure, Outgoing, PlayError, Refund, SeatedPlayer, TableHost,
    TableView, Update,
};

/// The next hand is dealt this long after the last one ends.
pub const NEXT_HAND_MS: u64 = 1000;
/// An away player's move comes after this long.
pub const BOT_PACE_MS: u64 = 700;
/// How long players have to send a fair-deal commitment, then a reveal.
pub const FAIR_STEP_MS: u64 = 4000;
/// How long a deal may wait for a fair-deal round still under way before
/// going ahead without one.
pub const FAIR_WAIT_MS: u64 = 6000;
const FAIR_RETRY_MS: u64 = 300;
/// Fair-deal results kept per player, to send again when they come back.
const FAIR_KEEP: usize = 3;
/// A restored table waits this long for its people; then whoever isn't back
/// counts as disconnected, and the clocks run again.
pub const RESUME_GRACE_MS: u64 = 60_000;
/// Idle seats: how long without a hand before someone is asked or stood up.
pub const IDLE_MS: u64 = 10 * 60_000;
/// How long to answer "Still here?".
pub const ASK_MS: u64 = 2 * 60_000;
/// While the host can't reach anyone, or a restored table waits, no one is
/// stood up; the idle check looks again after this.
const IDLE_RECHECK_MS: u64 = 30_000;
/// A table with people at it ticks this often, for the sit-out limit.
const SWEEP_MS: u64 = 5000;

/// A member's command: one for the table, or one the runner answers itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlayerCommand {
    /// A table command (a join carries the member's club name).
    Table(Command),
    /// The answer to "Still here?".
    StillHere,
    /// A commitment for the fair-deal round `round`.
    FairCommit {
        /// The round it's for.
        round: String,
        /// The commitment.
        commitment: Bytes32,
    },
    /// A reveal for the fair-deal round `round`.
    FairReveal {
        /// The round it's for.
        round: String,
        /// The value and nonce.
        reveal: Reveal,
    },
    /// A message the table can't read: it's refused ("bad message").
    Unreadable,
    /// A fair-deal message that can't be read: left out, like a missing one.
    Ignored,
}

impl PlayerCommand {
    /// Someone at their app, not their page on its own.
    fn is_active(&self) -> bool {
        match self {
            Self::StillHere => true,
            Self::Table(c) => matches!(
                c,
                Command::Act { .. }
                    | Command::SitIn { .. }
                    | Command::RequestChips { .. }
                    | Command::TimeBank
                    | Command::RunTwice { .. }
            ),
            _ => false,
        }
    }
}

/// What happens to a table, with the time it happens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A member's command, and whether they're connected, if known.
    Cmd {
        /// Who.
        id: String,
        /// What.
        cmd: PlayerCommand,
        /// Whether they're connected to the relay.
        online: Option<bool>,
    },
    /// A member's page left the table.
    Disconnected {
        /// Who.
        id: String,
    },
    /// A member is connected to the relay, or not.
    Presence {
        /// Who.
        id: String,
        /// Connected.
        on: bool,
    },
    /// The host can reach the relay, or not.
    Link {
        /// It can.
        ok: bool,
    },
    /// The club's answer to a chip request.
    Chips {
        /// The seat that asked.
        seat: usize,
        /// Approved.
        ok: bool,
    },
    /// Stand a seat up: now between hands, else when the hand ends.
    Remove {
        /// The seat.
        seat: usize,
    },
    /// A member is being stood up (no idle checks for them), or not any more.
    Leaving {
        /// Who.
        id: String,
        /// Being stood up.
        on: bool,
    },
    /// The hit-and-run rule holds a member: sitting out blinds them off.
    MustPlay {
        /// Who.
        id: String,
        /// Held.
        on: bool,
    },
    /// The table deals new hands (false: it's closing).
    Open {
        /// Dealing.
        on: bool,
    },
    /// The table as it is.
    State,
    /// The time the last step asked for came.
    Wake,
}

/// An engine result: the host's view and everything the club needs from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableResult {
    /// Goes up whenever anything changes.
    pub seq: u64,
    /// The host's view (no hidden cards).
    pub state: TableView,
    /// An away player should act next.
    pub bot_to_act: bool,
    /// Time left for whoever is acting, if a person is on the clock.
    pub turn_ms_left: Option<u64>,
    /// People who'd be dealt into the next hand.
    pub players_in: usize,
    /// Chip requests waiting for the club.
    pub chip_requests: Vec<ChipRequest>,
    /// People at the table and their chips.
    pub players: Vec<SeatedPlayer>,
    /// People who left since the last result, and the chips they took.
    pub departed: Vec<Departure>,
    /// Approved top-ups that didn't fit under the maximum.
    pub refunds: Vec<Refund>,
    /// Each member's update.
    pub out: Vec<Outgoing>,
    /// Only from a new hand: the fair-deal seed it was dealt from, if any.
    pub seed: Option<Option<Bytes32>>,
}

/// A fair-deal result for one player dealt into a hand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FairResult {
    /// The hand.
    pub hand: u64,
    /// The game ("nlhe", "plo4", …).
    pub game: String,
    /// How many were dealt in.
    pub dealt: usize,
    /// Where this player's cards are in the deal.
    pub index: usize,
    /// The seed.
    pub seed: Bytes32,
    /// Everyone's commitments, host first.
    pub list: Vec<(String, Bytes32)>,
    /// The reveals used, in list order.
    pub reveals: Vec<(String, Reveal)>,
}

/// A message the runner sends a member itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunnerUpdate {
    /// Commit to a value for the fair-deal round.
    FairCommit {
        /// The round.
        round: String,
    },
    /// Everyone's commitments: reveal yours.
    FairReveal {
        /// The round.
        round: String,
        /// The commitments, host first.
        list: Vec<(String, Bytes32)>,
    },
    /// A fair-dealt hand is over: check it.
    FairResult(FairResult),
    /// "Still here?", with this many seconds to answer.
    IdleAsk {
        /// Seconds to answer.
        secs: u64,
    },
    /// The question goes away.
    IdleDone,
}

/// What a step did, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// An engine result.
    Engine(Box<TableResult>),
    /// A message for a member.
    Send {
        /// Who.
        to: String,
        /// What.
        update: RunnerUpdate,
    },
    /// A member's last activity, for their seat's record.
    Active {
        /// Who.
        id: String,
        /// When.
        at: u64,
    },
    /// Idle too long: the club stands them up.
    Idle {
        /// Who.
        id: String,
    },
}

/// What an event did, and when to wake the runner next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// What happened, in order.
    pub actions: Vec<Action>,
    /// When to send [`Event::Wake`]; `None`: no need.
    pub wake: Option<u64>,
    /// The table's clock is stopped (no one seated can be reached).
    pub stopped: bool,
    /// A restored table is waiting for its people.
    pub paused: bool,
}

/// How a runner starts.
#[derive(Clone, Debug)]
pub struct RunnerConfig {
    /// The table's id, for fair-deal round names.
    pub id: String,
    /// The game, for fair-deal results.
    pub game: String,
    /// The table deals new hands (false: closing).
    pub open: bool,
    /// The host can reach the relay.
    pub link: bool,
    /// Restored, waiting for its people.
    pub paused: bool,
    /// Each seated member's last activity (a restored table's).
    pub active: Vec<(String, u64)>,
    /// Idle time before someone is asked or stood up.
    pub idle_ms: u64,
    /// Time to answer "Still here?".
    pub ask_ms: u64,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            game: "nlhe".to_string(),
            open: true,
            link: true,
            paused: false,
            active: Vec::new(),
            idle_ms: IDLE_MS,
            ask_ms: ASK_MS,
        }
    }
}

/// When each timed thing is next due.
#[derive(Default)]
struct Due {
    grace: Option<u64>,
    fair: Option<u64>,
    tick: Option<u64>,
    sweep: Option<u64>,
    advance: Option<u64>,
    deal: Option<u64>,
    idle: Option<u64>,
}

struct Round {
    id: String,
    started: u64,
    host: Reveal,
    invited: Vec<String>,
    commits: Vec<(String, Bytes32)>,
    reveals: HashMap<String, Reveal>,
    list: Option<Vec<(String, Bytes32)>>,
    used: Vec<(String, Reveal)>,
    seed: Option<Bytes32>,
}

struct Dealt {
    hand: u64,
    seed: Bytes32,
    list: Vec<(String, Bytes32)>,
    used: Vec<(String, Reveal)>,
    seats: Vec<(String, usize)>,
}

/// One club table's loop. See the module docs.
pub struct TableRunner {
    host: TableHost,
    id: String,
    game: String,
    idle_ms: u64,
    ask_ms: u64,
    due: Due,
    grace_unset: bool,
    now: u64,
    actions: Vec<Action>,
    stopped_at: Option<u64>,
    lost: u64,
    stopped: bool,
    link: bool,
    open: bool,
    paused: bool,
    online: HashSet<String>,
    back: HashSet<String>,
    players: Vec<SeatedPlayer>,
    view: Option<TableView>,
    complete: bool,
    hand_number: u64,
    dealt_hand: u64,
    recorded: u64,
    active: HashMap<String, u64>,
    asked: HashMap<String, u64>,
    leaving: HashSet<String>,
    round: Option<Round>,
    dealt: Option<Dealt>,
    results: HashMap<String, Vec<FairResult>>,
}

impl TableRunner {
    /// Runs `host` (a club table, or one restored and paused).
    pub fn new(host: TableHost, config: RunnerConfig) -> Self {
        Self {
            host,
            id: config.id,
            game: config.game,
            idle_ms: config.idle_ms,
            ask_ms: config.ask_ms,
            due: Due::default(),
            grace_unset: config.paused,
            now: 0,
            actions: Vec::new(),
            stopped_at: None,
            lost: 0,
            stopped: false,
            link: config.link,
            open: config.open,
            paused: config.paused,
            online: HashSet::new(),
            back: HashSet::new(),
            players: Vec::new(),
            view: None,
            complete: true,
            hand_number: 0,
            dealt_hand: 0,
            recorded: 0,
            active: config.active.into_iter().collect(),
            asked: HashMap::new(),
            leaving: HashSet::new(),
            round: None,
            dealt: None,
            results: HashMap::new(),
        }
    }

    /// The table.
    pub fn host(&self) -> &TableHost {
        &self.host
    }

    /// The engine's time: real time, less the time the clock was stopped.
    fn time(&self) -> u64 {
        self.stopped_at
            .unwrap_or(self.now)
            .saturating_sub(self.lost)
    }

    /// The table to save, to resume after a restart.
    pub fn snapshot(&mut self, now: u64) -> crate::HostSnapshot {
        self.now = now;
        self.host.snapshot(self.time())
    }

    /// Takes in `ev` at time `now`.
    pub fn event(&mut self, ev: Event, now: u64) -> Step {
        self.now = now;
        self.actions.clear();
        if self.grace_unset {
            self.grace_unset = false;
            self.due.grace = Some(now + RESUME_GRACE_MS);
        }
        match ev {
            Event::Cmd { id, cmd, online } => {
                if let Some(on) = online {
                    self.set_online(&id, on);
                }
                self.cmd(&id, cmd);
            }
            Event::Disconnected { id } => {
                let out = self.host.disconnected(&id, self.time());
                self.took(out, None);
            }
            Event::Presence { id, on } => {
                self.set_online(&id, on);
                if self.sync_clock() {
                    self.state();
                }
                if on {
                    self.came_back(&id);
                }
            }
            Event::Link { ok } => {
                self.link = ok;
                if self.sync_clock() {
                    self.state();
                }
            }
            Event::Chips { seat, ok } => {
                let t = self.time();
                let out = if ok {
                    self.host.approve_chips(seat, t)
                } else {
                    self.host.deny_chips(seat, t)
                };
                self.took(out, None);
            }
            Event::Remove { seat } => {
                let out = self.host.remove(seat, self.time());
                self.took(out, None);
            }
            Event::Leaving { id, on } => {
                if on {
                    self.leaving.insert(id);
                } else {
                    self.leaving.remove(&id);
                }
            }
            Event::MustPlay { id, on } => {
                let t = self.time();
                self.host.set_must_play(&id, on, t);
            }
            Event::Open { on } => self.open = on,
            Event::State => self.state(),
            Event::Wake => self.wake(),
        }
        // A table with people at it ticks regularly, for the sit-out limit.
        // One whose clock is stopped doesn't: the limit doesn't count then.
        if !self.players.is_empty() && !self.stopped {
            self.due.sweep.get_or_insert(now + SWEEP_MS);
        } else {
            self.due.sweep = None;
        }
        self.due.idle = self.idle_due();
        let d = &self.due;
        let wake = [d.grace, d.fair, d.tick, d.sweep, d.advance, d.deal, d.idle]
            .into_iter()
            .flatten()
            .min();
        Step {
            actions: std::mem::take(&mut self.actions),
            wake,
            stopped: self.stopped,
            paused: self.paused,
        }
    }

    fn set_online(&mut self, id: &str, on: bool) {
        if on {
            self.online.insert(id.to_string());
        } else {
            self.online.remove(id);
        }
    }

    fn state(&mut self) {
        self.took(Vec::new(), None);
    }

    /// Takes in what the engine did.
    fn took(&mut self, out: Vec<Outgoing>, seed: Option<Option<Bytes32>>) {
        let now = self.time();
        let res = TableResult {
            seq: self.host.seq(),
            state: self.host.host_view(),
            bot_to_act: self.host.auto_to_act(),
            turn_ms_left: self.host.turn_ms_left(now),
            players_in: self.host.players_in(),
            chip_requests: self.host.chip_requests(),
            players: self.host.seated(),
            departed: self.host.take_departures(),
            refunds: self.host.take_refunds(),
            out,
            seed,
        };
        self.players = res.players.clone();
        for d in &res.departed {
            self.active.remove(&d.client_id);
            self.asked.remove(&d.client_id);
            self.leaving.remove(&d.client_id);
        }
        self.complete = res.state.complete;
        self.hand_number = res.state.hand_number;
        // Idle seats: anyone new is counted from now; a hand dealt to them is activity.
        for p in &self.players {
            self.active.entry(p.client_id.clone()).or_insert(self.now);
        }
        if !self.complete && self.hand_number != 0 && self.hand_number != self.dealt_hand {
            self.dealt_hand = self.hand_number;
            let dealt: Vec<String> = self
                .players
                .iter()
                .filter(|p| {
                    !p.pending
                        && res
                            .state
                            .seats
                            .get(p.seat)
                            .is_some_and(|s| !s.empty && !s.sitting_out)
                })
                .map(|p| p.client_id.clone())
                .collect();
            for id in dealt {
                self.mark_active(&id);
            }
        }
        let (turn_ms_left, bot_to_act, players_in) =
            (res.turn_ms_left, res.bot_to_act, res.players_in);
        self.view = Some(res.state.clone());
        self.actions.push(Action::Engine(Box::new(res)));
        // A fair-dealt hand's reveals go out after its final state.
        if self.complete && self.hand_number != 0 && self.hand_number != self.recorded {
            self.recorded = self.hand_number;
            self.fair_publish();
        }
        let restarted = self.sync_clock();
        self.pace(turn_ms_left, bot_to_act, players_in);
        // Restarted: the state's turn clock and next step set the times again.
        if restarted {
            self.state();
        }
    }

    /// Stops or restarts the clock to match who can be reached. Returns
    /// whether it restarted.
    fn sync_clock(&mut self) -> bool {
        let stop = !self.link
            || !self
                .players
                .iter()
                .any(|p| self.online.contains(&p.client_id));
        if stop == self.stopped {
            return false;
        }
        self.stopped = stop;
        if stop {
            self.stopped_at = Some(self.now);
            let d = &mut self.due;
            (d.tick, d.sweep, d.advance, d.deal, d.fair) = (None, None, None, None, None);
            self.round = None;
            return false;
        }
        if let Some(at) = self.stopped_at.take() {
            self.lost += self.now.saturating_sub(at);
        }
        true
    }

    /// What happens next: the clock's or an away player's move, the next hand.
    fn pace(&mut self, turn_ms_left: Option<u64>, bot_to_act: bool, players_in: usize) {
        if self.stopped {
            return;
        }
        self.due.tick = turn_ms_left.map(|ms| self.now + ms + 50);
        if bot_to_act {
            self.due.advance = Some(self.now + BOT_PACE_MS);
        }
        // A restored table deals no new hand until its people are back.
        if self.complete && !self.paused && self.due.deal.is_none() && self.open && players_in >= 2
        {
            // The next hand's fair-deal round, if one isn't under way already.
            if self.round.is_none() {
                self.fair_start();
            }
            self.due.deal = Some(
                self.now
                    + if self.hand_number != 0 {
                        NEXT_HAND_MS
                    } else {
                        500
                    },
            );
        }
    }

    /// Deals the next hand, from the fair-deal round's seed. A round still
    /// under way gets a little longer; after that the hand is dealt without
    /// one rather than holding the table up.
    fn deal(&mut self) {
        if let Some(f) = &self.round
            && f.seed.is_none()
            && self.now.saturating_sub(f.started) < FAIR_WAIT_MS
        {
            self.due.deal = Some(self.now + FAIR_RETRY_MS);
            return;
        }
        let round = self.round.take();
        let seed = round.as_ref().and_then(|f| f.seed);
        let t = self.time();
        // If people left meanwhile the engine refuses; the next change
        // (someone sitting down, chips) tries again.
        let res: Result<Vec<Outgoing>, PlayError> = match &seed {
            Some(s) => self.host.new_hand_from_seed(t, s),
            None => self.host.new_hand(t),
        };
        let Ok(out) = res else { return };
        if let (Some(f), Some(seed)) = (round, seed) {
            // Who was dealt in: the players whose view came with their own
            // cards (someone sitting out isn't).
            let dealt_in: HashSet<&str> = out
                .iter()
                .filter(|o| matches!(&o.update, Update::State { view, .. } if view.seats.first().is_some_and(|s| s.cards.is_some())))
                .map(|o| o.to.as_str())
                .collect();
            let seats = self
                .host
                .seated()
                .into_iter()
                .filter(|p| dealt_in.contains(p.client_id.as_str()))
                .map(|p| (p.client_id, p.seat))
                .collect();
            self.dealt = Some(Dealt {
                hand: self.host.host_view().hand_number,
                seed,
                list: f.list.unwrap_or_default(),
                used: f.used,
                seats,
            });
        }
        self.took(out, Some(seed));
        // The round for the hand after this one runs while this one is played.
        self.fair_start();
    }

    // ---- fair deals ----
    //
    // Each seated player's page and the host commit to a random value, then,
    // once every commitment is in (a list everyone gets before revealing),
    // reveal it. The hand is dealt from the seed of all the values revealed,
    // so no single party picks the cards. After the hand the host publishes
    // the reveals, and each page checks its own cards and the board against
    // the seed. A player who doesn't commit or reveal in time is left out of
    // the seed, so the table never waits on them.

    fn fair_start(&mut self) {
        if self.round.is_some() {
            return;
        }
        let invited: Vec<String> = self.players.iter().map(|p| p.client_id.clone()).collect();
        let round = format!("{}:{}", self.id, self.now);
        for to in &invited {
            self.send(
                to,
                RunnerUpdate::FairCommit {
                    round: round.clone(),
                },
            );
        }
        self.round = Some(Round {
            id: round,
            started: self.now,
            host: Reveal::random(),
            invited,
            commits: Vec::new(),
            reveals: HashMap::new(),
            list: None,
            used: Vec::new(),
            seed: None,
        });
        self.due.fair = Some(self.now + FAIR_STEP_MS);
    }

    /// Every commitment is in (or the time's up): everyone gets the list, then reveals.
    fn fair_commits_in(&mut self) {
        let Some(f) = &mut self.round else { return };
        let mut list = vec![("host".to_string(), f.host.commitment())];
        list.extend(f.commits.iter().cloned());
        f.list = Some(list.clone());
        let (round, to): (String, Vec<String>) = (
            f.id.clone(),
            f.commits.iter().map(|(id, _)| id.clone()).collect(),
        );
        let none = to.is_empty();
        for id in to {
            self.send(
                &id,
                RunnerUpdate::FairReveal {
                    round: round.clone(),
                    list: list.clone(),
                },
            );
        }
        if none {
            self.fair_reveals_in();
        } else {
            self.due.fair = Some(self.now + FAIR_STEP_MS);
        }
    }

    /// Every reveal is in (or the time's up): the seed, from the values in list order.
    fn fair_reveals_in(&mut self) {
        self.due.fair = None;
        let Some(f) = &mut self.round else { return };
        let list = f.list.clone().unwrap_or_default();
        f.used = list
            .iter()
            .filter_map(|(id, _)| {
                if id == "host" {
                    Some((id.clone(), f.host))
                } else {
                    f.reveals.get(id).map(|r| (id.clone(), *r))
                }
            })
            .collect();
        let values: Vec<Bytes32> = f.used.iter().map(|(_, r)| r.value).collect();
        f.seed = Some(hand_seed(&values));
    }

    /// A player's commitment or reveal for the round under way.
    fn fair_incoming(&mut self, from: &str, cmd: PlayerCommand) {
        let Some(f) = &mut self.round else { return };
        if !f.invited.iter().any(|i| i == from) {
            return;
        }
        match cmd {
            PlayerCommand::FairCommit { round, commitment } => {
                if round != f.id || f.list.is_some() || f.commits.iter().any(|(id, _)| id == from) {
                    return;
                }
                f.commits.push((from.to_string(), commitment));
                if f.commits.len() == f.invited.len() {
                    self.fair_commits_in();
                }
            }
            PlayerCommand::FairReveal { round, reveal } => {
                if round != f.id
                    || f.list.is_none()
                    || f.seed.is_some()
                    || f.reveals.contains_key(from)
                {
                    return;
                }
                // A reveal that doesn't open its commitment is left out, like a missing one.
                let Some((_, c)) = f.commits.iter().find(|(id, _)| id == from) else {
                    return;
                };
                if !reveal.opens(c) {
                    return;
                }
                f.reveals.insert(from.to_string(), reveal);
                if f.reveals.len() == f.commits.len() {
                    self.fair_reveals_in();
                }
            }
            _ => {}
        }
    }

    /// A hand dealt from a seed is over: everyone dealt in gets the reveals,
    /// so their page can check the deal, with where their cards are in it.
    fn fair_publish(&mut self) {
        let Some(d) = self.dealt.take_if(|d| d.hand == self.hand_number) else {
            return;
        };
        // Cards go to the dealt seats in seat order (Deal::from_seed).
        let mut order = d.seats.clone();
        order.sort_by_key(|(_, seat)| *seat);
        for (index, (id, _)) in order.iter().enumerate() {
            let r = FairResult {
                hand: d.hand,
                game: self.game.clone(),
                dealt: order.len(),
                index,
                seed: d.seed,
                list: d.list.clone(),
                reveals: d.used.clone(),
            };
            let kept = self.results.entry(id.clone()).or_default();
            kept.push(r.clone());
            if kept.len() > FAIR_KEEP {
                kept.remove(0);
            }
            self.send(id, RunnerUpdate::FairResult(r));
        }
    }

    // ---- idle seats ----
    //
    // Someone seated idle_ms without being dealt a hand (sitting out, or
    // alone at the table) is stood up. If they're connected they're asked
    // first ("Still here?") and have ask_ms to say so, which gives them
    // another idle_ms. This runs even while the clock is stopped: that's how
    // a seat no one came back to was once held for two weeks.

    /// Member `id` did something here (or was dealt a hand): idle from now.
    fn mark_active(&mut self, id: &str) {
        self.active.insert(id.to_string(), self.now);
        self.actions.push(Action::Active {
            id: id.to_string(),
            at: self.now,
        });
        if self.asked.remove(id).is_some() {
            self.send(id, RunnerUpdate::IdleDone);
        }
    }

    /// When the idle check is next due, for whoever's due first.
    fn idle_due(&self) -> Option<u64> {
        // A closing table stands everyone up anyway.
        if !self.open {
            return None;
        }
        let at = self
            .players
            .iter()
            .filter(|p| !self.leaving.contains(&p.client_id))
            .map(|p| match self.asked.get(&p.client_id) {
                Some(a) => a + self.ask_ms,
                None => self.active.get(&p.client_id).copied().unwrap_or(self.now) + self.idle_ms,
            })
            .min()?;
        Some(if !self.link || self.paused {
            at.max(self.now + IDLE_RECHECK_MS)
        } else {
            at
        })
    }

    /// Asks or stands up whoever's been idle too long.
    fn check_idle(&mut self) {
        if !self.link || self.paused || !self.open {
            return;
        }
        let ids: Vec<String> = self.players.iter().map(|p| p.client_id.clone()).collect();
        for id in ids {
            let since = self.active.get(&id).copied().unwrap_or(self.now);
            if self.leaving.contains(&id) || self.now.saturating_sub(since) < self.idle_ms {
                continue;
            }
            let online = self.online.contains(&id);
            match self.asked.get(&id).copied() {
                None if online => {
                    self.asked.insert(id.clone(), self.now);
                    self.send(
                        &id,
                        RunnerUpdate::IdleAsk {
                            secs: (self.ask_ms + 500) / 1000,
                        },
                    );
                    continue;
                }
                Some(a) if online && self.now.saturating_sub(a) < self.ask_ms => continue,
                _ => {}
            }
            self.leaving.insert(id.clone());
            self.actions.push(Action::Idle { id });
        }
    }

    // ---- after a restart ----

    /// A restored table's clocks run again: everyone not back yet counts as
    /// disconnected (they check or fold, and the sit-out limit counts).
    fn resume(&mut self) {
        if !self.paused {
            return;
        }
        self.paused = false;
        self.due.grace = None;
        let away: Vec<String> = self
            .players
            .iter()
            .filter(|p| !self.back.contains(&p.client_id))
            .map(|p| p.client_id.clone())
            .collect();
        for id in away {
            let out = self.host.disconnected(&id, self.time());
            self.took(out, None);
        }
        let out = self.host.resume(self.time());
        self.took(out, None);
    }

    /// Someone at a restored table is back: once everyone is, play goes on.
    fn came_back(&mut self, id: &str) {
        if !self.paused {
            return;
        }
        self.back.insert(id.to_string());
        if self
            .players
            .iter()
            .all(|p| self.back.contains(&p.client_id))
        {
            self.resume();
        }
    }

    // ---- events ----

    fn wake(&mut self) {
        // Each in turn, if it's due (an earlier one can set or clear a later one).
        let now = self.now;
        let due = |slot: &mut Option<u64>| -> bool {
            if slot.is_some_and(|t| t <= now) {
                *slot = None;
                true
            } else {
                false
            }
        };
        if due(&mut self.due.grace) {
            self.resume();
        }
        if due(&mut self.due.fair) {
            match &self.round {
                Some(f) if f.list.is_none() => self.fair_commits_in(),
                Some(f) if f.seed.is_none() => self.fair_reveals_in(),
                _ => {}
            }
        }
        if due(&mut self.due.tick) {
            let out = self.host.tick(self.time());
            self.took(out, None);
        }
        if due(&mut self.due.sweep) && !self.players.is_empty() {
            let out = self.host.tick(self.time());
            self.took(out, None);
        }
        if due(&mut self.due.advance)
            && let Ok(out) = self.host.advance(self.time())
        {
            self.took(out, None);
        }
        if due(&mut self.due.deal) {
            self.deal();
        }
        if due(&mut self.due.idle) {
            self.check_idle();
        }
    }

    fn cmd(&mut self, id: &str, cmd: PlayerCommand) {
        if matches!(
            cmd,
            PlayerCommand::FairCommit { .. } | PlayerCommand::FairReveal { .. }
        ) {
            self.fair_incoming(id, cmd);
            return;
        }
        if cmd.is_active() && self.players.iter().any(|p| p.client_id == id) {
            self.mark_active(id);
        }
        let c = match cmd {
            PlayerCommand::Table(c) => c,
            PlayerCommand::Unreadable => {
                let reason = "bad message".to_string();
                let out = vec![Outgoing {
                    to: id.to_string(),
                    update: Update::Rejected { reason },
                }];
                self.took(out, None);
                return;
            }
            _ => return,
        };
        let join = matches!(c, Command::Join { .. });
        let out = self.host.handle(id, c, self.time());
        self.took(out, None);
        // Back at a restored table; and the last few hands' fair-deal
        // results again, in case they were away when they went.
        if join {
            self.came_back(id);
            for r in self.results.get(id).cloned().unwrap_or_default() {
                self.send(id, RunnerUpdate::FairResult(r));
            }
        }
    }

    fn send(&mut self, to: &str, update: RunnerUpdate) {
        self.actions.push(Action::Send {
            to: to.to_string(),
            update,
        });
    }
}
