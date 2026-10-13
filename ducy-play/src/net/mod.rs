//! Neural-network bots (#215): a small multilayer perceptron that turns the
//! decision features (#214) into a probability for each action class, and
//! `NetBot`, which plays from it.

pub mod actions;

pub use actions::{ActionClass, NUM_CLASSES};
