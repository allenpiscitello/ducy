//! ducy.cards' Bankroll Quest (ducy-web #299): turn $1,000 into $1,000,000
//! across a world of card rooms. The world is built from the quest's seed, so
//! it's the same on every reload and different from someone else's: the same
//! seed gives the same world as the page's JavaScript did, so saves keep
//! working.
//!
//! - [`dice`]: the world's dice, a number from the seed and some keys.
//! - [`world`]: the rooms, regions and stakes, and which games run when.
//! - [`people`]: the card rooms' generic players: their styles, names, regs'
//!   rosters and tourists.
//! - [`waitlist`]: who's waiting for each game and how fast seats open.
//! - [`lineup`]: who sits at each table, and their stacks.
//!
//! Amounts are in hundredths (100 is $1); times are game minutes since the
//! world began, a Monday at midnight.

#![warn(missing_docs)]
// Hours that wrap past midnight read better as `hour >= 22 || hour < 4`.
#![allow(clippy::manual_range_contains)]

pub mod dice;
pub mod lineup;
pub mod people;
pub mod waitlist;
pub mod world;
