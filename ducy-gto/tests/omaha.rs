use ducy::{
    deck::Deck,
    games::{
        GameEvaluation,
        flop_game::FlopGame,
        omaha::{OmahaGameEvaluation, OmahaGameState},
    },
};
use ducy_gto::{
    Rng,
    holdem::cards::{Card, bit, mask, to_string},
    omaha::showdown::{PairTable, RiverBoard, draw, equity_vs_random, showdown},
};

fn deck(cards: &[Card]) -> Deck {
    let text: Vec<String> = cards.iter().map(|&c| to_string(c)).collect();
    Deck::parse(&text.join(" ")).unwrap()
}

fn deal(rng: &mut Rng, k: usize) -> (Vec<Card>, Vec<Card>, [Card; 5]) {
    let mut used = 0;
    let a: Vec<Card> = (0..k).map(|_| draw(&mut used, rng)).collect();
    let b: Vec<Card> = (0..k).map(|_| draw(&mut used, rng)).collect();
    let board = std::array::from_fn(|_| draw(&mut used, rng));
    (a, b, board)
}

#[test]
fn showdowns_match_ducys_omaha_evaluator() {
    let mut rng = Rng::new(7);
    let mut results = [0; 3];
    for k in [4, 5, 6] {
        for _ in 0..4000 {
            let (a, b, board) = deal(&mut rng, k);
            let mut state = OmahaGameState::new(k as u32);
            state.add_player(deck(&a)).unwrap();
            state.add_player(deck(&b)).unwrap();
            state.set_flop(deck(&board[..3])).unwrap();
            state
                .set_turn(deck(&board[3..4]).iter(true).next().unwrap())
                .unwrap();
            state
                .set_river(deck(&board[4..]).iter(true).next().unwrap())
                .unwrap();
            let winners: Vec<usize> = OmahaGameEvaluation {}
                .evaluate_winners(&state)
                .iter()
                .map(|w| w.player_index())
                .collect();
            let expected = match winners.as_slice() {
                [0] => 1,
                [1] => -1,
                _ => 0,
            };
            let got = showdown(&a, &b, &board);
            assert_eq!(got, expected, "{a:?} vs {b:?} on {board:?}");
            results[(got + 1) as usize] += 1;
        }
    }
    // Both sides win often, and splits happen.
    assert!(results.iter().all(|&n| n > 100), "{results:?}");
}

#[test]
fn the_pair_table_scores_like_the_board() {
    let mut rng = Rng::new(8);
    for _ in 0..50 {
        let (a, b, board) = deal(&mut rng, 4);
        let table = PairTable::new(&board);
        let river = RiverBoard::new(&board);
        assert_eq!(table.score(&a), river.score(&a));
        assert_eq!(table.score(&b), river.score(&b));
    }
}

/// Exact equity against every opponent hand, over every river.
fn exact_turn_equity(hole: &[Card], board: &[Card; 4]) -> f64 {
    let used = mask(hole) | mask(board);
    let (mut won, mut n) = (0.0, 0u64);
    for river in 0..52 as Card {
        if used & bit(river) != 0 {
            continue;
        }
        let full = [board[0], board[1], board[2], board[3], river];
        let table = PairTable::new(&full);
        let mine = table.score(hole);
        let gone = used | bit(river);
        let left: Vec<Card> = (0..52).filter(|&c| gone & bit(c) == 0).collect();
        let m = left.len();
        for a in 0..m {
            for b in a + 1..m {
                for c in b + 1..m {
                    for d in c + 1..m {
                        let theirs = table.score(&[left[a], left[b], left[c], left[d]]);
                        won += match mine.cmp(&theirs) {
                            std::cmp::Ordering::Greater => 1.0,
                            std::cmp::Ordering::Equal => 0.5,
                            std::cmp::Ordering::Less => 0.0,
                        };
                        n += 1;
                    }
                }
            }
        }
    }
    won / n as f64
}

#[test]
fn sampled_equity_is_within_its_stated_error() {
    // Three turn spots: a made hand, a draw, and a weak hand. At 10,000
    // samples the stated standard error is at most 0.5%, and the estimate
    // lands within four of them of the exact value.
    let p = |s: &str| ducy_gto::holdem::cards::parse(s).unwrap();
    let mut rng = Rng::new(9);
    for (hole, board) in [
        ("As Ks Qd Qc", "Qh 7s 2d 9c"),
        ("Js Ts 9h 8h", "Qh 7s 2s 3c"),
        ("6c 5d 3h 2s", "Kh Kd 9s Jc"),
    ] {
        let hole = p(hole);
        let b = p(board);
        let board = [b[0], b[1], b[2], b[3]];
        let exact = exact_turn_equity(&hole, &board);
        let e = equity_vs_random(&hole, &board, 10_000, &mut rng);
        assert!(e.std_error <= 0.005, "{e:?}");
        assert!(
            (e.mean - exact).abs() <= 4.0 * e.std_error,
            "{hole:?} on {board:?}: sampled {e:?}, exact {exact:.4}"
        );
    }
}

#[test]
fn equity_on_every_street_is_sensible() {
    let p = |s: &str| ducy_gto::holdem::cards::parse(s).unwrap();
    let mut rng = Rng::new(10);
    let aces = p("As Ad Ks Kd");
    let trash = p("7c 4d 3h 2s");
    // Preflop: double-suited aces are a big favourite over a random hand,
    // a rainbow 7-4-3-2 an underdog.
    let a = equity_vs_random(&aces, &[], 4000, &mut rng).mean;
    let t = equity_vs_random(&trash, &[], 4000, &mut rng).mean;
    assert!(a > 0.62 && t < 0.42, "aces {a}, trash {t}");
    // The nuts on the river wins every time.
    let board = p("Ah Kh Qh 2c 3d");
    let royal = p("Jh Th 4s 5s");
    let e = equity_vs_random(&royal, &board, 2000, &mut rng);
    assert_eq!(e.mean, 1.0);
    assert_eq!(e.std_error, 0.0);
}
