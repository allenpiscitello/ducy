//! How often the six-handed PLO bot's trained postflop spots play (#290),
//! by the bot's position: the bot (`plo6s`) sits in one seat against five
//! random Regulars for `--deals` hands per position, and reports its
//! postflop decisions: not a heads-up pot, no trained spot for the line, off
//! the spot's tree, or played by the spot.
//!
//!     cargo run --release -p ducy-gto --example sixmax_plo_spot_debug -- \
//!         --cards plo-cards.bin --solve solve-r-100.bin --equity equity.bin \
//!         --spots a.bin,b.bin,… [--deals 2000]

use std::sync::Arc;

use ducy_gto::{
    Config, Discount, Mccfr,
    holdem::hunl::Buckets,
    omaha::abstraction::PloAbstraction,
    sixmax::{
        bot::{PreflopStrategy, SixMaxBot},
        equity::PreflopEquity,
        plo_spot::{PloSpotBot, PloSpotGame},
        preflop::{POSITIONS, PreflopGame},
        realize::Realization,
        solver::PreflopSolver,
    },
};
use ducy_play::{Bot, Deal, Hand, TableRules, Variant, personality::Personality, play_hand};

fn main() {
    let (mut cards_path, mut solve_path, mut equity_path) =
        (String::new(), String::new(), String::new());
    let mut spot_files: Vec<String> = Vec::new();
    let mut deals = 2000u64;
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut v = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--cards" => cards_path = v(),
            "--solve" => solve_path = v(),
            "--equity" => equity_path = v(),
            "--spots" => spot_files = v().split(',').map(str::to_string).collect(),
            "--deals" => deals = v().parse().expect("--deals"),
            f => panic!("unknown option {f}"),
        }
    }
    let cards: &'static PloAbstraction = Box::leak(Box::new(
        PloAbstraction::load(&std::fs::read(&cards_path).expect("read --cards"))
            .expect("abstraction"),
    ));
    let n = cards.bucket_count(0);
    let eq = PreflopEquity::from_bytes(&std::fs::read(&equity_path).expect("read --equity"))
        .expect("equity");
    let game = PreflopGame::pot_limit(100, Realization::uniform(n));
    let solver = PreflopSolver::new(&game, &eq, 1);
    solver
        .load(&std::fs::read(&solve_path).expect("read --solve"))
        .expect("a solve");
    let strategy = Arc::new(PreflopStrategy::from_solver(&game, &solver));
    let spots: Vec<(Arc<PloSpotGame<'static>>, Arc<ducy_gto::Profile<u64>>)> = spot_files
        .iter()
        .map(|file| {
            let line = file
                .rsplit('-')
                .next()
                .unwrap()
                .trim_end_matches(".bin")
                .to_string();
            let g = PloSpotGame::from_line(&strategy, &line, cards).expect("a line");
            let config = Config {
                seed: 1,
                batch: 4096,
                discount: Discount::DCFR,
                prune: None,
            };
            let profile = {
                let m = Mccfr::load(&g, config, &std::fs::read(file).expect("read spot"))
                    .expect("spot");
                Arc::new(m.average())
            };
            println!(
                "{} ({line}): positions {:?}, raises {}, aggressor {:?}",
                g.name, g.positions, g.raises, g.aggressor
            );
            (Arc::new(g), profile)
        })
        .collect();
    let rules = TableRules::pot_limit_omaha(1, 2).with_variant(Variant::Omaha {
        hole_cards: cards.config.hole_cards as u32,
    });
    println!("\nposition  seen  not-HU  no-spot  off-tree  played");
    for (pos, name) in POSITIONS.iter().enumerate() {
        let mut hero = SixMaxBot::new(strategy.clone(), Personality::DougPoker.bot(Some(7)), 7)
            .with_plo_buckets(Arc::new(cards.clone()))
            .with_plo_spots(PloSpotBot::new(spots.clone(), 5));
        for d in 0..deals {
            let button = (d as usize) % 6;
            // The hero's seat for its position: position = (seat - button + 3) % 6.
            let seat = (pos + button + 3) % 6;
            let mut others: Vec<Box<dyn Bot>> = (0..5)
                .map(|i| {
                    let p = Personality::ALL[(d as usize * 5 + i) % Personality::ALL.len()];
                    Box::new(p.bot(Some(d * 10 + i as u64))) as Box<dyn Bot>
                })
                .collect();
            let deal = Deal::random(rules.variant, 6, Some(d * 31 + pos as u64)).expect("deal");
            let mut hand = Hand::new(rules, &[200; 6], button, deal).expect("hand");
            let mut seated: Vec<&mut dyn Bot> = Vec::with_capacity(6);
            let mut o = others.iter_mut();
            let mut me = Some(&mut hero as &mut dyn Bot);
            for s in 0..6 {
                if s == seat {
                    seated.push(me.take().expect("one hero seat"));
                } else {
                    seated.push(&mut **o.next().unwrap());
                }
            }
            play_hand(&mut hand, &mut seated).expect("play");
        }
        let s = hero.plo_spot_stats().unwrap_or_default();
        println!(
            "{:<8} {:>5} {:>7} {:>8} {:>9} {:>7}",
            name, s.seen, s.not_heads_up, s.no_spot, s.off_tree, s.played
        );
    }
}
