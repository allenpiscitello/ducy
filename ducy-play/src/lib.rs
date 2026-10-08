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

#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;

mod bot;
pub mod bots;
mod deal;
mod error;
mod hand;
mod host;
mod matchup;
pub mod personality;
#[cfg(feature = "process")]
mod process;
mod rules;
mod showdown;
pub mod snapshot;
pub mod stats;
pub mod strength;
pub mod structure;
mod table;
mod tournament;

pub use bot::{Bot, HandOutcome, HandSummary, Observation, SeatView, fallback_action, play_hand};
pub use deal::Deal;
pub use error::PlayError;
pub use hand::{
    Action, Event, Hand, HandResult, LegalActions, MAX_PLAYERS, Post, RaiseRange, Street,
};
pub use host::{
    ChipRequest, ChipsView, Command, DROP_AFTER_HANDS, Departure, MAX_NAME, Outgoing, Refund,
    SeatStatus, SeatedPlayer, TableHost, Update,
};
pub use matchup::{MatchConfig, MatchResult, run_match};
pub use personality::{Personality, PersonalityBot, Style, position_strength, scale_for_table};
#[cfg(feature = "process")]
pub use process::ProcessBot;
pub use rules::{BettingStructure, TableRules, Variant};
pub use showdown::{Award, Pot};
pub use snapshot::{
    HandSnapshot, HostSnapshot, SNAPSHOT_VERSION, TOURNAMENT_SNAPSHOT_VERSION, TableSnapshot,
    TournamentSnapshot,
};
pub use table::{SeatState, Table, TableSeat, TableView};
pub use tournament::{
    Entrant, Finish, HandReport, Level, Move, SeatRef, Standing, Tournament, TournamentConfig,
};
