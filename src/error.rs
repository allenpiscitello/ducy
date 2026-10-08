use std::fmt;

/// Why a ducy operation failed: bad card text, cards already in use, the
/// wrong number of cards, or an invalid range.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DucyError {
    /// Invalid character for a card rank
    InvalidRank,
    /// Invalid character for a card suit
    InvalidSuit,
    /// Invalid card string representation
    InvalidCard,
    /// Not enough cards in the deck for the requested operation
    NotEnoughCards,
    /// Referenced cards are not available in the remaining deck
    CardsNotAvailable,
    /// Wrong number of cards provided for the operation
    IncorrectCardCount,
    /// Too many community cards (would exceed 5)
    TooManyCommunityCards,
    /// Flop must be set before turn
    FlopNotSet,
    /// Turn must be set before river
    TurnNotSet,
    /// Invalid range specification
    InvalidRange,
    /// Too many players (maximum 10)
    TooManyPlayers,
    /// Board index is out of range
    InvalidBoardIndex,
}

impl fmt::Display for DucyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DucyError::InvalidRank => write!(f, "invalid rank character"),
            DucyError::InvalidSuit => write!(f, "invalid suit character"),
            DucyError::InvalidCard => write!(f, "invalid card string"),
            DucyError::NotEnoughCards => write!(f, "not enough cards in deck"),
            DucyError::CardsNotAvailable => write!(f, "cards not available in deck"),
            DucyError::IncorrectCardCount => write!(f, "incorrect number of cards"),
            DucyError::TooManyCommunityCards => write!(f, "too many community cards"),
            DucyError::FlopNotSet => write!(f, "flop must be set before turn"),
            DucyError::TurnNotSet => write!(f, "turn must be set before river"),
            DucyError::InvalidRange => write!(f, "invalid range specification"),
            DucyError::TooManyPlayers => write!(f, "too many players (maximum 10)"),
            DucyError::InvalidBoardIndex => write!(f, "board index out of range"),
        }
    }
}

impl std::error::Error for DucyError {}
