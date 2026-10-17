//! Writes a six-handed preflop solve in its compact form, for the web page
//! (`PreflopStrategy::to_compact`): only the decisions that come up, each
//! probability in a byte.
//!
//!     cargo run --release -p ducy-gto --example sixmax_export -- \
//!         --equity preflop-equity.bin --realize realize-100.bin --bb 100 \
//!         --solve solve-limps-100.bin --field field-100.bin \
//!         [--cover 0.999] --out sixmax-100.bin
//!
//! Two ways of coming up are kept:
//! - **Against the field:** the decisions `--field` (counts from
//!   `sixmax_field`, how the Regulars play) reaches most, until they cover
//!   `--cover` of every decision recorded there.
//! - **Against itself:** the decisions most likely to be reached when every
//!   seat plays the solve (each hand class weighted by its combos), until
//!   they cover `--cover` of the chance of reaching any.
//!
//! The rest are left to the bot's fallback.

use ducy_gto::holdem::{chart::combos, iso::NUM_PREFLOP_CLASSES};
use ducy_gto::sixmax::{
    bot::PreflopStrategy,
    equity::PreflopEquity,
    field::FieldCounts,
    preflop::{Node, PreflopGame},
    realize::Realization,
    solver::PreflopSolver,
};

/// Marks the decisions with the most `weight`, most first, until they cover
/// `cover` of the total. Returns how many were added.
fn keep_top(weights: &[(f64, u32)], cover: f64, keep: &mut [bool]) -> usize {
    let mut sorted: Vec<_> = weights.iter().filter(|w| w.0 > 0.0).copied().collect();
    sorted.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
    let total: f64 = sorted.iter().map(|w| w.0).sum();
    let mut covered = 0.0;
    let mut added = 0;
    for (w, id) in sorted {
        if covered >= cover * total {
            break;
        }
        covered += w;
        if !keep[id as usize] {
            keep[id as usize] = true;
            added += 1;
        }
    }
    added
}

fn main() {
    let mut equity_path = String::from("preflop-equity.bin");
    let mut realize_path: Option<String> = None;
    let mut bb = 100u32;
    let mut solve_path = String::new();
    let mut field_path = String::new();
    let mut cover = 0.999f64;
    let mut out = String::from("sixmax-100.bin");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut v = || args.next().unwrap_or_else(|| panic!("{a} needs a value"));
        match a.as_str() {
            "--equity" => equity_path = v(),
            "--realize" => realize_path = Some(v()),
            "--bb" => bb = v().parse().expect("--bb"),
            "--solve" => solve_path = v(),
            "--field" => field_path = v(),
            "--cover" => cover = v().parse().expect("--cover"),
            "--out" => out = v(),
            _ => panic!("unknown argument {a}"),
        }
    }
    let eq = PreflopEquity::from_bytes(&std::fs::read(&equity_path).expect("read --equity"))
        .expect("equity table");
    let realize = realize_path.map_or_else(Realization::default, |p| {
        Realization::from_bytes(&std::fs::read(p).expect("read --realize"))
            .expect("realization table")
    });
    let game = PreflopGame::with_realization(bb, realize);
    let solver = PreflopSolver::new(&game, &eq, 1);
    solver
        .load(&std::fs::read(&solve_path).expect("read --solve"))
        .expect("a solve for this game");
    let strategy = PreflopStrategy::from_solver(&game, &solver);
    let field = FieldCounts::from_bytes(&game, &std::fs::read(&field_path).expect("read --field"))
        .expect("field counts");

    // Against the field: decisions by how often the Regulars reached them.
    let visits: Vec<(f64, u32)> = game
        .nodes
        .iter()
        .filter_map(|n| match n {
            Node::Decision { id, .. } => Some(*id),
            _ => None,
        })
        .map(|id| {
            let n: u64 = (0..NUM_PREFLOP_CLASSES)
                .map(|c| field.counts(id, c).iter().map(|&x| x as u64).sum::<u64>())
                .sum();
            (n as f64, id)
        })
        .collect();

    // Against itself: the chance of reaching each decision when every seat
    // plays the solve, each action's chance the combo-weighted average over
    // hand classes (cards held by others aren't removed).
    let weight: Vec<f64> = (0..NUM_PREFLOP_CLASSES).map(combos).collect();
    let all: f64 = weight.iter().sum();
    let mut reach = vec![0.0f64; game.nodes.len()];
    reach[game.root()] = 1.0;
    let mut stack = vec![game.root()];
    let mut reached = Vec::new();
    while let Some(i) = stack.pop() {
        let Node::Decision { id, actions, .. } = &game.nodes[i] else {
            continue;
        };
        reached.push((reach[i], *id));
        for (a, (_, child)) in actions.iter().enumerate() {
            let p: f64 = (0..NUM_PREFLOP_CLASSES)
                .map(|c| weight[c] * strategy.probs(*id, c)[a] as f64)
                .sum::<f64>()
                / all;
            let child = *child as usize;
            reach[child] += reach[i] * p;
            if p > 0.0 {
                stack.push(child);
            }
        }
    }

    let mut keep = vec![false; game.decisions as usize];
    let from_field = keep_top(&visits, cover, &mut keep);
    let from_self = keep_top(&reached, cover, &mut keep);
    let bytes = strategy.to_compact(|id| keep[id as usize]);
    std::fs::write(&out, &bytes).expect("write --out");
    // It reads back.
    let back = PreflopStrategy::from_compact(&bytes).expect("reads back");
    assert_eq!(back.game().decisions, game.decisions);
    println!(
        "kept {} of {} decisions ({from_field} for {:.1}% of the field's, {from_self} more for {:.1}% of its own reach); {} bytes in {out}",
        from_field + from_self,
        game.decisions,
        100.0 * cover,
        100.0 * cover,
        bytes.len()
    );
}
