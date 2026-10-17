//! Writes Ducybot's heads-up preflop charts at one depth as JSON, for the
//! site's chart viewer, in the same form as `sixmax_charts`: each spot of the
//! heads-up preflop tree with every hand's mix of fold, call, raise and
//! all-in.
//!
//!     cargo run --release -p ducy-gto --example hu_charts -- \
//!         --cards cards.bin --blueprint blueprint-50.bin --bb 50 --out hu-50.json
//!
//! The spots: the button first in, the big blind facing the open (the
//! smallest raise), the button facing the 3-bet, the big blind facing the
//! 4-bet, the big blind after the button limps, and the button after limping
//! and being raised. A raise to the whole stack counts as all-in.

use std::path::PathBuf;

use ducy_gto::holdem::{
    abstraction::CardAbstraction,
    blueprint::Blueprint,
    chart::{class_of, combos},
    depths::depth_config,
    hunl::{Hunl, HunlAction},
};

/// A chart spot: id, kind, hero, villain, and the path to it from the root.
type Spot = (
    &'static str,
    &'static str,
    &'static str,
    Option<&'static str>,
    &'static [&'static str],
);

/// The 169 classes in chart order: rows from aces down, pairs on the
/// diagonal, suited above it, offsuit below.
fn grid_classes() -> Vec<usize> {
    let mut out = Vec::with_capacity(169);
    for row in (0..13u8).rev() {
        for col in (0..13u8).rev() {
            out.push(if row == col {
                class_of(row, col, false)
            } else {
                class_of(row.max(col), row.min(col), col < row)
            });
        }
    }
    out
}

/// The node reached from the root by `path`: "call" takes the call (or
/// check), "raise" the smallest raise.
fn walk(game: &Hunl, path: &[&str]) -> Option<u32> {
    let mut node = 0u32;
    for step in path {
        let n = &game.tree.nodes[node as usize];
        let i = match *step {
            "call" => n
                .actions
                .iter()
                .position(|a| matches!(a, HunlAction::Call | HunlAction::Check))?,
            _ => {
                n.actions
                    .iter()
                    .enumerate()
                    .filter_map(|(i, a)| match a {
                        HunlAction::Raise(to) | HunlAction::Bet(to) => Some((i, *to)),
                        _ => None,
                    })
                    .min_by_key(|&(_, to)| to)?
                    .0
            }
        };
        node = n.children[i];
    }
    (!game.tree.nodes[node as usize].actions.is_empty()).then_some(node)
}

fn main() {
    let mut cards_path = PathBuf::new();
    let mut blueprint_path = PathBuf::new();
    let mut bb = 100u64;
    let mut out = String::from("hu.json");
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--cards" => cards_path = value().into(),
            "--blueprint" => blueprint_path = value().into(),
            "--bb" => bb = value().parse().expect("--bb"),
            "--out" => out = value(),
            f => panic!("unknown option {f}"),
        }
    }
    let cards = CardAbstraction::load(&std::fs::read(&cards_path).expect("read --cards"))
        .expect("a card abstraction");
    let config = depth_config(bb);
    let stack = config.stack;
    let game = Hunl::new(config, Some(&cards));
    let blueprint = Blueprint::load(
        &std::fs::read(&blueprint_path).expect("read --blueprint"),
        &game,
        &cards,
    )
    .expect("a blueprint for this game and depth");

    let spots: [Spot; 6] = [
        ("hu-open-BTN", "open", "BTN", None, &[]),
        (
            "hu-vs-open-BB-BTN",
            "vs-open",
            "BB",
            Some("BTN"),
            &["raise"],
        ),
        (
            "hu-vs-3bet-BTN-BB",
            "vs-3bet",
            "BTN",
            Some("BB"),
            &["raise", "raise"],
        ),
        (
            "hu-vs-4bet-BB-BTN",
            "vs-4bet",
            "BB",
            Some("BTN"),
            &["raise", "raise", "raise"],
        ),
        ("hu-vs-limp-BB-BTN", "vs-limp", "BB", Some("BTN"), &["call"]),
        (
            "hu-vs-iso-BTN-BB",
            "vs-iso",
            "BTN",
            Some("BB"),
            &["call", "raise"],
        ),
    ];
    let order = grid_classes();
    let mut parts = Vec::new();
    for (id, kind, hero, villain, path) in spots {
        let Some(node) = walk(&game, path) else {
            eprintln!("{id}: not in this tree");
            continue;
        };
        let actions = &game.tree.nodes[node as usize].actions;
        // Each action as fold 0, call 1, raise 2, all-in 3.
        let slot: Vec<usize> = actions
            .iter()
            .map(|a| match a {
                HunlAction::Fold => 0,
                HunlAction::Check | HunlAction::Call => 1,
                HunlAction::Bet(to) | HunlAction::Raise(to) if *to >= stack => 3,
                HunlAction::Bet(_) | HunlAction::Raise(_) => 2,
            })
            .collect();
        let mix = |class: usize| {
            let mut m = [0.0f64; 4];
            for (s, p) in slot.iter().zip(blueprint.probs(node, class as u16)) {
                m[*s] += p;
            }
            m
        };
        let mut overall = [0.0; 4];
        let cells: Vec<String> = order
            .iter()
            .map(|&c| {
                let m = mix(c);
                for (o, x) in overall.iter_mut().zip(m) {
                    *o += x * combos(c) / 1326.0;
                }
                format!(
                    "[{}]",
                    m.iter()
                        .map(|x| format!("{:.0}", 100.0 * x))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            })
            .collect();
        let villain = villain.map_or("null".to_string(), |v| format!("\"{v}\""));
        parts.push(format!(
            "\n  {{\"id\": \"{id}\", \"kind\": \"{kind}\", \"hero\": \"{hero}\", \"villain\": {villain}, \"overall\": [{}], \"cells\": [{}]}}",
            overall.iter().map(|x| format!("{:.1}", 100.0 * x)).collect::<Vec<_>>().join(", "),
            cells.join(",")
        ));
    }
    let json = format!(
        "{{\"format\": \"hu\", \"bb\": {bb}, \"source\": \"ducybot\", \"actions\": [\"fold\", \"call\", \"raise\", \"allin\"], \"spots\": [{}\n]}}\n",
        parts.join(",")
    );
    std::fs::write(&out, json).expect("write --out");
    println!("heads-up {bb} BB charts in {out}");
}
