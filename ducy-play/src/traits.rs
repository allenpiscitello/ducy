//! A personality's play as numbers (#213): what a bot does over a batch of
//! hands ([`Traits`]), what each personality's should be ([`targets`]), and
//! how far a bot has drifted from them ([`drift`]).
//!
//! Tuning the style numbers (#206) and the neural-network bots (#219) both
//! need these: a change to a bot keeps its character only while its traits
//! stay within [`tolerance`] of its targets.
//!
//! Traits are measured the same way every time ([`measure`]): six-handed
//! No-limit Hold'em, 1/2 blinds and 100 big blinds, against tables of the
//! other personalities that change every few hundred hands.

use ducy::deck::{Card, Deck};

use crate::{
    bot::{Bot, play_hand},
    deal::Deal,
    hand::{Event, Hand, Street},
    personality::Personality,
    ranges::made_score,
    rules::{TableRules, Variant},
    stats::hand_counts,
};

/// One thing measured about a bot's play. Every trait is a share between 0
/// and 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Trait {
    /// Hands played voluntarily preflop.
    Vpip,
    /// Hands raised preflop.
    Pfr,
    /// Re-raises when facing exactly one preflop raise.
    ThreeBet,
    /// Bets and raises after the flop, as a share of its bets, raises, calls
    /// and folds there (aggression frequency, which stays between 0 and 1
    /// where the bets-per-call factor doesn't).
    Aggression,
    /// The same, on the flop only.
    FlopAggression,
    /// On the turn only.
    TurnAggression,
    /// On the river only.
    RiverAggression,
    /// Bets and raises after the flop holding no pair of its own (neither a
    /// pocket pair nor a hole card pairing the board) and no straight or
    /// better, as a share of all its bets and raises there.
    Bluff,
    /// Folds when facing a bet or raise after the flop.
    FoldToBet,
    /// Raises after checking on the same street, as a share of the times it
    /// checked and then faced a bet.
    CheckRaise,
    /// Hands that reached a showdown with it still in, of those where it saw
    /// the flop.
    Showdown,
}

/// How many traits there are.
pub const TRAITS: usize = 11;

impl Trait {
    /// Every trait, in the order of [`Traits`]' values.
    pub const ALL: [Trait; TRAITS] = [
        Self::Vpip,
        Self::Pfr,
        Self::ThreeBet,
        Self::Aggression,
        Self::FlopAggression,
        Self::TurnAggression,
        Self::RiverAggression,
        Self::Bluff,
        Self::FoldToBet,
        Self::CheckRaise,
        Self::Showdown,
    ];

    /// A short name, for tables and CSV headers.
    pub fn name(self) -> &'static str {
        match self {
            Self::Vpip => "vpip",
            Self::Pfr => "pfr",
            Self::ThreeBet => "3bet",
            Self::Aggression => "agg",
            Self::FlopAggression => "agg_flop",
            Self::TurnAggression => "agg_turn",
            Self::RiverAggression => "agg_river",
            Self::Bluff => "bluff",
            Self::FoldToBet => "fold_to_bet",
            Self::CheckRaise => "check_raise",
            Self::Showdown => "wtsd",
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    /// How far a bot may stray from its target on this trait before it has
    /// drifted from its character, for a trait that isn't the character's
    /// signature. Wider where the trait is seen less often (3-bets,
    /// check-raises), so the same sample measures it less precisely.
    pub fn base_tolerance(self) -> f64 {
        match self {
            Self::Vpip | Self::Pfr => 0.04,
            Self::Aggression | Self::FoldToBet | Self::Showdown => 0.05,
            Self::FlopAggression | Self::TurnAggression | Self::RiverAggression | Self::Bluff => {
                0.07
            }
            Self::ThreeBet | Self::CheckRaise => 0.08,
        }
    }
}

/// A bot's traits: one share per [`Trait`], in [`Trait::ALL`]'s order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Traits(pub [f64; TRAITS]);

impl Traits {
    /// The value of trait `t`.
    pub fn get(&self, t: Trait) -> f64 {
        self.0[t.index()]
    }
}

/// The raw counts behind [`Traits`], added up hand by hand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TraitCounts {
    /// Hands dealt.
    pub hands: u32,
    /// Hands played voluntarily preflop.
    pub vpip: u32,
    /// Hands raised preflop.
    pub pfr: u32,
    /// Chances to 3-bet, and 3-bets.
    pub three_bet_chances: u32,
    /// 3-bets made.
    pub three_bets: u32,
    /// Bets and raises after the flop: flop, turn, river.
    pub aggressive: [u32; 3],
    /// Calls and folds after the flop: flop, turn, river.
    pub passive: [u32; 3],
    /// Bets and raises after the flop with no pair of its own.
    pub bluffs: u32,
    /// Times it faced a bet or raise after the flop.
    pub faced_bets: u32,
    /// Times it folded to one.
    pub folds_to_bets: u32,
    /// Times it checked and then faced a bet on the same street.
    pub check_raise_chances: u32,
    /// Times it raised then.
    pub check_raises: u32,
    /// Hands where it saw the flop.
    pub saw_flop: u32,
    /// Of those, hands that reached a showdown with it still in.
    pub showdowns: u32,
}

impl TraitCounts {
    /// Adds another batch's counts.
    pub fn add(&mut self, o: &TraitCounts) {
        self.hands += o.hands;
        self.vpip += o.vpip;
        self.pfr += o.pfr;
        self.three_bet_chances += o.three_bet_chances;
        self.three_bets += o.three_bets;
        for i in 0..3 {
            self.aggressive[i] += o.aggressive[i];
            self.passive[i] += o.passive[i];
        }
        self.bluffs += o.bluffs;
        self.faced_bets += o.faced_bets;
        self.folds_to_bets += o.folds_to_bets;
        self.check_raise_chances += o.check_raise_chances;
        self.check_raises += o.check_raises;
        self.saw_flop += o.saw_flop;
        self.showdowns += o.showdowns;
    }

    /// The traits these counts make (0 for a trait never seen).
    pub fn traits(&self) -> Traits {
        let rate = |n: u32, d: u32| {
            if d == 0 {
                0.0
            } else {
                f64::from(n) / f64::from(d)
            }
        };
        let agg: u32 = self.aggressive.iter().sum();
        let pas: u32 = self.passive.iter().sum();
        let street = |i: usize| rate(self.aggressive[i], self.aggressive[i] + self.passive[i]);
        Traits([
            rate(self.vpip, self.hands),
            rate(self.pfr, self.hands),
            rate(self.three_bets, self.three_bet_chances),
            rate(agg, agg + pas),
            street(0),
            street(1),
            street(2),
            rate(self.bluffs, agg),
            rate(self.folds_to_bets, self.faced_bets),
            rate(self.check_raises, self.check_raise_chances),
            rate(self.showdowns, self.saw_flop),
        ])
    }
}

/// The counts for each seat from one finished hand.
pub fn count_hand(hand: &Hand) -> Vec<TraitCounts> {
    let seats = hand.num_seats();
    let variant = hand.rules().variant;
    let basic = hand_counts(hand.events(), seats);
    let showdown = hand.result().is_some_and(|r| r.showdown);
    let holes: Vec<Vec<Card>> = (0..seats)
        .map(|s| {
            hand.hole_cards(s)
                .map_or_else(Vec::new, |d| d.iter(false).collect())
        })
        .collect();
    let mut out: Vec<TraitCounts> = basic
        .iter()
        .enumerate()
        .map(|(s, b)| TraitCounts {
            hands: 1,
            vpip: u32::from(b.vpip),
            pfr: u32::from(b.pfr),
            three_bet_chances: u32::from(b.three_bet_chance),
            three_bets: u32::from(b.three_bet),
            faced_bets: b.faced_bets,
            folds_to_bets: b.folds_to_bets,
            saw_flop: u32::from(b.saw_flop),
            showdowns: u32::from(b.saw_flop && showdown && !hand.has_folded(s)),
            ..TraitCounts::default()
        })
        .collect();

    let mut street = Street::Preflop;
    let mut board: Vec<Card> = Vec::new();
    // On this street: who has checked, and who of them has since faced a bet.
    let mut checked = vec![false; seats];
    let mut chance = vec![false; seats];
    let after_flop = |st: Street| match st {
        Street::Flop => Some(0),
        Street::Turn => Some(1),
        Street::River => Some(2),
        _ => None,
    };
    for event in hand.events() {
        match event {
            Event::Board {
                street: next,
                cards,
            } => {
                street = *next;
                board.extend(cards.iter().copied());
                checked.iter_mut().for_each(|c| *c = false);
                chance.iter_mut().for_each(|c| *c = false);
            }
            Event::Check { seat } if *seat < seats => checked[*seat] = true,
            Event::Bet { seat, .. } | Event::Raise { seat, .. } if *seat < seats => {
                let Some(i) = after_flop(street) else {
                    continue;
                };
                let c = &mut out[*seat];
                c.aggressive[i] += 1;
                if no_pair_of_its_own(variant, &holes[*seat], &board) {
                    c.bluffs += 1;
                }
                if chance[*seat] {
                    c.check_raises += 1;
                    chance[*seat] = false;
                    checked[*seat] = false;
                }
                // Everyone else who checked now faces a bet.
                for s in 0..seats {
                    if s != *seat && checked[s] && !chance[s] {
                        chance[s] = true;
                        out[s].check_raise_chances += 1;
                    }
                }
            }
            Event::Call { seat, .. } | Event::Fold { seat } if *seat < seats => {
                if let Some(i) = after_flop(street) {
                    out[*seat].passive[i] += 1;
                }
                // Its chance to check-raise passed.
                chance[*seat] = false;
                checked[*seat] = false;
            }
            _ => {}
        }
    }
    out
}

/// Whether `hole` has no pair of its own on `board`: no pocket pair, no hole
/// card pairing the board, and nothing made of a straight or better (a
/// paired board alone doesn't count).
pub fn no_pair_of_its_own(variant: Variant, hole: &[Card], board: &[Card]) -> bool {
    if hole.is_empty() || board.len() < 3 {
        return false;
    }
    let pairs = hole.iter().enumerate().any(|(i, a)| {
        hole[i + 1..].iter().any(|b| a.rank() == b.rank())
            || board.iter().any(|b| a.rank() == b.rank())
    });
    !pairs && made_score(variant, hole, board) < straight_floor()
}

/// The score of the lowest straight: anything below it is a pair or less.
fn straight_floor() -> u32 {
    use std::sync::OnceLock;
    static FLOOR: OnceLock<u32> = OnceLock::new();
    *FLOOR.get_or_init(|| {
        let wheel = Deck::parse("As 2d 3c 4h 5s").expect("cards");
        ducy::ranking::hand_rank::StandardHandRanker::score(&wheel)
    })
}

/// Hands per table before the opponents change in [`measure`].
const PER_TABLE: usize = 300;
/// Players at each table in [`measure`].
const PLAYERS: usize = 6;

/// How [`measure`] plays its hands.
#[derive(Clone, Copy, Debug)]
pub struct MeasureConfig {
    /// Hands to play.
    pub hands: usize,
    /// Seeds the deals and the opponents.
    pub seed: u64,
    /// Monte Carlo samples for the opponents' equity estimates (`None`
    /// keeps each personality's own); fewer is quicker.
    pub opponent_samples: Option<usize>,
}

impl MeasureConfig {
    /// `hands` hands from `seed`, with the opponents as they are.
    pub fn new(hands: usize, seed: u64) -> Self {
        Self {
            hands,
            seed,
            opponent_samples: None,
        }
    }
}

/// Plays `bot` in seat 0 for `config.hands` six-handed No-limit Hold'em
/// hands (1/2 blinds, 100 big blinds each hand) against five personalities,
/// changing every [`PER_TABLE`] hands and leaving out `like` (the
/// personality being measured, so it never plays itself), and counts what it
/// does.
pub fn measure_counts(
    bot: &mut dyn Bot,
    like: Option<Personality>,
    config: &MeasureConfig,
) -> TraitCounts {
    let rules = TableRules::no_limit_holdem(1, 2);
    let others: Vec<Personality> = Personality::ALL
        .into_iter()
        .filter(|&o| Some(o) != like)
        .collect();
    let mut total = TraitCounts::default();
    for block in 0..config.hands.div_ceil(PER_TABLE) {
        let mut table: Vec<Box<dyn Bot>> = (0..PLAYERS - 1)
            .map(|i| {
                let o = others[(block * (PLAYERS - 1) + i) % others.len()];
                let seed = config
                    .seed
                    .wrapping_mul(1_000_003)
                    .wrapping_add((block * PLAYERS + i) as u64);
                let mut style = o.style();
                if let Some(n) = config.opponent_samples {
                    style.samples = n;
                }
                Box::new(crate::personality::PersonalityBot::new(
                    o.name(),
                    style,
                    Some(seed),
                )) as Box<dyn Bot>
            })
            .collect();
        let start = block * PER_TABLE;
        for h in start..(start + PER_TABLE).min(config.hands) {
            let id = config.seed.wrapping_mul(7_919).wrapping_add(h as u64);
            let deal = Deal::random(rules.variant, PLAYERS, Some(id)).expect("deal");
            let mut hand = Hand::new(rules, &[200; PLAYERS], h % PLAYERS, deal).expect("hand");
            let mut seated: Vec<&mut dyn Bot> = vec![&mut *bot];
            for b in table.iter_mut() {
                seated.push(&mut **b);
            }
            play_hand(&mut hand, &mut seated).expect("play");
            total.add(&count_hand(&hand)[0]);
        }
    }
    total
}

/// [`measure_counts`]' traits.
pub fn measure(bot: &mut dyn Bot, like: Option<Personality>, config: &MeasureConfig) -> Traits {
    measure_counts(bot, like, config).traits()
}

/// Each personality's targets: its traits as the rule-based bot played them
/// over 30,000 hands of [`measure`] (seed 1), from
/// `cargo run --release -p ducy-play --example traits -- 30000 --targets`.
pub fn targets(p: Personality) -> Traits {
    Traits(match p {
        Personality::DougPoker => TARGETS[0],
        Personality::JonnySlow => TARGETS[1],
        Personality::MisterCheating => TARGETS[2],
        Personality::MilkKing => TARGETS[3],
        Personality::PhilBigmouth => TARGETS[4],
        Personality::Rampart => TARGETS[5],
        Personality::DannySmallball => TARGETS[6],
        Personality::ChrisMoneybags => TARGETS[7],
        Personality::RobinJaneLewd => TARGETS[8],
        Personality::GusBluffsen => TARGETS[9],
        Personality::JenSilly => TARGETS[10],
        Personality::MichaelMiserable => TARGETS[11],
        Personality::BradOwned => TARGETS[12],
        Personality::NikAirbag => TARGETS[13],
        Personality::Bungleman => TARGETS[14],
        Personality::TomCollins => TARGETS[15],
        Personality::TheMathematician => TARGETS[16],
        Personality::AndrewFavorable => TARGETS[17],
        Personality::TommySweeden => TARGETS[18],
    })
}

// In Personality::ALL's order; columns in Trait::ALL's order: vpip, pfr,
// 3bet, agg, agg_flop, agg_turn, agg_river, bluff, fold_to_bet,
// check_raise, wtsd. Measured shares, so some land near a math constant.
#[rustfmt::skip]
#[allow(clippy::approx_constant)]
const TARGETS: [[f64; TRAITS]; 19] = [
    [0.2324, 0.1636, 0.0823, 0.4702, 0.4494, 0.4866, 0.5056, 0.2806, 0.5127, 0.0697, 0.4586], // Doug Poker
    [0.1198, 0.0333, 0.0207, 0.1510, 0.1191, 0.1870, 0.1752, 0.0315, 0.6068, 0.0157, 0.4901], // Jonny Slow
    [0.3961, 0.2813, 0.1650, 0.5737, 0.5474, 0.5949, 0.6219, 0.3826, 0.4887, 0.1078, 0.4430], // Mister Cheating
    [0.7300, 0.2791, 0.1323, 0.5085, 0.4912, 0.5170, 0.5464, 0.3928, 0.3052, 0.1428, 0.4892], // Milk King
    [0.1617, 0.1109, 0.0459, 0.4651, 0.4419, 0.4955, 0.4779, 0.2155, 0.5610, 0.0680, 0.4458], // Phil Bigmouth
    [0.4270, 0.3184, 0.2484, 0.6691, 0.6523, 0.6854, 0.7036, 0.4716, 0.3552, 0.1631, 0.5155], // Rampart
    [0.3435, 0.1788, 0.0408, 0.4411, 0.3921, 0.4836, 0.4945, 0.2878, 0.5273, 0.0554, 0.4519], // Danny Smallball
    [0.4232, 0.0782, 0.0233, 0.3358, 0.3139, 0.3506, 0.3626, 0.4609, 0.3203, 0.0830, 0.5529], // Chris Moneybags
    [0.4325, 0.0287, 0.0105, 0.1476, 0.1145, 0.1716, 0.1828, 0.2008, 0.3330, 0.0420, 0.5907], // Robin Jane Lewd
    [0.3652, 0.2566, 0.1275, 0.6102, 0.5899, 0.6420, 0.6104, 0.4266, 0.5174, 0.0979, 0.4276], // Gus Bluffsen
    [0.3610, 0.0671, 0.0691, 0.2906, 0.2465, 0.3271, 0.3634, 0.2521, 0.5375, 0.0620, 0.4256], // Jen Silly
    [0.2109, 0.1508, 0.0784, 0.4393, 0.4052, 0.4754, 0.4788, 0.2080, 0.5412, 0.0631, 0.4557], // Michael Miserable
    [0.3255, 0.0468, 0.0185, 0.2322, 0.1842, 0.2808, 0.2962, 0.0725, 0.6341, 0.0297, 0.3885], // Brad Owned
    [0.5417, 0.3829, 0.2731, 0.7029, 0.7188, 0.7052, 0.6048, 0.4367, 0.1434, 0.1301, 0.5735], // Nik Airbag
    [0.2858, 0.2059, 0.1264, 0.5081, 0.4764, 0.5279, 0.5758, 0.3610, 0.4738, 0.1188, 0.4602], // Bungleman
    [0.4878, 0.3708, 0.0743, 0.5851, 0.5464, 0.6159, 0.6824, 0.3513, 0.6924, 0.0145, 0.3671], // Tom Collins
    [0.2070, 0.1728, 0.0697, 0.5279, 0.5118, 0.5409, 0.5571, 0.2926, 0.5102, 0.0560, 0.4512], // The Mathematician
    [0.2668, 0.1909, 0.1034, 0.4629, 0.4269, 0.5000, 0.5032, 0.2340, 0.5569, 0.0554, 0.4433], // Andrew Favorable
    [0.2714, 0.2230, 0.1198, 0.5169, 0.4878, 0.5351, 0.5902, 0.4045, 0.5552, 0.3070, 0.4417], // Tommy Sweeden
];

/// The traits that are `p`'s character, held to half the usual tolerance:
/// what you'd name first about how it plays.
pub fn signature(p: Personality) -> &'static [Trait] {
    use Trait::*;
    match p {
        Personality::DougPoker => &[Vpip, Bluff],
        Personality::JonnySlow => &[Vpip, Pfr, Aggression],
        Personality::MisterCheating => &[Vpip, Aggression],
        Personality::MilkKing => &[Vpip, FoldToBet],
        Personality::PhilBigmouth => &[Vpip],
        Personality::Rampart => &[Vpip, Bluff],
        Personality::DannySmallball => &[Vpip],
        Personality::ChrisMoneybags => &[Vpip],
        Personality::RobinJaneLewd => &[Pfr, FoldToBet],
        Personality::GusBluffsen => &[Bluff],
        Personality::JenSilly => &[Vpip],
        Personality::MichaelMiserable => &[Vpip, Aggression],
        Personality::BradOwned => &[Vpip, FoldToBet],
        Personality::NikAirbag => &[Pfr, Aggression],
        Personality::Bungleman => &[Vpip],
        Personality::TomCollins => &[Pfr, Aggression],
        Personality::TheMathematician => &[Vpip, Pfr],
        Personality::AndrewFavorable => &[Vpip],
        Personality::TommySweeden => &[CheckRaise],
    }
}

/// How far `p` may stray from its target on `t`: [`Trait::base_tolerance`],
/// halved for its [`signature`] traits.
pub fn tolerance(p: Personality, t: Trait) -> f64 {
    let base = t.base_tolerance();
    if signature(p).contains(&t) {
        base / 2.0
    } else {
        base
    }
}

/// How far `traits` are from `p`'s targets: the root mean square of each
/// trait's distance in tolerances. Under 1 on average is within tolerance;
/// [`within_tolerance`] checks every trait.
pub fn drift(p: Personality, traits: &Traits) -> f64 {
    let target = targets(p);
    let sum: f64 = Trait::ALL
        .iter()
        .map(|&t| ((traits.get(t) - target.get(t)) / tolerance(p, t)).powi(2))
        .sum();
    (sum / TRAITS as f64).sqrt()
}

/// The traits on which `traits` are further from `p`'s targets than its
/// tolerance, with how far (signed: positive is above the target).
pub fn out_of_tolerance(p: Personality, traits: &Traits) -> Vec<(Trait, f64)> {
    let target = targets(p);
    Trait::ALL
        .iter()
        .map(|&t| (t, traits.get(t) - target.get(t)))
        .filter(|&(t, d)| d.abs() > tolerance(p, t))
        .collect()
}

/// Whether every trait is within `p`'s tolerance of its target.
pub fn within_tolerance(p: Personality, traits: &Traits) -> bool {
    out_of_tolerance(p, traits).is_empty()
}

#[cfg(test)]
mod test {
    use super::*;

    fn cards(s: &str) -> Vec<Card> {
        Deck::parse(s).unwrap().iter(false).collect()
    }

    #[test]
    fn test_no_pair_of_its_own() {
        let he = Variant::Holdem;
        // Ace high, a pocket pair, a hole card pairing the board.
        assert!(no_pair_of_its_own(he, &cards("As Kd"), &cards("7c 8d 2h")));
        assert!(!no_pair_of_its_own(he, &cards("5s 5d"), &cards("7c 8d 2h")));
        assert!(!no_pair_of_its_own(he, &cards("As 8s"), &cards("7c 8d 2h")));
        // A paired board alone isn't its own pair; a made straight is a hand.
        assert!(no_pair_of_its_own(he, &cards("As Kd"), &cards("7c 7d 2h")));
        assert!(!no_pair_of_its_own(he, &cards("9s Td"), &cards("7c 8d Jh")));
        // A flush draw is still a bluff; the flush isn't.
        assert!(no_pair_of_its_own(he, &cards("As Ks"), &cards("7s 8s 2h")));
        assert!(!no_pair_of_its_own(he, &cards("As Ks"), &cards("7s 8s 2s")));
        // Omaha: any two of its cards pairing count.
        let plo = Variant::Omaha { hole_cards: 4 };
        assert!(!no_pair_of_its_own(
            plo,
            &cards("As Ad Kc Qh"),
            &cards("7c 8d 2h")
        ));
        assert!(no_pair_of_its_own(
            plo,
            &cards("As Kd Qc Jh"),
            &cards("7c 8d 2h")
        ));
        // Preflop, nothing is a bluff.
        assert!(!no_pair_of_its_own(he, &cards("As Kd"), &[]));
    }

    #[test]
    fn test_traits_from_counts() {
        let c = TraitCounts {
            hands: 10,
            vpip: 3,
            pfr: 2,
            three_bet_chances: 4,
            three_bets: 1,
            aggressive: [2, 1, 1],
            passive: [2, 1, 3],
            bluffs: 1,
            faced_bets: 5,
            folds_to_bets: 2,
            check_raise_chances: 2,
            check_raises: 1,
            saw_flop: 4,
            showdowns: 2,
        };
        let t = c.traits();
        assert_eq!(t.get(Trait::Vpip), 0.3);
        assert_eq!(t.get(Trait::ThreeBet), 0.25);
        assert_eq!(t.get(Trait::Aggression), 0.4);
        assert_eq!(t.get(Trait::FlopAggression), 0.5);
        assert_eq!(t.get(Trait::RiverAggression), 0.25);
        assert_eq!(t.get(Trait::Bluff), 0.25);
        assert_eq!(t.get(Trait::Showdown), 0.5);
        // Nothing seen is 0, not NaN.
        assert_eq!(TraitCounts::default().traits(), Traits([0.0; TRAITS]));
    }

    #[test]
    fn test_drift() {
        let p = Personality::NikAirbag;
        let target = targets(p);
        assert_eq!(drift(p, &target), 0.0);
        assert!(within_tolerance(p, &target));
        // Off by exactly the tolerance on one trait.
        let mut t = target;
        t.0[Trait::Showdown.index()] += Trait::Showdown.base_tolerance() * 0.999;
        assert!(within_tolerance(p, &t));
        t.0[Trait::Showdown.index()] += Trait::Showdown.base_tolerance() * 0.01;
        assert_eq!(out_of_tolerance(p, &t).len(), 1);
        // A signature trait is held to half.
        let mut t = target;
        t.0[Trait::Pfr.index()] -= Trait::Pfr.base_tolerance() * 0.6;
        assert_eq!(out_of_tolerance(p, &t)[0].0, Trait::Pfr);
        assert!(drift(p, &t) > 0.0);
    }
}
