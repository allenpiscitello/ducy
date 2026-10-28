//! A club table's runner (`ducy_play::runner`, ducy-web #452): events in
//! with the time, steps out with the next time to wake it, against a real
//! club table.

use ducy_play::fair_deal::{Reveal, hand_seed};
use ducy_play::runner::{
    ASK_MS, Action, Event, FAIR_STEP_MS, IDLE_MS, PlayerCommand, RESUME_GRACE_MS, RunnerConfig,
    RunnerUpdate, Step, TableResult, TableRunner,
};
use ducy_play::{Command, Table, TableHost, TableRules, TableSeat};

/// A club table: four empty seats, blinds 1/2, buy-ins 40 to 200.
fn runner(turn_ms: u64, config: RunnerConfig) -> TableRunner {
    let seats = (0..4).map(|_| TableSeat::empty()).collect();
    let table = Table::new(TableRules::no_limit_holdem(1, 2), seats, 40, 3).unwrap();
    let host = TableHost::without_host(table, turn_ms, 40, 200).unwrap();
    TableRunner::new(
        host,
        RunnerConfig {
            id: "t1".into(),
            ..config
        },
    )
}

fn cmd(id: &str, c: Command, online: Option<bool>) -> Event {
    Event::Cmd {
        id: id.into(),
        cmd: PlayerCommand::Table(c),
        online,
    }
}

fn join(id: &str, online: bool) -> Event {
    cmd(
        id,
        Command::Join {
            name: id.to_uppercase(),
        },
        Some(online),
    )
}

/// `id` joins and is given 100 chips (seat in join order).
fn sit(r: &mut TableRunner, id: &str, seat: usize, online: bool, now: u64) -> Step {
    r.event(join(id, online), now);
    r.event(cmd(id, Command::RequestChips { amount: 100 }, None), now);
    r.event(Event::Chips { seat, ok: true }, now)
}

fn results(step: &Step) -> Vec<&TableResult> {
    step.actions
        .iter()
        .filter_map(|a| {
            if let Action::Engine(r) = a {
                Some(r.as_ref())
            } else {
                None
            }
        })
        .collect()
}

fn sends(step: &Step) -> Vec<(&str, &RunnerUpdate)> {
    step.actions
        .iter()
        .filter_map(|a| {
            if let Action::Send { to, update } = a {
                Some((to.as_str(), update))
            } else {
                None
            }
        })
        .collect()
}

fn idle(step: &Step) -> Vec<&str> {
    step.actions
        .iter()
        .filter_map(|a| {
            if let Action::Idle { id } = a {
                Some(id.as_str())
            } else {
                None
            }
        })
        .collect()
}

/// Wakes the runner whenever it asks, until `until`, or `stop` holds for a
/// step. Returns the time it stopped at and the step.
fn run_until(
    r: &mut TableRunner,
    mut wake: Option<u64>,
    until: u64,
    stop: impl Fn(&Step) -> bool,
) -> Option<(u64, Step)> {
    while let Some(t) = wake {
        if t > until {
            return None;
        }
        let step = r.event(Event::Wake, t);
        if stop(&step) {
            return Some((t, step));
        }
        wake = step.wake;
    }
    None
}

/// Everyone checks or calls until the hand is over.
fn play_out(r: &mut TableRunner, players: &[&str], now: u64) -> Step {
    let mut last = r.event(Event::State, now);
    while r.host().table().in_hand() {
        let seat = r.host().table().to_act().expect("someone to act");
        let legal = r.host().table().view(seat).legal.expect("their turn");
        let kind = if legal.can_check { "check" } else { "call" };
        let seq = r.host().seq();
        last = r.event(
            cmd(
                players[seat],
                Command::Act {
                    seq,
                    kind: kind.into(),
                    amount: 0,
                },
                None,
            ),
            now,
        );
    }
    last
}

#[test]
fn the_clock_stops_while_no_one_can_be_reached_and_goes_on_with_the_time_left() {
    let mut r = runner(20_000, RunnerConfig::default());
    sit(&mut r, "a", 0, true, 0);
    let step = sit(&mut r, "b", 1, true, 0);
    // The first hand comes once the fair-deal round has had its time.
    let (t, step) = run_until(&mut r, step.wake, 20_000, |s| {
        results(s).iter().any(|res| !res.state.complete)
    })
    .expect("a hand");
    assert!(t <= 2 * FAIR_STEP_MS + 300, "dealt at {t}");
    let left = results(&step).last().unwrap().turn_ms_left.unwrap();
    assert!(
        step.wake.unwrap() <= t + left + 50,
        "woken by the turn clock at the latest"
    );
    // 500 ms on, the link goes: stopped.
    let step = r.event(Event::Link { ok: false }, t + 500);
    assert!(step.stopped);
    // Five minutes later it's back: the turn clock has lost only those 500 ms.
    let step = r.event(Event::Link { ok: true }, t + 300_500);
    assert!(!step.stopped);
    assert_eq!(
        results(&step).last().unwrap().turn_ms_left,
        Some(left - 500)
    );
    // Everyone seated drops off: stopped too; back: going again.
    assert!(
        !r.event(
            Event::Presence {
                id: "a".into(),
                on: false
            },
            t + 301_000
        )
        .stopped,
        "b is still here"
    );
    assert!(
        r.event(
            Event::Presence {
                id: "b".into(),
                on: false
            },
            t + 301_000
        )
        .stopped
    );
    let step = r.event(
        Event::Presence {
            id: "a".into(),
            on: true,
        },
        t + 400_000,
    );
    assert_eq!(
        results(&step).last().unwrap().turn_ms_left,
        Some(left - 1000)
    );
}

#[test]
fn nothing_but_the_idle_check_wakes_a_stopped_table() {
    let mut r = runner(20_000, RunnerConfig::default());
    sit(&mut r, "a", 0, true, 0);
    sit(&mut r, "b", 1, true, 0);
    let step = r.event(Event::Link { ok: false }, 100);
    assert!(step.stopped);
    assert_eq!(step.wake, Some(IDLE_MS), "only the idle check");
}

#[test]
fn a_hand_is_dealt_from_a_fair_deal_round() {
    let mut r = runner(0, RunnerConfig::default());
    sit(&mut r, "a", 0, true, 0);
    let step = sit(&mut r, "b", 1, true, 0);
    let asks: Vec<_> = sends(&step)
        .into_iter()
        .filter_map(|(to, u)| {
            if let RunnerUpdate::FairCommit { round } = u {
                Some((to, round.clone()))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        asks.iter().map(|(to, _)| *to).collect::<Vec<_>>(),
        ["a", "b"]
    );
    let round = asks[0].1.clone();
    // Both commit: the list goes to each, host first.
    let (ra, rb) = (Reveal::random(), Reveal::random());
    r.event(
        Event::Cmd {
            id: "a".into(),
            cmd: PlayerCommand::FairCommit {
                round: round.clone(),
                commitment: ra.commitment(),
            },
            online: None,
        },
        100,
    );
    let step = r.event(
        Event::Cmd {
            id: "b".into(),
            cmd: PlayerCommand::FairCommit {
                round: round.clone(),
                commitment: rb.commitment(),
            },
            online: None,
        },
        200,
    );
    let lists: Vec<_> = sends(&step)
        .into_iter()
        .filter_map(|(_, u)| {
            if let RunnerUpdate::FairReveal { list, .. } = u {
                Some(list.clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(lists.len(), 2);
    assert_eq!(
        lists[0]
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        ["host", "a", "b"]
    );
    // Ann reveals; Bo's doesn't open his commitment, so he's left out.
    r.event(
        Event::Cmd {
            id: "a".into(),
            cmd: PlayerCommand::FairReveal {
                round: round.clone(),
                reveal: ra,
            },
            online: None,
        },
        300,
    );
    let step = r.event(
        Event::Cmd {
            id: "b".into(),
            cmd: PlayerCommand::FairReveal {
                round: round.clone(),
                reveal: Reveal::random(),
            },
            online: None,
        },
        400,
    );
    let (_, step) = run_until(&mut r, step.wake, 20_000, |s| {
        results(s).iter().any(|res| res.seed.is_some())
    })
    .expect("dealt");
    let seed = results(&step)
        .iter()
        .find_map(|res| res.seed)
        .unwrap()
        .expect("from the round");
    // The hand over: each dealt in gets the reveals, after its final state.
    let step = play_out(&mut r, &["a", "b", "", ""], 30_000);
    let first_send = step
        .actions
        .iter()
        .position(|a| matches!(a, Action::Send { .. }))
        .unwrap();
    assert!(
        step.actions[..first_send]
            .iter()
            .any(|a| matches!(a, Action::Engine(_)))
    );
    let got: Vec<_> = sends(&step)
        .into_iter()
        .filter_map(|(to, u)| {
            if let RunnerUpdate::FairResult(f) = u {
                Some((to, f))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        got.iter()
            .map(|(to, f)| (*to, f.index, f.dealt, f.seed))
            .collect::<Vec<_>>(),
        [("a", 0, 2, seed), ("b", 1, 2, seed)]
    );
    // The seed is the host's value then Ann's, and the host's opens its commitment.
    let used = &got[0].1.reveals;
    assert_eq!(
        used.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
        ["host", "a"]
    );
    assert!(used[0].1.opens(&lists[0][0].1));
    assert_eq!(used[1].1, ra);
    assert_eq!(seed, hand_seed(&[used[0].1.value, ra.value]));
    // Ann comes back to the table later: she gets that result again.
    let step = r.event(join("a", true), 31_000);
    assert!(
        sends(&step)
            .iter()
            .any(|(to, u)| *to == "a" && matches!(u, RunnerUpdate::FairResult(_)))
    );
}

#[test]
fn a_seed_picker_chooses_a_hand_and_then_leaves_the_deals_alone() {
    use std::cell::RefCell;
    use std::rc::Rc;
    let mut r = runner(
        0,
        RunnerConfig {
            next_hand_ms: 100,
            ..RunnerConfig::default()
        },
    );
    // It picks the first hand only, and sees who's seated when it does.
    let seen = Rc::new(RefCell::new(Vec::new()));
    let saw = seen.clone();
    r.set_seed_picker(Some(Box::new(move |host| {
        let mut saw = saw.borrow_mut();
        saw.push(host.seated().len());
        (saw.len() == 1).then_some([7; 32])
    })));
    sit(&mut r, "a", 0, true, 0);
    let step = sit(&mut r, "b", 1, true, 0);
    let dealt = |s: &Step| results(s).iter().find_map(|res| res.seed);
    let (_, step) = run_until(&mut r, step.wake, 20_000, |s| dealt(s).is_some()).expect("dealt");
    assert_eq!(dealt(&step), Some(Some([7; 32])));
    assert_eq!(*seen.borrow(), [2]);
    // Its hand over: no fair-deal results, since no round dealt it.
    let step = play_out(&mut r, &["a", "b", "", ""], 30_000);
    assert!(
        !sends(&step)
            .iter()
            .any(|(_, u)| matches!(u, RunnerUpdate::FairResult(_)))
    );
    // The next hand is the table's own, dealt the table's 100 ms later.
    let (at, step) = run_until(&mut r, step.wake, 60_000, |s| dealt(s).is_some()).expect("dealt");
    assert_eq!(at, 30_100);
    assert_ne!(dealt(&step), Some(Some([7; 32])));
    assert_eq!(seen.borrow().len(), 2);
}

#[test]
fn idle_seats_are_asked_when_connected_and_stood_up_when_not() {
    let mut r = runner(0, RunnerConfig::default());
    r.event(join("a", true), 0);
    r.event(join("b", false), 0);
    let step = r.event(Event::Wake, IDLE_MS);
    assert_eq!(idle(&step), ["b"], "not connected: stood up");
    assert_eq!(
        sends(&step),
        [(
            "a",
            &RunnerUpdate::IdleAsk {
                secs: ASK_MS / 1000
            }
        )]
    );
    // She answers: the question goes, and she has another while.
    let step = r.event(
        Event::Cmd {
            id: "a".into(),
            cmd: PlayerCommand::StillHere,
            online: None,
        },
        IDLE_MS + 1000,
    );
    assert_eq!(sends(&step), [("a", &RunnerUpdate::IdleDone)]);
    assert!(step.actions.contains(&Action::Active {
        id: "a".into(),
        at: IDLE_MS + 1000
    }));
    // Bo is on his way out: not again. Ann is asked again later, and this time doesn't answer.
    let step = r.event(Event::Wake, 2 * IDLE_MS + 1000);
    assert!(idle(&step).is_empty());
    let step = r.event(Event::Wake, 2 * IDLE_MS + 1000 + ASK_MS);
    assert_eq!(idle(&step), ["a"]);
}

#[test]
fn no_one_is_stood_up_while_the_host_cant_reach_the_relay() {
    let mut r = runner(0, RunnerConfig::default());
    r.event(join("a", false), 0);
    r.event(Event::Link { ok: false }, 0);
    let step = r.event(Event::Wake, 3 * IDLE_MS);
    assert!(idle(&step).is_empty());
    assert_eq!(
        step.wake,
        Some(3 * IDLE_MS + 30_000),
        "it looks again in a while"
    );
}

/// A table with Ann and Bo seated, saved and restored, waiting for them.
fn restored() -> TableRunner {
    let mut r = runner(20_000, RunnerConfig::default());
    sit(&mut r, "a", 0, true, 0);
    sit(&mut r, "b", 1, true, 0);
    let snap = r.snapshot(10);
    let host = TableHost::restore(&snap, 1000, |_| None).unwrap();
    TableRunner::new(
        host,
        RunnerConfig {
            id: "t1".into(),
            paused: true,
            ..RunnerConfig::default()
        },
    )
}

#[test]
fn a_restored_table_goes_on_once_everyone_is_back() {
    let mut r = restored();
    assert!(r.event(Event::State, 0).paused);
    assert!(
        r.event(
            Event::Presence {
                id: "a".into(),
                on: true
            },
            100
        )
        .paused
    );
    let step = r.event(join("b", true), 200);
    assert!(!step.paused);
}

#[test]
fn a_restored_table_goes_on_without_whoever_isnt_back_after_the_grace_time() {
    let mut r = restored();
    r.event(Event::State, 0);
    let step = r.event(
        Event::Presence {
            id: "a".into(),
            on: true,
        },
        100,
    );
    assert!(step.wake.unwrap() <= RESUME_GRACE_MS);
    let step = r.event(Event::Wake, RESUME_GRACE_MS);
    assert!(!step.paused);
}

#[test]
fn the_clubs_answers_reach_the_table() {
    let mut r = runner(0, RunnerConfig::default());
    r.event(join("a", true), 0);
    let step = r.event(cmd("a", Command::RequestChips { amount: 100 }, None), 1);
    assert_eq!(results(&step)[0].chip_requests.len(), 1);
    let step = r.event(Event::Chips { seat: 0, ok: true }, 2);
    assert_eq!(results(&step)[0].players[0].chips, 100);
    let step = r.event(Event::Remove { seat: 0 }, 3);
    let res = results(&step);
    assert_eq!(res[0].departed.len(), 1);
    assert_eq!(res[0].departed[0].chips, 100);
}
