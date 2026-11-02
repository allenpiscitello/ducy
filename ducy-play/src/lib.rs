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
//! - Hosting a table for remote players (commands in, private views out,
//!   turn clocks, chip requests) is the `ducy-host` crate's. A table or
//!   tournament can be saved mid-hand and restored ([`snapshot`]).
//! - [`Tournament`]: several tables with blind [`Level`]s, eliminations,
//!   balancing and hand-for-hand play on the bubble.
//! - [`Bot`]: a seat played by code, given only what that seat may see
//!   ([`play_hand`]). Ready-made bots are the `ducy-bots` crate's.
//!
//! # Features
//!
//! - `serde` (default): `Serialize`/`Deserialize` for rules, actions,
//!   events, views and snapshots (JSON for external bots and clients).
//!
//! A seed makes deals reproducible. Without one ([`Deal::random`] with
//! `None`, or [`Table::with_secure_deals`]) every deal comes from fresh system
//! randomness: use that whenever people play each other.

#![warn(missing_docs)]

#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;

mod bot;
mod deal;
mod error;
pub mod fair_deal;
mod hand;
pub mod hands;
pub mod replay;
mod rules;
mod showdown;
pub mod snapshot;
pub mod structure;
mod table;
mod tournament;

pub use bot::{Bot, HandOutcome, HandSummary, Observation, SeatView, fallback_action, play_hand};
pub use deal::{Deal, Dealer, HiddenDeal};
pub use error::PlayError;
pub use hand::{
    Action, Awaiting, Event, Hand, HandResult, LegalActions, MAX_PLAYERS, Post, RaiseRange, Street,
};
pub use rules::{BettingStructure, TableRules, Variant};
pub use showdown::{Award, Pot, best_hands};
pub use snapshot::{
    HandSnapshot, SNAPSHOT_VERSION, SeatSnapshot, TOURNAMENT_SNAPSHOT_VERSION, TableSnapshot,
    TournamentSnapshot,
};
pub use table::{BombPots, SeatState, Table, TableSeat, TableView};
pub use tournament::{
    Entrant, Finish, HandReport, Level, Move, SeatRef, Standing, Tournament, TournamentConfig,
};
