//! Imitation data for the network bots (#216, stage 1): every decision the
//! rule-based personality bots make, as the network sees it (the decision
//! features, #214) with the action class they chose (`net::ActionClass`).
//!
//! [`Recorder`] wraps a [`PersonalityBot`]: at each decision it computes the
//! features for the bot's own style in that spot, asks the bot, records the
//! pair and plays the bot's action unchanged.
//!
//! The file format, little-endian:
//!
//! - header (20 bytes): magic `DUCYNND\0`, version `u32` (1), feature count
//!   `u32` ([`NUM_FEATURES`]), class count `u32` ([`NUM_CLASSES`]);
//! - then records of [`RECORD_BYTES`] each: the features as `f32`s, the
//!   chosen class `u8`, the legal classes as a bit mask `u8` (bit `i` for
//!   class `i`), the personality `u8` (its index in `Personality::ALL`) and
//!   the game `u8` ([`game_id`]).

use std::{cell::RefCell, rc::Rc};

use rand::{SeedableRng, rngs::StdRng};

use crate::{
    bot::{Bot, HandSummary, Observation, fallback_action},
    features::{FeatureConfig, NUM_FEATURES, decision_features},
    hand::Action,
    net::{ActionClass, NUM_CLASSES},
    personality::PersonalityBot,
    rules::Variant,
    stats::OpponentModel,
};

/// The file's first bytes.
pub const MAGIC: &[u8; 8] = b"DUCYNND\0";
/// The format version this code writes and reads.
pub const VERSION: u32 = 1;
/// Header bytes.
pub const HEADER_BYTES: usize = 20;
/// Bytes per record.
pub const RECORD_BYTES: usize = NUM_FEATURES * 4 + 4;

/// One decision.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    /// The decision features (#214).
    pub features: [f32; NUM_FEATURES],
    /// The class of the action taken (`ActionClass::index`).
    pub class: u8,
    /// The legal classes, bit `i` for class `i`.
    pub mask: u8,
    /// The personality, its index in `Personality::ALL`.
    pub personality: u8,
    /// The game ([`game_id`]).
    pub game: u8,
}

/// A game's id in the data: 0 Hold'em, then the Omaha hole-card count
/// (4, 5 or 6).
pub fn game_id(variant: Variant) -> u8 {
    match variant {
        Variant::Holdem => 0,
        Variant::Omaha { hole_cards } => hole_cards as u8,
    }
}

/// A mask's bits from the legal classes.
pub fn mask_bits(mask: &[bool; NUM_CLASSES]) -> u8 {
    mask.iter()
        .enumerate()
        .fold(0, |m, (i, &on)| if on { m | 1 << i } else { m })
}

/// The header, for the start of a file.
pub fn header() -> Vec<u8> {
    let mut h = Vec::with_capacity(HEADER_BYTES);
    h.extend_from_slice(MAGIC);
    h.extend_from_slice(&VERSION.to_le_bytes());
    h.extend_from_slice(&(NUM_FEATURES as u32).to_le_bytes());
    h.extend_from_slice(&(NUM_CLASSES as u32).to_le_bytes());
    h
}

impl Record {
    /// Appends the record's bytes to `out`.
    pub fn write(&self, out: &mut Vec<u8>) {
        for f in self.features {
            out.extend_from_slice(&f.to_le_bytes());
        }
        out.extend_from_slice(&[self.class, self.mask, self.personality, self.game]);
    }

    fn read(b: &[u8]) -> Self {
        let mut features = [0f32; NUM_FEATURES];
        for (i, f) in features.iter_mut().enumerate() {
            *f = f32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().expect("four bytes"));
        }
        let t = &b[NUM_FEATURES * 4..];
        Self {
            features,
            class: t[0],
            mask: t[1],
            personality: t[2],
            game: t[3],
        }
    }
}

/// Why a file couldn't be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataError {
    /// Not a dataset (wrong magic, or too short).
    NotADataset,
    /// Another version, or other feature or class counts.
    Mismatch,
    /// Ends partway through a record.
    Truncated,
}

/// Every record in a dataset file's bytes.
pub fn read_records(bytes: &[u8]) -> Result<Vec<Record>, DataError> {
    if bytes.len() < HEADER_BYTES || &bytes[..8] != MAGIC {
        return Err(DataError::NotADataset);
    }
    let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().expect("four bytes"));
    if u32_at(8) != VERSION
        || u32_at(12) as usize != NUM_FEATURES
        || u32_at(16) as usize != NUM_CLASSES
    {
        return Err(DataError::Mismatch);
    }
    let body = &bytes[HEADER_BYTES..];
    if body.len() % RECORD_BYTES != 0 {
        return Err(DataError::Truncated);
    }
    Ok(body.chunks_exact(RECORD_BYTES).map(Record::read).collect())
}

/// Plays as a personality bot and records each of its decisions into a
/// shared list (several recorders at one table share one).
pub struct Recorder {
    bot: PersonalityBot,
    personality: u8,
    model: OpponentModel,
    rng: StdRng,
    config: FeatureConfig,
    out: Rc<RefCell<Vec<Record>>>,
}

impl Recorder {
    /// Records `bot`, personality index `personality`, into `out`; `seed`
    /// drives the features' equity sampling.
    pub fn new(
        bot: PersonalityBot,
        personality: u8,
        seed: u64,
        out: Rc<RefCell<Vec<Record>>>,
    ) -> Self {
        Self {
            bot,
            personality,
            model: OpponentModel::new(),
            rng: StdRng::seed_from_u64(seed),
            config: FeatureConfig::default(),
            out,
        }
    }
}

impl Bot for Recorder {
    fn act(&mut self, obs: &Observation) -> Option<Action> {
        let style = self.bot.style_for(obs);
        let features = decision_features(obs, &self.model, &style, &self.config, &mut self.rng);
        let action = self.bot.act(obs);
        // What the table will play: the bot's action, or the fallback.
        let played = action.unwrap_or_else(|| fallback_action(&obs.legal));
        self.out.borrow_mut().push(Record {
            features,
            class: ActionClass::of(obs, played).index() as u8,
            mask: mask_bits(&ActionClass::mask(obs)),
            personality: self.personality,
            game: game_id(obs.rules.variant),
        });
        action
    }

    fn hand_over(&mut self, summary: &HandSummary) {
        self.model
            .record(&summary.history, summary.result.net.len());
        self.bot.hand_over(summary);
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{MatchConfig, TableRules, personality::Personality, run_match};

    #[test]
    fn records_round_trip_and_match_the_play() {
        let out = Rc::new(RefCell::new(Vec::new()));
        let mut bots: Vec<Box<dyn Bot>> = Personality::ALL[..3]
            .iter()
            .enumerate()
            .map(|(i, p)| {
                Box::new(Recorder::new(
                    p.bot(Some(i as u64)),
                    i as u8,
                    i as u64,
                    out.clone(),
                )) as Box<dyn Bot>
            })
            .collect();
        run_match(
            &MatchConfig::new(TableRules::no_limit_holdem(1, 2), 5, 9),
            &mut bots,
        )
        .expect("match");
        let records = out.borrow().clone();
        assert!(!records.is_empty());
        for r in &records {
            assert!(usize::from(r.class) < NUM_CLASSES);
            assert!(
                r.mask & (1 << r.class) != 0,
                "the class played is legal: {r:?}"
            );
            assert!(r.personality < 3 && r.game == 0);
        }
        let mut bytes = header();
        for r in &records {
            r.write(&mut bytes);
        }
        assert_eq!(bytes.len(), HEADER_BYTES + records.len() * RECORD_BYTES);
        assert_eq!(read_records(&bytes), Ok(records));
        assert_eq!(
            read_records(&bytes[..bytes.len() - 1]),
            Err(DataError::Truncated)
        );
        assert_eq!(read_records(b"nope"), Err(DataError::NotADataset));
        let mut other = bytes.clone();
        other[8] = 2;
        assert_eq!(read_records(&other), Err(DataError::Mismatch));
    }

    #[test]
    fn game_ids_and_masks() {
        assert_eq!(game_id(Variant::Holdem), 0);
        assert_eq!(game_id(Variant::Omaha { hole_cards: 6 }), 6);
        let mut m = [false; NUM_CLASSES];
        m[1] = true;
        m[6] = true;
        assert_eq!(mask_bits(&m), 0b100_0010);
    }
}
