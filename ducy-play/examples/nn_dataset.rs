//! Writes imitation data for the network bots (#216, stage 1): the
//! rule-based personality bots play mixed tables (2 to 9 seats, six for
//! six-card Omaha, 40 to 200 big blinds) and every decision is recorded with
//! its features and the action class chosen (`ducy_play::net_data`).
//!
//!     cargo run --release -p ducy-play --example nn_dataset -- \
//!         --hands 100000 --out data.bin --game all --threads 6
//!
//! `--game` is `holdem`, `plo4`, `plo6` or `all` (taking turns). Tables are
//! played on `--threads` workers (default half the cores); the records come
//! out in the order tables finish, so a run is reproducible per table, not
//! in file order. Reports the decisions written, the classes chosen overall
//! and per personality, and decisions a second.

use std::{
    cell::RefCell,
    fs::File,
    io::{BufWriter, Write},
    rc::Rc,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    time::Instant,
};

use ducy_play::{
    Bot, MatchConfig, TableRules, Variant,
    net::{ActionClass, NUM_CLASSES},
    net_data::{Record, Recorder, header},
    personality::Personality,
    run_match,
};
use rand::{RngExt, SeedableRng, rngs::StdRng};

/// Deals per table before new seats are drawn.
const DEALS_PER_TABLE: usize = 25;

fn rules(game: &str) -> TableRules {
    match game {
        "holdem" => TableRules::no_limit_holdem(1, 2),
        "plo4" => TableRules::pot_limit_omaha(1, 2),
        "plo6" => TableRules::pot_limit_omaha(1, 2).with_variant(Variant::Omaha { hole_cards: 6 }),
        g => panic!("unknown game {g}"),
    }
}

/// Plays table number `table` and returns its records and hands.
fn play_table(table: u64, games: &[&str], seed: u64) -> (Vec<Record>, usize) {
    let mut rng = StdRng::seed_from_u64(seed ^ table.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    let g = games[table as usize % games.len()];
    // Six-card Omaha deals to at most six, as the club tables do.
    let seats = rng.random_range(2..=if g == "plo6" { 6 } else { 9 });
    let n_people = Personality::ALL.len();
    // Random personalities, repeats allowed.
    let picks: Vec<usize> = (0..seats).map(|_| rng.random_range(0..n_people)).collect();
    let records = Rc::new(RefCell::new(Vec::<Record>::new()));
    let mut bots: Vec<Box<dyn Bot>> = picks
        .iter()
        .enumerate()
        .map(|(i, &p)| {
            let s = seed
                .wrapping_mul(1_000_003)
                .wrapping_add(table * 16 + i as u64);
            Box::new(Recorder::new(
                Personality::ALL[p].bot(Some(s)),
                p as u8,
                s,
                records.clone(),
            )) as Box<dyn Bot>
        })
        .collect();
    let mut config = MatchConfig::new(rules(g), DEALS_PER_TABLE, seed.wrapping_add(table * 7919));
    config.starting_stack = [40u64, 100, 200][rng.random_range(0..3usize)] * config.rules.big_blind;
    let result = run_match(&config, &mut bots).expect("match");
    drop(bots);
    let records = Rc::try_unwrap(records)
        .expect("the bots are gone")
        .into_inner();
    (records, result.hands)
}

fn main() {
    let mut hands = 10_000usize;
    let mut out = String::new();
    let mut seed = 1u64;
    let mut game = "all".to_string();
    let mut threads = std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).max(1));
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut v = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--hands" => hands = v().replace('_', "").parse().expect("--hands"),
            "--out" => out = v(),
            "--seed" => seed = v().parse().expect("--seed"),
            "--game" => game = v(),
            "--threads" => threads = v().parse().expect("--threads"),
            f => panic!("unknown option {f}"),
        }
    }
    assert!(!out.is_empty(), "--out FILE is required");
    let games: Vec<&str> = match game.as_str() {
        "all" => vec!["holdem", "plo4", "plo6"],
        g => vec![g],
    };
    let mut file = BufWriter::new(File::create(&out).expect("create --out"));
    file.write_all(&header()).expect("write");

    let n_people = Personality::ALL.len();
    let mut counts = vec![[0u64; NUM_CLASSES]; n_people];
    let played = AtomicUsize::new(0);
    let next_table = AtomicU64::new(0);
    let (mut written, mut tables) = (0u64, 0u64);
    let start = Instant::now();
    let (tx, rx) = mpsc::channel::<(Vec<Record>, usize)>();
    std::thread::scope(|s| {
        for _ in 0..threads.max(1) {
            let tx = tx.clone();
            let (played, next_table, games) = (&played, &next_table, &games);
            s.spawn(move || {
                while played.load(Ordering::Relaxed) < hands {
                    let table = next_table.fetch_add(1, Ordering::Relaxed);
                    let (records, n) = play_table(table, games, seed);
                    played.fetch_add(n, Ordering::Relaxed);
                    if tx.send((records, n)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        let mut buf = Vec::new();
        for (records, _) in rx {
            tables += 1;
            buf.clear();
            for r in &records {
                counts[r.personality as usize][r.class as usize] += 1;
                r.write(&mut buf);
                written += 1;
            }
            file.write_all(&buf).expect("write");
            if tables % 500 == 0 {
                eprintln!(
                    "{} hands, {written} decisions, {:.0}/s",
                    played.load(Ordering::Relaxed),
                    written as f64 / start.elapsed().as_secs_f64()
                );
            }
        }
    });
    file.flush().expect("flush");
    let secs = start.elapsed().as_secs_f64();
    println!(
        "{} hands at {tables} tables on {threads} threads: {written} decisions in {secs:.0}s ({:.0}/s) to {out}",
        played.load(Ordering::Relaxed),
        written as f64 / secs
    );
    let names: Vec<String> = ActionClass::ALL.iter().map(|c| format!("{c:?}")).collect();
    let pct = |row: &[u64; NUM_CLASSES]| {
        let total: u64 = row.iter().sum();
        row.iter()
            .map(|&n| format!("{:>10.1}", 100.0 * n as f64 / total.max(1) as f64))
            .collect::<String>()
    };
    println!(
        "{:>20} {:>9}{}",
        "",
        "n",
        names.iter().map(|n| format!("{n:>10}")).collect::<String>()
    );
    let mut all = [0u64; NUM_CLASSES];
    for (p, row) in counts.iter().enumerate() {
        for (a, n) in all.iter_mut().zip(row) {
            *a += n;
        }
        println!(
            "{:>20} {:>9}{}",
            Personality::ALL[p].name(),
            row.iter().sum::<u64>(),
            pct(row)
        );
    }
    println!(
        "{:>20} {:>9}{}",
        "all (%)",
        all.iter().sum::<u64>(),
        pct(&all)
    );
}
