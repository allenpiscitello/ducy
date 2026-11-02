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
//!
//! `--per-bot FILE.json` also counts each Regular's own decisions at the
//! chart spots (`sixmax::chart::spots`), for the site's scouting charts:
//! {"format": "6max", "bb", "source": "regulars", "actions": [...],
//! "bots": [{"id", "name", "spots": [{"id", "kind", "hero", "villain",
//! "hands", "overall": [4 percentages], "cells": [169 × ([4 percentages] or
//! null when fewer than MIN_CELL hands)]}]}]}, cells in chart order. It
//! starts fresh each run (it isn't resumed).

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering::Relaxed},
    },
    thread,
    time::Instant,
};

use std::{collections::HashMap, sync::atomic::AtomicU32};

use ducy_bots::personality::Personality;
use ducy_gto::{
    holdem::{
        cards::Card,
        cards::from_ducy,
        chart::combos,
        hunl::HunlConfig,
        iso::{NUM_PREFLOP_CLASSES, preflop_class},
    },
    sixmax::{
        bot::follow,
        chart::{Spot, act_name, grid_classes, spots},
        field::FieldCounts,
        preflop::{Node, POSITIONS, PreflopGame},
    },
};
use ducy_play::{Bot, Deal, Hand, TableRules, play_hand};

const SEATS: usize = 6;
const ACTIONS: [&str; 4] = ["fold", "call", "raise", "allin"];
/// Hands of a class a Regular needs at a spot before its cell shows.
const MIN_CELL: u32 = 20;

/// Each Regular's action counts at the chart spots, by hand class.
struct BotSpots {
    spots: Vec<Spot>,
    /// Node → (spot index, each of its actions as an index into ACTIONS).
    at: HashMap<usize, (usize, Vec<usize>)>,
    /// [bot][spot][class][action]
    counts: Vec<AtomicU32>,
}

impl BotSpots {
    fn new(game: &PreflopGame) -> Self {
        let spots = spots(game);
        let at = spots
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let Node::Decision { actions, .. } = &game.nodes[s.node] else {
                    panic!("a spot is a decision");
                };
                let map = actions
                    .iter()
                    .map(|(a, _)| {
                        ACTIONS
                            .iter()
                            .position(|n| *n == act_name(a))
                            .expect("action")
                    })
                    .collect();
                (s.node, (i, map))
            })
            .collect();
        let n = Personality::ALL.len() * spots.len() * NUM_PREFLOP_CLASSES * ACTIONS.len();
        Self {
            counts: (0..n).map(|_| AtomicU32::new(0)).collect(),
            spots,
            at,
        }
    }

    fn slot(&self, bot: usize, spot: usize, class: usize, action: usize) -> &AtomicU32 {
        let i = ((bot * self.spots.len() + spot) * NUM_PREFLOP_CLASSES + class) * ACTIONS.len()
            + action;
        &self.counts[i]
    }

    fn json(&self, bb: u32) -> String {
        let order = grid_classes();
        let mut bots = Vec::new();
        for (b, p) in Personality::ALL.iter().enumerate() {
            let mut out = Vec::new();
            for (s, spot) in self.spots.iter().enumerate() {
                let n = |c: usize, a: usize| self.slot(b, s, c, a).load(Relaxed) as f64;
                let hands: f64 = (0..NUM_PREFLOP_CLASSES)
                    .map(|c| (0..4).map(|a| n(c, a)).sum::<f64>())
                    .sum();
                if hands == 0.0 {
                    continue;
                }
                // Over all hands: each class's mix, weighted by its combos
                // (so the dealing's luck doesn't tilt it).
                let mut overall = [0.0; 4];
                let mut weight = 0.0;
                for c in 0..NUM_PREFLOP_CLASSES {
                    let t: f64 = (0..4).map(|a| n(c, a)).sum();
                    if t > 0.0 {
                        weight += combos(c);
                        for (a, o) in overall.iter_mut().enumerate() {
                            *o += combos(c) * n(c, a) / t;
                        }
                    }
                }
                let cells: Vec<String> = order
                    .iter()
                    .map(|&c| {
                        let t: f64 = (0..4).map(|a| n(c, a)).sum();
                        if t < MIN_CELL as f64 {
                            "null".to_string()
                        } else {
                            format!(
                                "[{}]",
                                (0..4)
                                    .map(|a| format!("{:.0}", 100.0 * n(c, a) / t))
                                    .collect::<Vec<_>>()
                                    .join(",")
                            )
                        }
                    })
                    .collect();
                let villain = spot
                    .villain
                    .map_or("null".to_string(), |v| format!("\"{}\"", POSITIONS[v]));
                out.push(format!(
                    "{{\"id\": \"{}\", \"kind\": \"{}\", \"hero\": \"{}\", \"villain\": {villain}, \"hands\": {hands}, \"overall\": [{}], \"cells\": [{}]}}",
                    spot.id,
                    spot.kind,
                    POSITIONS[spot.hero],
                    overall.iter().map(|o| format!("{:.1}", 100.0 * o / weight.max(1.0))).collect::<Vec<_>>().join(", "),
                    cells.join(",")
                ));
            }
            bots.push(format!(
                "\n {{\"id\": \"{}\", \"name\": \"{}\", \"spots\": [\n  {}]}}",
                p.id(),
                p.name(),
                out.join(",\n  ")
            ));
        }
        format!(
            "{{\"format\": \"6max\", \"bb\": {bb}, \"source\": \"regulars\", \"actions\": [\"fold\", \"call\", \"raise\", \"allin\"], \"bots\": [{}\n]}}\n",
            bots.join(",")
        )
    }
}

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
    per_bot: Option<&BotSpots>,
) -> usize {
    let deal = Deal::random(rules.variant, SEATS, Some(seed.wrapping_add(d))).expect("deal");
    let button = d as usize % SEATS;
    let people = lineup(d ^ seed);
    let mut bots: Vec<Box<dyn Bot>> = people
        .iter()
        .copied()
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
    if let Some(per) = per_bot {
        follow(game, hand.events(), button, |node, seat, action| {
            if let Some((spot, map)) = per.at.get(&node) {
                let bot = Personality::ALL
                    .iter()
                    .position(|p| *p == people[seat])
                    .expect("a Regular");
                let class = preflop_class(holes[seat][0], holes[seat][1]);
                per.slot(bot, *spot, class, map[action])
                    .fetch_add(1, Relaxed);
            }
        });
    }
    counts.record(game, hand.events(), button, &holes, |_| true)
}

fn main() {
    let mut hands = 1_000_000u64;
    let mut stack_bb = 100u32;
    let mut seed = 1000u64;
    let mut every = 1_000_000u64;
    let mut out = String::from("field.bin");
    let mut progress: Option<String> = None;
    let mut per_bot_path: Option<String> = None;
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
            "--per-bot" => per_bot_path = Some(v()),
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
    let per_bot = per_bot_path.as_ref().map(|_| BotSpots::new(&game));
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
                let per_bot = per_bot.as_ref();
                let (next, recorded, counts, game) =
                    (next.clone(), recorded.clone(), counts.clone(), &game);
                scope.spawn(move || {
                    loop {
                        let i = next.fetch_add(1, Relaxed);
                        if i >= step {
                            break;
                        }
                        let d = start + done + i;
                        recorded.fetch_add(
                            play(rules, stack, d, seed, game, &counts, per_bot) as u64,
                            Relaxed,
                        );
                    }
                });
            }
        });
        done += step;
        std::fs::write(&out, counts.to_bytes()).expect("write --out");
        if let (Some(path), Some(per)) = (&per_bot_path, &per_bot) {
            std::fs::write(path, per.json(stack_bb)).expect("write --per-bot");
        }
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
