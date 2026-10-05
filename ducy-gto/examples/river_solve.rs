//! Times the river solver on typical spots, single-threaded: random (but
//! not uniform) ranges for both players, the default river menu, and
//! exploitability as the iterations go.
//!
//!     cargo run --release -p ducy-gto --example river_solve

use std::time::Instant;

use ducy_gto::{
    Rng,
    holdem::{
        cards::{NUM_HOLES, parse},
        hunl::{Betting, HunlConfig},
        river::RiverSolver,
    },
};

fn main() {
    let config = HunlConfig::default();
    // (board, pot per player, stack behind)
    let spots = [
        ("Qs Td 7h 4c 2s", 10, 190),
        ("Ah Kh 8d 8c 3h", 30, 170),
        ("9s 8s 7d 2c 2h", 60, 140),
        ("Kc Jd 5s 4h 3d", 100, 100),
    ];
    let mut rng = Rng::new(7);
    if std::env::var("SDBENCH").is_ok() {
        let b: [u8; 5] = parse("Qs Td 7h 4c 2s").unwrap().try_into().unwrap();
        let h = ducy_gto::holdem::river::RiverHands::new(b);
        let opp: Vec<f32> = (0..h.len()).map(|_| rng.next_f64() as f32).collect();
        let mut out = vec![0f32; h.len()];
        let t = Instant::now();
        for _ in 0..10000 {
            h.showdown(&opp, 1.0, &mut out);
        }
        println!("showdown {:?}", t.elapsed() / 10000);
        let t = Instant::now();
        for _ in 0..10000 {
            h.fold(&opp, 1.0, &mut out);
        }
        println!("fold {:?}", t.elapsed() / 10000);
    }
    for (board, put_in, behind) in spots {
        let b: [u8; 5] = parse(board).unwrap().try_into().unwrap();
        let root = Betting::street_start(3, [behind, behind], [put_in, put_in], 2);
        let ranges: Vec<Vec<f64>> = (0..2)
            .map(|_| (0..NUM_HOLES).map(|_| rng.next_f64().powi(2)).collect())
            .collect();
        let t = Instant::now();
        let mut s = RiverSolver::new(b, &root, &config, &[], [&ranges[0], &ranges[1]]);
        let setup = t.elapsed().as_secs_f64() * 1000.0;
        let decisions = s.tree.nodes.iter().filter(|n| !n.actions.is_empty()).count();
        println!(
            "{board}: pot {}, stacks {behind}: {} nodes ({decisions} decisions), setup {setup:.1} ms",
            2 * put_in,
            s.tree.nodes.len()
        );
        let pot = (2 * put_in) as f64;
        let mut done = 0;
        let mut spent = 0.0;
        for target in [25, 50, 100, 200, 400, 1000] {
            let t = Instant::now();
            s.run(target - done);
            spent += t.elapsed().as_secs_f64() * 1000.0;
            done = target;
            let e = s.exploitability();
            println!(
                "  {target:>5} iterations {spent:>8.0} ms   exploitability {:.3}% of the pot",
                100.0 * e / pot
            );
        }
    }
}
