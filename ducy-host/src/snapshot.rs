//! Saving a hosted table or tournament and loading it again, e.g. so a host
//! whose browser restarts can resume the hand in progress (see
//! [`TableHost::snapshot`] and [`TableHost::restore`]).
//!
//! A snapshot is plain data (JSON with the `serde` feature): the engine's
//! [`TableSnapshot`] or [`TournamentSnapshot`] and the hosting around it.
//! Clocks are saved as time left, so they go on from where they stopped.
//!
//! A snapshot holds every card of the hand in progress, the cards to come
//! included, as the host's memory does. Keep it where only the host can
//! read it.
//!
//! [`TableHost::snapshot`]: crate::TableHost::snapshot
//! [`TableHost::restore`]: crate::TableHost::restore

pub use ducy_play::snapshot::{SNAPSHOT_VERSION, TOURNAMENT_SNAPSHOT_VERSION};
use ducy_play::snapshot::{TableSnapshot, TournamentSnapshot};

/// A whole [`crate::TournamentHost`]: see [`crate::TournamentHost::snapshot`].
/// The tournament, and the hosting around it: who's in it, each table's turn
/// clock (as time left), and timeouts in a row. Who's absent is part of the
/// tables.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TournamentHostSnapshot {
    /// [`TOURNAMENT_SNAPSHOT_VERSION`] when it was saved.
    pub version: u32,
    pub(crate) tournament: TournamentSnapshot,
    pub(crate) turn_ms: u64,
    pub(crate) seq: u64,
    /// The people in it (sorted).
    pub(crate) people: Vec<String>,
    /// Turns in a row each person ran out of time.
    pub(crate) timeouts: Vec<(String, u32)>,
    /// Per table: the turn on the clock (hand number, events so far), and
    /// the time it had left.
    pub(crate) turns: Vec<(usize, (u64, usize))>,
    pub(crate) turn_ms_left: Vec<(usize, u64)>,
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
    /// Time left in their time bank (full for a table saved before there was one).
    #[cfg_attr(feature = "serde", serde(default = "full_time_bank"))]
    pub(crate) time_bank_ms: u64,
    /// Owes the table hands ([`crate::TableHost::set_must_play`]).
    #[cfg_attr(feature = "serde", serde(default))]
    pub(crate) must_play: bool,
}

#[cfg(feature = "serde")]
fn full_time_bank() -> u64 {
    crate::host::TIME_BANK_MS
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
    /// A time bank running on this turn: whose, and how much was added.
    #[cfg_attr(feature = "serde", serde(default))]
    pub(crate) time_bank: Option<(String, u64)>,
}
