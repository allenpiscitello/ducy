use ducy_gto::{
    Rng,
    holdem::{
        cards::{Card, parse, rank},
        hunl::Buckets,
    },
    omaha::abstraction::{
        BoardView, NUM_NUT_FEATURES, PloAbstraction, PloAbstractionConfig, PreflopClasses,
        beaten_share, canonical_key, nut_outcomes, nutness, random_spot,
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

#[test]
fn six_cards_bucket_preflop_with_a_model() {
    let cards = PloAbstraction::build(
        PloAbstractionConfig {
            hole_cards: 6,
            preflop: 30,
            flop: 20,
            turn: 20,
            river: 20,
            fit_hands: 1500,
            equity_samples: 150,
            ..PloAbstractionConfig::default()
        },
        |_| {},
    );
    // Training's per-deal buckets equal play's per-hand ones, on every street.
    let mut rng = Rng::new(12);
    for _ in 0..30 {
        let (h, b) = random_spot(12, 5, &mut rng);
        let board = [b[0], b[1], b[2], b[3], b[4]];
        let deal = cards.deal_buckets([&h[..6], &h[6..]], &board);
        for (player, hole) in [&h[..6], &h[6..]].into_iter().enumerate() {
            for (street, n) in [0usize, 3, 4, 5].into_iter().enumerate() {
                let b = cards.hand_bucket(hole, &board[..n]);
                assert_eq!(deal[player][street], b);
                assert!((b as usize) < cards.bucket_count(n));
            }
        }
    }
    assert_eq!(cards.bucket_count(0), 30);
    // Save and load give the same abstraction; a truncated file doesn't load.
    let bytes = cards.save();
    let loaded = PloAbstraction::load(&bytes).expect("loads");
    assert_eq!(loaded.save(), bytes);
    assert_eq!(loaded.config.hole_cards, 6);
    assert!(PloAbstraction::load(&bytes[..bytes.len() - 1]).is_none());
    // Suit isomorphism holds without a class table: the features see ranks
    // and suit counts only.
    assert_eq!(
        cards.preflop_bucket(&p("As Ad Ks Kd Qs Jd")),
        cards.preflop_bucket(&p("Ah Ac Kh Kc Qh Jc"))
    );
    // A double-suited rundown predicts more equity than a rainbow pile of
    // low cards with trips.
    let model = cards.street_model(0).expect("a preflop model");
    let e = |h: &str| model.predict(&cards.preflop_features(&p(h)));
    assert!(
        e("As Ks Qd Jd Th 9h") > e("7c 7d 7h 2s 3c 4d") + 0.03,
        "{} {}",
        e("As Ks Qd Jd Th 9h"),
        e("7c 7d 7h 2s 3c 4d")
    );
    // River equity samples six-card opponents: on an unpaired board with no
    // straight flush, the ace-high flush is never beaten.
    let view = BoardView::for_hands(&p("Kh 8h 3h Qc 2d"), 6);
    assert_eq!(view.river_equity(&p("Ah 5h 4s 4c 9s 6c")), 1.0);
}

/// A small build that isn't nut-aware, as abstractions were before #204.
fn small_plain(hole_cards: usize) -> PloAbstraction {
    PloAbstraction::build(
        PloAbstractionConfig {
            hole_cards,
            preflop: if hole_cards == 4 { 0 } else { 20 },
            flop: 20,
            turn: 20,
            river: 20,
            fit_hands: 1000,
            equity_samples: 100,
            nut_aware: false,
            ..PloAbstractionConfig::default()
        },
        |_| {},
    )
}

#[test]
fn plain_abstractions_save_and_hash_as_before() {
    for k in [4, 6] {
        let cards = small_plain(k);
        let bytes = cards.save();
        // Versions 1 (four cards) and 2 (six), as before #204.
        assert_eq!(
            &bytes[..9],
            if k == 4 {
                b"DUCYPLOA\x01"
            } else {
                b"DUCYPLOA\x02"
            }
        );
        let loaded = PloAbstraction::load(&bytes).expect("loads");
        assert!(!loaded.config.nut_aware);
        assert_eq!(loaded.save(), bytes);
        // Published blueprints hash these settings: they mustn't change.
        assert_eq!(
            cards.settings(),
            format!(
                "PloAbstractionConfig {{ hole_cards: {k}, preflop: {}, flop: 20, turn: 20, \
                 river: 20, fit_hands: 1000, equity_samples: 100, seed: 1 }}|plo-features-v1",
                if k == 4 { 0 } else { 20 }
            )
        );
    }
}

#[test]
fn nut_aware_abstractions_save_as_version_3() {
    let cards = small();
    assert!(cards.config.nut_aware);
    let bytes = cards.save();
    assert_eq!(&bytes[..9], b"DUCYPLOA\x03");
    let loaded = PloAbstraction::load(&bytes).expect("loads");
    assert!(loaded.config.nut_aware);
    assert_eq!(loaded.save(), bytes);
    assert!(cards.settings().ends_with("|plo-features-v2"));
}

#[test]
fn nutness_counts_holdings_that_beat_the_hand() {
    // The nut flush beats every two-card holding: nutness 1.
    let board: [Card; 5] = p("Kh 8h 3h Qc 2d").try_into().unwrap();
    assert_eq!(beaten_share(&p("Ah 5h 4s 4c"), &board), 0.0);
    assert_eq!(nutness(0.0), 1.0);
    // A low flush is beaten by many hands holding two higher hearts.
    let low = beaten_share(&p("6h 5h 4s 4c"), &board);
    assert!(low > 0.01 && low < 0.1, "{low}");
    // Bottom pair is far from the nuts.
    assert_eq!(nutness(beaten_share(&p("2s 7c 9d Td"), &board)), 0.0);
    // Blockers count: holding the A♥ alone, fewer hands make the nut flush.
    let without = beaten_share(&p("Kd Ks 9c 9d"), &board);
    let with = beaten_share(&p("Kd Ks 9c Ah"), &board);
    assert!(with < without, "{with} {without}");
}

#[test]
fn nut_features_tell_nut_draws_from_weaker_ones() {
    let at = |board: &str, hole: &str| BoardView::exact(&p(board)).nut_features(&p(hole));
    // Indices past the plain features: 14 near the nuts, 15 the nuts, 16
    // nut straight outs, 17 second-nut flush draw, 18 backdoor nut flush
    // draw, 21 a lone nut-flush blocker, 22 set draws.
    let set = at("8c 7d 2h", "8s 8h Kc 3d");
    assert_eq!(set[15], 1.0, "top set on a dry flop is the nuts");
    // On 8-7-2 a J, T, 9, 6, 5 or 4 brings straights. J-T-9 holds the top
    // one after a J, T, 9 or 6; 6-5-4-3 never does.
    let high = at("8c 7d 2h", "Js Ts 9d 3c");
    let low = at("8c 7d 2h", "6s 5s 4d 3c");
    assert!(high[16] > low[16], "{} {}", high[16], low[16]);
    // Flush draws: K-high with the ace out is the second nuts.
    let second = at("Qh 7h 2c", "Kh 5h 9s 9d");
    let nut = at("Qh 7h 2c", "Ah 5h 9s 9d");
    assert_eq!(second[17], 1.0);
    assert_eq!(nut[17], 0.0);
    assert_eq!(nut[4], 1.0);
    // A backdoor nut flush draw on the flop.
    assert_eq!(at("Qh 7c 2d", "Ah 5h 9s 9d")[18], 1.0);
    // A lone ace on a three-heart board blocks the nut flush.
    assert_eq!(at("Qh 7h 2h", "Ah 5c 9s 9d")[21], 1.0);
    // Two pocket pairs to make sets with.
    assert!((at("Qh 7c 2d", "Ah 5h 9s 9d Ks Kc")[22] - 2.0 / 3.0).abs() < 1e-9);
}

#[test]
fn nut_aware_flops_separate_nut_draws_of_similar_equity() {
    let cards = small();
    let flop = p("Qh 7h 2c");
    let view = cards.view(&flop);
    // The nut flush draw predicts more nut potential than the K-high one.
    let nut = p("Ah 5h 9s 8d");
    let second = p("Kh 5h 9s 8d");
    assert!(
        cards.predicted_nuts(&nut, &view) > cards.predicted_nuts(&second, &view),
        "{} {}",
        cards.predicted_nuts(&nut, &view),
        cards.predicted_nuts(&second, &view)
    );
}

#[test]
fn nut_outcomes_split_the_nuts_from_second_best() {
    let mut rng = Rng::new(21);
    // On a full board: the nut flush is the nuts; the queen-high flush is
    // beaten only by the ace-high ones (under 3% of holdings): second best.
    let board = p("Kh 8h 3h Qc 2d");
    assert_eq!(
        nut_outcomes(&p("Ah 5h 4s 4c"), &board, 1, &mut rng),
        [1.0, 0.0, 0.0]
    );
    assert_eq!(
        nut_outcomes(&p("Qh 5h 4s 4c"), &board, 1, &mut rng),
        [0.0, 1.0, 0.0]
    );
    assert_eq!(
        nut_outcomes(&p("2s 7c 9d Td"), &board, 1, &mut rng),
        [0.0; 3]
    );
    // On the flop, top set improves past the current nuts (a set) on any
    // board pair, and a nut flush draw ends as the nuts more often than a
    // king-high one.
    let flop = p("Qh 7h 2c");
    let set = nut_outcomes(&p("Qs Qd 5c 4c"), &flop, 400, &mut rng);
    assert!(set[2] > 0.2, "{set:?}");
    let nut = nut_outcomes(&p("Ah 5h 9s 8d"), &flop, 400, &mut rng);
    let second = nut_outcomes(&p("Kh 5h 9s 8d"), &flop, 400, &mut rng);
    assert!(nut[0] > second[0] + 0.1, "{nut:?} {second:?}");
    assert!(second[1] > nut[1], "{nut:?} {second:?}");
}

#[test]
fn nut_first_features_see_freerolls() {
    let at = |board: &str, hole: &str| BoardView::exact(&p(board)).nut_first_features(&p(hole));
    let n = NUM_NUT_FEATURES;
    // Past the nut-aware ones: the nuts with the nut flush draw, redraws to a
    // higher nut straight, a set with a draw, a set.
    let set_fd = at("8h 7h 2c", "8s 8d Ah 5h");
    assert_eq!(set_fd[n + 2], 1.0, "a set with the nut flush draw");
    assert_eq!(set_fd[n + 3], 1.0);
    assert_eq!(
        set_fd[n], 1.0,
        "top set is the nuts, with the nut flush draw"
    );
    // T-9 is the nut straight on 8-7-6; with J-Q as well, more turns keep
    // it the nuts as the straight moves up.
    let wrap = at("8c 7d 6h", "Ts 9s Jd Qc");
    let bare = at("8c 7d 6h", "Ts 9s 3c 2d");
    assert!(wrap[n + 1] > bare[n + 1], "{} {}", wrap[n + 1], bare[n + 1]);
    assert_eq!(
        at("8c 7d 6h", "Ks Qs 3c 2d")[n + 1],
        0.0,
        "no straight, no redraw"
    );
}

#[test]
fn nut_first_abstractions_save_as_version_4() {
    let cards = PloAbstraction::build(
        PloAbstractionConfig {
            hole_cards: 6,
            preflop: 20,
            flop: 20,
            turn: 20,
            river: 20,
            fit_hands: 1200,
            equity_samples: 100,
            nut_first: true,
            ..PloAbstractionConfig::default()
        },
        |_| {},
    );
    assert!(cards.config.nut_aware && cards.config.nut_first);
    let bytes = cards.save();
    assert_eq!(&bytes[..9], b"DUCYPLOA\x04");
    let loaded = PloAbstraction::load(&bytes).expect("loads");
    assert!(loaded.config.nut_first);
    assert_eq!(loaded.save(), bytes);
    assert!(PloAbstraction::load(&bytes[..bytes.len() - 1]).is_none());
    assert!(cards.settings().ends_with("|plo-features-v3"));
    // Training's per-deal buckets equal play's per-hand ones.
    let mut rng = Rng::new(13);
    for _ in 0..20 {
        let (h, b) = random_spot(12, 5, &mut rng);
        let board = [b[0], b[1], b[2], b[3], b[4]];
        let deal = cards.deal_buckets([&h[..6], &h[6..]], &board);
        for (player, hole) in [&h[..6], &h[6..]].into_iter().enumerate() {
            for (street, n) in [0usize, 3, 4, 5].into_iter().enumerate() {
                let b = loaded.hand_bucket(hole, &board[..n]);
                assert_eq!(deal[player][street], b);
                assert!((b as usize) < cards.bucket_count(n));
            }
        }
    }
    // The nut flush draw predicts the nuts more often than the K-high one.
    let flop = p("Qh 7h 2c");
    let view = cards.view(&flop);
    let nut = p("Ah 5h 9s 8d 4c 3c");
    let second = p("Kh 5h 9s 8d 4c 3c");
    assert!(cards.predicted_nuts(&nut, &view) > cards.predicted_nuts(&second, &view));
}
