//! Toward GTO-style play at six-handed tables (#245).
//!
//! Most raised six-handed pots are down to two players by the flop, so the
//! plan solves the preflop for six players and reuses the heads-up machinery
//! after it. This module holds the six-handed pieces:
//!
//! - [`equity`]: preflop equity tables over the 169 starting-hand classes.
//! - [`preflop`]: the six-handed preflop game.
//! - [`realize`]: how much of its equity each hand realizes after the flop,
//!   fitted from the heads-up blueprint.
//! - [`solver`]: Monte Carlo CFR for more than two players, on that game.
//! - [`chart`]: opening and defending charts from the solve.
//! - [`bot`]: a six-handed bot playing the solve before the flop.
//! - [`field`]: how a field of players actually plays preflop, recorded on
//!   the tree, for solving against that field.
//! - [`plo`]: six-handed pot-limit Omaha (#290): the PLO abstraction's
//!   preflop buckets as hand classes, their equity, and the dealer.

pub mod bot;
pub mod chart;
pub mod equity;
pub mod field;
pub mod plo;
pub mod plo_spot;
pub mod preflop;
pub mod realize;
pub mod solver;
pub mod spot;
pub mod styled;
