//! Bots with a playing style: how many hands they play, how aggressive they
//! are, how often they bluff, and how easily they fold. [`Personality`] has
//! ready-made characters; [`Style`] lets you tune your own.

use rand::{RngExt, rngs::StdRng};

use crate::{
    bot::{Bot, HandSummary, Observation},
    bots::rng,
    hand::{Action, Event, LegalActions, Street},
    stats::OpponentModel,
    strength::{observation_equity, preflop_percentile},
};

/// The knobs behind a playing style. Fractions are between 0 and 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    /// Share of starting hands it plays (VPIP): it plays the top `vpip` of
    /// hands, e.g. 0.25 for the best quarter.
    pub vpip: f64,
    /// Share of starting hands it opens with a raise (PFR), from the top.
    /// Hands inside `vpip` but outside `pfr` limp or call.
    pub pfr: f64,
    /// Share of starting hands it re-raises with when facing a raise.
    pub three_bet: f64,
    /// Share of its `vpip` range that continues (calls) against a raise.
    pub defend: f64,
    /// Widens `vpip`, `pfr` and `three_bet` by this fraction on the button
    /// and cutoff, e.g. 0.4 plays 40% more hands in late position.
    pub position_bonus: f64,
    /// Equity above an even share of the pot (1 / players) needed to bet or
    /// raise for value after the flop.
    pub value_margin: f64,
    /// Chance it actually bets or raises a value hand rather than checking
    /// or calling (lower means more slow-playing).
    pub aggression: f64,
    /// Chance it bets a weak hand as a bluff when checked to.
    pub bluff: f64,
    /// Chance it raises as a bluff when bet into.
    pub bluff_raise: f64,
    /// Equity it needs to call, as a multiple of the pot odds: above 1 folds
    /// more than is correct, below 1 calls too much.
    pub call_factor: f64,
    /// Extra equity it wants per pot-sized bet it faces. Its equity is
    /// measured against random hands, but players who bet usually hold
    /// something, so this is how much a bet scares it.
    pub caution: f64,
    /// Bet size as a fraction of the pot.
    pub bet_size: f64,
    /// Opening raise size in big blinds.
    pub open_size: f64,
    /// Whether it adjusts to each opponent's tendencies (see
    /// [`PersonalityBot`]).
    pub exploit: bool,
    /// Monte Carlo deals per equity estimate. More is steadier but slower.
    pub samples: usize,
}

/// Ready-made characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Personality {
    /// Balanced and solid: positional ranges, two-thirds-pot bets with a
    /// bluff for roughly every two value bets, and calls whenever the pot
    /// odds are there. An approximation of game-theory-optimal play, not a
    /// solver.
    DougPoker,
    /// A nit: plays only premium hands, folds to pressure, needs a clear
    /// edge to call, and almost never bluffs.
    OldManCoffee,
    /// Loose and aggressive: plays lots of hands, raises and bluffs often,
    /// and watches how each opponent plays to exploit them: bluffing those
    /// who fold too much, value-betting those who call too much, stealing
    /// from tight players. It sees only what every player sees.
    MisterCheating,
    /// Loose and passive: plays most hands, rarely raises, and calls far too
    /// much.
    MilkKing,
}

impl Personality {
    /// Every personality.
    pub const ALL: [Personality; 4] = [
        Self::DougPoker,
        Self::OldManCoffee,
        Self::MisterCheating,
        Self::MilkKing,
    ];

    /// Display name, e.g. "Doug Poker".
    pub fn name(self) -> &'static str {
        match self {
            Self::DougPoker => "Doug Poker",
            Self::OldManCoffee => "Old Man Coffee",
            Self::MisterCheating => "Mister Cheating",
            Self::MilkKing => "Milk King",
        }
    }

    /// Short identifier, e.g. "doug_poker".
    pub fn id(self) -> &'static str {
        match self {
            Self::DougPoker => "doug_poker",
            Self::OldManCoffee => "old_man_coffee",
            Self::MisterCheating => "mister_cheating",
            Self::MilkKing => "milk_king",
        }
    }

    /// One-line description of how it plays.
    pub fn description(self) -> &'static str {
        match self {
            Self::DougPoker => {
                "Balanced, near-GTO style: solid ranges, mixed bluffs, pot-odds defense."
            }
            Self::OldManCoffee => "Nitty: very few hands, folds easily, rarely bluffs.",
            Self::MisterCheating => {
                "Loose-aggressive: lots of hands, bold plays, exploits each opponent's leaks."
            }
            Self::MilkKing => "Loose-passive: plays almost anything and calls a lot.",
        }
    }

    /// Looks up a personality by [`Self::id`] or [`Self::name`] (any case).
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|p| p.id().eq_ignore_ascii_case(name) || p.name().eq_ignore_ascii_case(name))
    }

    /// The style behind the personality.
    pub fn style(self) -> Style {
        match self {
            Self::DougPoker => Style {
                vpip: 0.24,
                pfr: 0.19,
                three_bet: 0.07,
                defend: 0.6,
                position_bonus: 0.4,
                value_margin: 0.12,
                aggression: 0.75,
                // With 2/3-pot bets a balanced range is about 2 value : 1 bluff.
                bluff: 0.25,
                bluff_raise: 0.05,
                call_factor: 1.0,
                caution: 0.15,
                bet_size: 0.66,
                open_size: 2.5,
                exploit: false,
                samples: 150,
            },
            Self::OldManCoffee => Style {
                vpip: 0.10,
                pfr: 0.08,
                three_bet: 0.03,
                defend: 0.5,
                position_bonus: 0.0,
                value_margin: 0.2,
                aggression: 0.6,
                bluff: 0.02,
                bluff_raise: 0.0,
                call_factor: 1.5,
                caution: 0.3,
                bet_size: 0.5,
                open_size: 3.0,
                exploit: false,
                samples: 150,
            },
            Self::MisterCheating => Style {
                vpip: 0.45,
                pfr: 0.35,
                three_bet: 0.15,
                defend: 0.7,
                position_bonus: 0.3,
                value_margin: 0.08,
                aggression: 0.9,
                bluff: 0.4,
                bluff_raise: 0.15,
                call_factor: 0.9,
                caution: 0.08,
                bet_size: 0.8,
                open_size: 3.0,
                exploit: true,
                samples: 150,
            },
            Self::MilkKing => Style {
                vpip: 0.7,
                pfr: 0.04,
                three_bet: 0.01,
                defend: 0.9,
                position_bonus: 0.0,
                value_margin: 0.25,
                aggression: 0.2,
                bluff: 0.02,
                bluff_raise: 0.0,
                call_factor: 0.5,
                caution: 0.0,
                bet_size: 0.4,
                open_size: 2.0,
                exploit: false,
                samples: 150,
            },
        }
    }

    /// A bot with this personality; a `seed` makes it reproducible.
    pub fn bot(self, seed: Option<u64>) -> PersonalityBot {
        PersonalityBot::new(self.name(), self.style(), seed)
    }
}

/// A bot that plays a [`Style`].
///
/// Preflop it ranks its hand among all starting hands and plays, raises or
/// re-raises the top shares given by the style, wider in late position.
/// After the flop it estimates its equity against random hands for the
/// opponents still in, then bets for value, bluffs, calls or folds by the
/// style's thresholds and frequencies.
///
/// With `exploit` on, it tracks every seat's tendencies from hand histories
/// ([`OpponentModel`]) and, once it has seen enough hands, bluffs more into
/// players who fold too often, stops bluffing and bets bigger for value into
/// players who call too much, steals more against tight tables, and calls
/// down lighter against very aggressive players.
pub struct PersonalityBot {
    name: &'static str,
    style: Style,
    rng: StdRng,
    model: OpponentModel,
}

/// Hands seen before exploit adjustments kick in.
const MIN_HANDS_TO_EXPLOIT: u32 = 10;

impl PersonalityBot {
    /// A bot named `name` playing `style`; a `seed` makes it reproducible.
    pub fn new(name: &'static str, style: Style, seed: Option<u64>) -> Self {
        Self {
            name,
            style,
            rng: rng(seed),
            model: OpponentModel::new(),
        }
    }

    /// The bot's name.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The style it plays.
    pub fn style(&self) -> &Style {
        &self.style
    }

    /// What it has learned about each seat.
    pub fn model(&self) -> &OpponentModel {
        &self.model
    }

    fn chance(&mut self, p: f64) -> bool {
        self.rng.random_range(0.0..1.0) < p
    }

    /// The style it will play in this spot: its own style, adjusted for
    /// the opponents still in the hand when `exploit` is on.
    pub fn style_for(&self, obs: &Observation) -> Style {
        let mut style = self.style;
        if !style.exploit {
            return style;
        }
        let opponents: Vec<_> = (0..obs.seats.len())
            .filter(|&s| s != obs.seat && !obs.seats[s].folded)
            .map(|s| self.model.seat(s))
            .filter(|s| s.hands >= MIN_HANDS_TO_EXPLOIT)
            .collect();
        if opponents.is_empty() {
            return style;
        }
        let n = opponents.len() as f64;
        let fold_to_bet = opponents.iter().map(|s| s.fold_to_bet()).sum::<f64>() / n;
        let vpip = opponents.iter().map(|s| s.vpip()).sum::<f64>() / n;
        let aggression = opponents.iter().map(|s| s.aggression()).sum::<f64>() / n;

        // Bluff in proportion to how often they fold.
        style.bluff = (style.bluff * fold_to_bet / 0.4).clamp(0.0, 0.9);
        style.bluff_raise = (style.bluff_raise * fold_to_bet / 0.4).clamp(0.0, 0.5);
        if fold_to_bet < 0.25 {
            // Calling stations: no bluffs, thinner and bigger value bets.
            style.bluff = 0.0;
            style.bluff_raise = 0.0;
            style.value_margin -= 0.05;
            style.bet_size *= 1.25;
        }
        if vpip < 0.2 {
            // Tight players give up their blinds: steal more.
            style.pfr = (style.pfr + 0.1).min(1.0);
            style.vpip = style.vpip.max(style.pfr);
        }
        if aggression > 2.5 {
            // Maniacs bluff a lot: call them down lighter.
            style.call_factor *= 0.85;
        }
        style
    }

    fn preflop(&mut self, obs: &Observation, style: &Style) -> Action {
        let legal = &obs.legal;
        let n = obs.seats.len();
        let late = obs.seat == obs.button || (n >= 4 && (obs.seat + 1) % n == obs.button);
        let widen = if late {
            1.0 + style.position_bonus
        } else {
            1.0
        };
        let (vpip, pfr, three_bet) = (
            (style.vpip * widen).min(1.0),
            (style.pfr * widen).min(1.0),
            (style.three_bet * widen).min(1.0),
        );
        // Share of starting hands better than this one.
        let top = 1.0
            - preflop_percentile(
                obs.rules.variant,
                obs.hole_cards,
                style.samples,
                &mut self.rng,
            );
        let big_blind = obs.rules.big_blind;
        let raised = obs.history.iter().any(|e| matches!(e, Event::Raise { .. }));

        if !raised {
            if top <= pfr {
                let to = (style.open_size * big_blind as f64).round() as u64;
                if let Some(action) = raise_to(legal, to) {
                    return action;
                }
            }
            if top <= vpip {
                return passive(legal);
            }
            return check_or_fold(legal);
        }

        if top <= three_bet {
            if let Some(action) = raise_to(legal, obs.current_bet * 3) {
                return action;
            }
        }
        if top <= vpip * style.defend {
            return passive(legal);
        }
        check_or_fold(legal)
    }

    fn postflop(&mut self, obs: &Observation, style: &Style) -> Action {
        let legal = &obs.legal;
        let players = obs.seats.iter().filter(|s| !s.folded).count().max(1);
        let equity = observation_equity(obs, style.samples, &mut self.rng);
        let edge = equity - 1.0 / players as f64;
        let pot = obs.pot as f64;

        match legal.call {
            None => {
                let size = (pot * style.bet_size).round() as u64;
                if edge >= style.value_margin && self.chance(style.aggression) {
                    if let Some(action) = bet(legal, size) {
                        return action;
                    }
                }
                // Bluff weak hands, mostly against one or two opponents.
                if edge < 0.0 && players <= 3 && self.chance(style.bluff) {
                    if let Some(action) = bet(legal, size) {
                        return action;
                    }
                }
                Action::Check
            }
            Some(call) => {
                let raise_size =
                    obs.current_bet + ((pot + call as f64) * style.bet_size).round() as u64;
                if edge >= style.value_margin + 0.1 && self.chance(style.aggression) {
                    if let Some(action) = raise_to(legal, raise_size) {
                        return action;
                    }
                }
                if edge < 0.0 && players <= 2 && self.chance(style.bluff_raise) {
                    if let Some(action) = raise_to(legal, raise_size) {
                        return action;
                    }
                }
                let pot_odds = call as f64 / (pot + call as f64);
                // The bet as a fraction of the pot before it was made.
                let bet_fraction = call as f64 / (pot - call as f64).max(1.0);
                let needed = pot_odds * style.call_factor + style.caution * bet_fraction;
                if equity >= needed {
                    Action::Call
                } else {
                    Action::Fold
                }
            }
        }
    }
}

impl Bot for PersonalityBot {
    fn act(&mut self, obs: &Observation) -> Option<Action> {
        let style = self.style_for(obs);
        Some(if obs.street == Street::Preflop {
            self.preflop(obs, &style)
        } else {
            self.postflop(obs, &style)
        })
    }

    fn hand_over(&mut self, summary: &HandSummary) {
        if self.style.exploit {
            self.model
                .record(&summary.history, summary.result.final_stacks.len());
        }
    }
}

fn passive(legal: &LegalActions) -> Action {
    if legal.can_check {
        Action::Check
    } else {
        Action::Call
    }
}

fn check_or_fold(legal: &LegalActions) -> Action {
    if legal.can_check {
        Action::Check
    } else {
        Action::Fold
    }
}

/// A raise to about `to`, kept inside the legal range, or `None` if raising
/// isn't allowed. Opens the betting instead when nobody has bet.
fn raise_to(legal: &LegalActions, to: u64) -> Option<Action> {
    if let Some(r) = legal.raise {
        return Some(Action::Raise(to.clamp(r.min_to, r.max_to)));
    }
    legal.bet.map(|r| Action::Bet(to.clamp(r.min_to, r.max_to)))
}

/// A bet of about `size`, kept inside the legal range, or `None`.
fn bet(legal: &LegalActions, size: u64) -> Option<Action> {
    legal
        .bet
        .map(|r| Action::Bet(size.clamp(r.min_to, r.max_to)))
}
