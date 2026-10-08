//! Provably fair deals with a commit–reveal seed (#141), an interim step
//! before the full trustless shuffle (#134).
//!
//! Before a hand, every seated player and the host each pick a random
//! 256-bit value and a nonce, and send the commitment
//! `SHA-256("ducy-fair-deal-v1/commit" ‖ value ‖ nonce)`, signed. Once every
//! commitment is in, everyone reveals value and nonce, and each reveal is
//! checked against its commitment. The hand's seed is
//! `SHA-256("ducy-fair-deal-v1/seed" ‖ count ‖ values…)` over the valid
//! reveals in seat order, and the deck is shuffled from it
//! ([`shuffled_deck`]). After the hand, anyone can rebuild the deal from the
//! published reveals ([`check_deal`]).
//!
//! The host can't choose or steer cards: as long as one value was truly
//! random, so is the order. It does still see the whole deck, which the
//! trustless shuffle fixes. Whoever reveals last could refuse after seeing
//! the others' values; a missing or wrong reveal sits that player out of the
//! hand and the seed comes from everyone else ([`FairRound::outcome`]), so
//! refusing gains nothing.
//!
//! Everything here is fully specified with SHA-256, so a client in another
//! language can check a deal without ducy:
//!
//! - **Deck order:** ducy's standard order, `Deck::all_cards().iter(false)`.
//!   A test pins it, so it can't change without notice.
//! - **Shuffle:** Fisher–Yates from the last position down: for `i` from 51
//!   to 1, swap position `i` with `j`, a uniform draw from `0..=i`.
//! - **Draws:** 64-bit words `w_k` = the first 8 bytes (big-endian) of
//!   `SHA-256("ducy-fair-deal-v1/shuffle" ‖ seed ‖ k)`, with `k` a big-endian
//!   u64 counting from 0 across the whole shuffle. A draw from `0..=i` takes
//!   words until one is below `2^64 − (2^64 mod (i + 1))`, then `j = w mod (i + 1)`.
//! - **Dealing:** each player's hole cards in turn (dealing order), then
//!   the five board cards ([`crate::Deal::from_seed`]).
//!
//! ```
//! use ducy_play::fair_deal::{FairRound, Reveal, check_deal};
//! use ducy_play::{Deal, Variant};
//!
//! // Two players and the host, in seat order (the host last).
//! let mine: Vec<Reveal> = (0..3).map(|_| Reveal::random()).collect();
//! let mut round = FairRound::new(3);
//! for (i, r) in mine.iter().enumerate() {
//!     round.commit(i, r.commitment()).unwrap();
//! }
//! for (i, r) in mine.iter().enumerate() {
//!     round.reveal(i, *r).unwrap();
//! }
//! let out = round.outcome();
//! let deal = Deal::from_seed(Variant::Holdem, 2, &out.seed).unwrap();
//! // Anyone can check it afterwards from the published reveals.
//! assert!(check_deal(Variant::Holdem, 2, &out.reveals, &deal));
//! ```

use ducy::deck::{Card, Deck};
use rand::{RngExt, rngs::StdRng};
use sha2::{Digest, Sha256};

use crate::{Deal, rules::Variant};

/// 32 bytes: a value, a nonce, a commitment or a seed.
pub type Bytes32 = [u8; 32];

const COMMIT: &[u8] = b"ducy-fair-deal-v1/commit";
const SEED: &[u8] = b"ducy-fair-deal-v1/seed";
const SHUFFLE: &[u8] = b"ducy-fair-deal-v1/shuffle";

/// One participant's secret for a hand: a random value and a nonce.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Reveal {
    /// The random value that goes into the hand's seed.
    pub value: Bytes32,
    /// Random padding that hides the value inside the commitment.
    pub nonce: Bytes32,
}

impl Reveal {
    /// A fresh value and nonce from system randomness (the OS, or
    /// `crypto.getRandomValues` in a browser).
    pub fn random() -> Self {
        let mut rng: StdRng = rand::make_rng();
        Self {
            value: rng.random(),
            nonce: rng.random(),
        }
    }

    /// What to send first: `SHA-256("ducy-fair-deal-v1/commit" ‖ value ‖ nonce)`.
    pub fn commitment(&self) -> Bytes32 {
        let mut h = Sha256::new();
        h.update(COMMIT);
        h.update(self.value);
        h.update(self.nonce);
        h.finalize().into()
    }

    /// Whether this reveal opens `commitment`.
    pub fn opens(&self, commitment: &Bytes32) -> bool {
        &self.commitment() == commitment
    }
}

/// The hand's seed from the revealed values, in seat order:
/// `SHA-256("ducy-fair-deal-v1/seed" ‖ count as u32 big-endian ‖ values…)`.
pub fn hand_seed(values: &[Bytes32]) -> Bytes32 {
    let mut h = Sha256::new();
    h.update(SEED);
    h.update((values.len() as u32).to_be_bytes());
    for v in values {
        h.update(v);
    }
    h.finalize().into()
}

/// The 52 cards shuffled from `seed` (see the module docs for the exact
/// procedure).
pub fn shuffled_deck(seed: &Bytes32) -> Vec<Card> {
    let mut cards: Vec<Card> = Deck::all_cards().iter(false).collect();
    let mut words = Words { seed, k: 0 };
    for i in (1..cards.len()).rev() {
        let j = words.below(i as u64 + 1) as usize;
        cards.swap(i, j);
    }
    cards
}

/// SHA-256 in counter mode, as a stream of 64-bit words.
struct Words<'a> {
    seed: &'a Bytes32,
    k: u64,
}

impl Words<'_> {
    fn next(&mut self) -> u64 {
        let mut h = Sha256::new();
        h.update(SHUFFLE);
        h.update(self.seed);
        h.update(self.k.to_be_bytes());
        self.k += 1;
        let d = h.finalize();
        u64::from_be_bytes(d[..8].try_into().expect("8 bytes"))
    }

    /// A uniform draw from `0..n` by rejection sampling.
    fn below(&mut self, n: u64) -> u64 {
        // 2^64 mod n: words at or above 2^64 − r would favour small results.
        let r = (u64::MAX % n + 1) % n;
        loop {
            let w = self.next();
            if r == 0 || w < r.wrapping_neg() {
                return w % n;
            }
        }
    }
}

/// Rebuilds a deal from published reveals (in seat order, as they were used
/// for the seed) and checks every card against `deal`.
pub fn check_deal(variant: Variant, players: usize, reveals: &[Reveal], deal: &Deal) -> bool {
    let values: Vec<Bytes32> = reveals.iter().map(|r| r.value).collect();
    Deal::from_seed(variant, players, &hand_seed(&values)).is_ok_and(|d| &d == deal)
}

/// Why a commitment or reveal was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FairError {
    /// No such participant.
    UnknownParticipant,
    /// A second commitment, or a commitment after the reveals started.
    AlreadyCommitted,
    /// A reveal before every commitment was in, or without a commitment.
    NotCommitted,
    /// A second reveal.
    AlreadyRevealed,
    /// The reveal doesn't open the participant's commitment: they sit out.
    Mismatch,
}

impl std::fmt::Display for FairError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            FairError::UnknownParticipant => "not in this deal",
            FairError::AlreadyCommitted => "already committed",
            FairError::NotCommitted => "reveal before every commitment is in",
            FairError::AlreadyRevealed => "already revealed",
            FairError::Mismatch => "the reveal doesn't match the commitment",
        })
    }
}

impl std::error::Error for FairError {}

/// The result of a round: the seed, the reveals it came from, and who sits
/// out the hand.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FairOutcome {
    /// The hand's seed ([`hand_seed`] of the used values).
    pub seed: Bytes32,
    /// The valid reveals, in participant order: publish these after the hand.
    pub reveals: Vec<Reveal>,
    /// Participants whose values went into the seed, in order.
    pub used: Vec<usize>,
    /// Participants who didn't commit, didn't reveal, or revealed a value
    /// that doesn't match: they sit out the hand.
    pub sat_out: Vec<usize>,
}

/// One hand's commit–reveal round, kept by the host. Participants are
/// numbered in seat order (players first, then the host), as the caller
/// decides.
#[derive(Clone, Debug)]
pub struct FairRound {
    commitments: Vec<Option<Bytes32>>,
    reveals: Vec<Option<Reveal>>,
    refused: Vec<bool>,
}

impl FairRound {
    /// A round for `participants` people, with nothing committed yet.
    pub fn new(participants: usize) -> Self {
        Self {
            commitments: vec![None; participants],
            reveals: vec![None; participants],
            refused: vec![false; participants],
        }
    }

    /// Participant `who` commits. Commitments close once the first reveal
    /// arrives.
    pub fn commit(&mut self, who: usize, commitment: Bytes32) -> Result<(), FairError> {
        let slot = self
            .commitments
            .get_mut(who)
            .ok_or(FairError::UnknownParticipant)?;
        if slot.is_some() || self.reveals.iter().any(Option::is_some) {
            return Err(FairError::AlreadyCommitted);
        }
        *slot = Some(commitment);
        Ok(())
    }

    /// Whether every participant has committed, so reveals can start.
    pub fn all_committed(&self) -> bool {
        self.commitments.iter().all(Option::is_some)
    }

    /// Participant `who` reveals. A reveal that doesn't open their
    /// commitment is refused, and they sit out the hand.
    pub fn reveal(&mut self, who: usize, reveal: Reveal) -> Result<(), FairError> {
        let c = *self
            .commitments
            .get(who)
            .ok_or(FairError::UnknownParticipant)?;
        let c = c.ok_or(FairError::NotCommitted)?;
        if self.reveals[who].is_some() || self.refused[who] {
            return Err(FairError::AlreadyRevealed);
        }
        if !reveal.opens(&c) {
            self.refused[who] = true;
            return Err(FairError::Mismatch);
        }
        self.reveals[who] = Some(reveal);
        Ok(())
    }

    /// Whether everyone who committed has revealed (or been refused).
    pub fn complete(&self) -> bool {
        (0..self.reveals.len())
            .all(|i| self.commitments[i].is_none() || self.reveals[i].is_some() || self.refused[i])
    }

    /// The seed from the valid reveals so far. Call it when everyone has
    /// revealed, or when time is up: anyone without a valid reveal sits out.
    pub fn outcome(&self) -> FairOutcome {
        let used: Vec<usize> = (0..self.reveals.len())
            .filter(|&i| self.reveals[i].is_some())
            .collect();
        let reveals: Vec<Reveal> = used
            .iter()
            .map(|&i| self.reveals[i].expect("revealed"))
            .collect();
        let values: Vec<Bytes32> = reveals.iter().map(|r| r.value).collect();
        FairOutcome {
            seed: hand_seed(&values),
            sat_out: (0..self.reveals.len())
                .filter(|i| !used.contains(i))
                .collect(),
            used,
            reveals,
        }
    }
}
