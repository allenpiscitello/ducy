//! Plays GtoBot against ducy-play's built-in bots in duplicate heads-up
//! matches and reports its win rate in big blinds per 100 hands, with a 95%
//! confidence interval.
//!
//!     cargo run --release -p ducy-gto --example gto_match -- \
//!         --cards abstraction.bin --blueprint blueprint.bin --deals 20000
//!
//! `--blueprint` takes a blueprint, or a training checkpoint (it's turned
//! into a blueprint on the fly, so a run in progress can be measured).
//! `--seed N` must then match the checkpoint's training seed (default 1).
//! `--only NAME` plays one opponent: `equity`, `calling`, `self` or a
//! personality id. `--lbr N` also runs Local Best Response for N hands: a
//! lower bound on how much the blueprint can be exploited.
//! `--stack N` starts both players N big blinds deep (default: the depth
//! the blueprint was trained for, 100); GtoBot plays deeper stacks off its
//! tree, as against Slumbot at 200.
//! `--depth BB:FILE` (repeatable) adds a blueprint trained BB big blinds
//! deep (`train_blueprint --bb BB`): the measured bot is then a
//! `DepthGtoBot`, playing each hand with whichever blueprint, `--blueprint`'s
//! 100 BB one included, was trained nearest that hand's depth. The `self`
//! opponent keeps playing `--blueprint` alone, so `--only self --stack 25
//! --depth 25:bp25.bin` measures what the 25 BB blueprint gains at 25 BB.
//! `--river-solve N` makes the measured bot solve the river in real time
//! with N iterations per solve (the opponents, `self` included, keep playing
//! the blueprint), and runs LBR against the river-solving bot too.
//! `--turn-solve N` makes it solve the turn too, depth-limited, with N
//! iterations per solve. LBR then compares it with the river-solving bot
//! alone, and a `river-solve` opponent (GtoBot solving the river, with the
//! `--river-solve` iterations) joins the match:
//! `--only river-solve --turn-solve 100 --river-solve 200` is turn and river
//! solving against river solving alone.
//!
//! Duplicate mode plays every deal twice with the seats swapped, which
//! cancels most of the luck of the cards. The interval comes from the
//! spread of results across blocks of deals.
//!
//! `--game plo` plays heads-up pot-limit Omaha instead (#131): `--cards`
//! is then a PLO abstraction (`plo_abstraction` builds one), `--blueprint`
//! a PLO blueprint or `train_plo` checkpoint, and `--bb N` the depth it was
//! trained for (default 100). The bot is `omaha::bot::PloGtoBot`, against
//! itself, `EquityBot`, a calling station and every personality bot.
//! `--river-solve N` makes it solve the river in real time (#132); `--only
//! self` is then river solving against the blueprint. The game has as many
//! hole cards as the abstraction (six for a PLO6 one, #200; `--game plo6`
//! is the same as `--game plo`). `--rival-cards FILE --rival-blueprint
//! FILE` adds a `rival` opponent: another PLO blueprint on its own
//! abstraction, such as the published one, to play head to head (#204).

use std::{cell::Cell, rc::Rc, sync::Arc, time::Instant};

use ducy_gto::{
    Config, Discount, Mccfr,
    holdem::{
        abstraction::CardAbstraction,
        blueprint::Blueprint,
        bot::{GtoBot, RiverSolving, TurnSolving},
        depths::{DepthGtoBot, depth_config},
        hunl::{BettingTree, Hunl, HunlConfig},
        lbr::{LbrResult, lbr_hands_solving},
    },
    omaha::{
        abstraction::PloAbstraction,
        bot::{PloGtoBot, load_blueprint},
        river::PloRiverSolving,
    },
};
use ducy_play::{
    Bot, MatchConfig, TableRules, Variant,
    bots::{CallingStation, EquityBot},
    personality::Personality,
    run_match,
};

/// Blocks of deals the interval is computed over.
const BLOCKS: usize = 40;

fn main() {
    let mut cards_path = String::new();
    let mut blueprint_path = String::new();
    let mut deals = 20_000usize;
    let mut seed = 1u64;
    let mut only: Option<String> = None;
    let mut lbr = 0usize;
    let mut river = 0usize;
    let mut turn = 0usize;
    let mut plo = false;
    let mut bb = 100u64;
    let mut stack_bb: Option<u64> = None;
    let mut depths: Vec<(u64, String)> = Vec::new();
    let mut rival_cards: Option<String> = None;
    let mut rival_blueprint: Option<String> = None;
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut v = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--cards" => cards_path = v(),
            "--blueprint" => blueprint_path = v(),
            "--deals" => deals = v().replace('_', "").parse().expect("--deals"),
            "--seed" => seed = v().parse().expect("--seed"),
            "--only" => only = Some(v()),
            "--lbr" => lbr = v().replace('_', "").parse().expect("--lbr"),
            "--river-solve" => river = v().parse().expect("--river-solve"),
            "--turn-solve" => turn = v().parse().expect("--turn-solve"),
            "--game" => plo = v().starts_with("plo"),
            "--bb" => bb = v().parse().expect("--bb"),
            "--stack" => stack_bb = Some(v().parse().expect("--stack")),
            "--depth" => {
                let s = v();
                let (bb, file) = s.split_once(':').expect("--depth BB:FILE");
                depths.push((bb.parse().expect("--depth BB"), file.to_string()));
            }
            "--rival-cards" => rival_cards = Some(v()),
            "--rival-blueprint" => rival_blueprint = Some(v()),
            f => panic!("unknown option {f}"),
        }
    }
    if plo {
        plo_match(
            &cards_path,
            &blueprint_path,
            bb,
            deals,
            seed,
            only.as_deref(),
            river,
            rival_cards.as_deref().zip(rival_blueprint.as_deref()),
        );
        return;
    }
    let cards = Arc::new(
        CardAbstraction::load(&std::fs::read(&cards_path).expect("read --cards"))
            .expect("abstraction"),
    );
    let config = HunlConfig::default();
    let game = Hunl::new(config.clone(), Some(&cards));
    let bytes = std::fs::read(&blueprint_path).expect("read --blueprint");
    let blueprint = match Blueprint::load(&bytes, &game, &cards) {
        Ok(b) => b,
        Err(_) => {
            let c = Config {
                seed,
                batch: 4096,
                discount: Discount::DCFR,
                prune: None,
            };
            let m = Mccfr::load(&game, c, &bytes)
                .expect("a blueprint, or a checkpoint for this game and --seed");
            println!("checkpoint at {} iterations", m.iterations());
            Blueprint::from_profile(&game, &cards, &m.average())
        }
    };
    // (turn, river) iterations per run; the last is compared with the first.
    let runs = match (turn, river) {
        (0, 0) => vec![(0, 0)],
        (0, r) => vec![(0, 0), (0, r)],
        (t, r) => vec![(0, r), (t, r)],
    };
    let mut per_hand = Vec::new();
    for &(t_iter, r_iter) in runs.iter().filter(|_| lbr > 0) {
        let t = Instant::now();
        let hands = lbr_hands_solving(&game, &cards, &blueprint, lbr, 99, t_iter, r_iter);
        let r = LbrResult::of(&hands);
        let what = match (t_iter, r_iter) {
            (0, 0) => "the blueprint".to_string(),
            (0, r) => format!("river solving ({r} iterations)"),
            (t, 0) => format!("turn solving ({t} iterations)"),
            (t, r) => format!("turn and river solving ({t} and {r} iterations)"),
        };
        println!(
            "LBR vs {what} over {} hands: {:.0} ± {:.0} mbb/hand ({:.0}s)",
            r.hands,
            r.mbb_per_hand,
            r.ci95,
            t.elapsed().as_secs_f64()
        );
        per_hand.push(hands);
    }
    if let [plain, solving] = &per_hand[..] {
        // The same hands up to the street solved: compare them pairwise.
        let diff: Vec<f64> = solving.iter().zip(plain).map(|(a, b)| a - b).collect();
        let r = LbrResult::of(&diff);
        let what = if turn > 0 { "turn" } else { "river" };
        println!(
            "LBR change from {what} solving: {:+.0} ± {:.0} mbb/hand (paired)",
            r.mbb_per_hand, r.ci95
        );
    }
    let blueprint = Arc::new(blueprint);
    let tree = Arc::new(BettingTree::build(&config));
    let gto = |s: u64| GtoBot::from_parts(cards.clone(), blueprint.clone(), tree.clone(), s);

    type Make<'a> = Box<dyn Fn(u64) -> Box<dyn Bot> + 'a>;
    let mut opponents: Vec<(String, Make)> = vec![
        (
            "self".into(),
            Box::new(|s| Box::new(gto(s)) as Box<dyn Bot>),
        ),
        (
            "equity".into(),
            Box::new(|s| Box::new(EquityBot::new(200, Some(s))) as Box<dyn Bot>),
        ),
        (
            "calling".into(),
            Box::new(|_| Box::new(CallingStation) as Box<dyn Bot>),
        ),
    ];
    if turn > 0 && river > 0 {
        opponents.push((
            "river-solve".into(),
            Box::new(|s| {
                Box::new(gto(s).with_river_solving(RiverSolving::new(river))) as Box<dyn Bot>
            }),
        ));
    }
    for p in Personality::ALL {
        opponents.push((
            p.id().to_string(),
            Box::new(move |s| Box::new(p.bot(Some(s))) as Box<dyn Bot>),
        ));
    }
    let rules = TableRules::no_limit_holdem(config.small_blind, config.big_blind);
    let stack = stack_bb.map_or(config.stack, |n| n * config.big_blind);
    let depth_bytes: Vec<(u64, Vec<u8>)> = depths
        .iter()
        .map(|(d, f)| (*d, std::fs::read(f).expect("read --depth FILE")))
        .collect();
    if !depths.is_empty() {
        let mut on_offer: Vec<u64> = depths.iter().map(|(d, _)| *d).collect();
        on_offer.push(config.stack / config.big_blind);
        on_offer.sort();
        println!("measured bot: the blueprint nearest each hand's depth, of {on_offer:?} BB");
    }
    let per_block = (deals / BLOCKS).max(1);
    println!(
        "{:>22} {:>9} {:>10} {:>10}",
        "opponent", "hands", "bb/100", "± 95%"
    );
    for (name, make) in &opponents {
        if only.as_ref().is_some_and(|o| o != name) {
            continue;
        }
        let t = Instant::now();
        let mut rates = Vec::with_capacity(BLOCKS);
        let mut hands = 0;
        // One bot pair for the whole match, so adaptive opponents keep learning.
        let solving = |b: GtoBot| {
            let b = b.with_river_solving(RiverSolving::new(river));
            if turn > 0 {
                b.with_turn_solving(TurnSolving::new(turn))
            } else {
                b
            }
        };
        let me: Box<dyn Bot> = if depth_bytes.is_empty() {
            Box::new(solving(gto(1000)))
        } else {
            let mut bots = vec![(config.stack / config.big_blind, solving(gto(1000)))];
            for (i, (d, bytes)) in depth_bytes.iter().enumerate() {
                let b = GtoBot::new(depth_config(*d), cards.clone(), bytes, 1001 + i as u64)
                    .expect("a blueprint trained at its --depth");
                bots.push((*d, solving(b)));
            }
            Box::new(DepthGtoBot::new(bots))
        };
        let mut bots: Vec<Box<dyn Bot>> = vec![me, make(2000)];
        for b in 0..BLOCKS {
            let m = MatchConfig::new(rules, per_block, 7919 * b as u64 + 1)
                .with_starting_stack(stack)
                .duplicate();
            let r = run_match(&m, &mut bots).expect("match");
            assert_eq!(r.fallbacks[0], 0, "GtoBot made an illegal action");
            rates.push(r.bb_per_100(0));
            hands += r.hands;
        }
        let mean = rates.iter().sum::<f64>() / BLOCKS as f64;
        let var = rates.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (BLOCKS - 1) as f64;
        let ci = 1.96 * (var / BLOCKS as f64).sqrt();
        println!(
            "{name:>22} {hands:>9} {mean:>10.1} {ci:>10.1}   ({:.0}s)",
            t.elapsed().as_secs_f64()
        );
    }
}

/// Duplicate matches between the PLO blueprint and every PLO opponent;
/// with `river` iterations, the measured bot solves the river in real time
/// (the opponents, `self` included, keep playing the blueprint).
#[allow(clippy::too_many_arguments)]
fn plo_match(
    cards_path: &str,
    blueprint_path: &str,
    bb: u64,
    deals: usize,
    seed: u64,
    only: Option<&str>,
    river: usize,
    rival: Option<(&str, &str)>,
) {
    let config = HunlConfig::pot_limit_omaha_lean(bb);
    let (cards, blueprint, tree) = load_plo(&config, cards_path, blueprint_path, seed);
    let k = cards.config.hole_cards;
    let gto = |s: u64| {
        PloGtoBot::from_parts(
            config.clone(),
            cards.clone(),
            blueprint.clone(),
            tree.clone(),
            s,
        )
    };
    // Another blueprint, on its own abstraction, to play head to head.
    let rival = rival.map(|(c, b)| {
        let r = load_plo(&config, c, b, seed);
        assert_eq!(r.0.config.hole_cards, k, "the rival plays the same game");
        r
    });
    type Make<'a> = Box<dyn Fn(u64) -> Box<dyn Bot> + 'a>;
    let mut opponents: Vec<(String, Make)> = Vec::new();
    if let Some((c, b, t)) = &rival {
        opponents.push((
            "rival".into(),
            Box::new(|s| {
                Box::new(PloGtoBot::from_parts(
                    config.clone(),
                    c.clone(),
                    b.clone(),
                    t.clone(),
                    s,
                )) as Box<dyn Bot>
            }),
        ));
    }
    opponents.extend::<Vec<(String, Make)>>(vec![
        (
            "self".into(),
            Box::new(|s| Box::new(gto(s)) as Box<dyn Bot>),
        ),
        (
            "equity".into(),
            Box::new(|s| Box::new(EquityBot::new(200, Some(s))) as Box<dyn Bot>),
        ),
        (
            "calling".into(),
            Box::new(|_| Box::new(CallingStation) as Box<dyn Bot>),
        ),
    ]);
    for p in Personality::ALL {
        opponents.push((
            p.id().to_string(),
            Box::new(move |s| Box::new(p.bot(Some(s))) as Box<dyn Bot>),
        ));
    }
    let rules = TableRules::pot_limit_omaha(config.small_blind, config.big_blind).with_variant(
        Variant::Omaha {
            hole_cards: k as u32,
        },
    );
    let per_block = (deals / BLOCKS).max(1);
    println!(
        "PLO{k} {bb} BB. {:>11} {:>9} {:>10} {:>10} {:>8}",
        "opponent", "hands", "bb/100", "± 95%", "off-tree"
    );
    for (name, make) in &opponents {
        if only.is_some_and(|o| o != name) {
            continue;
        }
        let t = Instant::now();
        let mut rates = Vec::with_capacity(BLOCKS);
        let mut hands = 0;
        let off_tree = Rc::new(Cell::new(0));
        let me = Watched(
            gto(1000).with_river_solving(PloRiverSolving::new(river)),
            off_tree.clone(),
        );
        let mut bots: Vec<Box<dyn Bot>> = vec![Box::new(me), make(2000)];
        for b in 0..BLOCKS {
            let m = MatchConfig::new(rules, per_block, 7919 * b as u64 + 1)
                .with_starting_stack(config.stack)
                .duplicate();
            let r = run_match(&m, &mut bots).expect("match");
            assert_eq!(r.fallbacks[0], 0, "PloGtoBot made an illegal action");
            rates.push(r.bb_per_100(0));
            hands += r.hands;
        }
        let mean = rates.iter().sum::<f64>() / BLOCKS as f64;
        let var = rates.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (BLOCKS - 1) as f64;
        let ci = 1.96 * (var / BLOCKS as f64).sqrt();
        println!(
            "{name:>22} {hands:>9} {mean:>10.1} {ci:>10.1} {:>8}   ({:.0}s)",
            off_tree.get(),
            t.elapsed().as_secs_f64()
        );
    }
}

/// A PLO abstraction and the blueprint (or checkpoint) trained on it.
fn load_plo(
    config: &HunlConfig,
    cards_path: &str,
    blueprint_path: &str,
    seed: u64,
) -> (Arc<PloAbstraction>, Arc<Blueprint>, Arc<BettingTree>) {
    let cards = Arc::new(
        PloAbstraction::load(&std::fs::read(cards_path).expect("read the abstraction"))
            .expect("a PLO abstraction"),
    );
    let bytes = std::fs::read(blueprint_path).expect("read the blueprint");
    let (blueprint, tree) = match load_blueprint(config, &cards, &bytes) {
        Ok(b) => b,
        Err(_) => match cards.config.hole_cards {
            4 => from_checkpoint::<4>(config, &cards, &bytes, seed),
            5 => from_checkpoint::<5>(config, &cards, &bytes, seed),
            _ => from_checkpoint::<6>(config, &cards, &bytes, seed),
        },
    };
    (cards, Arc::new(blueprint), Arc::new(tree))
}

/// The blueprint from a `train_plo` checkpoint for `H` hole cards.
fn from_checkpoint<const H: usize>(
    config: &HunlConfig,
    cards: &PloAbstraction,
    bytes: &[u8],
    seed: u64,
) -> (Blueprint, BettingTree) {
    let game = Hunl::<_, H>::with_cards(config.clone(), Some(cards));
    let c = Config {
        seed,
        batch: 4096,
        discount: Discount::DCFR,
        prune: None,
    };
    let m = Mccfr::load(&game, c, bytes)
        .expect("a PLO blueprint, or a checkpoint for this game and --seed");
    println!("checkpoint at {} iterations", m.iterations());
    let blueprint = Blueprint::from_strategy(&game, cards, |info, n| {
        m.average_at(&info)
            .unwrap_or_else(|| vec![1.0 / n as f64; n])
    });
    (blueprint, game.tree)
}

/// The PLO bot, sharing its off-tree count with the report.
struct Watched(PloGtoBot, Rc<Cell<u32>>);

impl Bot for Watched {
    fn act(&mut self, obs: &ducy_play::Observation) -> Option<ducy_play::Action> {
        let a = self.0.act(obs);
        self.1.set(self.0.off_tree);
        a
    }

    fn hand_over(&mut self, summary: &ducy_play::HandSummary) {
        self.0.hand_over(summary);
    }
}
