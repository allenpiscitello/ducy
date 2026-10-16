//! Writes tiny GTO models for tests, built in seconds, that load like the
//! real files:
//!
//! - Hold'em (`loadGto` in ducy-wasm): the quick card abstraction and an
//!   untrained (uniform) blueprint for the default game, as `cards.bin` and
//!   `blueprint.bin`; and one for the 25 BB game on the same cards
//!   (`loadGtoDepth`), as `blueprint-25.bin`.
//! - PLO (`loadGtoPlo`): a small PLO abstraction and an untrained blueprint
//!   for the lean 10 big blind game, as `plo-cards.bin` and
//!   `plo-blueprint.bin`; the same for six hole cards as `plo6-cards.bin`
//!   and `plo6-blueprint.bin`.
//! - Six-handed (`loadGto6`): a compact preflop strategy for 100 BB with
//!   every action equally likely, as `sixmax.bin`.
//!
//!     cargo run -p ducy-gto --example tiny_model -- target/tiny-model

use ducy_gto::{
    Profile,
    holdem::{
        abstraction::CardAbstraction,
        blueprint::Blueprint,
        depths::depth_config,
        hunl::{HuPlo, Hunl, HunlConfig},
    },
    omaha::abstraction::{PloAbstraction, PloAbstractionConfig},
    sixmax::{
        bot::PreflopStrategy,
        preflop::{Node, PreflopGame},
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
    // A 25 BB one on the same cards, for `loadGtoDepth`.
    let game = Hunl::new(depth_config(25), Some(&cards));
    let blueprint = Blueprint::from_profile(&game, &cards, &Profile::new());
    std::fs::write(dir.join("blueprint-25.bin"), blueprint.save()).expect("write blueprint-25.bin");

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

    let cards = PloAbstraction::build(
        PloAbstractionConfig {
            hole_cards: 6,
            preflop: 8,
            flop: 8,
            turn: 8,
            river: 8,
            fit_hands: 500,
            equity_samples: 50,
            ..PloAbstractionConfig::default()
        },
        |_| {},
    );
    let game = Hunl::<_, 6>::with_cards(HunlConfig::pot_limit_omaha_lean(10), Some(&cards));
    let blueprint = Blueprint::from_profile(&game, &cards, &Profile::new());
    std::fs::write(dir.join("plo6-cards.bin"), cards.save()).expect("write plo6-cards.bin");
    std::fs::write(dir.join("plo6-blueprint.bin"), blueprint.save())
        .expect("write plo6-blueprint.bin");
    // Six-handed (`loadGto6`): a compact preflop strategy for 100 BB, every
    // action equally likely, with only the first decisions kept.
    let game = PreflopGame::new(100);
    let mut actions = vec![0; game.decisions as usize];
    for n in &game.nodes {
        if let Node::Decision { id, actions: a, .. } = n {
            actions[*id as usize] = a.len();
        }
    }
    let uniform = PreflopStrategy::from_fn(&game, |id, _| {
        let n = actions[id as usize];
        vec![1.0 / n as f32; n]
    });
    std::fs::write(dir.join("sixmax.bin"), uniform.to_compact(|id| id < 500))
        .expect("write sixmax.bin");
    println!("wrote {}", dir.display());
}
