//! A tournament hosted for people (#142): 12 players over 2 tables play to a
//! winner through their commands, with secure deals. Chips are conserved,
//! no view ever has another player's unshown cards, a moved player gets
//! their new table at once, and a player who never shows up is blinded off.

use ducy_play::{
    BettingStructure, Command, Level, Outgoing, TournamentConfig, TournamentHost, Update, Variant,
};
use rand::{RngExt, SeedableRng, rngs::StdRng};

fn config(seed: u64) -> TournamentConfig {
    TournamentConfig {
        variant: Variant::Holdem,
        structure: BettingStructure::NoLimit,
        table_size: 6,
        starting_stack: 1000,
        levels: vec![
            Level::new(10, 20, 0),
            Level::new(25, 50, 5),
            Level::new(50, 100, 10),
            Level::new(100, 200, 25),
            Level::new(200, 400, 50),
        ],
        paid: 3,
        seed,
    }
}

fn players(n: usize) -> Vec<(String, String)> {
    (0..n).map(|i| (format!("p{i}"), format!("P{i}"))).collect()
}

/// No view sent shows anyone else's cards before they're shown down.
fn private(out: &[Outgoing]) {
    for o in out {
        if let Update::State { view, .. } = &o.update {
            for (i, s) in view.seats.iter().enumerate().skip(1) {
                if s.cards.is_some() {
                    assert!(
                        view.showdown && !s.folded,
                        "{}'s view shows seat {i}'s cards",
                        o.to
                    );
                }
            }
        }
    }
}

#[test]
fn twelve_players_over_two_tables_play_to_a_winner() {
    let mut rng = StdRng::seed_from_u64(9);
    let mut h = TournamentHost::new(config(5), players(12), 30_000).unwrap();
    assert!(h.tournament().secure_deals());
    assert_eq!(h.tournament().table_ids().len(), 2);
    // Everyone but p11 shows up; p11 never does.
    let mut now = 0;
    for i in 0..11 {
        let out = h.handle(
            &format!("p{i}"),
            Command::Join {
                name: String::new(),
            },
            now,
        );
        assert!(matches!(out[0].update, Update::Welcome { .. }));
        private(&out);
    }
    let total = h.tournament().total_chips();
    let mut hands = 0;
    let mut moved = 0;
    let mut last: Vec<Outgoing> = Vec::new();
    while !h.tournament().is_over() {
        now += 100;
        assert!(now < 50_000_000, "the tournament ends");
        let mut out = Vec::new();
        if h.can_deal() {
            // Between hands everywhere: every chip is someone's.
            if h.tournament()
                .table_ids()
                .iter()
                .all(|&t| !h.tournament().table(t).unwrap().in_hand())
            {
                let chips: u64 = h.standings().iter().map(|s| s.stack).sum();
                assert_eq!(chips, total, "chips are conserved");
            }
            out.extend(h.new_hands(now).unwrap());
            hands += 1;
            if hands % 8 == 0 {
                h.set_level(h.tournament().level() + 1);
            }
        } else if h.auto_to_act() {
            out.extend(h.advance(now).unwrap());
        } else {
            // Whoever is to act, from the last view they were sent.
            let mine = last.iter().rev().find_map(|o| match &o.update {
                Update::State { seq, view, .. } if view.legal.is_some() && *seq == h.seq() => {
                    Some((o.to.clone(), *seq, view.legal.unwrap()))
                }
                _ => None,
            });
            let Some((who, seq, legal)) = mine else {
                panic!("no one to act at seq {}", h.seq());
            };
            let kind = match rng.random_range(0..10) {
                0 => "fold",
                1 => "allin",
                _ if legal.can_check => "check",
                _ => "call",
            };
            let kind = if kind == "fold" && legal.can_check {
                "check"
            } else {
                kind
            };
            out.extend(h.handle(
                &who,
                Command::Act {
                    seq,
                    kind: kind.to_string(),
                    amount: 0,
                },
                now,
            ));
        }
        private(&out);
        assert!(out.iter().all(|o| o.to != "p11"), "p11 isn't connected");
        // A player moved to another table is sent a view of it at once.
        let events = h.take_events();
        for m in &events.moves {
            if m.id == "p11" {
                continue;
            }
            moved += 1;
            let sent = out.iter().rev().find_map(|o| match &o.update {
                Update::State { view, .. } if o.to == m.id => Some(view.seat),
                _ => None,
            });
            assert_eq!(sent, Some(m.to.seat), "{} sees their new seat", m.id);
            assert_eq!(
                h.tournament().find(&m.id).map(|s| s.table),
                Some(m.to.table)
            );
        }
        if !out.is_empty() {
            last = out;
        }
    }
    let t = h.tournament();
    assert_eq!(t.finishes().len(), 11);
    assert_eq!(t.winner().unwrap().place, 1);
    let mut places: Vec<usize> = t.finishes().iter().map(|f| f.place).collect();
    places.sort();
    assert_eq!(places, (2..=12).collect::<Vec<_>>());
    // p11 was blinded off: never played a hand, couldn't win.
    assert!(t.finishes().iter().any(|f| f.id == "p11"));
    assert_eq!(t.table_ids().len(), 1, "down to one table");
    assert!(moved > 0, "players were moved");
}

#[test]
fn a_player_out_of_time_twice_is_blinded_off_until_back() {
    let mut h = TournamentHost::new(config(1), players(3), 1000).unwrap();
    for i in 0..3 {
        h.handle(
            &format!("p{i}"),
            Command::Join {
                name: String::new(),
            },
            0,
        );
    }
    let mut now = 0;
    h.new_hands(now).unwrap();
    // Nobody acts; the clock acts for them, and after two timeouts each is absent.
    for _ in 0..40 {
        now += 1001;
        h.tick(now);
        while h.auto_to_act() {
            h.advance(now).unwrap();
        }
        if h.can_deal() {
            h.new_hands(now).unwrap();
        }
    }
    assert!(
        h.standings().iter().all(|s| s.absent),
        "everyone timed out twice"
    );
    // Back: no longer absent.
    h.handle(
        "p0",
        Command::SitIn {
            wait_for_big_blind: false,
        },
        now,
    );
    assert!(!h.standings().iter().find(|s| s.id == "p0").unwrap().absent);
}

#[test]
fn commands_are_checked() {
    let mut h = TournamentHost::new(config(2), players(2), 0).unwrap();
    let rejected = |out: &[Outgoing]| matches!(out[0].update, Update::Rejected { .. });
    assert!(rejected(&h.handle(
        "stranger",
        Command::Join {
            name: String::new()
        },
        0
    )));
    h.handle(
        "p0",
        Command::Join {
            name: String::new(),
        },
        0,
    );
    assert!(rejected(&h.handle(
        "p0",
        Command::RequestChips { amount: 10 },
        0
    )));
    h.new_hands(0).unwrap();
    let stale = Command::Act {
        seq: 0,
        kind: "call".into(),
        amount: 0,
    };
    assert!(rejected(&h.handle("p0", stale, 0)));
    // A late entry is seated, and absent until they join.
    h.add_entry("p9", "P9", 1).unwrap();
    assert!(h.standings().iter().find(|s| s.id == "p9").unwrap().absent);
}
