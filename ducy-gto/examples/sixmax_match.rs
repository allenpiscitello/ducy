//! Six-handed duplicate matches (#247), and how often six-handed pots reach
//! the flop with two players (#246).
//!
//!     cargo run --release -p ducy-gto --example sixmax_match -- \
//!         --bots doug_poker,gto --deals 5000 [--stack 100] \
//!         [--cards cards.bin --blueprint blueprint.bin]
//!
//! Each deal seats five Regulars, picked at random for that deal, and the
//! measured bot. The measured bot plays the deal six times, once from each
//! seat, with the five Regulars keeping their order around the rest of the
//! table; every bot in `--bots` plays exactly the same deals, seats and
//! opponents, so they can be compared pair by pair. Every bot starts every
//! hand fresh, with `--stack` big blinds (default 100).
//!
//! `--bots` takes personality ids, `equity` (EquityBot) and `gto` (GtoBot,
//! which needs `--cards` and `--blueprint`; at six seats it plays its
//! fallback rule) and `gto6` (the six-handed bot: the preflop solve before
//! the flop, Doug Poker after it; needs `--solve` and `--equity`, with
//! `--realize` if the solve used it, and `--solve-bb`, default 100). The
//! first bot is the one the others are compared with. `gto6@FILE` (and
//! `gto6hu@FILE`) play the solve in FILE instead of `--solve`, so two solves
//! can be compared on the same deals. `--progress FILE` writes a small JSON
//! status file every half minute.
//!
//! `--flops` instead plays tables of six random Regulars and reports how
//! many players see the flop, by how the preflop went.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
    },
    thread,
    time::{Duration, Instant},
};

/// Deals played so far, over all bots, for `--progress`.
static PLAYED: AtomicU64 = AtomicU64::new(0);

use ducy_gto::{
    holdem::{
        abstraction::CardAbstraction,
        blueprint::Blueprint,
        bot::GtoBot,
        hunl::{BettingTree, Hunl, HunlConfig},
    },
    sixmax::{
        bot::{PreflopStrategy, SixMaxBot},
        equity::PreflopEquity,
        preflop::PreflopGame,
        realize::Realization,
        solver::PreflopSolver,
    },
};
use ducy_play::{
    Bot, Deal, Event, Hand, Street, TableRules, bots::EquityBot, personality::Personality,
    play_hand,
};

const SEATS: usize = 6;
/// Positions by seats after the button.
const POSITIONS: [&str; SEATS] = ["BTN", "SB", "BB", "UTG", "HJ", "CO"];

type Make = Arc<dyn Fn(u64) -> Box<dyn Bot> + Send + Sync>;

/// A small deterministic generator for lineups: splitmix64.
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// `k` different Regulars for deal `d`.
fn lineup(d: u64, k: usize) -> Vec<Personality> {
    let mut pool: Vec<Personality> = Personality::ALL.to_vec();
    let mut out = Vec::with_capacity(k);
    let mut x = d.wrapping_mul(0x2545_f491_4f6c_dd1d) ^ 0x5eed;
    for _ in 0..k {
        x = mix(x);
        out.push(pool.swap_remove((x % pool.len() as u64) as usize));
    }
    out
}

fn regular(p: Personality) -> Make {
    Arc::new(move |s| Box::new(p.bot(Some(s))) as Box<dyn Bot>)
}

/// The measured bot's chips over one deal from each seat, by position.
fn play_deal(make: &Make, rules: TableRules, stack: u64, d: u64, seed: u64) -> [i64; SEATS] {
    let deal = Deal::random(rules.variant, SEATS, Some(seed.wrapping_add(d))).expect("deal");
    let button = (d as usize) % SEATS;
    let opponents = lineup(d ^ seed, SEATS - 1);
    let mut by_position = [0i64; SEATS];
    for hero in 0..SEATS {
        let mut bots: Vec<Box<dyn Bot>> = Vec::with_capacity(SEATS);
        let mut o = 0;
        for seat in 0..SEATS {
            let s = mix(seed ^ (d << 8) ^ seat as u64);
            if seat == hero {
                bots.push(make(s));
            } else {
                bots.push(regular(opponents[o])(s));
                o += 1;
            }
        }
        let mut hand = Hand::new(rules, &[stack; SEATS], button, deal.clone()).expect("hand");
        let mut seated: Vec<&mut dyn Bot> = Vec::with_capacity(SEATS);
        for bot in bots.iter_mut() {
            seated.push(&mut **bot);
        }
        let outcome = play_hand(&mut hand, &mut seated).expect("play");
        let position = (hero + SEATS - button) % SEATS;
        by_position[position] += outcome.result.net[hero];
    }
    by_position
}

struct Stats {
    /// Per deal: chips over the six seats.
    per_deal: Vec<i64>,
    by_position: [i64; SEATS],
}

fn run(make: &Make, rules: TableRules, stack: u64, deals: u64, seed: u64, threads: u64) -> Stats {
    let chunks: Vec<Vec<(i64, [i64; SEATS])>> = thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let make = make.clone();
                scope.spawn(move || {
                    (t..deals)
                        .step_by(threads as usize)
                        .map(|d| {
                            let p = play_deal(&make, rules, stack, d, seed);
                            PLAYED.fetch_add(1, Relaxed);
                            (p.iter().sum(), p)
                        })
                        .collect()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("thread"))
            .collect()
    });
    // Back in deal order, so bots can be paired deal by deal.
    let mut rows: Vec<(u64, i64, [i64; SEATS])> = Vec::with_capacity(deals as usize);
    for (t, chunk) in chunks.into_iter().enumerate() {
        for (i, (sum, p)) in chunk.into_iter().enumerate() {
            rows.push((t as u64 + i as u64 * threads, sum, p));
        }
    }
    rows.sort_by_key(|r| r.0);
    let mut by_position = [0i64; SEATS];
    for (_, _, p) in &rows {
        for (a, b) in by_position.iter_mut().zip(p) {
            *a += b;
        }
    }
    Stats {
        per_deal: rows.into_iter().map(|r| r.1).collect(),
        by_position,
    }
}

/// bb/100 and its 95% interval from per-deal totals (six hands each).
fn rate(per_deal: &[f64], bb: f64) -> (f64, f64) {
    let n = per_deal.len() as f64;
    let mean = per_deal.iter().sum::<f64>() / n;
    let var = per_deal.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0);
    let per_hand = |x: f64| x / SEATS as f64 / bb * 100.0;
    (per_hand(mean), per_hand(1.96 * (var / n).sqrt()))
}

fn flops(rules: TableRules, stack: u64, deals: u64, seed: u64) {
    // [pot type][players at the flop]
    let kinds = ["limped", "single-raised", "3-bet", "4-bet+"];
    let mut counts = [[0u64; SEATS + 1]; 4];
    let mut no_flop = 0u64;
    for d in 0..deals {
        let deal = Deal::random(rules.variant, SEATS, Some(seed.wrapping_add(d))).expect("deal");
        let mut bots: Vec<Box<dyn Bot>> = lineup(d ^ seed, SEATS)
            .into_iter()
            .enumerate()
            .map(|(seat, p)| regular(p)(mix(seed ^ (d << 8) ^ seat as u64)))
            .collect();
        let mut hand = Hand::new(rules, &[stack; SEATS], d as usize % SEATS, deal).expect("hand");
        let mut seated: Vec<&mut dyn Bot> = Vec::with_capacity(SEATS);
        for bot in bots.iter_mut() {
            seated.push(&mut **bot);
        }
        play_hand(&mut hand, &mut seated).expect("play");
        let mut raises = 0;
        let mut folds = 0;
        let mut flop = false;
        for e in hand.events() {
            match e {
                Event::Raise { .. } => raises += 1,
                Event::Fold { .. } => folds += 1,
                Event::Board {
                    street: Street::Flop,
                    ..
                } => {
                    flop = true;
                    break;
                }
                _ => {}
            }
        }
        if flop {
            counts[raises.min(3)][SEATS - folds] += 1;
        } else {
            no_flop += 1;
        }
    }
    println!(
        "{deals} hands of six random Regulars, {} BB deep",
        stack / rules.big_blind
    );
    println!(
        "no flop (won preflop): {:.1}%",
        100.0 * no_flop as f64 / deals as f64
    );
    println!(
        "{:>14} {:>8} {:>8} {:>8} {:>8} {:>8}",
        "pot", "flops", "2 in", "3 in", "4+ in", "of hands"
    );
    for (k, row) in kinds.iter().zip(counts) {
        let total: u64 = row.iter().sum();
        if total == 0 {
            continue;
        }
        let pct = |c: u64| 100.0 * c as f64 / total as f64;
        let four: u64 = row[4..].iter().sum();
        println!(
            "{k:>14} {total:>8} {:>7.1}% {:>7.1}% {:>7.1}% {:>7.1}%",
            pct(row[2]),
            pct(row[3]),
            pct(four),
            100.0 * total as f64 / deals as f64
        );
    }
}

fn main() {
    let mut names = String::from("doug_poker");
    let mut deals = 2_000u64;
    let mut stack_bb = 100u64;
    let mut seed = 1u64;
    let mut cards_path = None;
    let mut blueprint_path = None;
    let mut flop_mode = false;
    let mut solve_path: Option<String> = None;
    let mut solve_bb = 100u32;
    let mut equity_path: Option<String> = None;
    let mut realize_path: Option<String> = None;
    let mut progress: Option<String> = None;
    let mut threads = thread::available_parallelism().map_or(4, |n| n.get()) as u64;
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut v = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--bots" => names = v(),
            "--deals" => deals = v().parse().expect("--deals"),
            "--stack" => stack_bb = v().parse().expect("--stack"),
            "--seed" => seed = v().parse().expect("--seed"),
            "--threads" => threads = v().parse().expect("--threads"),
            "--cards" => cards_path = Some(v()),
            "--blueprint" => blueprint_path = Some(v()),
            "--flops" => flop_mode = true,
            "--solve" => solve_path = Some(v()),
            "--solve-bb" => solve_bb = v().parse().expect("--solve-bb"),
            "--equity" => equity_path = Some(v()),
            "--realize" => realize_path = Some(v()),
            "--progress" => progress = Some(v()),
            f => panic!("unknown option {f}"),
        }
    }
    let config = HunlConfig::default();
    let rules = TableRules::no_limit_holdem(config.small_blind, config.big_blind);
    let stack = stack_bb * config.big_blind;
    if flop_mode {
        flops(rules, stack, deals, seed);
        return;
    }

    let mut bots: Vec<(String, Make)> = Vec::new();
    for name in names.split(',') {
        let (kind, solve_file) = match name.split_once('@') {
            Some((k, f)) => (k, Some(f.to_string())),
            None => (name, solve_path.clone()),
        };
        let make: Make = match kind {
            "equity" => Arc::new(|s| Box::new(EquityBot::new(200, Some(s))) as Box<dyn Bot>),
            "gto6" | "gto6hu" => {
                // gto6hu also plays heads-up pots after the flop with the
                // heads-up blueprint (--cards, --blueprint).
                let heads_up = (kind == "gto6hu").then(|| {
                    let cards = Arc::new(
                        CardAbstraction::load(
                            &std::fs::read(cards_path.as_ref().expect("gto6hu needs --cards"))
                                .expect("read --cards"),
                        )
                        .expect("abstraction"),
                    );
                    let game = Hunl::new(config.clone(), Some(&cards));
                    let bytes =
                        std::fs::read(blueprint_path.as_ref().expect("gto6hu needs --blueprint"))
                            .expect("read --blueprint");
                    let blueprint =
                        Arc::new(Blueprint::load(&bytes, &game, &cards).expect("blueprint"));
                    (cards, blueprint, Arc::new(BettingTree::build(&config)))
                });
                let eq = PreflopEquity::from_bytes(
                    &std::fs::read(equity_path.as_ref().expect("gto6 needs --equity"))
                        .expect("read --equity"),
                )
                .expect("equity table");
                let realize = realize_path
                    .as_ref()
                    .map_or_else(Realization::default, |p| {
                        Realization::from_bytes(&std::fs::read(p).expect("read --realize"))
                            .expect("realization table")
                    });
                let game = PreflopGame::with_realization(solve_bb, realize);
                let solver = PreflopSolver::new(&game, &eq, 1);
                solver
                    .load(
                        &std::fs::read(solve_file.as_ref().expect("gto6 needs --solve"))
                            .expect("read --solve"),
                    )
                    .expect("a solve for this game");
                let strategy = Arc::new(PreflopStrategy::from_solver(&game, &solver));
                Arc::new(move |s| {
                    let mut bot =
                        SixMaxBot::new(strategy.clone(), Personality::DougPoker.bot(Some(s)), s);
                    if let Some((cards, blueprint, tree)) = &heads_up {
                        bot = bot.with_heads_up(Box::new(GtoBot::from_parts(
                            cards.clone(),
                            blueprint.clone(),
                            tree.clone(),
                            s ^ 0x6,
                        )));
                    }
                    Box::new(bot) as Box<dyn Bot>
                })
            }
            "gto" => {
                let cards = Arc::new(
                    CardAbstraction::load(
                        &std::fs::read(cards_path.as_ref().expect("gto needs --cards"))
                            .expect("read --cards"),
                    )
                    .expect("abstraction"),
                );
                let game = Hunl::new(config.clone(), Some(&cards));
                let bytes = std::fs::read(blueprint_path.as_ref().expect("gto needs --blueprint"))
                    .expect("read --blueprint");
                let blueprint =
                    Arc::new(Blueprint::load(&bytes, &game, &cards).expect("blueprint"));
                let tree = Arc::new(BettingTree::build(&config));
                Arc::new(move |s| {
                    Box::new(GtoBot::from_parts(
                        cards.clone(),
                        blueprint.clone(),
                        tree.clone(),
                        s,
                    )) as Box<dyn Bot>
                })
            }
            id => regular(Personality::from_name(id).unwrap_or_else(|| panic!("unknown bot {id}"))),
        };
        // A solve's file by its name alone, so the table stays readable.
        let shown = match name.split_once('@') {
            Some((k, f)) => format!("{k}@{}", f.rsplit('/').next().unwrap_or(f)),
            None => name.to_string(),
        };
        bots.push((shown, make));
    }

    let finished = Arc::new(AtomicBool::new(false));
    let reporter = progress.map(|path| {
        let total = deals * bots.len() as u64;
        let finished = finished.clone();
        thread::spawn(move || {
            let t = Instant::now();
            while !finished.load(Relaxed) {
                let done = PLAYED.load(Relaxed);
                let rate = done as f64 / t.elapsed().as_secs_f64().max(1.0);
                let eta = (total - done.min(total)) as f64 / rate.max(1e-9) / 60.0;
                let json = format!(
                    "{{\"label\": \"6-max: duplicate matches against the Regulars\", \"done\": {done}, \"total\": {total}, \"unit\": \"deals\", \"etaMin\": {eta:.0}}}\n"
                );
                let _ = std::fs::write(&path, json);
                for _ in 0..30 {
                    if finished.load(Relaxed) {
                        break;
                    }
                    thread::sleep(Duration::from_secs(1));
                }
            }
        })
    });

    let bb = config.big_blind as f64;
    println!(
        "{deals} deals × 6 seats against five random Regulars, {stack_bb} BB deep (bb/100 won, ± 95%)\n"
    );
    print!("{:>18} {:>14}", "bot", "bb/100");
    for p in POSITIONS {
        print!(" {p:>7}");
    }
    println!(" {:>16} {:>16}", "vs first", "vs second");
    let diff = |a: &[i64], b: &[i64]| {
        let d: Vec<f64> = a.iter().zip(b).map(|(x, y)| (x - y) as f64).collect();
        rate(&d, bb)
    };
    let mut results: Vec<Vec<i64>> = Vec::new();
    for (name, make) in &bots {
        let s = run(make, rules, stack, deals, seed, threads);
        let per_deal: Vec<f64> = s.per_deal.iter().map(|&x| x as f64).collect();
        let (r, ci) = rate(&per_deal, bb);
        print!("{name:>18} {:>7.1} ± {:<4.1}", r, ci);
        for c in s.by_position {
            print!(" {:>+7.1}", c as f64 / bb / deals as f64 * 100.0);
        }
        // Paired with the first bot, and from the third on with the second.
        for other in results.iter().take(2) {
            let (d, dci) = diff(&s.per_deal, other);
            print!(" {:>+7.1} ± {:<5.1}", d, dci);
        }
        println!();
        results.push(s.per_deal);
    }
    finished.store(true, Relaxed);
    if let Some(r) = reporter {
        r.join().expect("reporter");
    }
}
