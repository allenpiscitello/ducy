//! Writes Ducybot's six-handed preflop charts at one depth as JSON, for the
//! site's chart viewer: every chart spot (`sixmax::chart::spots`), with each
//! hand's mix of fold, call, raise and all-in.
//!
//!     cargo run --release -p ducy-gto --example sixmax_charts -- \
//!         --solve sixmax-100.bin --out charts-100.json
//!
//! `--solve` is a compact strategy (`sixmax_export`) or, with `--equity`
//! and `--realize`, a solve's checkpoint (`--bb` then gives its depth).
//!
//! The JSON: {"format": "6max", "bb", "source": "ducybot", "actions":
//! ["fold", "call", "raise", "allin"], "spots": [{"id", "kind", "hero",
//! "villain", "overall": [4 percentages over all hands, by combos],
//! "cells": [169 × [4 percentages]]}]}, cells in chart order (rows from
//! aces down, pairs on the diagonal, suited above it, offsuit below).

use ducy_gto::holdem::chart::combos;
use ducy_gto::sixmax::{
    bot::PreflopStrategy,
    chart::{act_name, grid_classes, spots},
    equity::PreflopEquity,
    preflop::{Node, POSITIONS, PreflopGame},
    realize::Realization,
    solver::PreflopSolver,
};

const ACTIONS: [&str; 4] = ["fold", "call", "raise", "allin"];

fn main() {
    let mut solve_path = String::new();
    let mut equity_path: Option<String> = None;
    let mut realize_path: Option<String> = None;
    let mut bb = 100u32;
    let mut out = String::from("charts.json");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut v = || args.next().unwrap_or_else(|| panic!("{a} needs a value"));
        match a.as_str() {
            "--solve" => solve_path = v(),
            "--equity" => equity_path = Some(v()),
            "--realize" => realize_path = Some(v()),
            "--bb" => bb = v().parse().expect("--bb"),
            "--out" => out = v(),
            _ => panic!("unknown argument {a}"),
        }
    }
    let bytes = std::fs::read(&solve_path).expect("read --solve");
    let strategy = PreflopStrategy::from_compact(&bytes).unwrap_or_else(|_| {
        let eq = PreflopEquity::from_bytes(
            &std::fs::read(equity_path.as_ref().expect("a checkpoint needs --equity"))
                .expect("read --equity"),
        )
        .expect("equity table");
        let realize = realize_path
            .as_ref()
            .map_or_else(Realization::default, |p| {
                Realization::from_bytes(&std::fs::read(p).expect("read --realize"))
                    .expect("realization table")
            });
        let game = PreflopGame::with_realization(bb, realize);
        let solver = PreflopSolver::new(&game, &eq, 1);
        solver.load(&bytes).expect("a solve for this game");
        PreflopStrategy::from_solver(&game, &solver)
    });
    let game = strategy.game();
    let depth = game.stack / 2;
    let order = grid_classes();
    let mut json = format!(
        "{{\"format\": \"6max\", \"bb\": {depth}, \"source\": \"ducybot\", \"actions\": [\"fold\", \"call\", \"raise\", \"allin\"], \"spots\": ["
    );
    let mut first = true;
    for spot in spots(game) {
        let Node::Decision { id, actions, .. } = &game.nodes[spot.node] else {
            continue;
        };
        // Each class's mix, summed into the four chart actions.
        let mix = |class: usize| -> Option<[f64; 4]> {
            let probs = strategy.probs(*id, class);
            if probs.is_empty() {
                return None;
            }
            let mut m = [0.0; 4];
            for ((a, _), p) in actions.iter().zip(probs) {
                m[ACTIONS
                    .iter()
                    .position(|n| *n == act_name(a))
                    .expect("action")] += *p as f64;
            }
            Some(m)
        };
        if mix(0).is_none() {
            continue; // left out of a compact strategy
        }
        let mut overall = [0.0; 4];
        let mut cells = Vec::with_capacity(order.len());
        for &class in &order {
            let m = mix(class).expect("every class");
            for (o, x) in overall.iter_mut().zip(m) {
                *o += x * combos(class) / 1326.0;
            }
            cells.push(format!(
                "[{}]",
                m.iter()
                    .map(|x| format!("{:.0}", 100.0 * x))
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        let villain = spot
            .villain
            .map_or("null".to_string(), |v| format!("\"{}\"", POSITIONS[v]));
        json.push_str(&format!(
            "{}\n  {{\"id\": \"{}\", \"kind\": \"{}\", \"hero\": \"{}\", \"villain\": {villain}, \"overall\": [{}], \"cells\": [{}]}}",
            if first { "" } else { "," },
            spot.id,
            spot.kind,
            POSITIONS[spot.hero],
            overall.iter().map(|x| format!("{:.1}", 100.0 * x)).collect::<Vec<_>>().join(", "),
            cells.join(",")
        ));
        first = false;
    }
    json.push_str("\n]}\n");
    std::fs::write(&out, json).expect("write --out");
    println!("{depth} BB charts in {out}");
}
