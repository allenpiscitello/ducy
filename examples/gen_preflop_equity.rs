//! Regenerates `src/preflop/equity169.bin`, the bundled class-vs-class
//! preflop equity table: `cargo run --release --example gen_preflop_equity [samples]`.
use ducy::preflop::PreflopEquityTable;

fn main() {
    let samples: u32 = std::env::args()
        .nth(1)
        .map(|s| s.parse().expect("samples must be a number"))
        .unwrap_or(200_000);
    let start = std::time::Instant::now();
    let table = PreflopEquityTable::compute(samples);
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/preflop/equity169.bin");
    std::fs::write(path, table.to_bytes()).expect("failed to write table");
    println!(
        "wrote {path} ({samples} samples per matchup) in {:.1?}",
        start.elapsed()
    );
}
