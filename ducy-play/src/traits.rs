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
    [0.2344, 0.1640, 0.0821, 0.4777, 0.4520, 0.5168, 0.4762, 0.3028, 0.5168, 0.0597, 0.4531], // Doug Poker
    [0.1203, 0.0335, 0.0212, 0.1443, 0.1053, 0.1884, 0.1727, 0.0266, 0.5934, 0.0195, 0.4755], // Jonny Slow
    [0.3953, 0.2808, 0.1637, 0.5759, 0.5482, 0.6061, 0.6077, 0.3759, 0.4966, 0.1001, 0.4439], // Mister Cheating
    [0.7306, 0.2773, 0.1361, 0.5238, 0.5058, 0.5389, 0.5467, 0.3830, 0.3094, 0.1412, 0.4922], // Milk King
    [0.1616, 0.1114, 0.0457, 0.4686, 0.4470, 0.4945, 0.4825, 0.2370, 0.5535, 0.0672, 0.4567], // Phil Bigmouth
    [0.4229, 0.3174, 0.2437, 0.6755, 0.6643, 0.6869, 0.6936, 0.4829, 0.3558, 0.1556, 0.5142], // Rampart
    [0.3420, 0.1764, 0.0415, 0.4448, 0.4045, 0.4837, 0.4844, 0.3016, 0.5427, 0.0613, 0.4443], // Danny Smallball
    [0.4247, 0.0779, 0.0218, 0.3289, 0.3159, 0.3506, 0.3195, 0.4572, 0.3181, 0.0885, 0.5488], // Chris Moneybags
    [0.4318, 0.0298, 0.0088, 0.1424, 0.1165, 0.1665, 0.1578, 0.2004, 0.3278, 0.0402, 0.6011], // Robin Jane Lewd
    [0.3655, 0.2583, 0.1347, 0.5383, 0.4456, 0.5471, 0.7202, 0.5473, 0.4967, 0.0922, 0.4337], // Gus Bluffsen
    [0.3629, 0.0686, 0.0719, 0.2844, 0.2486, 0.3057, 0.3542, 0.2537, 0.5512, 0.0490, 0.4199], // Jen Silly
    [0.2115, 0.1530, 0.0821, 0.4351, 0.3965, 0.4692, 0.4889, 0.2049, 0.5451, 0.0575, 0.4460], // Michael Miserable
    [0.3256, 0.0465, 0.0170, 0.2292, 0.1847, 0.2785, 0.2798, 0.0814, 0.6274, 0.0435, 0.3946], // Brad Owned
    [0.5409, 0.3811, 0.2686, 0.7124, 0.7277, 0.7156, 0.6158, 0.4333, 0.1489, 0.1182, 0.5795], // Nik Airbag
    [0.2836, 0.2048, 0.1252, 0.5079, 0.4827, 0.5317, 0.5411, 0.3546, 0.4908, 0.1059, 0.4535], // Bungleman
    [0.4865, 0.3724, 0.0737, 0.5891, 0.5568, 0.6151, 0.6580, 0.3624, 0.7084, 0.0130, 0.3702], // Tom Collins
    [0.2088, 0.1736, 0.0702, 0.5298, 0.5100, 0.5533, 0.5454, 0.3091, 0.5169, 0.0615, 0.4466], // The Mathematician
    [0.2683, 0.1917, 0.1056, 0.4656, 0.4304, 0.4993, 0.5137, 0.2365, 0.5853, 0.0587, 0.4235], // Andrew Favorable
    [0.2714, 0.2208, 0.1162, 0.5278, 0.5007, 0.5566, 0.5650, 0.4060, 0.5623, 0.3064, 0.4471], // Tommy Sweeden
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
