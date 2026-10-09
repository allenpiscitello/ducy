//! Running it twice: when everyone left is all-in with board cards to come,
//! the players may run the rest of the board twice, each pot split between
//! the two boards. At a hosted table every person still in must agree; bots
//! go along with them.

use ducy::deck::{Card, Deck};
use ducy_play::{
    Action, Awaiting, Command, Deal, Event, Hand, HandSnapshot, Outgoing, Table, TableHost,
    TableRules, TableSeat, Update, Variant,
};

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

/// Heads-up: aces against kings. The first board (no king) goes to the
/// aces, the second (a king) to the kings.
fn aces_kings(stacks: &[u64]) -> Hand {
    let board: [Card; 5] = cards("2c 7d 9h Jc 3s").try_into().unwrap();
    let deal = Deal::new(Variant::Holdem, vec![deck("As Ah"), deck("Ks Kh")], board)
        .unwrap()
        .with_rest(cards("Kd 4c 5d 8s Tc"))
        .unwrap();
    let mut hand = Hand::new(TableRules::no_limit_holdem(1, 2), stacks, 0, deal).unwrap();
    hand.offer_run_twice(true);
    hand
}

#[test]
fn all_in_preflop_waits_for_the_choice_then_splits_each_pot_between_the_boards() {
    let mut hand = aces_kings(&[100, 100]);
    hand.act(Action::AllIn).unwrap();
    hand.act(Action::Call).unwrap();
    assert_eq!(hand.awaiting(), Some(&Awaiting::RunChoice(vec![0, 1])));
    assert!(hand.to_act().is_none());
    assert!(hand.board().is_empty(), "nothing dealt yet");
    hand.choose_runs(2).unwrap();
    let r = hand.result().expect("over");
    assert_eq!(r.payouts, vec![100, 100], "a board each");
    assert_eq!(hand.second_board(), cards("Kd 4c 5d 8s Tc"));
    let events = hand.events();
    assert!(events.contains(&Event::Runs { count: 2 }));
    assert!(events.contains(&Event::SecondBoard {
        cards: cards("Kd 4c 5d 8s Tc")
    }));
}

#[test]
fn running_it_once_is_the_usual_hand() {
    let mut hand = aces_kings(&[100, 100]);
    hand.act(Action::AllIn).unwrap();
    hand.act(Action::Call).unwrap();
    hand.choose_runs(1).unwrap();
    assert_eq!(hand.result().unwrap().payouts, vec![200, 0]);
    assert!(hand.second_board().is_empty());
    assert!(
        !hand
            .events()
            .iter()
            .any(|e| matches!(e, Event::SecondBoard { .. }))
    );
}

#[test]
fn without_the_offer_the_board_just_runs() {
    let board: [Card; 5] = cards("2c 7d 9h Jc 3s").try_into().unwrap();
    let deal = Deal::new(Variant::Holdem, vec![deck("As Ah"), deck("Ks Kh")], board).unwrap();
    let mut hand = Hand::new(TableRules::no_limit_holdem(1, 2), &[100, 100], 0, deal).unwrap();
    hand.act(Action::AllIn).unwrap();
    hand.act(Action::Call).unwrap();
    assert!(hand.is_complete());
    assert!(hand.choose_runs(2).is_err());
}

#[test]
fn all_in_on_the_flop_shares_the_flop() {
    let mut hand = aces_kings(&[100, 100]);
    hand.act(Action::Call).unwrap();
    hand.act(Action::Check).unwrap();
    // The flop: no betting possible after this all-in and call.
    hand.act(Action::AllIn).unwrap();
    hand.act(Action::Call).unwrap();
    assert!(matches!(hand.awaiting(), Some(Awaiting::RunChoice(_))));
    assert_eq!(hand.board(), cards("2c 7d 9h"));
    hand.choose_runs(2).unwrap();
    assert_eq!(hand.second_board(), cards("2c 7d 9h Kd 4c"));
    assert!(hand.events().contains(&Event::SecondBoard {
        cards: cards("Kd 4c")
    }));
    assert_eq!(hand.result().unwrap().payouts, vec![100, 100]);
}

#[test]
fn an_odd_chip_goes_to_the_first_board() {
    // Three players: Bo folds his small blind, so the pot is 201.
    let board: [Card; 5] = cards("3c 7d 9h Jc 4s").try_into().unwrap();
    let deal = Deal::new(
        Variant::Holdem,
        vec![deck("As Ah"), deck("2d 2s"), deck("Ks Kh")],
        board,
    )
    .unwrap()
    .with_rest(cards("Kd 5c 6d 8s Tc"))
    .unwrap();
    let mut hand = Hand::new(TableRules::no_limit_holdem(1, 2), &[100, 100, 100], 0, deal).unwrap();
    hand.offer_run_twice(true);
    hand.act(Action::AllIn).unwrap();
    hand.act(Action::Fold).unwrap();
    hand.act(Action::Call).unwrap();
    assert_eq!(hand.awaiting(), Some(&Awaiting::RunChoice(vec![0, 2])));
    hand.choose_runs(2).unwrap();
    assert_eq!(hand.result().unwrap().payouts, vec![101, 0, 100]);
}

#[test]
fn a_hand_saved_before_or_after_the_choice_restores() {
    let mut hand = aces_kings(&[100, 100]);
    hand.act(Action::AllIn).unwrap();
    hand.act(Action::Call).unwrap();
    let json = serde_json::to_string(&hand.snapshot()).unwrap();
    let mut back = Hand::restore(&serde_json::from_str::<HandSnapshot>(&json).unwrap()).unwrap();
    assert_eq!(back.awaiting(), hand.awaiting());
    back.choose_runs(2).unwrap();
    hand.choose_runs(2).unwrap();
    assert_eq!(back.result(), hand.result());
    let json = serde_json::to_string(&hand.snapshot()).unwrap();
    let again = Hand::restore(&serde_json::from_str::<HandSnapshot>(&json).unwrap()).unwrap();
    assert_eq!(again.events(), hand.events());
    assert_eq!(again.second_board(), hand.second_board());
}

// ---- at a hosted table ----

fn club(turn_ms: u64) -> TableHost {
    let seats = (0..2).map(|_| TableSeat::empty()).collect();
    let mut table = Table::new(TableRules::no_limit_holdem(1, 2), seats, 40, 7).unwrap();
    table.set_run_it_twice(true);
    let mut h = TableHost::without_host(table, turn_ms, 40, 200).unwrap();
    for (c, seat) in [("a", 0), ("b", 1)] {
        h.handle(
            c,
            Command::Join {
                name: c.to_string(),
            },
            0,
        );
        h.handle(c, Command::RequestChips { amount: 100 }, 0);
        h.approve_chips(seat, 0);
    }
    h
}

/// Both all-in preflop at a club table.
fn all_in(h: &mut TableHost) {
    h.new_hand(1).unwrap();
    for _ in 0..2 {
        let seat = h.table().to_act().unwrap();
        let id = ["a", "b"][seat];
        let legal = h.table().view(seat).legal.unwrap();
        let kind = if legal.call.is_some() && legal.raise.is_none() {
            "call"
        } else {
            "allin"
        };
        let seq = h.seq();
        h.handle(
            id,
            Command::Act {
                seq,
                kind: kind.to_string(),
                amount: 0,
            },
            2,
        );
    }
    assert!(
        matches!(h.awaiting(), Some(Awaiting::RunChoice(_))),
        "{:?}",
        h.awaiting()
    );
}

fn runs_of(h: &TableHost) -> u8 {
    h.table().hand().unwrap().runs()
}

fn my_answer(out: &[Outgoing], client: &str) -> Option<bool> {
    out.iter().rev().find_map(|o| match &o.update {
        Update::State { me, .. } if o.to == client => me.as_ref().map(|m| m.run_twice),
        _ => None,
    })?
}

#[test]
fn both_players_must_agree_to_run_it_twice() {
    let mut h = club(10_000);
    all_in(&mut h);
    let view = h.table().view(0);
    assert!(view.run_choice, "the view says it's asking");
    let out = h.handle("a", Command::RunTwice { yes: true }, 3);
    assert_eq!(
        my_answer(&out, "a"),
        Some(true),
        "Ann's answer is in her view"
    );
    assert!(h.table().in_hand(), "waiting for Bo");
    h.handle("b", Command::RunTwice { yes: true }, 3);
    assert_eq!(runs_of(&h), 2);
    assert!(!h.table().in_hand());
    assert_eq!(h.table().view(0).second_board.len(), 5);

    // Next time Bo says no: it's run once.
    let mut h = club(10_000);
    all_in(&mut h);
    h.handle("a", Command::RunTwice { yes: true }, 3);
    h.handle("b", Command::RunTwice { yes: false }, 3);
    assert_eq!(runs_of(&h), 1);
}

#[test]
fn no_answer_in_time_or_someone_away_runs_it_once() {
    let mut h = club(10_000);
    all_in(&mut h);
    h.handle("a", Command::RunTwice { yes: true }, 3);
    assert!(h.turn_ms_left(3).is_some(), "choosing is on the clock");
    h.tick(3 + 10_000);
    assert_eq!(runs_of(&h), 1);

    let mut h = club(0);
    all_in(&mut h);
    h.handle("a", Command::RunTwice { yes: true }, 3);
    h.disconnected("b", 4);
    assert_eq!(runs_of(&h), 1, "Bo left: once");
}

#[test]
fn bots_go_along_with_the_person() {
    // The host against a bot, all-in: the host's yes is enough.
    let seats = vec![
        TableSeat::human("Host", "you"),
        TableSeat::bot(
            ducy_play::Personality::ALL[0].id(),
            ducy_play::Personality::ALL[0].bot(Some(1)),
        ),
    ];
    let mut table = Table::new(TableRules::no_limit_holdem(1, 2), seats, 100, 3).unwrap();
    table.set_run_it_twice(true);
    let mut h = TableHost::new(table, vec![false, false], 0).unwrap();
    for _ in 0..20 {
        h.new_hand(1).unwrap();
        // The host shoves; the bot does what it does.
        while h.table().in_hand() && h.awaiting().is_none() {
            if h.auto_to_act() {
                h.advance(2).unwrap();
            } else if h.table().to_act() == Some(0) {
                let legal = h.host_view().legal.unwrap();
                let kind = if legal.raise.is_some() || legal.bet.is_some() {
                    "allin"
                } else {
                    "call"
                };
                h.host_act(kind, 0, 2).unwrap();
            }
        }
        if matches!(h.awaiting(), Some(Awaiting::RunChoice(_))) {
            h.host_run_twice(true, 3).unwrap();
            assert_eq!(runs_of(&h), 2, "the bot went along");
            return;
        }
    }
    panic!("the bot never called an all-in");
}
