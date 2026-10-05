//! Calibrates the hand review: plays GtoBot against other bots, reviews the
//! other bot's decisions, and reports what the review charges them.
//!
//!     cargo run --release -p ducy-gto --example review_calibration -- \
//!         --cards cards.bin --blueprint blueprint.bin --hands 200
//!
//! For each opponent (GtoBot itself, EquityBot, a calling station and every
//! personality): big blinds charged per 100 hands and per decision, the
//! grades, what it actually won (luck included), and how long a review takes
//! per decision on each street. GtoBot reviewing itself should be charged
//! about nothing; the bots GtoBot beats most should be charged the most.
//! `--only NAME` reviews one opponent; `--show N` prints the first N
//! reviews in full.

use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use ducy_gto::holdem::{
    abstraction::CardAbstraction,
    blueprint::Blueprint,
    bot::GtoBot,
    hunl::{BettingTree, Hunl, HunlConfig},
    range::BucketCache,
    review::{Evaluator, HandRecord, ReviewConfig, Reviewer, SessionReview, replay},
};
use ducy_play::{
    Bot, HandSummary, MatchConfig, Observation, TableRules,
    bots::{CallingStation, EquityBot},
    personality::Personality,
    run_match,
};

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

fn main() {
    let mut cards_path = String::new();
    let mut blueprint_path = String::new();
    let mut hands = 200usize;
    let mut only: Option<String> = None;
    let mut show = 0usize;
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut v = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--cards" => cards_path = v(),
            "--blueprint" => blueprint_path = v(),
            "--hands" => hands = v().replace('_', "").parse().expect("--hands"),
            "--only" => only = Some(v()),
            "--show" => show = v().parse().expect("--show"),
            f => panic!("unknown option {f}"),
        }
    }
    let cards = Arc::new(
        CardAbstraction::load(&std::fs::read(&cards_path).expect("read --cards"))
            .expect("abstraction"),
    );
    let config = HunlConfig::default();
    let game = Hunl::new(config.clone(), Some(&cards));
    let bytes = std::fs::read(&blueprint_path).expect("read --blueprint");
    let blueprint = Arc::new(Blueprint::load(&bytes, &game, &cards).expect("blueprint"));
    let tree = Arc::new(BettingTree::build(&config));
    let gto = |s: u64| GtoBot::from_parts(cards.clone(), blueprint.clone(), tree.clone(), s);

    type Make<'a> = Box<dyn Fn(u64) -> Box<dyn Bot> + 'a>;
    let mut players: Vec<(String, Make)> = vec![
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
        players.push((
            p.id().to_string(),
            Box::new(move |s| Box::new(p.bot(Some(s))) as Box<dyn Bot>),
        ));
    }
    let reviewer = Reviewer {
        tree: &tree,
        blueprint: &blueprint,
        cards: &cards,
        tree_big_blind: config.big_blind,
        config: ReviewConfig::default(),
    };
    let eval = Evaluator {
        tree: &tree,
        blueprint: &blueprint,
        cards: &cards,
        tree_big_blind: config.big_blind,
    };
    println!(
        "{:>14} {:>6} {:>9} {:>9} {:>9} {:>6} {:>6} {:>6} {:>6} {:>9} {:>9}",
        "player",
        "hands",
        "decisions",
        "lost/100",
        "lost/dec",
        "fine%",
        "inacc",
        "mist",
        "blund",
        "won/100",
        "ms/hand"
    );
    // Review time per decision by street: total ms and count.
    let mut street_ms = [(0f64, 0usize); 4];
    let mut shown = 0;
    for (name, make) in &players {
        if only.as_ref().is_some_and(|o| o != name) {
            continue;
        }
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let mut bots: Vec<Box<dyn Bot>> = vec![
            Box::new(gto(1)),
            Box::new(Recorder {
                bot: make(2),
                hands: recorded.clone(),
            }),
        ];
        let m = MatchConfig::new(TableRules::no_limit_holdem(1, 2), hands, 11)
            .with_starting_stack(config.stack);
        run_match(&m, &mut bots).expect("match");
        let mut cache = BucketCache::default();
        let t = Instant::now();
        let mut reviews = Vec::new();
        let summaries = recorded.lock().unwrap().clone();
        for s in &summaries {
            let Some(rec) = HandRecord::from_summary(s) else {
                continue;
            };
            let r = reviewer.review(&rec, &mut cache);
            if shown < show {
                shown += 1;
                println!("{r:#?}");
            }
            reviews.push(r);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / reviews.len().max(1) as f64;
        // Time the evaluator alone by street, on this opponent's first hands.
        for s in summaries.iter().take(20) {
            let Some(rec) = HandRecord::from_summary(s) else {
                continue;
            };
            let me = rec.side();
            let mut rng = ducy_gto::Rng::new(1);
            for d in replay(&rec, &tree, &cards, &blueprint, &mut cache, 1) {
                let Some(node) = d.node.filter(|_| d.player == me) else {
                    continue;
                };
                let board = &rec.board[..[0, 3, 4, 5][d.street].min(rec.board.len())];
                let t = Instant::now();
                let v = eval.values(
                    node,
                    me,
                    rec.hole,
                    board,
                    &d.ranges[1 - me],
                    &reviewer.config,
                    &mut rng,
                );
                if v.is_some() {
                    street_ms[d.street].0 += t.elapsed().as_secs_f64() * 1000.0;
                    street_ms[d.street].1 += 1;
                }
            }
        }
        let s = SessionReview::of(&reviews);
        println!(
            "{:>14} {:>6} {:>9} {:>9.1} {:>9.3} {:>5.0}% {:>6} {:>6} {:>6} {:>9.1} {:>9.0}",
            name,
            s.hands,
            s.decisions,
            s.bb_lost_per_100,
            s.bb_lost / s.decisions.max(1) as f64,
            100.0 * s.fine as f64 / s.decisions.max(1) as f64,
            s.inaccuracies,
            s.mistakes,
            s.blunders,
            100.0 * s.result / s.hands.max(1) as f64,
            ms
        );
    }
    println!("\nEvaluator time per decision, by street (one core):");
    for (street, (ms, n)) in ["preflop", "flop", "turn", "river"].iter().zip(street_ms) {
        if n > 0 {
            println!("{street:>8}: {:.0} ms over {n} decisions", ms / n as f64);
        }
    }
}
