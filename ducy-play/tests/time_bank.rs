//! The time bank: each person at a table with a turn clock has
//! [`TIME_BANK_MS`] of extra time. Asking for it on their turn adds all of
//! it to the clock; what's left when they act goes back in; each hand puts
//! [`TIME_BANK_REFILL_MS`] back, up to full.

use ducy_play::{
    Command, HostSnapshot, Outgoing, SeatStatus, TIME_BANK_MS, TIME_BANK_REFILL_MS, Table,
    TableHost, TableRules, TableSeat, Update,
};

const TURN: u64 = 10_000;

/// A club table with Ann and Bo, 100 chips each, a 10 s turn clock, and a
/// hand dealt at time 0.
fn table() -> TableHost {
    let seats = (0..4).map(|_| TableSeat::empty()).collect();
    let t = Table::new(TableRules::no_limit_holdem(1, 2), seats, 40, 3).unwrap();
    let mut h = TableHost::without_host(t, TURN, 40, 200).unwrap();
    for (c, name, seat) in [("a", "Ann", 0), ("b", "Bo", 1)] {
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
    h.new_hand(0).unwrap();
    h
}

/// Who's to act, as a client id.
fn to_act(h: &TableHost) -> &'static str {
    if h.table().to_act() == Some(0) {
        "a"
    } else {
        "b"
    }
}

/// `client`'s own status in the last state sent to them.
fn me(out: &[Outgoing], client: &str) -> SeatStatus {
    out.iter()
        .rev()
        .find_map(|o| match &o.update {
            Update::State { me: Some(me), .. } if o.to == client => Some(me.clone()),
            _ => None,
        })
        .expect("a state for them")
}

fn rejected(out: &[Outgoing], client: &str) -> bool {
    out.iter()
        .any(|o| o.to == client && matches!(o.update, Update::Rejected { .. }))
}

fn call(h: &mut TableHost, client: &str, now: u64) -> Vec<Outgoing> {
    let seq = h.seq();
    h.handle(
        client,
        Command::Act {
            seq,
            kind: "call".to_string(),
            amount: 0,
        },
        now,
    )
}

#[test]
fn everyone_starts_with_a_full_bank_and_it_adds_to_the_clock() {
    let mut h = table();
    let who = to_act(&h);
    let other = if who == "a" { "b" } else { "a" };
    assert_eq!(me(&h.updates(1), who).time_bank_ms, TIME_BANK_MS);
    // Not their turn: refused.
    assert!(rejected(&h.handle(other, Command::TimeBank, 1_000), other));
    // Eight seconds in, they ask for it: a minute more on the clock.
    let out = h.handle(who, Command::TimeBank, 8_000);
    assert_eq!(h.turn_ms_left(8_000), Some(TURN - 8_000 + TIME_BANK_MS));
    let status = me(&out, who);
    assert!(status.time_bank_on);
    assert_eq!(status.time_bank_ms, 0, "all of it is on the clock");
    // Once per turn.
    assert!(rejected(&h.handle(who, Command::TimeBank, 9_000), who));
    // They act at 20 s: 10 s of the bank used, the rest goes back.
    let out = call(&mut h, who, 20_000);
    let status = me(&out, who);
    assert!(!status.time_bank_on);
    assert_eq!(status.time_bank_ms, TIME_BANK_MS - 10_000);
}

#[test]
fn running_out_while_it_runs_empties_it_and_each_hand_refills_it() {
    let mut h = table();
    let who = to_act(&h);
    h.handle(who, Command::TimeBank, 1_000);
    // The turn runs out, bank and all: they check or fold, and it's empty.
    let out = h.tick(TURN + TIME_BANK_MS + 1);
    assert_eq!(me(&out, who).time_bank_ms, 0);
    // Play the hand out, then each new hand puts some back, up to full.
    let mut now = TURN + TIME_BANK_MS + 2;
    let mut bank = 0;
    for _ in 0..14 {
        while h.table().in_hand() {
            let c = to_act(&h);
            call(&mut h, c, now);
            let seq = h.seq();
            h.handle(
                c,
                Command::Act {
                    seq,
                    kind: "check".to_string(),
                    amount: 0,
                },
                now,
            );
            now += 10;
        }
        now += 10;
        let out = h.new_hand(now).unwrap();
        bank = (bank + TIME_BANK_REFILL_MS).min(TIME_BANK_MS);
        assert_eq!(me(&out, who).time_bank_ms, bank);
    }
    assert_eq!(bank, TIME_BANK_MS, "full again");
}

#[test]
fn a_running_bank_is_saved_and_restored() {
    let mut h = table();
    let who = to_act(&h);
    h.handle(who, Command::TimeBank, 2_000);
    let json = serde_json::to_string(&h.snapshot(3_000)).unwrap();
    let s: HostSnapshot = serde_json::from_str(&json).unwrap();
    let mut r = TableHost::restore(&s, 100_000, |_| None).unwrap();
    for c in ["a", "b"] {
        r.handle(
            c,
            Command::Join {
                name: String::new(),
            },
            100_000,
        );
    }
    r.resume(100_000);
    // The clock goes on from where it was, bank included.
    assert_eq!(r.turn_ms_left(100_000), Some(TURN - 3_000 + TIME_BANK_MS));
    // Acting a second later gives almost all of it back.
    let out = call(&mut r, who, 101_000);
    assert_eq!(me(&out, who).time_bank_ms, TIME_BANK_MS);
}
