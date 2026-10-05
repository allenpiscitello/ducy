use ducy_gto::{
    Rng,
    holdem::{
        cards::{NUM_HOLES, hole_index, parse},
        hunl::{BetMenu, Betting, HunlAction, HunlConfig, Size},
        river::{RiverHands, RiverSolver, RiverTree, Strategy},
    },
};

fn board(s: &str) -> [u8; 5] {
    parse(s).unwrap().try_into().unwrap()
}

/// A config whose river menu is `sizes` by raise depth.
fn menu(sizes: Vec<Vec<Size>>) -> HunlConfig {
    HunlConfig {
        menu: BetMenu {
            preflop: BetMenu::default().preflop,
            postflop: sizes,
        },
        ..HunlConfig::default()
    }
}

fn random_range(rng: &mut Rng, hands: &RiverHands) -> Vec<f32> {
    (0..hands.len())
        .map(|_| {
            let x = rng.next_f64() as f32;
            if x < 0.3 { 0.0 } else { x }
        })
        .collect()
}

#[test]
fn showdown_and_fold_values_match_brute_force() {
    let mut rng = Rng::new(5);
    // A paired board has many ties.
    for b in ["Ks 9d 6h 2c 2d", "Ah Kh Qh Jh 3c", "7c 7d 7h 7s 2d"] {
        let hands = RiverHands::new(board(b));
        let n = hands.len();
        assert_eq!(n, 1081);
        let opp = random_range(&mut rng, &hands);
        let mut fast = vec![0f32; n];
        hands.showdown(&opp, 3.0, &mut fast);
        let mut fold = vec![0f32; n];
        hands.fold(&opp, -2.0, &mut fold);
        for h in 0..n {
            let [a, b] = hands.cards[h];
            let (mut sd, mut fd) = (0f64, 0f64);
            for g in 0..n {
                let c = hands.cards[g];
                if c.contains(&a) || c.contains(&b) {
                    continue;
                }
                let w = opp[g] as f64;
                sd += w * hands.strength[h].cmp(&hands.strength[g]) as i32 as f64;
                fd += w;
            }
            assert!((fast[h] as f64 - 3.0 * sd).abs() < 1e-3, "{b} hand {h}");
            assert!((fold[h] as f64 + 2.0 * fd).abs() < 1e-3, "{b} hand {h}");
        }
    }
}

#[test]
fn tree_follows_the_menu_and_forced_sizes() {
    let root = Betting::street_start(3, [180, 180], [20, 20], 2);
    let config = HunlConfig::default();
    let tree = RiverTree::build(&root, &config, &[]);
    // Big blind first: check, a third, three quarters, 1.25 pot, all-in.
    assert_eq!(
        tree.nodes[0].actions,
        vec![
            HunlAction::Check,
            HunlAction::Bet(13),
            HunlAction::Bet(30),
            HunlAction::Bet(50),
            HunlAction::Bet(180)
        ]
    );
    // A real bet of 23 after a check is added at the button's node.
    let path = [HunlAction::Check, HunlAction::Bet(23)];
    let tree = RiverTree::build(&root, &config, &path);
    let at = tree.follow(&path).expect("on the tree");
    let n = &tree.nodes[at as usize];
    assert_eq!(n.betting.to_act, 1);
    assert_eq!(n.betting.to_call(), 23);
    let button = &tree.nodes[tree.follow(&path[..1]).unwrap() as usize];
    assert_eq!(button.actions.len(), 6);
    assert!(button.actions.windows(2).all(|w| match (w[0], w[1]) {
        (HunlAction::Bet(x), HunlAction::Bet(y)) => x < y,
        _ => true,
    }));
}

/// Pot 20, a single pot-sized bet and no raises. The big blind holds the
/// nuts (nines or sixes full, 6 combos) or air (T8, 16 combos); the button
/// holds KQ, which beats the air and loses to the nuts.
#[test]
fn polar_range_bluffs_and_bluff_catcher_calls_at_indifference() {
    let b = board("Ks 9d 6h 2c 2d");
    let config = menu(vec![vec![Size::Pot(1.0)], vec![]]);
    let root = Betting::street_start(3, [180, 180], [10, 10], 2);
    let mut ranges = [vec![0f64; NUM_HOLES], vec![0f64; NUM_HOLES]];
    let combos = |r: &str| -> Vec<usize> {
        let mut out = Vec::new();
        for x in "cdhs".chars() {
            for y in "cdhs".chars() {
                let s = format!("{}{x} {}{y}", &r[..1], &r[1..]);
                if let Some(c) = parse(&s)
                    && c[0] != c[1]
                    && !b.contains(&c[0])
                    && !b.contains(&c[1])
                {
                    out.push(hole_index(c[0], c[1]));
                }
            }
        }
        out.sort();
        out.dedup();
        out
    };
    let (value, air, catchers) = (
        [combos("99"), combos("66")].concat(),
        combos("T8"),
        combos("KQ"),
    );
    assert_eq!((value.len(), air.len(), catchers.len()), (6, 16, 12));
    for &h in value.iter().chain(&air) {
        ranges[1][h] = 1.0;
    }
    for &h in &catchers {
        ranges[0][h] = 1.0;
    }
    let mut s = RiverSolver::new(b, &root, &config, &[], [&ranges[0], &ranges[1]]);
    s.run(1000);
    let bet_freq = |hs: &[usize]| -> f64 {
        hs.iter().map(|&h| s.probs(0, h).unwrap()[1]).sum::<f64>()
    };
    let (v, bl) = (bet_freq(&value), bet_freq(&air));
    // Bluffs to value: bet / (pot + bet) = 1/2.
    let ratio = bl / v;
    assert!(v > 5.9, "value bets {v}");
    assert!((ratio - 0.5).abs() < 0.03, "bluff:value {ratio}");
    // Facing the bet, KQ calls pot / (pot + bet) = 1/2 of the time.
    let facing = s.tree.follow(&[HunlAction::Bet(20)]).unwrap();
    let call = catchers
        .iter()
        .map(|&h| s.probs(facing, h).unwrap()[1])
        .sum::<f64>()
        / catchers.len() as f64;
    assert!((call - 0.5).abs() < 0.03, "call frequency {call}");
    // Within 2% of the pot (many hands are indifferent here).
    assert!(s.exploitability() < 0.4, "{}", s.exploitability());
}

fn spot(seed: u64) -> (RiverSolver, HunlConfig) {
    let b = board("Qs Td 7h 4c 2s");
    let config = menu(vec![vec![Size::Pot(0.5), Size::AllIn], vec![Size::AllIn]]);
    let root = Betting::street_start(3, [60, 60], [20, 20], 2);
    let mut rng = Rng::new(seed);
    let ranges: Vec<Vec<f64>> = (0..2)
        .map(|_| (0..NUM_HOLES).map(|_| rng.next_f64()).collect())
        .collect();
    let s = RiverSolver::new(b, &root, &config, &[], [&ranges[0], &ranges[1]]);
    (s, config)
}

#[test]
fn exploitability_falls_toward_zero() {
    let (mut s, _) = spot(1);
    s.run(5);
    let early = s.exploitability();
    s.run(195);
    let late = s.exploitability();
    println!("pot 40: exploitability {early:.3} after 5, {late:.4} after 200");
    assert!(late < early / 10.0, "{early} -> {late}");
    // Under 0.25% of the pot.
    assert!(late < 0.1, "{late}");
}

/// The gadget's promise: against the re-solved strategy, no opponent hand
/// does better than it would against a reference strategy (here a uniform
/// one), up to convergence.
#[test]
fn gadget_never_lets_an_opponent_hand_gain_over_the_reference() {
    let (mut s, _) = spot(2);
    let n = s.hands.len();
    let uniform: Strategy = s
        .tree
        .nodes
        .iter()
        .map(|x| vec![1.0 / x.actions.len().max(1) as f32; x.actions.len() * n])
        .collect();
    // The button (0) plays the solution; the big blind may opt out.
    let target = s.best_response(1, &uniform);
    s.set_gadget(1, target.clone());
    s.run(300);
    let after = s.best_response(1, &s.average());
    let range = s.range(1);
    let (mut worst, mut gain) = (0f32, 0f64);
    for h in 0..n {
        if range[h] > 0.0 {
            // Relative to how much opponent weight the hand can face.
            worst = worst.max(after[h] - target[h]);
        }
        gain += ((target[h] - after[h]) * range[h]) as f64;
    }
    println!("worst hand over target {worst:.4}; total improvement {gain:.1}");
    // Values are in chips times opponent weight (about 500): allow 0.05% of
    // the pot of 40 per unit of weight.
    assert!(worst < 0.0005 * 40.0 * 600.0, "{worst}");
    assert!(gain > 0.0);
}

#[test]
fn frozen_nodes_keep_their_strategy() {
    let (mut s, _) = spot(3);
    let n = s.hands.len();
    let k = s.tree.nodes[0].actions.len();
    let mut f = vec![0f32; k * n];
    f[..n].fill(1.0);
    s.freeze(0, f.clone());
    s.run(20);
    assert_eq!(s.average_at(0), f);
    assert_eq!(s.probs(0, hole_index(0, 1)).unwrap()[0], 1.0);
}
