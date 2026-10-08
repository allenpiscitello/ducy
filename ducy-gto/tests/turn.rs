use ducy_gto::{
    Rng,
    holdem::{
        cards::{NUM_CARDS, NUM_HOLES, bit, hole_cards, hole_index, mask, parse, score},
        hunl::{BetMenu, Betting, HunlAction, HunlConfig, Size},
        river::{RiverTree, Strategy},
        turn::{Bias, Continuation, TurnSolver},
    },
};

fn board(s: &str) -> [u8; 4] {
    parse(s).unwrap().try_into().unwrap()
}

/// A config whose postflop menu is `sizes` by raise depth.
fn menu(sizes: Vec<Vec<Size>>) -> HunlConfig {
    HunlConfig {
        menu: BetMenu {
            preflop: BetMenu::default().preflop,
            postflop: sizes,
        },
        ..HunlConfig::default()
    }
}

fn random_ranges(seed: u64) -> [Vec<f64>; 2] {
    let mut rng = Rng::new(seed);
    let mut r = || -> Vec<f64> {
        (0..NUM_HOLES)
            .map(|_| {
                let x = rng.next_f64();
                if x < 0.3 { 0.0 } else { x }
            })
            .collect()
    };
    [r(), r()]
}

fn solver(b: &str, config: &HunlConfig, root: &Betting, seed: u64, uniform: bool) -> TurnSolver {
    let ranges = random_ranges(seed);
    TurnSolver::new(
        board(b),
        root,
        config,
        &[],
        [&ranges[0], &ranges[1]],
        |_, leaf| {
            if uniform {
                Continuation::uniform(leaf, config)
            } else {
                Continuation::passive(leaf, config)
            }
        },
        |_| vec![0; NUM_HOLES],
    )
}

/// Every hand plays its node's actions equally often.
fn uniform(s: &TurnSolver) -> Strategy {
    let n = s.hands.len();
    s.tree
        .nodes
        .iter()
        .map(|x| vec![1.0 / x.actions.len().max(1) as f32; x.actions.len() * n])
        .collect()
}

#[test]
fn the_tree_stops_where_the_turn_ends() {
    let root = Betting::street_start(2, [180, 180], [20, 20], 2);
    let tree = RiverTree::build(&root, &HunlConfig::default(), &[]);
    let leaves: Vec<_> = tree.nodes.iter().filter(|x| x.actions.is_empty()).collect();
    assert!(
        leaves
            .iter()
            .all(|x| x.betting.street == 3 || x.betting.is_over())
    );
    // Check-check, and a bet called, reach the river with chips behind.
    let cc = tree
        .follow(&[HunlAction::Check, HunlAction::Check])
        .unwrap();
    let x = &tree.nodes[cc as usize];
    assert!(x.actions.is_empty() && x.betting.street == 3 && !x.betting.is_over());
}

/// With checking only, the one leaf is a showdown after every river:
/// compare with scoring each pair of hands on each river.
#[test]
fn check_down_leaf_values_match_brute_force() {
    let b = board("Ks 9d 6h 2c");
    let config = menu(vec![vec![]]);
    let root = Betting::street_start(2, [180, 180], [20, 20], 2);
    let s = solver("Ks 9d 6h 2c", &config, &root, 7, false);
    let ranges = random_ranges(7);
    assert_eq!(s.hands.len(), 1128);
    let v = s.best_response(0, &uniform(&s));
    let bm = mask(&b);
    let rivers: Vec<u8> = (0..NUM_CARDS as u8).filter(|&c| bm & bit(c) == 0).collect();
    for h in (0..s.hands.len()).step_by(41) {
        let [a, c] = s.hands.cards[h];
        let mut total = 0f64;
        for (o, &w) in ranges[1].iter().enumerate() {
            let (x, y) = hole_cards(o);
            let blocked = bm | bit(a) | bit(c);
            if w == 0.0 || blocked & (bit(x) | bit(y)) != 0 {
                continue;
            }
            let mut sum = 0f64;
            for &r in &rivers {
                if (blocked | bit(x) | bit(y)) & bit(r) != 0 {
                    continue;
                }
                let me = score(bm | bit(r) | bit(a) | bit(c));
                let them = score(bm | bit(r) | bit(x) | bit(y));
                sum += me.cmp(&them) as i32 as f64;
            }
            total += w * 20.0 * sum / 44.0;
        }
        assert!(
            (v[h] as f64 - total).abs() < 1e-2 * (1.0 + total.abs()),
            "hand {h}: {} vs {total}",
            v[h]
        );
    }
    // The leaf is reached by the hand in the right place too.
    let i = s.hands.index[hole_index(s.hands.cards[0][0], s.hands.cards[0][1])];
    assert_eq!(i, 0);
}

fn spot(seed: u64, uniform: bool) -> TurnSolver {
    let config = menu(vec![vec![Size::Pot(0.75), Size::AllIn], vec![Size::AllIn]]);
    let root = Betting::street_start(2, [80, 80], [20, 20], 2);
    solver("Qs Td 7h 4c", &config, &root, seed, uniform)
}

#[test]
fn exploitability_falls_toward_zero() {
    let mut s = spot(1, false);
    s.run(5);
    let early = s.exploitability();
    s.run(95);
    let late = s.exploitability();
    println!("pot 40: exploitability {early:.3} after 5, {late:.4} after 100");
    assert!(late < early / 5.0, "{early} -> {late}");
    // Under 0.5% of the pot.
    assert!(late < 0.2, "{late}");
}

/// Dealing a few rivers per iteration still converges: the sampled leaf
/// values are unbiased.
#[test]
fn sampled_rivers_still_converge() {
    let mut s = spot(1, false);
    s.set_river_samples(8, 9);
    s.run(5);
    let early = s.exploitability();
    s.run(95);
    let late = s.exploitability();
    println!("8 rivers per iteration: {early:.3} after 5, {late:.4} after 100");
    assert!(late < early / 4.0, "{early} -> {late}");
    // Under 1.5% of the pot.
    assert!(late < 0.6, "{late}");
}

/// A chooser who may also bias their river play does at least as well
/// with each hand as one who can't, and the solver still converges.
#[test]
fn a_chooser_gains_from_its_continuations() {
    let plain = spot(2, true);
    let mut chooser = spot(2, true);
    chooser.set_chooser(1, &Bias::ALL);
    let fixed = uniform(&plain);
    let (a, b) = (
        plain.best_response(1, &fixed),
        chooser.best_response(1, &fixed),
    );
    let mut better = 0;
    for h in 0..a.len() {
        assert!(b[h] >= a[h] - 1e-3 * (1.0 + a[h].abs()), "hand {h}");
        if b[h] > a[h] + 1e-3 {
            better += 1;
        }
    }
    assert!(better > 100, "{better} hands gain");
    chooser.run(3);
    let early = chooser.exploitability();
    chooser.run(17);
    let late = chooser.exploitability();
    println!("with a chooser: {early:.3} after 3, {late:.3} after 20");
    assert!(late < early, "{early} -> {late}");
    let leaf = chooser
        .tree
        .follow(&[HunlAction::Check, HunlAction::Check])
        .unwrap();
    let pick = chooser.choice_at(leaf).expect("a choice at check-check");
    assert_eq!(pick.len(), 4 * chooser.hands.len());
}

/// As on the river: against the re-solved strategy no opponent hand does
/// better than against the reference it was given.
#[test]
fn gadget_never_lets_an_opponent_hand_gain_over_the_reference() {
    let mut s = spot(3, false);
    let n = s.hands.len();
    let reference = uniform(&s);
    let target = s.best_response(1, &reference);
    s.set_gadget(1, target.clone());
    s.run(100);
    let after = s.best_response(1, &s.average());
    let range = s.range(1);
    let (mut worst, mut gain) = (0f32, 0f64);
    for h in 0..n {
        if range[h] > 0.0 {
            worst = worst.max(after[h] - target[h]);
        }
        gain += ((target[h] - after[h]) * range[h]) as f64;
    }
    println!("worst hand over target {worst:.4}; total improvement {gain:.1}");
    // Values are chips times opponent weight (about 500 pairs): allow
    // 0.01% of the pot of 40 per unit of weight.
    assert!(worst < 0.0001 * 40.0 * 600.0, "{worst}");
    assert!(gain > 0.0);
}

#[test]
fn frozen_nodes_keep_their_strategy() {
    let mut s = spot(4, false);
    let n = s.hands.len();
    let k = s.tree.nodes[0].actions.len();
    let mut f = vec![0f32; k * n];
    f[..n].fill(1.0);
    s.freeze(0, f.clone());
    s.run(10);
    assert_eq!(s.average_at(0), f);
    let [a, b] = s.hands.cards[0];
    assert_eq!(s.probs(0, hole_index(a, b)).unwrap()[0], 1.0);
}
