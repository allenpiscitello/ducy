use ducy_gto::{
    Rng,
    holdem::{
        cards::{Card, parse, rank},
        hunl::Buckets,
    },
    omaha::abstraction::{
        BoardView, PloAbstraction, PloAbstractionConfig, PreflopClasses, canonical_key, random_spot,
    },
};

fn p(s: &str) -> Vec<Card> {
    parse(s).unwrap()
}

/// A small, quick build for tests: one bucket per preflop class (no
/// preflop equity), few fitting hands.
fn small() -> PloAbstraction {
    PloAbstraction::build(
        PloAbstractionConfig {
            preflop: 0,
            flop: 30,
            turn: 30,
            river: 30,
            fit_hands: 1500,
            equity_samples: 150,
            ..PloAbstractionConfig::default()
        },
        |_| {},
    )
}

#[test]
fn preflop_classes_follow_suit_isomorphism() {
    let classes = PreflopClasses::new(4);
    assert_eq!(classes.len(), 16_432, "the known number of PLO4 classes");
    // The same hand with its suits swapped is the same class; different
    // suit structure is not.
    let a = p("As Ad Ks Kd");
    let b = p("Ah Ac Kh Kc");
    let c = p("As Ah Ks Kd");
    assert_eq!(classes.class(&a), classes.class(&b));
    assert_eq!(canonical_key(&a), canonical_key(&b));
    assert_ne!(classes.class(&a), classes.class(&c));
    for i in [0, 100, 16_431] {
        assert_eq!(classes.class(classes.representative(i)), i);
    }
}

/// Brute force: two hole ranks and three board ranks forming five
/// consecutive ranks (ace high or low).
fn straight_brute(hole: &[Card], board: &[Card]) -> bool {
    let r = |c: Card| rank(c) as i32 + 2;
    for i in 0..hole.len() {
        for j in i + 1..hole.len() {
            for a in 0..board.len() {
                for b in a + 1..board.len() {
                    for c in b + 1..board.len() {
                        let five = [hole[i], hole[j], board[a], board[b], board[c]];
                        for ace_low in [false, true] {
                            let mut v: Vec<i32> = five
                                .iter()
                                .map(|&x| if ace_low && r(x) == 14 { 1 } else { r(x) })
                                .collect();
                            v.sort_unstable();
                            if v.windows(2).all(|w| w[1] == w[0] + 1) {
                                return true;
                            }
                        }
                    }
                }
            }
        }
    }
    false
}

#[test]
fn straight_outs_count_cards_that_make_a_straight() {
    // A wrap on 8-7-2: 9, T, 6, 5 and J make straights (two from hand, three
    // from the board).
    let view = BoardView::new(&p("8c 7d 2h"));
    let wrap = view.features(&p("Ts 9s 6h 5h"));
    let air = view.features(&p("Ks Qd 3c 3d"));
    assert!(wrap[6] > 0.25, "a 20-out wrap: {}", wrap[6]);
    assert_eq!(air[6], 0.0);
    // Against brute force on random flops and turns.
    let mut rng = Rng::new(3);
    let mut seen = 0;
    for _ in 0..300 {
        let (hole, board) = random_spot(4, 3 + (seen % 2), &mut rng);
        let view = BoardView::new(&board);
        let outs = view.features(&hole)[6];
        let now = straight_brute(&hole, &board);
        let mut expected = 0;
        let mut unseen = 0;
        for c in 0..52 as Card {
            if hole.contains(&c) || board.contains(&c) {
                continue;
            }
            unseen += 1;
            let mut next = board.clone();
            next.push(c);
            if !now && straight_brute(&hole, &next) {
                expected += 1;
            }
        }
        assert!(
            (outs - expected as f64 / unseen as f64).abs() < 1e-9,
            "{hole:?} on {board:?}"
        );
        seen += 1;
    }
}

#[test]
fn buckets_are_consistent_saved_and_sensible() {
    let cards = small();
    // Training's per-deal buckets equal play's per-hand ones.
    let mut rng = Rng::new(11);
    for _ in 0..40 {
        let (h, b) = random_spot(8, 5, &mut rng);
        let board = [b[0], b[1], b[2], b[3], b[4]];
        let deal = cards.deal_buckets([&h[..4], &h[4..]], &board);
        for (player, hole) in [&h[..4], &h[4..]].into_iter().enumerate() {
            for (street, n) in [0usize, 3, 4, 5].into_iter().enumerate() {
                let b = cards.hand_bucket(hole, &board[..n]);
                assert_eq!(deal[player][street], b);
                assert!((b as usize) < cards.bucket_count(n));
            }
        }
    }
    // Save and load give the same abstraction.
    let bytes = cards.save();
    let loaded = PloAbstraction::load(&bytes).expect("loads");
    assert_eq!(loaded.save(), bytes);
    assert!(PloAbstraction::load(&bytes[..bytes.len() - 1]).is_none());
    assert_eq!(cards.bucket_count(0), 16_432);

    // The river nuts are in the top bucket, a busted hand near the bottom.
    let board = p("Ah Kh Qh 7c 2d");
    let top = (cards.bucket_count(5) - 1) as u16;
    assert_eq!(cards.hand_bucket(&p("Jh Th 3s 4s"), &board), top);
    assert!(cards.hand_bucket(&p("6s 5c 4d 3s"), &board) < 5);
    // On the flop a set, a big wrap and air predict different equities, in
    // that order, and land in different buckets.
    let flop = p("8c 7d 2h");
    let view = BoardView::new(&flop);
    let set = p("8s 8h Kc 3d");
    let wrap = p("Ts 9s 6h 5h");
    let air = p("Ks Qd 3c 3s");
    let e = |h: &[Card]| cards.predicted_equity(h, &view);
    assert!(
        e(&set) > e(&wrap) && e(&wrap) > e(&air),
        "{} {} {}",
        e(&set),
        e(&wrap),
        e(&air)
    );
    let b = |h: &[Card]| cards.hand_bucket(h, &flop);
    assert!(b(&set) != b(&wrap) && b(&wrap) != b(&air));
}

#[test]
fn river_equity_ignores_opponents_holding_our_cards() {
    // On a board where only a heart flush matters, holding the A♥ and K♥
    // is the nuts; every sampled opponent that also "held" one of them is
    // skipped, so the equity stays 1.
    let view = BoardView::new(&p("Qh Jh 9h 2c 3d"));
    assert_eq!(view.river_equity(&p("Ah Kh 4s 4c")), 1.0);
    // And the same hand's bucket never depends on which other hand shares
    // the board (the samples come from the board alone).
    let again = BoardView::new(&p("Qh Jh 9h 2c 3d"));
    assert_eq!(
        view.river_equity(&p("Ts 8s 4d 5d")),
        again.river_equity(&p("Ts 8s 4d 5d"))
    );
}
