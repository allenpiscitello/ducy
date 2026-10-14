use crate::error::PlayError;

/// The poker game being played, which decides hole cards and showdown rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Variant {
    /// Texas Hold'em: two hole cards, best five from any seven.
    Holdem,
    /// Omaha (high): `hole_cards` cards (4 for PLO, 5 for PLO5, 6 for PLO6);
    /// a hand uses exactly two of them and three from the board.
    Omaha {
        /// Hole cards per player, 4 to 6.
        hole_cards: u32,
    },
}

impl Variant {
    /// Hole cards dealt to each player.
    pub fn hole_cards(&self) -> usize {
        match self {
            Self::Holdem => 2,
            Self::Omaha { hole_cards } => *hole_cards as usize,
        }
    }
}

/// How much a player may bet or raise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum BettingStructure {
    /// Any amount up to the player's stack.
    NoLimit,
    /// Up to the size of the pot after calling.
    PotLimit,
}

/// The rules for a hand: game, betting structure and forced bets. Amounts are
/// whole chips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TableRules {
    /// The game being played.
    pub variant: Variant,
    /// No-limit or pot-limit.
    pub structure: BettingStructure,
    /// Small blind, posted by the seat after the button (by the button when
    /// heads-up).
    pub small_blind: u64,
    /// Big blind. Also the minimum bet.
    pub big_blind: u64,
    /// Ante posted by every player before the blinds. It goes into the pot
    /// but doesn't count toward calling.
    pub ante: u64,
    /// A bomb pot: everyone posts the ante and nothing else, there's no
    /// preflop betting, and the hand starts on the flop. The big blind is
    /// still the minimum bet. Needs an ante.
    #[cfg_attr(feature = "serde", serde(default))]
    pub bomb_pot: bool,
    /// Two boards from the flop on (double board), dealt from the cards after
    /// the first: each pot is split between the best hand on each board,
    /// as when running it twice. Needs a dealer that shows the engine every
    /// card (a [`crate::Deal`]).
    #[cfg_attr(feature = "serde", serde(default))]
    pub double_board: bool,
}

impl TableRules {
    /// No-limit Hold'em with the given blinds and no ante.
    pub fn no_limit_holdem(small_blind: u64, big_blind: u64) -> Self {
        Self {
            variant: Variant::Holdem,
            structure: BettingStructure::NoLimit,
            small_blind,
            big_blind,
            ante: 0,
            bomb_pot: false,
            double_board: false,
        }
    }

    /// Pot-limit Omaha with four hole cards, the given blinds and no ante.
    pub fn pot_limit_omaha(small_blind: u64, big_blind: u64) -> Self {
        Self {
            variant: Variant::Omaha { hole_cards: 4 },
            structure: BettingStructure::PotLimit,
            small_blind,
            big_blind,
            ante: 0,
            bomb_pot: false,
            double_board: false,
        }
    }

    /// The same rules as a bomb pot (see [`TableRules::bomb_pot`]) with this
    /// ante, on two boards when `double_board`.
    pub fn bomb_pot(mut self, ante: u64, double_board: bool) -> Self {
        self.ante = ante;
        self.bomb_pot = true;
        self.double_board = double_board;
        self
    }

    /// The same rules with an ante.
    pub fn with_ante(mut self, ante: u64) -> Self {
        self.ante = ante;
        self
    }

    /// The same rules with a different variant, e.g. PLO5.
    pub fn with_variant(mut self, variant: Variant) -> Self {
        self.variant = variant;
        self
    }

    /// The same rules with a different betting structure.
    pub fn with_structure(mut self, structure: BettingStructure) -> Self {
        self.structure = structure;
        self
    }

    pub(crate) fn validate(&self) -> Result<(), PlayError> {
        if self.small_blind == 0 || self.big_blind == 0 || self.small_blind > self.big_blind {
            return Err(PlayError::InvalidBlinds);
        }
        if self.bomb_pot && self.ante == 0 {
            return Err(PlayError::InvalidSetup);
        }
        if let Variant::Omaha { hole_cards } = self.variant {
            if !(4..=6).contains(&hole_cards) {
                return Err(PlayError::InvalidDeal);
            }
        }
        Ok(())
    }
}
