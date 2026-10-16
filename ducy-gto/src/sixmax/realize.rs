//! Equity realization for the six-handed preflop game (#249): how much of
//! its preflop equity a starting hand wins once the betting goes on after
//! the flop, in position (last to act after the flop) and out of position.
//!
//! A hand that flops the pot share its equity says realizes 1.0. Out of
//! position, and with weak offsuit hands, players realize less: they fold
//! more often when they miss and get less paid when they hit. These numbers
//! are fitted from the heads-up blueprint playing itself (`fit`, the
//! `sixmax_realize` example), per starting-hand class, at a stack depth.

use super::equity::PreflopEquity;
use crate::holdem::iso::NUM_PREFLOP_CLASSES;

const C: usize = NUM_PREFLOP_CLASSES;
const MAGIC: &[u8; 8] = b"DUCYRLZ\0";
const VERSION: u32 = 1;

/// Before anything is fitted: everyone in position realizes 1.08, out of
/// position 0.9.
pub const DEFAULT_IP: f64 = 1.08;
pub const DEFAULT_OOP: f64 = 0.9;

/// Realization per class, in position and out of position.
#[derive(Clone, Debug, PartialEq)]
pub struct Realization {
    pub ip: Vec<f32>,
    pub oop: Vec<f32>,
}

impl Default for Realization {
    fn default() -> Self {
        Self {
            ip: vec![DEFAULT_IP as f32; C],
            oop: vec![DEFAULT_OOP as f32; C],
        }
    }
}

/// Sums for fitting one position: per class, what hands won from the flop
/// pot and what their equity said they should.
#[derive(Clone, Debug)]
pub struct Tally {
    won: Vec<f64>,
    owed: Vec<f64>,
    hands: Vec<u64>,
}

impl Default for Tally {
    fn default() -> Self {
        Self {
            won: vec![0.0; C],
            owed: vec![0.0; C],
            hands: vec![0; C],
        }
    }
}

impl Tally {
    /// One hand that saw the flop: `class` with `equity` against the other
    /// hand, a flop pot of `pot`, and `won` chips from that point on (its
    /// stack at the end less its stack at the flop; the two players' sum to
    /// the pot).
    pub fn add(&mut self, class: usize, equity: f64, pot: f64, won: f64) {
        self.won[class] += won;
        self.owed[class] += equity * pot;
        self.hands[class] += 1;
    }

    pub fn merge(&mut self, other: &Tally) {
        for c in 0..C {
            self.won[c] += other.won[c];
            self.owed[c] += other.owed[c];
            self.hands[c] += other.hands[c];
        }
    }

    /// Realization over all classes.
    pub fn overall(&self) -> f64 {
        self.won.iter().sum::<f64>() / self.owed.iter().sum::<f64>().max(1e-9)
    }

    /// Hands seen per class.
    pub fn hands(&self, class: usize) -> u64 {
        self.hands[class]
    }

    /// Per class, shrunk toward the overall rate by `prior` pots' worth of
    /// the overall rate, so rarely seen classes don't swing wildly; clamped
    /// to a sane range.
    pub fn per_class(&self, prior: f64) -> Vec<f32> {
        let all = self.overall();
        let mean_owed =
            self.owed.iter().sum::<f64>() / self.hands.iter().sum::<u64>().max(1) as f64;
        (0..C)
            .map(|c| {
                let k = prior * mean_owed;
                // Aces win far more than their equity's share of the flop
                // pot (the stacks go in when they're ahead): room up to 3.
                (((self.won[c] + k * all) / (self.owed[c] + k)).clamp(0.2, 3.0)) as f32
            })
            .collect()
    }
}

impl Realization {
    /// From tallies in position and out of position.
    pub fn fit(ip: &Tally, oop: &Tally, prior: f64) -> Self {
        Self {
            ip: ip.per_class(prior),
            oop: oop.per_class(prior),
        }
    }

    /// Average realization over hands, weighted by combos, for a summary.
    pub fn mean(values: &[f32]) -> f64 {
        use crate::holdem::chart::combos;
        (0..C).map(|c| values[c] as f64 * combos(c)).sum::<f64>() / 1326.0
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 + 8 * C);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&(C as u32).to_le_bytes());
        for x in self.ip.iter().chain(&self.oop) {
            out.extend_from_slice(&x.to_le_bytes());
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() != 16 + 8 * C || &bytes[..8] != MAGIC {
            return Err("not a realization table".into());
        }
        let word = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().expect("4 bytes"));
        if word(8) != VERSION || word(12) as usize != C {
            return Err("realization table of another version".into());
        }
        let floats: Vec<f32> = bytes[16..]
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes(c.try_into().expect("4 bytes")))
            .collect();
        Ok(Self {
            ip: floats[..C].to_vec(),
            oop: floats[C..].to_vec(),
        })
    }
}

/// A hand's heads-up equity, for tallies: class `a` against class `b`.
pub fn hu_equity(eq: &PreflopEquity, a: usize, b: usize) -> f64 {
    eq.heads_up(a, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitting_and_round_trip() {
        let mut ip = Tally::default();
        let mut oop = Tally::default();
        // Class 0 in position wins 120 of the 100 it's owed; out of position
        // it wins 70 of 100.
        for _ in 0..1000 {
            ip.add(0, 0.5, 200.0, 120.0);
            oop.add(0, 0.5, 200.0, 70.0);
        }
        let r = Realization::fit(&ip, &oop, 10.0);
        assert!((r.ip[0] - 1.2).abs() < 0.01, "{}", r.ip[0]);
        assert!((r.oop[0] - 0.7).abs() < 0.01, "{}", r.oop[0]);
        // Unseen classes get the overall rate.
        assert!((r.oop[100] - 0.7).abs() < 0.01);
        assert_eq!(Realization::from_bytes(&r.to_bytes()), Ok(r));
        assert_eq!(Realization::default().ip[5], DEFAULT_IP as f32);
    }
}
