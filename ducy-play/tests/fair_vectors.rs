//! Fixed outputs for the fair deal: commitments, the hand seed and the deck
//! a seed shuffles to. Players' pages check these values, so a dependency
//! upgrade must leave every byte the same.

use ducy_play::fair_deal::{Reveal, hand_seed, shuffled_deck};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn a_commitment_is_fixed() {
    let r = Reveal {
        value: [1; 32],
        nonce: [2; 32],
    };
    assert_eq!(
        hex(&r.commitment()),
        "b233b6ef9d18d2e02b4145af0c567adc49800e317d2226cb709687dec19522f4"
    );
}

#[test]
fn a_seed_and_its_deck_are_fixed() {
    let seed = hand_seed(&[[3; 32], [4; 32], [5; 32]]);
    assert_eq!(
        hex(&seed),
        "7cb87241f95346f852da1d080783d0fbcf13c6fe302320ac929483a7fca78c99"
    );
    let deck: Vec<String> = shuffled_deck(&seed).iter().map(|c| c.to_string()).collect();
    assert_eq!(
        deck.join(" "),
        "Qh 3c 2h 2c 9d Ad 5s Js Qs Kh 7s 5d 7h Td Kc Ts 9s As 6c 6s 4d Jh 8c 8s Ah Ks Tc 6d 8h 2s Ac 5h 9h Kd 5c Jd 3d Qc 9c 8d Qd Jc Th 2d 7d 6h 3s 4s 7c 3h 4h 4c"
    );
}
