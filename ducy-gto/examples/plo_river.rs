//! Measures real-time river solving for heads-up PLO against the blueprint
//! (#132):
//!
//!     cargo run --release -p ducy-gto --example plo_river -- \
//!         --cards plo-cards.bin --blueprint plo-blueprint.bin --spots 300
//!
//! Options: `--bb N` (the blueprint's depth, default 100), `--spots N`
//! (river spots, default 300), `--iterations N` and `--hands N` (the solve's
//! settings, default 200 and 256), `--exploiter N` (hands in the
//! exploiter's range, default 1024).
//!
//! River spots come from the blueprint playing itself on its tree until
//! the river. At each one, for each player as the bot, the bot's range is
//! sampled as it samples it when playing (its own hand first), and it
//! plays either the blueprint's river strategy or the solved one. A best
//! response to each, by the other player holding a fresh, larger sample of
//! their range, measures how exploitable it is on this river. The paired
//! difference, solved minus blueprint, is the change solving makes, in
//! mbb/hand with a 95% interval across spots. Negative means solving is
//! harder to exploit.

use std::time::Instant;

use ducy_gto::{
    Game, Rng, Turn,
    holdem::{
        blueprint::Blueprint,
        cards::{Card, mask},
        hunl::{HuPlo, HunlConfig},
    },
    omaha::{
        abstraction::PloAbstraction,
        river::{PloRiverSolver, best_response_value, sample_range},
    },
};

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn num(name: &str, default: usize) -> usize {
    arg(name).map_or(default, |v| v.replace('_', "").parse().expect(name))
}

fn mean_ci(x: &[f64]) -> (f64, f64) {
    let n = x.len().max(2) as f64;
    let m = x.iter().sum::<f64>() / n;
    let v = x.iter().map(|y| (y - m).powi(2)).sum::<f64>() / (n - 1.0);
    (m, 1.96 * (v / n).sqrt())
}

fn main() {
    let cards = PloAbstraction::load(&std::fs::read(arg("--cards").expect("--cards")).unwrap())
        .expect("a PLO abstraction");
    let bb = num("--bb", 100) as u64;
    let config = HunlConfig::pot_limit_omaha_lean(bb);
    let game = HuPlo::with_cards(config.clone(), Some(&cards));
    let bytes = std::fs::read(arg("--blueprint").expect("--blueprint")).unwrap();
    let blueprint = Blueprint::load(&bytes, &game, &cards).expect("a blueprint for this game");
    let spots = num("--spots", 300);
    let iterations = num("--iterations", 200);
    let hands = num("--hands", 256);
    let exploiter = num("--exploiter", 1024);

    let mut rng = Rng::new(17);
    let (mut diffs, mut plain, mut solved_v) = (Vec::new(), Vec::new(), Vec::new());
    let mut times = Vec::new();
    let mut found = 0;
    while found < spots {
        // Play the blueprint on its tree to a river decision.
        let mut s = game.sample_chance(&game.root(), &mut rng);
        let mut steps = Vec::new();
        let river_root = loop {
            match game.turn(&s) {
                Turn::Player(p) => {
                    let node = s.node;
                    let b = game.betting(&s);
                    if b.street == 3 {
                        break Some(node);
                    }
                    let probs = blueprint.probs(node, s.buckets[p][b.street]);
                    let a = rng.sample(&probs);
                    steps.push((node, a));
                    s = game.apply(&s, a);
                }
                _ => break None,
            }
        };
        let Some(bp_root) = river_root else { continue };
        found += 1;
        let board = s.board;
        let root = game.tree.nodes[bp_root as usize].betting.clone();
        for bot in 0..2 {
            let hole = s.hole[bot];
            let mut sample = |player: usize, blocked: u64, first: Option<[Card; 4]>, n: usize| {
                sample_range(
                    &cards, &blueprint, &game.tree, &steps, player, &board, blocked, n, first,
                    &mut rng,
                )
            };
            let mine = sample(bot, 0, Some(hole), hands);
            let theirs = sample(1 - bot, mask(&hole), None, hands);
            let fresh = sample(1 - bot, 0, None, exploiter);
            let ranges = if bot == 0 {
                [mine.clone(), theirs]
            } else {
                [theirs, mine.clone()]
            };
            let t = Instant::now();
            let mut solver = PloRiverSolver::new(&board, &root, &config, &[], ranges);
            let reference =
                solver.blueprint_strategy(&cards, &board, &blueprint, &game.tree, bp_root);
            let target = solver.best_response(1 - bot, &reference);
            solver.set_gadget(1 - bot, target);
            solver.run(iterations, None);
            times.push(t.elapsed().as_secs_f64());
            let tree = &solver.tree;
            let mbb = |chips: f64| chips / config.big_blind as f64 * 1000.0;
            let a = mbb(best_response_value(
                tree,
                &board,
                1 - bot,
                &mine,
                &reference,
                &fresh,
            ));
            let b = mbb(best_response_value(
                tree,
                &board,
                1 - bot,
                &mine,
                &solver.average(),
                &fresh,
            ));
            plain.push(a);
            solved_v.push(b);
            diffs.push(b - a);
        }
    }
    let (d, dci) = mean_ci(&diffs);
    let (p, pci) = mean_ci(&plain);
    let (q, qci) = mean_ci(&solved_v);
    times.sort_by(f64::total_cmp);
    let mean_t = times.iter().sum::<f64>() / times.len() as f64;
    println!(
        "{spots} river spots, both players as the bot ({} solves of {iterations} iterations, {hands} hands per range; exploiter {exploiter} hands)",
        times.len()
    );
    println!("best response vs the blueprint's river: {p:.0} ± {pci:.0} mbb/hand");
    println!("best response vs the solved river:      {q:.0} ± {qci:.0} mbb/hand");
    println!("change from solving (paired):           {d:+.0} ± {dci:.0} mbb/hand");
    println!(
        "solve time: mean {:.0} ms, median {:.0} ms, slowest {:.0} ms (one core)",
        mean_t * 1e3,
        times[times.len() / 2] * 1e3,
        times[times.len() - 1] * 1e3
    );
}
