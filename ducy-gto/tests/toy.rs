use ducy_gto::{
    Cfr, Game, Profile, Turn, Variant, best_response_value, expected_value, exploitability,
    games::{kuhn, leduc},
};

/// Rock-paper-scissors as a sequential game where player 1 can't see
/// player 0's move: the textbook case with a unique equilibrium (uniform).
struct Rps;

impl Game for Rps {
    type State = Vec<usize>;
    type Info = usize;
    fn root(&self) -> Vec<usize> {
        Vec::new()
    }
    fn turn(&self, s: &Vec<usize>) -> Turn {
        match s.len() {
            2 => Turn::Terminal,
            n => Turn::Player(n),
        }
    }
    fn utility(&self, s: &Vec<usize>) -> f64 {
        match (3 + s[0] - s[1]) % 3 {
            0 => 0.0,
            1 => 1.0,
            _ => -1.0,
        }
    }
    fn chance_outcomes(&self, _: &Vec<usize>) -> Vec<(Vec<usize>, f64)> {
        Vec::new()
    }
    fn num_actions(&self, _: &Vec<usize>) -> usize {
        3
    }
    fn apply(&self, s: &Vec<usize>, a: usize) -> Vec<usize> {
        let mut n = s.clone();
        n.push(a);
        n
    }
    /// Each player only knows whose turn it is.
    fn info(&self, s: &Vec<usize>) -> usize {
        s.len()
    }
}

/// One player, one decision with a single action: regret matching must play
/// it with probability 1.
struct OneAction;

impl Game for OneAction {
    type State = u8;
    type Info = ();
    fn root(&self) -> u8 {
        0
    }
    fn turn(&self, s: &u8) -> Turn {
        if *s == 0 {
            Turn::Player(0)
        } else {
            Turn::Terminal
        }
    }
    fn utility(&self, _: &u8) -> f64 {
        -1.0
    }
    fn chance_outcomes(&self, _: &u8) -> Vec<(u8, f64)> {
        Vec::new()
    }
    fn num_actions(&self, _: &u8) -> usize {
        1
    }
    fn apply(&self, _: &u8, _: usize) -> u8 {
        1
    }
    fn info(&self, _: &u8) {}
}

#[test]
fn best_response_to_a_fixed_strategy() {
    // Player 0 always plays rock (0): the best response is paper, winning 1.
    let mut p = Profile::new();
    p.set(0, vec![1.0, 0.0, 0.0]);
    assert_eq!(best_response_value(&Rps, &p, 1), 1.0);
    // Uniform play can't be exploited.
    assert!(exploitability(&Rps, &Profile::new()).abs() < 1e-12);
    // Player 0's rock against uniform: worth 0, and its best response gains 0.
    assert!(expected_value(&Rps, &p).abs() < 1e-12);
    assert!(best_response_value(&Rps, &p, 0).abs() < 1e-12);
}

#[test]
fn regret_matching_with_one_action() {
    let mut cfr = Cfr::new(&OneAction, Variant::Plus);
    cfr.run(5);
    assert_eq!(cfr.average().get(&()), Some(&[1.0][..]));
    assert_eq!(expected_value(&OneAction, &cfr.average()), -1.0);
}

#[test]
fn rock_paper_scissors_converges_to_uniform() {
    let mut cfr = Cfr::new(&Rps, Variant::Plus);
    cfr.run(2000);
    assert!(exploitability(&Rps, &cfr.average()) < 1e-3);
}

#[test]
fn kuhn_value_and_exploitability() {
    for (variant, iters, tol) in [
        (Variant::Vanilla, 20_000, 3e-3),
        (Variant::Plus, 2_000, 1e-4),
    ] {
        let mut cfr = Cfr::new(&kuhn::Kuhn, variant);
        cfr.run(iters);
        let avg = cfr.average();
        assert_eq!(cfr.num_infosets(), 12);
        let e = exploitability(&kuhn::Kuhn, &avg);
        assert!(e < tol, "{variant:?}: exploitability {e}");
        let v = expected_value(&kuhn::Kuhn, &avg);
        assert!(
            (v - kuhn::GAME_VALUE).abs() < 1e-3,
            "{variant:?}: value {v}"
        );
    }
}

#[test]
fn kuhn_strategy_matches_the_known_equilibrium() {
    let mut cfr = Cfr::new(&kuhn::Kuhn, Variant::Plus);
    cfr.run(5000);
    let avg = cfr.average();
    let bet = |info: &str| avg.get(&info.to_string()).unwrap()[kuhn::BET];
    // Player 1 calls a bet with the king, folds the jack, and bets the king
    // when checked to.
    assert!(bet("K:b") > 0.999);
    assert!(bet("J:b") < 0.001);
    assert!(bet("K:p") > 0.999);
    // Player 1 bluffs the jack a third of the time after a check.
    assert!((bet("J:p") - 1.0 / 3.0).abs() < 0.01);
    // Player 0 never opens the queen, and calls with it a third of the time
    // more than it bluffs the jack (call = alpha + 1/3).
    assert!(bet("Q:") < 0.01);
    assert!((bet("Q:pb") - (bet("J:") + 1.0 / 3.0)).abs() < 0.02);
    // Player 0 bets the king three times as often as it bluffs the jack.
    assert!((bet("K:") - 3.0 * bet("J:")).abs() < 0.02);
}

#[test]
fn leduc_converges_below_one_millichip() {
    let mut cfr = Cfr::new(&leduc::Leduc, Variant::Plus);
    cfr.run(1000);
    assert_eq!(cfr.num_infosets(), 288);
    let avg = cfr.average();
    let e = exploitability(&leduc::Leduc, &avg);
    assert!(e < 1e-3, "exploitability {e}");
    let v = expected_value(&leduc::Leduc, &avg);
    assert!((v - leduc::GAME_VALUE).abs() < 1e-3, "value {v}");
}

#[test]
fn leduc_rules() {
    let g = leduc::Leduc;
    let root = g.root();
    assert_eq!(g.turn(&root), Turn::Chance);
    let deals = g.chance_outcomes(&root);
    assert_eq!(deals.len(), 30);
    // Player 0 holds a king (card 4), player 1 a jack (card 0).
    let s = deals
        .iter()
        .find(|(s, _)| s.cards == Some([4, 0]))
        .unwrap()
        .0
        .clone();
    assert_eq!(g.turn(&s), Turn::Player(0));
    assert_eq!(s.actions(), &['k', 'b']);
    let s = g.apply(&s, 1); // bet 2
    assert_eq!(s.actions(), &['f', 'c', 'r']);
    let s = g.apply(&s, 2); // raise to 5
    assert_eq!(s.contrib, [3, 5]);
    assert_eq!(s.actions(), &['f', 'c']); // capped at two bets
    let s = g.apply(&s, 1); // call
    assert_eq!(g.turn(&s), Turn::Chance);
    // Board: the other jack (card 1) pairs player 1.
    let s = g
        .chance_outcomes(&s)
        .into_iter()
        .find(|(s, _)| s.board == Some(1))
        .unwrap()
        .0;
    assert_eq!(g.info(&s), "KJ:brc/");
    let s = g.apply(&g.apply(&s, 0), 0); // check, check
    assert_eq!(g.turn(&s), Turn::Terminal);
    assert_eq!(g.utility(&s), -5.0);
}
