//! A small multilayer perceptron (#215): dense layers with ReLU between them,
//! plain matrix-vector code with no dependencies, so it runs natively and in
//! WebAssembly.
//!
//! The weight file is versioned: the magic `DUCYNET\x01`, the number of
//! layer sizes and the sizes themselves (u32 each), then each layer's weights
//! (output-major) and biases as little-endian f32. Two hidden layers of 64
//! over the 75 decision features come to about 9,700 weights, 39 KB.

use rand::{RngExt, SeedableRng, rngs::StdRng};

const MAGIC: &[u8; 8] = b"DUCYNET\x01";

/// Why a weight file didn't load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetError {
    /// Not a network file, or another version of the format.
    BadMagic,
    /// It ends early, or has bytes left over.
    WrongLength,
    /// Fewer than two layer sizes, or a size of 0.
    BadShape,
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::BadMagic => "not a ducy network file",
            Self::WrongLength => "the network file is the wrong length",
            Self::BadShape => "the network file's layer sizes are invalid",
        })
    }
}

impl std::error::Error for NetError {}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Layer {
    pub(super) inputs: usize,
    pub(super) outputs: usize,
    /// `outputs × inputs`, row by row.
    pub(super) weights: Vec<f32>,
    pub(super) biases: Vec<f32>,
}

impl Layer {
    fn forward(&self, x: &[f32], out: &mut Vec<f32>, relu: bool) {
        out.clear();
        for (o, row) in self.weights.chunks_exact(self.inputs).enumerate() {
            let mut s = self.biases[o];
            for (w, v) in row.iter().zip(x) {
                s += w * v;
            }
            out.push(if relu { s.max(0.0) } else { s });
        }
    }
}

/// A network: layer sizes from input to output, ReLU on every hidden layer.
#[derive(Clone, Debug, PartialEq)]
pub struct Mlp {
    pub(super) layers: Vec<Layer>,
}

impl Mlp {
    /// A network of `sizes` (input first) with the given weights and biases
    /// per layer; `None` if a shape doesn't fit.
    pub fn from_parts(sizes: &[usize], params: Vec<(Vec<f32>, Vec<f32>)>) -> Option<Self> {
        if sizes.len() < 2 || sizes.contains(&0) || params.len() != sizes.len() - 1 {
            return None;
        }
        let layers = params
            .into_iter()
            .zip(sizes.windows(2))
            .map(|((weights, biases), w)| {
                (weights.len() == w[0] * w[1] && biases.len() == w[1]).then_some(Layer {
                    inputs: w[0],
                    outputs: w[1],
                    weights,
                    biases,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self { layers })
    }

    /// A network of `sizes` with small random weights (He initialisation)
    /// and zero biases: a starting point for training, and for tests.
    pub fn random(sizes: &[usize], seed: u64) -> Self {
        let mut rng = StdRng::seed_from_u64(seed);
        let params = sizes
            .windows(2)
            .map(|w| {
                let scale = (2.0 / w[0] as f64).sqrt();
                let weights = (0..w[0] * w[1])
                    .map(|_| ((rng.random::<f64>() * 2.0 - 1.0) * scale * 3f64.sqrt()) as f32)
                    .collect();
                (weights, vec![0.0; w[1]])
            })
            .collect();
        Self::from_parts(sizes, params).expect("valid sizes")
    }

    /// The layer sizes, input first.
    pub fn sizes(&self) -> Vec<usize> {
        let mut s = vec![self.layers[0].inputs];
        s.extend(self.layers.iter().map(|l| l.outputs));
        s
    }

    /// Weights and biases in all.
    pub fn num_params(&self) -> usize {
        self.layers
            .iter()
            .map(|l| l.weights.len() + l.biases.len())
            .sum()
    }

    /// The output layer's raw values (logits) for `input`.
    pub fn forward(&self, input: &[f32]) -> Vec<f32> {
        assert_eq!(input.len(), self.layers[0].inputs, "input size");
        let (mut a, mut b) = (input.to_vec(), Vec::new());
        let last = self.layers.len() - 1;
        for (i, layer) in self.layers.iter().enumerate() {
            layer.forward(&a, &mut b, i < last);
            std::mem::swap(&mut a, &mut b);
        }
        a
    }

    /// A probability for each output, softmax over those `allowed`, 0 for the
    /// rest. All zeros if nothing is allowed.
    pub fn probs(&self, input: &[f32], allowed: &[bool]) -> Vec<f32> {
        let logits = self.forward(input);
        let max = logits
            .iter()
            .zip(allowed)
            .filter(|(_, ok)| **ok)
            .map(|(l, _)| *l)
            .fold(f32::NEG_INFINITY, f32::max);
        let exp: Vec<f32> = logits
            .iter()
            .zip(allowed)
            .map(|(l, ok)| if *ok { (l - max).exp() } else { 0.0 })
            .collect();
        let sum: f32 = exp.iter().sum();
        if sum > 0.0 {
            exp.iter().map(|e| e / sum).collect()
        } else {
            exp
        }
    }

    /// The weight file's bytes (see the module docs).
    pub fn save(&self) -> Vec<u8> {
        let sizes = self.sizes();
        let mut out = Vec::with_capacity(16 + 4 * (sizes.len() + self.num_params()));
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(sizes.len() as u32).to_le_bytes());
        for s in &sizes {
            out.extend_from_slice(&(*s as u32).to_le_bytes());
        }
        for l in &self.layers {
            for v in l.weights.iter().chain(&l.biases) {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        out
    }

    /// Reads a weight file.
    pub fn load(bytes: &[u8]) -> Result<Self, NetError> {
        let rest = bytes
            .strip_prefix(MAGIC.as_slice())
            .ok_or(NetError::BadMagic)?;
        let mut words = rest.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]);
        if rest.len() % 4 != 0 {
            return Err(NetError::WrongLength);
        }
        let mut next_u32 = || {
            words
                .next()
                .map(u32::from_le_bytes)
                .ok_or(NetError::WrongLength)
        };
        let n = next_u32()? as usize;
        if !(2..=16).contains(&n) {
            return Err(NetError::BadShape);
        }
        let sizes = (0..n)
            .map(|_| next_u32().map(|s| s as usize))
            .collect::<Result<Vec<_>, _>>()?;
        if sizes.iter().any(|&s| s == 0 || s > 4096) {
            return Err(NetError::BadShape);
        }
        let want: usize = sizes.windows(2).map(|w| w[0] * w[1] + w[1]).sum();
        let floats: Vec<f32> = words.map(f32::from_le_bytes).collect();
        if floats.len() != want {
            return Err(NetError::WrongLength);
        }
        let mut at = 0;
        let params = sizes
            .windows(2)
            .map(|w| {
                let weights = floats[at..at + w[0] * w[1]].to_vec();
                at += w[0] * w[1];
                let biases = floats[at..at + w[1]].to_vec();
                at += w[1];
                (weights, biases)
            })
            .collect();
        Self::from_parts(&sizes, params).ok_or(NetError::BadShape)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// 2 inputs → 3 hidden (ReLU) → 2 outputs, every value worked by hand.
    fn tiny() -> Mlp {
        Mlp::from_parts(
            &[2, 3, 2],
            vec![
                (vec![1.0, 0.0, 0.0, 1.0, -1.0, -1.0], vec![0.0, 0.5, 0.0]),
                (vec![1.0, 1.0, 1.0, 0.5, -1.0, 2.0], vec![0.0, -0.25]),
            ],
        )
        .unwrap()
    }

    #[test]
    fn forward_matches_values_worked_by_hand() {
        // Hidden: relu(2) = 2, relu(-1 + 0.5) = 0, relu(-2 + 1) = 0.
        // Out: 2 + 0 + 0 = 2, 1 - 0 + 0 - 0.25 = 0.75.
        assert_eq!(tiny().forward(&[2.0, -1.0]), vec![2.0, 0.75]);
        // Hidden: 1, 1.5, relu(-2) = 0. Out: 2.5, 0.5 - 1.5 - 0.25 = -1.25.
        assert_eq!(tiny().forward(&[1.0, 1.0]), vec![2.5, -1.25]);
    }

    #[test]
    fn probabilities_only_go_to_allowed_outputs() {
        let p = tiny().probs(&[2.0, -1.0], &[true, true]);
        let e = (0.75f32 - 2.0).exp();
        assert!((p[0] - 1.0 / (1.0 + e)).abs() < 1e-6);
        assert!((p.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        assert_eq!(tiny().probs(&[2.0, -1.0], &[false, true]), vec![0.0, 1.0]);
        assert_eq!(tiny().probs(&[2.0, -1.0], &[false, false]), vec![0.0, 0.0]);
    }

    #[test]
    fn a_saved_network_loads_back_the_same() {
        let net = Mlp::random(&[75, 64, 64, 7], 7);
        assert_eq!(net.num_params(), 75 * 64 + 64 + 64 * 64 + 64 + 64 * 7 + 7);
        let bytes = net.save();
        assert_eq!(bytes.len(), 8 + 4 * 5 + 4 * net.num_params());
        assert_eq!(Mlp::load(&bytes).unwrap(), net);
        assert_eq!(
            Mlp::load(&bytes[..bytes.len() - 4]),
            Err(NetError::WrongLength)
        );
        assert_eq!(Mlp::load(b"NOTANET!xxxx"), Err(NetError::BadMagic));
        let mut bad = bytes.clone();
        bad[12..16].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(Mlp::load(&bad), Err(NetError::BadShape));
    }
}
