//! Game-theory-optimal poker strategies.
//!
//! In a two-player zero-sum game a Nash equilibrium strategy can't lose in
//! expectation to any opponent, and how far a strategy is from one can be
//! measured as its *exploitability*: what a best-responding opponent wins
//! against it. This crate finds equilibria with counterfactual regret
//! minimization (CFR) and measures them.
//!
//! - [`Game`]: the interface a game implements (chance, players, terminal
//!   payoffs, information sets).
//! - [`Cfr`]: vanilla CFR and CFR+ over the full tree, for small games.
//! - [`exploitability`], [`best_response_value`], [`expected_value`]: exact
//!   evaluation of a [`Profile`].
//! - [`games`]: Kuhn poker and Leduc hold'em, small games with known
//!   equilibrium values for checking the solver.
//!
//! ```
//! use ducy_gto::{Cfr, Variant, exploitability, expected_value, games::kuhn::{Kuhn, GAME_VALUE}};
//!
//! let mut cfr = Cfr::new(&Kuhn, Variant::Plus);
//! cfr.run(1000);
//! let strategy = cfr.average();
//! assert!(exploitability(&Kuhn, &strategy) < 0.001);
//! assert!((expected_value(&Kuhn, &strategy) - GAME_VALUE).abs() < 0.001);
//! ```
//!
//! This is the first step toward a near-GTO heads-up no-limit Hold'em bot for
//! ducy-play; see the README for the plan.

mod cfr;
mod game;
pub mod games;
mod profile;

pub use cfr::{Cfr, Variant};
pub use game::{Game, Turn};
pub use profile::{Profile, best_response_value, expected_value, exploitability};
