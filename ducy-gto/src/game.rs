//! The interface every game implements, from Kuhn poker to abstracted
//! Hold'em: a tree of chance, player and terminal nodes, with players who only
//! see their own information sets.

use std::hash::Hash;

/// Who moves at a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turn {
    /// The game is over; see [`Game::utility`].
    Terminal,
    /// Chance (the deal) moves; see [`Game::chance_outcomes`].
    Chance,
    /// Player 0 or 1 chooses one of [`Game::num_actions`] actions.
    Player(usize),
}

/// A two-player zero-sum game with imperfect information and perfect recall.
pub trait Game {
    /// A node of the game tree, including every card dealt so far.
    type State: Clone;
    /// What the player to act knows: their own cards, the public cards and the
    /// betting. States a player can't tell apart share one key.
    type Info: Clone + Eq + Hash;

    fn root(&self) -> Self::State;
    fn turn(&self, state: &Self::State) -> Turn;
    /// Player 0's payoff at a terminal state; player 1 gets the negation.
    fn utility(&self, state: &Self::State) -> f64;
    /// The states chance can move to and their probabilities (summing to 1).
    fn chance_outcomes(&self, state: &Self::State) -> Vec<(Self::State, f64)>;
    /// How many actions the player to act has (at least 1).
    fn num_actions(&self, state: &Self::State) -> usize;
    fn apply(&self, state: &Self::State, action: usize) -> Self::State;
    /// The information set of the player to act.
    fn info(&self, state: &Self::State) -> Self::Info;
}
