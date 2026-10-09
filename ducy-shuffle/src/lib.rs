//! A trustless ("mental poker") shuffle: the deck is encrypted and shuffled
//! by every player and the host in turn, so no single party controls or sees
//! the deal (allenpiscitello/ducy#134, step 2: #136).
//!
//! # How it works
//!
//! Each card is a fixed point on Ristretto255 ([`card_point`]). Every party
//! makes a fresh [`Secret`] per hand. *Locking* a card multiplies its point by
//! the secret, *unlocking* by the secret's inverse. Scalar multiplication
//! commutes, so locks can be added and removed in any order:
//!
//! 1. **Shuffle rounds** ([`shuffle_round`]): starting from [`open_deck`], each
//!    party in turn (players in seat order, then the host) locks every card
//!    and applies a secret random permutation, and passes the deck on. Once
//!    every party has gone, no one knows which position holds which card.
//! 2. **Lock removal** ([`Layout`]): for each player's hole-card positions,
//!    every *other* party removes its lock ([`Secret::unlock`]), leaving only
//!    the owner's; the owner unlocks their own and [`decode`]s them. For the
//!    board positions, every *player* removes their lock, leaving only the
//!    host's.
//! 3. **Streets:** the host removes its last lock from one street's
//!    positions at a time, so the board is public only when it's dealt.
//! 4. **Showdown:** a player publishes their secret; anyone can unlock that
//!    player's hole positions with it and see their cards.
//! 5. **Audit** ([`audit`]): after the hand, every party publishes its secret
//!    and permutation, and anyone can re-check the whole deal.
//!
//! The host never sees anyone's hole cards. The host does see the board
//! before it's dealt (decided on #134: the host has no seat at club tables).
//!
//! Every step can carry a zero-knowledge proof ([`proof`], #140), checked
//! before the deal goes on, and [`setup`] runs a whole deck's setup that way
//! for the host, leaving out anyone who drops out (#137).
//!
//! This is the cryptographic core only: no networking or table logic. It
//! should be reviewed by someone with cryptography expertise before it's
//! trusted with real games.
//!
//! ```
//! use ducy_shuffle::{decode, open_deck, shuffle_round, Layout, Secret};
//!
//! // Two players and the host, Hold'em.
//! let layout = Layout::new(2, 2, 5);
//! let secrets: Vec<Secret> = (0..3).map(|_| Secret::random()).collect();
//! let mut rng: rand::rngs::StdRng = rand::make_rng();
//! let mut deck = open_deck();
//! for s in &secrets {
//!     deck = shuffle_round(&deck, s, &mut rng).0;
//! }
//! // Everyone but player 0 (the host is party 2) removes their lock from
//! // player 0's hole cards; then player 0 opens them.
//! let mut mine: Vec<_> = layout.hole(0).map(|p| deck[p]).collect();
//! for party in [1, 2] {
//!     mine = mine.iter().map(|c| secrets[party].unlock(c)).collect();
//! }
//! let cards: Vec<_> = mine.iter().map(|c| decode(&secrets[0].unlock(c)).unwrap()).collect();
//! assert_eq!(cards.len(), 2);
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod audit;
pub mod proof;
pub mod setup;

pub use audit::{Fault, FaultKind, PartyRound, Transcript, Unlock, audit};
pub use proof::{PublicKey, ShuffleProof, UnlockProof};
pub use setup::{DeckSetup, Ready, Request, SetupParty, Step};

use std::collections::HashMap;
use std::ops::Range;
use std::sync::OnceLock;

use curve25519_dalek::ristretto::{CompressedRistretto, RistrettoPoint};
use curve25519_dalek::scalar::Scalar;
use ducy::deck::{Card, Deck};
use rand::{Rng, RngExt, rngs::StdRng};
use sha2::Sha512;

/// Domain separation for the card points: changing it changes every card.
const CARD_DOMAIN: &str = "ducy-shuffle/v1/card/";

/// A card as the parties see it: a point on Ristretto255, plain (a card's
/// [`card_point`]) or under any number of locks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Masked(RistrettoPoint);

impl Masked {
    /// The 32 bytes to send to other parties.
    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.compress().to_bytes()
    }

    /// A card from another party; `None` if the bytes aren't a valid point.
    pub fn from_bytes(bytes: &[u8; 32]) -> Option<Self> {
        CompressedRistretto(*bytes).decompress().map(Masked)
    }
}

/// The fixed point for `card`: a hash of its name onto the curve, so no one
/// knows any relation between two cards' points.
pub fn card_point(card: Card) -> Masked {
    Masked(RistrettoPoint::hash_from_bytes::<Sha512>(
        format!("{CARD_DOMAIN}{card}").as_bytes(),
    ))
}

/// The 52 cards, unlocked, in ducy's order: where every shuffle starts.
pub fn open_deck() -> Vec<Masked> {
    all_cards().iter().map(|&c| card_point(c)).collect()
}

fn all_cards() -> &'static [Card] {
    static CARDS: OnceLock<Vec<Card>> = OnceLock::new();
    CARDS.get_or_init(|| Deck::all_cards().iter(false).collect())
}

/// The card an unlocked point stands for, or `None` if it isn't one (some
/// lock is still on it, or it was tampered with).
pub fn decode(m: &Masked) -> Option<Card> {
    static TABLE: OnceLock<HashMap<[u8; 32], Card>> = OnceLock::new();
    TABLE
        .get_or_init(|| {
            all_cards()
                .iter()
                .map(|&c| (card_point(c).to_bytes(), c))
                .collect()
        })
        .get(&m.to_bytes())
        .copied()
}

/// One party's key for one hand. Never reuse it: after the hand it's
/// published for the audit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Secret(Scalar);

impl Secret {
    /// A fresh secret from the operating system's random source.
    pub fn random() -> Self {
        let mut rng: StdRng = rand::make_rng();
        Self::from_rng(&mut rng)
    }

    /// A secret from `rng` (seeded RNGs are for tests).
    pub fn from_rng<R: Rng + ?Sized>(rng: &mut R) -> Self {
        loop {
            let mut wide = [0u8; 64];
            rng.fill_bytes(&mut wide);
            let s = Scalar::from_bytes_mod_order_wide(&wide);
            if s != Scalar::ZERO {
                return Secret(s);
            }
        }
    }

    /// Adds this party's lock to a card.
    pub fn lock(&self, m: &Masked) -> Masked {
        Masked(m.0 * self.0)
    }

    /// Removes this party's lock from a card (it must be on it, or the
    /// result is garbage that decodes to nothing).
    pub fn unlock(&self, m: &Masked) -> Masked {
        Masked(m.0 * self.0.invert())
    }

    /// The 32 bytes to publish (at showdown, or for the audit).
    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.to_bytes()
    }

    /// A published secret; `None` if it isn't a valid, non-zero scalar.
    pub fn from_bytes(bytes: &[u8; 32]) -> Option<Self> {
        Option::<Scalar>::from(Scalar::from_canonical_bytes(*bytes))
            .filter(|s| *s != Scalar::ZERO)
            .map(Secret)
    }
}

/// One party's turn: lock every card and shuffle. Returns the new deck and
/// the permutation used (`out[i]` is `deck[perm[i]]`, locked), which the
/// party keeps secret until the audit.
pub fn shuffle_round<R: Rng + ?Sized>(
    deck: &[Masked],
    secret: &Secret,
    rng: &mut R,
) -> (Vec<Masked>, Vec<u32>) {
    let mut perm: Vec<u32> = (0..deck.len() as u32).collect();
    for i in 0..perm.len() {
        let j = rng.random_range(i..perm.len());
        perm.swap(i, j);
    }
    let out = apply_round(deck, secret, &perm).expect("a permutation of the deck");
    (out, perm)
}

/// A shuffle round from its published secret and permutation: what the
/// party should have sent. `None` if `perm` isn't a permutation of the deck.
pub fn apply_round(deck: &[Masked], secret: &Secret, perm: &[u32]) -> Option<Vec<Masked>> {
    if !is_permutation(perm, deck.len()) {
        return None;
    }
    Some(
        perm.iter()
            .map(|&i| secret.lock(&deck[i as usize]))
            .collect(),
    )
}

fn is_permutation(perm: &[u32], n: usize) -> bool {
    if perm.len() != n {
        return false;
    }
    let mut seen = vec![false; n];
    perm.iter().all(|&i| {
        let i = i as usize;
        i < n && !std::mem::replace(&mut seen[i], true)
    })
}

/// Which deck positions are whose once the shuffle is done: each player's
/// hole cards in seat order, then the board. Parties are numbered players
/// `0..players`, then the host ([`Layout::host`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// Players dealt in.
    pub players: usize,
    /// Hole cards each.
    pub hole: usize,
    /// Board cards.
    pub board: usize,
}

impl Layout {
    /// # Panics
    /// If the cards don't fit in one deck.
    pub fn new(players: usize, hole: usize, board: usize) -> Self {
        assert!(players * hole + board <= 52, "not enough cards");
        Layout {
            players,
            hole,
            board,
        }
    }

    /// The host's party number: after the players.
    pub fn host(&self) -> usize {
        self.players
    }

    /// Player `p`'s hole-card positions.
    pub fn hole(&self, p: usize) -> Range<usize> {
        p * self.hole..(p + 1) * self.hole
    }

    /// The board's positions, in dealing order.
    pub fn board(&self) -> Range<usize> {
        self.players * self.hole..self.players * self.hole + self.board
    }

    /// Who must remove their lock from position `pos` during setup: for a
    /// hole card everyone but its owner; for a board card every player (the
    /// host keeps its lock until the street is dealt). Other positions are
    /// never dealt.
    pub fn unlockers(&self, pos: usize) -> Vec<usize> {
        if pos < self.players * self.hole {
            let owner = pos / self.hole;
            (0..=self.players).filter(|&q| q != owner).collect()
        } else if self.board().contains(&pos) {
            (0..self.players).collect()
        } else {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests;
