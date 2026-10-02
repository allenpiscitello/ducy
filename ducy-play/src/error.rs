use std::fmt;

/// Errors from setting up or playing a hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayError {
    /// A hand needs 2 to 10 players.
    InvalidPlayerCount,
    /// Every player needs a positive stack, and the button a valid seat.
    InvalidSetup,
    /// Blinds must be positive with the small blind no larger than the big blind.
    InvalidBlinds,
    /// The deal has the wrong number of cards or repeats a card.
    InvalidDeal,
    /// Not enough cards in the deck for this many players.
    NotEnoughCards,
    /// The hand is over; no one is left to act.
    HandComplete,
    /// The action isn't allowed for the player to act (for example checking
    /// while facing a bet, or a raise outside the legal range).
    IllegalAction,
}

impl fmt::Display for PlayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self {
            Self::InvalidPlayerCount => "a hand needs 2 to 10 players",
            Self::InvalidSetup => "stacks must be positive and the button a valid seat",
            Self::InvalidBlinds => {
                "blinds must be positive and the small blind at most the big blind"
            }
            Self::InvalidDeal => "the deal has the wrong number of cards or repeats a card",
            Self::NotEnoughCards => "not enough cards for this many players",
            Self::HandComplete => "the hand is complete",
            Self::IllegalAction => "that action is not legal now",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for PlayError {}
