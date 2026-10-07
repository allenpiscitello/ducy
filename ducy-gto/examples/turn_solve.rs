//! Times the depth-limited turn solver on typical spots: random (but not
//! uniform) ranges for both players, the default menu on the turn and the
//! river, every river played out with uniform continuations, and
//! exploitability as the iterations go. Runs each spot with no chooser and
//! with the opponent choosing among the four continuations.
//!
//!     cargo run --release -p ducy-gto --example turn_solve
//!
//! A number caps the iterations (`-- 50`), and a second deals only that
//! many rivers per iteration (`-- 50 8`).

use std::time::Instant;

use ducy_gto::{
    Rng,
    holdem::{
        cards::{NUM_HOLES, parse},
        hunl::{Betting, HunlConfig},
        turn::{Bias, Continuation, TurnSolver},
    },
};

fn main() {
    let most: usize = std::env::args()
        .nth(1)
        .map_or(100, |s| s.parse().expect("iterations"));
    let samples: usize = std::env::args()
        .nth(2)
        .map_or(0, |s| s.parse().expect("rivers per iteration"));
    let config = HunlConfig::default();
    // (board, pot per player, stack behind)
    let spots = [
        ("Qs Td 7h 4c", 10, 190),
        ("Ah Kh 8d 8c", 30, 170),
        ("Kc Jd 5s 4h", 60, 140),
    ];
    let mut rng = Rng::new(7);
    for (board, put_in, behind) in spots {
        let b: [u8; 4] = parse(board).unwrap().try_into().unwrap();
        let root = Betting::street_start(2, [behind, behind], [put_in, put_in], 2);
        let ranges: Vec<Vec<f64>> = (0..2)
            .map(|_| (0..NUM_HOLES).map(|_| rng.next_f64().powi(2)).collect())
            .collect();
        for chooser in [false, true] {
            let t = Instant::now();
            let mut s = TurnSolver::new(
                b,
                &root,
                &config,
                &[],
                [&ranges[0], &ranges[1]],
                |_, leaf| Continuation::uniform(leaf, &config),
                |_| vec![0; NUM_HOLES],
            );
            if chooser {
                s.set_chooser(0, &Bias::ALL);
            }
            s.set_river_samples(samples, 1);
            let setup = t.elapsed().as_secs_f64() * 1000.0;
            let leaves = s
                .tree
                .nodes
                .iter()
                .filter(|n| n.actions.is_empty() && n.betting.folded.is_none())
                .count();
            println!(
                "{board}: pot {}, stacks {behind}, {}: {} nodes ({leaves} leaves), setup {setup:.0} ms",
                2 * put_in,
                if chooser {
                    "button chooses"
                } else {
                    "no chooser"
                },
                s.tree.nodes.len()
            );
            let pot = (2 * put_in) as f64;
            let (mut done, mut spent) = (0, 0.0);
            for target in [5, 10, 25, 50, 100, 200].into_iter().filter(|&t| t <= most) {
                let t = Instant::now();
                s.run(target - done);
                spent += t.elapsed().as_secs_f64() * 1000.0;
                done = target;
                let e = s.exploitability();
                println!(
                    "  {target:>4} iterations {spent:>8.0} ms   exploitability {:.3}% of the pot",
                    100.0 * e / pot
                );
            }
        }
    }
}
