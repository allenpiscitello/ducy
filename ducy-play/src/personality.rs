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
    ranges::{has_nuts, range_equity},
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
    //
    // `vpip`, `pfr`, `three_bet` and `four_bet` are baselines for a full
    // 9-handed table in middle position. At shorter tables they widen (see
    // `scale_for_table`), and they widen in position and narrow out of
    // position by `position_bonus`.
    /// Share of starting hands it plays (VPIP) 9-handed: the top `vpip` of
    /// hands, e.g. 0.25 for the best quarter.
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
    /// How much position changes its ranges: on the button it plays
    /// `1 + position_bonus` times its range, first to act after the flop
    /// (small blind) `1 - position_bonus` times, scaling in between. The big
    /// blind counts as middle position, since it closes the preflop action
    /// at a discount. 0.4 means 40% wider on the button, 40% narrower in the
    /// small blind.
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
    /// Chance it checks a value hand (not only a monster) to check-raise,
    /// when someone is still to act behind it. Having checked, when bet into
    /// it raises its value hands (as often as it would have bet them), and
    /// heads-up sometimes a weak one as a bluff.
    pub check_raise: f64,
    /// Chance it bets a weak hand as a bluff when checked to.
    pub bluff: f64,
    /// Chance it raises as a bluff when bet into.
    pub bluff_raise: f64,
    /// Plays the river polarized ([`RIVER_VALUE_EXTRA`]): bets and raises
    /// strong hands for value and hands with no showdown value as bluffs, and
    /// checks the ones between. Off, it bets the river like any other street.
    pub polar_river: bool,
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

    // --- By game ---
    /// How much looser and more aggressive it plays in Omaha than in
    /// Hold'em: 0 plays the same; 0.5 plays and raises half again as many
    /// hands, re-raises more, and bets and bluffs a little more often.
    pub omaha: f64,

    // --- Reading opponents ---
    /// Whether it adjusts to each opponent's tendencies (see
    /// [`PersonalityBot`]).
    pub exploit: bool,
    /// Hands it must see of a seat before adjusting to it.
    pub exploit_after: u32,

    /// Monte Carlo deals per equity estimate. More is steadier but slower.
    pub samples: usize,
    /// Judges its hand after the flop against what each opponent probably
    /// holds, from their actions this hand ([`crate::ranges`]), instead of
    /// against random hands. Then `caution` counts for less
    /// ([`RANGE_CAUTION`]): a bet already narrows the range.
    pub ranges: bool,
}

/// How much of `caution` still applies when judging against ranges.
pub const RANGE_CAUTION: f64 = 0.35;

impl Default for Style {
    /// A solid, balanced regular: about a quarter of hands, positional,
    /// 2/3-pot bets with some bluffs, calls by pot odds.
    fn default() -> Self {
        Self {
            vpip: 0.17,
            pfr: 0.13,
            three_bet: 0.05,
            four_bet: 0.02,
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
            check_raise: 0.0,
            bluff: 0.25,
            bluff_raise: 0.05,
            polar_river: true,
            call_factor: 1.0,
            pair_call_factor: 1.0,
            caution: 0.15,
            bet_size: 0.66,
            tilt: 0.0,
            heater: 0.0,
            recovery: 0.15,
            omaha: 0.0,
            exploit: false,
            exploit_after: 10,
            samples: 150,
            ranges: true,
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
    /// A rock: tight and passive. Plays few hands, limps and check-calls
    /// rather than raising, folds to pressure, almost never bluffs. (Old Man
    /// Coffee until 2026-10.)
    JonnySlow,
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
    /// Lucky amateur who gets bolder with every pot he wins.
    ChrisMoneybags,
    /// Loose-passive hero caller: plays lots of hands, rarely raises, and
    /// never folds a pair, to keep you honest. (Uncle Gary until 2026-10.)
    RobinJaneLewd,
    /// Fearless bluffer: bets his good hands and a lot of his air.
    GusBluffsen,
    /// Plays any suited hand and any ace, because they're pretty. (Lady Luck
    /// Linda until 2026-10.)
    JenSilly,
    /// Disciplined, relentless grinder who never looks happy about it.
    MichaelMiserable,
    /// Sees lots of flops and gives up when he misses. Pocket jacks are
    /// always in play.
    BradOwned,
    /// A true maniac: raises almost everything, bluffs huge, never slows down.
    NikAirbag,
    /// Wild and unpredictable: plays any suited hand, traps one hand and
    /// fires huge bluffs the next.
    Bungleman,
    /// The table captain: raises nearly every pot nobody has raised, limped
    /// ones included, and bets big whenever he's checked to. Pushes back, and
    /// he gets out of the way: careful against a raise before the flop, and
    /// folds to one after it unless he's strong.
    TomCollins,
    /// The theorist: plays by the numbers. Tight and strongly positional
    /// starting hands, raise or fold when first in, calls by the pot odds
    /// alone, bluffs at a balanced ratio, and never tilts.
    TheMathematician,
    /// Straightforward and reasonable, and happiest at the Omaha table: in
    /// pot-limit Omaha he plays more hands, and plays them harder.
    AndrewFavorable,
    /// Aggressive, but a bit more careful than the loose-aggressive crowd:
    /// raises a fair range, bets his good hands, and above all loves to
    /// check-raise. Losing a pot gets to him, and he steams for a while.
    TommySweeden,
}

impl Personality {
    /// Every personality.
    pub const ALL: [Personality; 19] = [
        Self::DougPoker,
        Self::JonnySlow,
        Self::MisterCheating,
        Self::MilkKing,
        Self::PhilBigmouth,
        Self::Rampart,
        Self::DannySmallball,
        Self::ChrisMoneybags,
        Self::RobinJaneLewd,
        Self::GusBluffsen,
        Self::JenSilly,
        Self::MichaelMiserable,
        Self::BradOwned,
        Self::NikAirbag,
        Self::Bungleman,
        Self::TomCollins,
        Self::TheMathematician,
        Self::AndrewFavorable,
        Self::TommySweeden,
    ];

    /// Display name, e.g. "Doug Poker".
    pub fn name(self) -> &'static str {
        match self {
            Self::DougPoker => "Doug Poker",
            Self::JonnySlow => "Jonny Slow",
            Self::MisterCheating => "Mister Cheating",
            Self::MilkKing => "Milk King",
            Self::PhilBigmouth => "Phil Bigmouth",
            Self::Rampart => "Rampart",
            Self::DannySmallball => "Danny Smallball",
            Self::ChrisMoneybags => "Chris Moneybags",
            Self::RobinJaneLewd => "Robin Jane Lewd",
            Self::GusBluffsen => "Gus Bluffsen",
            Self::JenSilly => "Jen Silly",
            Self::MichaelMiserable => "Michael Miserable",
            Self::BradOwned => "Brad Owned",
            Self::NikAirbag => "Nik Airbag",
            Self::Bungleman => "Bungleman",
            Self::TomCollins => "Tom Collins",
            Self::TheMathematician => "The Mathematician",
            Self::AndrewFavorable => "Andrew Favorable",
            Self::TommySweeden => "Tommy Sweeden",
        }
    }

    /// Short identifier, e.g. "doug_poker".
    pub fn id(self) -> &'static str {
        match self {
            Self::DougPoker => "doug_poker",
            Self::JonnySlow => "jonny_slow",
            Self::MisterCheating => "mister_cheating",
            Self::MilkKing => "milk_king",
            Self::PhilBigmouth => "phil_bigmouth",
            Self::Rampart => "rampart",
            Self::DannySmallball => "danny_smallball",
            Self::ChrisMoneybags => "chris_moneybags",
            Self::RobinJaneLewd => "robin_jane_lewd",
            Self::GusBluffsen => "gus_bluffsen",
            Self::JenSilly => "jen_silly",
            Self::MichaelMiserable => "michael_miserable",
            Self::BradOwned => "brad_owned",
            Self::NikAirbag => "nik_airbag",
            Self::Bungleman => "bungleman",
            Self::TomCollins => "tom_collins",
            Self::TheMathematician => "the_mathematician",
            Self::AndrewFavorable => "andrew_favorable",
            Self::TommySweeden => "tommy_sweeden",
        }
    }

    /// One-line description of how it plays.
    pub fn description(self) -> &'static str {
        match self {
            Self::DougPoker => "Balanced, near-GTO: solid ranges, mixed bluffs, pot-odds defense.",
            Self::JonnySlow => {
                "A rock: few hands, limps and check-calls, folds to pressure, never bluffs."
            }
            Self::MisterCheating => {
                "Loose-aggressive: lots of hands, bold plays, exploits each opponent's leaks."
            }
            Self::MilkKing => "Gambler: plays almost anything and loves to get the money in.",
            Self::PhilBigmouth => "Tight and proud, until a bad beat sends him on monumental tilt.",
            Self::Rampart => {
                "Splashy LAG: lots of hands, big bluffs, hero calls, rides his heaters."
            }
            Self::DannySmallball => "Small ball: many hands, small pots, small bets, tricky calls.",
            Self::ChrisMoneybags => "Lucky amateur who gets braver with every pot he drags.",
            Self::RobinJaneLewd => {
                "Hero caller: plays lots of hands, calls with any pair to keep you honest."
            }
            Self::GusBluffsen => "Fearless bluffer: bets his good hands and a lot of his air.",
            Self::JenSilly => "Plays any suited hand and any ace, because they're pretty.",
            Self::MichaelMiserable => {
                "Disciplined, relentless grinder who never looks happy about it."
            }
            Self::BradOwned => {
                "Fit-or-fold: lots of flops, gives up when he misses, always plays jacks."
            }
            Self::NikAirbag => "Maniac: raises almost everything and bluffs huge.",
            Self::Bungleman => "Mostly solid, then out of nowhere a trap or a huge bluff.",
            Self::TomCollins => {
                "Table captain: takes charge of every pot, until someone pushes back."
            }
            Self::TheMathematician => {
                "Theorist: tight and positional, calls by the odds, bluffs at a balanced ratio, never tilts."
            }
            Self::AndrewFavorable => {
                "Straightforward and reasonable; loves PLO, where he plays more hands and plays them harder."
            }
            Self::TommySweeden => {
                "Aggressive but careful; loves a check-raise, and steams after a lost pot."
            }
        }
    }

    /// Something it might say at the table.
    pub fn catchphrase(self) -> &'static str {
        match self {
            Self::DougPoker => "It's all just frequencies.",
            Self::JonnySlow => "I'll wait for aces. I've got time.",
            Self::MisterCheating => "I've seen how you play.",
            Self::MilkKing => "Let's get it all in and see.",
            Self::PhilBigmouth => "If it weren't for luck, I'd win every hand.",
            Self::Rampart => "I had to see it.",
            Self::DannySmallball => "I put you on exactly king-jack.",
            Self::ChrisMoneybags => "Wait, I won again?",
            Self::RobinJaneLewd => "Gotta keep you honest.",
            Self::GusBluffsen => "Every hand is a bluff. Except this one.",
            Self::JenSilly => "They're suited!",
            Self::MichaelMiserable => "Another day at the office. Ugh.",
            Self::BradOwned => "Jacks again? Of course.",
            Self::NikAirbag => "Let's gamble.",
            Self::Bungleman => "I'm feeling it.",
            Self::TomCollins => "I'll take that.",
            Self::TheMathematician => {
                "Every time you play differently from how you would if you could see my cards, I gain."
            }
            Self::AndrewFavorable => "Favorable.",
            Self::TommySweeden => "Go ahead, bet.",
        }
    }

    /// Looks up a personality by [`Self::id`] or [`Self::name`] (any case),
    /// or by a former id or name, so saved tables and clubs still find a
    /// renamed personality.
    pub fn from_name(name: &str) -> Option<Self> {
        const FORMER: [(&str, Personality); 6] = [
            ("uncle_gary", Personality::RobinJaneLewd),
            ("Uncle Gary", Personality::RobinJaneLewd),
            ("old_man_coffee", Personality::JonnySlow),
            ("Old Man Coffee", Personality::JonnySlow),
            ("lady_luck_linda", Personality::JenSilly),
            ("Lady Luck Linda", Personality::JenSilly),
        ];
        if let Some(&(_, p)) = FORMER.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
            return Some(p);
        }
        Self::ALL
            .into_iter()
            .find(|p| p.id().eq_ignore_ascii_case(name) || p.name().eq_ignore_ascii_case(name))
    }

    /// The style behind the personality.
    pub fn style(self) -> Style {
        let base = Style::default();
        match self {
            Self::DougPoker => base,
            Self::JonnySlow => Style {
                // Tight preflop, but limps most of what he plays.
                vpip: 0.1,
                pfr: 0.03,
                three_bet: 0.013,
                four_bet: 0.007,
                defend: 0.5,
                position_bonus: 0.15,
                // Passive after the flop: bets only strong hands, and not
                // often; mostly checks and calls.
                value_margin: 0.25,
                aggression: 0.3,
                bluff: 0.01,
                bluff_raise: 0.0,
                // Folds to pressure without a clear edge.
                call_factor: 1.3,
                caution: 0.3,
                bet_size: 0.52,
                open_size: 3.0,
                ..base
            },
            Self::MisterCheating => Style {
                vpip: 0.33,
                pfr: 0.25,
                three_bet: 0.1,
                four_bet: 0.04,
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
                // A gambler: plays most hands, raises and re-raises plenty
                // of them, and calls a re-raise with almost anything.
                vpip: 0.55,
                pfr: 0.2,
                three_bet: 0.07,
                four_bet: 0.05,
                defend: 0.875,
                position_bonus: 0.15,
                // Any decent piece of the board is worth betting, and big;
                // draws get raised. (Tuned, #206.)
                value_margin: 0.07,
                aggression: 0.85,
                bluff: 0.2,
                bluff_raise: 0.15,
                // Hates to fold once the money's going in.
                call_factor: 0.65,
                pair_call_factor: 0.6,
                caution: 0.0,
                bet_size: 1.0,
                heater: 0.3,
                open_size: 2.0,
                ..base
            },
            Self::PhilBigmouth => Style {
                vpip: 0.1,
                pfr: 0.08,
                three_bet: 0.03,
                position_bonus: 0.2,
                bluff: 0.1,
                call_factor: 1.2,
                caution: 0.25,
                tilt: 0.9,
                recovery: 0.12,
                ..base
            },
            Self::Rampart => Style {
                vpip: 0.27,
                pfr: 0.2,
                three_bet: 0.12,
                four_bet: 0.08,
                defend: 0.8,
                value_margin: 0.045,
                aggression: 0.95,
                bluff: 0.55,
                bluff_raise: 0.25,
                // Hero calls.
                call_factor: 0.8,
                caution: 0.045,
                bet_size: 1.1,
                open_size: 3.0,
                // Bolder on a heater, a little tilty after big losses.
                heater: 0.3,
                tilt: 0.2,
                recovery: 0.2,
                ..base
            },
            Self::DannySmallball => Style {
                vpip: 0.25,
                pfr: 0.17,
                three_bet: 0.03,
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
            Self::ChrisMoneybags => Style {
                vpip: 0.26,
                pfr: 0.05,
                three_bet: 0.013,
                defend: 0.85,
                // Tuned (#206).
                position_bonus: 0.05,
                open_size: 3.0,
                value_margin: 0.15,
                aggression: 0.4,
                bluff: 0.05,
                bluff_raise: 0.025,
                call_factor: 0.7,
                pair_call_factor: 1.025,
                caution: 0.063,
                bet_size: 0.769,
                heater: 0.9,
                recovery: 0.1,
                ..base
            },
            Self::RobinJaneLewd => Style {
                // Tuned (#206).
                vpip: 0.363,
                pfr: 0.03,
                three_bet: 0.007,
                four_bet: 0.013,
                defend: 0.95,
                position_bonus: 0.15,
                open_size: 3.25,
                value_margin: 0.2,
                aggression: 0.45,
                check_raise: 0.025,
                bluff: 0.03,
                bluff_raise: 0.0,
                call_factor: 0.8,
                pair_call_factor: 0.15,
                caution: 0.05,
                bet_size: 0.594,
                ..base
            },
            Self::GusBluffsen => Style {
                vpip: 0.31,
                pfr: 0.23,
                three_bet: 0.08,
                // Bets his good hands, and a lot of his air too.
                aggression: 1.0,
                bluff: 0.55,
                bluff_raise: 0.18,
                bet_size: 0.75,
                ..base
            },
            Self::JenSilly => Style {
                vpip: 0.08,
                pfr: 0.04,
                any_suited: true,
                any_ace: true,
                position_bonus: 0.15,
                aggression: 0.5,
                bluff: 0.1,
                call_factor: 0.9,
                ..base
            },
            Self::MichaelMiserable => Style {
                vpip: 0.15,
                pfr: 0.12,
                bluff: 0.15,
                caution: 0.2,
                ..base
            },
            Self::BradOwned => Style {
                // Loose and passive preflop, sees lots of flops.
                vpip: 0.27,
                pfr: 0.05,
                three_bet: 0.013,
                four_bet: 0.007,
                defend: 0.7,
                position_bonus: 0.2,
                always_play: "JJ",
                // Honest after the flop: bets what he hits, rarely bluffs,
                // and lets go when he misses.
                value_margin: 0.15,
                aggression: 0.45,
                bluff: 0.03,
                bluff_raise: 0.0,
                call_factor: 1.0,
                caution: 0.45,
                bet_size: 0.55,
                ..base
            },
            Self::NikAirbag => Style {
                vpip: 0.45,
                pfr: 0.34,
                three_bet: 0.17,
                four_bet: 0.1,
                // Tuned (#206): a maniac who calls less, and a big bet
                // makes him think a little.
                defend: 0.6,
                position_bonus: 0.2,
                value_margin: 0.065,
                aggression: 1.0,
                bluff: 0.53,
                bluff_raise: 0.25,
                call_factor: 0.95,
                caution: 0.083,
                bet_size: 1.5,
                open_size: 3.5,
                // The maniac never believes a bet: he judges his hand
                // against random hands, whatever you did.
                ranges: false,
                ..base
            },
            Self::Bungleman => Style {
                // Close to a solid regular, with a few wild moves: traps
                // and the odd big bluff.
                vpip: 0.2,
                pfr: 0.15,
                three_bet: 0.07,
                four_bet: 0.03,
                defend: 0.65,
                value_margin: 0.1,
                aggression: 0.75,
                trap: 0.25,
                bluff: 0.3,
                bluff_raise: 0.1,
                call_factor: 0.95,
                caution: 0.1,
                bet_size: 0.85,
                heater: 0.1,
                tilt: 0.1,
                ..base
            },
            Self::TomCollins => Style {
                // Raises nearly every unopened or limped pot: these widen to
                // about 70% six-handed or on the button.
                vpip: 0.5,
                pfr: 0.46,
                // Facing a raise he's careful: re-raises only his best hands
                // and calls with few more.
                three_bet: 0.05,
                four_bet: 0.02,
                defend: 0.4,
                position_bonus: 0.25,
                open_size: 3.0,
                // Checked to, he bets: value hands nearly always, and a lot
                // of his air too, big.
                value_margin: 0.05,
                aggression: 0.95,
                bluff: 0.4,
                bet_size: 1.0,
                // Raised after the flop, he gets out of the way unless he's
                // strong: never bluff-raises, scared of bets, no sticky pairs.
                bluff_raise: 0.0,
                call_factor: 1.15,
                pair_call_factor: 1.05,
                caution: 0.3,
                ..base
            },
            Self::TheMathematician => Style {
                // Starting hands by the book: tight up front, much wider on
                // the button, and first in it raises or folds.
                vpip: 0.16,
                pfr: 0.15,
                three_bet: 0.05,
                four_bet: 0.025,
                defend: 0.5,
                position_bonus: 0.55,
                open_size: 3.0,
                // Value bets thin and reliably, with a bluff for about every
                // three value bets (tuned, #206).
                value_margin: 0.055,
                aggression: 0.875,
                trap: 0.05,
                bluff: 0.23,
                bluff_raise: 0.0,
                bet_size: 0.75,
                // Calls by the pot odds, with a small margin: no hero calls.
                call_factor: 1.1,
                pair_call_factor: 1.0,
                caution: 0.1,
                // No moods: the numbers don't care how the last hand went.
                tilt: 0.0,
                heater: 0.0,
                // More careful sums.
                samples: 300,
                ..base
            },
            Self::AndrewFavorable => Style {
                // Hold'em: a straightforward, reasonable regular, with few
                // fancy plays.
                vpip: 0.2,
                pfr: 0.15,
                three_bet: 0.06,
                four_bet: 0.02,
                defend: 0.6,
                position_bonus: 0.35,
                value_margin: 0.1,
                aggression: 0.75,
                bluff: 0.18,
                bluff_raise: 0.03,
                call_factor: 1.05,
                caution: 0.2,
                bet_size: 0.66,
                tilt: 0.1,
                // Omaha is his game: half again as many hands, and harder.
                omaha: 0.5,
                ..base
            },
            Self::TommySweeden => Style {
                // Aggressive, but picks his spots: a fair range, raised
                // more often than called.
                vpip: 0.19,
                pfr: 0.16,
                three_bet: 0.06,
                four_bet: 0.03,
                defend: 0.5,
                position_bonus: 0.4,
                open_size: 3.0,
                value_margin: 0.1,
                aggression: 0.85,
                // His move: checks a good hand to raise when bet into, and
                // sometimes a bad one heads-up.
                check_raise: 0.6,
                trap: 0.15,
                bluff: 0.25,
                bluff_raise: 0.06,
                bet_size: 0.75,
                // More careful than most aggressive players: bets scare him
                // a little, and he doesn't pay off with weak pairs.
                call_factor: 1.15,
                pair_call_factor: 1.1,
                caution: 0.35,
                // A lost pot gets to him, and it takes a while to wear off.
                tilt: 0.45,
                recovery: 0.12,
                ..base
            },
        }
    }

    /// A bot with this personality; a `seed` makes it reproducible.
    pub fn bot(self, seed: Option<u64>) -> PersonalityBot {
        PersonalityBot::new(self.name(), self.style(), seed)
    }
}

/// Table size the preflop range traits ([`Style::vpip`] and friends) are
/// written for.
pub const BASELINE_PLAYERS: usize = 9;

/// The widest a preflop range gets, however short the table.
const MAX_RANGE: f64 = 0.95;

/// A starting hand's equity against the strongest hands, as a share of its
/// equity against random ones, for a call that's priced in
/// (`fold_unless_priced_in`). In Hold'em it's about 0.4 across the board:
/// 72o has 34% against a random hand and 12–15% against the best, T9s 54%
/// and about 22%, 22 50% and about 19%. Omaha hands run much closer.
const STRONG_HOLDEM_SHARE: f64 = 0.4;
const STRONG_OMAHA_SHARE: f64 = 0.6;

/// Pot odds (the call's share of the pot after calling) at or below which
/// every bot calls, on any street: a call of at most a ninth of the pot.
const ALWAYS_CALL_ODDS: f64 = 0.1;

/// Share of its stack at the start of the hand a bot has put in before
/// preflop it calls whenever it's priced in against the strongest hands.
const COMMITTED_SHARE: f64 = 0.25;

// The spew guard: what keeps even the wildest Regular from throwing chips
// away where it costs most. Measured against GtoBot heads-up (ducy-gto's
// `leaks` example), every Regular lost most getting its stack in before the
// flop with weak hands, and the wild ones building huge pots after it with
// nothing or one pair.

/// Before the flop, a call or raise that puts in this many big blinds or
/// more, or [`STACK_OFF_SHARE`] of its stack, is a stack-off decision...
const STACK_OFF_BB: f64 = 40.0;
const STACK_OFF_SHARE: f64 = 0.25;
/// ...made only with the top share of hands (for a full table, widened for
/// shorter ones like the other ranges: about 15% heads-up), and up to
/// [`STACK_OFF_LOOSE`] more for the loosest players.
const STACK_OFF_RANGE: f64 = 0.035;
const STACK_OFF_LOOSE: f64 = 0.015;

/// After the flop, a pot is big once the effective stack behind is less
/// than this many pots. There bluffs and bluff raises come
/// [`BIG_POT_BLUFF`] times as often, value bets want [`BIG_POT_VALUE`] more
/// edge, and every hand is judged against what the bettors probably hold,
/// even by players who otherwise judge it against random hands.
const BIG_POT_SPR: f64 = 1.5;
const BIG_POT_BLUFF: f64 = 0.3;
const BIG_POT_VALUE: f64 = 0.05;

/// Pot odds at or below which a bot after the flop wants no more equity than
/// the bare odds, however cautious its style.
const PRICED_IN_ODDS: f64 = 0.2;

/// On the river no card is coming, so a bet only wins when worse hands call
/// or better hands fold. A mediocre hand that bets gets called by better and
/// folds out worse: it loses money, so it checks for its showdown value.
/// Betting is polarized: value bets (and raises) want this much more edge than
/// on earlier streets, to be ahead of the hands that call...
const RIVER_VALUE_EXTRA: f64 = 0.15;

/// ...and bluffs only come from hands with little showdown value: equity
/// below this share of an even split (a quarter, heads-up).
const RIVER_BLUFF_SHARE: f64 = 0.5;

/// Widens a range share written for [`BASELINE_PLAYERS`] to a table of
/// `players`: `1 - (1 - share)^(9 / players)`, at most 0.95.
///
/// A hand worth playing against eight opponents is worth playing against
/// fewer, and fewer opponents means fewer strong hands to run into, so
/// ranges grow as the table shrinks: a 20% 9-handed range is about 28%
/// 6-handed and 63% heads-up. Tables larger than 9 tighten the same way.
pub fn scale_for_table(share: f64, players: usize) -> f64 {
    if players == 0 || share <= 0.0 {
        return 0.0;
    }
    let share = share.min(1.0);
    (1.0 - (1.0 - share).powf(BASELINE_PLAYERS as f64 / players as f64)).min(MAX_RANGE)
}

/// How good `seat`'s position is, from 0 (first to act after the flop, the
/// small blind) to 1 (the button, last to act). The big blind counts as 0.5:
/// it is out of position, but closes the preflop action at a discount.
pub fn position_strength(seat: usize, button: usize, players: usize) -> f64 {
    if players < 2 {
        return 0.5;
    }
    let big_blind = if players == 2 {
        (button + 1) % 2
    } else {
        (button + 2) % players
    };
    if seat == big_blind {
        return 0.5;
    }
    // Order of action after the flop: left of the button first, button last.
    let order = (seat + players - button - 1) % players;
    order as f64 / (players - 1) as f64
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
/// - **After the flop** it estimates its equity against what the opponents
///   still in probably hold, from their actions this hand (or against random
///   hands with `ranges` off), then bets for value, slow-plays, bluffs, calls
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
    /// The street it last checked on this hand, while it hasn't raised
    /// since: bet into then, it's in a spot to check-raise.
    checked: Option<Street>,
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
            checked: None,
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

        // Its Omaha game, if it has one.
        let o = style.omaha;
        if o > 0.0 && matches!(obs.rules.variant, Variant::Omaha { .. }) {
            style.vpip = (style.vpip * (1.0 + o)).min(0.9);
            style.pfr = (style.pfr * (1.0 + o)).min(style.vpip);
            style.three_bet = (style.three_bet * (1.0 + o)).min(style.pfr);
            style.aggression = (style.aggression + 0.2 * o).min(1.0);
            style.bluff = (style.bluff + 0.2 * o).min(0.9);
        }

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
        let position =
            1.0 + style.position_bonus * (2.0 * position_strength(obs.seat, obs.button, n) - 1.0);
        let range = |share: f64| (scale_for_table(share, n) * position).clamp(0.0, MAX_RANGE);
        let vpip = range(style.vpip);
        let pfr = range(style.pfr).min(vpip);
        let three_bet = range(style.three_bet).min(vpip);
        let four_bet = range(style.four_bet).min(vpip);

        // Share of starting hands better than this one, pulled up for hands
        // it plays regardless of rank.
        let mut top = 1.0
            - preflop_percentile(
                obs.rules.variant,
                obs.hole_cards,
                style.samples,
                &mut self.rng,
            );
        // Hands played only because they're suited or hold an ace: always
        // just call (or check), whatever happened before.
        let pretty = (style.any_suited && has_suited(obs.hole_cards))
            || (style.any_ace && has_ace(obs.hole_cards));
        let favorite = self.is_favorite(obs);
        if favorite {
            top = top.min(pfr);
        }
        let only_pretty = pretty && !favorite && top > vpip;

        let big_blind = obs.rules.big_blind;
        // The spew guard: getting the stack in takes a hand in the top
        // `stack_off` share, whatever the style's ranges say.
        let me = &obs.seats[obs.seat];
        let start_stack = (me.contributed + me.stack) as f64;
        let loose = ((style.vpip - 0.2) / 0.35).clamp(0.0, 1.0);
        let stack_off = scale_for_table(STACK_OFF_RANGE + STACK_OFF_LOOSE * loose, n);
        let guarded = |to: u64| {
            top > stack_off
                && (to as f64 >= STACK_OFF_BB * big_blind as f64
                    || to as f64 >= STACK_OFF_SHARE * start_stack)
        };
        let call_to = me.contributed + legal.call.unwrap_or(0);
        let raise = |legal: &LegalActions, to: u64| {
            if guarded(to) {
                None
            } else {
                raise_to(legal, to)
            }
        };
        let call_ok = legal.call.is_none() || !guarded(call_to);
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
                self.fold_unless_priced_in(obs, style)
            };
        }

        if only_pretty && call_ok {
            return passive(legal);
        }

        match raises {
            0 => {
                if top <= pfr {
                    let to = (style.open_size * big_blind as f64).round() as u64;
                    if let Some(action) = raise(legal, to) {
                        return action;
                    }
                }
                if top <= vpip && call_ok {
                    return passive(legal);
                }
            }
            1 => {
                if top <= three_bet {
                    if let Some(action) = raise(legal, obs.current_bet * 3) {
                        return action;
                    }
                }
                if top <= vpip * style.defend && call_ok {
                    return passive(legal);
                }
            }
            _ => {
                if top <= four_bet {
                    let to = (obs.current_bet as f64 * 2.5).round() as u64;
                    if let Some(action) = raise(legal, to) {
                        return action;
                    }
                }
                if top <= four_bet * 2.5 && call_ok {
                    return passive(legal);
                }
            }
        }
        self.fold_unless_priced_in(obs, style)
    }

    /// Folds, unless the price makes folding a mistake: always at
    /// [`ALWAYS_CALL_ODDS`], and once it has [`COMMITTED_SHARE`] of its stack
    /// in, whenever the share of the pot it must put in to call is below this
    /// hand's equity against even the strongest hands. So a bot that has put
    /// most of its stack in and faces a shove for the rest calls, instead of
    /// folding a pot it's priced into.
    fn fold_unless_priced_in(&mut self, obs: &Observation, style: &Style) -> Action {
        let legal = &obs.legal;
        if legal.can_check {
            return Action::Check;
        }
        let Some(call) = legal.call else {
            return Action::Fold;
        };
        let need = call as f64 / (obs.pot + call) as f64;
        if need <= ALWAYS_CALL_ODDS {
            return Action::Call;
        }
        // Cheap early spots (completing the small blind, a min-raise) are
        // left to the style's ranges: there the price says nothing about
        // being committed. And no hand is a favourite against the strongest
        // hands by enough to call at worse than even money.
        let me = &obs.seats[obs.seat];
        let committed =
            me.contributed as f64 >= COMMITTED_SHARE * (me.contributed + me.stack) as f64;
        if !committed || need >= 0.5 {
            return Action::Fold;
        }
        // Equity against random hands, scaled down to what it has against
        // the strongest hands (starting hands run closer in Omaha).
        let share = match obs.rules.variant {
            Variant::Holdem => STRONG_HOLDEM_SHARE,
            Variant::Omaha { .. } => STRONG_OMAHA_SHARE,
        };
        let equity = observation_equity(obs, style.samples.max(100), &mut self.rng);
        if equity * share >= need {
            Action::Call
        } else {
            Action::Fold
        }
    }

    fn postflop(&mut self, obs: &Observation, style: &Style) -> Action {
        let legal = &obs.legal;
        let players = obs.seats.iter().filter(|s| !s.folded).count().max(1);
        // The spew guard: in a big pot (little behind for its size), every
        // hand is judged against what the others probably hold.
        let big_pot = (effective_stack(obs) as f64) < BIG_POT_SPR * obs.pot as f64;
        let (equity, caution) = if style.ranges || big_pot {
            (
                range_equity(obs, &self.model, style.samples, &mut self.rng),
                style.caution * RANGE_CAUTION,
            )
        } else {
            (
                observation_equity(obs, style.samples, &mut self.rng),
                style.caution,
            )
        };
        let edge = equity - 1.0 / players as f64;
        let monster = edge >= style.value_margin + 0.2;
        let pot = obs.pot as f64;
        // Holding the ace of the flush on an unpaired Omaha board: nobody can
        // have the nut flush, so it's the best spot there is to bluff.
        let (bluff, bluff_raise) = if nut_flush_blocker(obs) {
            (
                blocker_bluff(style.bluff).min(0.9),
                blocker_bluff(style.bluff_raise).min(0.5),
            )
        } else {
            (style.bluff, style.bluff_raise)
        };
        // Big pots get fewer bluffs and want more for value (the spew guard).
        let (bluff, bluff_raise) = if big_pot {
            (bluff * BIG_POT_BLUFF, bluff_raise * BIG_POT_BLUFF)
        } else {
            (bluff, bluff_raise)
        };
        // River betting is polarized (RIVER_VALUE_EXTRA): strong hands bet for
        // value, hands with no showdown value bluff, and the ones between
        // check.
        let river = obs.street == Street::River && style.polar_river;
        let value_margin = if river {
            style.value_margin + RIVER_VALUE_EXTRA
        } else {
            style.value_margin
        } + if big_pot { BIG_POT_VALUE } else { 0.0 };
        let bluff_hand = if river {
            equity < RIVER_BLUFF_SHARE / players as f64
        } else {
            edge < 0.0
        };

        match legal.call {
            None => {
                let size = (pot * style.bet_size).round() as u64;
                // The nuts on the river, last to act: checking would go to
                // showdown without a bet, so bet. (With someone still to act
                // behind, it may check to let them bet.)
                if nuts_on_river(obs) && !acts_after(obs) {
                    if let Some(action) = bet(legal, size) {
                        return action;
                    }
                }
                if monster && obs.street != Street::River && self.chance(style.trap) {
                    // Slow-play: check now, raise if bet into.
                    return Action::Check;
                }
                if style.check_raise > 0.0
                    && edge >= value_margin
                    && acts_after(obs)
                    && self.chance(style.check_raise)
                {
                    // Check, to raise when someone behind bets.
                    return Action::Check;
                }
                if edge >= value_margin && self.chance(style.aggression) {
                    if let Some(action) = bet(legal, size) {
                        return action;
                    }
                }
                // Bluff weak hands, mostly against one or two opponents.
                if bluff_hand && players <= 3 && self.chance(bluff) {
                    if let Some(action) = bet(legal, size) {
                        return action;
                    }
                }
                Action::Check
            }
            Some(call) => {
                let raise_size =
                    obs.current_bet + ((pot + call as f64) * style.bet_size).round() as u64;
                // The nuts on the river: never just call to showdown, raise
                // something (calling only when raising isn't allowed).
                if nuts_on_river(obs) {
                    return raise_to(legal, raise_size).unwrap_or(Action::Call);
                }
                let raise_chance = if monster {
                    style.aggression.max(style.trap)
                } else {
                    style.aggression
                };
                if edge >= value_margin + 0.1 && self.chance(raise_chance) {
                    if let Some(action) = raise_to(legal, raise_size) {
                        return action;
                    }
                }
                // Checked this street and now bet into: the check-raise.
                let check_raising = style.check_raise > 0.0 && self.checked == Some(obs.street);
                if check_raising
                    && edge >= value_margin
                    && self.chance(style.aggression.max(style.check_raise))
                {
                    if let Some(action) = raise_to(legal, raise_size) {
                        return action;
                    }
                }
                let bluff_raise = if check_raising {
                    bluff_raise + 0.3 * style.check_raise
                } else {
                    bluff_raise
                };
                if bluff_hand && players <= 2 && self.chance(bluff_raise) {
                    if let Some(action) = raise_to(legal, raise_size) {
                        return action;
                    }
                }
                let pot_odds = call as f64 / (pot + call as f64);
                // The bet as a fraction of the pot before it was made.
                let bet_fraction = call as f64 / (pot - call as f64).max(1.0);
                if pot_odds <= ALWAYS_CALL_ODDS {
                    return Action::Call;
                }
                let mut needed = pot_odds * style.call_factor + caution * bet_fraction;
                if has_pair(obs) {
                    needed *= style.pair_call_factor;
                }
                // Priced in: caution can't ask for more than the bare odds.
                if pot_odds <= PRICED_IN_ODDS {
                    needed = needed.min(pot_odds);
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
        if obs.street == Street::Preflop {
            self.checked = None;
        }
        let action = if obs.street == Street::Preflop {
            self.preflop(obs, &style)
        } else {
            self.postflop(obs, &style)
        };
        match action {
            Action::Check => self.checked = Some(obs.street),
            Action::Bet(_) | Action::Raise(_) => self.checked = None,
            _ => {}
        }
        Some(action)
    }

    fn hand_over(&mut self, summary: &HandSummary) {
        self.checked = None;
        // Ranges read the seats' VPIP and PFR; adjusting to them is `exploit`.
        if self.style.exploit || self.style.ranges {
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

/// Whether anyone still able to bet acts after `obs.seat` on this street
/// (after the flop, action starts left of the button).
fn acts_after(obs: &Observation) -> bool {
    let n = obs.seats.len();
    let order = |s: usize| (s + n - obs.button - 1) % n;
    let me = order(obs.seat);
    obs.seats
        .iter()
        .enumerate()
        .any(|(s, v)| s != obs.seat && !v.folded && !v.all_in && order(s) > me)
}

/// Whether it holds the nuts on the river: no hand anyone could hold beats
/// it (a tie still counts).
fn nuts_on_river(obs: &Observation) -> bool {
    if obs.street != Street::River {
        return false;
    }
    let hole: Vec<_> = obs.hole_cards.iter(false).collect();
    has_nuts(obs.rules.variant, &hole, &obs.board)
}

/// Omaha, a board with three or more of one suit and no pair (so no full
/// house or quads to fear), and the ace of that suit as its only card of the
/// suit: no made flush, but nobody else can hold the nut flush.
fn nut_flush_blocker(obs: &Observation) -> bool {
    if !matches!(obs.rules.variant, Variant::Omaha { .. }) {
        return false;
    }
    let mut suits = [0u8; 4];
    let mut ranks = [false; 13];
    for card in &obs.board {
        suits[card.suit() as usize] += 1;
        if std::mem::replace(&mut ranks[card.rank() as usize], true) {
            return false;
        }
    }
    let Some(suit) = (0..4).find(|&s| suits[s] >= 3) else {
        return false;
    };
    let mine: Vec<_> = obs
        .hole_cards
        .iter(false)
        .filter(|c| c.suit() as usize == suit)
        .collect();
    matches!(mine.as_slice(), [c] if c.rank() == Rank::Ace)
}

/// The chance to bluff with the nut-flush blocker: as if it had three
/// tries, so a 25% bluffer goes to 58% and one who never bluffs still
/// rarely does. Against calling stations an exploiting bot's bluffs are
/// already zero, and stay that.
fn blocker_bluff(chance: f64) -> f64 {
    1.0 - (1.0 - chance).powi(3)
}

fn passive(legal: &LegalActions) -> Action {
    if legal.can_check {
        Action::Check
    } else {
        Action::Call
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
