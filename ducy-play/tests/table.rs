use ducy_play::{
    Command, Event, Outgoing, Personality, Table, TableHost, TableRules, TableSeat, TableView,
    Update,
};
use rand::{RngExt, SeedableRng, rngs::StdRng};

fn bot_seat(p: Personality, seed: u64) -> TableSeat {
    TableSeat::bot(p.id(), p.bot(Some(seed)))
}

/// Host in seat 0, then three bots; seats 1 and 2 are open.
fn host(turn_ms: u64) -> TableHost {
    let seats = vec![
        TableSeat::human("Host", "you"),
        bot_seat(Personality::ALL[0], 1),
        bot_seat(Personality::ALL[1], 2),
        bot_seat(Personality::ALL[2], 3),
    ];
    let table = Table::new(TableRules::no_limit_holdem(1, 2), seats, 200, 42).unwrap();
    TableHost::new(table, vec![false, true, true, false], turn_ms).unwrap()
}

fn join(name: &str) -> Command {
    Command::Join {
        name: name.to_string(),
    }
}

fn act(seq: u64, kind: &str) -> Command {
    Command::Act {
        seq,
        kind: kind.to_string(),
        amount: 0,
    }
}

/// The last state sent to `client`.
fn last_view(out: &[Outgoing], client: &str) -> Option<(u64, TableView)> {
    out.iter().rev().find_map(|o| match &o.update {
        Update::State { seq, view, .. } if o.to == client => Some((*seq, (**view).clone())),
        _ => None,
    })
}

fn rejected(out: &[Outgoing], client: &str) -> bool {
    out.iter()
        .any(|o| o.to == client && matches!(o.update, Update::Rejected { .. }))
}

/// Plays bots and the host (check/call) until a remote person must act or
/// the hand ends.
fn run_to_person(h: &mut TableHost, now: u64) -> Vec<Outgoing> {
    let mut out = Vec::new();
    loop {
        if h.auto_to_act() {
            out.extend(h.advance(now).unwrap());
        } else if h.table().to_act() == Some(0) {
            let legal = h.host_view().legal.unwrap();
            let kind = if legal.can_check { "check" } else { "call" };
            out.extend(h.host_act(kind, 0, now).unwrap());
        } else {
            return out;
        }
    }
}

#[test]
fn view_puts_the_viewer_in_seat_0_and_hides_other_cards() {
    let seats = vec![
        TableSeat::human("A", "a"),
        TableSeat::human("B", "b"),
        TableSeat::human("C", "c"),
    ];
    let mut t = Table::new(TableRules::no_limit_holdem(1, 2), seats, 100, 7).unwrap();
    t.new_hand().unwrap(); // button on seat 0: seat 1 small blind, seat 2 big blind
    let v = t.view(2);
    assert_eq!((v.hero, v.seat), (0, 2));
    assert_eq!(v.seats[0].name, "C");
    assert_eq!(v.seats[1].name, "A");
    assert!(v.seats[0].cards.is_some());
    assert!(v.seats[1].cards.is_none() && v.seats[2].cards.is_none());
    // Seat 0 (the button) is seat 1 in C's view, and C's own big blind is seat 0.
    assert_eq!(v.button, 1);
    assert!(v.events.contains(&Event::BigBlind { seat: 0, amount: 2 }));
    assert!(v.events.contains(&Event::SmallBlind { seat: 2, amount: 1 }));
    // It's the button's turn: seat 1 in C's view, and C has no legal actions.
    assert_eq!(v.to_act, Some(1));
    assert!(v.legal.is_none());
    let a = t.view(0);
    assert_eq!(a.to_act, Some(0));
    assert_eq!(a.legal.unwrap().seat, 0);
}

#[test]
fn players_join_open_seats_and_play() {
    let mut h = host(0);
    let out = h.handle("c1", join("Alice"), 0);
    assert!(
        out.iter()
            .any(|o| o.to == "c1" && o.update == Update::Welcome { seat: 1 })
    );
    let (_, v) = last_view(&out, "c1").unwrap();
    assert_eq!(v.seats[0].name, "Alice");
    assert!(v.seats[0].human);
    assert_eq!(h.open_seats(), 1);

    h.new_hand(0).unwrap();
    let out = run_to_person(&mut h, 0);
    if h.table().in_hand() {
        assert_eq!(h.table().to_act(), Some(1));
        let (seq, v) = last_view(&out, "c1").unwrap();
        assert!(v.legal.is_some());
        let legal = v.legal.unwrap();
        let kind = if legal.can_check { "check" } else { "call" };
        // A stale seq is refused, the right one is played.
        assert!(rejected(&h.handle("c1", act(seq - 1, kind), 0), "c1"));
        let out = h.handle("c1", act(seq, kind), 0);
        assert!(!rejected(&out, "c1"));
        // Replaying the same action is refused too.
        assert!(rejected(&h.handle("c1", act(seq, kind), 0), "c1"));
    }
}

#[test]
fn table_full_and_unknown_clients_are_refused() {
    let mut h = host(0);
    h.handle("c1", join("A"), 0);
    h.handle("c2", join("B"), 0);
    assert!(rejected(&h.handle("c3", join("C"), 0), "c3"));
    assert!(rejected(&h.handle("c9", act(1, "fold"), 0), "c9"));
}

#[test]
fn joining_mid_hand_waits_for_the_next_hand() {
    let mut h = host(0);
    h.new_hand(0).unwrap();
    let out = h.handle("c1", join("Late"), 0);
    let (_, v) = last_view(&out, "c1").unwrap();
    // Still the bot's seat this hand, and no cards for the newcomer.
    assert!(!v.seats[0].human);
    assert!(v.seats[0].cards.is_none());
    assert!(v.legal.is_none());
    while h.table().in_hand() {
        if h.auto_to_act() {
            h.advance(0).unwrap();
        } else {
            let legal = h.host_view().legal.unwrap();
            h.host_act(if legal.can_check { "check" } else { "call" }, 0, 0)
                .unwrap();
        }
    }
    let out = h.new_hand(0).unwrap();
    let (_, v) = last_view(&out, "c1").unwrap();
    assert!(v.seats[0].human);
    assert_eq!(v.seats[0].name, "Late");
}

#[test]
fn timeouts_check_or_fold_then_sit_the_player_out() {
    let mut h = host(1000);
    h.handle("c1", join("Slow"), 0);
    let mut now = 0;
    let mut timeouts = 0;
    while timeouts < 2 {
        if !h.table().in_hand() {
            h.new_hand(now).unwrap();
        }
        run_to_person(&mut h, now);
        if h.table().to_act() == Some(1) {
            assert_eq!(h.turn_ms_left(now), Some(1000));
            assert!(h.tick(now + 999).is_empty());
            now += 1000;
            assert!(!h.tick(now).is_empty());
            timeouts += 1;
        }
    }
    // Away now: the seat plays itself without waiting.
    assert!(h.table().seat(1).away);
    h.handle("c1", Command::SitIn, now);
    assert!(!h.table().seat(1).away);
}

#[test]
fn disconnect_reconnect_and_leave() {
    let mut h = host(0);
    h.handle("c1", join("Bob"), 0);
    h.disconnected("c1", 0);
    assert!(h.table().seat(1).away);
    let out = h.handle("c1", join("ignored"), 0);
    assert!(
        out.iter()
            .any(|o| o.to == "c1" && o.update == Update::Welcome { seat: 1 })
    );
    assert!(!h.table().seat(1).away);
    assert_eq!(h.table().seat(1).name, "Bob");

    // A new connection with the same name takes over a disconnected seat.
    h.disconnected("c1", 0);
    let out = h.handle("c1b", join("bob"), 0);
    assert!(
        out.iter()
            .any(|o| o.to == "c1b" && o.update == Update::Welcome { seat: 1 })
    );
    assert!(rejected(&h.handle("c1", act(h.seq(), "check"), 0), "c1"));
    let c1 = "c1b";

    h.handle(c1, Command::Leave, 0);
    assert!(!h.table().seat(1).human);
    assert_eq!(h.table().seat(1).id, Personality::ALL[0].id());
    assert_eq!(h.open_seats(), 2);
}

/// Many hands with two remote players acting at random, sometimes late,
/// sometimes twice, sometimes out of turn, dropping and coming back.
#[test]
fn random_play_keeps_cards_private_and_chips_conserved() {
    let mut h = host(500);
    let mut rng = StdRng::seed_from_u64(9);
    let clients = ["c1", "c2"];
    for c in clients {
        h.handle(c, join(c), 0);
    }
    let mut seen: Vec<Option<(u64, TableView)>> = vec![None, None];
    let mut now = 0u64;
    let mut hands = 0;
    let mut played = 0;
    let check = |out: &[Outgoing], seen: &mut Vec<Option<(u64, TableView)>>| {
        for (i, c) in clients.iter().enumerate() {
            if let Some((seq, v)) = last_view(out, c) {
                // Only your own cards, or cards shown down by players still in.
                for (k, s) in v.seats.iter().enumerate() {
                    if k != 0 && s.cards.is_some() {
                        assert!(v.complete && v.showdown && !s.folded);
                    }
                }
                seen[i] = Some((seq, v));
            }
        }
    };
    for _ in 0..20_000 {
        now += rng.random_range(0..200);
        if !h.table().in_hand() {
            if hands == 300 {
                break;
            }
            if let Some(r) = h.table().hand().and_then(|h| h.result()) {
                assert_eq!(r.net.iter().sum::<i64>(), 0, "chips not conserved");
            }
            let out = h.new_hand(now).unwrap();
            check(&out, &mut seen);
            hands += 1;
            continue;
        }
        let out = match rng.random_range(0..10) {
            0..=2 => h.advance(now).unwrap(),
            3 => h.tick(now),
            4 if h.table().to_act() == Some(0) => {
                let l = h.host_view().legal.unwrap();
                h.host_act(if l.can_check { "check" } else { "call" }, 0, now)
                    .unwrap()
            }
            5 => {
                let i = rng.random_range(0..2);
                if rng.random_range(0..4) == 0 {
                    h.disconnected(clients[i], now)
                } else {
                    h.handle(clients[i], join("back"), now)
                }
            }
            _ => {
                let i = rng.random_range(0..2);
                let Some((seq, v)) = seen[i].clone() else {
                    continue;
                };
                let kind = match v.legal {
                    Some(l) if l.can_check => ["check", "bet", "allin"][rng.random_range(0..3)],
                    Some(_) => ["fold", "call", "raise", "allin"][rng.random_range(0..4)],
                    None => "check",
                };
                let amount = v
                    .legal
                    .and_then(|l| l.bet.or(l.raise))
                    .map_or(0, |r| rng.random_range(r.min_to..=r.max_to));
                // Sometimes act on an old state on purpose.
                let seq = if rng.random_range(0..5) == 0 {
                    seq.saturating_sub(1)
                } else {
                    seq
                };
                let out = h.handle(
                    clients[i],
                    Command::Act {
                        seq,
                        kind: kind.to_string(),
                        amount,
                    },
                    now,
                );
                played += usize::from(!rejected(&out, clients[i]));
                out
            }
        };
        check(&out, &mut seen);
    }
    assert!(hands >= 100, "only {hands} hands played");
    assert!(played >= 200, "remote players only acted {played} times");
}

#[test]
fn any_bot_can_take_a_seat() {
    let seats = vec![
        TableSeat::human("You", "you"),
        TableSeat::with_bot(
            "Rando",
            "random",
            Box::new(ducy_play::bots::RandomBot::new(Some(1))),
        ),
    ];
    let mut t = Table::new(TableRules::no_limit_holdem(1, 2), seats, 200, 3).unwrap();
    t.new_hand().unwrap();
    assert_eq!(t.view(0).seats[1].name, "Rando");
    // The bot plays whenever it's its turn; the person checks or calls.
    while t.in_hand() {
        if t.auto_to_act() {
            t.advance().unwrap();
        } else {
            t.act_default(0).unwrap();
        }
    }
}

#[test]
fn reset_stacks_last_into_the_next_hand() {
    let seats = vec![
        TableSeat::human("You", "you"),
        TableSeat::with_bot(
            "Rando",
            "random",
            Box::new(ducy_play::bots::RandomBot::new(Some(1))),
        ),
    ];
    let mut t = Table::new(TableRules::no_limit_holdem(1, 2), seats, 200, 3).unwrap();
    // Play hands until the stacks have moved.
    for _ in 0..50 {
        t.new_hand().unwrap();
        while t.in_hand() {
            if t.auto_to_act() {
                t.advance().unwrap();
            } else {
                t.act_default(0).unwrap();
            }
        }
        if t.stack(0) != 200 {
            break;
        }
    }
    assert_ne!(t.stack(0), 200, "the stacks never moved");
    t.reset_stack(0).unwrap();
    t.reset_stack(1).unwrap();
    assert_eq!((t.stack(0), t.stack(1)), (200, 200));
    t.new_hand().unwrap();
    let v = t.view(0);
    let total = |i: usize| v.seats[i].stack + v.seats[i].street_bet;
    assert_eq!(
        total(0) + total(1),
        400,
        "both start the hand with the buy-in"
    );
    assert_eq!((total(0), total(1)), (200, 200));
}

/// A table for friends: the host and five empty seats anyone may take,
/// blinds 1/2, buy-ins from 40 to 200 chips (20 to 100 big blinds).
fn friends(turn_ms: u64) -> TableHost {
    let mut seats = vec![TableSeat::human("Host", "you")];
    seats.extend((0..5).map(|_| TableSeat::empty()));
    let table = Table::new(TableRules::no_limit_holdem(1, 2), seats, 200, 7).unwrap();
    TableHost::new(table, vec![false, true, true, true, true, true], turn_ms)
        .unwrap()
        .with_bank(40, 200)
        .unwrap()
}

fn request(amount: u64) -> Command {
    Command::RequestChips { amount }
}

/// The chips view last sent to `client`.
fn last_chips(out: &[Outgoing], client: &str) -> Option<ducy_play::ChipsView> {
    out.iter().rev().find_map(|o| match &o.update {
        Update::State { chips, .. } if o.to == client => chips.clone(),
        _ => None,
    })
}

/// Plays the current hand out: the host and every remote person check or
/// call (a person who is away is played by the table).
fn play_out(h: &mut TableHost, players: &[&str], now: u64) {
    while h.table().in_hand() {
        let out = run_to_person(h, now);
        if !h.table().in_hand() {
            break;
        }
        let seat = h.table().to_act().unwrap();
        let client = players[seat - 1];
        let (seq, view) =
            last_view(&out, client).unwrap_or_else(|| (h.seq(), h.table().view(seat)));
        let legal = view.legal.expect("their turn");
        let kind = if legal.can_check { "check" } else { "call" };
        let r = h.handle(client, act(seq.max(h.seq()), kind), now);
        assert!(!rejected(&r, client), "{client} {kind}");
    }
}

#[test]
fn friends_sit_down_with_no_chips_and_ask_the_host() {
    let mut h = friends(0);
    // Only the host: nothing to deal, and the empty seats show as empty.
    assert!(h.new_hand(0).is_err());
    let v = h.host_view();
    assert_eq!(v.seats.len(), 6);
    assert!(v.seats[1..].iter().all(|s| s.empty));
    // A friend sits down with no chips and isn't dealt in.
    let out = h.handle("a", join("Ann"), 1);
    assert!(!rejected(&out, "a"));
    assert_eq!(h.table().stack(1), 0);
    assert!(h.new_hand(2).is_err(), "Ann has no chips yet");
    // Too much or too little is refused; a fair request waits for the host.
    assert!(rejected(&h.handle("a", request(300), 3), "a"));
    assert!(rejected(&h.handle("a", request(10), 3), "a"));
    let out = h.handle("a", request(150), 4);
    assert!(!rejected(&out, "a"));
    let chips = last_chips(&out, "a").unwrap();
    assert_eq!((chips.min, chips.max, chips.requested), (40, 200, 150));
    let reqs = h.chip_requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(
        (
            reqs[0].seat,
            reqs[0].name.as_str(),
            reqs[0].amount,
            reqs[0].stack
        ),
        (1, "Ann", 150, 0)
    );
    // Denied, then asked again and approved.
    h.deny_chips(1, 5);
    assert!(h.chip_requests().is_empty());
    assert_eq!(h.table().stack(1), 0);
    h.handle("a", request(150), 6);
    let out = h.approve_chips(1, 7);
    assert_eq!(last_chips(&out, "a").unwrap().requested, 0);
    assert_eq!(h.table().stack(1), 150);
    // Now the two of them play; the other seats stay empty.
    h.new_hand(8).unwrap();
    assert_eq!(h.table().dealt(), &[0, 1]);
    let v = h.host_view();
    assert!(v.seats[2..].iter().all(|s| s.empty && s.cards.is_none()));
    play_out(&mut h, &["a"], 9);
    // Chips carry over: no one is topped back up.
    let (s0, s1) = (h.table().stack(0), h.table().stack(1));
    assert_eq!(s0 + s1, 350);
    h.new_hand(10).unwrap();
    let v = h.host_view();
    let total = |i: usize| v.seats[i].stack + v.seats[i].street_bet;
    assert_eq!((total(0), total(1)), (s0, s1));
}

#[test]
fn chips_approved_mid_hand_arrive_before_the_next_one() {
    let mut h = friends(0);
    h.handle("a", join("Ann"), 0);
    h.handle("a", request(100), 0);
    h.approve_chips(1, 0);
    h.new_hand(1).unwrap();
    // Ann tops up during the hand: it waits for the next deal.
    h.handle("a", request(40), 2);
    let out = h.approve_chips(1, 3);
    assert_eq!(last_chips(&out, "a").unwrap().approved, 40);
    play_out(&mut h, &["a"], 4);
    let after_hand = h.table().stack(1);
    h.new_hand(5).unwrap();
    let v = h.table().view(1);
    assert_eq!(v.seats[0].stack + v.seats[0].street_bet, after_hand + 40);
    // The host tops up too, within the limits.
    assert!(h.host_chips(1000, 6).is_err());
}

#[test]
fn leaving_and_removal_free_the_seat() {
    let mut h = friends(0);
    for (c, n) in [("a", "Ann"), ("b", "Bo"), ("c", "Cy")] {
        h.handle(c, join(n), 0);
        h.handle(c, request(100), 0);
    }
    for seat in 1..=3 {
        h.approve_chips(seat, 0);
    }
    h.new_hand(1).unwrap();
    assert_eq!(h.table().dealt(), &[0, 1, 2, 3]);
    // Bo leaves mid-hand, and the host removes Cy: they fold when it's
    // their turn, and their seats are empty from the next hand.
    let out = h.handle("b", Command::Leave, 2);
    assert!(out.iter().all(|o| o.to != "b"), "no more updates for Bo");
    h.remove(3, 3);
    play_out(&mut h, &["a", "b", "c"], 4);
    h.new_hand(5).unwrap();
    assert_eq!(h.table().dealt(), &[0, 1]);
    let v = h.host_view();
    assert!(v.seats[2].empty && v.seats[3].empty);
    assert_eq!((h.table().stack(2), h.table().stack(3)), (0, 0));
    // Someone new takes the first free seat, with no chips.
    play_out(&mut h, &["a"], 6);
    h.handle("d", join("Di"), 7);
    assert_eq!(h.table().stack(2), 0);
    assert!(h.table().view(0).seats[2].human);
}

#[test]
fn a_player_who_stays_away_sits_out_then_is_dropped() {
    let mut h = friends(0);
    for (c, n) in [("a", "Ann"), ("b", "Bo")] {
        h.handle(c, join(n), 0);
        h.handle(c, request(200), 0);
    }
    h.approve_chips(1, 0);
    h.approve_chips(2, 0);
    // Bo's connection drops: from the next hand Bo sits out.
    h.disconnected("b", 1);
    for hand in 0..ducy_play::DROP_AFTER_HANDS {
        h.new_hand(2 + hand as u64).unwrap();
        assert_eq!(h.table().dealt(), &[0, 1], "hand {hand}");
        let v = h.host_view();
        assert!(v.seats[2].sitting_out && !v.seats[2].empty);
        play_out(&mut h, &["a", "b"], 2);
    }
    // After that many hands away, the seat is given up.
    h.new_hand(10).unwrap();
    assert!(h.host_view().seats[2].empty);
    // Ann comes and goes in time: reconnecting keeps the seat.
    play_out(&mut h, &["a", "b"], 11);
    h.disconnected("a", 12);
    h.handle("a", join("Ann"), 13);
    assert!(h.new_hand(14).is_ok());
    assert_eq!(h.table().dealt(), &[0, 1]);
}
