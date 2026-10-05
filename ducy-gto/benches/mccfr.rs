//! MCCFR speed on Leduc hold'em: iterations per second, single-threaded
//! (one chunk per batch) and with parallel batches.

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use ducy_gto::{Config, Discount, Mccfr, games::leduc::Leduc};

fn bench(c: &mut Criterion) {
    let mut g = c.benchmark_group("mccfr_leduc");
    for (name, batch) in [("batch16", 16u64), ("batch1024", 1024)] {
        g.throughput(Throughput::Elements(batch));
        let config = Config {
            seed: 1,
            batch,
            discount: Discount::DCFR,
            prune: None,
        };
        let mut solver = Mccfr::new(&Leduc, config);
        solver.run(10_000); // warm: every information set exists
        g.bench_function(name, |b| b.iter(|| solver.run(batch)));
        eprintln!(
            "{name}: {} infosets, {} bytes of regrets and sums ({:.1} bytes per infoset)",
            solver.num_infosets(),
            solver.table_bytes(),
            solver.table_bytes() as f64 / solver.num_infosets() as f64
        );
    }
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
