//! The trained strategy for the abstract game, stored compactly.
//!
//! For every decision node and bucket, each action's probability is a byte
//! (0 to 255, rows summing to about 255), laid out densely in node order, so
//! looking up a strategy is two array reads and no hashing. The header holds
//! a hash of the game and abstraction settings, so a blueprint is never used
//! with a betting tree or card abstraction it wasn't trained for.

use super::{
    abstraction::CardAbstraction,
    hunl::{Hunl, HunlConfig},
};
use crate::profile::Profile;

/// Why a blueprint couldn't be loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlueprintError {
    /// Not a blueprint, or a truncated one.
    Corrupt,
    /// Trained for different bet sizes, stacks, blinds or buckets.
    WrongSettings,
}

/// A blueprint strategy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blueprint {
    settings: u64,
    /// Where each node's rows start in `probs` (unused for terminal nodes).
    offsets: Vec<u32>,
    /// Buckets per node (the number for its street).
    buckets: Vec<u16>,
    actions: Vec<u8>,
    probs: Vec<u8>,
}

/// A hash of everything that shapes the abstract game.
pub fn settings_hash(game: &Hunl, cards: &CardAbstraction) -> u64 {
    let text = format!("{}|{:?}|v1", config_text(&game.config), cards.config);
    // FNV-1a.
    text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The game settings as hashed. A no-limit game hashes as it did before
/// `pot_limit` existed, so blueprints trained then still load.
pub fn config_text(c: &HunlConfig) -> String {
    if c.pot_limit {
        format!("{c:?}")
    } else {
        format!(
            "HunlConfig {{ small_blind: {:?}, big_blind: {:?}, stack: {:?}, menu: {:?} }}",
            c.small_blind, c.big_blind, c.stack, c.menu
        )
    }
}

fn layout(game: &Hunl, cards: &CardAbstraction) -> (Vec<u32>, Vec<u16>, Vec<u8>, usize) {
    let mut offsets = Vec::with_capacity(game.tree.nodes.len());
    let mut buckets = Vec::with_capacity(game.tree.nodes.len());
    let mut actions = Vec::with_capacity(game.tree.nodes.len());
    let mut at = 0usize;
    for n in &game.tree.nodes {
        let b = cards.num_buckets([0, 3, 4, 5][n.betting.street]) as u16;
        offsets.push(at as u32);
        buckets.push(b);
        actions.push(n.actions.len() as u8);
        at += n.actions.len() * b as usize;
    }
    (offsets, buckets, actions, at)
}

/// Rounds probabilities to bytes that sum to 255.
fn quantize(p: &[f64], out: &mut [u8]) {
    let mut q: Vec<u32> = p
        .iter()
        .map(|&x| (x.max(0.0) * 255.0).floor() as u32)
        .collect();
    let short = 255 - q.iter().sum::<u32>().min(255);
    // Hand the rounding remainder to the largest fractional parts.
    let mut order: Vec<usize> = (0..p.len()).collect();
    order.sort_by(|&a, &b| (p[b] * 255.0).fract().total_cmp(&(p[a] * 255.0).fract()));
    for &i in order.iter().cycle().take(short as usize) {
        q[i] += 1;
    }
    for (o, &x) in out.iter_mut().zip(&q) {
        *o = x.min(255) as u8;
    }
}

impl Blueprint {
    /// Stores `profile` (e.g. a solver's average strategy). Information sets
    /// the profile doesn't have play uniformly.
    pub fn from_profile(game: &Hunl, cards: &CardAbstraction, profile: &Profile<u64>) -> Self {
        let (offsets, buckets, actions, len) = layout(game, cards);
        let mut probs = vec![0u8; len];
        for (node, n) in game.tree.nodes.iter().enumerate() {
            let k = n.actions.len();
            if k == 0 {
                continue;
            }
            for b in 0..buckets[node] as usize {
                let p = profile.probs(&((node as u64) << 16 | b as u64), k);
                let at = offsets[node] as usize + b * k;
                quantize(&p, &mut probs[at..at + k]);
            }
        }
        Self {
            settings: settings_hash(game, cards),
            offsets,
            buckets,
            actions,
            probs,
        }
    }

    /// Action probabilities at betting node `node` for a hand in `bucket`.
    pub fn probs(&self, node: u32, bucket: u16) -> Vec<f64> {
        let k = self.actions[node as usize] as usize;
        let at = self.offsets[node as usize] as usize + bucket as usize * k;
        let row = &self.probs[at..at + k];
        let total: u32 = row.iter().map(|&x| x as u32).sum();
        if total == 0 {
            return vec![1.0 / k as f64; k];
        }
        row.iter().map(|&x| x as f64 / total as f64).collect()
    }

    /// Bytes of strategy data.
    pub fn len(&self) -> usize {
        self.probs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.probs.is_empty()
    }

    pub fn save(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend(self.settings.to_le_bytes());
        out.extend((self.offsets.len() as u64).to_le_bytes());
        out.extend((self.probs.len() as u64).to_le_bytes());
        out.extend(&self.probs);
        out
    }

    /// Loads a blueprint saved for exactly this game and abstraction.
    pub fn load(
        bytes: &[u8],
        game: &Hunl,
        cards: &CardAbstraction,
    ) -> Result<Self, BlueprintError> {
        use crate::key::{read_u64, take};
        let mut input = bytes;
        let input = &mut input;
        if take(input, MAGIC.len()).ok_or(BlueprintError::Corrupt)? != MAGIC {
            return Err(BlueprintError::Corrupt);
        }
        let settings = read_u64(input).ok_or(BlueprintError::Corrupt)?;
        let nodes = read_u64(input).ok_or(BlueprintError::Corrupt)? as usize;
        let len = read_u64(input).ok_or(BlueprintError::Corrupt)? as usize;
        let probs = take(input, len).ok_or(BlueprintError::Corrupt)?.to_vec();
        if !input.is_empty() {
            return Err(BlueprintError::Corrupt);
        }
        let (offsets, buckets, actions, expected) = layout(game, cards);
        if settings != settings_hash(game, cards) || nodes != offsets.len() || len != expected {
            return Err(BlueprintError::WrongSettings);
        }
        Ok(Self {
            settings,
            offsets,
            buckets,
            actions,
            probs,
        })
    }
}

const MAGIC: &[u8] = b"DUCYBLUE\x01";
