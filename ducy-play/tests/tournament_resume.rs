//! A hosted tournament survives its host restarting: saved (as JSON) and
//! restored at any point, hands in play included, it carries on: the same
//! tables, stacks, levels and finishes, each turn with the time it had left,
//! timeouts and absences kept. 12 players play to a winner through restarts
//! every few steps.

use ducy_play::{
    BettingStructure, Command, Level, Outgoing, TournamentConfig, TournamentHost,
    TournamentHostSnapshot, Update, Variant,
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

fn join(h: &mut TournamentHost, id: &str, now: u64) -> Vec<Outgoing> {
    h.handle(
        id,
        Command::Join {
            name: String::new(),
        },
        now,
    )
}

/// Saved to JSON and loaded again, as a host restarting does.
fn restart(h: &TournamentHost, now: u64, later: u64) -> TournamentHost {
    let json = serde_json::to_string(&h.snapshot(now)).unwrap();
    let s: TournamentHostSnapshot = serde_json::from_str(&json).unwrap();
    TournamentHost::restore(&s, later, |_| None).unwrap()
}

#[test]
fn twelve_players_play_to_a_winner_through_restarts() {
    let mut rng = StdRng::seed_from_u64(11);
    let mut h = TournamentHost::new(config(7), players(12), 30_000).unwrap();
    let here: Vec<String> = (0..11).map(|i| format!("p{i}")).collect();
    let mut now = 0;
    for id in &here {
        join(&mut h, id, now);
    }
    let total = h.tournament().total_chips();
    let mut last: Vec<Outgoing> = Vec::new();
    let mut steps = 0;
    let mut restarts = 0;
    while !h.tournament().is_over() {
        now += 100;
        steps += 1;
        assert!(steps < 200_000, "the tournament ends");
        // Every so often the host restarts, mid-hand or not; everyone who
        // was here joins again and is sent their view.
        if steps % 37 == 0 {
            let before = h.tournament().snapshot();
            h = restart(&h, now, now + 5);
            now += 5;
            restarts += 1;
            assert_eq!(h.tournament().snapshot(), before, "the same tournament");
            last.clear();
            for id in &here {
                if h.tournament().find(id).is_some() {
                    last.extend(join(&mut h, id, now));
                }
            }
            continue;
        }
        let mut out = Vec::new();
        if h.can_deal() {
            if h.tournament()
                .table_ids()
                .iter()
                .all(|&t| !h.tournament().table(t).unwrap().in_hand())
            {
                let chips: u64 = h.standings().iter().map(|s| s.stack).sum();
                assert_eq!(chips, total, "chips are conserved");
            }
            out.extend(h.new_hands(now).unwrap());
            if steps % 400 == 0 {
                h.set_level(h.tournament().level() + 1);
            }
        } else if h.auto_to_act() {
            out.extend(h.advance(now).unwrap());
        } else {
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
                1 => "allin",
                _ if legal.can_check => "check",
                _ => "call",
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
        h.take_events();
        if !out.is_empty() {
            last = out;
        }
    }
    assert!(restarts > 10, "restarted {restarts} times");
    let t = h.tournament();
    assert_eq!(t.finishes().len(), 11);
    let mut places: Vec<usize> = t.finishes().iter().map(|f| f.place).collect();
    places.sort();
    assert_eq!(places, (2..=12).collect::<Vec<_>>());
    assert_eq!(t.winner().unwrap().place, 1);
}

#[test]
fn a_restart_keeps_the_time_left_timeouts_and_who_is_absent() {
    let mut h = TournamentHost::new(config(3), players(3), 1000).unwrap();
    for i in 0..3 {
        join(&mut h, &format!("p{i}"), 0);
    }
    h.new_hands(0).unwrap();
    // The first turn runs out once: one timeout for whoever was to act.
    h.tick(1001);
    let table = h.tournament().table_ids()[0];
    let left = h.turn_ms_left(table, 1300).unwrap();
    assert!(left > 0 && left <= 1000);
    // p2 drops out: absent.
    h.disconnected("p2", 1300);
    let saved = h.snapshot(1300);

    // Restored much later: the turn still has the time it had left, and
    // saved again at that moment it's exactly what was saved (the turns on
    // the clock, timeouts in a row, who's absent).
    let back = restart(&h, 1300, 50_000);
    assert_eq!(back.turn_ms_left(table, 50_000), Some(left));
    assert_eq!(back.seq(), h.seq());
    assert_eq!(back.snapshot(50_000), saved);
    let absent =
        |h: &TournamentHost, id: &str| h.standings().iter().find(|s| s.id == id).unwrap().absent;
    assert!(absent(&back, "p2"), "still absent");
    assert!(!absent(&back, "p0"));

    // The clock runs on from there: when the time left is up, it acts.
    let mut back = back;
    let before = back.seq();
    assert!(back.tick(50_000 + left - 1).is_empty(), "not yet");
    back.tick(50_000 + left);
    assert!(back.seq() > before, "the turn ran out on time");

    // Not a snapshot of another version.
    let mut json: serde_json::Value = serde_json::to_value(&saved).unwrap();
    json["version"] = serde_json::json!(999);
    let bad: TournamentHostSnapshot = serde_json::from_value(json).unwrap();
    assert!(TournamentHost::restore(&bad, 0, |_| None).is_err());
}
