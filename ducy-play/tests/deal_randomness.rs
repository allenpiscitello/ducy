//! The shuffle deals every card to every position equally often, and tables
//! for people deal from fresh system randomness rather than a seed.

use ducy::deck::Deck;
use ducy_play::{Deal, Table, TableRules, TableSeat, Variant};
use std::collections::{HashMap, HashSet};

const PLAYERS: usize = 9;

/// Each card's index (0..52).
fn card_index() -> HashMap<String, usize> {
    Deck::all_cards()
        .iter(false)
        .enumerate()
        .map(|(i, c)| (c.to_string(), i))
        .collect()
}

/// Pearson's chi-square statistic for `counts` against equal expected counts.
fn chi_square(counts: &[u64], expected: f64) -> f64 {
    counts
        .iter()
        .map(|&o| (o as f64 - expected).powi(2) / expected)
        .sum()
}

#[test]
fn every_card_is_equally_likely_in_every_position() {
    let index = card_index();
    let deals = 60_000u64;
    // Each player's hole cards (as a set), and each board position.
    let mut players = vec![[0u64; 52]; PLAYERS];
    let mut board = vec![[0u64; 52]; 5];
    for _ in 0..deals {
        let d = Deal::random(Variant::Holdem, PLAYERS, None).unwrap();
        for (p, hole) in d.hole_cards().iter().enumerate() {
            for c in hole.iter(false) {
                players[p][index[&c.to_string()]] += 1;
            }
        }
        for (i, c) in d.board().iter().enumerate() {
            board[i][index[&c.to_string()]] += 1;
        }
    }
    // 14 positions × 51 degrees of freedom. For a fair shuffle the sum has
    // mean 714 and standard deviation about 38; failing needs it beyond 6
    // standard deviations, which a fair shuffle essentially never does.
    let df: f64 = 14.0 * 51.0;
    let stat: f64 = players
        .iter()
        .map(|c| chi_square(c, deals as f64 * 2.0 / 52.0))
        .chain(board.iter().map(|c| chi_square(c, deals as f64 / 52.0)))
        .sum();
    let limit = df + 6.0 * (2.0 * df).sqrt();
    assert!(
        stat < limit,
        "chi-square {stat:.0} over {limit:.0} (df {df})"
    );
    // And no single position is badly off.
    for (p, c) in players.iter().enumerate() {
        let s = chi_square(c, deals as f64 * 2.0 / 52.0);
        assert!(
            s < 51.0 + 6.0 * 102f64.sqrt(),
            "player {p}: chi-square {s:.0}"
        );
    }
}

#[test]
fn unseeded_deals_do_not_repeat() {
    let mut seen = HashSet::new();
    for _ in 0..20_000 {
        let d = Deal::random(Variant::Holdem, PLAYERS, None).unwrap();
        let key: Vec<String> = d
            .hole_cards()
            .iter()
            .map(|h| h.to_string())
            .chain(d.board().iter().map(|c| c.to_string()))
            .collect();
        assert!(seen.insert(key), "the same deal came up twice");
    }
}

fn two_humans(seed: u64) -> Table {
    let seats = vec![TableSeat::human("A", "a"), TableSeat::human("B", "b")];
    Table::new(TableRules::no_limit_holdem(1, 2), seats, 200, seed).unwrap()
}

/// The first hand's cards, as one string.
fn first_hand(mut t: Table) -> String {
    t.new_hand().unwrap();
    let v = t.view(0);
    format!("{:?} {:?}", v.seats[0].cards, t.view(1).seats[0].cards)
}

#[test]
fn seeded_tables_repeat_and_secure_tables_do_not() {
    // A seed replays the same session: right for tests and simulations.
    assert_eq!(first_hand(two_humans(42)), first_hand(two_humans(42)));
    // Secure deals ignore the seed: the same seed gives different hands.
    let secure: HashSet<String> = (0..20)
        .map(|_| first_hand(two_humans(42).with_secure_deals()))
        .collect();
    assert!(
        secure.len() > 15,
        "secure deals repeated: {} distinct of 20",
        secure.len()
    );
    assert!(two_humans(42).with_secure_deals().secure_deals());
    assert!(!two_humans(42).secure_deals());
}
