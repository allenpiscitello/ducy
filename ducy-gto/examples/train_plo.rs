//! Trains a heads-up PLO blueprint with Monte Carlo CFR (#130).
//!
//!     cargo run --release -p ducy-gto --example plo_abstraction -- --out plo-cards.bin
//!     cargo run --release -p ducy-gto --example train_plo -- \
//!         --cards plo-cards.bin --bb 100 --iters 20000000 --out plo-blueprint.bin
//!
//! Options (all optional except `--cards`):
//!
//! - `--cards FILE`: the PLO card abstraction (`plo_abstraction` builds one)
//! - `--bb N`: stack depth in big blinds (default 100; the test game is 20)
//! - `--menu lean|full`: `BetMenu::pot_limit_lean` (default) or `pot_limit`
//! - `--iters N`: iterations to run this time (default 10,000,000)
//! - `--every N`: report and checkpoint interval (default 1,000,000)
//! - `--checkpoint FILE`: resume from it if it exists, save to it as training goes
//! - `--out FILE`: the blueprint (default `plo-blueprint.bin`)
//! - `--br N`: at each report, a sampled best response fitted and measured
//!   on N deals each (`ducy_gto::sampled`); 0 turns it off (default 0)
//! - `--snapshots DIR`: also write the blueprint at each report, as
//!   `DIR/plo-<iterations>.bin`, to compare stages of training head to head
//! - `--seed N`, `--batch N`
//!
//! Each report prints the speed, information sets, the sampled
//! exploitability if asked for, and the preflop frequencies: the button's
//! fold / limp / raise, and the big blind's fold / call / 3-bet against a
//! pot-sized open.

use std::{path::PathBuf, time::Instant};

use ducy_gto::{
    Config, Discount, Mccfr, Rng,
    holdem::{
        blueprint::Blueprint,
        hunl::{BetMenu, Buckets, HuPlo, HunlConfig},
    },
    omaha::abstraction::{PloAbstraction, random_spot},
    sampled::sampled_exploitability,
};

struct Args {
    cards: PathBuf,
    bb: u64,
    full: bool,
    iters: u64,
    every: u64,
    checkpoint: Option<PathBuf>,
    out: PathBuf,
    br: usize,
    snapshots: Option<PathBuf>,
    seed: u64,
    batch: u64,
}

fn args() -> Args {
    let mut a = Args {
        cards: PathBuf::new(),
        bb: 100,
        full: false,
        iters: 10_000_000,
        every: 1_000_000,
        checkpoint: None,
        out: "plo-blueprint.bin".into(),
        br: 0,
        snapshots: None,
        seed: 1,
        batch: 4096,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        let num = |v: String| v.replace('_', "").parse::<u64>().expect("a number");
        match flag.as_str() {
            "--cards" => a.cards = value().into(),
            "--bb" => a.bb = num(value()),
            "--menu" => a.full = value() == "full",
            "--iters" => a.iters = num(value()),
            "--every" => a.every = num(value()),
            "--checkpoint" => a.checkpoint = Some(value().into()),
            "--out" => a.out = value().into(),
            "--br" => a.br = num(value()) as usize,
            "--snapshots" => a.snapshots = Some(value().into()),
            "--seed" => a.seed = num(value()),
            "--batch" => a.batch = num(value()),
            f => panic!("unknown option {f}"),
        }
    }
    assert!(!a.cards.as_os_str().is_empty(), "--cards FILE is required");
    a
}

/// The button's fold / limp / raise and the big blind's fold / call /
/// 3-bet against an open, averaged over random hands.
fn preflop_summary(
    game: &HuPlo<PloAbstraction>,
    cards: &PloAbstraction,
    strategy: &impl Fn(u64, usize) -> Vec<f64>,
) -> String {
    let mut rng = Rng::new(42);
    let root = &game.tree.nodes[0];
    let open = root.children[root.actions.len() - 1];
    let k = game.tree.nodes[open as usize].actions.len();
    let (mut sb, mut bb) = ([0.0; 3], [0.0; 3]);
    let n = 4000;
    for _ in 0..n {
        let (hole, _) = random_spot(4, 0, &mut rng);
        let b = cards.hand_bucket(&hole, &[]) as u64;
        let s = strategy(b, root.actions.len());
        sb[0] += s[0];
        sb[1] += s[1];
        sb[2] += s[2..].iter().sum::<f64>();
        let t = strategy((open as u64) << 16 | b, k);
        bb[0] += t[0];
        bb[1] += t[1];
        bb[2] += t[2..].iter().sum::<f64>();
    }
    let pct = |x: f64| 100.0 * x / n as f64;
    format!(
        "button fold/limp/raise {:.0}/{:.0}/{:.0}%, big blind vs open fold/call/3-bet {:.0}/{:.0}/{:.0}%",
        pct(sb[0]),
        pct(sb[1]),
        pct(sb[2]),
        pct(bb[0]),
        pct(bb[1]),
        pct(bb[2])
    )
}

fn main() {
    let a = args();
    let start = Instant::now();
    let cards = PloAbstraction::load(&std::fs::read(&a.cards).expect("read --cards"))
        .expect("a PLO card abstraction");
    let config = HunlConfig {
        menu: if a.full {
            BetMenu::pot_limit()
        } else {
            BetMenu::pot_limit_lean()
        },
        ..HunlConfig::pot_limit_omaha_lean(a.bb)
    };
    let game = HuPlo::with_cards(config, Some(&cards));
    let stats = game.tree_stats();
    println!(
        "PLO {} BB, {} menu: {} betting nodes; {:?} information sets per street; {:.0} MB of regrets and sums",
        a.bb,
        if a.full { "full" } else { "lean" },
        game.tree.nodes.len(),
        stats.infosets(&cards),
        stats.table_bytes(&cards) as f64 / 1e6
    );
    let solver_config = Config {
        seed: a.seed,
        batch: a.batch,
        discount: Discount::DCFR,
        prune: None,
    };
    let mut solver = match &a.checkpoint {
        Some(p) if p.exists() => {
            let m = Mccfr::load(
                &game,
                solver_config,
                &std::fs::read(p).expect("read checkpoint"),
            )
            .expect("a checkpoint for this run");
            println!("resumed {} at {} iterations", p.display(), m.iterations());
            m
        }
        _ => Mccfr::new(&game, solver_config),
    };

    let target = solver.iterations() + a.iters;
    while solver.iterations() < target {
        let t = Instant::now();
        let before = solver.iterations();
        solver.run(a.every.min(target - before));
        let done = solver.iterations() - before;
        let rate = done as f64 / t.elapsed().as_secs_f64();
        let strategy = |info: u64, n: usize| {
            solver
                .average_at(&info)
                .unwrap_or_else(|| vec![1.0 / n as f64; n])
        };
        let mut line = format!(
            "[{:>6.0}s] {:>11} iterations, {:>6.0}/s, {} infosets",
            start.elapsed().as_secs_f64(),
            solver.iterations(),
            rate,
            solver.num_infosets()
        );
        if a.br > 0 {
            let s = |i: &u64, n: usize| strategy(*i, n);
            let t = Instant::now();
            let r = sampled_exploitability(&game, &s, a.br, a.br, 9);
            let mbb = |x: f64| x / game.config.big_blind as f64 * 1000.0;
            line += &format!(
                " | sampled exploitability {:.0} ± {:.0} mbb/hand ({:.0} s)",
                mbb(r.value),
                mbb(1.96 * r.std_error),
                t.elapsed().as_secs_f64()
            );
        }
        println!("{line} | {}", preflop_summary(&game, &cards, &strategy));
        if let Some(dir) = &a.snapshots {
            let bp = Blueprint::from_strategy(&game, &cards, strategy);
            let path = dir.join(format!("plo-{}.bin", solver.iterations()));
            std::fs::write(&path, bp.save()).expect("write snapshot");
        }
        if let Some(p) = &a.checkpoint {
            std::fs::write(p, solver.save()).expect("write checkpoint");
        }
    }

    let blueprint = Blueprint::from_strategy(&game, &cards, |info, n| {
        solver
            .average_at(&info)
            .unwrap_or_else(|| vec![1.0 / n as f64; n])
    });
    std::fs::write(&a.out, blueprint.save()).expect("write blueprint");
    println!(
        "wrote {} ({:.1} MB) after {} iterations, {:.0} s",
        a.out.display(),
        blueprint.len() as f64 / 1e6,
        solver.iterations(),
        start.elapsed().as_secs_f64()
    );
}
