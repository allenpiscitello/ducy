//! Saving a hosted table and loading it again, e.g. so a host whose browser
//! restarts can resume the hand in progress (see [`TableHost::snapshot`] and
//! [`TableHost::restore`]).
//!
//! A snapshot is plain data (JSON with the `serde` feature). The hand in
//! progress is saved as how it started and what has happened since, and
//! loading it plays those actions again, so a restored hand is exactly the
//! hand that was saved; one that doesn't replay to the same events is
//! refused. Clocks are saved as time left, so they go on from where they
//! stopped.
//!
//! A snapshot holds every card of the hand in progress, the cards to come
//! included, as the host's memory does. Keep it where only the host can
//! read it.
//!
//! [`TableHost::snapshot`]: crate::TableHost::snapshot
//! [`TableHost::restore`]: crate::TableHost::restore

use ducy::deck::Card;

use crate::{Event, TableRules};

/// The snapshot format: a snapshot with any other version is refused.
pub const SNAPSHOT_VERSION: u32 = 1;

/// A hand: how it started and everything that has happened since.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct HandSnapshot {
    pub(crate) rules: TableRules,
    /// Each player's stack before the hand.
    pub(crate) stacks: Vec<u64>,
    pub(crate) button: usize,
    /// Each player's hole cards.
    pub(crate) hole_cards: Vec<Vec<Card>>,
    /// All five board cards, dealt or not.
    pub(crate) board: Vec<Card>,
    pub(crate) events: Vec<Event>,
    /// Dealt hidden (a trustless shuffle): the engine knows only the cards in
    /// its events (the board so far, and hands shown at showdown).
    #[cfg_attr(feature = "serde", serde(default))]
    pub(crate) hidden: bool,
}

/// One seat of a [`TableSnapshot`]. A bot isn't saved: the seat records that
/// it has one, and loading asks the caller for it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SeatSnapshot {
    pub(crate) name: String,
    pub(crate) id: String,
    pub(crate) has_bot: bool,
    pub(crate) bot_name: String,
    pub(crate) human: bool,
    pub(crate) away: bool,
    pub(crate) sitting_out: bool,
}

/// A [`crate::Table`] and the hand being played (or the last one).
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TableSnapshot {
    pub(crate) rules: TableRules,
    pub(crate) seats: Vec<SeatSnapshot>,
    pub(crate) stacks: Vec<u64>,
    pub(crate) buy_in: u64,
    pub(crate) button: usize,
    pub(crate) hand: Option<HandSnapshot>,
    pub(crate) dealt: Vec<usize>,
    pub(crate) hand_number: u64,
    pub(crate) synced: bool,
    pub(crate) top_up: bool,
    /// `None` for secure deals. Saved as text: a seed may not fit in a
    /// JavaScript number.
    pub(crate) seed: Option<String>,
    pub(crate) missed: Vec<(bool, bool)>,
    /// How each seat comes back after sitting out: "ready", "post" or
    /// "wait_for_big_blind"; or "arriving" for a newcomer not dealt in yet.
    pub(crate) returning: Vec<String>,
    pub(crate) last_blinds: Option<(usize, usize)>,
}

/// The tournament snapshot format: one with any other version is refused.
pub const TOURNAMENT_SNAPSHOT_VERSION: u32 = 1;

/// A whole [`crate::Tournament`]: see [`crate::Tournament::snapshot`]. Each
/// table is saved as a [`TableSnapshot`], the hand in progress included, so
/// it holds every card of every hand being played.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TournamentSnapshot {
    /// [`TOURNAMENT_SNAPSHOT_VERSION`] when it was saved.
    pub version: u32,
    pub(crate) config: crate::TournamentConfig,
    /// By table id; `None` for a broken table.
    pub(crate) tables: Vec<Option<TableSnapshot>>,
    pub(crate) level: usize,
    pub(crate) finishes: Vec<crate::Finish>,
    pub(crate) winner: Option<crate::Finish>,
    pub(crate) entries: usize,
    pub(crate) hand_for_hand: bool,
    pub(crate) hfh_hands: Vec<u64>,
    pub(crate) finished: Vec<u64>,
    pub(crate) draws: u64,
    /// Tables deal from system randomness (`Tournament::with_secure_deals`).
    #[cfg_attr(feature = "serde", serde(default))]
    pub(crate) secure: bool,
}

/// One person at a [`HostSnapshot`]'s table.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PlayerSnapshot {
    pub(crate) client_id: String,
    pub(crate) name: String,
    pub(crate) seat: usize,
    pub(crate) timeouts: u32,
    pub(crate) pending: bool,
    pub(crate) leaving: bool,
    pub(crate) missed: u32,
    pub(crate) requested: u64,
    pub(crate) approved: u64,
    pub(crate) out_by_choice: bool,
    /// How long they had been sitting out, if they were.
    pub(crate) out_ms: Option<u64>,
}

/// A whole [`crate::TableHost`]: see [`crate::TableHost::snapshot`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct HostSnapshot {
    /// [`SNAPSHOT_VERSION`] when it was saved.
    pub version: u32,
    pub(crate) table: TableSnapshot,
    pub(crate) open: Vec<bool>,
    pub(crate) players: Vec<PlayerSnapshot>,
    pub(crate) seq: u64,
    pub(crate) turn_ms: u64,
    /// Time left on the turn clock.
    pub(crate) turn_ms_left: Option<u64>,
    pub(crate) turn: Option<(u64, usize)>,
    pub(crate) bot_ids: Vec<String>,
    /// Buy-in limits, with a bank.
    pub(crate) bank: Option<(u64, u64)>,
    pub(crate) host_approved: u64,
    pub(crate) has_host: bool,
    pub(crate) departed: Vec<crate::Departure>,
    /// Approved chips that didn't fit under the maximum, not yet taken.
    #[cfg_attr(feature = "serde", serde(default))]
    pub(crate) refunds: Vec<crate::Refund>,
    pub(crate) sit_out_limit_ms: u64,
}
