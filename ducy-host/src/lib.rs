//! Host poker tables and tournaments for remote players, on top of
//! [`ducy_play`]'s engine.
//!
//! - [`TableHost`]: a table for remote players. It takes each player's
//!   [`Command`]s and answers with [`Update`]s showing only what that player
//!   may see. It also runs turn clocks, time banks, chip requests at a table
//!   with a bank, and sit-out limits. It can be saved mid-hand and restored
//!   ([`snapshot`]).
//! - [`TournamentHost`]: a [`ducy_play::Tournament`] for remote players, every
//!   table's clocks and views.
//! - [`runner`] and [`tourney_runner`]: a host's loop: when to deal the next
//!   hand, absent players' moves at a human pace, provably fair or hidden
//!   deals, and what to send and save.
//!
//! Bots take seats as any [`ducy_play::Bot`]; this crate has none of its own.
//!
//! # Features
//!
//! - `serde`: `Serialize`/`Deserialize` for commands, updates and snapshots.

#![warn(missing_docs)]

mod host;
pub mod runner;
pub mod snapshot;
mod tournament_host;
pub mod tourney_runner;

pub use host::{
    ChipRequest, ChipsView, Command, DROP_AFTER_HANDS, Departure, MAX_NAME, Outgoing, Refund,
    SeatStatus, SeatedPlayer, TIME_BANK_MS, TIME_BANK_REFILL_MS, TableHost, Update,
};
pub use snapshot::{HostSnapshot, PlayerSnapshot, TournamentHostSnapshot};
pub use tournament_host::{TournamentEvents, TournamentHost};
