//! Data for calibrating a postflop fair play check (`ducy_gto::judge`):
//! players of known kinds play heads-up against the GTO bot, and each of
//! their decisions from the flop on is scored as a club's would be. One JSON
//! line per decision: `{"player", "hand", "street", "frequency", "top",
//! "ev_loss", "stderr", "bb_lost"}`, no cards.
//!
//!     cargo run --release -p ducy-gto --example judge_calibration -- \
//!         --cards cards.bin --blueprint blueprint.bin --hands 1000 \
//!         --out scores.jsonl [--threads 4] [--runouts 32] [--only NAME] \
//!         [--plo BIG_BLINDS]
//!
//! Hold'em by default (the heads-up model, 100 BB); with `--plo`, `--cards`
//! and `--blueprint` are a heads-up PLO model trained that deep, for the
//! number of hole cards its abstraction was built for.
//!
//! The players: every personality (and in Hold'em EquityBot), honest
//! players from weak to very strong, and two solver users: `solver` plays
//! the blueprint itself, the best case for the check, and `solver-river`
//! solves the river in real time instead, a stand-in for a solver other than
//! ducy's. Prints, per player, decisions scored and seconds taken.

use std::{
    io::Write,
    sync::{Arc, Mutex},
    time::Instant,
};

use ducy_bots::{MatchConfig, bots::EquityBot, personality::Personality, run_match};
use ducy_gto::{
    holdem::{
        abstraction::CardAbstraction,
        blueprint::Blueprint,
        bot::{GtoBot, RiverSolving},
        cards::from_ducy,
        hunl::{BettingTree, Hunl, HunlConfig},
        range::BucketCache,
        review::{ReviewConfig, Reviewer},
    },
    judge::{self, Scored, TableHand},
    omaha::{
        abstraction::PloAbstraction,
        bot::{PloGtoBot, load_blueprint},
        river::PloRiverSolving,
    },
};
use ducy_play::{Bot, Event, HandSummary, Observation, TableRules, Variant};

/// Passes through to a bot and keeps each hand's summary for its seat.
struct Recorder {
    bot: Box<dyn Bot>,
    hands: Arc<Mutex<Vec<HandSummary>>>,
}

impl Bot for Recorder {
    fn act(&mut self, obs: &Observation) -> Option<ducy_play::Action> {
        self.bot.act(obs)
    }

    fn hand_over(&mut self, s: &HandSummary) {
        self.hands.lock().unwrap().push(s.clone());
        self.bot.hand_over(s);
    }
}

/// The model the players are judged by, and the GTO bot built on it.
enum Model {
    Holdem {
        cards: Arc<CardAbstraction>,
        blueprint: Arc<Blueprint>,
        tree: Arc<BettingTree>,
        config: HunlConfig,
        runouts: usize,
    },
    Plo {
        cards: Arc<PloAbstraction>,
        blueprint: Arc<Blueprint>,
        tree: Arc<BettingTree>,
        config: HunlConfig,
    },
}

impl Model {
    fn gto(&self, seed: u64, river: bool) -> Box<dyn Bot> {
        match self {
            Model::Holdem {
                cards,
                blueprint,
                tree,
                ..
            } => {
                let b = GtoBot::from_parts(cards.clone(), blueprint.clone(), tree.clone(), seed);
                if river {
                    Box::new(b.with_river_solving(RiverSolving::default()))
                } else {
                    Box::new(b)
                }
            }
            Model::Plo {
                cards,
                blueprint,
                tree,
                config,
            } => {
                let b = PloGtoBot::from_parts(
                    config.clone(),
                    cards.clone(),
                    blueprint.clone(),
                    tree.clone(),
                    seed,
                );
                if river {
                    Box::new(b.with_river_solving(PloRiverSolving::default()))
                } else {
                    Box::new(b)
                }
            }
        }
    }

    fn rules(&self) -> (TableRules, u64) {
        match self {
            Model::Holdem { config, .. } => (TableRules::no_limit_holdem(1, 2), config.stack),
            Model::Plo { cards, config, .. } => (
                TableRules::pot_limit_omaha(1, 2).with_variant(Variant::Omaha {
                    hole_cards: cards.config.hole_cards as u32,
                }),
                config.stack,
            ),
        }
    }

    fn score(&self, hand: &TableHand, cache: &mut BucketCache) -> Vec<Scored> {
        match self {
            Model::Holdem {
                cards,
                blueprint,
                tree,
                config,
                runouts,
            } => {
                let reviewer = Reviewer {
                    tree,
                    blueprint,
                    cards,
                    tree_big_blind: config.big_blind,
                    config: ReviewConfig {
                        flop_runouts: *runouts,
                        ..ReviewConfig::default()
                    },
                };
                judge::holdem(&reviewer, hand, cache)
            }
            Model::Plo {
                cards,
                blueprint,
                tree,
                ..
            } => judge::plo(cards, blueprint, tree, hand, 1),
        }
    }
}

fn main() {
    let mut cards_path = String::new();
    let mut blueprint_path = String::new();
    let mut out_path = String::new();
    let mut hands = 1000usize;
    let mut threads = 4usize;
    let mut runouts = ReviewConfig::default().flop_runouts;
    let mut only: Option<String> = None;
    let mut plo: Option<u64> = None;
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut v = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--cards" => cards_path = v(),
            "--blueprint" => blueprint_path = v(),
            "--out" => out_path = v(),
            "--hands" => hands = v().replace('_', "").parse().expect("--hands"),
            "--threads" => threads = v().parse().expect("--threads"),
            "--runouts" => runouts = v().parse().expect("--runouts"),
            "--only" => only = Some(v()),
            "--plo" => plo = Some(v().parse().expect("--plo")),
            f => panic!("unknown option {f}"),
        }
    }
    let card_bytes = std::fs::read(&cards_path).expect("read --cards");
    let bytes = std::fs::read(&blueprint_path).expect("read --blueprint");
    let model = match plo {
        None => {
            let cards = CardAbstraction::load(&card_bytes).expect("abstraction");
            let config = HunlConfig::default();
            let game = Hunl::new(config.clone(), Some(&cards));
            let blueprint = Blueprint::load(&bytes, &game, &cards).expect("blueprint");
            drop(game);
            Model::Holdem {
                cards: Arc::new(cards),
                blueprint: Arc::new(blueprint),
                tree: Arc::new(BettingTree::build(&config)),
                config,
                runouts,
            }
        }
        Some(bb) => {
            let cards = PloAbstraction::load(&card_bytes).expect("PLO abstraction");
            let config = HunlConfig::pot_limit_omaha_lean(bb);
            let (blueprint, tree) = load_blueprint(&config, &cards, &bytes).expect("blueprint");
            Model::Plo {
                cards: Arc::new(cards),
                blueprint: Arc::new(blueprint),
                tree: Arc::new(tree),
                config,
            }
        }
    };

    let mut names: Vec<String> = vec!["solver".into(), "solver-river".into()];
    if plo.is_none() {
        names.push("equity".into());
    }
    names.extend(Personality::ALL.iter().map(|p| p.id().to_string()));
    names.retain(|n| only.as_ref().is_none_or(|o| o == n));
    let make = |name: &str, s: u64| -> Box<dyn Bot> {
        match name {
            "solver" => model.gto(s, false),
            "solver-river" => model.gto(s, true),
            "equity" => Box::new(EquityBot::new(200, Some(s))),
            id => Box::new(
                Personality::ALL
                    .iter()
                    .find(|p| p.id() == id)
                    .expect("personality")
                    .bot(Some(s)),
            ),
        }
    };

    let out = Mutex::new(std::fs::File::create(&out_path).expect("create --out"));
    let queue = Mutex::new(names.into_iter().enumerate().collect::<Vec<_>>());
    std::thread::scope(|scope| {
        for _ in 0..threads.max(1) {
            scope.spawn(|| {
                loop {
                    let Some((i, name)) = queue.lock().unwrap().pop() else {
                        break;
                    };
                    let t = Instant::now();
                    let recorded = Arc::new(Mutex::new(Vec::new()));
                    let mut bots: Vec<Box<dyn Bot>> = vec![
                        model.gto(1000 + i as u64, false),
                        Box::new(Recorder {
                            bot: make(&name, 2000 + i as u64),
                            hands: recorded.clone(),
                        }),
                    ];
                    let (rules, stack) = model.rules();
                    let m = MatchConfig::new(rules, hands, 11 + i as u64).with_starting_stack(stack);
                    run_match(&m, &mut bots).expect("match");
                    let mut cache = BucketCache::default();
                    let mut lines = String::new();
                    let mut n = 0;
                    for (h, s) in recorded.lock().unwrap().iter().enumerate() {
                        let Some(hand) = table_hand(s) else {
                            continue;
                        };
                        for d in model.score(&hand, &mut cache) {
                            if d.seat != s.seat {
                                continue;
                            }
                            n += 1;
                            lines.push_str(&format!(
                                "{{\"player\":\"{name}\",\"hand\":{h},\"street\":{},\"frequency\":{:.5},\"top\":{:.5},\"ev_loss\":{},\"stderr\":{:.5},\"bb_lost\":{:.5}}}\n",
                                d.street,
                                d.frequency,
                                d.top,
                                d.ev_loss.map_or("null".to_string(), |x| format!("{x:.5}")),
                                d.stderr,
                                d.bb_lost
                            ));
                        }
                    }
                    out.lock().unwrap().write_all(lines.as_bytes()).expect("write");
                    println!(
                        "{name:>18}: {n} decisions in {:.0} s",
                        t.elapsed().as_secs_f64()
                    );
                }
            });
        }
    });
}

/// A heads-up hand's summary as a table hand, with only the summary's own
/// seat's cards.
fn table_hand(s: &HandSummary) -> Option<TableHand> {
    let button = s.history.iter().find_map(|e| match *e {
        Event::SmallBlind { seat, .. } => Some(seat),
        _ => None,
    })?;
    let n = s.result.net.len();
    Some(TableHand {
        history: s.history.clone(),
        button,
        small_blind: s.rules.small_blind,
        big_blind: s.rules.big_blind,
        stacks: (0..n)
            .map(|i| (s.result.final_stacks[i] as i64 - s.result.net[i]).max(0) as u64)
            .collect(),
        holes: (0..n)
            .map(|i| {
                (i == s.seat)
                    .then(|| s.shown.get(i)?.as_ref())
                    .flatten()
                    .map(|h| h.iter(false).map(from_ducy).collect())
            })
            .collect(),
        board: s.board.iter().copied().map(from_ducy).collect(),
    })
}
