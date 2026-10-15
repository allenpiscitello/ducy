//! Toward GTO-style play at six-handed tables (#245).
//!
//! Most raised six-handed pots are down to two players by the flop, so the
//! plan solves the preflop for six players and reuses the heads-up machinery
//! after it. This module holds the six-handed pieces:
//!
//! - [`equity`]: preflop equity tables over the 169 starting-hand classes.
//! - [`preflop`]: the six-handed preflop game.
//! - [`solver`]: Monte Carlo CFR for more than two players, on that game.
//! - [`chart`]: opening and defending charts from the solve.

pub mod chart;
pub mod equity;
pub mod preflop;
pub mod solver;
