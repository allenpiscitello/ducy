//! How close a trained six-handed postflop spot is to GTO (#267): its
//! sampled exploitability (`ducy_gto::sampled`), what a best response in the
//! same abstraction wins against it, beside the same for every action
//! equally likely, as a yardstick.
//!
//!     cargo run --release -p ducy-gto --example sixmax_spot_eval -- \
//!         --strategy sixmax-100.bin --cards cards-full.bin --opener BTN --caller BB \
//!         --checkpoint spot-100-BTN-BB.bin [--fit 20000] [--eval 20000]
//!
//! Values are in big blinds per hand and in percent of the pot at the flop.
//! The estimate is a lower bound (the fitted response is only near-best),
//! with its standard error.

use ducy_gto::{
    Config, Mccfr,
    holdem::abstraction::CardAbstraction,
    sampled::sampled_exploitability,
    sixmax::{bot::PreflopStrategy, preflop::POSITIONS, spot::SpotGame},
};

fn main() {
    let mut strategy_path = String::new();
    let mut cards_path = String::new();
    let mut opener = String::from("BTN");
    let mut caller = String::from("BB");
    let mut checkpoint = String::new();
    let mut fit = 20_000usize;
    let mut eval = 20_000usize;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut v = || args.next().unwrap_or_else(|| panic!("{a} needs a value"));
        match a.as_str() {
            "--strategy" => strategy_path = v(),
            "--cards" => cards_path = v(),
            "--opener" => opener = v(),
            "--caller" => caller = v(),
            "--checkpoint" => checkpoint = v(),
            "--fit" => fit = v().parse().expect("--fit"),
            "--eval" => eval = v().parse().expect("--eval"),
            _ => panic!("unknown argument {a}"),
        }
    }
    let pos = |name: &str| {
        POSITIONS
            .iter()
            .position(|p| *p == name)
            .unwrap_or_else(|| panic!("no position {name}"))
    };
    let strategy =
        PreflopStrategy::from_compact(&std::fs::read(&strategy_path).expect("read --strategy"))
            .expect("a compact strategy");
    let cards = CardAbstraction::load(&std::fs::read(&cards_path).expect("read --cards"))
        .expect("a card abstraction");
    let game = SpotGame::single_raised(&strategy, pos(&opener), pos(&caller), &cards)
        .expect("that line is in the strategy");
    let solver = Mccfr::load(
        &game,
        // The seed sixmax_spot_train trains with (a checkpoint checks it).
        Config { seed: 1, ..Config::default() },
        &std::fs::read(&checkpoint).expect("read --checkpoint"),
    )
    .expect("a checkpoint for this spot");
    let pot = (game.tree.nodes[0].betting.pot() + game.dead) as f64;
    let bb = game.config.big_blind as f64;
    let show = |name: &str, r: ducy_gto::sampled::SampledResponse| {
        println!(
            "{name:<20} {:>7.3} ± {:.3} BB a hand, {:>5.1}% of the pot",
            r.value / bb,
            r.std_error / bb,
            100.0 * r.value / pot
        );
    };
    println!(
        "{} at {} BB, {} iterations: sampled exploitability ({fit} deals to fit, {eval} to measure)",
        game.name,
        strategy.game().stack / 2,
        solver.iterations()
    );
    let trained = |info: &u64, n: usize| {
        solver
            .average_at(info)
            .unwrap_or_else(|| vec![1.0 / n as f64; n])
    };
    show(
        "trained",
        sampled_exploitability(&game, &trained, fit, eval, 11),
    );
    let uniform = |_: &u64, n: usize| vec![1.0 / n as f64; n];
    show(
        "uniform",
        sampled_exploitability(&game, &uniform, fit, eval, 11),
    );
}
