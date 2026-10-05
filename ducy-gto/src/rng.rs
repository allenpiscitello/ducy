//! A small, fast, seedable generator for sampling. Each traversal gets its
//! own stream derived from the seed and its iteration number, so training is
//! reproducible however the work is split across threads.

/// SplitMix64 (Steele, Lea and Flood 2014).
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The stream for iteration `i` of a run seeded with `seed`.
    pub fn for_iteration(seed: u64, i: u64) -> Self {
        let mut r = Self(seed ^ i.wrapping_mul(0xD1B5_4A32_D192_ED03));
        r.next_u64();
        r
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// An index drawn with the given probabilities (summing to about 1).
    pub fn sample(&mut self, probs: &[f64]) -> usize {
        let mut x = self.next_f64();
        for (i, &p) in probs.iter().enumerate() {
            if x < p {
                return i;
            }
            x -= p;
        }
        probs.iter().rposition(|&p| p > 0.0).unwrap_or(0)
    }
}
