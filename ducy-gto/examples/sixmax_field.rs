//! Records how the Regulars play preflop at six-handed tables (#245,
//! option B): every preflop decision on the six-handed tree, by hand class,
//! from tables of six Regulars picked at random per hand, the way
//! `sixmax_match` seats them.
//!
//!     cargo run --release -p ducy-gto --example sixmax_field -- \
//!         --hands 20000000 --stack 100 --out field-100.bin [--seed 1000] \
//!         [--every 1000000] [--progress progress.json]
//!
//! An existing `--out` is resumed: its counts are kept and added to. Use a
//! different `--seed` from the matches that measure the result, so the
//! field is recorded on other deals. `--progress` writes a small JSON status
//! file at every report.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering::Relaxed},
    },
    thread,
    time::Instant,
};

use ducy_gto::{
    holdem::{cards::Card, cards::from_ducy, hunl::HunlConfig},
    sixmax::{field::FieldCounts, preflop::PreflopGame},
};
use ducy_play::{Bot, Deal, Hand, TableRules, personality::Personality, play_hand};

const SEATS: usize = 6;

/// A small deterministic generator for lineups: splitmix64.
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// Six different Regulars for hand `d`.
fn lineup(d: u64) -> Vec<Personality> {
    let mut pool: Vec<Personality> = Personality::ALL.to_vec();
    let mut out = Vec::with_capacity(SEATS);
    let mut x = d.wrapping_mul(0x2545_f491_4f6c_dd1d) ^ 0x5eed;
    for _ in 0..SEATS {
        x = mix(x);
        out.push(pool.swap_remove((x % pool.len() as u64) as usize));
    }
    out
}

fn play(
    rules: TableRules,
    stack: u64,
    d: u64,
    seed: u64,
    game: &PreflopGame,
    counts: &FieldCounts,
) -> usize {
    let deal = Deal::random(rules.variant, SEATS, Some(seed.wrapping_add(d))).expect("deal");
    let button = d as usize % SEATS;
    let mut bots: Vec<Box<dyn Bot>> = lineup(d ^ seed)
        .into_iter()
        .enumerate()
        .map(|(seat, p)| Box::new(p.bot(Some(mix(seed ^ (d << 8) ^ seat as u64)))) as Box<dyn Bot>)
        .collect();
    let mut hand = Hand::new(rules, &[stack; SEATS], button, deal).expect("hand");
    let mut seated: Vec<&mut dyn Bot> = Vec::with_capacity(SEATS);
    for bot in bots.iter_mut() {
        seated.push(&mut **bot);
    }
    play_hand(&mut hand, &mut seated).expect("play");
    drop(seated);
    let holes: Vec<[Card; 2]> = (0..SEATS)
        .map(|s| {
            let c: Vec<Card> = hand
                .hole_cards(s)
                .expect("cards")
                .iter(false)
                .map(from_ducy)
                .collect();
            [c[0], c[1]]
        })
        .collect();
    counts.record(game, hand.events(), button, &holes, |_| true)
}

fn main() {
    let mut hands = 1_000_000u64;
    let mut stack_bb = 100u32;
    let mut seed = 1000u64;
    let mut every = 1_000_000u64;
    let mut out = String::from("field.bin");
    let mut progress: Option<String> = None;
    let mut threads = thread::available_parallelism().map_or(4, |n| n.get()) as u64;
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut v = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--hands" => hands = v().replace('_', "").parse().expect("--hands"),
            "--stack" => stack_bb = v().parse().expect("--stack"),
            "--seed" => seed = v().parse().expect("--seed"),
            "--every" => every = v().replace('_', "").parse().expect("--every"),
            "--out" => out = v(),
            "--progress" => progress = Some(v()),
            "--threads" => threads = v().parse().expect("--threads"),
            f => panic!("unknown option {f}"),
        }
    }
    let config = HunlConfig::default();
    let rules = TableRules::no_limit_holdem(config.small_blind, config.big_blind);
    let stack = stack_bb as u64 * config.big_blind;
    let game = PreflopGame::new(stack_bb);
    let counts = Arc::new(match std::fs::read(&out) {
        Ok(bytes) => {
            FieldCounts::from_bytes(&game, &bytes).expect("--out holds counts for this tree")
        }
        Err(_) => FieldCounts::new(&game),
    });
    // Hands already recorded into --out, from its decisions (about 6 a hand),
    // so a resumed run plays new deals.
    let start = counts.total() / 6;
    println!(
        "{hands} hands of six random Regulars, {stack_bb} BB deep, {} decisions in the tree, {threads} threads{}",
        game.decisions,
        if start > 0 {
            format!(", resuming after about {start} hands")
        } else {
            String::new()
        }
    );
    let t = Instant::now();
    let mut done = 0u64;
    while done < hands {
        let step = every.min(hands - done);
        let next = Arc::new(AtomicU64::new(0));
        let recorded = Arc::new(AtomicU64::new(0));
        thread::scope(|scope| {
            for _ in 0..threads {
                let (next, recorded, counts, game) =
                    (next.clone(), recorded.clone(), counts.clone(), &game);
                scope.spawn(move || {
                    loop {
                        let i = next.fetch_add(1, Relaxed);
                        if i >= step {
                            break;
                        }
                        let d = start + done + i;
                        recorded
                            .fetch_add(play(rules, stack, d, seed, game, &counts) as u64, Relaxed);
                    }
                });
            }
        });
        done += step;
        std::fs::write(&out, counts.to_bytes()).expect("write --out");
        let secs = t.elapsed().as_secs_f64();
        let rate = done as f64 / secs;
        println!(
            "[{secs:>7.0}s] {done} hands, {} decisions recorded in all, {rate:.0} hands/s",
            counts.total()
        );
        if let Some(path) = &progress {
            let eta = (hands - done) as f64 / rate / 60.0;
            let json = format!(
                "{{\"label\": \"6-max: recording the Regulars' preflop\", \"done\": {done}, \"total\": {hands}, \"unit\": \"hands\", \"etaMin\": {eta:.0}}}\n"
            );
            let _ = std::fs::write(path, json);
        }
    }
}
