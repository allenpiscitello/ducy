//! Saving a table and loading it again: hands resume exactly where they were.

use ducy_play::{
    Action, Command, Deal, Hand, HostSnapshot, Outgoing, PlayError, Table, TableHost, TableRules,
    TableSeat, Update,
};
use rand::{Rng, RngExt, SeedableRng, rngs::StdRng};

/// Through JSON and back, as the browser stores it.
fn round_trip(h: &TableHost, now: u64) -> TableHost {
    let json = serde_json::to_string(&h.snapshot(now)).unwrap();
    let s: HostSnapshot = serde_json::from_str(&json).unwrap();
    TableHost::restore(&s, now, |_| None).unwrap()
}

/// A random legal action, bets and raises included.
fn random_action(rng: &mut impl Rng, hand: &Hand) -> Action {
    let a = pick(rng, hand);
    if hand.clone().act(a).is_ok() {
        return a;
    }
    let legal = hand.legal_actions().unwrap();
    if legal.can_check {
        Action::Check
    } else {
        Action::Call
    }
}

fn pick(rng: &mut impl Rng, hand: &Hand) -> Action {
    let legal = hand.legal_actions().unwrap();
    match rng.random_range(0..6) {
        0 if legal.can_fold => Action::Fold,
        1 => Action::AllIn,
        2 if legal.bet.is_some() => {
            let r = legal.bet.unwrap();
            Action::Bet(rng.random_range(r.min_to..=r.max_to))
        }
        3 if legal.raise.is_some() => {
            let r = legal.raise.unwrap();
            Action::Raise(rng.random_range(r.min_to..=r.max_to))
        }
        _ if legal.can_check => Action::Check,
        _ => Action::Call,
    }
}

#[test]
fn a_hand_restored_at_any_point_plays_on_identically() {
    let mut rng = StdRng::seed_from_u64(5);
    for game in 0..300 {
        let rules = if game % 3 == 0 {
            TableRules::pot_limit_omaha(1, 2)
        } else {
            TableRules::no_limit_holdem(1, 2)
        };
        let n = rng.random_range(2..=6);
        let stacks: Vec<u64> = (0..n).map(|_| rng.random_range(10..300)).collect();
        let deal = Deal::random(rules.variant, n, Some(game)).unwrap();
        let mut hand = Hand::new(rules, &stacks, game as usize % n, deal).unwrap();
        while !hand.is_complete() {
            let restored = Hand::restore(&hand.snapshot()).unwrap();
            assert_eq!(restored.events(), hand.events());
            assert_eq!(restored.legal_actions(), hand.legal_actions());
            assert_eq!(restored.street(), hand.street());
            assert_eq!(restored.pot(), hand.pot());
            assert_eq!(restored.board(), hand.board());
            assert_eq!(restored.deal(), hand.deal());
            // Carry on with the restored hand.
            hand = restored;
            let a = random_action(&mut rng, &hand);
            hand.act(a).unwrap();
        }
        let restored = Hand::restore(&hand.snapshot()).unwrap();
        assert_eq!(restored.result(), hand.result());
    }
}

#[test]
fn a_hand_that_doesnt_replay_to_its_events_is_refused() {
    let rules = TableRules::no_limit_holdem(1, 2);
    let deal = Deal::random(rules.variant, 3, Some(1)).unwrap();
    let mut hand = Hand::new(rules, &[100, 100, 100], 0, deal).unwrap();
    hand.act(Action::Raise(6)).unwrap();
    hand.act(Action::Call).unwrap();
    let mut json = serde_json::to_value(hand.snapshot()).unwrap();
    // Someone changes the raise.
    let events = json["events"].as_array_mut().unwrap();
    let raise = events.iter_mut().find(|e| e["type"] == "raise").unwrap();
    raise["to"] = 8.into();
    let s = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(Hand::restore(&s).err(), Some(PlayError::InvalidSnapshot));
    // Or a card, so one is dealt twice.
    json["events"] = serde_json::to_value(hand.events()).unwrap();
    json["board"][0] = json["hole_cards"][0][0].clone();
    let s = serde_json::from_value(json).unwrap();
    assert_eq!(Hand::restore(&s).err(), Some(PlayError::InvalidSnapshot));
}

/// A club table of four people with chips, seeded so two copies deal alike.
fn club_of_four() -> TableHost {
    let seats = (0..4).map(|_| TableSeat::empty()).collect();
    let table = Table::new(TableRules::no_limit_holdem(1, 2), seats, 40, 9).unwrap();
    let mut h = TableHost::without_host(table, 30_000, 40, 200).unwrap();
    for (i, c) in ["a", "b", "c", "d"].iter().enumerate() {
        h.handle(c, join(c), 0);
        h.handle(
            c,
            Command::RequestChips {
                amount: 100 + 20 * i as u64,
            },
            0,
        );
        h.approve_chips(i, 0);
    }
    h
}

fn join(name: &str) -> Command {
    Command::Join {
        name: name.to_string(),
    }
}

fn client_in(h: &TableHost, seat: usize) -> String {
    h.seated()
        .into_iter()
        .find(|p| p.seat == seat)
        .unwrap()
        .client_id
}

fn rejected(out: &[Outgoing]) -> bool {
    out.iter()
        .any(|o| matches!(o.update, Update::Rejected { .. }))
}

/// Every seat's view, the host's, and who sits where.
fn same(a: &TableHost, b: &TableHost) {
    for s in 0..a.table().num_seats() {
        assert_eq!(a.table().view(s), b.table().view(s), "seat {s}");
    }
    assert_eq!(a.host_view(), b.host_view());
    assert_eq!(a.seated(), b.seated());
    assert_eq!(a.chip_requests(), b.chip_requests());
}

#[test]
fn a_club_table_saved_and_restored_at_random_points_plays_like_one_never_saved() {
    let mut rng = StdRng::seed_from_u64(17);
    // `twin` is never saved; `h` is saved and loaded again now and then.
    let mut twin = club_of_four();
    let mut h = club_of_four();
    let total = 100 + 120 + 140 + 160;
    let mut restores = 0;
    let mut left = 0;
    for n in 1..60u64 {
        let now = n * 1_000;
        if twin.new_hand(now).is_err() {
            break;
        }
        h.new_hand(now).unwrap();
        while twin.table().in_hand() {
            if rng.random_range(0..3) == 0 {
                h = round_trip(&h, now);
                assert!(h.is_paused());
                // Everyone comes back; nothing ran while they were away.
                for p in h.seated() {
                    h.handle(&p.client_id, join(&p.client_id), now);
                }
                h.resume(now);
                restores += 1;
            }
            same(&twin, &h);
            let seat = twin.table().to_act().unwrap();
            let hand = twin.table().hand().unwrap();
            let kind = match random_action(&mut rng, hand) {
                Action::Fold => ("fold", 0),
                Action::Check => ("check", 0),
                Action::Call => ("call", 0),
                Action::Bet(to) => ("bet", to),
                Action::Raise(to) => ("raise", to),
                Action::AllIn => ("allin", 0),
            };
            let client = client_in(&twin, seat);
            for x in [&mut twin, &mut h] {
                let cmd = Command::Act {
                    seq: x.seq(),
                    kind: kind.0.to_string(),
                    amount: kind.1,
                };
                assert!(!rejected(&x.handle(&client, cmd, now)));
            }
        }
        same(&twin, &h);
        // Anyone broke leaves.
        for p in twin.seated() {
            if p.chips == 0 {
                twin.handle(&p.client_id, Command::Leave, now);
                h.handle(&p.client_id, Command::Leave, now);
            }
        }
        let gone = twin.take_departures();
        assert_eq!(gone, h.take_departures());
        left += gone.iter().map(|d| d.chips).sum::<u64>();
        let at_table: u64 = h.seated().iter().map(|p| p.chips).sum();
        assert_eq!(at_table + left, total, "hand {n}");
    }
    assert!(restores > 10, "{restores}");
}

#[test]
fn clocks_go_on_from_what_was_left() {
    let mut h = club_of_four();
    h.set_sit_out_limit(600_000);
    h.new_hand(0).unwrap();
    // Ann sits out with 10 minutes; 4 minutes go by.
    let ann = client_in(&h, 0);
    h.handle(&ann, Command::SitOut, 0);
    // 10 seconds into a 30-second turn, the host's browser closes.
    let s = h.snapshot(10_000);
    // It's reopened an hour later.
    let mut h = TableHost::restore(&s, 3_600_000, |_| None).unwrap();
    assert_eq!(h.turn_ms_left(3_600_000), Some(20_000));
    // Paused: nothing runs out, however long it takes people to come back.
    let to_act = h.table().to_act().unwrap();
    let events = h.table().hand().unwrap().events().len();
    assert!(h.tick(9_000_000).is_empty());
    assert_eq!(h.table().hand().unwrap().events().len(), events);
    for p in h.seated() {
        h.handle(&p.client_id, join(&p.client_id), 9_000_000);
    }
    assert_eq!(h.turn_ms_left(9_000_000), Some(20_000));
    h.resume(10_000_000);
    assert_eq!(h.turn_ms_left(10_000_000), Some(20_000));
    h.tick(10_019_999);
    assert_eq!(h.table().to_act(), Some(to_act), "still their turn");
    h.tick(10_020_000);
    assert_ne!(h.table().to_act(), Some(to_act), "out of time");
    // Ann had 10 minutes less the 10 seconds before the save.
    let me = |h: &TableHost, now| {
        h.updates(now).into_iter().find_map(|o| match o.update {
            Update::State { me, .. } if o.to == ann => me,
            _ => None,
        })
    };
    assert_eq!(me(&h, 10_020_000).unwrap().out_ms_left, Some(570_000));
}

#[test]
fn a_restored_table_waits_for_people_instead_of_folding_them() {
    let mut h = club_of_four();
    h.new_hand(0).unwrap();
    let mut h = round_trip(&h, 5_000);
    // No one is back yet: no one is away, so no one is checked or folded for.
    assert!(!h.auto_to_act());
    assert!(h.updates(5_000).is_empty(), "no one is connected");
    let seat = h.table().to_act().unwrap();
    let client = client_in(&h, seat);
    let out = h.handle(&client, join("whoever"), 6_000);
    assert!(
        out.iter()
            .any(|o| o.to == client && matches!(o.update, Update::Welcome { .. })),
        "they get their seat back"
    );
    h.resume(6_000);
    let out = h.handle(
        &client,
        Command::Act {
            seq: h.seq(),
            kind: "call".into(),
            amount: 0,
        },
        6_000,
    );
    assert!(!rejected(&out));
}

#[test]
fn snapshots_of_another_version_are_refused() {
    let h = club_of_four();
    let mut s = h.snapshot(0);
    s.version += 1;
    assert_eq!(
        TableHost::restore(&s, 0, |_| None).err(),
        Some(PlayError::InvalidSnapshot)
    );
}

#[test]
fn bots_are_asked_for_again_and_a_missing_one_fails_the_load() {
    use ducy_play::Personality;
    let p = Personality::ALL[0];
    let seats = vec![
        TableSeat::human("Host", "you"),
        TableSeat::bot(p.id(), p.bot(Some(1))),
        TableSeat::bot(p.id(), p.bot(Some(2))),
    ];
    let table = Table::new(TableRules::no_limit_holdem(1, 2), seats, 200, 4).unwrap();
    let mut h = TableHost::new(table, vec![false, true, false], 0).unwrap();
    h.new_hand(0).unwrap();
    let s = h.snapshot(0);
    assert_eq!(
        TableHost::restore(&s, 0, |_| None).err(),
        Some(PlayError::InvalidSnapshot)
    );
    let mut asked = Vec::new();
    let r = TableHost::restore(&s, 0, |id| {
        asked.push(id.to_string());
        Personality::from_name(id).map(|p| Box::new(p.bot(None)) as Box<dyn ducy_play::Bot>)
    })
    .unwrap();
    assert_eq!(asked, [p.id(), p.id()]);
    assert_eq!(r.table().view(0), h.table().view(0));
}
