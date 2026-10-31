//! Measures postflop value tables from trained six-handed spots (#267), for
//! the preflop solve to use instead of equity times realization
//! (`sixmax_preflop --postflop`): for each spot, every pair of hand classes'
//! share of the flop pot when both play the trained strategy.
//!
//!     cargo run --release -p ducy-gto --example sixmax_postflop_tables -- \
//!         --strategy sixmax-100.bin --cards cards-full.bin \
//!         --spots spot-100-BTN-BB.bin,spot-100-CO-BB.bin --out postflop-100.bin \
//!         [--samples 24] [--seed 21]
//!
//! Each spot's deals come from its own seed (`--seed` plus its place in the
//! list), so the sampling noise of one pair of classes differs from table
//! to table and averages out when tables are pooled.
//!
//! Spot files are named spot-<bb>-<name>.bin (sixmax_spot_train,
//! `SpotGame::from_name`): single-raised, limped or 3-bet pots alike.

use std::time::Instant;

use ducy_gto::{
    Config, Mccfr,
    holdem::{abstraction::CardAbstraction, chart::class_name, iso::NUM_PREFLOP_CLASSES},
    sixmax::{bot::PreflopStrategy, preflop::PostflopTable, spot::SpotGame},
};

fn main() {
    let mut strategy_path = String::new();
    let mut cards_path = String::new();
    let mut spots: Vec<String> = Vec::new();
    let mut out = String::from("postflop.bin");
    let mut samples = 24usize;
    let mut seed = 21u64;
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut v = || args.next().unwrap_or_else(|| panic!("{a} needs a value"));
        match a.as_str() {
            "--strategy" => strategy_path = v(),
            "--cards" => cards_path = v(),
            "--spots" => spots = v().split(',').map(str::to_string).collect(),
            "--out" => out = v(),
            "--samples" => samples = v().parse().expect("--samples"),
            "--seed" => seed = v().parse().expect("--seed"),
            _ => panic!("unknown argument {a}"),
        }
    }
    let strategy =
        PreflopStrategy::from_compact(&std::fs::read(&strategy_path).expect("read --strategy"))
            .expect("a compact strategy");
    let cards = CardAbstraction::load(&std::fs::read(&cards_path).expect("read --cards"))
        .expect("a card abstraction");
    let class = |name: &str| {
        (0..NUM_PREFLOP_CLASSES)
            .find(|&c| class_name(c) == name)
            .expect("class")
    };
    let mut tables = Vec::new();
    for (index, file) in spots.iter().enumerate() {
        let t = Instant::now();
        let name = SpotGame::name_of_file(file)
            .unwrap_or_else(|| panic!("{file}: not spot-<bb>-<name>.bin"));
        let game =
            SpotGame::from_name(&strategy, &name, &cards).expect("that line is in the strategy");
        // The seed sixmax_spot_train trains with.
        let profile = Mccfr::load(
            &game,
            Config {
                seed: 1,
                ..Config::default()
            },
            &std::fs::read(file).expect("read a spot"),
        )
        .expect("a checkpoint for this spot")
        .average();
        let share = game.share_table(&profile, samples, seed + index as u64, threads);
        let at = |ip: &str, oop: &str| share[class(ip) * NUM_PREFLOP_CLASSES + class(oop)];
        println!(
            "{}: AA v 72o {:.2}, 72o v AA {:.2}, KQs v 99 {:.2}, 76s v AJo {:.2} ({:.0}s)",
            game.name,
            at("AA", "72o"),
            at("72o", "AA"),
            at("KQs", "99"),
            at("76s", "AJo"),
            t.elapsed().as_secs_f64()
        );
        // The table only needs the two positions and the preflop bet.
        let [ip, oop] = game.positions;
        tables.push(PostflopTable {
            opener: oop,
            caller: ip,
            to: game.preflop_bet() as u32,
            share,
        });
    }
    std::fs::write(&out, PostflopTable::to_bytes(&tables)).expect("write --out");
    println!("{} tables in {out}", tables.len());
}
