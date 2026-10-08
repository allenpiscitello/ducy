//! The proofs (#140): honest steps verify, and every way of cheating the
//! audit catches (#138) is refused on the spot instead.

use ducy_shuffle::{
    PublicKey, Secret, ShuffleProof, UnlockProof, apply_round, card_point, open_deck, shuffle_round,
};
use rand::{SeedableRng, rngs::StdRng};

fn rng(seed: u64) -> StdRng {
    StdRng::seed_from_u64(seed)
}

const CTX: &[u8] = b"table T1/hand 7/party 0";

#[test]
fn an_honest_shuffle_verifies_and_survives_bytes() {
    let mut r = rng(1);
    let x = Secret::from_rng(&mut r);
    let input = open_deck();
    let (output, perm) = shuffle_round(&input, &x, &mut r);
    let proof = ShuffleProof::prove(&x, &perm, &input, &output, CTX, &mut r);
    assert!(proof.verify(&x.public(), &input, &output, CTX));
    let back = ShuffleProof::from_bytes(&proof.to_bytes()).expect("round trip");
    assert!(back.verify(&x.public(), &input, &output, CTX));
    // A second, already-locked round verifies too.
    let y = Secret::from_rng(&mut r);
    let (out2, perm2) = shuffle_round(&output, &y, &mut r);
    assert!(
        ShuffleProof::prove(&y, &perm2, &output, &out2, CTX, &mut r).verify(
            &y.public(),
            &output,
            &out2,
            CTX
        )
    );
}

#[test]
fn a_fake_shuffle_is_refused() {
    let mut r = rng(2);
    let x = Secret::from_rng(&mut r);
    let input = open_deck();
    let (mut output, perm) = shuffle_round(&input, &x, &mut r);
    // Swap in a card of the cheat's choosing (here: a copy of another one).
    output[3] = output[4];
    let proof = ShuffleProof::prove(&x, &perm, &input, &output, CTX, &mut r);
    assert!(!proof.verify(&x.public(), &input, &output, CTX));
    // A card that isn't in the deck at all.
    let (mut output, perm) = shuffle_round(&input, &x, &mut r);
    // (the ace of spades locked twice: no single lock of a deck card).
    output[0] = x.lock(&x.lock(&card_point(ducy::deck::Card::parse("As").unwrap())));
    assert!(
        !ShuffleProof::prove(&x, &perm, &input, &output, CTX, &mut r).verify(
            &x.public(),
            &input,
            &output,
            CTX
        )
    );
}

#[test]
fn a_shuffle_locked_with_another_key_is_refused() {
    let mut r = rng(3);
    let (x, other) = (Secret::from_rng(&mut r), Secret::from_rng(&mut r));
    let input = open_deck();
    let (output, perm) = shuffle_round(&input, &other, &mut r);
    // Proved with the other secret, checked against x's key: refused.
    let proof = ShuffleProof::prove(&other, &perm, &input, &output, CTX, &mut r);
    assert!(!proof.verify(&x.public(), &input, &output, CTX));
}

#[test]
fn a_proof_for_another_deck_or_context_is_refused() {
    let mut r = rng(4);
    let x = Secret::from_rng(&mut r);
    let input = open_deck();
    let (output, perm) = shuffle_round(&input, &x, &mut r);
    let proof = ShuffleProof::prove(&x, &perm, &input, &output, CTX, &mut r);
    assert!(!proof.verify(&x.public(), &input, &output, b"table T1/hand 8/party 0"));
    let (other, _) = shuffle_round(&input, &x, &mut r);
    assert!(!proof.verify(&x.public(), &input, &other, CTX));
    // Tampered bytes don't verify (or don't parse).
    let mut bytes = proof.to_bytes();
    let n = bytes.len();
    bytes[n - 5] ^= 1;
    assert!(ShuffleProof::from_bytes(&bytes).is_none_or(|p| !p.verify(
        &x.public(),
        &input,
        &output,
        CTX
    )));
}

#[test]
fn honest_unlocks_verify_in_one_proof() {
    let mut r = rng(5);
    let x = Secret::from_rng(&mut r);
    let deck = apply_round(&open_deck(), &x, &(0..52).collect::<Vec<_>>()).unwrap();
    let pairs: Vec<_> = deck[..9].iter().map(|c| (*c, x.unlock(c))).collect();
    let proof = UnlockProof::prove(&x, &pairs, CTX, &mut r);
    assert!(proof.verify(&x.public(), &pairs, CTX));
    assert!(
        UnlockProof::from_bytes(&proof.to_bytes())
            .unwrap()
            .verify(&x.public(), &pairs, CTX)
    );
}

#[test]
fn a_wrong_unlock_or_a_substituted_card_is_refused() {
    let mut r = rng(6);
    let (x, other) = (Secret::from_rng(&mut r), Secret::from_rng(&mut r));
    let deck = apply_round(&open_deck(), &x, &(0..52).collect::<Vec<_>>()).unwrap();
    // One card unlocked with another secret.
    let mut pairs: Vec<_> = deck[..5].iter().map(|c| (*c, x.unlock(c))).collect();
    pairs[2].1 = other.unlock(&pairs[2].0);
    assert!(!UnlockProof::prove(&x, &pairs, CTX, &mut r).verify(&x.public(), &pairs, CTX));
    // A card swapped for another as it's handed back.
    let mut pairs: Vec<_> = deck[..5].iter().map(|c| (*c, x.unlock(c))).collect();
    pairs[4].1 = x.unlock(&deck[20]);
    assert!(!UnlockProof::prove(&x, &pairs, CTX, &mut r).verify(&x.public(), &pairs, CTX));
    // An honest proof checked against another party's key.
    let pairs: Vec<_> = deck[..5].iter().map(|c| (*c, x.unlock(c))).collect();
    assert!(!UnlockProof::prove(&x, &pairs, CTX, &mut r).verify(&other.public(), &pairs, CTX));
}

#[test]
fn a_secret_shown_at_showdown_must_match_its_key() {
    let mut r = rng(7);
    let (x, other) = (Secret::from_rng(&mut r), Secret::from_rng(&mut r));
    let key = PublicKey::from_bytes(&x.public().to_bytes()).unwrap();
    assert!(key.matches(&x));
    assert!(!key.matches(&other));
}
