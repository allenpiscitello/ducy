//! Trains six-handed pot-limit Omaha preflop (#290) over a PLO
//! abstraction's preflop buckets, and reports how each position plays.
//!
//!     cargo run --release -p ducy-gto --example sixmax_plo -- \
//!         --cards plo-cards.bin --equity plo-equity.bin --bb 100 \
//!         --iters 50000000 --checkpoint sixmax-plo-100.bin [--every 5000000]
//!
//! `--cards` is a PLO abstraction (`plo_abstraction`; four, five or six hole
//! cards). `--equity` is the bucket-vs-bucket equity table: built and saved
//! there first if the file doesn't exist (`--pool` hands per bucket,
//! default 200, and `--samples` deals per bucket pair, default 300).
//! `--realize FILE` is a realization table for these buckets (default: in
//! position 1.08, out of position 0.9). With `--checkpoint`, a run resumes
//! from the file if it exists and saves to it every `--every` iterations.
//! Each report prints, per position, how often it folds, limps and raises
//! first in, and how the big blind answers the button's open.
//!
//! `--export FILE` writes the first-in play as JSON for the chart course:
//! per position, the overall fold / limp / raise rates and, over 200,000
//! random hands grouped by shape (as `plo_preflop_chart` groups them), each
//! shape's rates and share of hands.

use std::time::Instant;

use ducy_gto::{
    Rng,
    holdem::{
        cards::{Card, rank, suit},
        hunl::Buckets,
    },
    omaha::{abstraction::PloAbstraction, showdown::draw},
    sixmax::{
        equity::PreflopEquity,
        plo::{PloDealer, bucket_equity},
        preflop::{Act, Node, POSITIONS, PreflopGame},
        realize::Realization,
        solver::PreflopSolver,
    },
};

/// The decision where `seat` acts after everyone before it folds, then (if
/// `then` is set) the node after it raises and folds come round to `then`.
fn first_in(game: &PreflopGame, seat: usize, then: Option<usize>) -> Option<usize> {
    let mut node = game.root();
    let mut raised = false;
    loop {
        let Node::Decision {
            seat: s, actions, ..
        } = &game.nodes[node]
        else {
            return None;
        };
        let want = if raised { then? } else { seat };
        if *s == want {
            if raised || then.is_none() {
                return Some(node);
            }
            node = actions
                .iter()
                .find(|(a, _)| matches!(a, Act::Raise(_) | Act::AllIn))?
                .1 as usize;
            raised = true;
            continue;
        }
        node = actions.iter().find(|(a, _)| *a == Act::Fold)?.1 as usize;
    }
}

const ROWS: [&str; 9] = [
    "Aces",
    "Kings",
    "Queens or jacks",
    "Tens to deuces",
    "Two pair",
    "Three of a kind",
    "Rundown",
    "Ace high",
    "Other",
];
const COLS: [&str; 3] = ["Double-suited", "Single-suited", "Three or more of a suit"];

/// A hand's shape row: what it pairs, or for unpaired hands rundown / ace
/// high / other (four distinct ranks within five, the ace playing low too).
fn shape_row(hole: &[Card]) -> usize {
    let mut count = [0u8; 13];
    for &c in hole {
        count[rank(c) as usize] += 1;
    }
    if count.iter().any(|&n| n >= 3) {
        return 5;
    }
    let pairs: Vec<usize> = (0..13).rev().filter(|&r| count[r] == 2).collect();
    match pairs.len() {
        2.. => 4,
        1 => match pairs[0] {
            12 => 0,
            11 => 1,
            9 | 10 => 2,
            _ => 3,
        },
        _ => {
            let mut have = [false; 14];
            for &c in hole {
                have[rank(c) as usize + 1] = true;
            }
            have[0] = have[13];
            if (0..10).any(|lo| (lo..lo + 5).filter(|&i| have[i]).count() >= 4) {
                6
            } else if count[12] > 0 {
                7
            } else {
                8
            }
        }
    }
}

/// A hand's suit column: double-suited, single-suited, or three or more of
/// one suit (and no second pair of suits).
fn shape_col(hole: &[Card]) -> usize {
    let mut by_suit = [0u8; 4];
    for &c in hole {
        by_suit[suit(c) as usize] += 1;
    }
    by_suit.sort_unstable_by(|a, b| b.cmp(a));
    if by_suit[0] >= 3 && by_suit[1] < 2 {
        2
    } else if by_suit[1] >= 2 {
        0
    } else {
        1
    }
}

/// Fold, limp and raise at `node` for one bucket.
fn grouped(game: &PreflopGame, solver: &PreflopSolver, node: usize, bucket: usize) -> [f64; 3] {
    let Node::Decision { id, actions, .. } = &game.nodes[node] else {
        return [0.0; 3];
    };
    let mut out = [0.0; 3];
    for (p, (a, _)) in solver.strategy(*id, bucket).iter().zip(actions) {
        let k = match a {
            Act::Fold => 0,
            Act::Call => 1,
            Act::Raise(_) | Act::AllIn => 2,
        };
        out[k] += p;
    }
    out
}

/// First-in play by position as JSON (see the module docs).
fn export(cards: &PloAbstraction, game: &PreflopGame, solver: &PreflopSolver, bb: u32) -> String {
    let k = cards.config.hole_cards;
    let hands = 200_000;
    let mut rng = Rng::new(11);
    let dealt: Vec<(usize, usize)> = (0..hands)
        .map(|_| {
            let mut used = 0u64;
            let h: Vec<Card> = (0..k).map(|_| draw(&mut used, &mut rng)).collect();
            (
                cards.preflop_bucket(&h) as usize,
                shape_row(&h) * COLS.len() + shape_col(&h),
            )
        })
        .collect();
    let pct = |x: f64| (1000.0 * x).round() / 10.0;
    let mut positions = Vec::new();
    for (seat, name) in POSITIONS.iter().enumerate().take(5) {
        let Some(node) = first_in(game, seat, None) else {
            continue;
        };
        let mut by_bucket = vec![None; cards.bucket_count(0)];
        let mut sum = vec![[0.0f64; 3]; ROWS.len() * COLS.len()];
        let mut n = vec![0usize; sum.len()];
        let mut all = [0.0f64; 3];
        for &(b, cell) in &dealt {
            let g = *by_bucket[b].get_or_insert_with(|| grouped(game, solver, node, b));
            n[cell] += 1;
            for i in 0..3 {
                sum[cell][i] += g[i];
                all[i] += g[i];
            }
        }
        let cells: Vec<String> = (0..sum.len())
            .map(|c| {
                let d = n[c].max(1) as f64;
                format!(
                    "{{\"mix\":[{},{},{}],\"share\":{}}}",
                    pct(sum[c][0] / d),
                    pct(sum[c][1] / d),
                    pct(sum[c][2] / d),
                    pct(n[c] as f64 / hands as f64)
                )
            })
            .collect();
        positions.push(format!(
            "{{\"pos\":\"{name}\",\"overall\":[{},{},{}],\"cells\":[{}]}}",
            pct(all[0] / hands as f64),
            pct(all[1] / hands as f64),
            pct(all[2] / hands as f64),
            cells.join(",")
        ));
    }
    format!(
        "{{\"game\":\"PLO{k}\",\"bb\":{bb},\"iterations\":{},\"rows\":{ROWS:?},\"cols\":{COLS:?},\"positions\":[{}]}}",
        solver.iterations(),
        positions.join(",")
    )
}

/// Fold, call (limp or check) and raise rates at `node`, over hands as dealt.
fn rates(game: &PreflopGame, solver: &PreflopSolver, node: usize, weight: &[f64]) -> [f64; 3] {
    let Node::Decision { id, actions, .. } = &game.nodes[node] else {
        return [0.0; 3];
    };
    let mut out = [0.0; 3];
    for (b, &w) in weight.iter().enumerate() {
        for (p, (a, _)) in solver.strategy(*id, b).iter().zip(actions) {
            let k = match a {
                Act::Fold => 0,
                Act::Call => 1,
                Act::Raise(_) | Act::AllIn => 2,
            };
            out[k] += w * p;
        }
    }
    out
}

fn main() {
    let mut cards_path = String::new();
    let mut equity_path = String::from("plo-equity.bin");
    let mut realize_path: Option<String> = None;
    let mut bb = 100u32;
    let mut iters = 10_000_000u64;
    let mut every = 1_000_000u64;
    let mut checkpoint: Option<String> = None;
    let mut seed = 1u64;
    let mut pool = 200usize;
    let mut samples = 300u32;
    let mut threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut export_path: Option<String> = None;
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut v = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--cards" => cards_path = v(),
            "--equity" => equity_path = v(),
            "--realize" => realize_path = Some(v()),
            "--bb" => bb = v().parse().expect("--bb"),
            "--iters" => iters = v().replace('_', "").parse().expect("--iters"),
            "--every" => every = v().replace('_', "").parse().expect("--every"),
            "--checkpoint" => checkpoint = Some(v()),
            "--seed" => seed = v().parse().expect("--seed"),
            "--pool" => pool = v().parse().expect("--pool"),
            "--samples" => samples = v().parse().expect("--samples"),
            "--threads" => threads = v().parse().expect("--threads"),
            "--export" => export_path = Some(v()),
            f => panic!("unknown option {f}"),
        }
    }
    let cards = PloAbstraction::load(&std::fs::read(&cards_path).expect("read --cards"))
        .expect("a PLO abstraction");
    let n = cards.bucket_count(0);
    let k = cards.config.hole_cards;
    let eq = match std::fs::read(&equity_path) {
        Ok(bytes) => PreflopEquity::from_bytes(&bytes).expect("--equity: an equity table"),
        Err(_) => {
            let t = Instant::now();
            println!(
                "building the PLO{k} equity table: {n} buckets, {pool} hands each, {samples} deals a pair"
            );
            let eq = bucket_equity(&cards, pool, samples, seed, threads);
            std::fs::write(&equity_path, eq.to_bytes()).expect("write --equity");
            println!("built in {:.0}s", t.elapsed().as_secs_f64());
            eq
        }
    };
    assert_eq!(eq.classes(), n, "--equity was built for these buckets");
    let realize = match &realize_path {
        Some(p) => Realization::from_bytes(&std::fs::read(p).expect("read --realize"))
            .expect("realization table"),
        None => Realization::uniform(n),
    };
    let game = PreflopGame::pot_limit(bb, realize);
    let dealer = PloDealer { cards: &cards };
    let deal = |rng: &mut Rng| dealer.deal(rng);
    // How often each bucket is dealt, for the reports.
    let mut weight = vec![0f64; n];
    let mut rng = Rng::new(seed ^ 77);
    let draws = 20_000;
    for _ in 0..draws {
        for b in dealer.deal(&mut rng) {
            weight[b] += 1.0 / (6 * draws) as f64;
        }
    }
    let solver = PreflopSolver::with_dealer(&game, &eq, &deal, seed);
    if let Some(path) = &checkpoint {
        if let Ok(bytes) = std::fs::read(path) {
            solver.load(&bytes).expect("checkpoint");
            println!("resumed at {} iterations", solver.iterations());
        }
    }
    println!(
        "PLO{k} six-handed, {bb} BB deep: {} decisions, {n} buckets, {threads} threads",
        game.decisions
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
        println!("first in      fold   limp  raise");
        for (seat, name) in POSITIONS.iter().enumerate().take(5) {
            if let Some(node) = first_in(&game, seat, None) {
                let r = rates(&game, &solver, node, &weight);
                println!(
                    "  {:<6} {:>6.1} {:>6.1} {:>6.1}",
                    name,
                    100.0 * r[0],
                    100.0 * r[1],
                    100.0 * r[2]
                );
            }
        }
        if let Some(node) = first_in(&game, 3, Some(5)) {
            let r = rates(&game, &solver, node, &weight);
            println!(
                "BB vs BTN open: fold {:.1}, call {:.1}, 3-bet {:.1}",
                100.0 * r[0],
                100.0 * r[1],
                100.0 * r[2]
            );
        }
    }
    if let Some(path) = &export_path {
        std::fs::write(path, export(&cards, &game, &solver, bb)).expect("write --export");
        println!("wrote {path}");
    }
}
