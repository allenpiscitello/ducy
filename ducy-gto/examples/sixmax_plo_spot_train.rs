//! Trains six-handed PLO postflop spots (#290): pots two players took to the
//! flop, from the flop on, with both ranges from the six-handed PLO solve
//! (`sixmax::plo_spot`).
//!
//!     cargo run --release -p ducy-gto --example sixmax_plo_spot_train -- \
//!         --cards plo-cards.bin --solve solve-100.bin --equity equity.bin \
//!         --line fffrfc --total 20000000 --every 2000000 \
//!         --checkpoint plo-spot-100-fffrfc.bin
//!
//! `--solve` is a `sixmax_plo` checkpoint for `--cards`' preflop buckets
//! (with its `--equity`; realization doesn't change how it loads). `--line`
//! is the preflop line, one letter per decision from UTG (`f` fold, `c`
//! call or check, `r` raise); `--lines all` trains each of
//! `plo_spot::LINES` in turn, `--total` iterations each, to
//! `<checkpoint dir>/plo-spot-<bb>-<line>.bin`. `--total` trains up to that
//! many iterations in all (a stopped run resumes); a checkpoint is saved at
//! every report.

use std::time::Instant;

use ducy_gto::{
    Config, Discount, Game, Mccfr, Turn,
    holdem::hunl::{Buckets, HunlAction},
    omaha::abstraction::PloAbstraction,
    sixmax::{
        bot::PreflopStrategy,
        equity::PreflopEquity,
        plo_spot::{LINES, PloSpotGame},
        preflop::PreflopGame,
        realize::Realization,
        solver::PreflopSolver,
    },
};

fn train(game: &PloSpotGame, total: u64, every: u64, path: &str, seed: u64) {
    let t0 = Instant::now();
    let pot = game.tree.nodes[0].betting.pot() + game.dead;
    println!(
        "{} ({}): pot {pot} chips ({} dead), {} betting nodes",
        game.name,
        game.line,
        game.dead,
        game.tree.nodes.len()
    );
    let config = Config {
        seed,
        batch: 4096,
        discount: Discount::DCFR,
        prune: None,
    };
    let mut solver = match std::fs::read(path) {
        Ok(bytes) => {
            let m = Mccfr::load(game, config, &bytes).expect("a checkpoint for this spot");
            println!("resumed at {} iterations", m.iterations());
            m
        }
        Err(_) => Mccfr::new(game, config),
    };
    // How the player out of position starts the flop, over their range.
    let root_mix = |solver: &Mccfr<PloSpotGame>| -> String {
        let mut rng = ducy_gto::Rng::new(7);
        let actions = &game.tree.nodes[0].actions;
        let mut sum = vec![0.0; actions.len()];
        let n = 2000;
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
    while solver.iterations() < total {
        let t = Instant::now();
        let before = solver.iterations();
        solver.run(every.min(total - before));
        let rate = (solver.iterations() - before) as f64 / t.elapsed().as_secs_f64();
        println!(
            "[{:>6.0}s] {:>11} iterations, {:>6.0}/s, {} infosets | OOP flop: {}",
            t0.elapsed().as_secs_f64(),
            solver.iterations(),
            rate,
            solver.num_infosets(),
            root_mix(&solver)
        );
        std::fs::write(path, solver.save()).expect("write --checkpoint");
    }
}

fn main() {
    let (mut cards_path, mut solve_path, mut equity_path) =
        (String::new(), String::new(), String::new());
    let mut line = String::from("fffrfc");
    let mut total = 10_000_000u64;
    let mut every = 1_000_000u64;
    let mut checkpoint = String::new();
    let mut bb = 100u32;
    let mut seed = 1u64;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut v = || args.next().unwrap_or_else(|| panic!("{a} needs a value"));
        match a.as_str() {
            "--cards" => cards_path = v(),
            "--solve" => solve_path = v(),
            "--equity" => equity_path = v(),
            "--line" | "--lines" => line = v(),
            "--total" => total = v().replace('_', "").parse().expect("--total"),
            "--every" => every = v().replace('_', "").parse().expect("--every"),
            "--checkpoint" => checkpoint = v(),
            "--bb" => bb = v().parse().expect("--bb"),
            "--seed" => seed = v().parse().expect("--seed"),
            _ => panic!("unknown argument {a}"),
        }
    }
    assert!(!checkpoint.is_empty(), "--checkpoint FILE is required");
    let cards = PloAbstraction::load(&std::fs::read(&cards_path).expect("read --cards"))
        .expect("a PLO abstraction");
    let n = cards.bucket_count(0);
    let eq = PreflopEquity::from_bytes(&std::fs::read(&equity_path).expect("read --equity"))
        .expect("equity table");
    let game = PreflopGame::pot_limit(bb, Realization::uniform(n));
    let solver = PreflopSolver::new(&game, &eq, 1);
    solver
        .load(&std::fs::read(&solve_path).expect("read --solve"))
        .expect("a sixmax_plo solve for these buckets");
    let strategy = PreflopStrategy::from_solver(&game, &solver);
    if line == "all" {
        let dir = std::path::Path::new(&checkpoint)
            .parent()
            .map_or(".".to_string(), |d| d.display().to_string());
        for l in LINES {
            let spot = PloSpotGame::from_line(&strategy, l, &cards).expect("a line of this solve");
            train(
                &spot,
                total,
                every,
                &format!("{dir}/plo-spot-{bb}-{l}.bin"),
                seed,
            );
        }
    } else {
        let spot = PloSpotGame::from_line(&strategy, &line, &cards).expect("a line of this solve");
        train(&spot, total, every, &checkpoint, seed);
    }
}
