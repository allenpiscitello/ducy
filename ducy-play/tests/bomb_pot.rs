//! Bomb pots: everyone antes, there's no preflop betting, and the hand starts
//! on the flop. On a double board a second board is dealt alongside the
//! first, and each pot is split between the best hand on each.

use ducy::deck::{Card, Deck};
use ducy_play::{Action, Deal, Event, Hand, PlayError, Street, TableRules, Variant};

fn card(s: &str) -> Card {
    Card::parse(s).unwrap()
}

fn cards(s: &str) -> Vec<Card> {
    s.split_whitespace().map(card).collect()
}

fn deck(s: &str) -> Deck {
    let mut d = Deck::empty();
    for c in cards(s) {
        d |= c;
    }
    d
}

/// Three players at PLO. Aces win the first board; the second board's king
/// gives the kings trips.
fn deal() -> Deal {
    let board: [Card; 5] = cards("2c 7d 9h Jc 3s").try_into().unwrap();
    Deal::new(
        Variant::Omaha { hole_cards: 4 },
        vec![
            deck("As Ah Qd 6h"),
            deck("Ks Kh 6c 6d"),
            deck("Qs Qh 8d 8c"),
        ],
        board,
    )
    .unwrap()
    .with_rest(cards("Kd 4c 5d 8s Tc"))
    .unwrap()
}

fn bomb(double_board: bool) -> Hand {
    let rules = TableRules::pot_limit_omaha(2, 5).bomb_pot(5, double_board);
    Hand::new(rules, &[500, 500, 500], 0, deal()).unwrap()
}

#[test]
fn everyone_antes_and_it_starts_on_the_flop() {
    let hand = bomb(false);
    let events = hand.events();
    assert_eq!(
        &events[..3],
        &[
            Event::Ante { seat: 0, amount: 5 },
            Event::Ante { seat: 1, amount: 5 },
            Event::Ante { seat: 2, amount: 5 },
        ]
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::SmallBlind { .. } | Event::BigBlind { .. })),
        "no blinds"
    );
    assert_eq!(hand.street(), Street::Flop);
    assert_eq!(hand.board(), cards("2c 7d 9h"));
    assert_eq!(hand.pot(), 15);
    // First to act is left of the button, and may check or bet from the big blind up.
    let legal = hand.legal_actions().unwrap();
    assert_eq!(legal.seat, 1);
    assert!(legal.can_check);
    assert_eq!(legal.bet.unwrap().min_to, 5);
    assert!(hand.second_board().is_empty(), "one board");
}

#[test]
fn a_double_board_deals_both_boards_and_splits_the_pot_between_them() {
    let mut hand = bomb(true);
    assert_eq!(hand.second_board(), cards("Kd 4c 5d"));
    assert!(hand.events().contains(&Event::SecondBoard {
        cards: cards("Kd 4c 5d")
    }));
    for _ in 0..3 {
        hand.act(Action::Check).unwrap();
    }
    assert_eq!(hand.street(), Street::Turn);
    assert_eq!(hand.second_board(), cards("Kd 4c 5d 8s"));
    while !hand.is_complete() {
        hand.act(Action::Check).unwrap();
    }
    assert_eq!(hand.second_board(), cards("Kd 4c 5d 8s Tc"));
    let r = hand.result().unwrap();
    assert!(r.showdown);
    // Aces take the first board's half (and the odd chip), kings the second's.
    assert_eq!(r.payouts, vec![8, 7, 0]);
}

#[test]
fn betting_works_as_usual_from_the_flop() {
    let mut hand = bomb(true);
    hand.act(Action::Bet(15)).unwrap();
    hand.act(Action::Fold).unwrap();
    hand.act(Action::Fold).unwrap();
    let r = hand.result().unwrap();
    assert!(!r.showdown);
    assert_eq!(r.payouts, vec![0, 30, 0]);
}

#[test]
fn a_saved_bomb_pot_restores() {
    let mut hand = bomb(true);
    hand.act(Action::Bet(10)).unwrap();
    hand.act(Action::Call).unwrap();
    let restored = Hand::restore(&hand.snapshot()).unwrap();
    assert_eq!(restored.events(), hand.events());
    assert_eq!(restored.second_board(), hand.second_board());
    assert_eq!(restored.to_act(), hand.to_act());
}

#[test]
fn a_bomb_pot_needs_an_ante() {
    let rules = TableRules::pot_limit_omaha(2, 5).bomb_pot(0, false);
    assert_eq!(
        Hand::new(rules, &[500, 500, 500], 0, deal()).err(),
        Some(PlayError::InvalidSetup)
    );
}
