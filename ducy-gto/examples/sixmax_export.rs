//! Writes a six-handed preflop solve in its compact form, for the web page
//! (`PreflopStrategy::to_compact`): only the decisions that come up, each
//! probability in a byte.
//!
//!     cargo run --release -p ducy-gto --example sixmax_export -- \
//!         --equity preflop-equity.bin --realize realize-100.bin --bb 100 \
//!         --solve solve-field-w20.bin --field field-100.bin \
//!         [--cover 0.999] --out sixmax-100.bin
//!
//! Which decisions come up is read from `--field` (counts from
//! `sixmax_field`): the most visited ones are kept until they cover
//! `--cover` of every decision recorded there. The rest are left to the
//! bot's fallback.

use ducy_gto::holdem::iso::NUM_PREFLOP_CLASSES;
use ducy_gto::sixmax::{
    bot::PreflopStrategy,
    equity::PreflopEquity,
    field::FieldCounts,
    preflop::{Node, PreflopGame},
    realize::Realization,
    solver::PreflopSolver,
};

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

    // Decisions by how often the field reached them, most first.
    let mut visits: Vec<(u64, u32)> = game
        .nodes
        .iter()
        .filter_map(|n| match n {
            Node::Decision { id, .. } => Some(*id),
            _ => None,
        })
        .map(|id| {
            (
                (0..NUM_PREFLOP_CLASSES)
                    .map(|c| field.counts(id, c).iter().map(|&x| x as u64).sum::<u64>())
                    .sum(),
                id,
            )
        })
        .collect();
    visits.sort_unstable_by(|a, b| b.cmp(a));
    let total: u64 = visits.iter().map(|v| v.0).sum();
    let mut keep = vec![false; game.decisions as usize];
    let mut covered = 0u64;
    let mut kept = 0usize;
    for (n, id) in &visits {
        if *n == 0 || covered as f64 >= cover * total as f64 {
            break;
        }
        keep[*id as usize] = true;
        covered += n;
        kept += 1;
    }
    let bytes = strategy.to_compact(|id| keep[id as usize]);
    std::fs::write(&out, &bytes).expect("write --out");
    // It reads back.
    let back = PreflopStrategy::from_compact(&bytes).expect("reads back");
    assert_eq!(back.game().decisions, game.decisions);
    println!(
        "kept {kept} of {} decisions, covering {:.3}% of the {total} the field made; {} bytes in {out}",
        game.decisions,
        100.0 * covered as f64 / total.max(1) as f64,
        bytes.len()
    );
}
