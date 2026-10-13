//! Neural-network bots (#215): a small multilayer perceptron that turns the
//! decision features (#214) into a probability for each action class, and
//! `NetBot`, which plays from it.

pub mod actions;
mod bot;
mod mlp;

pub use actions::{ActionClass, NUM_CLASSES};
pub use bot::NetBot;
pub use mlp::{Mlp, NetError};

/// The default shape: the decision features, two hidden layers of 64, an
/// output per action class.
pub const DEFAULT_SIZES: [usize; 4] = [crate::features::NUM_FEATURES, 64, 64, NUM_CLASSES];
