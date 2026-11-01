//! A club tournament's runner (`ducy_host::tourney_runner`, ducy-web #458):
//! events in with the time, steps out with the next time to wake it. The
//! same cases as ducy-web's `test/tourney-runner.test.mjs`, against a real
//! tournament.

use ducy_host::tourney_runner::{
    BOT_PACE_MS, StructureStep, TourneyAction, TourneyConfig, TourneyEvent, TourneyRunner,
    TourneyStep,
};
use ducy_host::{Command, TournamentHost, Update};
use ducy_play::{BettingStructure, Level, TournamentConfig, Variant};

const MIN: u64 = 60_000;

/// Level 1 (10 min), a break (5), level 2 (10), level 3 (10).
fn structure() -> Vec<StructureStep> {
    let step = |minutes, is_break| StructureStep { minutes, is_break };
    vec![
        step(10, false),
        step(5, true),
        step(10, false),
        step(10, false),
    ]
}

/// A tournament of `n` players at tables of six, all absent until they join.
fn host(n: usize) -> TournamentHost {
    let config = TournamentConfig {
        variant: Variant::Holdem,
        structure: BettingStructure::NoLimit,
        table_size: 6,
        starting_stack: 200,
        levels: vec![
            Level::new(10, 20, 0),
            Level::new(25, 50, 5),
            Level::new(50, 100, 10),
        ],
        paid: 2,
        seed: 7,
    };
    let players = (0..n).map(|i| (format!("p{i}"), format!("P{i}"))).collect();
    TournamentHost::new(config, players, 30_000).unwrap()
}

fn runner(h: TournamentHost, config: TourneyConfig) -> TourneyRunner {
    TourneyRunner::new(
        h,
        TourneyConfig {
            id: "x".into(),
            structure: structure(),
            ..config
        },
    )
}

fn ev(r: &mut TourneyRunner, e: TourneyEvent, now: u64) -> TourneyStep {
    r.event(e, now).unwrap()
}

#[test]
fn the_level_clock_runs_its_levels_and_a_break_and_the_last_stays() {
    let mut r = runner(host(4), TourneyConfig::default());
    let step = ev(&mut r, TourneyEvent::Begin, 0);
    assert_eq!(
        (step.step, step.on_break, step.step_ends_at),
        (0, false, Some(10 * MIN))
    );
    assert!(step.wake.unwrap() <= 10 * MIN);
    // The break: no new hands, however long it's due.
    let step = ev(&mut r, TourneyEvent::Wake, 10 * MIN);
    assert_eq!(
        (step.step, step.on_break, step.step_ends_at),
        (1, true, Some(15 * MIN))
    );
    let hands = r
        .host()
        .tournament()
        .table_ids()
        .iter()
        .map(|&t| r.host().table_view(t).unwrap().hand_number)
        .max();
    let step = ev(&mut r, TourneyEvent::Wake, 15 * MIN - 1);
    assert!(step.wake.unwrap() >= 15 * MIN - 1);
    assert_eq!(
        r.host()
            .tournament()
            .table_ids()
            .iter()
            .map(|&t| r.host().table_view(t).unwrap().hand_number)
            .max(),
        hands,
        "no hand dealt in the break"
    );
    // Level 2 after it.
    let step = ev(&mut r, TourneyEvent::Wake, 15 * MIN);
    assert_eq!((step.step, step.level, step.on_break), (2, 1, false));
    assert_eq!(r.host().tournament().level(), 1);
    // Level 3 is the last: it stays.
    let step = ev(&mut r, TourneyEvent::Wake, 25 * MIN);
    assert_eq!(
        (step.step, step.level, step.step_ends_at),
        (3, 2, Some(35 * MIN))
    );
    let step = ev(&mut r, TourneyEvent::Wake, 35 * MIN);
    assert_eq!((step.step, step.level), (3, 2));
}

#[test]
fn resumed_the_clock_goes_on_with_the_time_it_had_left() {
    let mut h = host(4);
    h.set_level(1);
    let mut r = runner(
        h,
        TourneyConfig {
            step: 2,
            step_left_ms: Some(4 * MIN),
            level: 1,
            ..TourneyConfig::default()
        },
    );
    let step = ev(&mut r, TourneyEvent::Begin, 1000);
    assert_eq!(
        (step.step, step.level, step.step_ends_at),
        (2, 1, Some(1000 + 4 * MIN))
    );
}

#[test]
fn the_runner_plays_a_tournament_to_a_winner() {
    // Only p0 joins, and goes all in whenever it's their turn; everyone
    // else is absent and folds or checks, played for at the bot's pace. The
    // runner deals, moves the levels on and breaks tables until p0 has
    // every chip.
    let mut r = runner(
        host(8),
        TourneyConfig {
            level_ms: Some(2 * MIN),
            hands: true,
            ..TourneyConfig::default()
        },
    );
    let mut now = 0;
    let mut step = ev(&mut r, TourneyEvent::Begin, now);
    step = ev(
        &mut r,
        TourneyEvent::Cmd {
            id: "p0".into(),
            cmd: Some(Command::Join { name: "P0".into() }),
        },
        now,
    )
    .merge(step);
    let (mut hands, mut winner, mut paced) = (0, None, false);
    for _ in 0..100_000 {
        // p0's turn: all in, at once.
        let turn = r
            .host()
            .view_for("p0")
            .is_some_and(|v| v.legal.is_some())
            .then(|| r.host().seq());
        let before = now;
        step = match turn {
            // A move that isn't legal just now is refused: the next one instead.
            Some(seq) => ["allin", "call", "check", "fold"]
                .into_iter()
                .map(|kind| {
                    ev(
                        &mut r,
                        TourneyEvent::Cmd {
                            id: "p0".into(),
                            cmd: Some(Command::Act {
                                seq,
                                kind: kind.into(),
                                amount: 0,
                            }),
                        },
                        now,
                    )
                })
                .find(|s| !refused(s))
                .expect("some move is legal"),
            None => {
                let Some(t) = step.wake else { break };
                now = t;
                ev(&mut r, TourneyEvent::Wake, now)
            }
        };
        paced |= now - before == BOT_PACE_MS;
        for a in &step.actions {
            match a {
                TourneyAction::Hand { view, .. } => {
                    assert!(view.complete);
                    hands += 1;
                }
                TourneyAction::Engine(res) => {
                    assert_eq!(res.total_chips, 1600, "chips conserved");
                    if let Some(w) = &res.events.winner {
                        winner = Some(w.id.clone());
                    }
                }
            }
        }
    }
    assert_eq!(winner.as_deref(), Some("p0"));
    assert_eq!(step.wake, None, "nothing more to do");
    assert!(hands > 10, "{hands} hands reported");
    assert!(paced, "absent players moved at the bot's pace");
}

/// p0's move was refused.
fn refused(step: &TourneyStep) -> bool {
    step.actions.iter().any(|a| matches!(a, TourneyAction::Engine(res) if res.out.iter().any(|o| o.to == "p0" && matches!(o.update, Update::Rejected { .. }))))
}

trait Merge {
    fn merge(self, earlier: TourneyStep) -> TourneyStep;
}

impl Merge for TourneyStep {
    /// This step, after `earlier`'s actions.
    fn merge(mut self, mut earlier: TourneyStep) -> TourneyStep {
        earlier.actions.append(&mut self.actions);
        self.actions = earlier.actions;
        self
    }
}

#[test]
fn a_command_that_cant_be_read_is_refused_and_entries_need_a_place() {
    let mut r = runner(host(4), TourneyConfig::default());
    ev(&mut r, TourneyEvent::Begin, 0);
    let step = ev(
        &mut r,
        TourneyEvent::Cmd {
            id: "p0".into(),
            cmd: None,
        },
        1,
    );
    let TourneyAction::Engine(res) = &step.actions[0] else {
        panic!("an engine result")
    };
    assert!(matches!(&res.out[0].update, Update::Rejected { reason } if reason == "bad message"));
    let step = ev(
        &mut r,
        TourneyEvent::Cmd {
            id: "p0".into(),
            cmd: Some(Command::Join { name: "P0".into() }),
        },
        2,
    );
    assert!(!step.actions.is_empty());
    let step = ev(
        &mut r,
        TourneyEvent::AddEntry {
            id: "p9".into(),
            name: "P9".into(),
        },
        3,
    );
    let TourneyAction::Engine(res) = &step.actions[0] else {
        panic!("an engine result")
    };
    assert_eq!(res.players_left, 5);
}
