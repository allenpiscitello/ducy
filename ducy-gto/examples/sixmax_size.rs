//! Sizes the six-handed game to the river (#338) before any long run:
//! betting-tree counts, information sets and memory for several menus and
//! bucket counts, and, given card abstractions, a short timed run.
//!
//!     cargo run --release -p ducy-gto --example sixmax_size -- \
//!         --cards holdem-cards.bin --plo-cards plo-cards.bin --secs 120
//!
//! Options (all optional):
//!
//! - `--cards FILE`: a Hold'em card abstraction; times a Hold'em run, and a
//!   heads-up one to calibrate the training-time estimate
//! - `--plo-cards FILE`: a PLO card abstraction (any hole-card count);
//!   times a PLO run
//! - `--secs N`: seconds per timed run (default 60)
//! - `--menu NAME`: only this menu (`MultiwayConfig::presets`)
//! - `--limit N`: skip trees with more nodes than this (default 1.5e9)
//! - `--hu-iters N`: iterations at which the heads-up blueprint plays well,
//!   for the estimate (default 300,000,000: the live 100 BB blueprint)
//! - `--betting-only`: also time each menu with every hand in one bucket,
//!   to see what the cards cost
//!
//! Memory is estimated per information set (the solver's index and slot,
//! about 36 bytes) plus 8 bytes per action (an f32 regret and strategy sum),
//! times 1.25 for the slack of tables that grow by doubling (measured: a
//! Hold'em run held 1.4 GB for 20.8M information sets, a PLO4 run 2.2 GB for
//! 33.4M); a checkpoint takes 9 bytes per information set plus 8 per action,
//! a blueprint 9 plus 1 per action. The
//! counts are upper bounds: the solver only stores information sets it
//! reaches, and lines nobody plays are never reached. The timed runs show
//! how many it touches.
//!
//! Training time is estimated from information-set visits (one per decision
//! a traversal passes), against the full count: at 1,000 visits per
//! information set, and at as many as the heads-up blueprint gets in
//! `--hu-iters` iterations. Both are rough: visits fall mostly on early
//! streets and common lines.

use std::{
    sync::atomic::{AtomicU64, Ordering::Relaxed},
    time::Instant,
};

use ducy_gto::{
    Config, Discount, Game, Mccfr, Rng, Turn,
    holdem::{
        abstraction::CardAbstraction,
        hunl::{Buckets, Hunl, HunlConfig},
    },
    omaha::abstraction::PloAbstraction,
    sixmax::multiway::{MultiwayConfig, MultiwayGame, MultiwayTree, MwStats},
};

/// Bytes per stored information set besides its actions: the hash index
/// entry with its load factor (~28) and the slot (8).
const PER_INFOSET: f64 = 36.0;
/// Growth slack on top of the exact table size.
const SLACK: f64 = 1.25;

/// A game that counts information-set visits.
struct Counting<'g, G> {
    game: &'g G,
    visits: AtomicU64,
}

impl<G: Game> Game for Counting<'_, G> {
    type State = G::State;
    type Info = G::Info;
    fn root(&self) -> G::State {
        self.game.root()
    }
    fn turn(&self, s: &G::State) -> Turn {
        self.game.turn(s)
    }
    fn utility(&self, s: &G::State) -> f64 {
        self.game.utility(s)
    }
    fn num_players(&self) -> usize {
        self.game.num_players()
    }
    fn utility_for(&self, s: &G::State, p: usize) -> f64 {
        self.game.utility_for(s, p)
    }
    fn chance_outcomes(&self, s: &G::State) -> Vec<(G::State, f64)> {
        self.game.chance_outcomes(s)
    }
    fn sample_chance(&self, s: &G::State, rng: &mut Rng) -> G::State {
        self.game.sample_chance(s, rng)
    }
    fn shared_deal(&self) -> bool {
        self.game.shared_deal()
    }
    fn num_actions(&self, s: &G::State) -> usize {
        self.game.num_actions(s)
    }
    fn apply(&self, s: &G::State, a: usize) -> G::State {
        self.game.apply(s, a)
    }
    fn info(&self, s: &G::State) -> G::Info {
        self.visits.fetch_add(1, Relaxed);
        self.game.info(s)
    }
}

/// What a timed run measured.
struct Timed {
    iters_per_sec: f64,
    visits_per_sec: f64,
    visits_per_iter: f64,
    infosets: usize,
    table_bytes: usize,
}

fn timed<G>(game: &G, secs: f64) -> Timed
where
    G: Game + Sync,
    G::State: Send,
    G::Info: ducy_gto::Key,
{
    let counting = Counting {
        game,
        visits: AtomicU64::new(0),
    };
    let config = Config {
        seed: 1,
        batch: 4096,
        discount: Discount::DCFR,
        prune: None,
    };
    let mut m = Mccfr::new(&counting, config);
    let start = Instant::now();
    // Warm up for a fifth of the time, then measure.
    while start.elapsed().as_secs_f64() < secs / 5.0 {
        m.run(4096);
    }
    let (i0, v0, t0) = (
        m.iterations(),
        counting.visits.load(Relaxed),
        Instant::now(),
    );
    while start.elapsed().as_secs_f64() < secs {
        m.run(4096);
    }
    let dt = t0.elapsed().as_secs_f64();
    let iters = (m.iterations() - i0) as f64;
    let visits = (counting.visits.load(Relaxed) - v0) as f64;
    Timed {
        iters_per_sec: iters / dt,
        visits_per_sec: visits / dt,
        visits_per_iter: visits / iters,
        infosets: m.num_infosets(),
        table_bytes: m.table_bytes(),
    }
}

fn gb(bytes: f64) -> String {
    format!("{:.1} GB", bytes / 1e9)
}

fn m(n: u64) -> String {
    format!("{:.1}M", n as f64 / 1e6)
}

/// Memory and checkpoint size for these tree counts and buckets.
fn sizes(stats: &MwStats, buckets: [u64; 4]) -> (u64, f64, f64) {
    let infosets: u64 = stats.infosets(buckets).iter().sum();
    let entries = stats.entries(buckets);
    let memory =
        (infosets as f64 * PER_INFOSET + entries as f64 * 8.0) * SLACK + stats.nodes as f64 * 8.0;
    let checkpoint = infosets as f64 * 9.0 + entries as f64 * 8.0;
    (infosets, memory, checkpoint)
}

fn main() {
    let (mut cards, mut plo_cards, mut secs, mut only, mut limit, mut hu_iters) =
        (None, None, 60.0, None, 1_500_000_000u64, 300_000_000f64);
    let mut betting_only = false;
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--betting-only" => betting_only = true,
            "--cards" => cards = Some(value()),
            "--plo-cards" => plo_cards = Some(value()),
            "--secs" => secs = value().parse().expect("--secs"),
            "--menu" => only = Some(value()),
            "--limit" => limit = value().replace('_', "").parse().expect("--limit"),
            "--hu-iters" => hu_iters = value().replace('_', "").parse().expect("--hu-iters"),
            f => panic!("unknown option {f}"),
        }
    }

    println!("## Betting trees (100 BB, six seats)\n");
    println!(
        "| menu | nodes | flop / turn / river decisions (2 players) | same, 3+ players | preflop decisions | build |"
    );
    println!("|---|---|---|---|---|---|");
    let mut counted = Vec::new();
    for (name, c) in MultiwayConfig::presets(100) {
        if only.as_deref().is_some_and(|o| o != name) {
            continue;
        }
        let t = Instant::now();
        let Some(s) = MultiwayTree::count(&c, limit) else {
            println!("| {name} | > {} | | | | |", m(limit));
            continue;
        };
        let hu = |st: usize| s.by_players[st][2];
        let multi = |st: usize| s.by_players[st][3..].iter().sum::<u64>();
        println!(
            "| {name} | {} | {} / {} / {} | {} / {} / {} | {} | {:.0} s |",
            m(s.nodes),
            m(hu(1)),
            m(hu(2)),
            m(hu(3)),
            m(multi(1)),
            m(multi(2)),
            m(multi(3)),
            s.sequences[0],
            t.elapsed().as_secs_f64()
        );
        counted.push((name, c, s));
    }

    println!("\n## Information sets (upper bound) and memory\n");
    println!("| menu | buckets (pre / flop / turn / river) | infosets | memory | checkpoint |");
    println!("|---|---|---|---|---|");
    for (name, c, s) in &counted {
        // Hold'em: the 169 classes, then 50 to 200 buckets. PLO: down to the
        // nuts-first abstractions' 500 / 500 / 500 / 200.
        let options: [[u64; 4]; 3] = if c.pot_limit {
            [
                [200, 100, 100, 100],
                [500, 200, 200, 200],
                [500, 500, 500, 200],
            ]
        } else {
            [
                [169, 50, 50, 50],
                [169, 100, 100, 100],
                [169, 200, 200, 200],
            ]
        };
        for b in options {
            let (n, mem, ck) = sizes(s, b);
            println!(
                "| {name} | {} / {} / {} / {} | {} | {} | {} |",
                b[0],
                b[1],
                b[2],
                b[3],
                m(n),
                gb(mem),
                gb(ck)
            );
        }
    }

    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let holdem_cards = cards.map(|path| {
        CardAbstraction::load(&std::fs::read(path).expect("read --cards"))
            .expect("a card abstraction")
    });
    let plo_cards = plo_cards.map(|path| {
        PloAbstraction::load(&std::fs::read(path).expect("read --plo-cards"))
            .expect("a PLO card abstraction")
    });
    let mut hu_visits = None;
    if let Some(ca) = &holdem_cards {
        let hunl = Hunl::new(HunlConfig::default(), Some(ca));
        let infosets: u64 = hunl.tree_stats().infosets(ca).iter().sum();
        let t = timed(&hunl, secs);
        // Visits each heads-up information set gets by `hu_iters`.
        let per = hu_iters * t.visits_per_iter / infosets as f64;
        println!(
            "\n## Heads-up calibration ({threads} threads)\n\n{:.0} iterations/s, {:.0} visits per \
             iteration, {} visits/s; {} information sets; {:.0} visits each by {} iterations",
            t.iters_per_sec,
            t.visits_per_iter,
            m(t.visits_per_sec as u64),
            m(infosets),
            per,
            m(hu_iters as u64)
        );
        hu_visits = Some(per);
    }
    if holdem_cards.is_none() && plo_cards.is_none() && !betting_only {
        return;
    }

    println!("\n## Timed runs ({threads} threads, {secs:.0} s each)\n");
    println!(
        "| menu | buckets | iterations/s | visits/s | visits/iteration | infosets touched | their memory \
         | full count | hours at 1,000 visits each | hours at heads-up's visits |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|");
    let row = |name: &str, b: [u64; 4], t: Timed, s: &MwStats| {
        let (full, _, _) = sizes(s, b);
        let touched_mem = (t.infosets as f64 * PER_INFOSET + t.table_bytes as f64) * SLACK;
        let hours = |per: f64| format!("{:.0}", per * full as f64 / t.visits_per_sec / 3600.0);
        println!(
            "| {name} | {} / {} / {} / {} | {:.0} | {} | {:.0} | {} | {} | {} | {} | {} |",
            b[0],
            b[1],
            b[2],
            b[3],
            t.iters_per_sec,
            m(t.visits_per_sec as u64),
            t.visits_per_iter,
            m(t.infosets as u64),
            gb(touched_mem),
            m(full),
            hours(1000.0),
            hu_visits.map_or("-".to_string(), hours),
        );
    };
    for (name, c, s) in &counted {
        if c.pot_limit {
            if let Some(pa) = &plo_cards {
                let t = match pa.config.hole_cards {
                    4 => timed(&MultiwayGame::<_, 4>::new(c.clone(), Some(pa)), secs),
                    5 => timed(&MultiwayGame::<_, 5>::new(c.clone(), Some(pa)), secs),
                    _ => timed(&MultiwayGame::<_, 6>::new(c.clone(), Some(pa)), secs),
                };
                let label = format!("{name} (PLO{})", pa.config.hole_cards);
                row(&label, bucket_counts(pa), t, s);
            }
        } else if let Some(ca) = &holdem_cards {
            let t = timed(&MultiwayGame::<_, 2>::new(c.clone(), Some(ca)), secs);
            row(name, bucket_counts(ca), t, s);
        }
        if betting_only {
            let game = MultiwayGame::<CardAbstraction, 2>::new(c.clone(), None);
            let t = timed(&game, secs);
            row(&format!("{name} (no cards)"), [1; 4], t, s);
        }
    }
}

fn bucket_counts(b: &impl Buckets) -> [u64; 4] {
    [0usize, 3, 4, 5].map(|n| b.bucket_count(n) as u64)
}
