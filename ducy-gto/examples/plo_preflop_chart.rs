//! Preflop charts from a heads-up PLO blueprint, for the chart course. PLO
//! hands don't fit a 13×13 grid, so hands are grouped by shape: what they
//! pair (aces, kings, …, two pair, unpaired rundowns, …) against how their
//! suits fall (double-suited, single-suited, three or more of a suit). For
//! each spot (the button's first move, the big blind against a raise or a
//! limp, the button against a 3-bet) every cell gets how often its hands
//! fold, call or check, and raise, and its share of all hands. A sample of
//! hands with their own frequencies is included for quizzes.
//!
//!     cargo run --release -p ducy-gto --example plo_preflop_chart -- \
//!         --cards cards.bin --blueprint blueprint.bin [--bb 100] \
//!         [--hands 300000] [--quiz 1500] > plo5.json

use ducy_gto::{
    Rng,
    holdem::{
        blueprint::Blueprint,
        cards::{Card, rank, suit, to_string},
        hunl::{BettingTree, HunlAction, HunlConfig},
    },
    omaha::{
        abstraction::{PloAbstraction, random_spot},
        bot::load_blueprint,
    },
};

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

/// Fold, call or check, raise: index 0, 1 or 2.
fn group(a: &HunlAction) -> usize {
    match a {
        HunlAction::Fold => 0,
        HunlAction::Check | HunlAction::Call => 1,
        HunlAction::Bet(_) | HunlAction::Raise(_) => 2,
    }
}

/// Four distinct ranks within a five-rank window (the ace plays low too).
fn rundown(ranks: &[u8]) -> bool {
    let mut have = [false; 14];
    for &r in ranks {
        have[r as usize + 1] = true;
        if r == 12 {
            have[0] = true;
        }
    }
    (0..10).any(|lo| (lo..lo + 5).filter(|&i| have[i]).count() >= 4)
}

fn row(hole: &[Card]) -> usize {
    let mut count = [0u8; 13];
    for &c in hole {
        count[rank(c) as usize] += 1;
    }
    let pairs: Vec<usize> = (0..13).rev().filter(|&r| count[r] >= 2).collect();
    if count.iter().any(|&n| n >= 3) {
        return 5;
    }
    match pairs.len() {
        2.. => 4,
        1 => match pairs[0] {
            12 => 0,
            11 => 1,
            9 | 10 => 2,
            _ => 3,
        },
        _ => {
            let ranks: Vec<u8> = hole.iter().map(|&c| rank(c)).collect();
            if rundown(&ranks) {
                6
            } else if count[12] > 0 {
                7
            } else {
                8
            }
        }
    }
}

fn col(hole: &[Card]) -> usize {
    let mut by_suit = [0u8; 4];
    for &c in hole {
        by_suit[suit(c) as usize] += 1;
    }
    let mut s = by_suit;
    s.sort_unstable_by(|a, b| b.cmp(a));
    if s[0] >= 3 && s[1] < 2 {
        2
    } else if s[1] >= 2 {
        0
    } else {
        1
    }
}

/// The node reached by `path` ("call" or "raise"); a raise is the size the
/// player before took most often over `sample`.
fn walk(tree: &BettingTree, blueprint: &Blueprint, buckets: &[u16], path: &[&str]) -> Option<u32> {
    let mut node = 0u32;
    for step in path {
        let n = &tree.nodes[node as usize];
        let i = if *step == "call" {
            n.actions
                .iter()
                .position(|a| matches!(a, HunlAction::Call | HunlAction::Check))?
        } else {
            let mut used = vec![0.0; n.actions.len()];
            for &b in buckets {
                for (u, p) in used.iter_mut().zip(blueprint.probs(node, b)) {
                    *u += p;
                }
            }
            (0..n.actions.len())
                .filter(|&i| group(&n.actions[i]) == 2)
                .max_by(|&a, &b| used[a].total_cmp(&used[b]))?
        };
        node = tree.nodes[node as usize].children[i];
    }
    (!tree.nodes[node as usize].actions.is_empty()).then_some(node)
}

fn grouped(tree: &BettingTree, blueprint: &Blueprint, node: u32, bucket: u16) -> [f64; 3] {
    let mut g = [0.0; 3];
    for (a, p) in tree.nodes[node as usize]
        .actions
        .iter()
        .zip(blueprint.probs(node, bucket))
    {
        g[group(a)] += p;
    }
    g
}

fn main() {
    let (mut cards_path, mut blueprint_path) = (String::new(), String::new());
    let (mut bb, mut hands, mut quiz) = (100u64, 300_000usize, 1500usize);
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut v = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--cards" => cards_path = v(),
            "--blueprint" => blueprint_path = v(),
            "--bb" => bb = v().parse().expect("--bb"),
            "--hands" => hands = v().parse().expect("--hands"),
            "--quiz" => quiz = v().parse().expect("--quiz"),
            f => panic!("unknown option {f}"),
        }
    }
    let cards = PloAbstraction::load(&std::fs::read(&cards_path).expect("read --cards"))
        .expect("a PLO abstraction");
    let k = cards.config.hole_cards;
    let config = HunlConfig::pot_limit_omaha_lean(bb);
    let (blueprint, tree) = load_blueprint(
        &config,
        &cards,
        &std::fs::read(&blueprint_path).expect("read --blueprint"),
    )
    .expect("a blueprint for these cards and this game");

    let mut rng = Rng::new(7);
    let deal: Vec<(Vec<Card>, u16)> = (0..hands)
        .map(|_| {
            let (h, _) = random_spot(k, 0, &mut rng);
            let b = cards.preflop_bucket(&h);
            (h, b)
        })
        .collect();
    let buckets: Vec<u16> = deal.iter().take(20_000).map(|d| d.1).collect();

    let spots = [
        ("open", "Button first in", vec![]),
        ("vs-raise", "Big blind against a raise", vec!["raise"]),
        ("vs-limp", "Big blind against a limp", vec!["call"]),
        ("vs-3bet", "Button against a 3-bet", vec!["raise", "raise"]),
    ];
    let mut out_spots = Vec::new();
    let mut nodes = Vec::new();
    for (id, label, path) in &spots {
        let Some(node) = walk(&tree, &blueprint, &buckets, path) else {
            eprintln!("no {id} spot in this tree");
            continue;
        };
        nodes.push((*id, node));
        let mut sum = vec![[0.0f64; 3]; ROWS.len() * COLS.len()];
        let mut n = vec![0usize; ROWS.len() * COLS.len()];
        let mut overall = [0.0f64; 3];
        for (h, b) in &deal {
            let g = grouped(&tree, &blueprint, node, *b);
            let c = row(h) * COLS.len() + col(h);
            n[c] += 1;
            for i in 0..3 {
                sum[c][i] += g[i];
                overall[i] += g[i];
            }
        }
        let pct = |x: f64, d: usize| (1000.0 * x / d.max(1) as f64).round() / 10.0;
        let cells: Vec<String> = (0..sum.len())
            .map(|c| {
                format!(
                    "{{\"mix\":[{},{},{}],\"share\":{}}}",
                    pct(sum[c][0], n[c]),
                    pct(sum[c][1], n[c]),
                    pct(sum[c][2], n[c]),
                    pct(n[c] as f64, deal.len())
                )
            })
            .collect();
        let checks = tree.nodes[node as usize]
            .actions
            .iter()
            .any(|a| matches!(a, HunlAction::Check));
        out_spots.push(format!(
            "{{\"id\":\"{id}\",\"label\":\"{label}\",\"actions\":[\"Fold\",\"{}\",\"{}\"],\"overall\":[{},{},{}],\"cells\":[{}]}}",
            if checks { "Check" } else { "Call" },
            if path.is_empty() || *id == "vs-limp" { "Raise" } else if path.len() == 1 { "3-bet" } else { "4-bet" },
            pct(overall[0], deal.len()),
            pct(overall[1], deal.len()),
            pct(overall[2], deal.len()),
            cells.join(",")
        ));
    }
    let quiz_hands: Vec<String> = deal
        .iter()
        .take(quiz)
        .map(|(h, b)| {
            let mut sorted = h.clone();
            sorted.sort_unstable_by(|a, b| rank(*b).cmp(&rank(*a)).then(suit(*a).cmp(&suit(*b))));
            let name: Vec<String> = sorted.iter().map(|&c| to_string(c)).collect();
            let mixes: Vec<String> = nodes
                .iter()
                .map(|(id, node)| {
                    let g = grouped(&tree, &blueprint, *node, *b);
                    format!(
                        "\"{id}\":[{},{},{}]",
                        (g[0] * 100.0).round(),
                        (g[1] * 100.0).round(),
                        (g[2] * 100.0).round()
                    )
                })
                .collect();
            format!(
                "{{\"h\":\"{}\",\"r\":{},\"c\":{},{}}}",
                name.join(" "),
                row(h),
                col(h),
                mixes.join(",")
            )
        })
        .collect();
    println!(
        "{{\"game\":\"PLO{k}\",\"bb\":{bb},\"rows\":{:?},\"cols\":{:?},\"spots\":[{}],\"hands\":[{}]}}",
        ROWS,
        COLS,
        out_spots.join(","),
        quiz_hands.join(",")
    );
}
