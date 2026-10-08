//! Fast poker hand analysis: cards and decks, hand ranking, winners and
//! equity for Hold'em, Omaha (4, 5 and 6 cards, hi and hi-lo, bomb pots), stud
//! and draw games, and hand ranges.
//!
//! Cards and decks are `u64` bitfields ([`deck::Deck`]), so set operations and
//! enumerating runouts are cheap. Each game has a state you deal into (players'
//! cards, then the board) and an evaluation that finds the winners, or each
//! player's equity over every way the rest of the board can come.
//!
//! # Winners
//!
//! ```
//! use ducy::deck::Deck;
//! use ducy::games::flop_game::FlopGame;
//! use ducy::games::holdem::{HoldemGameEvaluation, HoldemGameState};
//! use ducy::games::GameEvaluation;
//!
//! let mut game = HoldemGameState::new();
//! game.add_player(Deck::parse("As Ac").unwrap()).unwrap();
//! game.add_player(Deck::parse("Ks Kd").unwrap()).unwrap();
//! game.set_flop(Deck::parse("Kc Qd Js").unwrap()).unwrap();
//!
//! let winners = HoldemGameEvaluation {}.evaluate_winners(&game);
//! assert_eq!(winners[0].player_index(), 1); // a set of kings
//! ```
//!
//! # Equity
//!
//! Every remaining runout is enumerated (in parallel with the default
//! `parallel` feature):
//!
//! ```
//! use ducy::deck::{Card, Deck};
//! use ducy::games::flop_game::FlopGame;
//! use ducy::games::omaha::{OmahaGameEvaluation, OmahaGameState};
//! use ducy::games::GameEquityEvaluation;
//!
//! let mut game = OmahaGameState::new(4);
//! game.add_player(Deck::parse("As Ac Jc Ts").unwrap()).unwrap();
//! game.add_player(Deck::parse("9h 8h 7d 6d").unwrap()).unwrap();
//! game.set_flop(Deck::parse("Jh Th Qd").unwrap()).unwrap();
//! game.set_turn(Card::parse("Jd").unwrap()).unwrap();
//!
//! let equity = OmahaGameEvaluation {}.evaluate_equity(&game);
//! assert_eq!(equity.len(), 2);
//! ```
//!
//! # Ranges
//!
//! Hold'em ranges use the usual notation (`"AQo+, 77+"`, see [`deck::range`]);
//! Omaha ranges use hand classes such as double-suited aces or rundowns (see
//! [`games::omaha_range`]):
//!
//! ```
//! use ducy::games::omaha_range::OmahaRange;
//!
//! let range = OmahaRange::parse("AAxx$ds, $rd$ds", 4).unwrap();
//! assert!(range.coverage() > 0.0 && range.coverage() < 0.05);
//! ```
//!
//! # Features
//!
//! - `parallel` (default): equity over every core, with rayon. Without it,
//!   everything still works on one thread (for example in WebAssembly).
//! - `serde`: `Serialize`/`Deserialize` for cards, decks and errors.
//!
//! # Related crates
//!
//! [`ducy-play`](https://docs.rs/ducy-play) plays hands out (blinds, betting,
//! side pots, showdown, tables and bots), built on this crate.

#![warn(missing_docs)]

// The README's examples run as doctests, so the ones on crates.io keep working.
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;

/// Card, deck, and range primitives using bitfield representation.
pub mod deck;
/// Custom error types for the library.
pub mod error;
/// Game state, evaluation, and equity calculation for poker variants.
pub mod games;
/// Hand ranking systems for poker hands.
pub mod ranking;

#[cfg(test)]
pub(crate) mod test_util {
    use crate::deck::{Card, Deck};

    pub fn deck_from_cards(val: &str) -> Deck {
        let card_strs = val.split(" ");
        let cards: Vec<Card> = card_strs.map(|x| Card::parse(x).unwrap()).collect();
        let mut deck = Deck::empty();
        deck.insert_cards(cards.iter());
        deck
    }
}
