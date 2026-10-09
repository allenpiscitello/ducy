//! Play out poker hands on top of [`ducy`]: antes and blinds, betting rounds,
//! side pots and the showdown.
//!
//! Supports Texas Hold'em and Omaha (4, 5 or 6 hole cards), each with
//! no-limit or pot-limit betting.
//!
//! ```
//! use ducy_play::{Action, Deal, Hand, TableRules};
//!
//! let rules = TableRules::no_limit_holdem(1, 2);
//! let deal = Deal::random(rules.variant, 3, Some(42)).unwrap();
//! let mut hand = Hand::new(rules, &[200, 200, 200], 0, deal).unwrap();
//!
//! // Everyone calls or checks to the river.
//! while let Some(legal) = hand.legal_actions() {
//!     let action = if legal.can_check { Action::Check } else { Action::Call };
//!     hand.act(action).unwrap();
//! }
//! let result = hand.result().unwrap();
//! assert_eq!(result.final_stacks.iter().sum::<u64>(), 600);
//! ```
//!
//! # From one hand to a tournament
//!
//! - [`Hand`]: one hand, from the blinds to the payout, with its [`Event`]
//!   history and each player's [`LegalActions`].
//! - [`Table`]: seats over many hands: the button moves, people sit out and
//!   come back (posting missed blinds or waiting for the big blind), and bots
//!   play their seats ([`Table::advance`]).
//! - [`TableHost`]: a table for remote players. It takes each player's
//!   [`Command`]s and answers with [`Update`]s showing only what that player
//!   may see. It also runs turn clocks, chip requests at a table with a bank,
//!   and sit-out limits. It can be saved mid-hand and restored ([`snapshot`]).
//! - [`Tournament`]: several tables with blind [`Level`]s, eliminations,
//!   balancing and hand-for-hand play on the bubble.
//! - Bots: the [`Bot`] trait, simple [`bots`], 15 [`Personality`] bots,
//!   [`run_match`] for duplicate matches, and `ProcessBot` (feature
//!   `process`) for bots in any language, talking JSON over stdin/stdout.
//!
//! # Features
//!
//! - `process` (default): `ProcessBot`, and `serde`.
//! - `serde`: `Serialize`/`Deserialize` for rules, actions, events, views,
//!   commands and updates (JSON for external bots and clients) and snapshots.
//!
//! A seed makes deals reproducible. Without one ([`Deal::random`] with
//! `None`, or [`Table::with_secure_deals`]) every deal comes from fresh system
//! randomness: use that whenever people play each other.

#![warn(missing_docs)]

#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;

mod bot;
pub mod bots;
mod deal;
mod error;
pub mod fair_deal;
mod hand;
mod host;
mod matchup;
pub mod personality;
#[cfg(feature = "process")]
mod process;
pub mod replay;
mod rules;
mod showdown;
pub mod snapshot;
pub mod stats;
pub mod strength;
pub mod structure;
mod table;
mod tournament;
mod tournament_host;

pub use bot::{Bot, HandOutcome, HandSummary, Observation, SeatView, fallback_action, play_hand};
pub use deal::{Deal, Dealer, HiddenDeal};
pub use error::PlayError;
pub use hand::{
    Action, Awaiting, Event, Hand, HandResult, LegalActions, MAX_PLAYERS, Post, RaiseRange, Street,
};
pub use host::{
    ChipRequest, ChipsView, Command, DROP_AFTER_HANDS, Departure, MAX_NAME, Outgoing, Refund,
    SeatStatus, SeatedPlayer, TIME_BANK_MS, TIME_BANK_REFILL_MS, TableHost, Update,
};
pub use matchup::{MatchConfig, MatchResult, run_match};
pub use personality::{Personality, PersonalityBot, Style, position_strength, scale_for_table};
#[cfg(feature = "process")]
pub use process::ProcessBot;
pub use rules::{BettingStructure, TableRules, Variant};
pub use showdown::{Award, Pot};
pub use snapshot::{
    HandSnapshot, HostSnapshot, SNAPSHOT_VERSION, TOURNAMENT_SNAPSHOT_VERSION, TableSnapshot,
    TournamentHostSnapshot, TournamentSnapshot,
};
pub use table::{SeatState, Table, TableSeat, TableView};
pub use tournament::{
    Entrant, Finish, HandReport, Level, Move, SeatRef, Standing, Tournament, TournamentConfig,
};
pub use tournament_host::{TournamentEvents, TournamentHost};
