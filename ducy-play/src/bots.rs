//! Simple built-in bots, mostly as opponents and baselines.

use rand::{RngExt, SeedableRng, rngs::StdRng};

use crate::{
    bot::{Bot, Observation},
    hand::{Action, LegalActions},
    strength::observation_equity,
};

pub(crate) fn rng(seed: Option<u64>) -> StdRng {
    match seed {
        Some(seed) => StdRng::seed_from_u64(seed),
        None => rand::make_rng(),
    }
}

/// Checks when it can, otherwise calls. Never folds, bets or raises.
#[derive(Clone, Copy, Debug, Default)]
pub struct CallingStation;

impl Bot for CallingStation {
    fn act(&mut self, obs: &Observation) -> Option<Action> {
        Some(passive(&obs.legal))
    }
}

fn passive(legal: &LegalActions) -> Action {
    if legal.can_check {
        Action::Check
    } else {
        Action::Call
    }
}

/// Bets or raises the minimum whenever it can, otherwise checks or calls.
#[derive(Clone, Copy, Debug, Default)]
pub struct Raiser;

impl Bot for Raiser {
    fn act(&mut self, obs: &Observation) -> Option<Action> {
        let legal = &obs.legal;
        Some(match (legal.bet, legal.raise) {
            (Some(r), _) => Action::Bet(r.min_to),
            (_, Some(r)) => Action::Raise(r.min_to),
            _ => passive(legal),
        })
    }
}

/// Picks among the legal actions at random: check/call most often, then
/// bet/raise (to a random legal size), then fold.
#[derive(Debug)]
pub struct RandomBot {
    rng: StdRng,
}

impl RandomBot {
    /// A random bot; a `seed` makes its choices reproducible.
    pub fn new(seed: Option<u64>) -> Self {
        Self { rng: rng(seed) }
    }
}

impl Bot for RandomBot {
    fn act(&mut self, obs: &Observation) -> Option<Action> {
        let legal = &obs.legal;
        let roll = self.rng.random_range(0..10);
        if roll < 2 && legal.can_fold {
            return Some(Action::Fold);
        }
        if roll >= 7 {
            if let Some(r) = legal.bet.or(legal.raise) {
                let to = self.rng.random_range(r.min_to..=r.max_to);
                return Some(if legal.bet.is_some() {
                    Action::Bet(to)
                } else {
                    Action::Raise(to)
                });
            }
        }
        Some(passive(legal))
    }
}

/// Estimates its equity against random hands for every opponent still in,
/// then bets or raises about the pot with a strong hand, calls when the pot
/// odds are good enough, and otherwise checks or folds.
///
/// Opponents are assumed to hold random hands, so it overvalues hands
/// against opponents who only continue with strong holdings. It is meant as
/// a sensible baseline, not a strong player.
#[derive(Debug)]
pub struct EquityBot {
    samples: usize,
    margin: f64,
    rng: StdRng,
}

impl EquityBot {
    /// `samples` Monte Carlo runouts per decision (a few hundred is plenty);
    /// a `seed` makes its play reproducible.
    pub fn new(samples: usize, seed: Option<u64>) -> Self {
        Self {
            samples: samples.max(1),
            margin: 0.2,
            rng: rng(seed),
        }
    }

    /// How much above an even share of the pot (1 / players) its equity must
    /// be before it bets or raises. Defaults to 0.2.
    pub fn with_margin(mut self, margin: f64) -> Self {
        self.margin = margin;
        self
    }

    /// Share of the pot this seat wins on average against random hands.
    pub fn equity(&mut self, obs: &Observation) -> f64 {
        observation_equity(obs, self.samples, &mut self.rng)
    }
}

impl Bot for EquityBot {
    fn act(&mut self, obs: &Observation) -> Option<Action> {
        let legal = &obs.legal;
        let players = obs.seats.iter().filter(|s| !s.folded).count().max(1);
        let equity = self.equity(obs);

        if equity > 1.0 / players as f64 + self.margin {
            // About a pot-sized bet or raise, within the legal range.
            let to_call = legal.call.unwrap_or(0);
            let target = obs.current_bet + obs.pot + to_call;
            if let Some(r) = legal.bet {
                return Some(Action::Bet(target.clamp(r.min_to, r.max_to)));
            }
            if let Some(r) = legal.raise {
                return Some(Action::Raise(target.clamp(r.min_to, r.max_to)));
            }
        }
        match legal.call {
            None => Some(Action::Check),
            Some(call) => {
                let pot_odds = call as f64 / (obs.pot + call) as f64;
                Some(if equity >= pot_odds {
                    Action::Call
                } else {
                    Action::Fold
                })
            }
        }
    }
}
