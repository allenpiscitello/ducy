//! Network bots (#215): they play legal full matches in every game, from an
//! untrained network, and the hard rule holds: the nuts on the river bet.

use std::sync::Arc;

use ducy::deck::{Card, Deck};
use ducy_play::{
    Action, Bot, Deal, Hand, MatchConfig, TableRules, Variant,
    net::{DEFAULT_SIZES, Mlp, NetBot},
    personality::Personality,
    run_match,
};

fn plays_legally(rules: TableRules, deals: usize) {
    let net = Arc::new(Mlp::random(&DEFAULT_SIZES, 1));
    let mut bots: Vec<Box<dyn Bot>> = Personality::ALL[..6]
        .iter()
        .enumerate()
        .map(|(i, p)| Box::new(NetBot::new(*p, net.clone(), Some(i as u64))) as Box<dyn Bot>)
        .collect();
    let r = run_match(&MatchConfig::new(rules, deals, 5), &mut bots).expect("match");
    assert_eq!(r.fallbacks, vec![0; 6], "every action legal");
    assert_eq!(r.net.iter().sum::<i64>(), 0, "chips conserved");
}

#[test]
fn network_bots_play_legal_holdem() {
    plays_legally(TableRules::no_limit_holdem(1, 2), 30);
}

// Omaha's exact strength features are slow in a debug build: fewer deals.
#[test]
fn network_bots_play_legal_omaha() {
    plays_legally(TableRules::pot_limit_omaha(1, 2), 6);
    plays_legally(
        TableRules::pot_limit_omaha(1, 2).with_variant(Variant::Omaha { hole_cards: 6 }),
        6,
    );
}

fn deck(s: &str) -> Deck {
    let mut d = Deck::empty();
    for c in s.split(' ') {
        d |= c.parse::<Card>().unwrap();
    }
    d
}

#[test]
fn the_nuts_on_the_river_always_bet() {
    let rules = TableRules::no_limit_holdem(1, 2);
    let board: Vec<Card> = "Qh Jh Th 2c 3d"
        .split(' ')
        .map(|c| c.parse().unwrap())
        .collect();
    let deal = Deal::new(
        rules.variant,
        vec![deck("Ah Kh"), deck("2s 3s")],
        board.try_into().unwrap(),
    )
    .unwrap();
    // Seat 1 has the button: it calls, seat 0 checks, then both check to the river.
    let mut hand = Hand::new(rules, &[200, 200], 1, deal).unwrap();
    hand.act(Action::Call).unwrap();
    hand.act(Action::Check).unwrap();
    for _ in 0..2 {
        hand.act(Action::Check).unwrap();
        hand.act(Action::Check).unwrap();
    }
    let obs = hand.observation(0).expect("seat 0 first on the river");
    // An untrained network might check; the rule bets the pot (4).
    for seed in 0..5 {
        let mut bot = NetBot::new(
            Personality::DougPoker,
            Arc::new(Mlp::random(&DEFAULT_SIZES, seed)),
            Some(seed),
        );
        assert_eq!(bot.act(&obs), Some(Action::Bet(4)));
    }
}

#[test]
fn the_policy_covers_only_possible_actions() {
    let rules = TableRules::no_limit_holdem(1, 2);
    let deal = Deal::random(rules.variant, 3, Some(9)).unwrap();
    let hand = Hand::new(rules, &[200, 200, 200], 0, deal).unwrap();
    let obs = hand.observation(hand.to_act().unwrap()).unwrap();
    let mut bot = NetBot::new(
        Personality::NikAirbag,
        Arc::new(Mlp::random(&DEFAULT_SIZES, 3)),
        Some(3),
    );
    let p = bot.policy(&obs);
    assert!((p.iter().sum::<f32>() - 1.0).abs() < 1e-5);
    // Facing the big blind preflop, every class is possible here.
    assert!(p.iter().all(|&q| q > 0.0));
}
