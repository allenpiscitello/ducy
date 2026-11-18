//! Running it twice: when everyone left is all-in with board cards to come,
//! the players may run the rest of the board twice, each pot split between
//! the two boards. At a hosted table it's settled pot by pot: a pot runs
//! twice when every person who can win it agrees; bots go along with them.

use ducy::deck::{Card, Deck};
use ducy_host::{Command, Outgoing, TableHost, Update};
use ducy_play::{
    Action, Awaiting, Deal, Event, Hand, HandSnapshot, Table, TableRules, TableSeat, Variant,
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
    assert!(events.contains(&Event::Runs {
        count: 2,
        pots: vec![2]
    }));
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

// ---- three-way: a short stack and two big stacks ----

/// Ann (50), Bo and Cy (100 each) at a club table, all-in preflop: a main
/// pot for all three and a side pot for Bo and Cy.
fn three_way(turn_ms: u64) -> TableHost {
    let ids = ["a", "b", "c"];
    let seats = (0..3).map(|_| TableSeat::empty()).collect();
    let mut table = Table::new(TableRules::no_limit_holdem(1, 2), seats, 40, 7).unwrap();
    table.set_run_it_twice(true);
    let mut h = TableHost::without_host(table, turn_ms, 40, 200).unwrap();
    for (seat, (c, chips)) in ids.iter().zip([50, 100, 100]).enumerate() {
        h.handle(
            c,
            Command::Join {
                name: c.to_string(),
            },
            0,
        );
        h.handle(c, Command::RequestChips { amount: chips }, 0);
        h.approve_chips(seat, 0);
    }
    h.new_hand(1).unwrap();
    while h.awaiting().is_none() {
        let seat = h.table().to_act().unwrap();
        let legal = h.table().view(seat).legal.unwrap();
        let kind = if legal.raise.is_some() || legal.bet.is_some() {
            "allin"
        } else {
            "call"
        };
        let seq = h.seq();
        h.handle(
            ids[seat],
            Command::Act {
                seq,
                kind: kind.to_string(),
                amount: 0,
            },
            2,
        );
    }
    assert!(matches!(h.awaiting(), Some(Awaiting::RunChoice(_))));
    assert_eq!(h.table().run_pots(), vec![vec![0, 1, 2], vec![1, 2]]);
    h
}

fn pot_runs_of(h: &TableHost) -> Vec<u8> {
    h.table().hand().unwrap().pot_runs().to_vec()
}

#[test]
fn the_short_stack_says_no_and_the_big_stacks_run_their_side_pot_twice() {
    let mut h = three_way(10_000);
    let out = h.handle("a", Command::RunTwice { yes: false }, 3);
    assert!(h.table().in_hand(), "the side pot can still run twice");
    assert_eq!(my_answer(&out, "a"), Some(false), "Ann waits for the others");
    h.handle("b", Command::RunTwice { yes: true }, 3);
    assert!(h.table().in_hand(), "waiting for Cy");
    h.handle("c", Command::RunTwice { yes: true }, 3);
    assert!(!h.table().in_hand());
    assert_eq!(pot_runs_of(&h), vec![1, 2]);
    assert_eq!(runs_of(&h), 2);
    let view = h.table().view(0);
    assert_eq!(view.pot_runs, vec![1, 2]);
    assert_eq!(view.second_board.len(), 5);
    let runs: Vec<u8> = view.pots.iter().map(|p| p.runs).collect();
    assert_eq!(runs, vec![1, 2]);
    // The main pot is won whole, on the first board.
    let main = &view.pots[0];
    assert_eq!(
        main.awards.iter().map(|a| a.amount).sum::<u64>(),
        main.amount
    );
}

#[test]
fn a_no_in_every_pot_settles_at_once() {
    // Bo is in both pots: his no runs everything once, without waiting.
    let mut h = three_way(10_000);
    h.handle("b", Command::RunTwice { yes: false }, 3);
    assert!(!h.table().in_hand());
    assert_eq!(pot_runs_of(&h), vec![1, 1]);
    assert_eq!(runs_of(&h), 1);
    assert!(h.table().view(0).second_board.is_empty());

    // Ann's no and then Cy's: the main pot and the side pot each have one.
    let mut h = three_way(10_000);
    h.handle("a", Command::RunTwice { yes: false }, 3);
    assert!(h.table().in_hand());
    h.handle("c", Command::RunTwice { yes: false }, 3);
    assert!(!h.table().in_hand(), "no need to wait for Bo");
    assert_eq!(pot_runs_of(&h), vec![1, 1]);
}

#[test]
fn everyone_yes_runs_every_pot_twice() {
    let mut h = three_way(10_000);
    for c in ["a", "b", "c"] {
        h.handle(c, Command::RunTwice { yes: true }, 3);
    }
    assert_eq!(pot_runs_of(&h), vec![2, 2]);
}

#[test]
fn out_of_time_a_missing_answer_is_a_no_for_that_players_pots() {
    // Bo and Cy say yes, Ann never answers: her main pot runs once.
    let mut h = three_way(10_000);
    h.handle("b", Command::RunTwice { yes: true }, 3);
    h.handle("c", Command::RunTwice { yes: true }, 3);
    assert!(h.table().in_hand(), "waiting for Ann");
    h.tick(3 + 10_000);
    assert!(!h.table().in_hand());
    assert_eq!(pot_runs_of(&h), vec![1, 2]);

    // Only Ann answers: Bo and Cy's silence runs everything once.
    let mut h = three_way(10_000);
    h.handle("a", Command::RunTwice { yes: true }, 3);
    h.tick(3 + 10_000);
    assert_eq!(pot_runs_of(&h), vec![1, 1]);
    assert_eq!(runs_of(&h), 1);
}

#[test]
fn someone_away_is_a_no_for_their_pots() {
    let mut h = three_way(0);
    h.handle("b", Command::RunTwice { yes: true }, 3);
    h.handle("c", Command::RunTwice { yes: true }, 3);
    h.disconnected("a", 4);
    assert!(!h.table().in_hand());
    assert_eq!(pot_runs_of(&h), vec![1, 2], "Ann left: her pot runs once");
}

#[test]
fn bots_go_along_with_the_person() {
    // The host against a bot, all-in: the host's yes is enough.
    let seats = vec![
        TableSeat::human("Host", "you"),
        ducy_bots::Personality::ALL[0]
            .bot(Some(1))
            .seat(ducy_bots::Personality::ALL[0].id()),
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
