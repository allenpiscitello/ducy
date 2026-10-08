//! Times the depth-limited turn solver as the bot uses it: the real card
//! abstraction and blueprint, leaves continued with the blueprint's river
//! strategy, and the opponent choosing among the four continuations. The
//! spots follow the blueprint's tree to the turn (preflop limped and
//! checked, the flop checked through) on a few boards.
//!
//!     cargo run --release -p ducy-gto --example turn_bench -- \
//!         --cards model/cards.bin --blueprint model/blueprint.bin [--iterations 50] [--rivers 8]

use std::time::Instant;

use ducy_gto::holdem::{
    abstraction::CardAbstraction,
    blueprint::Blueprint,
    cards::{NUM_HOLES, parse},
    hunl::{BettingTree, Hunl, HunlAction, HunlConfig},
    turn::{Bias, TurnSolver},
};

fn main() {
    let (mut cards_path, mut blueprint_path) = (String::new(), String::new());
    let (mut iterations, mut rivers) = (50usize, 0usize);
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut v = || it.next().expect("a value");
        match flag.as_str() {
            "--cards" => cards_path = v(),
            "--blueprint" => blueprint_path = v(),
            "--iterations" => iterations = v().parse().expect("--iterations"),
            "--rivers" => rivers = v().parse().expect("--rivers"),
            f => panic!("unknown option {f}"),
        }
    }
    let cards = CardAbstraction::load(&std::fs::read(&cards_path).expect("read --cards"))
        .expect("abstraction");
    let config = HunlConfig::default();
    let game = Hunl::new(config.clone(), Some(&cards));
    let blueprint = Blueprint::load(
        &std::fs::read(&blueprint_path).expect("read --blueprint"),
        &game,
        &cards,
    )
    .expect("a blueprint");
    let tree = BettingTree::build(&config);
    println!("river buckets: {}", cards.num_buckets(5));
    // Follow calls and checks to the start of the turn.
    let mut node = 0u32;
    while tree.nodes[node as usize].betting.street < 2 {
        let x = &tree.nodes[node as usize];
        let a = x
            .actions
            .iter()
            .position(|a| matches!(a, HunlAction::Call | HunlAction::Check))
            .expect("a passive action");
        node = x.children[a];
    }
    let root = tree.nodes[node as usize].betting.clone();
    let ranges = vec![1.0f64; NUM_HOLES];
    for board in ["Qs Td 7h 4c", "Ah Kh 8d 8c", "9c 6c 5d 2h"] {
        let b: [u8; 4] = parse(board).unwrap().try_into().unwrap();
        let t = Instant::now();
        let mut s = TurnSolver::from_blueprint(
            b,
            &root,
            &config,
            &[],
            [&ranges, &ranges],
            &cards,
            &blueprint,
            &tree,
            node,
        );
        s.set_chooser(0, &Bias::ALL);
        s.set_river_samples(rivers, 1);
        let setup = t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        s.run(iterations);
        let spent = t.elapsed().as_secs_f64() * 1000.0;
        let pot = (root.contributed[0] + root.contributed[1]) as f64;
        println!(
            "{board}: {} nodes, setup {setup:.0} ms, {iterations} iterations {spent:.0} ms ({:.1} ms each), exploitability {:.2}% of the pot",
            s.tree.nodes.len(),
            spent / iterations as f64,
            100.0 * s.exploitability() / pot
        );
    }
}
