//! Stage-1 training (#216): teach a network to copy the rule-based
//! personalities from recorded decisions (`net_data`).
//!
//! Plain Rust, no dependencies: backpropagation through the ReLU layers,
//! Adam, and minibatches. The loss is cross-entropy over the possible
//! action classes only (the same masked softmax `NetBot` plays from).
//!
//! Decisions can be weighted by class, so rare classes such as overbets count
//! for more: with weight power `w`, a class seen `n` times weighs
//! `(mean / n)^w`, scaled to average 1. It's off by default. Weighting picks
//! the rare classes out better but shifts the network towards them, so the
//! copies play too aggressively: on the first dataset, `w = 0.25` put 8 of
//! the 19 personalities out of trait tolerance, against 1 of 19 unweighted.

use rand::{RngExt, SeedableRng, rngs::StdRng};

use super::{Mlp, NUM_CLASSES};
use crate::net_data::Record;

/// How to train.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrainConfig {
    /// Decisions per update.
    pub batch: usize,
    /// Adam's step size.
    pub lr: f32,
    /// How much rare classes are weighted up: 0 for not at all, 1 for fully
    /// by inverse frequency.
    pub class_weight_power: f32,
    /// Seeds the shuffles.
    pub seed: u64,
}

impl Default for TrainConfig {
    fn default() -> Self {
        Self {
            batch: 256,
            lr: 1e-3,
            class_weight_power: 0.0,
            seed: 1,
        }
    }
}

/// How well a network copies a set of decisions.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Metrics {
    /// Decisions measured.
    pub n: u64,
    /// Mean cross-entropy (unweighted), in nats.
    pub loss: f64,
    /// Share of decisions where the network's likeliest class is the one
    /// taken.
    pub accuracy: f64,
    /// Per class: (likeliest when taken, times taken).
    pub per_class: [(u64, u64); NUM_CLASSES],
}

impl Metrics {
    /// Share of class `c`'s decisions the network gets right.
    pub fn recall(&self, c: usize) -> f64 {
        let (hit, n) = self.per_class[c];
        if n == 0 { 0.0 } else { hit as f64 / n as f64 }
    }
}

/// How often each class was taken in `data`.
pub fn class_counts(data: &[Record]) -> [u64; NUM_CLASSES] {
    let mut counts = [0u64; NUM_CLASSES];
    for r in data {
        counts[r.class as usize] += 1;
    }
    counts
}

/// The class weights for `counts` (see the module docs).
pub fn class_weights(counts: &[u64; NUM_CLASSES], power: f32) -> [f32; NUM_CLASSES] {
    let seen: Vec<u64> = counts.iter().copied().filter(|&n| n > 0).collect();
    let mean = seen.iter().sum::<u64>() as f64 / seen.len().max(1) as f64;
    let raw: [f64; NUM_CLASSES] = std::array::from_fn(|c| {
        if counts[c] == 0 {
            0.0
        } else {
            (mean / counts[c] as f64).powf(power as f64)
        }
    });
    // Scaled so the average decision weighs 1.
    let total: f64 = counts.iter().sum::<u64>() as f64;
    let avg = counts
        .iter()
        .zip(&raw)
        .map(|(&n, w)| n as f64 * w)
        .sum::<f64>()
        / total.max(1.0);
    std::array::from_fn(|c| (raw[c] / avg.max(1e-12)) as f32)
}

/// Gradients (or Adam's moments), shaped like the network's layers.
#[derive(Clone)]
struct Grads {
    weights: Vec<Vec<f32>>,
    biases: Vec<Vec<f32>>,
}

impl Grads {
    fn zeros(net: &Mlp) -> Self {
        Self {
            weights: net
                .layers
                .iter()
                .map(|l| vec![0.0; l.weights.len()])
                .collect(),
            biases: net
                .layers
                .iter()
                .map(|l| vec![0.0; l.biases.len()])
                .collect(),
        }
    }

    fn clear(&mut self) {
        for v in self.weights.iter_mut().chain(self.biases.iter_mut()) {
            v.fill(0.0);
        }
    }
}

/// Trains a network with Adam.
pub struct Trainer {
    /// The network being trained.
    pub net: Mlp,
    config: TrainConfig,
    weights: [f32; NUM_CLASSES],
    grads: Grads,
    m: Grads,
    v: Grads,
    steps: i32,
    rng: StdRng,
    /// Each layer's outputs for the decision being worked on (after ReLU on
    /// hidden layers); `acts[0]` is the input.
    acts: Vec<Vec<f32>>,
    deltas: Vec<Vec<f32>>,
}

impl Trainer {
    /// Trains `net` on data whose classes were taken `counts` times.
    pub fn new(net: Mlp, config: TrainConfig, counts: &[u64; NUM_CLASSES]) -> Self {
        let sizes = net.sizes();
        Self {
            weights: class_weights(counts, config.class_weight_power),
            grads: Grads::zeros(&net),
            m: Grads::zeros(&net),
            v: Grads::zeros(&net),
            steps: 0,
            rng: StdRng::seed_from_u64(config.seed),
            acts: sizes.iter().map(|&s| vec![0.0; s]).collect(),
            deltas: sizes.iter().map(|&s| vec![0.0; s]).collect(),
            net,
            config,
        }
    }

    /// The class weights in use.
    pub fn class_weights(&self) -> [f32; NUM_CLASSES] {
        self.weights
    }

    /// One pass over `data` in a fresh random order; returns the training
    /// metrics as the network was at each decision.
    pub fn epoch(&mut self, data: &[Record]) -> Metrics {
        let mut order: Vec<usize> = (0..data.len()).collect();
        for i in (1..order.len()).rev() {
            let j = self.rng.random_range(0..=i);
            order.swap(i, j);
        }
        let mut metrics = Metrics::default();
        for chunk in order.chunks(self.config.batch.max(1)) {
            self.grads.clear();
            for &i in chunk {
                self.accumulate(&data[i], &mut metrics);
            }
            self.step(chunk.len());
        }
        finish(&mut metrics);
        metrics
    }

    /// How well the network copies `data`, without training.
    pub fn evaluate(&mut self, data: &[Record]) -> Metrics {
        let mut metrics = Metrics::default();
        for r in data {
            let p = self.forward(r);
            score(&p, r, &mut metrics);
        }
        finish(&mut metrics);
        metrics
    }

    /// Runs the network on `r`, filling `acts`; returns the masked
    /// probabilities.
    fn forward(&mut self, r: &Record) -> [f32; NUM_CLASSES] {
        self.acts[0].copy_from_slice(&r.features);
        let last = self.net.layers.len() - 1;
        for (li, layer) in self.net.layers.iter().enumerate() {
            let (before, after) = self.acts.split_at_mut(li + 1);
            let x = &before[li];
            let out = &mut after[0];
            for (o, row) in layer.weights.chunks_exact(layer.inputs).enumerate() {
                let mut s = layer.biases[o];
                for (w, v) in row.iter().zip(x) {
                    s += w * v;
                }
                out[o] = if li < last { s.max(0.0) } else { s };
            }
        }
        let logits = &self.acts[last + 1];
        let allowed = |c: usize| r.mask & (1 << c) != 0;
        let max = (0..NUM_CLASSES)
            .filter(|&c| allowed(c))
            .map(|c| logits[c])
            .fold(f32::NEG_INFINITY, f32::max);
        let mut p = [0f32; NUM_CLASSES];
        let mut sum = 0.0;
        for c in 0..NUM_CLASSES {
            if allowed(c) {
                p[c] = (logits[c] - max).exp();
                sum += p[c];
            }
        }
        for q in &mut p {
            *q /= sum.max(1e-30);
        }
        p
    }

    /// Adds `r`'s gradient to `grads`.
    fn accumulate(&mut self, r: &Record, metrics: &mut Metrics) {
        let p = self.forward(r);
        score(&p, r, metrics);
        let c = r.class as usize;
        let w = self.weights[c];
        let n = self.net.layers.len();
        // The loss's gradient at the logits: weight × (p − one-hot), 0 for
        // impossible classes.
        for (k, (d, pk)) in self.deltas[n].iter_mut().zip(p).enumerate() {
            *d = if r.mask & (1 << k) != 0 {
                w * (pk - if k == c { 1.0 } else { 0.0 })
            } else {
                0.0
            };
        }
        for li in (0..n).rev() {
            let layer = &self.net.layers[li];
            let (gw, gb) = (&mut self.grads.weights[li], &mut self.grads.biases[li]);
            let delta = &self.deltas[li + 1];
            let x = &self.acts[li];
            for (o, &d) in delta.iter().enumerate() {
                if d == 0.0 {
                    continue;
                }
                gb[o] += d;
                for (g, v) in gw[o * layer.inputs..(o + 1) * layer.inputs]
                    .iter_mut()
                    .zip(x)
                {
                    *g += d * v;
                }
            }
            if li > 0 {
                // Back through the weights, then the ReLU.
                let (lower, upper) = self.deltas.split_at_mut(li + 1);
                let prev = &mut lower[li];
                prev.fill(0.0);
                for (o, &d) in upper[0].iter().enumerate() {
                    if d == 0.0 {
                        continue;
                    }
                    for (pv, wv) in prev
                        .iter_mut()
                        .zip(&layer.weights[o * layer.inputs..(o + 1) * layer.inputs])
                    {
                        *pv += d * wv;
                    }
                }
                for (pv, a) in prev.iter_mut().zip(&self.acts[li]) {
                    if *a <= 0.0 {
                        *pv = 0.0;
                    }
                }
            }
        }
    }

    /// One Adam update from the gradient summed over `n` decisions.
    fn step(&mut self, n: usize) {
        const B1: f32 = 0.9;
        const B2: f32 = 0.999;
        const EPS: f32 = 1e-8;
        self.steps += 1;
        let (c1, c2) = (1.0 - B1.powi(self.steps), 1.0 - B2.powi(self.steps));
        let scale = 1.0 / n.max(1) as f32;
        let lr = self.config.lr;
        for (li, layer) in self.net.layers.iter_mut().enumerate() {
            let update = |params: &mut [f32], g: &[f32], m: &mut [f32], v: &mut [f32]| {
                for i in 0..params.len() {
                    let gi = g[i] * scale;
                    m[i] = B1 * m[i] + (1.0 - B1) * gi;
                    v[i] = B2 * v[i] + (1.0 - B2) * gi * gi;
                    params[i] -= lr * (m[i] / c1) / ((v[i] / c2).sqrt() + EPS);
                }
            };
            update(
                &mut layer.weights,
                &self.grads.weights[li],
                &mut self.m.weights[li],
                &mut self.v.weights[li],
            );
            update(
                &mut layer.biases,
                &self.grads.biases[li],
                &mut self.m.biases[li],
                &mut self.v.biases[li],
            );
        }
    }
}

fn score(p: &[f32; NUM_CLASSES], r: &Record, metrics: &mut Metrics) {
    let c = r.class as usize;
    metrics.n += 1;
    metrics.loss += -(p[c].max(1e-12) as f64).ln();
    let best = (0..NUM_CLASSES)
        .max_by(|&a, &b| p[a].total_cmp(&p[b]))
        .unwrap_or(0);
    metrics.per_class[c].1 += 1;
    if best == c {
        metrics.per_class[c].0 += 1;
        metrics.accuracy += 1.0;
    }
}

fn finish(metrics: &mut Metrics) {
    let n = metrics.n.max(1) as f64;
    metrics.loss /= n;
    metrics.accuracy /= n;
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::features::NUM_FEATURES;

    /// Decisions where the class depends on the first two features: a
    /// network learns it.
    fn toy(n: usize, seed: u64) -> Vec<Record> {
        let mut rng = StdRng::seed_from_u64(seed);
        (0..n)
            .map(|_| {
                let mut features = [0f32; NUM_FEATURES];
                for f in &mut features {
                    *f = rng.random::<f32>();
                }
                let class = if features[0] > 0.5 {
                    1
                } else if features[1] > 0.5 {
                    0
                } else {
                    3
                };
                Record {
                    features,
                    class,
                    mask: 0b0001011,
                    personality: 0,
                    game: 0,
                }
            })
            .collect()
    }

    #[test]
    fn rare_classes_weigh_more_and_the_average_is_one() {
        let w = class_weights(&[100, 900, 0, 10, 0, 0, 0], 0.5);
        assert!(w[3] > w[0] && w[0] > w[1]);
        assert_eq!(w[2], 0.0);
        let mean = (100.0 * w[0] + 900.0 * w[1] + 10.0 * w[3]) / 1010.0;
        assert!((mean - 1.0).abs() < 1e-5);
        assert_eq!(class_weights(&[5, 5, 0, 5, 0, 0, 0], 0.0)[..2], [1.0, 1.0]);
    }

    #[test]
    fn the_gradient_matches_finite_differences() {
        let data = toy(1, 3);
        let net = Mlp::random(&[NUM_FEATURES, 5, 7], 2);
        let mut t = Trainer::new(
            net.clone(),
            TrainConfig {
                class_weight_power: 0.0,
                ..Default::default()
            },
            &[1; NUM_CLASSES],
        );
        t.grads.clear();
        t.accumulate(&data[0], &mut Metrics::default());
        let loss = |net: &Mlp| {
            let mut t = Trainer::new(net.clone(), TrainConfig::default(), &[1; NUM_CLASSES]);
            let p = t.forward(&data[0]);
            -(p[data[0].class as usize] as f64).ln()
        };
        for (li, i) in [(0, 7), (0, 140), (1, 3), (1, 20)] {
            let mut up = net.clone();
            let mut down = net.clone();
            let h = 1e-3;
            up.layers[li].weights[i] += h;
            down.layers[li].weights[i] -= h;
            let numeric = (loss(&up) - loss(&down)) / (2.0 * h as f64);
            let analytic = t.grads.weights[li][i] as f64;
            assert!(
                (numeric - analytic).abs() < 1e-3 + 0.02 * numeric.abs(),
                "layer {li} weight {i}: {numeric} vs {analytic}"
            );
        }
    }

    #[test]
    fn it_learns_a_simple_rule() {
        let train = toy(4000, 1);
        let test = toy(1000, 2);
        let net = Mlp::random(&[NUM_FEATURES, 16, 7], 1);
        let mut t = Trainer::new(
            net,
            TrainConfig {
                lr: 3e-3,
                ..Default::default()
            },
            &class_counts(&train),
        );
        let before = t.evaluate(&test);
        for _ in 0..25 {
            t.epoch(&train);
        }
        let after = t.evaluate(&test);
        assert!(after.loss < before.loss * 0.5, "{before:?} → {after:?}");
        assert!(after.accuracy > 0.9, "{after:?}");
        // Only possible classes are ever chosen.
        assert_eq!(after.per_class[2].1 + after.per_class[4].1, 0);
    }
}
