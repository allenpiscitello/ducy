//! Generic players for the Bankroll Challenge's card rooms (ducy-web #299,
//! #256): nameless regulars and tourists built on the personality engine, in
//! four families. Each player's style is its family's base with every number
//! jittered from a seed, so no two play exactly alike and a player keeps their
//! style forever.
//!
//! A generic player's id is `g-<family>-<seed>`, e.g. `g-rock-1234`, which
//! [`bot_by_id`] turns back into a bot: the page seats them by id like any
//! personality.
//!
//! Measured strength (ducy-gto's `gto_match --only g-<family>`, heads-up at
//! 100 BB, 160,000 duplicate hands over 40 players of each family): GtoBot
//! wins about 36 bb/100 against rocks, 60 against stations, 58 against
//! maniacs and 11 against grinders, inside the targets of #256 (30–50,
//! 40–80, 40–80 and 10–25).

use crate::personality::{PersonalityBot, Style};

/// The four families of generic players.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    /// Tight-passive: few hands and only big ones, rarely bluffs, folds to
    /// pressure without a strong hand. Easy to steal from, hard to get paid.
    Rock,
    /// Loose-passive, a calling station: too many hands, calls too much,
    /// rarely raises or bluffs.
    Station,
    /// Loose-aggressive: raises and re-raises light, bluffs a lot, overbets.
    /// The spew guard still keeps them from simply handing chips over.
    Maniac,
    /// A low-stakes grinder: tight-aggressive and value-heavy, adjusts to
    /// weak players, but overfolds to aggression from tough ones.
    Grinder,
}

impl Family {
    /// Every family.
    pub const ALL: [Family; 4] = [Self::Rock, Self::Station, Self::Maniac, Self::Grinder];

    /// Short identifier, as in a player id: "rock", "station", "maniac",
    /// "grinder".
    pub fn id(self) -> &'static str {
        match self {
            Self::Rock => "rock",
            Self::Station => "station",
            Self::Maniac => "maniac",
            Self::Grinder => "grinder",
        }
    }

    /// Display name of the family: "Rock", "Calling station", "Maniac",
    /// "Grinder".
    pub fn name(self) -> &'static str {
        match self {
            Self::Rock => "Rock",
            Self::Station => "Calling station",
            Self::Maniac => "Maniac",
            Self::Grinder => "Grinder",
        }
    }

    /// The family with id `id` ([`Self::id`]), or None.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.id() == id)
    }

    /// The family's base style, before a player's own jitter.
    pub fn base(self) -> Style {
        let d = Style::default();
        match self {
            Self::Rock => Style {
                vpip: 0.09,
                pfr: 0.045,
                three_bet: 0.015,
                four_bet: 0.008,
                defend: 0.35,
                position_bonus: 0.1,
                open_size: 3.0,
                value_margin: 0.3,
                aggression: 0.35,
                trap: 0.1,
                bluff: 0.02,
                bluff_raise: 0.01,
                polar_river: false,
                call_factor: 1.8,
                caution: 0.5,
                bet_size: 0.55,
                ..d
            },
            Self::Station => Style {
                vpip: 0.5,
                pfr: 0.05,
                three_bet: 0.015,
                four_bet: 0.01,
                defend: 0.9,
                position_bonus: 0.1,
                open_size: 3.0,
                value_margin: 0.2,
                aggression: 0.35,
                bluff: 0.06,
                bluff_raise: 0.02,
                polar_river: false,
                call_factor: 0.55,
                pair_call_factor: 0.55,
                caution: 0.02,
                bet_size: 0.45,
                tilt: 0.2,
                ranges: false,
                ..d
            },
            Self::Maniac => Style {
                vpip: 0.45,
                pfr: 0.35,
                three_bet: 0.16,
                four_bet: 0.06,
                defend: 0.75,
                position_bonus: 0.25,
                open_size: 3.5,
                value_margin: 0.03,
                aggression: 0.9,
                bluff: 0.6,
                bluff_raise: 0.25,
                call_factor: 0.75,
                caution: 0.05,
                bet_size: 1.2,
                tilt: 0.3,
                heater: 0.3,
                ranges: false,
                ..d
            },
            Self::Grinder => Style {
                vpip: 0.2,
                pfr: 0.16,
                three_bet: 0.06,
                four_bet: 0.025,
                defend: 0.6,
                position_bonus: 0.45,
                aggression: 0.8,
                bluff: 0.2,
                call_factor: 1.1,
                caution: 0.2,
                exploit: true,
                exploit_after: 15,
                ..d
            },
        }
    }
}

/// One generic player: a family and the seed their style is drawn from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GenericPlayer {
    /// Their family.
    pub family: Family,
    /// The seed their style is drawn from.
    pub seed: u64,
}

/// A number in [0, 1) from `seed` and `k`: splitmix64.
fn unit(seed: u64, k: u64) -> f64 {
    let mut z = seed
        .wrapping_add(0x9e37_79b9_7f4a_7c15u64.wrapping_mul(k + 1))
        .wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

impl GenericPlayer {
    /// The player of `family` drawn from `seed`.
    pub fn new(family: Family, seed: u64) -> Self {
        Self { family, seed }
    }

    /// The player with id `g-<family>-<seed>`, or None.
    pub fn from_id(id: &str) -> Option<Self> {
        let rest = id.strip_prefix("g-")?;
        let (family, seed) = rest.split_once('-')?;
        Some(Self::new(Family::from_id(family)?, seed.parse().ok()?))
    }

    /// `g-<family>-<seed>`.
    pub fn id(&self) -> String {
        format!("g-{}-{}", self.family.id(), self.seed)
    }

    /// Their style: the family's base with each number moved by up to about
    /// a fifth either way (the shares kept in their ranges, the preflop
    /// shares kept in order: raises within plays).
    pub fn style(&self) -> Style {
        let b = self.family.base();
        let mut k = 0;
        let mut j = |x: f64, spread: f64| {
            k += 1;
            x * (1.0 + spread * (2.0 * unit(self.seed, k) - 1.0))
        };
        let share = |x: f64| x.clamp(0.0, 1.0);
        let vpip = share(j(b.vpip, 0.2));
        let pfr = share(j(b.pfr, 0.2)).min(vpip);
        let three_bet = share(j(b.three_bet, 0.25)).min(pfr.max(0.01));
        let four_bet = share(j(b.four_bet, 0.25)).min(three_bet);
        Style {
            vpip,
            pfr,
            three_bet,
            four_bet,
            defend: share(j(b.defend, 0.15)),
            position_bonus: share(j(b.position_bonus, 0.25)),
            open_size: j(b.open_size, 0.12).max(2.0),
            value_margin: j(b.value_margin, 0.25).max(0.0),
            aggression: share(j(b.aggression, 0.15)),
            trap: share(j(b.trap, 0.3)),
            bluff: share(j(b.bluff, 0.25)),
            bluff_raise: share(j(b.bluff_raise, 0.3)),
            call_factor: j(b.call_factor, 0.1).max(0.3),
            pair_call_factor: j(b.pair_call_factor, 0.1).max(0.3),
            caution: j(b.caution, 0.25).max(0.0),
            bet_size: j(b.bet_size, 0.15).max(0.25),
            tilt: j(b.tilt, 0.4).max(0.0),
            heater: j(b.heater, 0.4).max(0.0),
            ..b
        }
    }

    /// Their bot; a `seed` makes its play reproducible.
    pub fn bot(&self, seed: Option<u64>) -> PersonalityBot {
        PersonalityBot::new(self.family.name(), self.style(), seed)
    }
}

/// The bot for generic player id `id` (`g-<family>-<seed>`), or None if
/// `id` isn't one.
pub fn bot_by_id(id: &str, seed: Option<u64>) -> Option<PersonalityBot> {
    GenericPlayer::from_id(id).map(|g| g.bot(seed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        for f in Family::ALL {
            let g = GenericPlayer::new(f, 987_654_321);
            assert_eq!(GenericPlayer::from_id(&g.id()), Some(g));
        }
        assert_eq!(GenericPlayer::from_id("g-nobody-1"), None);
        assert_eq!(GenericPlayer::from_id("doug_poker"), None);
        assert_eq!(GenericPlayer::from_id("g-rock-x"), None);
    }

    #[test]
    fn the_same_seed_gives_the_same_style() {
        let a = GenericPlayer::new(Family::Maniac, 42).style();
        let b = GenericPlayer::new(Family::Maniac, 42).style();
        let c = GenericPlayer::new(Family::Maniac, 43).style();
        assert_eq!(a.vpip, b.vpip);
        assert_eq!(a.bluff, b.bluff);
        assert_ne!(a.vpip, c.vpip);
    }

    #[test]
    fn the_families_stay_apart_over_many_seeds() {
        let avg = |f: Family, get: fn(&Style) -> f64| {
            (0..200)
                .map(|s| get(&GenericPlayer::new(f, s).style()))
                .sum::<f64>()
                / 200.0
        };
        let vpip = |s: &Style| s.vpip;
        let pfr = |s: &Style| s.pfr;
        let calls = |s: &Style| s.call_factor;
        let bluff = |s: &Style| s.bluff;
        // Rocks tightest, stations and maniacs loosest.
        assert!(avg(Family::Rock, vpip) < avg(Family::Grinder, vpip));
        assert!(avg(Family::Grinder, vpip) < avg(Family::Station, vpip));
        // Stations call most (lowest call factor), rocks least.
        assert!(avg(Family::Station, calls) < avg(Family::Maniac, calls));
        assert!(avg(Family::Rock, calls) > avg(Family::Grinder, calls));
        // Maniacs raise and bluff most; stations raise least of the loose ones.
        assert!(avg(Family::Maniac, pfr) > avg(Family::Grinder, pfr));
        assert!(avg(Family::Station, pfr) < avg(Family::Grinder, pfr));
        assert!(avg(Family::Maniac, bluff) > avg(Family::Grinder, bluff));
        // Every jittered style keeps its shares in order.
        for f in Family::ALL {
            for s in 0..200 {
                let st = GenericPlayer::new(f, s).style();
                assert!(
                    st.pfr <= st.vpip && st.four_bet <= st.three_bet,
                    "{f:?} {s}"
                );
                assert!((0.0..=1.0).contains(&st.vpip));
            }
        }
    }
}
