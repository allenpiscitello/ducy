//! Saving a table or tournament and loading it again, e.g. so a host whose
//! browser restarts can resume the hand in progress (see [`Table::snapshot`]
//! and [`Table::restore`]; ducy-host saves the hosting around them).
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
//! [`Table::snapshot`]: crate::Table::snapshot
//! [`Table::restore`]: crate::Table::restore

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
    /// Running it twice was offered (see `Hand::offer_run_twice`).
    #[cfg_attr(feature = "serde", serde(default))]
    pub(crate) run_offer: bool,
    /// The rest of the deck after the board, in order: a second board's cards.
    #[cfg_attr(feature = "serde", serde(default))]
    pub(crate) rest: Vec<Card>,
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
    /// Players all-in may run it twice.
    #[cfg_attr(feature = "serde", serde(default))]
    pub(crate) run_it_twice: bool,
    /// A bomb pot every few hands.
    #[cfg_attr(feature = "serde", serde(default))]
    pub(crate) bomb_pots: Option<crate::BombPots>,
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

impl TableSnapshot {
    /// The table's seats.
    pub fn seat_count(&self) -> usize {
        self.seats.len()
    }
}
