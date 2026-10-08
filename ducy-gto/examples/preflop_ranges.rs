//! Preflop ranges from a GTO blueprint, as range text: for each spot of the
//! heads-up preflop tree (the button's first move, the big blind's answer to
//! an open or a limp, and the 3-bets and 4-bets that follow), each starting
//! hand goes to the action it takes most often, with how often each action
//! is taken overall. ducy-web's GTO range presets come from this.
//!
//!     cargo run --release -p ducy-gto --example preflop_ranges -- \
//!         --cards cards.bin --blueprint blueprint.bin
//!
//! Prints one JSON object per spot: {spot, actions: [{action, overall,
//! range, hands, mixed}]}, where `overall` is the share of all 1,326 hands
//! the blueprint plays that way (mixing included), `range` the hands whose
//! most frequent action it is, and `mixed` those of them that take it less
//! than 75% of the time.

use std::path::PathBuf;

use ducy_gto::holdem::{
    abstraction::CardAbstraction,
    blueprint::Blueprint,
    chart::{class_name, class_of, combos},
    hunl::{Hunl, HunlAction, HunlConfig},
    iso::NUM_PREFLOP_CLASSES,
};

const RANKS: &[u8; 13] = b"23456789TJQKA";

/// The action groups shown: folding, calling or checking, raising (any size,
/// all-in included).
fn group(a: &HunlAction) -> &'static str {
    match a {
        HunlAction::Fold => "fold",
        HunlAction::Check | HunlAction::Call => "call",
        HunlAction::Bet(_) | HunlAction::Raise(_) => "raise",
    }
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

/// Range text for a set of classes, in the usual shorthand: "TT+", "A2s+",
/// "K9s+", "A5s-A2s", single hands otherwise.
fn range_text(classes: &[usize]) -> String {
    let has = |c: usize| classes.contains(&c);
    let r = |x: u8| RANKS[x as usize] as char;
    let mut out = Vec::new();
    // Pairs: "77+" for a run up to aces, single pairs otherwise.
    let pairs: Vec<u8> = (0..13u8)
        .rev()
        .filter(|&p| has(class_of(p, p, false)))
        .collect();
    let top_run = pairs
        .iter()
        .enumerate()
        .take_while(|&(i, &p)| p == 12 - i as u8)
        .count();
    let rest = if top_run >= 2 {
        let p = pairs[top_run - 1];
        out.push(format!("{}{}+", r(p), r(p)));
        top_run
    } else {
        0
    };
    for &p in &pairs[rest..] {
        out.push(format!("{}{}", r(p), r(p)));
    }
    // Suited, then offsuit: by high card, runs of kickers.
    for (suited, tag) in [(true, 's'), (false, 'o')] {
        for hi in (1..13u8).rev() {
            let kickers: Vec<u8> = (0..hi)
                .rev()
                .filter(|&lo| has(class_of(hi, lo, suited)))
                .collect();
            let mut i = 0;
            while i < kickers.len() {
                let mut j = i;
                while j + 1 < kickers.len() && kickers[j + 1] + 1 == kickers[j] {
                    j += 1;
                }
                let (top, bottom) = (kickers[i], kickers[j]);
                out.push(if top == hi - 1 && j > i {
                    format!("{}{}{tag}+", r(hi), r(bottom))
                } else if j == i {
                    format!("{}{}{tag}", r(hi), r(top))
                } else {
                    format!("{}{}{tag}-{}{}{tag}", r(hi), r(top), r(hi), r(bottom))
                });
                i = j + 1;
            }
        }
    }
    out.join(", ")
}

fn main() {
    let mut cards_path = PathBuf::new();
    let mut blueprint_path = PathBuf::new();
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--cards" => cards_path = value().into(),
            "--blueprint" => blueprint_path = value().into(),
            f => panic!("unknown option {f}"),
        }
    }
    let cards = CardAbstraction::load(&std::fs::read(&cards_path).expect("read --cards"))
        .expect("a card abstraction");
    let game = Hunl::new(HunlConfig::default(), Some(&cards));
    let blueprint = Blueprint::load(
        &std::fs::read(&blueprint_path).expect("read --blueprint"),
        &game,
        &cards,
    )
    .expect("a blueprint for this game");

    let spots: [(&str, &[&str]); 6] = [
        ("btn-first", &[]),
        ("bb-vs-open", &["raise"]),
        ("btn-vs-3bet", &["raise", "raise"]),
        ("bb-vs-4bet", &["raise", "raise", "raise"]),
        ("bb-vs-limp", &["call"]),
        ("btn-vs-iso", &["call", "raise"]),
    ];
    for (name, path) in spots {
        let Some(node) = walk(&game, path) else {
            eprintln!("{name}: not in this tree");
            continue;
        };
        let actions = &game.tree.nodes[node as usize].actions;
        let groups: Vec<&str> = {
            let mut g: Vec<&str> = actions.iter().map(group).collect();
            g.dedup();
            g
        };
        // Each class's frequency per group.
        let freq: Vec<Vec<f64>> = (0..NUM_PREFLOP_CLASSES)
            .map(|c| {
                let p = blueprint.probs(node, c as u16);
                groups
                    .iter()
                    .map(|g| {
                        actions
                            .iter()
                            .zip(&p)
                            .filter(|(a, _)| group(a) == *g)
                            .map(|(_, x)| x)
                            .sum()
                    })
                    .collect()
            })
            .collect();
        let out: Vec<String> = groups
            .iter()
            .enumerate()
            .map(|(k, g)| {
                let overall: f64 = (0..NUM_PREFLOP_CLASSES).map(|c| combos(c) * freq[c][k]).sum::<f64>() / 1326.0;
                let mine: Vec<usize> = (0..NUM_PREFLOP_CLASSES)
                    .filter(|&c| freq[c].iter().enumerate().all(|(j, &x)| j == k || freq[c][k] > x || (freq[c][k] == x && k < j)))
                    .collect();
                let mixed: Vec<String> = mine.iter().filter(|&&c| freq[c][k] < 0.75).map(|&c| class_name(c)).collect();
                let combos_in: f64 = mine.iter().map(|&c| combos(c)).sum();
                format!(
                    "{{\"action\":\"{g}\",\"overall\":{:.4},\"range_share\":{:.4},\"range\":\"{}\",\"mixed\":[{}]}}",
                    overall,
                    combos_in / 1326.0,
                    range_text(&mine),
                    mixed.iter().map(|m| format!("\"{m}\"")).collect::<Vec<_>>().join(",")
                )
            })
            .collect();
        println!("{{\"spot\":\"{name}\",\"actions\":[{}]}}", out.join(","));
    }
}
