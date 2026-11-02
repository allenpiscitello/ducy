//! Comparing versions of one bot with the luck taken out (#216).
//!
//! [`run_variants`] plays every variant of a bot (network versions, training
//! checkpoints, tuned styles) on the same deals, in the same seats, against
//! its own copy of the same opponents. Each variant meets the same cards in
//! the same spots, so the difference between two variants on a deal is mostly
//! their play rather than the cards: [`VariantResult::compare`] reads it as a
//! paired difference, which needs far fewer hands than comparing two
//! separate matches.
//!
//! The opponents are made afresh for every hand, from a seed that depends
//! only on the deal and the seat rotation, so every variant faces bots in
//! exactly the same state with the same random draws to come. They only part
//! ways where the variants play differently: an opponent then sees a
//! different hand and may answer differently, which is the effect being
//! measured. The variants are made afresh for every hand the same way, with
//! a seed shared between them, so they draw the same random numbers too. (Bots
//! made once per match drift apart for good after the first different action,
//! as their random draws shift: that left pairing barely better than two
//! separate matches.) The price: no bot carries reads or moods from hand to
//! hand.
//!
//! Measured (`examples/variants.rs`, 12,000 six-handed hands): two close
//! versions of The Mathematician differ by +2.7 ± 7.7 bb/100 paired, against
//! ± 37.6 from two separate matches, about 24 times fewer hands for the same
//! precision; two different Regulars, about 1.8 times.

use ducy_play::{Bot, Deal, Hand, PlayError, play_hand};

use crate::matchup::MatchConfig;

/// Each variant's result on each deal.
#[derive(Clone, Debug, PartialEq)]
pub struct VariantResult {
    /// Hands each variant played: deals × seats in duplicate mode, else deals.
    pub hands: usize,
    /// Hands per deal (every seat in duplicate mode, else 1).
    pub per_deal: usize,
    /// `net[v][d]`: chips variant `v` won on deal `d`, over its rotations.
    pub net: Vec<Vec<i64>>,
    /// How often each variant gave no action or an illegal one.
    pub fallbacks: Vec<u32>,
    /// The big blind, for big blinds per 100 hands.
    pub big_blind: u64,
}

/// A win rate or a difference of win rates, in big blinds per 100 hands,
/// with a 95% confidence interval (± `ci95`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Comparison {
    /// The mean, in big blinds per 100 hands.
    pub bb_per_100: f64,
    /// Half the width of the 95% interval.
    pub ci95: f64,
}

impl VariantResult {
    fn rate(&self, per_deal: impl Iterator<Item = f64>) -> Comparison {
        let xs: Vec<f64> = per_deal.collect();
        let n = xs.len() as f64;
        // Chips per deal → big blinds per 100 hands.
        let k = 100.0 / (self.big_blind as f64 * self.per_deal as f64);
        if n < 2.0 {
            return Comparison {
                bb_per_100: xs.first().copied().unwrap_or(0.0) * k,
                ci95: f64::INFINITY,
            };
        }
        let mean = xs.iter().sum::<f64>() / n;
        let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
        Comparison {
            bb_per_100: mean * k,
            ci95: 1.96 * (var / n).sqrt() * k,
        }
    }

    /// Variant `v`'s own win rate against the opponents.
    pub fn win_rate(&self, v: usize) -> Comparison {
        self.rate(self.net[v].iter().map(|&x| x as f64))
    }

    /// How much more variant `v` wins than variant `base`, deal by deal: the
    /// luck of the cards cancels, so the interval is much tighter than
    /// comparing their win rates.
    pub fn compare(&self, v: usize, base: usize) -> Comparison {
        self.rate(
            self.net[v]
                .iter()
                .zip(&self.net[base])
                .map(|(&a, &b)| (a - b) as f64),
        )
    }
}

/// Plays each of `variants` against the same opponents, on the same deals in
/// the same seats: `opponents(k, seed)` makes opponent `k` (0 to `players` −
/// 2) for one hand, and should seed it with `seed`, which is the same for
/// every variant. `players` seats in all, the variant's included.
///
/// Every hand starts from `config.starting_stack` and the button moves each
/// deal. In duplicate mode each deal is played once with the variant in each
/// seat (the opponents keeping their order round the others); otherwise the
/// variant always sits in seat 0.
///
/// Each variant is made the same way, afresh for every hand: `variants[v](seed)`
/// with a seed shared by every variant on that hand, so where two variants
/// would play alike they also draw the same random numbers.
pub fn run_variants(
    config: &MatchConfig,
    variants: &[&dyn Fn(u64) -> Box<dyn Bot>],
    opponents: &dyn Fn(usize, u64) -> Box<dyn Bot>,
    players: usize,
) -> Result<VariantResult, PlayError> {
    if players < 2 {
        return Err(PlayError::InvalidPlayerCount);
    }
    let rotations = if config.duplicate { players } else { 1 };
    let stacks = vec![config.starting_stack; players];
    let mut result = VariantResult {
        hands: config.deals * rotations,
        per_deal: rotations,
        net: vec![vec![0; config.deals]; variants.len()],
        fallbacks: vec![0; variants.len()],
        big_blind: config.rules.big_blind,
    };
    for (v, make) in variants.iter().enumerate() {
        for d in 0..config.deals {
            let deal = Deal::random(
                config.rules.variant,
                players,
                Some(config.seed.wrapping_add(d as u64)),
            )?;
            let button = d % players;
            for r in 0..rotations {
                let mut hand = Hand::new(config.rules, &stacks, button, deal.clone())?;
                // Opponents for this hand, the same for every variant.
                let seed = config
                    .seed
                    .wrapping_mul(0x9e37_79b9_7f4a_7c15)
                    .wrapping_add((d * rotations + r) as u64 * 1_000_003);
                let mut others: Vec<Box<dyn Bot>> = (0..players - 1)
                    .map(|k| opponents(k, seed.wrapping_add(k as u64)))
                    .collect();
                // And the variant, seeded alike for every variant.
                let mut hero = make(seed.wrapping_add(0xffff));
                // The variant in seat r, the opponents in order round the rest.
                let mut seated: Vec<&mut dyn Bot> = Vec::with_capacity(players);
                for o in others.iter_mut() {
                    seated.push(&mut **o);
                }
                seated.insert(r, &mut *hero);
                let outcome = play_hand(&mut hand, &mut seated)?;
                result.net[v][d] += outcome.result.net[r];
                result.fallbacks[v] += outcome.fallbacks[r];
            }
        }
    }
    Ok(result)
}
