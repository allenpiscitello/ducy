//! The actions a network bot chooses between (#215, #216): fold,
//! check or call, four bet sizes as fractions of the pot, and all-in.
//!
//! A bet or raise of fraction `f` goes to `current_bet + f × (pot + to call)`,
//! the size after calling, clamped to the legal range. A real action maps
//! back to the nearest class ([`ActionClass::of`]), so the rule-based bots'
//! decisions can be recorded as training labels.

use crate::{
    bot::Observation,
    hand::{Action, RaiseRange},
};

/// One of the network's choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ActionClass {
    /// Give up the hand.
    Fold = 0,
    /// Check, or call the bet faced.
    CheckCall = 1,
    /// Bet or raise a third of the pot.
    Third = 2,
    /// Two thirds of the pot.
    TwoThirds = 3,
    /// The pot.
    Pot = 4,
    /// Twice the pot (pot-limit caps it at the pot).
    Overbet = 5,
    /// Every chip.
    AllIn = 6,
}

/// Classes per decision.
pub const NUM_CLASSES: usize = 7;

impl ActionClass {
    /// Every class, in index order.
    pub const ALL: [Self; NUM_CLASSES] = [
        Self::Fold,
        Self::CheckCall,
        Self::Third,
        Self::TwoThirds,
        Self::Pot,
        Self::Overbet,
        Self::AllIn,
    ];

    /// The class with index `i`.
    pub fn from_index(i: usize) -> Option<Self> {
        Self::ALL.get(i).copied()
    }

    /// Its index, 0 to [`NUM_CLASSES`] − 1.
    pub fn index(self) -> usize {
        self as usize
    }

    /// The pot fraction of a sized bet.
    pub fn fraction(self) -> Option<f64> {
        match self {
            Self::Third => Some(1.0 / 3.0),
            Self::TwoThirds => Some(2.0 / 3.0),
            Self::Pot => Some(1.0),
            Self::Overbet => Some(2.0),
            _ => None,
        }
    }

    /// The legal action this class means for `obs`, or `None` when it isn't
    /// possible (folding with nothing to call, betting with no chips behind).
    pub fn action(self, obs: &Observation) -> Option<Action> {
        let legal = &obs.legal;
        match self {
            Self::Fold => legal.can_fold.then_some(Action::Fold),
            Self::CheckCall => {
                if legal.can_check {
                    Some(Action::Check)
                } else {
                    legal.call.map(|_| Action::Call)
                }
            }
            Self::AllIn => {
                let (range, _) = raise_range(obs)?;
                // Pot-limit may cap it below every chip: the biggest bet then.
                if range.max_to >= all_in_to(obs) {
                    Some(Action::AllIn)
                } else {
                    Some(sized(obs, range.max_to))
                }
            }
            _ => {
                let (range, _) = raise_range(obs)?;
                let f = self.fraction().expect("a sized class");
                let to = target(obs, f).clamp(range.min_to, range.max_to);
                if to >= all_in_to(obs) {
                    Some(Action::AllIn)
                } else {
                    Some(sized(obs, to))
                }
            }
        }
    }

    /// Which classes are possible for `obs`.
    pub fn mask(obs: &Observation) -> [bool; NUM_CLASSES] {
        Self::ALL.map(|c| c.action(obs).is_some())
    }

    /// The class nearest to `action` in `obs`: bets and raises by their size
    /// as a fraction of the pot after calling (nearest on a log scale), and
    /// anything that puts every chip in as all-in.
    pub fn of(obs: &Observation, action: Action) -> Self {
        match action {
            Action::Fold => Self::Fold,
            Action::Check | Action::Call => Self::CheckCall,
            Action::AllIn => Self::AllIn,
            Action::Bet(to) | Action::Raise(to) => {
                if to >= all_in_to(obs) {
                    return Self::AllIn;
                }
                let me = &obs.seats[obs.seat];
                let to_call = obs.current_bet.saturating_sub(me.street_bet);
                let size = (to.saturating_sub(obs.current_bet)) as f64;
                let f = (size / (obs.pot + to_call).max(1) as f64).max(0.05);
                [Self::Third, Self::TwoThirds, Self::Pot, Self::Overbet]
                    .into_iter()
                    .min_by(|a, b| {
                        let d = |c: &Self| (c.fraction().unwrap().ln() - f.ln()).abs();
                        d(a).total_cmp(&d(b))
                    })
                    .expect("four sizes")
            }
        }
    }
}

/// The bet or raise range, and whether it's a bet (nothing to call).
fn raise_range(obs: &Observation) -> Option<(RaiseRange, bool)> {
    match (obs.legal.bet, obs.legal.raise) {
        (Some(r), _) => Some((r, true)),
        (None, Some(r)) => Some((r, false)),
        _ => None,
    }
}

/// The street total that puts every chip in.
fn all_in_to(obs: &Observation) -> u64 {
    let me = &obs.seats[obs.seat];
    me.street_bet + me.stack
}

/// The street total for a bet or raise of `f` times the pot after calling.
fn target(obs: &Observation, f: f64) -> u64 {
    let me = &obs.seats[obs.seat];
    let to_call = obs.current_bet.saturating_sub(me.street_bet);
    obs.current_bet + (f * (obs.pot + to_call) as f64).round() as u64
}

fn sized(obs: &Observation, to: u64) -> Action {
    if obs.legal.bet.is_some() {
        Action::Bet(to)
    } else {
        Action::Raise(to)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{Deal, Hand, TableRules, Variant};

    fn first_decision(rules: TableRules, stacks: &[u64]) -> Observation {
        let deal = Deal::random(rules.variant, stacks.len(), Some(3)).unwrap();
        let hand = Hand::new(rules, stacks, 0, deal).unwrap();
        hand.observation(hand.to_act().unwrap()).unwrap()
    }

    #[test]
    fn classes_map_to_legal_actions_and_back() {
        // Three-handed, 100 big blinds, the button opens.
        let obs = first_decision(TableRules::no_limit_holdem(1, 2), &[200, 200, 200]);
        let mask = ActionClass::mask(&obs);
        assert_eq!(mask, [true; NUM_CLASSES]);
        assert_eq!(ActionClass::CheckCall.action(&obs), Some(Action::Call));
        // Pot after calling is 5: a pot-sized raise goes to 2 + 5 = 7.
        assert_eq!(ActionClass::Pot.action(&obs), Some(Action::Raise(7)));
        assert_eq!(ActionClass::AllIn.action(&obs), Some(Action::AllIn));
        for c in ActionClass::ALL {
            let a = c.action(&obs).unwrap();
            assert_eq!(ActionClass::of(&obs, a), c, "{c:?} → {a:?}");
        }
    }

    #[test]
    fn pot_limit_caps_the_overbet_and_all_in() {
        let obs = first_decision(
            TableRules::pot_limit_omaha(1, 2).with_variant(Variant::Omaha { hole_cards: 4 }),
            &[200, 200, 200],
        );
        // The pot limit is 7: the overbet and all-in both raise to it.
        assert_eq!(ActionClass::Overbet.action(&obs), Some(Action::Raise(7)));
        assert_eq!(ActionClass::AllIn.action(&obs), Some(Action::Raise(7)));
    }

    #[test]
    fn short_stacks_go_all_in() {
        let obs = first_decision(TableRules::no_limit_holdem(1, 2), &[5, 200, 200]);
        assert_eq!(ActionClass::Overbet.action(&obs), Some(Action::AllIn));
        assert_eq!(ActionClass::of(&obs, Action::Raise(5)), ActionClass::AllIn);
    }
}
