//! Which six-handed postflop spots come up (#266): plays hands with Ducybot
//! (the six-handed bot on a compact GTO strategy, Doug Poker after the flop)
//! and sorts every hand that reaches the flop by its preflop line, counting
//! the postflop decisions in each, so the most common spots can be solved
//! first (#267).
//!
//!     cargo run --release -p ducy-gto --example sixmax_spots -- \
//!         --strategy sixmax-100.bin --hands 2000000 [--mode field|self] \
//!         [--seed 4000] [--out spots-100.txt]
//!
//! `--mode field` (the default): Ducybot and five random Regulars, Ducybot in
//! a different seat each hand. `--mode self`: six Ducybots.
//!
//! Spots, by the players left at the flop:
//! - heads-up: "SRP CO v BB" (one raise: the raiser, then the caller),
//!   "3BP BTN v SB" (the opener, then the 3-bettor), "4BP+ …", "limped SB v BB"
//!   (no raise), "iso BB v SB" (a raise over limps: the raiser, then the
//!   limper who called);
//! - three or more: "3-way SRP", "4-way limped" and so on.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering::Relaxed},
    },
    thread,
};

use ducy_gto::sixmax::{
    bot::{PreflopStrategy, SixMaxBot, position},
    preflop::POSITIONS,
};
use ducy_play::{Bot, Deal, Event, Hand, TableRules, personality::Personality, play_hand};

const SEATS: usize = 6;

fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// Five different Regulars for hand `d`.
fn regulars(d: u64) -> Vec<Personality> {
    let mut pool: Vec<Personality> = Personality::ALL.to_vec();
    let mut x = d.wrapping_mul(0x2545_f491_4f6c_dd1d) ^ 0x5eed;
    (0..SEATS - 1)
        .map(|_| {
            x = mix(x);
            pool.swap_remove((x % pool.len() as u64) as usize)
        })
        .collect()
}

/// The spot a hand's preflop line makes, from the events before the flop,
/// or None if it ended preflop.
fn spot(events: &[Event], button: usize) -> Option<String> {
    let pos = |s: usize| POSITIONS[position(s, button)];
    let mut folded = [false; SEATS];
    let mut raisers: Vec<usize> = Vec::new();
    let mut limpers: Vec<usize> = Vec::new();
    let mut callers_of_last_raise: Vec<usize> = Vec::new();
    for e in events {
        match *e {
            Event::Fold { seat } => folded[seat] = true,
            Event::Call { seat, .. } => {
                if raisers.is_empty() {
                    limpers.push(seat);
                } else {
                    callers_of_last_raise.push(seat);
                }
            }
            Event::Bet { seat, .. } | Event::Raise { seat, .. } => {
                raisers.push(seat);
                callers_of_last_raise.clear();
            }
            Event::Board { .. } => break,
            _ => {}
        }
    }
    if !events.iter().any(|e| matches!(e, Event::Board { .. })) {
        return None;
    }
    let live: Vec<usize> = (0..SEATS).filter(|&s| !folded[s]).collect();
    let kind = match raisers.len() {
        0 => "limped",
        1 => "SRP",
        2 => "3BP",
        _ => "4BP+",
    };
    if live.len() > 2 {
        return Some(format!("{}-way {kind}", live.len()));
    }
    let (a, b) = (live[0], live[1]);
    let other = |x: usize| if x == a { b } else { a };
    Some(match raisers.len() {
        0 => {
            // The limper first; the big blind checked.
            let bb = live
                .iter()
                .copied()
                .find(|&s| position(s, button) == 5)
                .unwrap_or(b);
            format!("limped {} v {}", pos(other(bb)), pos(bb))
        }
        1 => {
            let r = raisers[0];
            if limpers.iter().any(|l| live.contains(l) && *l != r) {
                format!("iso {} v {}", pos(r), pos(other(r)))
            } else {
                format!("SRP {} v {}", pos(r), pos(other(r)))
            }
        }
        _ => {
            // The first raiser still in, then the other player.
            let first = raisers
                .iter()
                .copied()
                .find(|r| live.contains(r))
                .unwrap_or(a);
            format!("{kind} {} v {}", pos(first), pos(other(first)))
        }
    })
}

/// Postflop decisions in a hand: all of them, and the seat `who`'s.
fn decisions(events: &[Event], who: Option<usize>) -> (u64, u64) {
    let mut after = false;
    let (mut all, mut mine) = (0, 0);
    for e in events {
        let seat = match *e {
            Event::Board { .. } => {
                after = true;
                continue;
            }
            Event::Fold { seat } | Event::Check { seat } => seat,
            Event::Call { seat, .. } | Event::Bet { seat, .. } | Event::Raise { seat, .. } => seat,
            _ => continue,
        };
        if after {
            all += 1;
            if Some(seat) == who {
                mine += 1;
            }
        }
    }
    (all, mine)
}

#[derive(Default, Clone, Copy)]
struct Tally {
    hands: u64,
    decisions: u64,
    ducybot: u64,
}

fn main() {
    let mut strategy_path = String::new();
    let mut hands = 1_000_000u64;
    let mut self_play = false;
    let mut seed = 4000u64;
    let mut out: Option<String> = None;
    let threads = thread::available_parallelism().map_or(4, |n| n.get()) as u64;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut v = || args.next().unwrap_or_else(|| panic!("{a} needs a value"));
        match a.as_str() {
            "--strategy" => strategy_path = v(),
            "--hands" => hands = v().replace('_', "").parse().expect("--hands"),
            "--mode" => self_play = v() == "self",
            "--seed" => seed = v().parse().expect("--seed"),
            "--out" => out = Some(v()),
            _ => panic!("unknown argument {a}"),
        }
    }
    let strategy = Arc::new(
        PreflopStrategy::from_compact(&std::fs::read(&strategy_path).expect("read --strategy"))
            .expect("a compact strategy"),
    );
    let bb_deep = strategy.game().stack as u64 / 2;
    let rules = TableRules::no_limit_holdem(1, 2);
    let stack = bb_deep * 2;
    let tallies: Arc<Mutex<HashMap<String, Tally>>> = Arc::default();
    let next = Arc::new(AtomicU64::new(0));
    let flops = Arc::new(AtomicU64::new(0));
    thread::scope(|scope| {
        for _ in 0..threads {
            let (strategy, tallies, next, flops) = (
                strategy.clone(),
                tallies.clone(),
                next.clone(),
                flops.clone(),
            );
            scope.spawn(move || {
                let mut local: HashMap<String, Tally> = HashMap::new();
                loop {
                    let d = next.fetch_add(1, Relaxed);
                    if d >= hands {
                        break;
                    }
                    let deal = Deal::random(rules.variant, SEATS, Some(seed.wrapping_add(d)))
                        .expect("deal");
                    let button = d as usize % SEATS;
                    let ducy_seat = (mix(d ^ seed) % SEATS as u64) as usize;
                    let regs = regulars(d ^ seed);
                    let mut bots: Vec<Box<dyn Bot>> = (0..SEATS)
                        .map(|s| {
                            let bseed = mix(seed ^ (d << 8) ^ s as u64);
                            if self_play || s == ducy_seat {
                                Box::new(SixMaxBot::new(
                                    strategy.clone(),
                                    Personality::DougPoker.bot(Some(bseed)),
                                    bseed,
                                )) as Box<dyn Bot>
                            } else {
                                let r = regs[if s < ducy_seat { s } else { s - 1 }];
                                Box::new(r.bot(Some(bseed))) as Box<dyn Bot>
                            }
                        })
                        .collect();
                    let mut hand = Hand::new(rules, &[stack; SEATS], button, deal).expect("hand");
                    let mut seated: Vec<&mut dyn Bot> = Vec::with_capacity(SEATS);
                    for bot in bots.iter_mut() {
                        seated.push(&mut **bot);
                    }
                    play_hand(&mut hand, &mut seated).expect("play");
                    drop(seated);
                    let events = hand.events();
                    if let Some(key) = spot(events, button) {
                        flops.fetch_add(1, Relaxed);
                        let (all, mine) = decisions(events, (!self_play).then_some(ducy_seat));
                        let t = local.entry(key).or_default();
                        t.hands += 1;
                        t.decisions += all;
                        t.ducybot += if self_play { all } else { mine };
                    }
                }
                let mut all = tallies.lock().expect("lock");
                for (k, t) in local {
                    let e = all.entry(k).or_default();
                    e.hands += t.hands;
                    e.decisions += t.decisions;
                    e.ducybot += t.ducybot;
                }
            });
        }
    });
    let all = tallies.lock().expect("lock");
    let mut rows: Vec<(&String, &Tally)> = all.iter().collect();
    // By Ducybot's own postflop decisions: the ones he needs solved.
    rows.sort_by(|a, b| b.1.ducybot.cmp(&a.1.ducybot));
    let total_dec: u64 = rows.iter().map(|r| r.1.decisions).sum();
    let total_mine: u64 = rows.iter().map(|r| r.1.ducybot).sum();
    let mut text = format!(
        "{hands} hands, {bb_deep} BB, {}: {} reached the flop; {total_dec} postflop decisions, {total_mine} of them Ducybot's\n\n{:<22} {:>9} {:>8} {:>8} {:>8}\n",
        if self_play {
            "six Ducybots"
        } else {
            "Ducybot and five Regulars"
        },
        flops.load(Relaxed),
        "spot",
        "hands",
        "all %",
        "his %",
        "his cum",
    );
    let mut cum = 0.0;
    for (k, t) in rows {
        let his = 100.0 * t.ducybot as f64 / total_mine.max(1) as f64;
        cum += his;
        text.push_str(&format!(
            "{k:<22} {:>9} {:>7.2}% {:>7.2}% {:>7.1}%\n",
            t.hands,
            100.0 * t.decisions as f64 / total_dec.max(1) as f64,
            his,
            cum
        ));
    }
    print!("{text}");
    if let Some(path) = out {
        std::fs::write(path, text).expect("write --out");
    }
}
