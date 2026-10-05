//! Plays many hands with GtoBot (untrained blueprint on the quick
//! abstraction) against wild and sensible opponents, counting illegal actions
//! and decisions it couldn't follow on the tree.
//!
//!     cargo run --release -p ducy-gto --example gto_stress -- 100000

use std::sync::Arc;

use ducy_gto::{
    Profile,
    holdem::{
        abstraction::CardAbstraction,
        blueprint::Blueprint,
        bot::GtoBot,
        hunl::{Hunl, HunlConfig},
    },
};
use ducy_play::{
    Bot, MatchConfig, TableRules,
    bots::{EquityBot, RandomBot},
    personality::Personality,
    run_match,
};

type MakeBot = Box<dyn Fn() -> Box<dyn Bot>>;

fn main() {
    let deals: usize = std::env::args()
        .nth(1)
        .map_or(20_000, |s| s.parse().expect("deals"));
    let cards = Arc::new(CardAbstraction::quick(8));
    let config = HunlConfig::default();
    let game = Hunl::new(config.clone(), Some(&cards));
    let bp = Blueprint::from_profile(&game, &cards, &Profile::new()).save();
    let rules = TableRules::no_limit_holdem(1, 2);
    let opponents: Vec<(&str, MakeBot)> = vec![
        ("random", Box::new(|| Box::new(RandomBot::new(Some(1))))),
        (
            "equity",
            Box::new(|| Box::new(EquityBot::new(100, Some(2)))),
        ),
        (
            "personality",
            Box::new(|| Box::new(Personality::ALL[3].bot(Some(3)))),
        ),
    ];
    for (name, make) in opponents {
        let gto = GtoBot::new(config.clone(), cards.clone(), &bp, 7).unwrap();
        let shared = Arc::new(std::sync::Mutex::new(gto));
        struct Shared(Arc<std::sync::Mutex<GtoBot>>);
        impl Bot for Shared {
            fn act(&mut self, o: &ducy_play::Observation) -> Option<ducy_play::Action> {
                self.0.lock().unwrap().act(o)
            }
            fn hand_over(&mut self, s: &ducy_play::HandSummary) {
                self.0.lock().unwrap().hand_over(s)
            }
        }
        let mut bots: Vec<Box<dyn Bot>> = vec![Box::new(Shared(shared.clone())), make()];
        let r = run_match(&MatchConfig::new(rules, deals, 5).duplicate(), &mut bots).unwrap();
        println!(
            "{name:>12}: {} hands, illegal actions {}, off-tree decisions {}",
            r.hands,
            r.fallbacks[0],
            shared.lock().unwrap().off_tree
        );
    }
}
