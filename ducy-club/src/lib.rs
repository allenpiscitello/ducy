//! ducy.cards' poker clubs: a club's rules, money, tables, waitlists and
//! must move, tournaments and promotions, the manager's commands and views, a
//! member's side, and the relay's registry and rooms. Built on [`ducy_play`]'s
//! tables and tournaments, which know nothing of clubs.
//!
//! Everything here is ducy.cards' own: the generic poker engine is `ducy`
//! (cards and evaluation) and `ducy-play` (hands, tables, tournaments).

#![warn(missing_docs)]

pub mod book;
pub mod groups;
pub mod host;
pub mod interest;
pub mod lists;
pub mod member;
pub mod money;
pub mod oneoff;
pub mod overview;
pub mod promos;
pub mod registry;
pub mod room;
pub mod rules;
pub mod seats;
pub mod service;
pub mod tourney_host;
pub mod tourneys;
pub mod views;
