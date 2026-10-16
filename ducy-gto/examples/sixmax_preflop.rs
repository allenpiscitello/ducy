//! Trains the six-handed preflop strategy (#249) and prints its charts
//! (#250).
//!
//!     cargo run --release -p ducy-gto --example sixmax_preflop -- \
//!         --equity preflop-equity.bin --bb 100 --iters 20000000 \
//!         --checkpoint sixmax-100.bin [--every 1000000] [--grids]
//!
//! `--equity` is the file `sixmax_equity` writes; `--realize FILE` the
//! realization `sixmax_realize` fits for this depth (default: in position
//! 1.08, out of position 0.9, for every hand). With `--checkpoint`, a
//! run resumes from the file if it exists and saves to it every `--every`
//! iterations. Every report prints how much each position opens first in
//! and how the big blind defends against it; `--grids` also prints each
//! position's opening grid at the end.
//!
//! `--field FILE` (counts from `sixmax_field`) solves against that field
//! instead of for an equilibrium: the other five seats play the field's
//! recorded strategy, smoothed toward `--prior` (an equilibrium solve's
//! checkpoint, for the same tree) with `--prior-weight` decisions' worth
//! (default 20). `--progress FILE` writes a small JSON status file at every
//! report.

use std::time::Instant;

use ducy_gto::sixmax::{
    bot::PreflopStrategy,
    chart::{Charts, SpotChart},
    equity::PreflopEquity,
    field::FieldCounts,
    preflop::{POSITIONS, PreflopGame},
    realize::Realization,
    solver::PreflopSolver,
};

fn main() {
    let mut equity_path = String::from("preflop-equity.bin");
    let mut bb = 100u32;
    let mut iters = 10_000_000u64;
    let mut every = 1_000_000u64;
    let mut checkpoint: Option<String> = None;
    let mut seed = 1u64;
    let mut grids = false;
    let mut realize_path: Option<String> = None;
    let mut field_path: Option<String> = None;
    let mut prior_path: Option<String> = None;
    let mut prior_weight = 20f32;
    let mut progress: Option<String> = None;
    let mut threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut v = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--equity" => equity_path = v(),
            "--bb" => bb = v().parse().expect("--bb"),
            "--iters" => iters = v().replace('_', "").parse().expect("--iters"),
            "--every" => every = v().replace('_', "").parse().expect("--every"),
            "--checkpoint" => checkpoint = Some(v()),
            "--seed" => seed = v().parse().expect("--seed"),
            "--threads" => threads = v().parse().expect("--threads"),
            "--grids" => grids = true,
            "--realize" => realize_path = Some(v()),
            "--field" => field_path = Some(v()),
            "--prior" => prior_path = Some(v()),
            "--prior-weight" => prior_weight = v().parse().expect("--prior-weight"),
            "--progress" => progress = Some(v()),
            f => panic!("unknown option {f}"),
        }
    }
    let eq = PreflopEquity::from_bytes(&std::fs::read(&equity_path).expect("read --equity"))
        .expect("equity table");
    let game = match &realize_path {
        Some(path) => PreflopGame::with_realization(
            bb,
            Realization::from_bytes(&std::fs::read(path).expect("read --realize"))
                .expect("realization table"),
        ),
        None => PreflopGame::new(bb),
    };
    let field = field_path.as_ref().map(|path| {
        let counts = FieldCounts::from_bytes(&game, &std::fs::read(path).expect("read --field"))
            .expect("field counts for this tree");
        let prior_solver = PreflopSolver::new(&game, &eq, 1);
        prior_solver
            .load(
                &std::fs::read(prior_path.as_ref().expect("--field needs --prior"))
                    .expect("read --prior"),
            )
            .expect("--prior: a solve for this tree");
        let prior = PreflopStrategy::from_solver(&game, &prior_solver);
        println!(
            "against the field: {} decisions recorded, prior weight {prior_weight}",
            counts.total()
        );
        counts.strategy(&prior, prior_weight)
    });
    let solver = match &field {
        Some(f) => PreflopSolver::against_field(&game, &eq, f, seed),
        None => PreflopSolver::new(&game, &eq, seed),
    };
    if let Some(path) = &checkpoint {
        if let Ok(bytes) = std::fs::read(path) {
            solver.load(&bytes).expect("checkpoint");
            println!("resumed at {} iterations", solver.iterations());
        }
    }
    println!(
        "{bb} BB deep: {} decisions, {} threads",
        game.decisions, threads
    );
    let t = Instant::now();
    let target = solver.iterations() + iters;
    while solver.iterations() < target {
        let step = every.min(target - solver.iterations());
        let before = solver.iterations();
        let lap = Instant::now();
        solver.run(step, threads);
        if let Some(path) = &checkpoint {
            std::fs::write(path, solver.to_bytes()).expect("write --checkpoint");
        }
        println!(
            "\n{} iterations ({:.0}/s, {:.0}s in all)",
            solver.iterations(),
            (solver.iterations() - before) as f64 / lap.elapsed().as_secs_f64(),
            t.elapsed().as_secs_f64()
        );
        print!("{}", Charts::read(&game, &solver).summary());
        if let Some(path) = &progress {
            let rate = (solver.iterations() - before) as f64 / lap.elapsed().as_secs_f64();
            let left = (target - solver.iterations()) as f64 / rate / 60.0;
            let done = iters - (target - solver.iterations());
            let label = if field.is_some() {
                "6-max: solving against the field"
            } else {
                "6-max: preflop solve"
            };
            let json = format!(
                "{{\"label\": \"{label}\", \"done\": {done}, \"total\": {iters}, \"unit\": \"iterations\", \"etaMin\": {left:.0}}}\n"
            );
            let _ = std::fs::write(path, json);
        }
    }
    if grids {
        let charts = Charts::read(&game, &solver);
        for (s, open) in charts.opens.iter().enumerate() {
            println!("\n{} opens first in (%):", POSITIONS[s]);
            print!("{}", open.grid(SpotChart::raise));
        }
    }
}
