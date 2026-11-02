//! Where a personality loses to GtoBot heads-up: its result split by how
//! each hand went (big calls that lost at showdown, bets and raises given up,
//! preflop all-ins…), on duplicate deals, so the spots worth fixing show.
//!
//!     cargo run --release -p ducy-gto --example leaks -- \
//!         --cards abstraction.bin --blueprint blueprint.bin --who nik_airbag --deals 5000
//!
//! `--stack N` starts both players N big blinds deep (default 100).

use std::{collections::BTreeMap, sync::Arc};

use ducy::deck::{Card, Deck};
use ducy_bots::{personality::Personality, ranges::made_score, traits::no_pair_of_its_own};
use ducy_gto::holdem::{
    abstraction::CardAbstraction,
    blueprint::Blueprint,
    bot::GtoBot,
    hunl::{BettingTree, Hunl, HunlConfig},
};
use ducy_play::{Bot, Deal, Event, Hand, Street, TableRules, Variant, play_hand};

/// The lowest two pair: below it, a pair or less.
fn two_pair_floor() -> u32 {
    let d = Deck::parse("3s 3d 2c 2h 4s").expect("cards");
    ducy::ranking::hand_rank::StandardHandRanker::score(&d)
}

/// What kind of hand it was, for `hero` (the personality's seat).
fn bucket(hand: &Hand, hero: usize, bb: u64, floor: u32) -> String {
    let villain = 1 - hero;
    let mut street = Street::Preflop;
    let mut bet = [0u64; 2]; // this street's bets
    let mut pot = 0u64; // before this street
    let mut big_call: Option<Street> = None; // a call of half the pot or more, after the flop
    let mut aggressed = false; // hero bet or raised after the flop
    let mut preflop_allin = false;
    let mut hero_folded = false;
    let mut villain_folded = false;
    for e in hand.events() {
        match e {
            Event::SmallBlind { seat, amount } | Event::BigBlind { seat, amount } => {
                bet[*seat] += amount
            }
            Event::Board { street: s, .. } => {
                pot += bet[0] + bet[1];
                bet = [0, 0];
                street = *s;
            }
            Event::Call {
                seat,
                amount,
                all_in,
            } => {
                if *seat == hero && street != Street::Preflop {
                    let before = pot + bet[0] + bet[1];
                    if *amount * 2 >= before && big_call.is_none_or(|s| s < street) {
                        big_call = Some(street);
                    }
                }
                if street == Street::Preflop && (*all_in || bet[villain].max(bet[hero]) >= 40 * bb)
                {
                    preflop_allin = true;
                }
                bet[*seat] += amount;
            }
            Event::Bet { seat, to, all_in } | Event::Raise { seat, to, all_in } => {
                if *seat == hero && street != Street::Preflop {
                    aggressed = true;
                }
                if street == Street::Preflop && *all_in {
                    preflop_allin = true;
                }
                bet[*seat] = *to;
            }
            Event::Fold { seat } => {
                if *seat == hero {
                    hero_folded = true;
                } else {
                    villain_folded = true;
                }
            }
            _ => {}
        }
    }
    let total = pot + bet[0] + bet[1];
    let size = if total >= 250 * bb {
        "pot 250bb+"
    } else if total >= 100 * bb {
        "pot 100bb+"
    } else if total >= 30 * bb {
        "pot 30-100bb"
    } else {
        "pot <30bb"
    };
    if preflop_allin {
        return "preflop all-in or 40bb+ preflop".into();
    }
    if villain_folded {
        return format!("won without showdown ({size})");
    }
    if hero_folded {
        return match (street, aggressed) {
            (Street::Preflop, _) => "folded preflop".into(),
            (_, true) => format!("bet or raised, then folded ({size})"),
            (_, false) => format!("folded after the flop, never bet ({size})"),
        };
    }
    // Showdown: how strong was hero's hand, and did hero call big to get there?
    let hole: Vec<Card> = hand
        .hole_cards(hero)
        .map_or_else(Vec::new, |d| d.iter(false).collect());
    let board = hand.board();
    let strength = if no_pair_of_its_own(Variant::Holdem, &hole, board) {
        "nothing"
    } else if made_score(Variant::Holdem, &hole, board) < floor {
        "a pair"
    } else {
        "two pair+"
    };
    match big_call {
        Some(s) => format!("showdown with {strength}, after calling big on the {s:?}"),
        None => format!("showdown with {strength}, no big call ({size})"),
    }
}

fn main() {
    let mut cards_path = String::new();
    let mut blueprint_path = String::new();
    let mut deals = 5_000usize;
    let mut who = Personality::NikAirbag;
    let mut stack_bb: Option<u64> = None;
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut v = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--cards" => cards_path = v(),
            "--blueprint" => blueprint_path = v(),
            "--deals" => deals = v().parse().expect("--deals"),
            "--who" => who = Personality::from_name(&v()).expect("unknown personality"),
            "--stack" => stack_bb = Some(v().parse().expect("--stack")),
            f => panic!("unknown option {f}"),
        }
    }
    let cards = Arc::new(
        CardAbstraction::load(&std::fs::read(&cards_path).expect("read --cards"))
            .expect("abstraction"),
    );
    let config = HunlConfig::default();
    let game = Hunl::new(config.clone(), Some(&cards));
    let blueprint = Arc::new(
        Blueprint::load(
            &std::fs::read(&blueprint_path).expect("read --blueprint"),
            &game,
            &cards,
        )
        .expect("blueprint"),
    );
    let tree = Arc::new(BettingTree::build(&config));
    let rules = TableRules::no_limit_holdem(config.small_blind, config.big_blind);
    let bb = config.big_blind;
    let floor = two_pair_floor();
    let stack = stack_bb.map_or(config.stack, |n| n * bb);

    // bucket → (hands, chips)
    let mut by: BTreeMap<String, (usize, i64)> = BTreeMap::new();
    let mut hands = 0usize;
    let mut net = 0i64;
    for d in 0..deals {
        let deal = Deal::random(rules.variant, 2, Some(d as u64 * 7919 + 1)).expect("deal");
        // Duplicate: the personality in each seat on the same cards.
        for hero in 0..2 {
            let mut gto = GtoBot::from_parts(
                cards.clone(),
                blueprint.clone(),
                tree.clone(),
                d as u64 * 2 + hero as u64,
            );
            let mut me = who.bot(Some(d as u64 * 2 + hero as u64 + 99));
            let mut hand = Hand::new(rules, &[stack, stack], d % 2, deal.clone()).expect("hand");
            let mut seated: Vec<&mut dyn Bot> = if hero == 0 {
                vec![&mut me, &mut gto]
            } else {
                vec![&mut gto, &mut me]
            };
            play_hand(&mut hand, &mut seated).expect("play");
            let won = hand.result().expect("result").net[hero];
            let e = by.entry(bucket(&hand, hero, bb, floor)).or_default();
            e.0 += 1;
            e.1 += won;
            hands += 1;
            net += won;
        }
    }
    let per100 = |chips: i64| chips as f64 / bb as f64 / hands as f64 * 100.0;
    println!(
        "{} against GtoBot: {hands} hands, {:+.1} bb/100\n",
        who.name(),
        per100(net)
    );
    println!(
        "{:>8} {:>7} {:>9}  how the hand went",
        "hands", "share", "bb/100"
    );
    let mut rows: Vec<_> = by.into_iter().collect();
    rows.sort_by_key(|(_, (_, c))| *c);
    for (k, (n, c)) in rows {
        println!(
            "{n:>8} {:>6.1}% {:>+9.1}  {k}",
            n as f64 / hands as f64 * 100.0,
            per100(c)
        );
    }
}
