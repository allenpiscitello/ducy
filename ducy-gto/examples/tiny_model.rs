//! Writes tiny GTO models for tests, built in seconds, that load like the
//! real files:
//!
//! - Hold'em (`loadGto` in ducy-wasm): the quick card abstraction and an
//!   untrained (uniform) blueprint for the default game, as `cards.bin` and
//!   `blueprint.bin`.
//! - PLO (`loadGtoPlo`): a small PLO abstraction and an untrained blueprint
//!   for the lean 10 big blind game, as `plo-cards.bin` and
//!   `plo-blueprint.bin`.
//!
//!     cargo run -p ducy-gto --example tiny_model -- target/tiny-model

use ducy_gto::{
    Profile,
    holdem::{
        abstraction::CardAbstraction,
        blueprint::Blueprint,
        hunl::{HuPlo, Hunl, HunlConfig},
    },
    omaha::abstraction::{PloAbstraction, PloAbstractionConfig},
};

fn main() {
    let dir = std::env::args().nth(1).expect("an output directory");
    std::fs::create_dir_all(&dir).expect("create the directory");
    let cards = CardAbstraction::quick(8);
    let game = Hunl::new(HunlConfig::default(), Some(&cards));
    let blueprint = Blueprint::from_profile(&game, &cards, &Profile::new());
    let dir = std::path::Path::new(&dir);
    std::fs::write(dir.join("cards.bin"), cards.save()).expect("write cards.bin");
    std::fs::write(dir.join("blueprint.bin"), blueprint.save()).expect("write blueprint.bin");

    let cards = PloAbstraction::build(
        PloAbstractionConfig {
            preflop: 0,
            flop: 8,
            turn: 8,
            river: 8,
            fit_hands: 500,
            equity_samples: 50,
            ..PloAbstractionConfig::default()
        },
        |_| {},
    );
    let game = HuPlo::with_cards(HunlConfig::pot_limit_omaha_lean(10), Some(&cards));
    let blueprint = Blueprint::from_profile(&game, &cards, &Profile::new());
    std::fs::write(dir.join("plo-cards.bin"), cards.save()).expect("write plo-cards.bin");
    std::fs::write(dir.join("plo-blueprint.bin"), blueprint.save())
        .expect("write plo-blueprint.bin");
    println!("wrote {}", dir.display());
}
