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

use std::{cell::Cell, rc::Rc, sync::Arc, time::Instant};

use ducy_gto::{
    Config, Discount, Mccfr,
    holdem::{
        abstraction::CardAbstraction,
        blueprint::Blueprint,
        bot::{GtoBot, RiverSolving, TurnSolving},
        hunl::{BettingTree, HuPlo, Hunl, HunlConfig},
        lbr::{LbrResult, lbr_hands_solving},
    },
    omaha::{abstraction::PloAbstraction, bot::PloGtoBot},
};
use ducy_play::{
    Bot, MatchConfig, TableRules,
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
            "--game" => plo = v() == "plo",
            "--bb" => bb = v().parse().expect("--bb"),
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
        let mut me = gto(1000).with_river_solving(RiverSolving::new(river));
        if turn > 0 {
            me = me.with_turn_solving(TurnSolving::new(turn));
        }
        let mut bots: Vec<Box<dyn Bot>> = vec![Box::new(me), make(2000)];
        for b in 0..BLOCKS {
            let m = MatchConfig::new(rules, per_block, 7919 * b as u64 + 1)
                .with_starting_stack(config.stack)
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

/// Duplicate matches between the PLO blueprint and every PLO opponent.
fn plo_match(
    cards_path: &str,
    blueprint_path: &str,
    bb: u64,
    deals: usize,
    seed: u64,
    only: Option<&str>,
) {
    let cards = Arc::new(
        PloAbstraction::load(&std::fs::read(cards_path).expect("read --cards"))
            .expect("a PLO abstraction"),
    );
    let config = HunlConfig::pot_limit_omaha_lean(bb);
    let game = HuPlo::with_cards(config.clone(), Some(&*cards));
    let bytes = std::fs::read(blueprint_path).expect("read --blueprint");
    let blueprint = match Blueprint::load(&bytes, &game, &*cards) {
        Ok(b) => b,
        Err(_) => {
            let c = Config {
                seed,
                batch: 4096,
                discount: Discount::DCFR,
                prune: None,
            };
            let m = Mccfr::load(&game, c, &bytes)
                .expect("a PLO blueprint, or a checkpoint for this game and --seed");
            println!("checkpoint at {} iterations", m.iterations());
            Blueprint::from_strategy(&game, &*cards, |info, n| {
                m.average_at(&info)
                    .unwrap_or_else(|| vec![1.0 / n as f64; n])
            })
        }
    };
    let blueprint = Arc::new(blueprint);
    let tree = Arc::new(game.tree);
    let gto = |s: u64| {
        PloGtoBot::from_parts(
            config.clone(),
            cards.clone(),
            blueprint.clone(),
            tree.clone(),
            s,
        )
    };
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
    for p in Personality::ALL {
        opponents.push((
            p.id().to_string(),
            Box::new(move |s| Box::new(p.bot(Some(s))) as Box<dyn Bot>),
        ));
    }
    let rules = TableRules::pot_limit_omaha(config.small_blind, config.big_blind);
    let per_block = (deals / BLOCKS).max(1);
    println!(
        "PLO {bb} BB. {:>12} {:>9} {:>10} {:>10} {:>8}",
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
        let me = Watched(gto(1000), off_tree.clone());
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
