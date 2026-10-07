//! Writes a tiny GTO model for tests: the quick card abstraction and an
//! untrained (uniform) blueprint for the default game, as `cards.bin` and
//! `blueprint.bin` in the given directory. They load like the real files
//! (`loadGto` in ducy-wasm) but are built instantly.
//!
//!     cargo run -p ducy-gto --example tiny_model -- target/tiny-model

use ducy_gto::{
    Profile,
    holdem::{
        abstraction::CardAbstraction,
        blueprint::Blueprint,
        hunl::{Hunl, HunlConfig},
    },
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
    println!("wrote {}", dir.display());
}
