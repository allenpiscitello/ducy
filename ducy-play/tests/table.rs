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
