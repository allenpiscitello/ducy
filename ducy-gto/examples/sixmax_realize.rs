//! Fits equity realization for the six-handed preflop game (#249) from the
//! heads-up blueprint playing itself: every hand that sees the flop without
//! an all-in adds what each player won from the flop pot on, against what its
//! preflop equity says, per starting-hand class, in position (the button,
//! last to act after the flop) and out of position.
//!
//!     cargo run --release -p ducy-gto --example sixmax_realize -- \
//!         --cards cards-full.bin --blueprint blueprint.bin --bb 100 \
//!         --equity preflop-equity.bin --hands 2000000 --out realize-100.bin
//!
//! Use the full card abstraction (the compact one works its buckets out on
//! the fly, far too slowly for this). `--bb` is the depth the blueprint was
//! trained at; play is at that depth.

use std::{sync::Arc, thread, time::Instant};

use ducy_gto::{
    holdem::{
        abstraction::CardAbstraction,
        bot::GtoBot,
        cards::from_ducy,
        chart::class_name,
        depths::depth_config,
        iso::{NUM_PREFLOP_CLASSES, preflop_class},
    },
    sixmax::{
        equity::PreflopEquity,
        realize::{Realization, Tally},
    },
};
use ducy_play::{Bot, Deal, Event, Hand, Street, TableRules, play_hand};

/// One hand's contribution to the tallies, if it saw the flop without an
/// all-in: (class, equity, pot, won) in position, then out of position.
type Sample = [(usize, f64, f64, f64); 2];

fn play(
    bots: &mut [GtoBot; 2],
    rules: TableRules,
    stack: u64,
    eq: &PreflopEquity,
    seed: u64,
) -> Option<Sample> {
    let deal = Deal::random(rules.variant, 2, Some(seed)).expect("deal");
    let button = (seed % 2) as usize;
    let mut hand = Hand::new(rules, &[stack, stack], button, deal).expect("hand");
    {
        let [a, b] = bots;
        let mut seated: Vec<&mut dyn Bot> = vec![a, b];
        play_hand(&mut hand, &mut seated).expect("play");
    }
    // Chips each seat put in before the flop, and whether the flop came with
    // betting still to do.
    let mut bet = [0u64; 2];
    let mut flop = false;
    for e in hand.events() {
        match e {
            Event::SmallBlind { seat, amount } => {
                // Heads-up the button posts the small blind, and acts last
                // after the flop: it's the one in position.
                assert_eq!(*seat, button, "the button posts the small blind");
                bet[*seat] += amount
            }
            Event::BigBlind { seat, amount } => bet[*seat] += amount,
            Event::Call {
                seat,
                amount,
                all_in,
            } => {
                if *all_in {
                    return None;
                }
                bet[*seat] += amount;
            }
            Event::Bet { seat, to, all_in } | Event::Raise { seat, to, all_in } => {
                if *all_in {
                    return None;
                }
                bet[*seat] = *to;
            }
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
    if !flop {
        return None;
    }
    let pot = (bet[0] + bet[1]) as f64;
    let net = &hand.result().expect("result").net;
    let class = |seat: usize| {
        let cards: Vec<_> = hand
            .hole_cards(seat)
            .expect("cards")
            .iter(false)
            .map(from_ducy)
            .collect();
        preflop_class(cards[0], cards[1])
    };
    let (ip, oop) = (button, 1 - button);
    let (ci, co) = (class(ip), class(oop));
    let won = |seat: usize| (net[seat] + bet[seat] as i64) as f64;
    Some([
        (ci, eq.heads_up(ci, co), pot, won(ip)),
        (co, eq.heads_up(co, ci), pot, won(oop)),
    ])
}

fn main() {
    let mut cards_path = String::new();
    let mut blueprint_path = String::new();
    let mut equity_path = String::from("preflop-equity.bin");
    let mut bb = 100u64;
    let mut hands = 1_000_000u64;
    let mut out = String::new();
    let mut prior = 20.0f64;
    let mut threads = thread::available_parallelism().map_or(4, |n| n.get()) as u64;
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut v = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--cards" => cards_path = v(),
            "--blueprint" => blueprint_path = v(),
            "--equity" => equity_path = v(),
            "--bb" => bb = v().parse().expect("--bb"),
            "--hands" => hands = v().replace('_', "").parse().expect("--hands"),
            "--out" => out = v(),
            "--prior" => prior = v().parse().expect("--prior"),
            "--threads" => threads = v().parse().expect("--threads"),
            f => panic!("unknown option {f}"),
        }
    }
    assert!(!out.is_empty(), "--out FILE is required");
    let cards = Arc::new(
        CardAbstraction::load(&std::fs::read(&cards_path).expect("read --cards"))
            .expect("abstraction"),
    );
    let bytes = std::fs::read(&blueprint_path).expect("read --blueprint");
    let eq = PreflopEquity::from_bytes(&std::fs::read(&equity_path).expect("read --equity"))
        .expect("equity table");
    let config = depth_config(bb);
    let rules = TableRules::no_limit_holdem(config.small_blind, config.big_blind);
    let stack = config.stack;
    let t = Instant::now();
    let tallies: Vec<(Tally, Tally, u64)> = thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|th| {
                let (cards, bytes, eq, config) = (cards.clone(), &bytes, &eq, config.clone());
                scope.spawn(move || {
                    let bot = |s| {
                        GtoBot::new(config.clone(), cards.clone(), bytes, s).expect("blueprint")
                    };
                    let mut bots = [bot(th * 2 + 1), bot(th * 2 + 2)];
                    let (mut ip, mut oop, mut seen) = (Tally::default(), Tally::default(), 0);
                    for i in (th..hands).step_by(threads as usize) {
                        if let Some([a, b]) = play(&mut bots, rules, stack, eq, i * 7919 + 13) {
                            ip.add(a.0, a.1, a.2, a.3);
                            oop.add(b.0, b.1, b.2, b.3);
                            seen += 1;
                        }
                    }
                    (ip, oop, seen)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("thread"))
            .collect()
    });
    let (mut ip, mut oop, mut seen) = (Tally::default(), Tally::default(), 0);
    for (a, b, s) in &tallies {
        ip.merge(a);
        oop.merge(b);
        seen += s;
    }
    let r = Realization::fit(&ip, &oop, prior);
    std::fs::write(&out, r.to_bytes()).expect("write --out");
    println!(
        "{bb} BB: {hands} hands, {seen} saw the flop without an all-in ({:.0}s); written to {out}",
        t.elapsed().as_secs_f64()
    );
    println!(
        "overall: in position {:.3}, out of position {:.3}",
        ip.overall(),
        oop.overall()
    );
    println!(
        "combo-weighted: in position {:.3}, out of position {:.3}",
        Realization::mean(&r.ip),
        Realization::mean(&r.oop)
    );
    let class = |name: &str| {
        (0..NUM_PREFLOP_CLASSES)
            .find(|&c| class_name(c) == name)
            .expect("class")
    };
    for name in ["AA", "AKs", "KQo", "T9s", "77", "A5s", "J7o", "94o", "72o"] {
        let c = class(name);
        println!(
            "{name:>4}: in position {:.2} ({} hands), out of position {:.2} ({} hands)",
            r.ip[c],
            ip.hands(c),
            r.oop[c],
            oop.hands(c)
        );
    }
}
