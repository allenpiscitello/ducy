//! A club table dealt by a trustless shuffle (#139): the host never sees a
//! hole card. It waits for the board street by street and for hands at
//! showdown, forfeits whoever can't or won't show, and can be saved and
//! restored mid-hand.

use ducy::deck::{Card, Deck};
use ducy_play::{Awaiting, Command, Event, HostSnapshot, Table, TableHost, TableRules, TableSeat};

fn card(s: &str) -> Card {
    Card::parse(s).unwrap()
}

fn hand(cards: &[&str]) -> Deck {
    let mut d = Deck::empty();
    for c in cards {
        d |= card(c);
    }
    d
}

/// A club table with Ann, Bo and Cy seated with 100 chips each.
fn club() -> TableHost {
    let seats = (0..4).map(|_| TableSeat::empty()).collect();
    let table = Table::new(TableRules::no_limit_holdem(1, 2), seats, 40, 3).unwrap();
    let mut h = TableHost::without_host(table, 10_000, 40, 200).unwrap();
    for (c, name, seat) in [("a", "Ann", 0), ("b", "Bo", 1), ("c", "Cy", 2)] {
        h.handle(
            c,
            Command::Join {
                name: name.to_string(),
            },
            0,
        );
        h.handle(c, Command::RequestChips { amount: 100 }, 0);
        h.approve_chips(seat, 0);
    }
    h
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// Everyone checks or calls until the hand waits for something or ends.
fn call_down(h: &mut TableHost, players: &[&str], now: u64) {
    while let Some(seat) = h.table().to_act() {
        let legal = h.table().view(seat).legal.expect("their turn");
        let kind = if legal.can_check { "check" } else { "call" };
        let seq = h.seq();
        h.handle(
            players[seat],
            Command::Act {
                seq,
                kind: kind.to_string(),
                amount: 0,
            },
            now,
        );
    }
}

const BOARD: [&str; 5] = ["2c", "7d", "9h", "Js", "Kd"];

/// Calls down to showdown, dealing each street when it's waited for.
fn to_showdown(h: &mut TableHost, players: &[&str], now: u64) {
    let mut dealt = 0;
    loop {
        call_down(h, players, now);
        match h.awaiting() {
            Some(Awaiting::Board(street)) => {
                let n = street.board_cards() - dealt;
                let cards: Vec<Card> = BOARD[dealt..dealt + n].iter().map(|c| card(c)).collect();
                h.deal_board(&cards, now).unwrap();
                dealt += n;
            }
            _ => return,
        }
    }
}

fn host_sees_no_hole_cards(h: &TableHost) {
    for s in &h.host_view().seats {
        assert!(s.cards.is_none(), "the host sees {s:?}");
    }
}

#[test]
fn a_hidden_hand_waits_for_the_board_and_for_hands_at_showdown() {
    let mut h = club();
    h.new_hand_hidden(1, &ids(&["a", "b", "c"])).unwrap();
    assert_eq!(h.table().dealt(), &[0, 1, 2]);
    host_sees_no_hole_cards(&h);
    // Nobody's own view has cards from the engine: they open their own.
    assert!(h.table().view(0).seats[0].cards.is_none());

    to_showdown(&mut h, &["a", "b", "c"], 2);
    host_sees_no_hole_cards(&h);
    let Some(Awaiting::Reveals(seats)) = h.awaiting() else {
        panic!("waits for hands at showdown");
    };
    assert_eq!(seats, vec![0, 1, 2]);
    assert!(h.turn_ms_left(2).is_some(), "showing is on the clock");

    // Ann and Bo show; Cy runs out of time and can't win.
    h.reveal(0, hand(&["As", "Ah"]), 3).unwrap();
    h.reveal(1, hand(&["Qs", "Qh"]), 3).unwrap();
    assert!(h.table().in_hand());
    h.tick(3 + 10_000);
    assert!(!h.table().in_hand());
    let events = h.table().hand().unwrap().events().to_vec();
    assert!(events.contains(&Event::Forfeit { seat: 2 }));
    // Aces win: Ann gets everyone's 2 × 3.
    assert_eq!(h.table().stack(0), 104);
}

#[test]
fn only_players_the_deck_was_made_for_are_dealt_in() {
    let mut h = club();
    h.new_hand_hidden(1, &ids(&["a", "c"])).unwrap();
    assert_eq!(h.table().dealt(), &[0, 2], "Bo sits this one out");
    assert!(
        h.new_hand_hidden(2, &ids(&["a", "b"])).is_err(),
        "a hand is on"
    );
}

#[test]
fn someone_gone_at_showdown_forfeits_at_once() {
    let mut h = club();
    h.new_hand_hidden(1, &ids(&["a", "b", "c"])).unwrap();
    to_showdown(&mut h, &["a", "b", "c"], 2);
    h.disconnected("c", 3);
    h.reveal(0, hand(&["As", "Ah"]), 3).unwrap();
    h.reveal(1, hand(&["Qs", "Qh"]), 3).unwrap();
    assert!(!h.table().in_hand(), "no waiting for Cy");

    // Gone while the river is dealt: forfeits as soon as showdown comes.
    h.new_hand_hidden(10, &ids(&["a", "b"])).unwrap();
    let mut dealt = 0;
    while dealt < 4 {
        call_down(&mut h, &["a", "b", "c"], 10);
        let Some(Awaiting::Board(street)) = h.awaiting() else {
            panic!("waits for the board");
        };
        let n = street.board_cards() - dealt;
        let cards: Vec<Card> = BOARD[dealt..dealt + n].iter().map(|c| card(c)).collect();
        h.deal_board(&cards, 10).unwrap();
        dealt += n;
    }
    call_down(&mut h, &["a", "b", "c"], 10);
    h.disconnected("b", 11);
    h.deal_board(&[card(BOARD[4])], 11).unwrap();
    // Bo is away: the hand checks for him, and showdown waits only for Ann.
    while h.auto_to_act() || h.table().to_act().is_some() {
        if h.auto_to_act() {
            h.advance(12).unwrap();
        } else {
            call_down(&mut h, &["a", "b", "c"], 12);
        }
    }
    // With Bo out, Ann is the last one in: she wins without showing.
    assert!(!h.table().in_hand());
    let events = h.table().hand().unwrap().events().to_vec();
    assert!(events.contains(&Event::Forfeit { seat: 1 }));
    assert!(!events.iter().any(|e| matches!(e, Event::Reveal { .. })));
    assert!(h.table().stack(0) > h.table().stack(1));
}

#[test]
fn a_hidden_card_already_seen_is_refused() {
    let mut h = club();
    h.new_hand_hidden(1, &ids(&["a", "b", "c"])).unwrap();
    to_showdown(&mut h, &["a", "b", "c"], 2);
    // A board card, shown as a hole card: refused.
    assert!(h.reveal(0, hand(&["2c", "Ah"]), 3).is_err());
    h.reveal(0, hand(&["As", "Ah"]), 3).unwrap();
    assert!(h.reveal(1, hand(&["As", "Qh"]), 3).is_err(), "Ann's ace");
}

#[test]
fn a_table_with_bots_or_a_host_seat_cant_deal_hidden() {
    let seats = vec![TableSeat::human("Host", "you"), TableSeat::empty()];
    let table = Table::new(TableRules::no_limit_holdem(1, 2), seats, 200, 1).unwrap();
    let mut h = TableHost::new(table, vec![false, true], 0).unwrap();
    assert!(h.new_hand_hidden(0, &ids(&["you"])).is_err());
}

#[test]
fn a_hidden_hand_is_saved_and_restored_mid_hand() {
    let mut h = club();
    h.new_hand_hidden(1, &ids(&["a", "b", "c"])).unwrap();
    // To the turn, with the flop dealt.
    call_down(&mut h, &["a", "b", "c"], 2);
    let flop: Vec<Card> = BOARD[..3].iter().map(|c| card(c)).collect();
    h.deal_board(&flop, 2).unwrap();
    call_down(&mut h, &["a", "b", "c"], 2);
    assert!(matches!(h.awaiting(), Some(Awaiting::Board(_))));

    let saved: HostSnapshot = h.snapshot(3);
    let json = serde_json::to_string(&saved).unwrap();
    let back: HostSnapshot = serde_json::from_str(&json).unwrap();
    let mut r = TableHost::restore(&back, 4, |_| None).unwrap();
    assert_eq!(
        r.table().hand().unwrap().events(),
        h.table().hand().unwrap().events()
    );
    assert!(matches!(r.awaiting(), Some(Awaiting::Board(_))));
    host_sees_no_hole_cards(&r);

    // Paused until everyone's back: no one forfeits meanwhile.
    for (c, name) in [("a", "Ann"), ("b", "Bo"), ("c", "Cy")] {
        r.handle(
            c,
            Command::Join {
                name: name.to_string(),
            },
            5,
        );
    }
    r.resume(5);
    r.deal_board(&[card(BOARD[3])], 5).unwrap();
    call_down(&mut r, &["a", "b", "c"], 5);
    r.deal_board(&[card(BOARD[4])], 5).unwrap();
    call_down(&mut r, &["a", "b", "c"], 5);
    assert!(matches!(r.awaiting(), Some(Awaiting::Reveals(_))));

    // Saved at showdown, after one hand is shown.
    r.reveal(1, hand(&["Qs", "Qh"]), 6).unwrap();
    let json = serde_json::to_string(&r.snapshot(6)).unwrap();
    let back: HostSnapshot = serde_json::from_str(&json).unwrap();
    let r2 = TableHost::restore(&back, 7, |_| None).unwrap();
    assert_eq!(
        r2.table().hand().unwrap().events(),
        r.table().hand().unwrap().events()
    );
    assert_eq!(r2.awaiting(), Some(Awaiting::Reveals(vec![0, 2])));
}
