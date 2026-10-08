//! Pluggable dealing (#135): hands whose cards the engine never sees. Hole
//! cards stay with their players until showdown, the board arrives street
//! by street, and only shown hands can win.

use ducy::deck::{Card, Deck};
use ducy_play::{
    Action, Awaiting, Event, Hand, HandSnapshot, HiddenDeal, PlayError, Street, Table, TableRules,
    TableSeat,
};

fn cards(s: &str) -> Vec<Card> {
    s.split(' ').map(|c| Card::parse(c).unwrap()).collect()
}

fn deck(s: &str) -> Deck {
    Deck::parse(s).unwrap()
}

fn hidden(stacks: &[u64], button: usize) -> Hand {
    let rules = TableRules::no_limit_holdem(1, 2);
    Hand::with_dealer(
        rules,
        stacks,
        button,
        &HiddenDeal {
            players: stacks.len(),
        },
        &[],
    )
    .unwrap()
}

/// Checks or calls while anyone has to act.
fn check_down(hand: &mut Hand) {
    while let Some(legal) = hand.legal_actions() {
        hand.act(if legal.can_check {
            Action::Check
        } else {
            Action::Call
        })
        .unwrap();
    }
}

/// Plays the whole board out checking, feeding each street.
fn run_board(hand: &mut Hand, board: &str) {
    let b = cards(board);
    for (street, range) in [
        (Street::Flop, 0..3),
        (Street::Turn, 3..4),
        (Street::River, 4..5),
    ] {
        check_down(hand);
        assert_eq!(hand.awaiting(), Some(&Awaiting::Board(street)));
        hand.deal_board(&b[range]).unwrap();
    }
    check_down(hand);
}

#[test]
fn the_engine_never_holds_hidden_hole_cards() {
    let hand = hidden(&[100, 100, 100], 0);
    assert!(hand.deal().is_none());
    assert!((0..3).all(|s| hand.hole_cards(s).is_none()));
    // A player to act sees no cards from the engine: their own come from the shuffle.
    let seat = hand.to_act().unwrap();
    assert_eq!(hand.observation(seat).unwrap().hole_cards, Deck::empty());
}

#[test]
fn the_board_waits_for_each_street_and_is_checked() {
    let mut hand = hidden(&[100, 100, 100], 0);
    check_down(&mut hand);
    assert_eq!(hand.awaiting(), Some(&Awaiting::Board(Street::Flop)));
    assert_eq!(hand.to_act(), None, "nobody acts while the flop is coming");
    assert_eq!(hand.act(Action::Check), Err(PlayError::HandComplete));
    assert_eq!(
        hand.deal_board(&cards("2c 7d")),
        Err(PlayError::InvalidDeal),
        "three for the flop"
    );
    assert_eq!(
        hand.deal_board(&cards("2c 7d 2c")),
        Err(PlayError::InvalidDeal),
        "no card twice"
    );
    hand.deal_board(&cards("2c 7d 9h")).unwrap();
    assert_eq!(hand.street(), Street::Flop);
    assert_eq!(hand.board(), cards("2c 7d 9h").as_slice());
    check_down(&mut hand);
    assert_eq!(
        hand.deal_board(&cards("2c")),
        Err(PlayError::InvalidDeal),
        "already on the board"
    );
    hand.deal_board(&cards("Jc")).unwrap();
    assert_eq!(hand.street(), Street::Turn);
    assert_eq!(
        hand.events().last(),
        Some(&Event::Board {
            street: Street::Turn,
            cards: cards("Jc")
        })
    );
    assert!(hand.to_act().is_some(), "betting resumes on the turn");
}

#[test]
fn only_shown_hands_win_and_a_player_who_does_not_show_forfeits() {
    // Seat 1 would have the best hand, but doesn't show.
    let mut hand = hidden(&[100, 100, 100], 0);
    run_board(&mut hand, "2c 7d 9h Jc 3s");
    assert_eq!(hand.awaiting(), Some(&Awaiting::Reveals(vec![0, 1, 2])));
    hand.reveal(0, deck("Ah Kh")).unwrap();
    assert_eq!(
        hand.reveal(0, deck("Ah Kh")),
        Err(PlayError::IllegalAction),
        "only once"
    );
    assert_eq!(
        hand.reveal(2, deck("Ah Qd")),
        Err(PlayError::InvalidDeal),
        "a card already shown"
    );
    assert_eq!(
        hand.reveal(2, deck("Qd")),
        Err(PlayError::InvalidDeal),
        "two cards"
    );
    hand.forfeit(1).unwrap();
    assert!(hand.has_forfeited(1));
    hand.reveal(2, deck("Qd Qs")).unwrap();
    let r = hand
        .result()
        .expect("settled once everyone has shown or forfeited");
    assert!(r.showdown);
    assert_eq!(
        r.payouts,
        vec![0, 0, 6],
        "queens beat ace-king; seat 1 can't win"
    );
    assert_eq!(hand.hole_cards(1), None, "never shown");
    assert_eq!(hand.hole_cards(2), Some(deck("Qd Qs")));
    assert!(hand.events().contains(&Event::Forfeit { seat: 1 }));
    assert!(
        hand.events()
            .iter()
            .any(|e| matches!(e, Event::Reveal { seat: 2, .. }))
    );
}

#[test]
fn if_everyone_else_forfeits_the_last_player_in_wins_unseen() {
    let mut hand = hidden(&[100, 100], 0);
    run_board(&mut hand, "2c 7d 9h Jc 3s");
    assert_eq!(hand.awaiting(), Some(&Awaiting::Reveals(vec![0, 1])));
    hand.forfeit(0).unwrap();
    let r = hand.result().expect("no one left to show against");
    assert_eq!(r.payouts, vec![0, 4]);
    assert_eq!(hand.hole_cards(1), None, "the winner never had to show");
}

#[test]
fn if_everyone_forfeits_they_share_the_pot() {
    let mut hand = hidden(&[100, 100, 100], 0);
    run_board(&mut hand, "2c 7d 9h Jc 3s");
    for s in 0..3 {
        hand.forfeit(s).ok();
    }
    let r = hand.result().expect("settled");
    assert_eq!(r.payouts.iter().sum::<u64>(), 6, "no chips lost");
}

#[test]
fn folding_out_needs_no_board_or_reveal() {
    let mut hand = hidden(&[100, 100, 100], 0);
    // Seat 0 (button) acts first three-handed preflop; seats 0 and 1 fold.
    hand.act(Action::Fold).unwrap();
    hand.act(Action::Fold).unwrap();
    let r = hand.result().expect("over");
    assert!(!r.showdown);
    assert_eq!(r.payouts, vec![0, 0, 3]);
    assert_eq!(hand.awaiting(), None);
}

#[test]
fn an_all_in_runout_waits_for_each_street_then_the_reveals() {
    let mut hand = hidden(&[50, 50], 0);
    hand.act(Action::AllIn).unwrap();
    hand.act(Action::Call).unwrap();
    assert_eq!(hand.awaiting(), Some(&Awaiting::Board(Street::Flop)));
    hand.deal_board(&cards("2c 7d 9h")).unwrap();
    assert_eq!(
        hand.awaiting(),
        Some(&Awaiting::Board(Street::Turn)),
        "no betting: straight on"
    );
    hand.deal_board(&cards("Jc")).unwrap();
    hand.deal_board(&cards("3s")).unwrap();
    assert_eq!(hand.awaiting(), Some(&Awaiting::Reveals(vec![0, 1])));
    hand.reveal(1, deck("Jh Js")).unwrap();
    hand.reveal(0, deck("Ah Ad")).unwrap();
    assert_eq!(
        hand.result().unwrap().payouts,
        vec![0, 100],
        "a set of jacks"
    );
}

#[test]
fn a_hidden_hand_is_not_saved_for_resuming() {
    let mut hand = hidden(&[100, 100], 0);
    hand.act(Action::Call).unwrap();
    let snap: HandSnapshot = hand.snapshot();
    assert_eq!(Hand::restore(&snap).err(), Some(PlayError::InvalidSnapshot));
}

#[test]
fn a_table_deals_hidden_hands_and_maps_seats() {
    let seats = ["A", "B", "C", "D"]
        .iter()
        .map(|n| TableSeat::human(*n, n.to_lowercase()))
        .collect();
    let mut t = Table::new(TableRules::no_limit_holdem(1, 2), seats, 200, 1).unwrap();
    t.seat_mut(1).sitting_out = true;
    t.new_hand_hidden().unwrap();
    assert_eq!(t.dealt(), &[0, 2, 3]);
    // Nobody's cards are in anyone's view.
    for s in [0, 2, 3] {
        assert!(
            t.view(s).seats.iter().all(|seat| seat.cards.is_none()),
            "seat {s}"
        );
    }
    // An away player is still folded or checked for at their turn, as always.
    let away = t.to_act().unwrap();
    t.seat_mut(away).away = true;
    assert!(t.advance().unwrap());
    let play_on = |t: &mut Table| {
        while let Some(seat) = t.to_act() {
            let legal = t.hand().unwrap().legal_actions().unwrap();
            t.act(
                seat,
                if legal.can_check {
                    Action::Check
                } else {
                    Action::Call
                },
            )
            .unwrap();
        }
    };
    play_on(&mut t);
    assert_eq!(t.awaiting(), Some(Awaiting::Board(Street::Flop)));
    t.deal_board(&cards("2c 7d 9h")).unwrap();
    for c in ["Jc", "3s"] {
        play_on(&mut t);
        t.deal_board(&cards(c)).unwrap();
    }
    play_on(&mut t);
    let Some(Awaiting::Reveals(seats)) = t.awaiting() else {
        panic!("showdown")
    };
    assert!(
        seats.iter().all(|s| [0, 2, 3].contains(s)),
        "table seats: {seats:?}"
    );
    // Seat 3 shows first (were the others to forfeit first, it would win unseen).
    assert!(seats.contains(&3));
    t.reveal(3, deck("Qd Qs")).unwrap();
    for s in seats.into_iter().filter(|&s| s != 3) {
        t.forfeit(s).unwrap();
    }
    assert!(t.hand().unwrap().is_complete());
    assert_eq!(
        t.view(0).seats.iter().filter(|s| s.cards.is_some()).count(),
        1,
        "only the shown hand is visible"
    );
}
