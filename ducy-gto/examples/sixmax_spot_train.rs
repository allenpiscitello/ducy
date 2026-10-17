//! Trains a six-handed postflop spot (#267): a single-raised pot, from the
//! flop, with both players' ranges from the six-handed preflop solve.
//!
//!     cargo run --release -p ducy-gto --example sixmax_spot_train -- \
//!         --strategy sixmax-100.bin --cards cards-full.bin --opener BTN --caller BB \
//!         --iters 30000000 --every 2000000 --checkpoint spot-100-BTN-BB.bin \
//!         [--progress progress.json] [--threads N]
//!
//! `--strategy` is the compact preflop strategy (`sixmax_export`); its depth
//! is the spot's. A checkpoint is resumed if it exists and saved at every
//! report. Each report prints the speed, the information sets so far, and
//! how the player out of position starts the flop (check or bet, by share
//! of their range).

use std::time::Instant;

use ducy_gto::{
    Config, Discount, Game, Mccfr, Turn,
    holdem::{abstraction::CardAbstraction, hunl::HunlAction},
    sixmax::{bot::PreflopStrategy, preflop::POSITIONS, spot::SpotGame},
};

fn main() {
    let mut strategy_path = String::new();
    let mut cards_path = String::new();
    let mut opener = String::from("BTN");
    let mut caller = String::from("BB");
    let mut iters = 10_000_000u64;
    let mut every = 1_000_000u64;
    let mut checkpoint: Option<String> = None;
    let mut progress: Option<String> = None;
    let mut seed = 1u64;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut v = || args.next().unwrap_or_else(|| panic!("{a} needs a value"));
        match a.as_str() {
            "--strategy" => strategy_path = v(),
            "--cards" => cards_path = v(),
            "--opener" => opener = v(),
            "--caller" => caller = v(),
            "--iters" => iters = v().replace('_', "").parse().expect("--iters"),
            "--every" => every = v().replace('_', "").parse().expect("--every"),
            "--checkpoint" => checkpoint = Some(v()),
            "--progress" => progress = Some(v()),
            "--seed" => seed = v().parse().expect("--seed"),
            _ => panic!("unknown argument {a}"),
        }
    }
    let pos = |name: &str| {
        POSITIONS
            .iter()
            .position(|p| *p == name)
            .unwrap_or_else(|| panic!("no position {name}"))
    };
    let t0 = Instant::now();
    let strategy =
        PreflopStrategy::from_compact(&std::fs::read(&strategy_path).expect("read --strategy"))
            .expect("a compact strategy");
    let cards = CardAbstraction::load(&std::fs::read(&cards_path).expect("read --cards"))
        .expect("a card abstraction");
    let game = SpotGame::single_raised(&strategy, pos(&opener), pos(&caller), &cards)
        .expect("that line is in the strategy");
    let depth = strategy.game().stack / 2;
    let pot = game.tree.nodes[0].betting.pot() + game.dead;
    println!(
        "{} at {depth} BB: pot {} chips ({} dead), {} betting nodes, loaded in {:.0}s",
        game.name,
        pot,
        game.dead,
        game.tree.nodes.len(),
        t0.elapsed().as_secs_f64()
    );
    let config = Config {
        seed,
        batch: 256,
        discount: Discount::DCFR,
        prune: None,
    };
    let mut solver = match &checkpoint {
        Some(p) if std::path::Path::new(p).exists() => {
            let m = Mccfr::load(&game, config, &std::fs::read(p).expect("read --checkpoint"))
                .expect("a checkpoint for this spot");
            println!("resumed at {} iterations", m.iterations());
            m
        }
        _ => Mccfr::new(&game, config),
    };
    // How the player out of position starts the flop, over their range:
    // sample deals and average their strategy at the root.
    let root_mix = |solver: &Mccfr<SpotGame>| -> String {
        let mut rng = ducy_gto::Rng::new(7);
        let actions = &game.tree.nodes[0].actions;
        let mut sum = vec![0.0; actions.len()];
        let n = 4000;
        for _ in 0..n {
            let s = game.sample_chance(&game.root(), &mut rng);
            debug_assert_eq!(game.turn(&s), Turn::Player(1));
            let p = solver
                .average_at(&game.info(&s))
                .unwrap_or_else(|| vec![1.0 / actions.len() as f64; actions.len()]);
            for (x, q) in sum.iter_mut().zip(p) {
                *x += q / n as f64;
            }
        }
        actions
            .iter()
            .zip(sum)
            .map(|(a, p)| {
                let name = match a {
                    HunlAction::Check => "check".to_string(),
                    HunlAction::Bet(to) => format!("bet {:.0}%", 100.0 * *to as f64 / pot as f64),
                    other => format!("{other:?}"),
                };
                format!("{name} {:.0}%", 100.0 * p)
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let target = solver.iterations() + iters;
    while solver.iterations() < target {
        let t = Instant::now();
        let before = solver.iterations();
        solver.run(every.min(target - before));
        let rate = (solver.iterations() - before) as f64 / t.elapsed().as_secs_f64();
        println!(
            "[{:>6.0}s] {:>11} iterations, {:>6.0}/s, {} infosets | OOP flop: {}",
            t0.elapsed().as_secs_f64(),
            solver.iterations(),
            rate,
            solver.num_infosets(),
            root_mix(&solver)
        );
        if let Some(p) = &checkpoint {
            std::fs::write(p, solver.save()).expect("write --checkpoint");
        }
        if let Some(p) = &progress {
            let left = (target - solver.iterations()) as f64 / rate / 60.0;
            let json = format!(
                "{{\"label\": \"6-max postflop: {} at {depth} BB\", \"done\": {}, \"total\": {target}, \"unit\": \"iterations\", \"etaMin\": {left:.0}}}\n",
                game.name,
                solver.iterations()
            );
            let _ = std::fs::write(p, json);
        }
    }
}
