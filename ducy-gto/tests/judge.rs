//! `ducy_gto::judge` on tiny models: a six-handed hand two players took to
//! the flop is scored for both of them, from the flop on, in Hold'em (with
//! values) and in PLO (frequencies only).

use ducy_gto::{
    Profile,
    holdem::{
        abstraction::CardAbstraction,
        blueprint::Blueprint,
        cards::parse,
        hunl::{HuPlo, Hunl, HunlConfig},
        range::BucketCache,
        review::{ReviewConfig, Reviewer},
    },
    judge::{self, TableHand},
    omaha::abstraction::{PloAbstraction, PloAbstractionConfig},
};
use ducy_play::{Event, Street};

fn board(street: Street, cards: &str) -> Event {
    Event::Board {
        street,
        cards: cards
            .split_whitespace()
            .map(|c| c.parse().expect("card"))
            .collect(),
    }
}

/// Six seats, the button on 3, blinds 1 and 2, stacks `stack`: UTG (seat 0)
/// opens, the button calls, the rest fold; then a bet and a call on each
/// street. `holes`: seat 0's and seat 3's cards.
fn hand(stack: u64, holes: [&str; 2], cards: [&str; 3]) -> TableHand {
    let bet = |seat, to| Event::Bet {
        seat,
        to,
        all_in: false,
    };
    let call = |seat, amount| Event::Call {
        seat,
        amount,
        all_in: false,
    };
    let mut h = vec![None; 6];
    h[0] = parse(holes[0]);
    h[3] = parse(holes[1]);
    TableHand {
        history: vec![
            Event::SmallBlind { seat: 4, amount: 1 },
            Event::BigBlind { seat: 5, amount: 2 },
            Event::Raise {
                seat: 0,
                to: 5,
                all_in: false,
            },
            Event::Fold { seat: 1 },
            Event::Fold { seat: 2 },
            call(3, 5),
            Event::Fold { seat: 4 },
            Event::Fold { seat: 5 },
            board(Street::Flop, cards[0]),
            bet(0, 6),
            call(3, 6),
            board(Street::Turn, cards[1]),
            bet(0, 15),
            call(3, 15),
            board(Street::River, cards[2]),
            Event::Check { seat: 0 },
            Event::Check { seat: 3 },
        ],
        button: 3,
        small_blind: 1,
        big_blind: 2,
        stacks: vec![stack; 6],
        holes: h,
        board: parse(&cards.join(" ")).expect("board"),
    }
}

#[test]
fn a_holdem_hand_is_scored_for_both_players_from_the_flop_on() {
    let cards = CardAbstraction::quick(8);
    let config = HunlConfig::default();
    let game = Hunl::new(config.clone(), Some(&cards));
    // Every action equally likely.
    let blueprint = Blueprint::from_profile(&game, &cards, &Profile::new());
    let reviewer = Reviewer {
        tree: &game.tree,
        blueprint: &blueprint,
        cards: &cards,
        tree_big_blind: config.big_blind,
        config: ReviewConfig {
            flop_runouts: 4,
            ..ReviewConfig::default()
        },
    };
    let h = hand(200, ["Ah Kd", "9c 9s"], ["Ks 9d 4h", "2c", "7d"]);
    let scored = judge::holdem(&reviewer, &h, &mut BucketCache::default());
    for seat in [0, 3] {
        let streets: Vec<usize> = scored
            .iter()
            .filter(|d| d.seat == seat)
            .map(|d| d.street)
            .collect();
        assert_eq!(streets, [1, 2, 3], "seat {seat}");
    }
    assert!(scored.iter().all(|d| d.seat == 0 || d.seat == 3));
    for d in &scored {
        // A uniform strategy: every play is as frequent as the top one.
        assert!((d.frequency - d.top).abs() < 1e-9, "{d:?}");
        assert!(d.ev_loss.is_some_and(|x| x >= 0.0), "{d:?}");
    }

    // Without a player's cards, only the other is scored.
    let mut one = h.clone();
    one.holes[3] = None;
    assert!(
        judge::holdem(&reviewer, &one, &mut BucketCache::default())
            .iter()
            .all(|d| d.seat == 0)
    );
}

#[test]
fn a_plo_hand_gets_the_blueprints_frequencies() {
    let cards = PloAbstraction::build(
        PloAbstractionConfig {
            preflop: 0,
            flop: 8,
            turn: 8,
            river: 8,
            fit_hands: 300,
            equity_samples: 30,
            ..PloAbstractionConfig::default()
        },
        |_| {},
    );
    let config = HunlConfig::pot_limit_omaha_lean(100);
    let game = HuPlo::with_cards(config, Some(&cards));
    let blueprint = Blueprint::from_profile(&game, &cards, &Profile::new());
    let h = hand(
        200,
        ["Ah Kd Qs Jc", "9c 9s 8h 7h"],
        ["Ks 9d 4h", "2c", "7d"],
    );
    let scored = judge::plo(&cards, &blueprint, &game.tree, &h, 1);
    assert!(!scored.is_empty());
    for d in &scored {
        assert!(
            [0, 3].contains(&d.seat) && (1..=3).contains(&d.street),
            "{d:?}"
        );
        assert!(d.frequency > 0.0 && d.frequency <= d.top + 1e-9, "{d:?}");
        assert_eq!(d.ev_loss, None);
    }
    // Two-card hands aren't this model's.
    let holdem = hand(200, ["Ah Kd", "9c 9s"], ["Ks 9d 4h", "2c", "7d"]);
    assert!(judge::plo(&cards, &blueprint, &game.tree, &holdem, 1).is_empty());
}
