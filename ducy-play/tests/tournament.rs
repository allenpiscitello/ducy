//! Multi-table tournaments: whole simulated tournaments with bots, checking
//! chips, balance, places, absent players and hand-for-hand play.

use std::collections::HashSet;

use ducy_play::bots::{CallingStation, RandomBot};
use ducy_play::{
    Action, BettingStructure, Bot, Entrant, Event, Level, Observation, PlayError, Table,
    TableRules, TableSeat, Tournament, TournamentConfig, Variant,
};

/// Shoves every hand: raises or bets all in, or calls an all-in.
struct Shover;

impl Bot for Shover {
    fn act(&mut self, obs: &Observation) -> Option<Action> {
        let l = &obs.legal;
        Some(match (l.bet, l.raise) {
            (Some(r), _) => Action::Bet(r.max_to),
            (_, Some(r)) => Action::Raise(r.max_to),
            _ if l.can_check => Action::Check,
            _ => Action::Call,
        })
    }
}

fn levels() -> Vec<Level> {
    [
        (5, 10, 0),
        (10, 20, 0),
        (15, 30, 5),
        (25, 50, 5),
        (50, 100, 10),
        (100, 200, 25),
        (200, 400, 50),
        (400, 800, 100),
        (800, 1600, 200),
    ]
    .iter()
    .map(|&(s, b, a)| Level::new(s, b, a))
    .collect()
}

fn config(table_size: usize, paid: usize, seed: u64) -> TournamentConfig {
    TournamentConfig {
        variant: Variant::Holdem,
        structure: BettingStructure::NoLimit,
        table_size,
        starting_stack: 1000,
        levels: levels(),
        paid,
        seed,
    }
}

fn random_field(n: usize, seed: u64) -> Vec<Entrant> {
    (0..n)
        .map(|i| {
            Entrant::bot(
                format!("p{i}"),
                format!("Player {i}"),
                Box::new(RandomBot::new(Some(seed * 1000 + i as u64))),
            )
        })
        .collect()
}

fn chips(t: &Tournament) -> u64 {
    t.standings().iter().map(|s| s.stack).sum()
}

fn sizes(t: &Tournament) -> Vec<usize> {
    t.table_ids().iter().map(|&id| t.players_at(id)).collect()
}

fn play_out(t: &mut Tournament, id: usize) {
    let table = t.table_mut(id).unwrap();
    while table.advance().unwrap() {}
    assert!(!table.in_hand(), "only bots and absent players here");
}

/// Plays tables in turn, one hand each, raising the level every
/// `hands_per_level` hands dealt in all. Calls `check` after every hand.
fn run(t: &mut Tournament, hands_per_level: u64, mut check: impl FnMut(&Tournament, usize)) -> u64 {
    let mut hands = 0;
    while !t.is_over() {
        let mut dealt_any = false;
        for id in t.table_ids() {
            if !t.can_deal(id) {
                continue;
            }
            t.new_hand(id).unwrap();
            play_out(t, id);
            t.finish_hand(id).unwrap();
            hands += 1;
            dealt_any = true;
            t.set_level((hands / hands_per_level) as usize);
            check(t, id);
            if t.is_over() {
                break;
            }
        }
        assert!(dealt_any || t.is_over(), "stuck with tables {:?}", sizes(t));
        assert!(hands < 20_000, "tournament didn't finish");
    }
    hands
}

#[test]
fn thirty_entrants_on_three_tables_play_to_one_winner() {
    for seed in 1..=4 {
        let mut t = Tournament::new(config(10, 5, seed), random_field(30, seed)).unwrap();
        assert_eq!(sizes(&t), vec![10, 10, 10]);
        let total = t.total_chips();
        assert_eq!(total, 30_000);
        let mut max_tables = 3;
        run(&mut t, 40, |t, _| {
            if !t.is_over() {
                assert_eq!(chips(t), total, "chips conserved");
                let s = sizes(t);
                let (lo, hi) = (*s.iter().min().unwrap(), *s.iter().max().unwrap());
                assert!(hi - lo <= 1, "tables balanced within one: {s:?}");
                assert_eq!(
                    s.len(),
                    t.players_left().div_ceil(10),
                    "no more tables than needed"
                );
                assert!(s.len() <= max_tables, "broken tables stay broken");
                max_tables = s.len();
            }
        });
        let winner = t.winner().unwrap();
        assert_eq!(winner.place, 1);
        let places: HashSet<usize> = t.finishes().iter().map(|f| f.place).collect();
        assert_eq!(places, (2..=30).collect(), "every place 2..30 once");
        let ids: HashSet<&str> = t
            .finishes()
            .iter()
            .map(|f| f.id.as_str())
            .chain([winner.id.as_str()])
            .collect();
        assert_eq!(ids.len(), 30);
        // The places run downward in the order players went out.
        assert!(t.finishes().windows(2).all(|w| w[0].place > w[1].place));
    }
}

#[test]
fn an_absent_player_is_blinded_off_and_never_removed() {
    let mut players = vec![Entrant::person("away", "Away")];
    players.extend((0..5).map(|i| {
        Entrant::bot(
            format!("b{i}"),
            format!("Bot {i}"),
            Box::new(CallingStation),
        )
    }));
    let mut t = Tournament::new(config(6, 0, 3), players).unwrap();
    t.set_absent("away", true).unwrap();
    let mut posted = 0;
    let mut seen_hands = 0;
    while !t.is_over() && t.find("away").is_some() {
        let at = t.find("away").unwrap();
        t.new_hand(at.table).unwrap();
        let table = t.table(at.table).unwrap();
        let i = table
            .hand_index(at.seat)
            .expect("an absent player is still dealt in");
        seen_hands += 1;
        posted += table
            .hand()
            .unwrap()
            .events()
            .iter()
            .filter(|e| {
                matches!(e,
            Event::SmallBlind { seat, .. } | Event::BigBlind { seat, .. } if *seat == i)
            })
            .count();
        // Everyone acts on their own: bots, and the absent player checks or folds.
        play_out(&mut t, at.table);
        let report = t.finish_hand(at.table).unwrap();
        if report.busted.iter().any(|f| f.id == "away") {
            break;
        }
        assert!(
            t.find("away").is_some(),
            "never removed while they have chips"
        );
        t.set_level((seen_hands / 5) as usize);
    }
    assert!(seen_hands >= 3, "played {seen_hands} hands");
    assert!(
        posted >= 2,
        "posted blinds in {posted} of {seen_hands} hands"
    );
}

#[test]
fn players_busting_in_the_same_hand_are_placed_by_starting_stack() {
    // Uneven stacks, everyone all in: when the biggest stack wins, the other
    // two bust together and the one who started with more finishes higher.
    let mut double_busts = 0;
    for seed in 0..30 {
        let players = (0..3)
            .map(|i| Entrant::bot(format!("s{i}"), format!("Shover {i}"), Box::new(Shover)))
            .collect();
        let mut t = Tournament::new(config(3, 0, seed), players).unwrap();
        let id = t.table_ids()[0];
        let table = t.table_mut(id).unwrap();
        for (s, chips) in [2000, 600, 400].into_iter().enumerate() {
            table.set_stack(s, chips).unwrap();
        }
        let (big, mid, small) = (
            table.seat(0).id.clone(),
            table.seat(1).id.clone(),
            table.seat(2).id.clone(),
        );
        t.new_hand(id).unwrap();
        play_out(&mut t, id);
        let report = t.finish_hand(id).unwrap();
        // Places always come out best first.
        assert!(report.busted.windows(2).all(|w| w[0].place < w[1].place));
        if report.busted.len() == 2 {
            double_busts += 1;
            assert_eq!(report.winner.as_ref().unwrap().id, big);
            let place = |who: &str| t.finishes().iter().find(|f| f.id == who).unwrap().place;
            assert_eq!((place(&mid), place(&small)), (2, 3));
        }
    }
    assert!(
        double_busts >= 3,
        "only {double_busts} hands busted two players"
    );
}

#[test]
fn re_entries_get_a_new_stack_at_a_table_and_keep_chips_balanced() {
    let mut t = Tournament::new(config(6, 0, 5), random_field(12, 5)).unwrap();
    let mut reentries = 0;
    let mut hands = 0;
    while !t.is_over() && hands < 400 {
        for id in t.table_ids() {
            if !t.can_deal(id) {
                continue;
            }
            t.new_hand(id).unwrap();
            play_out(&mut t, id);
            let report = t.finish_hand(id).unwrap();
            hands += 1;
            t.set_level((hands / 30) as usize);
            // Re-enter everyone who busts during the first 60 hands.
            if hands < 60 {
                for f in report.busted {
                    let at = t
                        .add_entry(Entrant::bot(
                            f.id.clone(),
                            f.name,
                            Box::new(RandomBot::new(Some(hands))),
                        ))
                        .unwrap();
                    assert_eq!(t.table(at.table).unwrap().stack(at.seat), 1000);
                    reentries += 1;
                }
            }
            if t.is_over() {
                break;
            }
            assert_eq!(chips(&t), t.total_chips());
            let s = sizes(&t);
            assert!(
                s.iter().max().unwrap() - s.iter().min().unwrap() <= 1,
                "{s:?}"
            );
        }
    }
    assert_eq!(t.entries(), 12 + reentries);
    // Someone still in can't enter again.
    if let Some(s) = t.standings().first() {
        assert_eq!(
            t.add_entry(Entrant::person(s.id.clone(), "again")),
            Err(PlayError::InvalidSetup)
        );
    }
}

#[test]
fn hand_for_hand_on_the_bubble_keeps_tables_in_step() {
    // The bubble is skipped when two players bust in one hand, so look for it
    // over several seeds.
    let mut seeds_on_bubble = 0;
    for seed in 1..=10 {
        // Calling stations bust one or two at a time as the blinds rise.
        let field = (0..27)
            .map(|i| {
                Entrant::bot(
                    format!("c{i}"),
                    format!("Caller {i}"),
                    Box::new(CallingStation),
                )
            })
            .collect();
        let mut t = Tournament::new(config(9, 10, seed), field).unwrap();
        // Hands each table dealt since hand-for-hand began.
        let mut counts = std::collections::HashMap::<usize, u64>::new();
        let mut was_on = false;
        run(&mut t, 30, |t, id| {
            if !t.hand_for_hand() {
                was_on = false;
                return;
            }
            if !was_on {
                // It began after this hand; count from the next one.
                counts.clear();
                was_on = true;
                return;
            }
            *counts.entry(id).or_default() += 1;
            assert!(t.players_left() > 10, "hand-for-hand only before the money");
            let playing: Vec<u64> = t
                .table_ids()
                .iter()
                .filter(|&&x| t.table(x).unwrap().players_in() >= 2)
                .map(|x| counts.get(x).copied().unwrap_or(0))
                .collect();
            let (lo, hi) = (playing.iter().min().unwrap(), playing.iter().max().unwrap());
            assert!(hi - lo <= 1, "tables out of step: {playing:?}");
        });
        if was_on || !counts.is_empty() {
            seeds_on_bubble += 1;
        }
        assert!(!t.hand_for_hand(), "off once the tournament is over");
    }
    assert!(
        seeds_on_bubble >= 3,
        "only {seeds_on_bubble} of 10 seeds reached the bubble"
    );
}

#[test]
fn table_rules_change_between_hands_only() {
    let seats = vec![
        TableSeat::with_bot("A", "a", Box::new(CallingStation)),
        TableSeat::with_bot("B", "b", Box::new(CallingStation)),
        TableSeat::with_bot("C", "c", Box::new(CallingStation)),
    ];
    let mut table = Table::new(TableRules::no_limit_holdem(1, 2), seats, 200, 1).unwrap();
    table.set_top_up(false);
    table.new_hand().unwrap();
    let plo = TableRules {
        variant: Variant::Omaha { hole_cards: 4 },
        structure: BettingStructure::PotLimit,
        small_blind: 5,
        big_blind: 10,
        ante: 0,
    };
    assert_eq!(
        table.set_rules(plo),
        Err(PlayError::IllegalAction),
        "not mid-hand"
    );
    while table.advance().unwrap() {}
    table.set_rules(plo).unwrap();
    table.new_hand().unwrap();
    let hand = table.hand().unwrap();
    assert!(hand.deal().hole_cards().iter().all(|h| h.num_cards() == 4));
    assert_eq!(hand.rules().big_blind, 10);
    while table.advance().unwrap() {}
    assert_eq!((0..3).map(|s| table.stack(s)).sum::<u64>(), 600);
    let bad = TableRules {
        small_blind: 0,
        ..plo
    };
    assert_eq!(table.set_rules(bad), Err(PlayError::InvalidBlinds));
}

#[test]
fn every_hand_posts_one_small_and_one_big_blind() {
    let mut t = Tournament::new(config(6, 3, 11), random_field(14, 11)).unwrap();
    run(&mut t, 25, |t, id| {
        let Some(table) = t.table(id) else { return };
        let Some(hand) = table.hand() else { return };
        let ev = hand.events();
        let sb = ev
            .iter()
            .filter(|e| matches!(e, Event::SmallBlind { .. }))
            .count();
        let bb = ev
            .iter()
            .filter(|e| matches!(e, Event::BigBlind { .. }))
            .count();
        assert_eq!((sb, bb), (1, 1));
    });
}

// ---- dead button (#143) ----

/// Six seats, four bots in seats 0–3.
fn four_bots() -> Table {
    let seats = (0..6)
        .map(|i| {
            if i < 4 {
                TableSeat::with_bot(format!("B{i}"), format!("b{i}"), Box::new(CallingStation))
            } else {
                TableSeat::empty()
            }
        })
        .collect();
    Table::new(TableRules::no_limit_holdem(5, 10), seats, 1000, 3).unwrap()
}

#[test]
fn a_newcomer_is_not_dealt_in_on_the_button_or_the_small_blind() {
    // Play hands until seat 4, if someone sat there, would get the button or
    // the small blind next; then a newcomer sits there.
    for target in ["button", "small blind"] {
        let mut t = four_bots();
        loop {
            t.new_hand().unwrap();
            while t.advance().unwrap() {}
            *t.seat_mut(4) = TableSeat::with_bot("New", "new", Box::new(CallingStation));
            let (b, sb, _) = t.next_positions().unwrap();
            if (target == "button" && b == 4) || (target == "small blind" && sb == 4) {
                break;
            }
            *t.seat_mut(4) = TableSeat::empty();
            assert!(t.hand_number() < 50);
        }
        t.arrive(4);
        assert!(t.arriving(4));
        // The next hand deals without the newcomer, and so on until they're
        // neither the button nor the small blind.
        let mut skipped = 0;
        loop {
            t.new_hand().unwrap();
            if let Some(i) = t.hand_index(4) {
                let sb = t.hand().unwrap().events().iter().find_map(|e| match e {
                    Event::SmallBlind { seat, .. } => Some(*seat),
                    _ => None,
                });
                assert_ne!(t.button(), 4, "{target}: not dealt in on the button");
                assert_ne!(sb, Some(i), "{target}: not dealt in on the small blind");
                assert!(!t.arriving(4));
                break;
            }
            skipped += 1;
            while t.advance().unwrap() {}
            assert!(skipped < 6);
        }
        if target == "small blind" {
            assert!(
                skipped >= 1,
                "skipped the hand it would have been the small blind"
            );
        }
    }
}

/// Plays a 30-player tournament and checks, for every player moved to
/// another table, that their first hand there isn't on the button or the
/// small blind unless they also post the big blind.
#[test]
fn moved_players_never_skip_the_big_blind() {
    let mut moved_in = 0;
    let mut checked = 0;
    for seed in 1..=3 {
        let mut t = Tournament::new(config(10, 5, seed), random_field(30, seed)).unwrap();
        // Player id → table they were moved to and haven't played at yet.
        let mut arrived: std::collections::HashMap<String, usize> = Default::default();
        let mut hands = 0u64;
        while !t.is_over() {
            for id in t.table_ids() {
                if !t.can_deal(id) {
                    continue;
                }
                t.new_hand(id).unwrap();
                hands += 1;
                t.set_level((hands / 40) as usize);
                let table = t.table(id).unwrap();
                let hand = table.hand().unwrap();
                let blind = |pick: fn(&Event) -> Option<usize>| {
                    hand.events()
                        .iter()
                        .find_map(pick)
                        .map(|i| table.dealt()[i])
                };
                let sb = blind(|e| match e {
                    Event::SmallBlind { seat, .. } => Some(*seat),
                    _ => None,
                });
                let bb = blind(|e| match e {
                    Event::BigBlind { seat, .. } => Some(*seat),
                    _ => None,
                });
                for &seat in table.dealt() {
                    let who = table.seat(seat).id.clone();
                    if arrived.get(&who) == Some(&id) {
                        arrived.remove(&who);
                        checked += 1;
                        let free_ride =
                            (seat == table.button() || Some(seat) == sb) && Some(seat) != bb;
                        assert!(
                            !free_ride,
                            "{who} moved in and was dealt on the button/small blind without the big blind"
                        );
                    }
                }
                play_out(&mut t, id);
                let report = t.finish_hand(id).unwrap();
                for m in report.moves {
                    moved_in += 1;
                    arrived.insert(m.id, m.to.table);
                }
                if t.is_over() {
                    break;
                }
            }
            assert!(hands < 20_000);
        }
    }
    assert!(
        moved_in > 10 && checked > 10,
        "moves happened: {moved_in}, checked {checked}"
    );
}

// ---- saving a tournament (#143) ----

/// Calling stations and shovers: they keep no state, so a restored
/// tournament's bots play exactly as the originals would.
fn stateless_field(n: usize) -> Vec<Entrant> {
    (0..n)
        .map(|i| {
            let bot: Box<dyn Bot> = if i % 3 == 0 {
                Box::new(Shover)
            } else {
                Box::new(CallingStation)
            };
            Entrant::bot(format!("p{i}"), format!("Player {i}"), bot)
        })
        .collect()
}

fn bot_for(id: &str) -> Option<Box<dyn Bot>> {
    let i: usize = id.strip_prefix('p')?.parse().ok()?;
    Some(if i % 3 == 0 {
        Box::new(Shover)
    } else {
        Box::new(CallingStation)
    })
}

/// One step of play: deal at the first table that can, or finish the hand
/// one table is in. Returns false once the tournament is over.
fn step(t: &mut Tournament, hands: &mut u64) -> bool {
    if t.is_over() {
        return false;
    }
    for id in t.table_ids() {
        if t.table(id).unwrap().in_hand() {
            // One bot acts; once the hand is over, the tournament takes it in.
            assert!(
                t.table_mut(id).unwrap().advance().unwrap(),
                "only bots here"
            );
            if !t.table(id).unwrap().in_hand() {
                t.finish_hand(id).unwrap();
            }
            return true;
        }
    }
    for id in t.table_ids() {
        if t.can_deal(id) {
            t.new_hand(id).unwrap();
            *hands += 1;
            t.set_level((*hands / 30) as usize);
            return true;
        }
    }
    panic!("stuck");
}

#[test]
fn a_tournament_saved_mid_hand_finishes_exactly_as_it_would_have() {
    let mut a = Tournament::new(config(10, 5, 11), stateless_field(30)).unwrap();
    let mut hands = 0;
    // Play until well in, stopping in the middle of a hand.
    while hands < 20
        || !a
            .table_ids()
            .iter()
            .any(|&id| a.table(id).unwrap().in_hand())
    {
        assert!(step(&mut a, &mut hands));
    }
    assert!(a.players_left() < 30 && a.table_ids().len() <= 3);
    let snap = a.snapshot();
    #[cfg(feature = "serde")]
    let snap: ducy_play::TournamentSnapshot =
        serde_json::from_str(&serde_json::to_string(&snap).unwrap()).unwrap();
    let mut b = Tournament::restore(&snap, bot_for).unwrap();
    assert_eq!(b.snapshot(), a.snapshot(), "restores to the same state");

    assert_eq!((b.level(), b.finishes()), (a.level(), a.finishes()));

    // Both go on in lockstep to the end.
    let (mut ha, mut hb) = (hands, hands);
    while step(&mut a, &mut ha) {
        assert!(step(&mut b, &mut hb));
        assert_eq!(chips(&a), chips(&b));
    }
    assert!(b.is_over());
    assert_eq!(a.finishes(), b.finishes(), "same places");
    assert_eq!(a.winner(), b.winner());
    assert_eq!(a.level(), b.level(), "same level");
    let places: HashSet<usize> = b.finishes().iter().map(|f| f.place).collect();
    assert_eq!(places, (2..=30).collect());
}

#[test]
fn a_bad_tournament_snapshot_is_refused() {
    let t = Tournament::new(config(10, 5, 2), stateless_field(12)).unwrap();
    // A bot that can't be given back.
    assert!(matches!(
        Tournament::restore(&t.snapshot(), |_| None),
        Err(PlayError::InvalidSnapshot)
    ));
    // Another version.
    let mut s = t.snapshot();
    s.version += 1;
    assert!(matches!(
        Tournament::restore(&s, bot_for),
        Err(PlayError::InvalidSnapshot)
    ));
    assert!(Tournament::restore(&t.snapshot(), bot_for).is_ok());
}
