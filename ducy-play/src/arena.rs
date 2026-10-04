//! Long bot-vs-bot sessions: players are randomly seated at tables, reseated
//! every few hands, and every hand starts from the same stack, while their
//! wins and losses are tracked.

use rand::{RngExt, SeedableRng, rngs::StdRng};

use crate::{
    bot::{Bot, play_hand},
    deal::Deal,
    error::PlayError,
    hand::{Hand, MAX_PLAYERS},
    rules::TableRules,
    stats::{OpponentModel, SeatStats},
};

/// How to run an arena session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArenaConfig {
    /// Game, betting structure and blinds.
    pub rules: TableRules,
    /// Seats per table. Players are split into tables of this size; when the
    /// count doesn't divide evenly, the tables are as even as possible.
    pub table_size: usize,
    /// Hands each player should play. The session stops once every player
    /// has played at least this many.
    pub hands_per_player: usize,
    /// Hands played at a table before everyone is randomly reseated.
    pub reseat_every: usize,
    /// Every player starts every hand with this many big blinds.
    pub starting_bb: u64,
    /// Seed for seating and cards; the same seed replays the same session.
    pub seed: u64,
}

impl ArenaConfig {
    /// 6-handed tables, 100 big blind stacks, reseating every 9 hands.
    pub fn new(rules: TableRules, hands_per_player: usize, seed: u64) -> Self {
        Self {
            rules,
            table_size: 6,
            hands_per_player,
            reseat_every: 9,
            starting_bb: 100,
            seed,
        }
    }
}

/// One player's totals over a session.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerResult {
    /// The player's name.
    pub name: String,
    /// Hands played.
    pub hands: u64,
    /// Chips won (negative when losing).
    pub net_chips: i64,
    /// Big blinds won.
    pub net_bb: f64,
    /// Big blinds won per 100 hands.
    pub bb_per_100: f64,
    /// Standard error of `bb_per_100`. Roughly 95% of the time the true win
    /// rate is within two standard errors.
    pub std_error: f64,
    /// How often its bot gave no or an illegal action.
    pub fallbacks: u32,
    /// VPIP, PFR, fold-to-bet and aggression counts.
    pub stats: SeatStats,
}

/// Results of a session, best win rate first.
#[derive(Clone, Debug, PartialEq)]
pub struct ArenaResult {
    /// Total hands dealt across all tables.
    pub hands_dealt: u64,
    /// One entry per player, sorted by `bb_per_100`, highest first.
    pub players: Vec<PlayerResult>,
}

/// Progress reported while a session runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArenaProgress {
    /// Fewest hands any player has played so far.
    pub hands_per_player: u64,
    /// Target from the config.
    pub target: u64,
    /// Hands dealt so far across all tables.
    pub hands_dealt: u64,
}

struct Tally {
    hands: u64,
    net: i64,
    sum_bb: f64,
    sum_sq_bb: f64,
    fallbacks: u32,
    stats: SeatStats,
}

/// Splits `n` players into tables of at most `size` seats, as evenly as
/// possible.
fn table_sizes(n: usize, size: usize) -> Vec<usize> {
    let tables = n.div_ceil(size);
    (0..tables)
        .map(|t| n / tables + usize::from(t < n % tables))
        .collect()
}

/// Runs a session: players (with their names) are shuffled into tables,
/// play `reseat_every` hands with the button moving and every stack reset
/// to `starting_bb` big blinds, then get reshuffled, until each has played
/// `hands_per_player` hands. `progress` is called after every round of
/// tables.
///
/// Bots keep their state between hands and tables, so bots that learn or
/// tilt carry it with them. Errors if fewer than 2 players are given, the
/// table size is outside 2–10, or a table would end up with one player.
pub fn run_arena(
    config: &ArenaConfig,
    players: &mut [(String, Box<dyn Bot>)],
    mut progress: impl FnMut(&ArenaProgress),
) -> Result<ArenaResult, PlayError> {
    let n = players.len();
    if n < 2 || !(2..=MAX_PLAYERS).contains(&config.table_size) {
        return Err(PlayError::InvalidPlayerCount);
    }
    let sizes = table_sizes(n, config.table_size);
    if sizes.contains(&1) {
        return Err(PlayError::InvalidPlayerCount);
    }
    let big_blind = config.rules.big_blind;
    let stack = config.starting_bb * big_blind;
    let reseat_every = config.reseat_every.max(1);
    let mut rng = StdRng::seed_from_u64(config.seed);
    let mut tallies: Vec<Tally> = (0..n)
        .map(|_| Tally {
            hands: 0,
            net: 0,
            sum_bb: 0.0,
            sum_sq_bb: 0.0,
            fallbacks: 0,
            stats: SeatStats::default(),
        })
        .collect();
    let mut hands_dealt = 0u64;
    let target = config.hands_per_player as u64;

    while tallies.iter().map(|t| t.hands).min().unwrap_or(0) < target {
        // Random seating for this round.
        let mut order: Vec<usize> = (0..n).collect();
        for i in 0..n {
            let j = rng.random_range(i..n);
            order.swap(i, j);
        }
        let mut start = 0;
        for &size in &sizes {
            let seats = &order[start..start + size];
            start += size;
            let first_button = rng.random_range(0..size);
            for h in 0..reseat_every {
                let deal = Deal::random(
                    config.rules.variant,
                    size,
                    Some(rng.random_range(0..u64::MAX)),
                )?;
                let button = (first_button + h) % size;
                let mut hand = Hand::new(config.rules, &vec![stack; size], button, deal)?;

                // Borrow this table's bots, in seat order.
                let mut table: Vec<Option<&mut Box<dyn Bot>>> =
                    players.iter_mut().map(|(_, b)| Some(b)).collect();
                let mut seated: Vec<&mut dyn Bot> = Vec::with_capacity(size);
                for &p in seats {
                    if let Some(bot) = table[p].take() {
                        seated.push(&mut **bot);
                    }
                }
                let outcome = play_hand(&mut hand, &mut seated)?;
                hands_dealt += 1;

                let mut model = OpponentModel::new();
                model.record(hand.events(), size);
                for (seat, &p) in seats.iter().enumerate() {
                    let t = &mut tallies[p];
                    let net = outcome.result.net[seat];
                    let bb = net as f64 / big_blind as f64;
                    t.hands += 1;
                    t.net += net;
                    t.sum_bb += bb;
                    t.sum_sq_bb += bb * bb;
                    t.fallbacks += outcome.fallbacks[seat];
                    t.stats.add(&model.seat(seat));
                }
            }
        }
        progress(&ArenaProgress {
            hands_per_player: tallies.iter().map(|t| t.hands).min().unwrap_or(0),
            target,
            hands_dealt,
        });
    }

    let mut results: Vec<PlayerResult> = players
        .iter()
        .zip(&tallies)
        .map(|((name, _), t)| {
            let hands = t.hands.max(1) as f64;
            let mean = t.sum_bb / hands;
            let variance = (t.sum_sq_bb / hands - mean * mean).max(0.0);
            PlayerResult {
                name: name.clone(),
                hands: t.hands,
                net_chips: t.net,
                net_bb: t.sum_bb,
                bb_per_100: mean * 100.0,
                std_error: 100.0 * (variance / hands).sqrt(),
                fallbacks: t.fallbacks,
                stats: t.stats,
            }
        })
        .collect();
    results.sort_by(|a, b| b.bb_per_100.total_cmp(&a.bb_per_100));
    Ok(ArenaResult {
        hands_dealt,
        players: results,
    })
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_table_sizes() {
        assert_eq!(table_sizes(12, 6), vec![6, 6]);
        assert_eq!(table_sizes(15, 6), vec![5, 5, 5]);
        assert_eq!(table_sizes(7, 6), vec![4, 3]);
        assert_eq!(table_sizes(6, 6), vec![6]);
        assert_eq!(table_sizes(3, 2), vec![2, 1]);
    }
}
