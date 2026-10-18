//! A styled player's least exploitable game (#265, `sixmax::styled`): one seat
//! must play a target share of its hands (VPIP) while the five others adapt
//! to it; prints what the style costs.
//!
//!     cargo run --release -p ducy-gto --example sixmax_styled -- \
//!         --equity preflop-equity.bin --realize realize-100.bin --bb 100 \
//!         [--postflop postflop-100.bin] --vpip 0.35 \
//!         [--iters 200000000] [--every 10000000] [--rate 0.002] \
//!         [--checkpoint styled-100-35.bin] [--deals 2000000]
//!
//! Without `--vpip` the styled player has no price (`--lambda` sets a fixed
//! one): its VPIP is where it lands by itself. Each report prints the
//! iterations, the price and the styled player's VPIP; at the end, its
//! result per 100 hands against the adapting table (an all-GTO table
//! averages zero), and how it opens and how the big blind treats its opens.

use std::time::Instant;

use ducy_gto::{
    holdem::{chart::class_name, iso::NUM_PREFLOP_CLASSES},
    sixmax::{
        equity::PreflopEquity,
        preflop::{Act, Node, POSITIONS, PostflopTable, PreflopGame},
        realize::Realization,
        styled::StyledSolver,
    },
};

fn main() {
    let mut equity_path = String::new();
    let mut realize_path: Option<String> = None;
    let mut postflop_path: Option<String> = None;
    let mut bb = 100u32;
    let mut target: Option<f64> = None;
    let mut lambda: Option<f64> = None;
    let mut iters = 200_000_000u64;
    let mut every = 10_000_000u64;
    let mut rate = 0.002f64;
    let mut checkpoint: Option<String> = None;
    let mut deals = 2_000_000u64;
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut v = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--equity" => equity_path = v(),
            "--realize" => realize_path = Some(v()),
            "--postflop" => postflop_path = Some(v()),
            "--bb" => bb = v().parse().expect("--bb"),
            "--vpip" => target = Some(v().parse().expect("--vpip")),
            "--lambda" => lambda = Some(v().parse().expect("--lambda")),
            "--iters" => iters = v().replace('_', "").parse().expect("--iters"),
            "--every" => every = v().replace('_', "").parse().expect("--every"),
            "--rate" => rate = v().parse().expect("--rate"),
            "--checkpoint" => checkpoint = Some(v()),
            "--deals" => deals = v().replace('_', "").parse().expect("--deals"),
            f => panic!("unknown option {f}"),
        }
    }
    let eq = PreflopEquity::from_bytes(&std::fs::read(&equity_path).expect("read --equity"))
        .expect("equity table");
    let mut game = match &realize_path {
        Some(p) => PreflopGame::with_realization(
            bb,
            Realization::from_bytes(&std::fs::read(p).expect("read --realize"))
                .expect("realization"),
        ),
        None => PreflopGame::new(bb),
    };
    if let Some(p) = &postflop_path {
        game.postflop = PostflopTable::from_bytes(&std::fs::read(p).expect("read --postflop"))
            .expect("postflop tables");
    }
    let solver = StyledSolver::new(&game, &eq, 1);
    if let Some(p) = &checkpoint
        && let Ok(bytes) = std::fs::read(p)
    {
        solver
            .load(&bytes)
            .expect("a styled checkpoint for this game");
        println!(
            "resumed at {} iterations, price {:.3} BB",
            solver.iterations(),
            solver.lambda()
        );
    }
    if let Some(l) = lambda {
        solver.set_lambda(l);
    }
    let t0 = Instant::now();
    let target_text = target.map_or("none".to_string(), |t| format!("{:.0}%", 100.0 * t));
    println!("{bb} BB, VPIP target {target_text}, {threads} threads");
    while solver.iterations() < iters {
        let step = every.min(iters - solver.iterations());
        solver.run(step, threads);
        let vpip = match target {
            // The price moves less as training settles.
            Some(t) => solver.steer(t, rate),
            None => solver.steer(0.0, 0.0),
        };
        println!(
            "[{:>5.0}s] {:>11} iterations · price {:+.3} BB · VPIP {:.1}%",
            t0.elapsed().as_secs_f64(),
            solver.iterations(),
            solver.lambda(),
            100.0 * vpip
        );
        if let Some(p) = &checkpoint {
            std::fs::write(p, solver.to_bytes()).expect("write --checkpoint");
        }
    }
    let (ev, se, vpip) = solver.styled_ev(deals, 77, threads);
    println!(
        "styled player: VPIP {:.1}%, {:+.2} ± {:.2} bb/100 against the adapting table (GTO: 0)",
        100.0 * vpip,
        100.0 * ev,
        196.0 * se
    );
    // How the styled player opens first in, by position.
    let first_in = |pos: &str| {
        let id = format!("open-{pos}");
        ducy_gto::sixmax::chart::spots(&game)
            .into_iter()
            .find(|s| s.id == id)
    };
    let weight = |c: usize| {
        let n = class_name(c);
        if n.len() == 2 {
            6.0
        } else if n.ends_with('s') {
            4.0
        } else {
            12.0
        }
    };
    for pos in POSITIONS.iter().take(5) {
        let Some(spot) = first_in(pos) else { continue };
        let Node::Decision { id, actions, .. } = &game.nodes[spot.node] else {
            continue;
        };
        let fold = actions.iter().position(|(a, _)| *a == Act::Fold);
        let (mut played, mut total) = (0.0, 0.0);
        for c in 0..NUM_PREFLOP_CLASSES {
            let p = solver.strategy(*id, c, None);
            played += weight(c) * (1.0 - fold.map_or(0.0, |f| p[f]));
            total += weight(c);
        }
        println!("  {pos} first in: plays {:.1}%", 100.0 * played / total);
    }
}
