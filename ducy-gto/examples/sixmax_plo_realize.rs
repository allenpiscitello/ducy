//! Fits equity realization for six-handed PLO (#290), as `sixmax_realize`
//! does for hold'em: the heads-up PLO blueprint plays itself, and every hand
//! that sees the flop without an all-in adds what each player won from the
//! flop pot on, against what its preflop equity says, per preflop bucket, in
//! position (the button) and out of position.
//!
//!     cargo run --release -p ducy-gto --example sixmax_plo_realize -- \
//!         --cards plo-cards.bin --blueprint plo-100bb.bin --bb 100 \
//!         --equity plo-equity.bin --hands 2000000 --out plo-realize-100.bin
//!
//! `--equity` is `sixmax_plo`'s table for the same abstraction; `--bb` is the
//! depth the blueprint was trained at. The fit feeds `sixmax_plo --realize`.

use std::{sync::Arc, thread, time::Instant};

use ducy_gto::{
    holdem::{
        cards::from_ducy,
        hunl::{Buckets, HunlConfig},
    },
    omaha::{
        abstraction::PloAbstraction,
        bot::{PloGtoBot, load_blueprint},
    },
    sixmax::{
        equity::PreflopEquity,
        realize::{Realization, Tally},
    },
};
use ducy_play::{Bot, Deal, Event, Hand, Street, TableRules, Variant, play_hand};

/// One hand's contribution, if it saw the flop without an all-in: (bucket,
/// equity, pot, won) in position, then out of position.
type Sample = [(usize, f64, f64, f64); 2];

fn play(
    bots: &mut [PloGtoBot; 2],
    cards: &PloAbstraction,
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
    let mut bet = [0u64; 2];
    let mut flop = false;
    for e in hand.events() {
        match e {
            Event::SmallBlind { seat, amount } | Event::BigBlind { seat, amount } => {
                bet[*seat] += amount
            }
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
    let bucket = |seat: usize| {
        let hole: Vec<_> = hand
            .hole_cards(seat)
            .expect("cards")
            .iter(false)
            .map(from_ducy)
            .collect();
        cards.preflop_bucket(&hole) as usize
    };
    let (ip, oop) = (button, 1 - button);
    let (bi, bo) = (bucket(ip), bucket(oop));
    let won = |seat: usize| (net[seat] + bet[seat] as i64) as f64;
    Some([
        (bi, eq.heads_up(bi, bo), pot, won(ip)),
        (bo, eq.heads_up(bo, bi), pot, won(oop)),
    ])
}

fn main() {
    let mut cards_path = String::new();
    let mut blueprint_path = String::new();
    let mut equity_path = String::new();
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
        PloAbstraction::load(&std::fs::read(&cards_path).expect("read --cards"))
            .expect("a PLO abstraction"),
    );
    let n = cards.bucket_count(0);
    let eq = PreflopEquity::from_bytes(&std::fs::read(&equity_path).expect("read --equity"))
        .expect("equity table");
    assert_eq!(eq.classes(), n, "--equity was built for these buckets");
    let config = HunlConfig::pot_limit_omaha_lean(bb);
    let (blueprint, tree) = load_blueprint(
        &config,
        &cards,
        &std::fs::read(&blueprint_path).expect("read --blueprint"),
    )
    .expect("a PLO blueprint for these cards");
    let (blueprint, tree) = (Arc::new(blueprint), Arc::new(tree));
    let rules = TableRules::pot_limit_omaha(config.small_blind, config.big_blind).with_variant(
        Variant::Omaha {
            hole_cards: cards.config.hole_cards as u32,
        },
    );
    let stack = config.stack;
    let t = Instant::now();
    let tallies: Vec<(Tally, Tally, u64)> = thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|th| {
                let (cards, blueprint, tree, eq, config) =
                    (&cards, &blueprint, &tree, &eq, config.clone());
                scope.spawn(move || {
                    let bot = |s| {
                        PloGtoBot::from_parts(
                            config.clone(),
                            cards.clone(),
                            blueprint.clone(),
                            tree.clone(),
                            s,
                        )
                    };
                    let mut bots = [bot(th * 2 + 1), bot(th * 2 + 2)];
                    let (mut ip, mut oop, mut seen) = (Tally::new(n), Tally::new(n), 0);
                    for i in (th..hands).step_by(threads as usize) {
                        if let Some([a, b]) =
                            play(&mut bots, cards, rules, stack, eq, i * 7919 + 13)
                        {
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
    let (mut ip, mut oop, mut seen) = (Tally::new(n), Tally::new(n), 0);
    for (a, b, s) in &tallies {
        ip.merge(a);
        oop.merge(b);
        seen += s;
    }
    let r = Realization::fit(&ip, &oop, prior);
    std::fs::write(&out, r.to_bytes()).expect("write --out");
    println!(
        "PLO{} {bb} BB: {hands} hands, {seen} saw the flop without an all-in ({:.0}s); written to {out}",
        cards.config.hole_cards,
        t.elapsed().as_secs_f64()
    );
    println!(
        "overall: in position {:.3}, out of position {:.3}",
        ip.overall(),
        oop.overall()
    );
    // The spread over buckets, weighted by how often each saw a flop.
    for (name, t, v) in [
        ("in position", &ip, &r.ip),
        ("out of position", &oop, &r.oop),
    ] {
        let mut by: Vec<(f32, u64)> = (0..n).map(|b| (v[b], t.hands(b))).collect();
        by.sort_by(|a, b| a.0.total_cmp(&b.0));
        let total: u64 = by.iter().map(|x| x.1).sum();
        let at = |q: f64| {
            let mut acc = 0;
            for &(x, h) in &by {
                acc += h;
                if acc as f64 >= q * total as f64 {
                    return x;
                }
            }
            by.last().map_or(1.0, |x| x.0)
        };
        println!(
            "{name}: 10th percentile {:.2}, median {:.2}, 90th {:.2}",
            at(0.1),
            at(0.5),
            at(0.9)
        );
    }
}
