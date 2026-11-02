//! Poker bots for [`ducy_play`]: simple [`bots`], 15 [`Personality`] bots,
//! the hand reading and equity they play by, a small neural-net bot
//! ([`net`]), [`run_match`] for duplicate matches, and `ProcessBot` (feature
//! `process`) for bots in any language, talking JSON over stdin/stdout.
//!
//! ```
//! use ducy_bots::{MatchConfig, Personality, run_match};
//! use ducy_play::{Bot, TableRules};
//!
//! let config = MatchConfig::new(TableRules::no_limit_holdem(1, 2), 5, 1).duplicate();
//! let mut bots: Vec<Box<dyn Bot>> = vec![
//!     Box::new(Personality::DougPoker.bot(Some(1))),
//!     Box::new(Personality::MilkKing.bot(Some(2))),
//! ];
//! let result = run_match(&config, &mut bots).unwrap();
//! assert_eq!(result.net.iter().sum::<i64>(), 0);
//! ```
//!
//! # Features
//!
//! - `process` (default): `ProcessBot`, and `serde`.
//! - `serde`: `Serialize`/`Deserialize` for opponent [`stats`], and
//!   ducy-play's `serde`.

#![warn(missing_docs)]

#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;

pub mod bots;
pub mod features;
pub mod generic;
mod matchup;
pub mod net;
pub mod net_data;
pub mod personality;
#[cfg(feature = "process")]
mod process;
pub mod ranges;
pub mod stats;
pub mod strength;
pub mod traits;
mod variants;

pub use matchup::{MatchConfig, MatchResult, run_match};
pub use personality::{Personality, PersonalityBot, Style, position_strength, scale_for_table};
#[cfg(feature = "process")]
pub use process::ProcessBot;
pub use variants::{Comparison, VariantResult, run_variants};
