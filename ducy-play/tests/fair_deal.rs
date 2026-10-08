//! Provably fair deals (#141): commit–reveal, the seed, the shuffle, and
//! checking a deal afterwards.

use ducy::deck::{Card, Deck};
use ducy_play::fair_deal::{FairError, FairRound, Reveal, check_deal, hand_seed, shuffled_deck};
use ducy_play::{Deal, Table, TableRules, TableSeat, Variant};
use std::time::Instant;

fn reveal(byte: u8) -> Reveal {
    Reveal {
        value: [byte; 32],
        nonce: [byte ^ 0xff; 32],
    }
}

#[test]
fn a_full_round_gives_a_deal_anyone_can_check() {
    let secrets: Vec<Reveal> = (1..=4).map(reveal).collect();
    let mut round = FairRound::new(4);
    for (i, r) in secrets.iter().enumerate() {
        assert!(!round.all_committed());
        round.commit(i, r.commitment()).unwrap();
    }
    assert!(round.all_committed());
    for (i, r) in secrets.iter().enumerate() {
        round.reveal(i, *r).unwrap();
    }
    assert!(round.complete());
    let out = round.outcome();
    assert_eq!(out.used, vec![0, 1, 2, 3]);
    assert!(out.sat_out.is_empty());
    assert_eq!(
        out.seed,
        hand_seed(&secrets.iter().map(|r| r.value).collect::<Vec<_>>())
    );

    let deal = Deal::from_seed(Variant::Holdem, 3, &out.seed).unwrap();
    assert!(check_deal(Variant::Holdem, 3, &out.reveals, &deal));
    // Any other deal fails the check, and so does any changed reveal.
    let other = Deal::from_seed(Variant::Holdem, 3, &[9; 32]).unwrap();
    assert!(!check_deal(Variant::Holdem, 3, &out.reveals, &other));
    let mut forged = out.reveals.clone();
    forged[1].value[0] ^= 1;
    assert!(!check_deal(Variant::Holdem, 3, &forged, &deal));
}

#[test]
fn a_reveal_that_does_not_match_sits_that_player_out() {
    let secrets: Vec<Reveal> = (1..=3).map(reveal).collect();
    let mut round = FairRound::new(3);
    for (i, r) in secrets.iter().enumerate() {
        round.commit(i, r.commitment()).unwrap();
    }
    round.reveal(0, secrets[0]).unwrap();
    // Player 1 tries a different value after seeing player 0's.
    assert_eq!(round.reveal(1, reveal(99)), Err(FairError::Mismatch));
    assert_eq!(
        round.reveal(1, secrets[1]),
        Err(FairError::AlreadyRevealed),
        "no second try"
    );
    round.reveal(2, secrets[2]).unwrap();
    assert!(round.complete());
    let out = round.outcome();
    assert_eq!(out.used, vec![0, 2]);
    assert_eq!(out.sat_out, vec![1]);
    assert_eq!(out.seed, hand_seed(&[secrets[0].value, secrets[2].value]));
}

#[test]
fn someone_who_never_reveals_sits_out_when_time_is_up() {
    let secrets: Vec<Reveal> = (1..=3).map(reveal).collect();
    let mut round = FairRound::new(3);
    for (i, r) in secrets.iter().enumerate() {
        round.commit(i, r.commitment()).unwrap();
    }
    round.reveal(0, secrets[0]).unwrap();
    round.reveal(2, secrets[2]).unwrap();
    assert!(!round.complete());
    let out = round.outcome();
    assert_eq!(out.sat_out, vec![1]);
}

#[test]
fn commitments_close_once_reveals_start() {
    let mut round = FairRound::new(2);
    assert_eq!(round.reveal(0, reveal(1)), Err(FairError::NotCommitted));
    round.commit(0, reveal(1).commitment()).unwrap();
    assert_eq!(
        round.commit(0, reveal(2).commitment()),
        Err(FairError::AlreadyCommitted)
    );
    assert_eq!(
        round.commit(5, reveal(2).commitment()),
        Err(FairError::UnknownParticipant)
    );
    round.commit(1, reveal(2).commitment()).unwrap();
    round.reveal(0, reveal(1)).unwrap();
    // A late commitment can't sneak in after seeing a reveal.
    let mut late = FairRound::new(3);
    late.commit(0, reveal(1).commitment()).unwrap();
    late.commit(1, reveal(2).commitment()).unwrap();
    late.reveal(0, reveal(1)).unwrap();
    assert_eq!(
        late.commit(2, reveal(3).commitment()),
        Err(FairError::AlreadyCommitted)
    );
    // 1 committed but never revealed, 2 never committed: both sit out.
    assert_eq!(late.outcome().sat_out, vec![1, 2]);
}

#[test]
fn the_specification_is_pinned() {
    // These change only if the published procedure changes: a client in
    // another language reproduces the same numbers from the module docs.
    let deck: Vec<Card> = Deck::all_cards().iter(false).collect();
    assert_eq!(deck.len(), 52);
    let commit = reveal(1).commitment();
    let seed = hand_seed(&[[1; 32], [2; 32]]);
    let order = shuffled_deck(&seed);
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    // Every card exactly once.
    let mut seen = Deck::empty();
    for c in &order {
        assert!(!seen.has_card(c));
        seen |= *c;
    }
    assert_eq!(seen, Deck::all_cards());
    // Checked against an independent implementation of the module docs;
    // the shuffle as positions in ducy's standard order.
    let positions: Vec<String> = order
        .iter()
        .map(|c| deck.iter().position(|d| d == c).unwrap().to_string())
        .collect();
    insta(&format!(
        "{} {} {}",
        hex(&commit),
        hex(&seed),
        positions.join(",")
    ));
}

/// The pinned values, printed on failure so they can be updated knowingly.
fn insta(got: &str) {
    const PINNED: &str = include_str!("fair_deal_pinned.txt");
    assert_eq!(
        got,
        PINNED.trim(),
        "the fair-deal procedure changed; if that's intended, update tests/fair_deal_pinned.txt"
    );
}

#[test]
fn shuffles_are_uniform_enough() {
    // Where the ace of spades lands over many seeds: roughly even.
    let target = Deck::all_cards().iter(false).last().unwrap();
    let mut counts = [0u32; 52];
    let runs = 5_200u32;
    for n in 0..runs {
        let mut seed = [0u8; 32];
        seed[..4].copy_from_slice(&n.to_be_bytes());
        let pos = shuffled_deck(&seed)
            .iter()
            .position(|c| *c == target)
            .unwrap();
        counts[pos] += 1;
    }
    let expected = f64::from(runs) / 52.0;
    let chi2: f64 = counts
        .iter()
        .map(|&c| (f64::from(c) - expected).powi(2) / expected)
        .sum();
    // 51 degrees of freedom: the 99.9th percentile is about 87.
    assert!(chi2 < 87.0, "chi-square {chi2:.1}: {counts:?}");
}

#[test]
fn a_table_deals_the_agreed_seed_to_its_seats_in_order() {
    let seats = ["A", "B", "C", "D"]
        .iter()
        .map(|n| TableSeat::human(*n, n.to_lowercase()))
        .collect();
    let mut t = Table::new(TableRules::no_limit_holdem(1, 2), seats, 200, 1).unwrap();
    t.seat_mut(2).sitting_out = true; // didn't reveal
    let seed = hand_seed(&[[7; 32], [8; 32], [9; 32]]);
    t.new_hand_from_seed(&seed).unwrap();
    assert_eq!(t.dealt(), &[0, 1, 3]);
    let deal = Deal::from_seed(Variant::Holdem, 3, &seed).unwrap();
    let sorted = |mut v: Vec<String>| {
        v.sort();
        v
    };
    for (i, &s) in t.dealt().iter().enumerate() {
        let shown = t.view(s).seats[0].cards.clone().expect("own cards");
        let dealt: Vec<String> = deal.hole_cards()[i]
            .iter(false)
            .map(|c| c.to_string())
            .collect();
        assert_eq!(sorted(shown), sorted(dealt), "seat {s}");
    }
}

#[test]
fn a_nine_player_round_is_quick() {
    let start = Instant::now();
    let secrets: Vec<Reveal> = (0..10).map(|_| Reveal::random()).collect();
    let mut round = FairRound::new(10);
    for (i, r) in secrets.iter().enumerate() {
        round.commit(i, r.commitment()).unwrap();
    }
    for (i, r) in secrets.iter().enumerate() {
        round.reveal(i, *r).unwrap();
    }
    let out = round.outcome();
    let deal = Deal::from_seed(Variant::Holdem, 9, &out.seed).unwrap();
    assert!(check_deal(Variant::Holdem, 9, &out.reveals, &deal));
    // Network round trips aside, the work is a few hundred hashes.
    assert!(start.elapsed().as_millis() < 200, "{:?}", start.elapsed());
}
