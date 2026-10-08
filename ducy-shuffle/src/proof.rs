//! Zero-knowledge proofs (allenpiscitello/ducy#140, phase 2): each step of
//! the deal comes with a proof that it was done right, checked before the
//! deal goes on, so cheating is refused when it happens rather than found
//! by the audit afterwards ([`audit`](crate::audit) stays as a second check).
//!
//! Every party publishes its [`PublicKey`] (its secret times the base point)
//! before the shuffle. Then:
//!
//! - **Unlocks** ([`UnlockProof`]): a batched Chaum–Pedersen proof that every
//!   lock a party removed was removed with the secret behind its public key.
//!   One proof covers any number of cards: they're folded into one pair with
//!   random weights drawn from the statement, then a single proof of equal
//!   discrete logs is given for it.
//! - **Shuffles** ([`ShuffleProof`]): that the deck a party sent on is the deck
//!   it received, permuted and locked with that same secret, without showing
//!   the permutation or the secret. It's a cut-and-choose proof made
//!   non-interactive with Fiat–Shamir: [`SHUFFLE_ROUNDS`] shadow shuffles of
//!   the input; for each, the challenge asks for either how it was made from
//!   the input, or how the output is made from it. A cheat survives each
//!   round with probability ½, so 2^-[`SHUFFLE_ROUNDS`] in all. (Bayer–Groth
//!   proofs are much shorter but far more involved; this one is simple to
//!   check, and fast enough for a 52-card deck.)
//!
//! Every proof is bound to a `context` (say the table, the hand and the
//! party), so it can't be replayed elsewhere.

use curve25519_dalek::constants::RISTRETTO_BASEPOINT_POINT as B;
use curve25519_dalek::ristretto::{CompressedRistretto, RistrettoPoint};
use curve25519_dalek::scalar::Scalar;
use rand::{Rng, RngExt};
use sha2::{Digest, Sha512};

use crate::{Masked, Secret, apply_round};

/// Shadow shuffles in a [`ShuffleProof`]: a cheat gets through with
/// probability 2^-40.
pub const SHUFFLE_ROUNDS: usize = 40;

const UNLOCK_DOMAIN: &[u8] = b"ducy-shuffle/v1/unlock-proof";
const SHUFFLE_DOMAIN: &[u8] = b"ducy-shuffle/v1/shuffle-proof";

/// A party's public key: its [`Secret`] times the base point. Published
/// before the shuffle; every proof is checked against it, and a secret shown
/// at showdown must match it ([`PublicKey::matches`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PublicKey(RistrettoPoint);

impl PublicKey {
    /// The 32 bytes to publish.
    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.compress().to_bytes()
    }

    /// A published key; `None` if the bytes aren't a valid point.
    pub fn from_bytes(bytes: &[u8; 32]) -> Option<Self> {
        CompressedRistretto(*bytes).decompress().map(PublicKey)
    }

    /// Whether `secret` is the one behind this key (a secret published at
    /// showdown or for the audit).
    pub fn matches(&self, secret: &Secret) -> bool {
        secret.public() == *self
    }
}

impl Secret {
    /// This secret's [`PublicKey`].
    pub fn public(&self) -> PublicKey {
        PublicKey(B * self.0)
    }
}

fn point(bytes: &[u8]) -> Option<RistrettoPoint> {
    CompressedRistretto(bytes.try_into().ok()?).decompress()
}

fn scalar(bytes: &[u8]) -> Option<Scalar> {
    Option::from(Scalar::from_canonical_bytes(bytes.try_into().ok()?))
}

// ---- unlocks: batched Chaum–Pedersen ----

/// A proof that a party removed its lock from some cards correctly: for each
/// `(input, output)` pair, `output` is `input` unlocked with the secret behind
/// the party's public key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnlockProof {
    c: Scalar,
    s: Scalar,
}

/// Folds the pairs into one with weights drawn from the whole statement, so
/// one wrong pair can't be hidden: (Σ wᵢ·outputᵢ, Σ wᵢ·inputᵢ).
fn fold(
    key: &PublicKey,
    pairs: &[(Masked, Masked)],
    context: &[u8],
) -> (RistrettoPoint, RistrettoPoint) {
    let mut h = Sha512::new();
    h.update(UNLOCK_DOMAIN);
    h.update(b"/weights");
    h.update((context.len() as u64).to_be_bytes());
    h.update(context);
    h.update(key.to_bytes());
    for (i, o) in pairs {
        h.update(i.to_bytes());
        h.update(o.to_bytes());
    }
    let seed = h.finalize();
    let mut a = RistrettoPoint::default();
    let mut c = RistrettoPoint::default();
    for (n, (input, output)) in pairs.iter().enumerate() {
        let w = Scalar::hash_from_bytes::<Sha512>(&[&seed[..], &(n as u64).to_be_bytes()].concat());
        a += output.0 * w;
        c += input.0 * w;
    }
    (a, c)
}

fn unlock_challenge(
    key: &PublicKey,
    a: &RistrettoPoint,
    c: &RistrettoPoint,
    t1: &RistrettoPoint,
    t2: &RistrettoPoint,
    context: &[u8],
) -> Scalar {
    let mut h = Sha512::new();
    h.update(UNLOCK_DOMAIN);
    h.update((context.len() as u64).to_be_bytes());
    h.update(context);
    for p in [&key.0, a, c, t1, t2] {
        h.update(p.compress().as_bytes());
    }
    Scalar::from_hash(h)
}

impl UnlockProof {
    /// Proves that `pairs` are `(input, secret.unlock(input))`. Use the same
    /// `context` the verifier will.
    pub fn prove<R: Rng + ?Sized>(
        secret: &Secret,
        pairs: &[(Masked, Masked)],
        context: &[u8],
        rng: &mut R,
    ) -> Self {
        let key = secret.public();
        // input = x·output for each pair, so C = x·A for the folded pair.
        let (a, c) = fold(&key, pairs, context);
        let k = Secret::from_rng(rng).0;
        let (t1, t2) = (B * k, a * k);
        let ch = unlock_challenge(&key, &a, &c, &t1, &t2, context);
        UnlockProof {
            c: ch,
            s: k + ch * secret.0,
        }
    }

    /// Whether every pair is an input and its correct unlock by `key`'s secret.
    pub fn verify(&self, key: &PublicKey, pairs: &[(Masked, Masked)], context: &[u8]) -> bool {
        let (a, c) = fold(key, pairs, context);
        let t1 = B * self.s - key.0 * self.c;
        let t2 = a * self.s - c * self.c;
        unlock_challenge(key, &a, &c, &t1, &t2, context) == self.c
    }

    /// 64 bytes to send.
    pub fn to_bytes(&self) -> [u8; 64] {
        let mut out = [0; 64];
        out[..32].copy_from_slice(self.c.as_bytes());
        out[32..].copy_from_slice(self.s.as_bytes());
        out
    }

    /// A received proof; `None` if it isn't one.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != 64 {
            return None;
        }
        Some(UnlockProof {
            c: scalar(&bytes[..32])?,
            s: scalar(&bytes[32..])?,
        })
    }
}

// ---- shuffles: cut and choose ----

/// One shadow shuffle's opening: how the shadow was made from the input
/// (`Left`), or how the output is made from the shadow (`Right`).
#[derive(Clone, Debug, PartialEq, Eq)]
enum Opening {
    /// shadow = input permuted by `perm` and locked with `y`; Y = y·B.
    Left { y: Scalar, perm: Vec<u32> },
    /// output = shadow permuted by `perm` and locked with `z`, where
    /// z·Y = the party's public key.
    Right {
        big_y: RistrettoPoint,
        shadow: Vec<Masked>,
        z: Scalar,
        perm: Vec<u32>,
    },
}

/// A proof that a shuffle round's output is its input permuted and locked
/// with the secret behind the party's public key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShuffleProof {
    commitments: Vec<[u8; 64]>,
    openings: Vec<Opening>,
}

fn commit(j: usize, big_y: &RistrettoPoint, shadow: &[Masked]) -> [u8; 64] {
    let mut h = Sha512::new();
    h.update(SHUFFLE_DOMAIN);
    h.update(b"/shadow");
    h.update((j as u64).to_be_bytes());
    h.update(big_y.compress().as_bytes());
    for m in shadow {
        h.update(m.to_bytes());
    }
    h.finalize().into()
}

/// The challenge: one bit per round, from everything the verifier knows.
fn shuffle_challenge(
    key: &PublicKey,
    input: &[Masked],
    output: &[Masked],
    commitments: &[[u8; 64]],
    context: &[u8],
) -> Vec<bool> {
    let mut h = Sha512::new();
    h.update(SHUFFLE_DOMAIN);
    h.update((context.len() as u64).to_be_bytes());
    h.update(context);
    h.update(key.to_bytes());
    h.update((input.len() as u64).to_be_bytes());
    for m in input.iter().chain(output) {
        h.update(m.to_bytes());
    }
    for c in commitments {
        h.update(c);
    }
    let d = h.finalize();
    (0..commitments.len())
        .map(|j| d[j / 8] >> (j % 8) & 1 == 1)
        .collect()
}

fn inverse(perm: &[u32]) -> Vec<u32> {
    let mut inv = vec![0; perm.len()];
    for (i, &p) in perm.iter().enumerate() {
        inv[p as usize] = i as u32;
    }
    inv
}

impl ShuffleProof {
    /// Proves that `output` is `input` permuted by `perm` (as
    /// [`shuffle_round`](crate::shuffle_round) returns it: `output[i]` is
    /// `input[perm[i]]`, locked) with `secret`.
    pub fn prove<R: Rng + ?Sized>(
        secret: &Secret,
        perm: &[u32],
        input: &[Masked],
        output: &[Masked],
        context: &[u8],
        rng: &mut R,
    ) -> Self {
        let n = input.len();
        let mut shadows = Vec::with_capacity(SHUFFLE_ROUNDS);
        let mut commitments = Vec::with_capacity(SHUFFLE_ROUNDS);
        for j in 0..SHUFFLE_ROUNDS {
            let y = Secret::from_rng(rng);
            let mut pj: Vec<u32> = (0..n as u32).collect();
            for i in 0..n {
                let k = rng.random_range(i..n);
                pj.swap(i, k);
            }
            let shadow = apply_round(input, &y, &pj).expect("a permutation");
            let big_y = B * y.0;
            commitments.push(commit(j, &big_y, &shadow));
            shadows.push((y, pj, big_y, shadow));
        }
        let bits = shuffle_challenge(&secret.public(), input, output, &commitments, context);
        let openings = shadows
            .into_iter()
            .zip(bits)
            .map(|((y, pj, big_y, shadow), right)| {
                if !right {
                    return Opening::Left { y: y.0, perm: pj };
                }
                // output[i] = input[perm[i]]·x and shadow[k] = input[pj[k]]·y,
                // so output[i] = shadow[σ[i]]·(x/y) with σ = pj⁻¹ ∘ perm.
                let inv = inverse(&pj);
                let sigma = perm.iter().map(|&p| inv[p as usize]).collect();
                Opening::Right {
                    big_y,
                    shadow,
                    z: secret.0 * y.0.invert(),
                    perm: sigma,
                }
            })
            .collect();
        ShuffleProof {
            commitments,
            openings,
        }
    }

    /// Whether `output` is `input`, permuted and locked with the secret behind
    /// `key`.
    pub fn verify(
        &self,
        key: &PublicKey,
        input: &[Masked],
        output: &[Masked],
        context: &[u8],
    ) -> bool {
        if self.commitments.len() != SHUFFLE_ROUNDS
            || self.openings.len() != SHUFFLE_ROUNDS
            || input.len() != output.len()
        {
            return false;
        }
        let bits = shuffle_challenge(key, input, output, &self.commitments, context);
        self.openings
            .iter()
            .zip(&bits)
            .enumerate()
            .all(|(j, (o, &right))| match (o, right) {
                (Opening::Left { y, perm }, false) => {
                    *y != Scalar::ZERO
                        && apply_round(input, &Secret(*y), perm).is_some_and(|shadow| {
                            commit(j, &(B * y), &shadow) == self.commitments[j]
                        })
                }
                (
                    Opening::Right {
                        big_y,
                        shadow,
                        z,
                        perm,
                    },
                    true,
                ) => {
                    *z != Scalar::ZERO
                        && commit(j, big_y, shadow) == self.commitments[j]
                        && big_y * z == key.0
                        && apply_round(shadow, &Secret(*z), perm).is_some_and(|o| o == output)
                }
                _ => false,
            })
    }

    /// The proof as bytes: the commitments, then each opening.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = vec![self.commitments.len() as u8];
        for c in &self.commitments {
            out.extend_from_slice(c);
        }
        let perm_bytes = |out: &mut Vec<u8>, perm: &[u32]| {
            out.extend_from_slice(&(perm.len() as u16).to_be_bytes());
            for &p in perm {
                out.extend_from_slice(&(p as u16).to_be_bytes());
            }
        };
        for o in &self.openings {
            match o {
                Opening::Left { y, perm } => {
                    out.push(0);
                    out.extend_from_slice(y.as_bytes());
                    perm_bytes(&mut out, perm);
                }
                Opening::Right {
                    big_y,
                    shadow,
                    z,
                    perm,
                } => {
                    out.push(1);
                    out.extend_from_slice(big_y.compress().as_bytes());
                    out.extend_from_slice(z.as_bytes());
                    perm_bytes(&mut out, perm);
                    for m in shadow {
                        out.extend_from_slice(&m.to_bytes());
                    }
                }
            }
        }
        out
    }

    /// A received proof; `None` if it isn't one.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let mut r = Reader(bytes);
        let k = r.take(1)?[0] as usize;
        let commitments = (0..k)
            .map(|_| r.take(64).map(|b| b.try_into().unwrap()))
            .collect::<Option<Vec<[u8; 64]>>>()?;
        let mut openings = Vec::with_capacity(k);
        for _ in 0..k {
            let tag = r.take(1)?[0];
            let read_perm = |r: &mut Reader| -> Option<Vec<u32>> {
                let n = u16::from_be_bytes(r.take(2)?.try_into().ok()?) as usize;
                (0..n)
                    .map(|_| r.take(2).map(|b| u16::from_be_bytes([b[0], b[1]]) as u32))
                    .collect()
            };
            openings.push(match tag {
                0 => {
                    let y = scalar(r.take(32)?)?;
                    Opening::Left {
                        y,
                        perm: read_perm(&mut r)?,
                    }
                }
                1 => {
                    let big_y = point(r.take(32)?)?;
                    let z = scalar(r.take(32)?)?;
                    let perm = read_perm(&mut r)?;
                    let shadow = (0..perm.len())
                        .map(|_| {
                            r.take(32)
                                .and_then(|b| Masked::from_bytes(b.try_into().ok()?))
                        })
                        .collect::<Option<Vec<_>>>()?;
                    Opening::Right {
                        big_y,
                        shadow,
                        z,
                        perm,
                    }
                }
                _ => return None,
            });
        }
        r.0.is_empty().then_some(ShuffleProof {
            commitments,
            openings,
        })
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Some(a)
    }
}
