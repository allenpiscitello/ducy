use std::collections::HashSet;

use ducy_gto::{
    Rng,
    holdem::{
        cards::{Card, NUM_HOLES, card, hole_cards, hole_index, parse, rank, suit},
        equity::{equity, river_equities, river_equity},
        iso::{
            NUM_PREFLOP_CLASSES, apply, canonical, canonical_board, canonical_with_perm,
            preflop_class,
        },
        kmeans::{Distance, distance, kmeans, nearest},
    },
};

fn cards(s: &str) -> Vec<Card> {
    parse(s).unwrap()
}

/// Random distinct cards.
fn deal(rng: &mut Rng, n: usize) -> Vec<Card> {
    let mut out = Vec::new();
    while out.len() < n {
        let c = (rng.next_u64() % 52) as Card;
        if !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

#[test]
fn card_encoding_round_trips() {
    for c in 0..52u8 {
        assert_eq!(card(rank(c), suit(c)), c);
    }
    for h in 0..NUM_HOLES {
        let (a, b) = hole_cards(h);
        assert!(a < b);
        assert_eq!(hole_index(a, b), h);
        assert_eq!(hole_index(b, a), h);
    }
    assert_eq!(cards("As Kd"), vec![card(12, 3), card(11, 1)]);
}

#[test]
fn isomorphic_hands_share_a_key() {
    let mut rng = Rng::new(5);
    let perms: Vec<[u8; 4]> = {
        let mut v = Vec::new();
        for a in 0..4i32 {
            for b in 0..4i32 {
                for c in 0..4i32 {
                    let d = 6 - a - b - c;
                    if [a, b, c].iter().collect::<HashSet<_>>().len() == 3
                        && (0..4).contains(&d)
                        && ![a, b, c].contains(&d)
                    {
                        v.push([a as u8, b as u8, c as u8, d as u8]);
                    }
                }
            }
        }
        v
    };
    assert_eq!(perms.len(), 24);
    for n in [2, 5, 6, 7] {
        for _ in 0..500 {
            let hand = deal(&mut rng, n);
            let key = canonical(&hand);
            for p in &perms {
                let mut moved: Vec<Card> = hand.iter().map(|&c| apply(c, p)).collect();
                // Order within a round doesn't matter either.
                moved[..2].reverse();
                if n >= 5 {
                    moved[2..5].reverse();
                }
                assert_eq!(canonical(&moved), key, "{hand:?} vs {moved:?}");
            }
            // The returned relabeling really produces the canonical form.
            let (k, p) = canonical_with_perm(&hand);
            let moved: Vec<Card> = hand.iter().map(|&c| apply(c, &p)).collect();
            assert_eq!(canonical(&moved), k);
        }
    }
}

#[test]
fn different_hands_get_different_keys() {
    // Same ranks, but suited vs offsuit; and a board card swapped into the hand.
    assert_ne!(canonical(&cards("As Ks")), canonical(&cards("As Kd")));
    assert_ne!(
        canonical(&cards("As Ks 2c 3c 4c")),
        canonical(&cards("2c 3c As Ks 4c"))
    );
    // The flop is unordered but the turn is a separate round.
    assert_eq!(
        canonical(&cards("As Kd 2c 3h 4s 5d")),
        canonical(&cards("As Kd 4s 3h 2c 5d"))
    );
    assert_ne!(
        canonical(&cards("As Kd 2c 3h 4s 5d")),
        canonical(&cards("As Kd 2c 3h 5d 4s"))
    );
}

#[test]
fn distinct_counts() {
    // 169 starting hands, 1,755 flops: the known counts.
    let mut pre = HashSet::new();
    let mut classes = HashSet::new();
    for h in 0..NUM_HOLES {
        let (a, b) = hole_cards(h);
        pre.insert(canonical(&[a, b]));
        classes.insert(preflop_class(a, b));
    }
    assert_eq!(pre.len(), NUM_PREFLOP_CLASSES);
    assert_eq!(classes.len(), NUM_PREFLOP_CLASSES);
    assert!(classes.iter().all(|&c| c < NUM_PREFLOP_CLASSES));
    let mut flops = HashSet::new();
    for a in 0..52u8 {
        for b in a + 1..52 {
            for c in b + 1..52 {
                flops.insert(canonical_board(&[a, b, c]).0);
            }
        }
    }
    assert_eq!(flops.len(), 1755);
}

#[test]
fn river_equities_match_one_hand_at_a_time() {
    let mut rng = Rng::new(9);
    let mut eq = [0.0f32; NUM_HOLES];
    for _ in 0..3 {
        let board: [Card; 5] = deal(&mut rng, 5).try_into().unwrap();
        river_equities(&board, &mut eq);
        let mut checked = 0;
        for h in (0..NUM_HOLES).step_by(7) {
            let (a, b) = hole_cards(h);
            if board.contains(&a) || board.contains(&b) {
                assert_eq!(eq[h], -1.0);
                continue;
            }
            assert!((eq[h] - river_equity([a, b], &board)).abs() < 1e-6);
            checked += 1;
        }
        assert!(checked > 100);
    }
}

#[test]
fn equity_sanity() {
    let board: [Card; 5] = cards("Ah Kh Qh 2c 7d").try_into().unwrap();
    // The royal flush can't lose; the nut flush only loses to it and a few.
    assert_eq!(river_equity([card(8, 2), card(9, 2)], &board), 1.0); // Th Jh
    let j_high = river_equity([card(0, 0), card(1, 1)], &board); // 2c 3d: pair of twos
    assert!(j_high < 0.4);
    // On the flop, a set beats a flush draw beats nothing, on average.
    let flop = cards("9s 9d 4h");
    let set = equity([card(7, 0), card(7, 2)], &flop);
    let draw = equity([card(12, 2), card(10, 2)], &flop);
    let air = equity([card(0, 0), card(5, 1)], &flop);
    assert!(set > 0.9 && set > draw && draw > air, "{set} {draw} {air}");
}

#[test]
fn kmeans_finds_obvious_clusters() {
    // Histograms: mass at the low end, middle, or high end.
    let mut points = Vec::new();
    for i in 0..300 {
        let mut h = [0.0f32; 10];
        let at = [1, 5, 8][i % 3];
        h[at] = 0.9;
        h[(at + 1) % 10] = 0.1;
        points.extend(h);
    }
    let c = kmeans(&points, 10, 3, 20, 1, Distance::Emd);
    assert_eq!(c.len(), 30);
    let groups: HashSet<usize> = (0..3)
        .map(|i| nearest(&points[i * 10..(i + 1) * 10], &c, 10, Distance::Emd))
        .collect();
    assert_eq!(groups.len(), 3);
    // EMD: moving all mass one bin costs 1, two bins 2.
    let (mut a, mut b, mut d) = ([0.0f32; 4], [0.0f32; 4], [0.0f32; 4]);
    a[0] = 1.0;
    b[1] = 1.0;
    d[2] = 1.0;
    assert_eq!(distance(&a, &b, Distance::Emd), 1.0);
    assert_eq!(distance(&a, &d, Distance::Emd), 2.0);
    assert_eq!(distance(&a, &d, Distance::L2), 2.0);
}
