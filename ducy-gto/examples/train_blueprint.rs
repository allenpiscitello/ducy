//! Trains the heads-up no-limit Hold'em blueprint with Monte Carlo CFR.
//!
//!     cargo run --release -p ducy-gto --example train_blueprint -- \
//!         --cards abstraction.bin --out blueprint.bin --iters 50000000
//!
//! Options (all optional except `--cards`):
//!
//! - `--cards FILE`: the card abstraction (`build_abstraction` makes one)
//! - `--out FILE`: where to write the blueprint (default `blueprint.bin`)
//! - `--iters N`: iterations to run this time (default 10,000,000)
//! - `--checkpoint FILE`: resume from this file if it exists, and save to
//!   it as training goes (every `--every` iterations and at the end)
//! - `--every N`: report and checkpoint interval (default 1,000,000)
//! - `--seed N`, `--batch N`: sampling seed and parallel batch size
//!
//! Each report prints iterations per second and the button's and big blind's
//! preflop frequencies, which settle as the strategy converges.

use std::{path::PathBuf, time::Instant};

use ducy_gto::{
    Config, Discount, Mccfr,
    holdem::{
        abstraction::CardAbstraction,
        blueprint::Blueprint,
        chart::PreflopReport,
        hunl::{Hunl, HunlConfig},
    },
};

struct Args {
    cards: PathBuf,
    out: PathBuf,
    iters: u64,
    checkpoint: Option<PathBuf>,
    every: u64,
    seed: u64,
    batch: u64,
}

fn args() -> Args {
    let mut a = Args {
        cards: PathBuf::new(),
        out: "blueprint.bin".into(),
        iters: 10_000_000,
        checkpoint: None,
        every: 1_000_000,
        seed: 1,
        batch: 4096,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--cards" => a.cards = value().into(),
            "--out" => a.out = value().into(),
            "--iters" => a.iters = value().replace('_', "").parse().expect("--iters"),
            "--checkpoint" => a.checkpoint = Some(value().into()),
            "--every" => a.every = value().replace('_', "").parse().expect("--every"),
            "--seed" => a.seed = value().parse().expect("--seed"),
            "--batch" => a.batch = value().parse().expect("--batch"),
            f => panic!("unknown option {f}"),
        }
    }
    assert!(!a.cards.as_os_str().is_empty(), "--cards FILE is required");
    a
}

fn main() {
    let a = args();
    let start = Instant::now();
    let cards = CardAbstraction::load(&std::fs::read(&a.cards).expect("read --cards"))
        .expect("a card abstraction");
    println!(
        "loaded {} in {:.1}s",
        a.cards.display(),
        start.elapsed().as_secs_f64()
    );
    let game = Hunl::new(HunlConfig::default(), Some(&cards));
    let stats = game.tree_stats();
    println!(
        "{} betting nodes; {:?} information sets per street; {:.0} MB of regrets and sums",
        game.tree.nodes.len(),
        stats.infosets(&cards),
        stats.table_bytes(&cards) as f64 / 1e6
    );
    let config = Config {
        seed: a.seed,
        batch: a.batch,
        discount: Discount::DCFR,
        // DCFR halves negative regrets every batch (beta = 0), so they never
        // get deep enough for pruning to pay off.
        prune: None,
    };
    let mut solver = match &a.checkpoint {
        Some(p) if p.exists() => {
            let m = Mccfr::load(&game, config, &std::fs::read(p).expect("read checkpoint"))
                .expect("a checkpoint for this run");
            println!("resumed {} at {} iterations", p.display(), m.iterations());
            m
        }
        _ => Mccfr::new(&game, config),
    };
    let strategy = |solver: &Mccfr<Hunl>, node: u32, bucket: u16| {
        let n = game.tree.nodes[node as usize].actions.len();
        solver
            .average_at(&((node as u64) << 16 | bucket as u64))
            .unwrap_or_else(|| vec![1.0 / n as f64; n])
    };

    let target = solver.iterations() + a.iters;
    while solver.iterations() < target {
        let t = Instant::now();
        let before = solver.iterations();
        solver.run(a.every.min(target - before));
        let done = solver.iterations() - before;
        let report = PreflopReport::read(&game, |n, b| strategy(&solver, n, b));
        println!(
            "[{:>7.0}s] {:>11} iterations, {:>7.0}/s, {} infosets | {}",
            start.elapsed().as_secs_f64(),
            solver.iterations(),
            done as f64 / t.elapsed().as_secs_f64(),
            solver.num_infosets(),
            report.summary()
        );
        if let Some(p) = &a.checkpoint {
            std::fs::write(p, solver.save()).expect("write checkpoint");
        }
    }

    let profile = solver.average();
    let blueprint = Blueprint::from_profile(&game, &cards, &profile);
    std::fs::write(&a.out, blueprint.save()).expect("write blueprint");
    println!(
        "wrote {} ({:.1} MB)",
        a.out.display(),
        blueprint.len() as f64 / 1e6
    );
    let report = PreflopReport::read(&game, |n, b| blueprint.probs(n, b));
    println!("{}", report.summary());
    println!("button open %:\n{}", report.button.grid(|s, c| s.raise(c)));
    if let Some(o) = &report.vs_open {
        println!(
            "big blind vs open, continue %:\n{}",
            o.grid(|s, c| 1.0 - s.fold(c))
        );
    }
}
