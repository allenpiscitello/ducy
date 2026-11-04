//! Fixed bytes for the wire format: the card points, keys, a shuffle round,
//! and both proofs from a seeded RNG. A dependency upgrade (the curve, the
//! hash) must leave every byte the same, or decks and proofs from older
//! clients stop verifying.

use ducy_shuffle::{Masked, Secret, ShuffleProof, UnlockProof, open_deck, shuffle_round};
use rand::{SeedableRng, rngs::StdRng};
use sha2::{Digest, Sha512};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Everything a hand sends, in order.
fn transcript() -> Vec<u8> {
    let mut rng = StdRng::seed_from_u64(338);
    let mut out = Vec::new();
    let deck = open_deck();
    for m in &deck {
        out.extend(m.to_bytes());
    }
    let secret = Secret::from_rng(&mut rng);
    out.extend(secret.to_bytes());
    out.extend(secret.public().to_bytes());
    let (shuffled, perm) = shuffle_round(&deck, &secret, &mut rng);
    for m in &shuffled {
        out.extend(m.to_bytes());
    }
    out.extend(
        ShuffleProof::prove(&secret, &perm, &deck, &shuffled, b"vectors", &mut rng).to_bytes(),
    );
    let pairs: Vec<(Masked, Masked)> = shuffled[..5]
        .iter()
        .map(|m| (*m, secret.unlock(m)))
        .collect();
    out.extend(UnlockProof::prove(&secret, &pairs, b"vectors", &mut rng).to_bytes());
    out
}

#[test]
fn the_first_card_point_is_fixed() {
    assert_eq!(
        hex(&open_deck()[0].to_bytes()),
        "34512cb9af6679150ff32dbd07ed9092b1e24321f6d86628405c0330badfb419"
    );
}

#[test]
fn a_hand_sends_the_same_bytes() {
    assert_eq!(
        hex(&Sha512::digest(transcript())),
        "feb0501409d6b5628b41d74858141b30a4327a3134eab5ff3b2ebd1585c7f532dada26a831d6ddca6826b2cdd7490a7fbace04f58b9230cc0dd0b9576bfee45b"
    );
}
