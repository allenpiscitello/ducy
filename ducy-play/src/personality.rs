//! Bots with a playing style: how many hands they play, how aggressive they
//! are, how often they bluff, how easily they fold, whether they trap, tilt
//! or shove. [`Personality`] has ready-made characters; [`Style`] lets you
//! tune your own.

use std::collections::HashSet;

use ducy::deck::range::Range;
use ducy::{
    deck::Deck, deck::Rank, games::holdem::HoldemRange,
    games::omaha_range_equity::OmahaRangeSampler,
};
use rand::{RngExt, rngs::StdRng};

use crate::{
    bot::{Bot, HandSummary, Observation},
    bots::rng,
    hand::{Action, Event, LegalActions, Street},
    rules::Variant,
    stats::OpponentModel,
    strength::{observation_equity, preflop_percentile},
};

/// The knobs behind a playing style. Fractions are between 0 and 1.
///
/// [`Style::default`] is a solid, balanced player; personalities start from
/// it and change what makes them distinctive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    // --- Preflop ---
    /// Share of starting hands it plays (VPIP): the top `vpip` of hands,
    /// e.g. 0.25 for the best quarter.
    pub vpip: f64,
    /// Share of starting hands it opens with a raise (PFR), from the top.
    /// Hands inside `vpip` but outside `pfr` limp or call.
    pub pfr: f64,
    /// Share of starting hands it re-raises with against a raise (3-bet).
    pub three_bet: f64,
    /// Share of starting hands it raises again with when facing a re-raise
    /// (4-bet). It calls a re-raise with up to 2.5 times this share and
    /// folds the rest, so a tiny value means it folds to re-raises a lot.
    pub four_bet: f64,
    /// Share of its `vpip` range that calls a single raise.
    pub defend: f64,
    /// Widens `vpip`, `pfr` and `three_bet` by this fraction on the button
    /// and cutoff, e.g. 0.4 plays 40% more hands in late position.
    pub position_bonus: f64,
    /// Plays any hand with two cards of the same suit, whatever its rank.
    pub any_suited: bool,
    /// Plays any hand with an ace.
    pub any_ace: bool,
    /// Hands it always raises, in range syntax: Hold'em like `"T2, 72o"`,
    /// Omaha like `"AAxx, T2"` (see `OmahaRange`). Empty for none.
    pub always_play: &'static str,
    /// At or below this many big blinds (its own stack or the biggest
    /// opponent's, whichever is smaller) it only shoves all-in or folds
    /// preflop. 0 turns it off; `f64::INFINITY` shoves or folds every hand.
    pub push_fold_bb: f64,
    /// Opening raise size in big blinds.
    pub open_size: f64,

    // --- After the flop ---
    /// Equity above an even share of the pot (1 / players) needed to bet or
    /// raise for value.
    pub value_margin: f64,
    /// Chance it bets or raises a value hand rather than checking or calling.
    pub aggression: f64,
    /// Chance it slow-plays a monster: checks it (before the river) to
    /// check-raise later, and raises when bet into.
    pub trap: f64,
    /// Bets weak hands and checks strong ones: the bet-or-check decision is
    /// made as if its hand strength were flipped. Calling still uses its
    /// real strength.
    pub backwards: bool,
    /// Chance it bets a weak hand as a bluff when checked to.
    pub bluff: f64,
    /// Chance it raises as a bluff when bet into.
    pub bluff_raise: f64,
    /// Equity it needs to call, as a multiple of the pot odds: above 1 folds
    /// more than is correct, below 1 calls too much.
    pub call_factor: f64,
    /// Multiplies the equity it needs to call when it holds a pair (a pocket
    /// pair or a hole card matching the board). Below 1 makes it sticky.
    pub pair_call_factor: f64,
    /// Extra equity it wants per pot-sized bet it faces. Its equity is
    /// measured against random hands, but players who bet usually hold
    /// something, so this is how much a bet scares it.
    pub caution: f64,
    /// Bet size as a fraction of the pot. Above 1 overbets (no-limit only;
    /// pot-limit caps it at the pot).
    pub bet_size: f64,

    // --- Mood ---
    /// How much a losing hand puts it on tilt, per 40 big blinds lost. On
    /// tilt it plays more hands, bluffs more and calls lighter.
    pub tilt: f64,
    /// How much a winning hand makes it bolder, per 40 big blinds won, in
    /// the same way as tilt.
    pub heater: f64,
    /// Share of its tilt or heater that wears off each hand.
    pub recovery: f64,

    // --- Reading opponents ---
    /// Whether it adjusts to each opponent's tendencies (see
    /// [`PersonalityBot`]).
    pub exploit: bool,
    /// Hands it must see of a seat before adjusting to it.
    pub exploit_after: u32,

    /// Monte Carlo deals per equity estimate. More is steadier but slower.
    pub samples: usize,
}

impl Default for Style {
    /// A solid, balanced regular: about a quarter of hands, positional,
    /// 2/3-pot bets with some bluffs, calls by pot odds.
    fn default() -> Self {
        Self {
            vpip: 0.24,
            pfr: 0.19,
            three_bet: 0.07,
            four_bet: 0.03,
            defend: 0.6,
            position_bonus: 0.4,
            any_suited: false,
            any_ace: false,
            always_play: "",
            push_fold_bb: 0.0,
            open_size: 2.5,
            value_margin: 0.12,
            aggression: 0.75,
            trap: 0.0,
            backwards: false,
            bluff: 0.25,
            bluff_raise: 0.05,
            call_factor: 1.0,
            pair_call_factor: 1.0,
            caution: 0.15,
            bet_size: 0.66,
            tilt: 0.0,
            heater: 0.0,
            recovery: 0.15,
            exploit: false,
            exploit_after: 10,
            samples: 150,
        }
    }
}

/// Ready-made characters. Any resemblance to real players is, of course,
/// purely coincidental.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Personality {
    /// Balanced and solid: positional ranges, 2/3-pot bets with about one
    /// bluff per two value bets, pot-odds defense. Near-GTO in spirit, not a
    /// solver.
    DougPoker,
    /// A nit: premium hands only, folds to pressure, almost never bluffs.
    OldManCoffee,
    /// Loose-aggressive and exploitative: lots of hands, bold plays, and
    /// adjusts to every opponent's leaks.
    MisterCheating,
    /// Loose-passive: plays most hands, rarely raises, calls far too much.
    MilkKing,
    /// Tight and proud until he loses a big pot, then badly on tilt.
    PhilBigmouth,
    /// Splashy loose-aggressive vlogger: lots of hands, big bluffs, hero
    /// calls, and bolder still when he's running hot.
    Rampart,
    /// Small ball: plays lots of hands in small pots with small bets.
    DannySmallball,
    /// Old-school aggression, and always plays ten-deuce.
    DoyleBrunchson,
    /// Lucky amateur who gets bolder with every pot he wins.
    ChrisMoneybags,
    /// Slow-plays monsters and check-raises.
    JohnnyChampagne,
    /// Elite: solid base, reads opponents fast and exploits them.
    IveyLeague,
    /// Never folds a pair, to keep you honest.
    UncleGary,
    /// Every bet is the pot or more.
    CaptainOverbet,
    /// Fearless bluffer who bets his air and checks his monsters.
    GusBluffsen,
    /// Plays any suited hand and any ace, because they're pretty.
    LadyLuckLinda,
    /// Disciplined, relentless grinder who never looks happy about it.
    MichaelMiserable,
}

impl Personality {
    /// Every personality.
    pub const ALL: [Personality; 16] = [
        Self::DougPoker,
        Self::OldManCoffee,
        Self::MisterCheating,
        Self::MilkKing,
        Self::PhilBigmouth,
        Self::Rampart,
        Self::DannySmallball,
        Self::DoyleBrunchson,
        Self::ChrisMoneybags,
        Self::JohnnyChampagne,
        Self::IveyLeague,
        Self::UncleGary,
        Self::CaptainOverbet,
        Self::GusBluffsen,
        Self::LadyLuckLinda,
        Self::MichaelMiserable,
    ];

    /// Display name, e.g. "Doug Poker".
    pub fn name(self) -> &'static str {
        match self {
            Self::DougPoker => "Doug Poker",
            Self::OldManCoffee => "Old Man Coffee",
            Self::MisterCheating => "Mister Cheating",
            Self::MilkKing => "Milk King",
            Self::PhilBigmouth => "Phil Bigmouth",
            Self::Rampart => "Rampart",
            Self::DannySmallball => "Danny Smallball",
            Self::DoyleBrunchson => "Doyle Brunchson",
            Self::ChrisMoneybags => "Chris Moneybags",
            Self::JohnnyChampagne => "Johnny Champagne",
            Self::IveyLeague => "Ivey League",
            Self::UncleGary => "Uncle Gary",
            Self::CaptainOverbet => "Captain Overbet",
            Self::GusBluffsen => "Gus Bluffsen",
            Self::LadyLuckLinda => "Lady Luck Linda",
            Self::MichaelMiserable => "Michael Miserable",
        }
    }

    /// Short identifier, e.g. "doug_poker".
    pub fn id(self) -> &'static str {
        match self {
            Self::DougPoker => "doug_poker",
            Self::OldManCoffee => "old_man_coffee",
            Self::MisterCheating => "mister_cheating",
            Self::MilkKing => "milk_king",
            Self::PhilBigmouth => "phil_bigmouth",
            Self::Rampart => "rampart",
            Self::DannySmallball => "danny_smallball",
            Self::DoyleBrunchson => "doyle_brunchson",
            Self::ChrisMoneybags => "chris_moneybags",
            Self::JohnnyChampagne => "johnny_champagne",
            Self::IveyLeague => "ivey_league",
            Self::UncleGary => "uncle_gary",
            Self::CaptainOverbet => "captain_overbet",
            Self::GusBluffsen => "gus_bluffsen",
            Self::LadyLuckLinda => "lady_luck_linda",
            Self::MichaelMiserable => "michael_miserable",
        }
    }

    /// One-line description of how it plays.
    pub fn description(self) -> &'static str {
        match self {
            Self::DougPoker => "Balanced, near-GTO: solid ranges, mixed bluffs, pot-odds defense.",
            Self::OldManCoffee => "Nitty: very few hands, folds easily, rarely bluffs.",
            Self::MisterCheating => {
                "Loose-aggressive: lots of hands, bold plays, exploits each opponent's leaks."
            }
            Self::MilkKing => "Loose-passive: plays almost anything and calls a lot.",
            Self::PhilBigmouth => "Tight and proud, until a bad beat sends him on monumental tilt.",
            Self::Rampart => {
                "Splashy LAG: lots of hands, big bluffs, hero calls, rides his heaters."
            }
            Self::DannySmallball => "Small ball: many hands, small pots, small bets, tricky calls.",
            Self::DoyleBrunchson => "Old-school aggression, and never folds ten-deuce.",
            Self::ChrisMoneybags => "Lucky amateur who gets braver with every pot he drags.",
            Self::JohnnyChampagne => "Slow-plays the nuts and check-raises you.",
            Self::IveyLeague => "Solid base, reads you in five hands, then exploits you.",
            Self::UncleGary => "Calls with any pair to keep you honest.",
            Self::CaptainOverbet => "Pot or more, every single time.",
            Self::GusBluffsen => "Fearless bluffer: bets his air, checks his monsters.",
            Self::LadyLuckLinda => "Plays any suited hand and any ace, because they're pretty.",
            Self::MichaelMiserable => {
                "Disciplined, relentless grinder who never looks happy about it."
            }
        }
    }

    /// Something it might say at the table.
    pub fn catchphrase(self) -> &'static str {
        match self {
            Self::DougPoker => "It's all just frequencies.",
            Self::OldManCoffee => "I'll wait for aces. I've got time.",
            Self::MisterCheating => "I've seen how you play.",
            Self::MilkKing => "I call. What did you have?",
            Self::PhilBigmouth => "If it weren't for luck, I'd win every hand.",
            Self::Rampart => "I had to see it.",
            Self::DannySmallball => "I put you on exactly king-jack.",
            Self::DoyleBrunchson => "Ten-deuce, baby.",
            Self::ChrisMoneybags => "Wait, I won again?",
            Self::JohnnyChampagne => "Check.",
            Self::IveyLeague => "...",
            Self::UncleGary => "Gotta keep you honest.",
            Self::CaptainOverbet => "Pot.",
            Self::GusBluffsen => "Every hand is a bluff. Except this one.",
            Self::LadyLuckLinda => "They're suited!",
            Self::MichaelMiserable => "Another day at the office. Ugh.",
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
        let base = Style::default();
        match self {
            Self::DougPoker => base,
            Self::OldManCoffee => Style {
                vpip: 0.10,
                pfr: 0.08,
                three_bet: 0.03,
                four_bet: 0.015,
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
                ..base
            },
            Self::MisterCheating => Style {
                vpip: 0.45,
                pfr: 0.35,
                three_bet: 0.15,
                four_bet: 0.06,
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
                ..base
            },
            Self::MilkKing => Style {
                vpip: 0.7,
                pfr: 0.04,
                three_bet: 0.01,
                four_bet: 0.005,
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
                ..base
            },
            Self::PhilBigmouth => Style {
                vpip: 0.15,
                pfr: 0.12,
                three_bet: 0.04,
                position_bonus: 0.2,
                bluff: 0.1,
                call_factor: 1.2,
                caution: 0.25,
                tilt: 0.9,
                recovery: 0.12,
                ..base
            },
            Self::Rampart => Style {
                vpip: 0.38,
                pfr: 0.28,
                three_bet: 0.18,
                four_bet: 0.12,
                defend: 0.8,
                value_margin: 0.06,
                aggression: 0.95,
                bluff: 0.55,
                bluff_raise: 0.25,
                // Hero calls.
                call_factor: 0.8,
                caution: 0.02,
                bet_size: 1.1,
                open_size: 3.0,
                // Bolder on a heater, a little tilty after big losses.
                heater: 0.3,
                tilt: 0.2,
                recovery: 0.2,
                ..base
            },
            Self::DannySmallball => Style {
                vpip: 0.35,
                pfr: 0.25,
                three_bet: 0.05,
                defend: 0.85,
                position_bonus: 0.5,
                value_margin: 0.1,
                aggression: 0.7,
                bluff: 0.25,
                bluff_raise: 0.03,
                call_factor: 0.95,
                caution: 0.1,
                bet_size: 0.33,
                open_size: 2.2,
                ..base
            },
            Self::DoyleBrunchson => Style {
                vpip: 0.35,
                pfr: 0.3,
                three_bet: 0.12,
                four_bet: 0.05,
                value_margin: 0.08,
                aggression: 0.9,
                bluff: 0.35,
                bluff_raise: 0.1,
                caution: 0.08,
                bet_size: 0.9,
                open_size: 3.0,
                always_play: "T2",
                ..base
            },
            Self::ChrisMoneybags => Style {
                vpip: 0.45,
                pfr: 0.08,
                three_bet: 0.02,
                defend: 0.85,
                position_bonus: 0.0,
                value_margin: 0.15,
                aggression: 0.4,
                bluff: 0.05,
                call_factor: 0.7,
                caution: 0.05,
                bet_size: 0.6,
                heater: 0.9,
                recovery: 0.1,
                ..base
            },
            Self::JohnnyChampagne => Style {
                vpip: 0.2,
                pfr: 0.15,
                aggression: 0.5,
                trap: 0.75,
                bluff: 0.1,
                ..base
            },
            Self::IveyLeague => Style {
                vpip: 0.28,
                pfr: 0.22,
                three_bet: 0.09,
                bluff: 0.3,
                exploit: true,
                exploit_after: 5,
                ..base
            },
            Self::UncleGary => Style {
                vpip: 0.5,
                pfr: 0.05,
                three_bet: 0.01,
                four_bet: 0.005,
                defend: 0.9,
                position_bonus: 0.0,
                value_margin: 0.2,
                aggression: 0.3,
                bluff: 0.03,
                call_factor: 0.8,
                pair_call_factor: 0.15,
                caution: 0.05,
                bet_size: 0.5,
                ..base
            },
            Self::CaptainOverbet => Style {
                vpip: 0.35,
                pfr: 0.3,
                three_bet: 0.1,
                aggression: 0.9,
                bluff: 0.3,
                bet_size: 3.0,
                open_size: 5.0,
                caution: 0.1,
                ..base
            },
            Self::GusBluffsen => Style {
                vpip: 0.35,
                pfr: 0.25,
                backwards: true,
                aggression: 0.9,
                bluff: 0.15,
                bluff_raise: 0.1,
                bet_size: 0.75,
                ..base
            },
            Self::LadyLuckLinda => Style {
                vpip: 0.12,
                pfr: 0.06,
                any_suited: true,
                any_ace: true,
                position_bonus: 0.0,
                aggression: 0.5,
                bluff: 0.1,
                call_factor: 0.9,
                ..base
            },
            Self::MichaelMiserable => Style {
                vpip: 0.22,
                pfr: 0.18,
                bluff: 0.15,
                caution: 0.2,
                ..base
            },
        }
    }

    /// A bot with this personality; a `seed` makes it reproducible.
    pub fn bot(self, seed: Option<u64>) -> PersonalityBot {
        PersonalityBot::new(self.name(), self.style(), seed)
    }
}

/// Hands from [`Style::always_play`], parsed for one variant.
enum Favorites {
    Holdem(HashSet<Deck>),
    Omaha(Box<OmahaRangeSampler>),
    None,
}

impl Favorites {
    fn parse(terms: &str, variant: Variant) -> Self {
        if terms.trim().is_empty() {
            return Self::None;
        }
        match variant {
            Variant::Holdem => HoldemRange::parse(terms)
                .map(|r| Self::Holdem(r.iter().map(|item| item.get_deck()).collect()))
                .unwrap_or(Self::None),
            Variant::Omaha { hole_cards } => OmahaRangeSampler::parse(terms, hole_cards as usize)
                .map(|r| Self::Omaha(Box::new(r)))
                .unwrap_or(Self::None),
        }
    }

    fn contains(&self, hand: Deck) -> bool {
        match self {
            Self::Holdem(hands) => hands.contains(&hand),
            Self::Omaha(range) => range.contains(hand),
            Self::None => false,
        }
    }
}

/// A bot that plays a [`Style`].
///
/// - **Preflop** it ranks its hand among all starting hands and plays,
///   raises, re-raises or 4-bets the top shares given by the style (wider in
///   late position), plus any `any_suited` / `any_ace` / `always_play`
///   hands. In push/fold mode it shoves or folds.
/// - **After the flop** it estimates its equity against random hands for
///   the opponents still in, then bets for value, slow-plays, bluffs, calls
///   or folds by the style's thresholds and frequencies.
/// - **Mood:** big losses (`tilt`) and wins (`heater`) make it looser and
///   bolder for a while; it calms down by `recovery` each hand.
/// - **Exploiting:** with `exploit` on it tracks every seat's tendencies
///   ([`OpponentModel`]) and, after `exploit_after` hands, bluffs more into
///   players who fold too often, stops bluffing and bets bigger into players
///   who call too much, steals more against tight players, and calls down
///   lighter against very aggressive ones.
pub struct PersonalityBot {
    name: &'static str,
    style: Style,
    rng: StdRng,
    model: OpponentModel,
    mood: f64,
    favorites: Option<(Variant, Favorites)>,
}

impl PersonalityBot {
    /// A bot named `name` playing `style`; a `seed` makes it reproducible.
    pub fn new(name: &'static str, style: Style, seed: Option<u64>) -> Self {
        Self {
            name,
            style,
            rng: rng(seed),
            model: OpponentModel::new(),
            mood: 0.0,
            favorites: None,
        }
    }

    /// The bot's name.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The style it plays when calm and before adjusting to opponents.
    pub fn style(&self) -> &Style {
        &self.style
    }

    /// What it has learned about each seat.
    pub fn model(&self) -> &OpponentModel {
        &self.model
    }

    /// How tilted (or on a heater) it is, from 0 (calm) to 1.
    pub fn mood(&self) -> f64 {
        self.mood
    }

    fn chance(&mut self, p: f64) -> bool {
        self.rng.random_range(0.0..1.0) < p
    }

    /// The style it will play in this spot: its own style, loosened by its
    /// mood, and adjusted for the opponents still in the hand when `exploit`
    /// is on.
    pub fn style_for(&self, obs: &Observation) -> Style {
        let mut style = self.style;

        let m = self.mood;
        if m > 0.0 {
            style.vpip = (style.vpip * (1.0 + m)).min(0.9);
            style.pfr = (style.pfr * (1.0 + m)).min(style.vpip);
            style.three_bet = (style.three_bet * (1.0 + m)).min(style.pfr);
            style.bluff = (style.bluff + 0.4 * m).min(0.9);
            style.bluff_raise = (style.bluff_raise + 0.2 * m).min(0.5);
            style.call_factor *= 1.0 - 0.4 * m;
            style.caution *= 1.0 - m;
        }

        if !style.exploit {
            return style;
        }
        let opponents: Vec<_> = (0..obs.seats.len())
            .filter(|&s| s != obs.seat && !obs.seats[s].folded)
            .map(|s| self.model.seat(s))
            .filter(|s| s.hands >= style.exploit_after)
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

    fn is_favorite(&mut self, obs: &Observation) -> bool {
        let variant = obs.rules.variant;
        if self.style.always_play.is_empty() {
            return false;
        }
        if !matches!(&self.favorites, Some((v, _)) if *v == variant) {
            self.favorites = Some((variant, Favorites::parse(self.style.always_play, variant)));
        }
        self.favorites
            .as_ref()
            .is_some_and(|(_, f)| f.contains(obs.hole_cards))
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

        // Share of starting hands better than this one, pulled up for hands
        // it plays regardless of rank.
        let mut top = 1.0
            - preflop_percentile(
                obs.rules.variant,
                obs.hole_cards,
                style.samples,
                &mut self.rng,
            );
        if (style.any_suited && has_suited(obs.hole_cards))
            || (style.any_ace && has_ace(obs.hole_cards))
        {
            // Played whatever happens before it: call raises with them too.
            top = top.min(vpip * style.defend);
        }
        if self.is_favorite(obs) {
            top = top.min(pfr);
        }

        let big_blind = obs.rules.big_blind;
        let raises = obs
            .history
            .iter()
            .filter(|e| matches!(e, Event::Raise { .. }))
            .count();

        if style.push_fold_bb > 0.0
            && effective_stack(obs) as f64 <= style.push_fold_bb * big_blind as f64
        {
            let shove = match raises {
                0 => top <= vpip,
                _ => top <= vpip * style.defend,
            };
            return if shove {
                all_in(obs)
            } else {
                check_or_fold(legal)
            };
        }

        match raises {
            0 => {
                if top <= pfr {
                    let to = (style.open_size * big_blind as f64).round() as u64;
                    if let Some(action) = raise_to(legal, to) {
                        return action;
                    }
                }
                if top <= vpip {
                    return passive(legal);
                }
            }
            1 => {
                if top <= three_bet {
                    if let Some(action) = raise_to(legal, obs.current_bet * 3) {
                        return action;
                    }
                }
                if top <= vpip * style.defend {
                    return passive(legal);
                }
            }
            _ => {
                if top <= style.four_bet {
                    let to = (obs.current_bet as f64 * 2.5).round() as u64;
                    if let Some(action) = raise_to(legal, to) {
                        return action;
                    }
                }
                if top <= style.four_bet * 2.5 {
                    return passive(legal);
                }
            }
        }
        check_or_fold(legal)
    }

    fn postflop(&mut self, obs: &Observation, style: &Style) -> Action {
        let legal = &obs.legal;
        let players = obs.seats.iter().filter(|s| !s.folded).count().max(1);
        let equity = observation_equity(obs, style.samples, &mut self.rng);
        let edge = equity - 1.0 / players as f64;
        // What the bet-or-check decision sees: flipped for backwards players.
        let bet_edge = if style.backwards { -edge } else { edge };
        let monster = !style.backwards && edge >= style.value_margin + 0.2;
        let pot = obs.pot as f64;

        match legal.call {
            None => {
                let size = (pot * style.bet_size).round() as u64;
                if monster && obs.street != Street::River && self.chance(style.trap) {
                    // Slow-play: check now, raise if bet into.
                    return Action::Check;
                }
                if bet_edge >= style.value_margin && self.chance(style.aggression) {
                    if let Some(action) = bet(legal, size) {
                        return action;
                    }
                }
                // Bluff weak hands, mostly against one or two opponents.
                if bet_edge < 0.0 && players <= 3 && self.chance(style.bluff) {
                    if let Some(action) = bet(legal, size) {
                        return action;
                    }
                }
                Action::Check
            }
            Some(call) => {
                let raise_size =
                    obs.current_bet + ((pot + call as f64) * style.bet_size).round() as u64;
                let raise_chance = if monster {
                    style.aggression.max(style.trap)
                } else {
                    style.aggression
                };
                if bet_edge >= style.value_margin + 0.1 && self.chance(raise_chance) {
                    if let Some(action) = raise_to(legal, raise_size) {
                        return action;
                    }
                }
                if bet_edge < 0.0 && players <= 2 && self.chance(style.bluff_raise) {
                    if let Some(action) = raise_to(legal, raise_size) {
                        return action;
                    }
                }
                let pot_odds = call as f64 / (pot + call as f64);
                // The bet as a fraction of the pot before it was made.
                let bet_fraction = call as f64 / (pot - call as f64).max(1.0);
                let mut needed = pot_odds * style.call_factor + style.caution * bet_fraction;
                if has_pair(obs) {
                    needed *= style.pair_call_factor;
                }
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
        let big_blind = summary.rules.big_blind.max(1) as f64;
        let net = summary.result.net.get(summary.seat).copied().unwrap_or(0) as f64 / big_blind;
        self.mood *= 1.0 - self.style.recovery;
        let swing = (net.abs() / 40.0).min(1.0);
        self.mood += if net < 0.0 {
            self.style.tilt * swing
        } else {
            self.style.heater * swing
        };
        self.mood = self.mood.clamp(0.0, 1.0);
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

/// Chips behind for the acting seat that can actually be won or lost: its
/// stack or the biggest opponent stack still in, whichever is smaller.
fn effective_stack(obs: &Observation) -> u64 {
    let own = obs.seats[obs.seat].stack + obs.seats[obs.seat].street_bet;
    let biggest = obs
        .seats
        .iter()
        .enumerate()
        .filter(|&(s, v)| s != obs.seat && !v.folded)
        .map(|(_, v)| v.stack + v.street_bet)
        .max()
        .unwrap_or(0);
    own.min(biggest)
}

/// Every chip in: a shove if the rules allow, otherwise the biggest raise
/// (pot-limit), otherwise a call.
fn all_in(obs: &Observation) -> Action {
    let legal = &obs.legal;
    let all_in_to = obs.seats[obs.seat].stack + obs.seats[obs.seat].street_bet;
    match (legal.bet, legal.raise) {
        (Some(r), _) if r.max_to == all_in_to => Action::AllIn,
        (_, Some(r)) if r.max_to == all_in_to => Action::AllIn,
        (Some(r), _) => Action::Bet(r.max_to),
        (_, Some(r)) => Action::Raise(r.max_to),
        _ => passive(legal),
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

fn has_suited(hand: Deck) -> bool {
    let mut seen = [false; 4];
    hand.iter(false).any(|c| {
        let suit = c.suit() as usize;
        std::mem::replace(&mut seen[suit], true)
    })
}

fn has_ace(hand: Deck) -> bool {
    hand.iter(false).any(|c| c.rank() == Rank::Ace)
}

/// A pocket pair, or a hole card that pairs the board.
fn has_pair(obs: &Observation) -> bool {
    let mut seen = [false; 13];
    for card in obs.hole_cards.iter(false) {
        let rank = card.rank() as usize;
        if std::mem::replace(&mut seen[rank], true) {
            return true;
        }
    }
    obs.board.iter().any(|c| seen[c.rank() as usize])
}
