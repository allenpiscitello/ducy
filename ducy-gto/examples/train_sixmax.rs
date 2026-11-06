//! Trains the six-handed blueprint to the river (#338) with Monte Carlo CFR,
//! one traversal per seat per iteration. Hold'em with a Hold'em card
//! abstraction, or PLO (4 to 6 hole cards, as the abstraction says) with a
//! PLO one.
//!
//!     cargo run --release -p ducy-gto --example train_sixmax -- \
//!         --cards holdem-cards.bin --menu holdem --iters 100000000 \
//!         --every 5000000 --checkpoint holdem6.ckpt --out holdem6.bin
//!     cargo run --release -p ducy-gto --example train_sixmax -- \
//!         --plo-cards plo4-cards.bin --menu plo ...
//!
//! Size a configuration with `sixmax_size` first.
//!
//! Options (`--cards` or `--plo-cards` is required):
//!
//! - `--cards FILE` / `--plo-cards FILE`: the card abstraction
//! - `--menu NAME`: a menu from `MultiwayConfig::presets` (default `holdem`
//!   or `plo`)
//! - `--bb N`: stacks in big blinds (default 100)
//! - `--iters N`: iterations to run this time (default 10,000,000)
//! - `--every N`: report and checkpoint interval (default 1,000,000)
//! - `--checkpoint FILE`: resume from it if it exists, save to it as
//!   training goes (written to `FILE.tmp`, then renamed)
//! - `--out FILE`: the blueprint (default `sixmax-blueprint.bin`)
//! - `--max-gb N`: stop (after a checkpoint and the blueprint) once the
//!   solver's tables pass about N GB (default 20)
//! - `--seed N`, `--batch N`
//! - `--prune-threshold X`: turn on regret pruning (`Prune`): skip
//!   actions with regret below X (off by default)
//! - `--prune-after N`, `--prune-full-every N`: pruning starts after N
//!   iterations (default 1,000,000), every N-th iteration explores
//!   everything (default 20)
//!
//! Each report prints the speed, information sets, the tables' memory, and
//! how often UTG, the button (folded to) and the small blind (folded to)
//! open, averaged over random hands.

use std::{
    io::BufWriter,
    path::{Path, PathBuf},
    time::Instant,
};

use ducy_gto::{
    Config, Discount, Mccfr, Prune, Rng,
    holdem::{
        abstraction::CardAbstraction,
        cards::{Card, NUM_CARDS, bit},
        hunl::Buckets,
    },
    omaha::abstraction::PloAbstraction,
    sixmax::multiway::{MultiwayBlueprint, MultiwayConfig, MultiwayGame},
};

/// Bytes per stored information set besides its actions (index and slot).
const PER_INFOSET: f64 = 36.0;

struct Args {
    cards: Option<PathBuf>,
    plo_cards: Option<PathBuf>,
    menu: Option<String>,
    bb: u32,
    iters: u64,
    every: u64,
    checkpoint: Option<PathBuf>,
    out: PathBuf,
    max_gb: f64,
    seed: u64,
    batch: u64,
    prune_threshold: Option<f64>,
    prune_after: u64,
    prune_full_every: u64,
}

fn args() -> Args {
    let mut a = Args {
        cards: None,
        plo_cards: None,
        menu: None,
        bb: 100,
        iters: 10_000_000,
        every: 1_000_000,
        checkpoint: None,
        out: "sixmax-blueprint.bin".into(),
        max_gb: 20.0,
        seed: 1,
        batch: 4096,
        prune_threshold: None,
        prune_after: 1_000_000,
        prune_full_every: 20,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        let num = |v: String| v.replace('_', "").parse::<u64>().expect("a number");
        match flag.as_str() {
            "--cards" => a.cards = Some(value().into()),
            "--plo-cards" => a.plo_cards = Some(value().into()),
            "--menu" => a.menu = Some(value()),
            "--bb" => a.bb = num(value()) as u32,
            "--iters" => a.iters = num(value()),
            "--every" => a.every = num(value()),
            "--checkpoint" => a.checkpoint = Some(value().into()),
            "--out" => a.out = value().into(),
            "--max-gb" => a.max_gb = value().parse().expect("--max-gb"),
            "--seed" => a.seed = num(value()),
            "--batch" => a.batch = num(value()),
            "--prune-threshold" => {
                a.prune_threshold = Some(value().parse().expect("--prune-threshold"))
            }
            "--prune-after" => a.prune_after = num(value()),
            "--prune-full-every" => a.prune_full_every = num(value()),
            f => panic!("unknown option {f}"),
        }
    }
    assert!(
        a.cards.is_some() != a.plo_cards.is_some(),
        "give one of --cards FILE (Hold'em) or --plo-cards FILE (PLO)"
    );
    a
}

fn main() {
    let a = args();
    if let Some(path) = &a.cards {
        let cards = CardAbstraction::load(&std::fs::read(path).expect("read --cards"))
            .expect("a Hold'em card abstraction");
        train::<_, 2>(&a, &cards, "holdem");
    } else if let Some(path) = &a.plo_cards {
        let cards = PloAbstraction::load(&std::fs::read(path).expect("read --plo-cards"))
            .expect("a PLO card abstraction");
        match cards.config.hole_cards {
            4 => train::<_, 4>(&a, &cards, "plo"),
            5 => train::<_, 5>(&a, &cards, "plo"),
            _ => train::<_, 6>(&a, &cards, "plo"),
        }
    }
}

/// `H` random distinct cards.
fn random_hand<const H: usize>(rng: &mut Rng) -> [Card; H] {
    let mut used = 0u64;
    std::array::from_fn(|_| {
        loop {
            let c = (rng.next_u64() % NUM_CARDS as u64) as Card;
            if used & bit(c) == 0 {
                used |= bit(c);
                break c;
            }
        }
    })
}

/// How often the player at `node` raises, over random hands.
fn raise_rate<const H: usize>(
    game: &MultiwayGame<impl Buckets, H>,
    cards: &impl Buckets,
    node: u32,
    strategy: impl Fn(u64, usize) -> Vec<f64>,
) -> f64 {
    let mut rng = Rng::new(42);
    let n = game.tree.nodes[node as usize].actions as usize;
    // Fold, then call (when on the menu), then raises: count what isn't
    // a fold or a call.
    let first = game.tree.nodes[node as usize].first_child as usize;
    let raises: Vec<usize> = (0..n)
        .filter(|&k| game.tree.nodes[first + k].via >= 3)
        .collect();
    let hands = 2000;
    let mut total = 0.0;
    for _ in 0..hands {
        let hole = random_hand::<H>(&mut rng);
        let b = cards.hand_bucket(&hole, &[]) as u64;
        let p = strategy((node as u64) << 16 | b, n);
        total += raises.iter().map(|&k| p[k]).sum::<f64>();
    }
    total / hands as f64
}

/// The process's resident memory in GB (Linux), or 0 if unknown.
fn resident_gb() -> f64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<f64>().ok())
        .map_or(0.0, |pages| pages * 4096.0 / 1e9)
}

fn write_atomically(path: &Path, f: impl FnOnce(&mut BufWriter<std::fs::File>)) {
    let tmp = path.with_extension("tmp");
    {
        let mut w = BufWriter::new(std::fs::File::create(&tmp).expect("create file"));
        f(&mut w);
        std::io::Write::flush(&mut w).expect("write file");
    }
    std::fs::rename(&tmp, path).expect("rename into place");
}

fn train<A: Buckets + Sync, const H: usize>(a: &Args, cards: &A, game_name: &str) {
    let start = Instant::now();
    let menu = a.menu.clone().unwrap_or_else(|| game_name.to_string());
    let config = MultiwayConfig::presets(a.bb)
        .into_iter()
        .find(|(name, _)| *name == menu)
        .unwrap_or_else(|| panic!("no menu named {menu}"))
        .1;
    assert_eq!(config.pot_limit, H > 2, "a pot-limit menu for PLO only");
    let game = MultiwayGame::<_, H>::new(config, Some(cards));
    let settings = format!("{menu} {} BB, {H} hole cards | {}", a.bb, cards.settings());
    println!(
        "{settings}\n{} betting nodes, built in {:.0} s",
        game.tree.nodes.len(),
        start.elapsed().as_secs_f64()
    );
    let solver_config = Config {
        seed: a.seed,
        batch: a.batch,
        discount: Discount::DCFR,
        prune: a.prune_threshold.map(|threshold| Prune {
            after: a.prune_after,
            threshold,
            full_every: a.prune_full_every,
        }),
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
    // UTG first in; the button and the small blind when it's folded to them.
    let nodes = &game.tree.nodes;
    let fold_from =
        |node: u32, folds: usize| (0..folds).fold(node, |n, _| nodes[n as usize].first_child);
    let spots = [
        ("UTG", 0),
        ("BTN", fold_from(0, 3)),
        ("SB", fold_from(0, 4)),
    ];

    let target = solver.iterations() + a.iters;
    let mut stop = false;
    while solver.iterations() < target && !stop {
        let t = Instant::now();
        let before = solver.iterations();
        solver.run(a.every.min(target - before));
        let rate = (solver.iterations() - before) as f64 / t.elapsed().as_secs_f64();
        let memory = solver.num_infosets() as f64 * PER_INFOSET + solver.table_bytes() as f64;
        let strategy = |info: u64, n: usize| {
            solver
                .average_at(&info)
                .unwrap_or_else(|| vec![1.0 / n as f64; n])
        };
        let opens: Vec<String> = spots
            .iter()
            .map(|(name, node)| {
                format!(
                    "{name} {:.0}%",
                    100.0 * raise_rate(&game, cards, *node, strategy)
                )
            })
            .collect();
        println!(
            "[{:>7.0}s] {:>11} iterations, {:>6.0}/s, {} infosets, tables ~{:.1} GB, \
             resident {:.1} GB | opens {}",
            start.elapsed().as_secs_f64(),
            solver.iterations(),
            rate,
            solver.num_infosets(),
            memory / 1e9,
            resident_gb(),
            opens.join(", ")
        );
        if let Some(p) = &a.checkpoint {
            write_atomically(p, |w| solver.save_to(w).expect("write checkpoint"));
        }
        if memory > a.max_gb * 1e9 {
            println!("tables passed {} GB: stopping", a.max_gb);
            stop = true;
        }
    }

    write_atomically(&a.out, |w| {
        MultiwayBlueprint::write(
            &settings,
            solver.num_infosets(),
            |f| solver.average_each(f),
            w,
        )
        .expect("write blueprint")
    });
    println!(
        "wrote {} after {} iterations, {:.0} s",
        a.out.display(),
        solver.iterations(),
        start.elapsed().as_secs_f64()
    );
}
