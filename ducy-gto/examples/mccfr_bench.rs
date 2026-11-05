//! Six-handed MCCFR speed at a small fixed size, for comparing solver
//! changes: warms the tables, then times a run and prints the rate, the
//! table size and a hash of the checkpoint (equal hashes, equal results).
//!
//!     RAYON_NUM_THREADS=4 cargo run --release -p ducy-gto --example mccfr_bench -- \
//!         --plo-cards plo4-cards.bin --warm 100000 --iters 100000
//!
//! Options: `--plo-cards FILE` (PLO with that abstraction; Hold'em with a
//! quick 8-bucket abstraction without it), `--menu NAME`, `--bb N`,
//! `--warm N`, `--iters N`, `--batch N`, `--seed N`, and pruning with
//! `--prune-threshold X` (from the end of the warm-up, every 20th iteration
//! full).

use std::{path::PathBuf, time::Instant};

use ducy_gto::{
    Config, Discount, Mccfr, Prune,
    holdem::{abstraction::CardAbstraction, hunl::Buckets},
    omaha::abstraction::PloAbstraction,
    sixmax::multiway::{MultiwayConfig, MultiwayGame},
};

struct Args {
    plo_cards: Option<PathBuf>,
    menu: Option<String>,
    bb: u32,
    warm: u64,
    iters: u64,
    batch: u64,
    seed: u64,
    prune_threshold: Option<f64>,
}

fn args() -> Args {
    let mut a = Args {
        plo_cards: None,
        menu: None,
        bb: 100,
        warm: 100_000,
        iters: 100_000,
        batch: 4096,
        seed: 1,
        prune_threshold: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        let num = |v: String| v.replace('_', "").parse::<u64>().expect("a number");
        match flag.as_str() {
            "--plo-cards" => a.plo_cards = Some(value().into()),
            "--menu" => a.menu = Some(value()),
            "--bb" => a.bb = num(value()) as u32,
            "--warm" => a.warm = num(value()),
            "--iters" => a.iters = num(value()),
            "--batch" => a.batch = num(value()),
            "--seed" => a.seed = num(value()),
            "--prune-threshold" => {
                a.prune_threshold = Some(value().parse().expect("--prune-threshold"))
            }
            f => panic!("unknown option {f}"),
        }
    }
    a
}

fn main() {
    let a = args();
    if let Some(path) = &a.plo_cards {
        let cards = PloAbstraction::load(&std::fs::read(path).expect("read --plo-cards"))
            .expect("a PLO card abstraction");
        match cards.config.hole_cards {
            4 => bench::<_, 4>(&a, &cards, "plo"),
            5 => bench::<_, 5>(&a, &cards, "plo"),
            _ => bench::<_, 6>(&a, &cards, "plo"),
        }
    } else {
        bench::<_, 2>(&a, &CardAbstraction::quick(8), "holdem");
    }
}

/// FNV-1a over everything written.
struct Hash(u64);

impl std::io::Write for Hash {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        for &b in buf {
            self.0 = (self.0 ^ b as u64).wrapping_mul(0x100_0000_01b3);
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn bench<A: Buckets + Sync, const H: usize>(a: &Args, cards: &A, game_name: &str) {
    let menu = a.menu.clone().unwrap_or_else(|| game_name.to_string());
    let config = MultiwayConfig::presets(a.bb)
        .into_iter()
        .find(|(name, _)| *name == menu)
        .unwrap_or_else(|| panic!("no menu named {menu}"))
        .1;
    let game = MultiwayGame::<_, H>::new(config, Some(cards));
    let mut solver = Mccfr::new(
        &game,
        Config {
            seed: a.seed,
            batch: a.batch,
            discount: Discount::DCFR,
            prune: a.prune_threshold.map(|threshold| Prune {
                after: a.warm,
                threshold,
                full_every: 20,
            }),
        },
    );
    let t = Instant::now();
    solver.run(a.warm);
    let warm = t.elapsed().as_secs_f64();
    let warmed = solver.iterations();
    let t = Instant::now();
    solver.run(a.iters);
    let timed = t.elapsed().as_secs_f64();
    let ran = solver.iterations() - warmed;
    let mut hash = Hash(0xcbf2_9ce4_8422_2325);
    solver.save_to(&mut hash).expect("hash");
    println!(
        "warm {:.0}/s, timed {:.0}/s, {} infosets, tables {:.1} MB, checkpoint hash {:016x}",
        warmed as f64 / warm,
        ran as f64 / timed,
        solver.num_infosets(),
        solver.table_bytes() as f64 / 1e6,
        hash.0
    );
}
