//! Preflop equity over the 169 starting-hand classes (#248): each class
//! against each other heads-up, and each class against one to five random
//! hands, for multiway pots. Card removal counts: the cards of the two
//! classes never collide, and the board comes from what's left.
//!
//! Equities are sampled (a share of a pot, ties split), so they carry a
//! little noise: about ±0.4 points at 20,000 samples a pair.

use std::thread;

use crate::{
    Rng,
    holdem::{
        cards::{Card, NUM_CARDS, bit, card, score},
        chart::class_of,
        iso::NUM_PREFLOP_CLASSES,
    },
};

/// Up to this many random opponents in [`PreflopEquity::vs_random`].
pub const MAX_OPPONENTS: usize = 5;

const MAGIC: &[u8; 8] = b"DUCYEQ6\0";
const VERSION: u32 = 1;

/// A class's ranks and suitedness: `(high, low, suited)`, ranks 0 (deuce)
/// to 12 (ace).
pub fn class_ranks(class: usize) -> (u8, u8, bool) {
    for hi in 0..13u8 {
        for lo in 0..=hi {
            for suited in [false, true] {
                if (hi != lo || !suited) && class_of(hi, lo, suited) == class {
                    return (hi, lo, suited);
                }
            }
        }
    }
    panic!("no class {class}");
}

/// A random concrete hand of `class` avoiding the cards in `used`, or `None`
/// if every one collides.
fn deal_class(class: usize, used: u64, rng: &mut Rng) -> Option<[Card; 2]> {
    let (hi, lo, suited) = class_ranks(class);
    let mut options = [[0u8; 2]; 12];
    let mut n = 0;
    for a in 0..4u8 {
        for b in 0..4u8 {
            let ok = if hi == lo {
                a < b
            } else if suited {
                a == b
            } else {
                a != b
            };
            let hand = [card(hi, a), card(lo, b)];
            if ok && used & (bit(hand[0]) | bit(hand[1])) == 0 {
                options[n] = hand;
                n += 1;
            }
        }
    }
    (n > 0).then(|| options[(rng.next_u64() % n as u64) as usize])
}

/// A uniformly random hand avoiding `used`.
fn deal_any(used: u64, rng: &mut Rng) -> [Card; 2] {
    loop {
        let a = (rng.next_u64() % NUM_CARDS as u64) as Card;
        let b = (rng.next_u64() % NUM_CARDS as u64) as Card;
        if a != b && used & (bit(a) | bit(b)) == 0 {
            return [a, b];
        }
    }
}

/// Five board cards avoiding `used`, as a mask.
fn deal_board(mut used: u64, rng: &mut Rng) -> u64 {
    let mut board = 0;
    let mut n = 0;
    while n < 5 {
        let c = (rng.next_u64() % NUM_CARDS as u64) as Card;
        if used & bit(c) == 0 {
            used |= bit(c);
            board |= bit(c);
            n += 1;
        }
    }
    board
}

fn hand_mask(h: [Card; 2]) -> u64 {
    bit(h[0]) | bit(h[1])
}

/// The pot share of the first hand against the others on one board.
fn share(hands: &[[Card; 2]], board: u64) -> f64 {
    let scores: Vec<u32> = hands.iter().map(|&h| score(board | hand_mask(h))).collect();
    let best = *scores.iter().max().expect("hands");
    if scores[0] < best {
        return 0.0;
    }
    1.0 / scores.iter().filter(|&&s| s == best).count() as f64
}

/// Preflop equity tables, over the 169 hold'em classes or, for PLO, the
/// preflop buckets (`sixmax::plo`).
#[derive(Clone, Debug, PartialEq)]
pub struct PreflopEquity {
    /// Hand classes.
    n: usize,
    /// `[a * n + b]`: class `a`'s pot share against class `b`.
    heads_up: Vec<f32>,
    /// `[k - 1][class]`: the pot share against `k` random hands.
    vs_random: Vec<Vec<f32>>,
}

impl PreflopEquity {
    /// Samples the tables: `samples` boards per class pair and per class and
    /// opponent count, over `threads` threads.
    pub fn compute(samples: u32, seed: u64, threads: usize) -> Self {
        let n = NUM_PREFLOP_CLASSES;
        let rows: Vec<(usize, Vec<f32>, [f32; MAX_OPPONENTS])> = thread::scope(|scope| {
            let handles: Vec<_> = (0..threads.max(1))
                .map(|t| {
                    scope.spawn(move || {
                        (t..n)
                            .step_by(threads.max(1))
                            .map(|a| {
                                let mut rng = Rng::for_iteration(seed, a as u64);
                                let row = (0..n)
                                    .map(|b| Self::sample_pair(a, b, samples, &mut rng))
                                    .collect();
                                let mut multi = [0f32; MAX_OPPONENTS];
                                for (k, m) in multi.iter_mut().enumerate() {
                                    *m = Self::sample_random(a, k + 1, samples, &mut rng);
                                }
                                (a, row, multi)
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().expect("thread"))
                .collect()
        });
        let mut heads_up = vec![0f32; n * n];
        let mut vs_random = vec![vec![0f32; n]; MAX_OPPONENTS];
        for (a, row, multi) in rows {
            heads_up[a * n..(a + 1) * n].copy_from_slice(&row);
            for k in 0..MAX_OPPONENTS {
                vs_random[k][a] = multi[k];
            }
        }
        Self::from_tables(n, heads_up, vs_random)
    }

    /// Tables sampled elsewhere (PLO buckets): `heads_up[a * n + b]` and
    /// `vs_random[k - 1][a]`. Each pair is made to add up to one exactly, by
    /// averaging the two estimates.
    pub fn from_tables(n: usize, mut heads_up: Vec<f32>, vs_random: Vec<Vec<f32>>) -> Self {
        assert_eq!(heads_up.len(), n * n, "an n × n table");
        assert_eq!(vs_random.len(), MAX_OPPONENTS, "one row per opponent count");
        assert!(vs_random.iter().all(|r| r.len() == n), "n per row");
        for a in 0..n {
            for b in a..n {
                let x = (heads_up[a * n + b] + 1.0 - heads_up[b * n + a]) / 2.0;
                heads_up[a * n + b] = x;
                heads_up[b * n + a] = 1.0 - x;
            }
        }
        Self {
            n,
            heads_up,
            vs_random,
        }
    }

    /// Hand classes the tables cover.
    pub fn classes(&self) -> usize {
        self.n
    }

    fn sample_pair(a: usize, b: usize, samples: u32, rng: &mut Rng) -> f32 {
        let mut total = 0.0;
        let mut done = 0u32;
        let mut tries = 0u32;
        while done < samples && tries < samples * 4 {
            tries += 1;
            let Some(ha) = deal_class(a, 0, rng) else {
                break;
            };
            let Some(hb) = deal_class(b, hand_mask(ha), rng) else {
                continue;
            };
            let board = deal_board(hand_mask(ha) | hand_mask(hb), rng);
            total += share(&[ha, hb], board);
            done += 1;
        }
        if done == 0 {
            0.5
        } else {
            (total / done as f64) as f32
        }
    }

    fn sample_random(a: usize, opponents: usize, samples: u32, rng: &mut Rng) -> f32 {
        let mut total = 0.0;
        for _ in 0..samples {
            let ha = deal_class(a, 0, rng).expect("a hand");
            let mut used = hand_mask(ha);
            let mut hands = vec![ha];
            for _ in 0..opponents {
                let h = deal_any(used, rng);
                used |= hand_mask(h);
                hands.push(h);
            }
            total += share(&hands, deal_board(used, rng));
        }
        (total / samples as f64) as f32
    }

    /// Class `a`'s pot share against class `b`, heads-up.
    pub fn heads_up(&self, a: usize, b: usize) -> f64 {
        self.heads_up[a * self.n + b] as f64
    }

    /// Class `a`'s pot share against `opponents` random hands (1 to 5).
    pub fn vs_random(&self, a: usize, opponents: usize) -> f64 {
        self.vs_random[opponents.clamp(1, MAX_OPPONENTS) - 1][a] as f64
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 + 4 * (self.heads_up.len() + 5 * self.n));
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&(self.n as u32).to_le_bytes());
        for x in self
            .heads_up
            .iter()
            .chain(self.vs_random.iter().flat_map(|r| r.iter()))
        {
            out.extend_from_slice(&x.to_le_bytes());
        }
        out
    }

    /// Reads [`Self::to_bytes`]: any number of classes (169 for hold'em).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 16 || &bytes[..8] != MAGIC {
            return Err("not a preflop equity table".into());
        }
        let word = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().expect("4 bytes"));
        if word(8) != VERSION {
            return Err("preflop equity table of another version".into());
        }
        let n = word(12) as usize;
        if n == 0 || bytes.len() != 16 + 4 * (n * n + MAX_OPPONENTS * n) {
            return Err("not a preflop equity table".into());
        }
        let floats: Vec<f32> = bytes[16..]
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes(c.try_into().expect("4 bytes")))
            .collect();
        let heads_up = floats[..n * n].to_vec();
        let vs_random = floats[n * n..]
            .chunks_exact(n)
            .map(|c| c.to_vec())
            .collect();
        Ok(Self {
            n,
            heads_up,
            vs_random,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::holdem::chart::class_name;

    fn class(name: &str) -> usize {
        (0..NUM_PREFLOP_CLASSES)
            .find(|&c| class_name(c) == name)
            .expect("class")
    }

    #[test]
    fn classes_round_trip() {
        for c in 0..NUM_PREFLOP_CLASSES {
            let (hi, lo, suited) = class_ranks(c);
            assert_eq!(class_of(hi, lo, suited), c);
        }
    }

    #[test]
    fn known_matchups() {
        let mut rng = Rng::new(7);
        let near = |x: f32, want: f32| (x - want).abs() < 0.02;
        let pair = |a: &str, b: &str, rng: &mut Rng| {
            PreflopEquity::sample_pair(class(a), class(b), 5000, rng)
        };
        // Well-known values: AA vs KK about 82%, AKo vs 22 about 47%, AKs vs
        // QQ about 46%, and a class against itself 50%.
        assert!(near(pair("AA", "KK", &mut rng), 0.82));
        assert!(near(pair("AKo", "22", &mut rng), 0.47));
        assert!(near(pair("AKs", "QQ", &mut rng), 0.46));
        assert!(near(pair("T9s", "T9s", &mut rng), 0.5));
        // AA against one random hand about 85%, five about 49%; 72o against
        // one about 35%.
        let random =
            |a: &str, k: usize, rng: &mut Rng| PreflopEquity::sample_random(class(a), k, 5000, rng);
        assert!(near(random("AA", 1, &mut rng), 0.85));
        assert!(near(random("AA", 5, &mut rng), 0.49));
        assert!(near(random("72o", 1, &mut rng), 0.35));
    }

    #[test]
    fn tables_add_up_and_round_trip() {
        let t = PreflopEquity::compute(8, 7, 4);
        for a in 0..NUM_PREFLOP_CLASSES {
            for b in 0..NUM_PREFLOP_CLASSES {
                assert!((t.heads_up(a, b) + t.heads_up(b, a) - 1.0).abs() < 1e-6);
            }
        }
        assert_eq!(PreflopEquity::from_bytes(&t.to_bytes()), Ok(t));
    }
}
