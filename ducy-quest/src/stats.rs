//! The quest's stats (ducy-web #304): the bankroll over the run, win rate
//! and hands by stakes, the biggest pots, results with each named Regular,
//! and the hall of records. Built from the session history and past runs.

use serde::Serialize;

use crate::quest::{RunRecord, START_BANKROLL, Session};
use crate::world::{js_round, world};

/// A stakes' results.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AtStakes {
    /// The stakes, `1/2`.
    pub stakes: String,
    /// Sessions.
    pub sessions: usize,
    /// Hands.
    pub hands: f64,
    /// Net, in hundredths.
    pub net: f64,
    /// Big blinds won per 100 hands.
    pub bb100: Option<f64>,
}

/// Hands, net and bb/100 at each stakes, in stakes order.
pub fn by_stakes(history: &[Session]) -> Vec<AtStakes> {
    let mut out = Vec::new();
    for (name, blinds) in &world().stakes {
        let blinds = [blinds[0] as f64, blinds[1] as f64];
        let ss: Vec<&Session> = history
            .iter()
            .filter(|s| s.stakes == Some(blinds))
            .collect();
        if ss.is_empty() {
            continue;
        }
        let hands = ss.iter().fold(0.0, |a, s| a + s.hands);
        let net = ss.iter().fold(0.0, |a, s| a + s.net);
        out.push(AtStakes {
            stakes: name.clone(),
            sessions: ss.len(),
            hands,
            net,
            bb100: (hands != 0.0).then(|| (net / blinds[1] / hands) * 100.0),
        });
    }
    out
}

/// The biggest single pots won and lost across the run (lost is negative).
pub fn biggest_pots(history: &[Session]) -> (f64, f64) {
    let won = history.iter().fold(0.0, |a: f64, s| a.max(s.big_win));
    let lost = history.iter().fold(0.0, |a: f64, s| a.min(s.big_loss));
    (won, lost)
}

/// Your results with a named Regular.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WithRegular {
    /// Their id.
    pub id: String,
    /// Sessions with them.
    pub sessions: usize,
    /// Net in those.
    pub net: f64,
}

/// Your results in sessions with each named Regular at the table, most
/// played first.
pub fn with_regulars(history: &[Session]) -> Vec<WithRegular> {
    let mut out: Vec<WithRegular> = Vec::new();
    for s in history {
        for id in &s.with {
            match out.iter_mut().find(|r| r.id == *id) {
                Some(r) => {
                    r.sessions += 1;
                    r.net += s.net;
                }
                None => out.push(WithRegular {
                    id: id.clone(),
                    sessions: 1,
                    net: s.net,
                }),
            }
        }
    }
    out.sort_by(|a, b| b.sessions.cmp(&a.sessions).then(b.net.total_cmp(&a.net)));
    out
}

/// The bankroll after a session.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Point {
    /// The session, 0 the start.
    pub session: usize,
    /// The bankroll.
    pub bankroll: f64,
}

/// The bankroll after each session, from the start.
pub fn bankroll_series(history: &[Session]) -> Vec<Point> {
    let mut b = START_BANKROLL;
    let mut out = vec![Point {
        session: 0,
        bankroll: b,
    }];
    for (i, s) in history.iter().enumerate() {
        b = s.bankroll.unwrap_or(b + s.net);
        out.push(Point {
            session: i + 1,
            bankroll: b,
        });
    }
    out
}

/// Past runs for the hall of records: wins first (fastest first), then the
/// best peaks; ten at most.
pub fn hall_of_records(runs: &[RunRecord]) -> Vec<RunRecord> {
    hall_order(runs)
        .into_iter()
        .map(|i| runs[i].clone())
        .collect()
}

/// [`hall_of_records`] as indices into `runs`.
pub fn hall_order(runs: &[RunRecord]) -> Vec<usize> {
    let won = |i: &usize| runs[*i].result == "won";
    let mut wins: Vec<usize> = (0..runs.len()).filter(won).collect();
    wins.sort_by(|&a, &b| runs[a].minutes.total_cmp(&runs[b].minutes));
    let mut rest: Vec<usize> = (0..runs.len()).filter(|i| !won(i)).collect();
    rest.sort_by(|&a, &b| runs[b].peak.total_cmp(&runs[a].peak));
    wins.extend(rest);
    wins.truncate(10);
    wins
}

/// Game minutes as `3 days 4 hours` or `5 hours`.
pub fn duration_text(minutes: f64) -> String {
    let h = js_round(minutes / 60.0).max(0.0) as u64;
    let (d, r) = (h / 24, h % 24);
    let part = |n: u64, w: &str| format!("{n} {w}{}", if n == 1 { "" } else { "s" });
    match (d, r) {
        (0, r) => part(r, "hour"),
        (d, 0) => part(d, "day"),
        (d, r) => format!("{} {}", part(d, "day"), part(r, "hour")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read() {
        assert_eq!(duration_text(300.0), "5 hours");
        assert_eq!(duration_text(60.0), "1 hour");
        assert_eq!(duration_text(3.0 * 1440.0 + 240.0), "3 days 4 hours");
        assert_eq!(duration_text(1440.0), "1 day");
    }
}
