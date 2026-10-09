use ducy::deck::{Card as DucyCard, Deck};
use ducy_gto::{
    Config, Game, Mccfr, Rng, Turn,
    holdem::{
        abstraction::CardAbstraction,
        cards::{Card, to_string},
        hunl::{BetMenu, Buckets, HuPlo, Hunl, HunlAction, HunlConfig, Size},
    },
};
use ducy_play::{Action, BettingStructure, Deal, Hand, Street, TableRules, Variant};

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
/// every action must be legal, and chips, turn, street and the payoff
/// (showdowns included) must agree. Hold'em for two hole cards, Omaha for
/// more; no-limit or pot-limit as `config` says.
fn fuzz(config: HunlConfig, hands: usize, seed: u64) {
    fuzz_holes::<2>(config, hands, seed);
}

fn fuzz_holes<const H: usize>(config: HunlConfig, hands: usize, seed: u64) {
    let game = Hunl::<CardAbstraction, H>::with_cards(config.clone(), None);
    let variant = if H == 2 {
        Variant::Holdem
    } else {
        Variant::Omaha {
            hole_cards: H as u32,
        }
    };
    let structure = if config.pot_limit {
        BettingStructure::PotLimit
    } else {
        BettingStructure::NoLimit
    };
    let rules = TableRules::no_limit_holdem(config.small_blind, config.big_blind)
        .with_variant(variant)
        .with_structure(structure);
    let mut rng = Rng::new(seed);
    let mut actions_played = 0;
    let mut showdowns = 0;
    for _ in 0..hands {
        let mut s = game.sample_chance(&game.root(), &mut rng);
        let deal = Deal::new(
            variant,
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
                    assert_eq!(
                        street_index(hand.street()),
                        game.betting(&s).street,
                        "street"
                    );
                    let actions = game.actions(&s).to_vec();
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
                    let b = game.betting(&s);
                    for seat in 0..2 {
                        assert_eq!(
                            hand.contributed(seat),
                            b.contributed[seat],
                            "contributed after {a:?}"
                        );
                        if !hand.is_complete() {
                            assert_eq!(hand.stack(seat), b.stack[seat], "stack after {a:?}");
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
            pot_limit: false,
        };
        fuzz(config, 1500, stack);
    }
}

#[test]
fn pot_limit_actions_are_legal_in_ducy_play() {
    // Pot-limit Hold'em and PLO, at 100 BB, short and deep; the PLO menu and
    // an odd one whose sizes go over the pot (they're capped at it).
    use Size::*;
    for stack in [200, 37, 1000] {
        let config = HunlConfig {
            stack,
            ..HunlConfig::pot_limit_omaha()
        };
        fuzz(config.clone(), 1500, stack);
        fuzz_holes::<4>(config, 1500, stack + 1);
    }
    let odd = HunlConfig {
        small_blind: 2,
        big_blind: 5,
        stack: 333,
        menu: BetMenu {
            preflop: vec![vec![Bb(2.0), Pot(3.0), AllIn], vec![Times(4.0), AllIn]],
            postflop: vec![vec![Pot(0.01), Pot(2.0), AllIn], vec![AllIn]],
        },
        pot_limit: true,
    };
    fuzz_holes::<4>(odd, 1500, 9);
}

#[test]
fn omaha_five_and_six_cards_play_through_too() {
    fuzz_holes::<5>(HunlConfig::pot_limit_omaha(), 500, 5);
    fuzz_holes::<6>(HunlConfig::pot_limit_omaha(), 500, 6);
}

#[test]
fn pot_limit_caps_bets_at_the_pot() {
    let game = Hunl::new(HunlConfig::pot_limit_omaha(), None);
    let s = game.deal([[0, 1], [2, 3]], [4, 5, 6, 7, 8]);
    // Preflop the button may raise to 2.5 BB or the pot: call 1 to make it
    // 4, raise 4, to 6. No all-in at 100 BB.
    assert_eq!(
        game.actions(&s),
        vec![
            HunlAction::Fold,
            HunlAction::Call,
            HunlAction::Raise(5),
            HunlAction::Raise(6)
        ]
    );
    // Every raise anywhere in the tree is at most the pot after calling,
    // and all-in shows up only when it fits under that.
    let mut all_ins = 0;
    for n in &game.tree.nodes {
        let b = &n.betting;
        let cap = b.current_bet + b.pot() + b.to_call();
        for a in &n.actions {
            if let HunlAction::Bet(to) | HunlAction::Raise(to) = *a {
                assert!(to <= cap, "{a:?} over the pot ({cap}) at {b:?}");
                if to == b.all_in_to() {
                    all_ins += 1;
                }
            }
        }
    }
    assert!(all_ins > 0, "all-in once the pot is big enough");
}

#[test]
fn no_limit_trees_are_unchanged() {
    // The same counts as before pot-limit existed (see tree_statistics).
    let game = Hunl::new(
        HunlConfig {
            pot_limit: false,
            ..HunlConfig::default()
        },
        None,
    );
    assert_eq!(game.tree_stats().sequences, [16, 180, 1532, 9236]);
}

#[test]
fn combo_indexes_cover_every_hand_once() {
    use ducy_gto::holdem::cards::{NUM_HOLES, combo_cards, combo_index, hole_index, num_combos};
    assert_eq!(num_combos(2), NUM_HOLES);
    assert_eq!(
        [num_combos(4), num_combos(5), num_combos(6)],
        [270_725, 2_598_960, 20_358_520]
    );
    // Two cards: the same index as Hold'em's.
    for a in 0..52u8 {
        for b in 0..a {
            assert_eq!(combo_index(&[a, b]), hole_index(a, b));
        }
    }
    // Four cards: every hand gets its own index, and back again.
    let mut seen = vec![false; num_combos(4)];
    for a in 0..52u8 {
        for b in 0..a {
            for c in 0..b {
                for d in 0..c {
                    let i = combo_index(&[a, c, d, b]);
                    assert!(!seen[i], "index {i} twice");
                    seen[i] = true;
                    if i % 997 == 0 {
                        assert_eq!(combo_cards(i, 4), vec![d, c, b, a]);
                    }
                }
            }
        }
    }
    assert!(seen.iter().all(|&s| s));
}

#[test]
fn omaha_showdowns_use_exactly_two_hole_cards() {
    use ducy_gto::holdem::cards::{mask, omaha_score, parse, score};
    let c = |s: &str| parse(s).unwrap();
    // Four spades on board, one spade in hand: no flush in Omaha.
    let board = c("2s 7s 9s Ks 3d");
    let one_spade = c("As Ad Kd Qc");
    let two_spades = c("As Qs 4c 4d");
    assert!(omaha_score(&two_spades, &board) > omaha_score(&one_spade, &board));
    // In Hold'em the one spade (with any other card) would make the flush.
    assert!(score(mask(&board) | mask(&c("As Kd"))) > omaha_score(&one_spade, &board));
    // Quads on board can't play: aces full (the board's ace and two eights,
    // with two aces from hand), not quads.
    let quads = c("8c 8d 8h 8s Ac");
    let aces = c("Ad Ah 2c 3c");
    assert_eq!(
        omaha_score(&aces, &quads),
        score(mask(&c("Ac Ad Ah 8c 8d"))),
        "aces full, not quads"
    );
    // Hold'em would play the quads.
    assert!(omaha_score(&aces, &quads) < score(mask(&quads) | mask(&c("Ad Ah"))));
}

#[test]
fn plo_deals_flow_through_buckets_and_training() {
    // A placeholder abstraction: Omaha hands in buckets by their best two
    // ranks, just to show four-card hands go deal → buckets → MCCFR.
    struct TopRanks;
    impl Buckets for TopRanks {
        fn hand_bucket(&self, hole: &[Card], board: &[Card]) -> u16 {
            assert_eq!(hole.len(), 4);
            assert!(matches!(board.len(), 0 | 3 | 4 | 5));
            let mut r: Vec<u16> = hole.iter().map(|&c| (c / 4) as u16).collect();
            r.sort_unstable();
            r[3] / 4 * 4 + r[2] / 4
        }
        fn bucket_count(&self, _: usize) -> usize {
            16
        }
    }
    let config = HunlConfig {
        stack: 40,
        ..HunlConfig::pot_limit_omaha()
    };
    let game = HuPlo::with_cards(config, Some(&TopRanks));
    let mut rng = Rng::new(4);
    let s = game.sample_chance(&game.root(), &mut rng);
    let all: Vec<Card> = s.hole.iter().flatten().chain(&s.board).copied().collect();
    let mut distinct = all.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), 13, "13 different cards dealt");
    let mut m = Mccfr::new(&game, Config::default());
    m.run(2000);
    assert!(
        m.num_infosets() > 100,
        "{} information sets",
        m.num_infosets()
    );
    let strategy = m
        .average_at(&game.info(&s))
        .expect("the first decision was visited");
    assert!((strategy.iter().sum::<f64>() - 1.0).abs() < 1e-6);
}

#[test]
fn heads_up_order_and_blinds() {
    let game = Hunl::new(HunlConfig::default(), None);
    let s = game.deal([[0, 1], [2, 3]], [4, 5, 6, 7, 8]);
    // The button (player 0) posts the small blind and acts first preflop.
    assert_eq!(game.betting(&s).contributed, [1, 2]);
    assert_eq!(game.betting(&s).to_act, 0);
    let a = game.actions(&s).to_vec();
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
    assert_eq!((game.betting(&s).street, game.betting(&s).to_act), (1, 1));
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
    assert_eq!(stats.sequences, [16, 180, 1532, 9236]);
    assert_eq!(stats.actions, [45, 512, 4264, 24976]);
    // Children are numbered after their parent, and every action leads somewhere.
    for (i, n) in game.tree.nodes.iter().enumerate() {
        assert_eq!(n.actions.len(), n.children.len());
        assert!(n.children.iter().all(|&c| c as usize > i));
    }
}

#[test]
fn information_sets_depend_on_betting_and_bucket_only() {
    let game = Hunl::new(HunlConfig::default(), None);
    let mut rng = Rng::new(3);
    let a = game.sample_chance(&game.root(), &mut rng);
    let b = game.sample_chance(&game.root(), &mut rng);
    // No abstraction: every hand is bucket 0, so different deals share keys.
    assert_eq!(game.info(&a), game.info(&b));
    assert_ne!(game.info(&a), game.info(&game.apply(&a, 1)));
}
