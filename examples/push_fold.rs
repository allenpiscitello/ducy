//! Prints heads-up push/fold equilibrium charts:
//! `cargo run --release --example push_fold [stack_bb] [ante_bb]`.
use ducy::preflop::{HandClass, PreflopEquityTable, solve_heads_up_push_fold};

fn main() {
    let mut args = std::env::args().skip(1);
    let stack: f64 = args.next().map_or(10.0, |s| s.parse().unwrap());
    let ante: f64 = args.next().map_or(0.0, |s| s.parse().unwrap());
    let start = std::time::Instant::now();
    let sol = solve_heads_up_push_fold(stack, ante, PreflopEquityTable::bundled(), 5000);
    println!(
        "{stack} BB, ante {ante}: SB shoves {:.1}%, BB calls {:.1}% (exploitability {:.4} BB, {:.0?})",
        sol.push_range_fraction() * 100.0,
        sol.call_range_fraction() * 100.0,
        sol.exploitability_bb,
        start.elapsed()
    );
    for (title, freq) in [("SB shove %", &sol.push), ("BB call %", &sol.call)] {
        println!("\n{title}");
        for row in (0..13).rev() {
            let line: Vec<String> = (0..13)
                .rev()
                .map(|col| {
                    let class = HandClass::from_index(row * 13 + col).unwrap();
                    format!(
                        "{:>4}{:>4.0}",
                        class.to_string(),
                        freq[class.index()] * 100.0
                    )
                })
                .collect();
            println!("{}", line.join(""));
        }
    }
}
