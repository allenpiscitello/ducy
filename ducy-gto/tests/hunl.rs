use ducy::deck::{Card as DucyCard, Deck};
use ducy_gto::{
    Game, Rng, Turn,
    holdem::{
        cards::{Card, to_string},
        hunl::{BetMenu, Hunl, HunlAction, HunlConfig, Size},
    },
};
use ducy_play::{Action, Deal, Hand, Street, TableRules, Variant};

fn ducy_card(c: Card) -> DucyCard {
    DucyCard::parse(&to_string(c)).unwrap()
}

fn ducy_deck(cards: &[Card]) -> Deck {
    let mut d = Deck::empty();
    for &c in cards {
        d |= ducy_card(c);
    }
    d
}

fn street_index(s: Street) -> usize {
    match s {
        Street::Preflop => 0,
        Street::Flop => 1,
        Street::Turn => 2,
        Street::River => 3,
    }
}

/// Plays random abstract actions and replays each in a real ducy-play hand:
/// every action must be legal, and chips, turn and street must agree.
fn fuzz(config: HunlConfig, hands: usize, seed: u64) {
    let game = Hunl::new(config.clone(), None);
    let rules = TableRules::no_limit_holdem(config.small_blind, config.big_blind);
    let mut rng = Rng::new(seed);
    let mut actions_played = 0;
    let mut showdowns = 0;
    for _ in 0..hands {
        let mut s = game.sample_chance(&game.root(), &mut rng);
        let deal = Deal::new(
            Variant::Holdem,
            vec![ducy_deck(&s.hole[0]), ducy_deck(&s.hole[1])],
            s.board.map(ducy_card),
        )
        .unwrap();
        let mut hand = Hand::new(rules, &[config.stack, config.stack], 0, deal).unwrap();
        loop {
            match game.turn(&s) {
                Turn::Terminal => {
                    assert!(hand.is_complete(), "abstract hand over, real one isn't");
                    let net = hand.result().unwrap().net[0] as f64;
                    assert_eq!(game.utility(&s), net, "payoff");
                    if hand.result().unwrap().showdown {
                        showdowns += 1;
                    }
                    break;
                }
                Turn::Player(p) => {
                    assert_eq!(hand.to_act(), Some(p), "who acts");
                    assert_eq!(street_index(hand.street()), s.street, "street");
                    let actions = game.actions(&s);
                    let i = (rng.next_u64() % actions.len() as u64) as usize;
                    let a = actions[i];
                    let real = match a {
                        HunlAction::Fold => Action::Fold,
                        HunlAction::Check => Action::Check,
                        HunlAction::Call => Action::Call,
                        HunlAction::Bet(to) => Action::Bet(to),
                        HunlAction::Raise(to) => Action::Raise(to),
                    };
                    let legal = hand.legal_actions().unwrap();
                    assert!(
                        hand.act(real).is_ok(),
                        "{a:?} illegal; legal: {legal:?}; state {s:?}"
                    );
                    s = game.apply(&s, i);
                    actions_played += 1;
                    for seat in 0..2 {
                        assert_eq!(
                            hand.contributed(seat),
                            s.contributed[seat],
                            "contributed after {a:?}"
                        );
                        if !hand.is_complete() {
                            assert_eq!(hand.stack(seat), s.stack[seat], "stack after {a:?}");
                        }
                    }
                }
                Turn::Chance => unreachable!("only the root deals"),
            }
        }
    }
    assert!(
        actions_played > hands * 2 && showdowns > hands / 20,
        "{actions_played} actions, {showdowns} showdowns"
    );
}

#[test]
fn abstract_actions_are_legal_in_ducy_play() {
    fuzz(HunlConfig::default(), 3000, 1);
}

#[test]
fn odd_stacks_and_sizes_are_legal_too() {
    use Size::*;
    // Short and deep stacks, odd blinds, sizes that clamp to the minimum
    // raise or to all-in.
    for (sb, bb, stack) in [(1, 2, 37), (5, 10, 2000), (2, 5, 101), (1, 2, 3)] {
        let config = HunlConfig {
            small_blind: sb,
            big_blind: bb,
            stack,
            menu: BetMenu {
                preflop: vec![
                    vec![Bb(2.0), Bb(7.5), AllIn],
                    vec![Times(2.0), Pot(0.5)],
                    vec![Pot(0.1), AllIn],
                ],
                postflop: vec![vec![Pot(0.01), Pot(0.5), Pot(3.0)], vec![Pot(0.2), AllIn]],
            },
        };
        fuzz(config, 1500, stack);
    }
}

#[test]
fn heads_up_order_and_blinds() {
    let game = Hunl::new(HunlConfig::default(), None);
    let s = game.deal([[0, 1], [2, 3]], [4, 5, 6, 7, 8]);
    // The button (player 0) posts the small blind and acts first preflop.
    assert_eq!(s.contributed, [1, 2]);
    assert_eq!(s.to_act, 0);
    let a = game.actions(&s);
    assert_eq!(
        a,
        vec![
            HunlAction::Fold,
            HunlAction::Call,
            HunlAction::Raise(5),
            HunlAction::Raise(200)
        ]
    );
    // Limp, check: the flop, where the big blind acts first and bets are
    // a third, three quarters and 1.25 of the 4-chip pot.
    let s = game.apply(&game.apply(&s, 1), 0);
    assert_eq!((s.street, s.to_act), (1, 1));
    assert_eq!(
        game.actions(&s),
        vec![
            HunlAction::Check,
            HunlAction::Bet(2),
            HunlAction::Bet(3),
            HunlAction::Bet(5),
            HunlAction::Bet(198)
        ]
    );
}

#[test]
fn tree_statistics() {
    let game = Hunl::new(HunlConfig::default(), None);
    let stats = game.tree_stats();
    println!("{stats:?}");
    assert!(stats.sequences.iter().all(|&n| n > 0));
    // Every street's sequences are reachable from the one before it.
    assert!(stats.sequences[1] > stats.sequences[0] / 10);
}
