//! Multi-table tournaments: random seating, blind levels, eliminations,
//! balancing and breaking tables, and hand-for-hand play on the bubble.
//!
//! [`Tournament`] is pure game logic over several [`Table`]s, with no clock
//! and no I/O. The host drives it:
//!
//! 1. [`Tournament::new_hand`] deals the next hand at a table, at the current
//!    level's blinds.
//! 2. Play the hand through [`Tournament::table_mut`] (`act`, `advance`).
//! 3. [`Tournament::finish_hand`] once it's over: it records who busted and
//!    their places, then moves players to keep the tables balanced (and
//!    breaks tables) and reports what moved.
//!
//! [`Tournament::set_level`] raises the blinds; each table picks the new level
//! up at its next hand. Players can't be removed. One who isn't there is
//! marked absent ([`Tournament::set_absent`]): they're still dealt in, post
//! blinds and antes, and check or fold when it's their turn ("blinded off").
//!
//! A player who sits down at a table that's been playing (moved there, or a
//! late entry) isn't dealt in on the button or the small blind, so no one
//! plays an orbit without paying a big blind ([`Table::arrive`]).
//!
//! [`Tournament::snapshot`] saves the whole tournament, hands in progress
//! included, and [`Tournament::restore`] carries on from it exactly as the
//! original would, e.g. after a host or server restarts.
//!
//! ```
//! use ducy_play::{Entrant, Level, Tournament, TournamentConfig, Variant, BettingStructure};
//! use ducy_play::bots::CallingStation;
//!
//! let config = TournamentConfig {
//!     variant: Variant::Holdem,
//!     structure: BettingStructure::NoLimit,
//!     table_size: 6,
//!     starting_stack: 100,
//!     levels: vec![Level::new(5, 10, 0), Level::new(25, 50, 5)],
//!     paid: 2,
//!     seed: 7,
//! };
//! let players = (0..10)
//!     .map(|i| Entrant::bot(format!("p{i}"), format!("P{i}"), Box::new(CallingStation)))
//!     .collect();
//! let mut t = Tournament::new(config, players).unwrap();
//! assert_eq!(t.table_ids().len(), 2);
//! t.set_level(1);
//! while !t.is_over() {
//!     for id in t.table_ids() {
//!         if !t.can_deal(id) {
//!             continue;
//!         }
//!         t.new_hand(id).unwrap();
//!         let table = t.table_mut(id).unwrap();
//!         while table.advance().unwrap() {}
//!         t.finish_hand(id).unwrap();
//!     }
//! }
//! assert_eq!(t.finishes().len(), 9); // everyone but the winner
//! assert_eq!(t.winner().unwrap().place, 1);
//! ```

use rand::{RngExt, SeedableRng, rngs::StdRng};

use crate::{
    BettingStructure, Bot, MAX_PLAYERS, PlayError, TOURNAMENT_SNAPSHOT_VERSION, Table, TableRules,
    TableSeat, TournamentSnapshot, Variant,
};

/// One blind level. Amounts are whole chips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Level {
    pub small_blind: u64,
    pub big_blind: u64,
    /// Posted by every player dealt in.
    pub ante: u64,
}

impl Level {
    pub fn new(small_blind: u64, big_blind: u64, ante: u64) -> Self {
        Self {
            small_blind,
            big_blind,
            ante,
        }
    }
}

/// How a tournament is played.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TournamentConfig {
    pub variant: Variant,
    pub structure: BettingStructure,
    /// Seats per table, 2 to [`MAX_PLAYERS`].
    pub table_size: usize,
    /// Chips for every entry, re-entries included.
    pub starting_stack: u64,
    /// The blind levels in order. The last one stays once it's reached.
    pub levels: Vec<Level>,
    /// Places paid. Hand-for-hand play starts with one more player left than
    /// this, while there's more than one table. 0 turns it off.
    pub paid: usize,
    /// Seeds the seating and the deals, so a tournament can be replayed.
    pub seed: u64,
}

/// Someone entering the tournament: a person, or a bot that plays their seat.
pub struct Entrant {
    pub id: String,
    pub name: String,
    pub bot: Option<Box<dyn Bot>>,
}

impl Entrant {
    pub fn person(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            bot: None,
        }
    }

    pub fn bot(id: impl Into<String>, name: impl Into<String>, bot: Box<dyn Bot>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            bot: Some(bot),
        }
    }

    fn into_seat(self) -> TableSeat {
        match self.bot {
            Some(bot) => TableSeat::with_bot(self.name, self.id, bot),
            None => TableSeat::human(self.name, self.id),
        }
    }
}

/// Where someone sits: a table id and a seat at it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SeatRef {
    pub table: usize,
    pub seat: usize,
}

/// A finished entry: who, and the place they finished in (1 = winner).
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Finish {
    pub id: String,
    pub name: String,
    pub place: usize,
}

/// A player moved between tables.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Move {
    pub id: String,
    pub from: SeatRef,
    pub to: SeatRef,
}

/// What happened after a hand: eliminations, moves and broken tables.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct HandReport {
    /// Players knocked out this hand, best place first.
    pub busted: Vec<Finish>,
    /// Players moved to other tables, in order.
    pub moves: Vec<Move>,
    /// Tables closed this hand.
    pub broken: Vec<usize>,
    /// Set once one player has all the chips.
    pub winner: Option<Finish>,
}

/// One player still in, for standings.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Standing {
    pub id: String,
    pub name: String,
    pub stack: u64,
    pub at: SeatRef,
    pub absent: bool,
}

/// A multi-table tournament. See the [module docs](self).
pub struct Tournament {
    config: TournamentConfig,
    /// Indexed by table id; `None` once a table is broken.
    tables: Vec<Option<Table>>,
    level: usize,
    /// Knocked-out entries, in the order they went out (worst place first).
    finishes: Vec<Finish>,
    winner: Option<Finish>,
    entries: usize,
    hand_for_hand: bool,
    /// Hands each table has dealt since hand-for-hand began.
    hfh_hands: Vec<u64>,
    /// The last hand number `finish_hand` took in, per table.
    finished: Vec<u64>,
    /// Random choices made so far (seats for new players and moves): each
    /// comes from its own generator, seeded from the config's seed and this
    /// count, so a saved tournament goes on exactly as it would have.
    draws: u64,
}

impl Tournament {
    /// Seats `entrants` at random across as few tables as hold them, as
    /// evenly as possible.
    pub fn new(config: TournamentConfig, entrants: Vec<Entrant>) -> Result<Self, PlayError> {
        if !(2..=MAX_PLAYERS).contains(&config.table_size) || entrants.len() < 2 {
            return Err(PlayError::InvalidPlayerCount);
        }
        if config.starting_stack == 0 {
            return Err(PlayError::InvalidSetup);
        }
        if config.levels.is_empty() {
            return Err(PlayError::InvalidBlinds);
        }
        let mut t = Self {
            tables: Vec::new(),
            level: 0,
            finishes: Vec::new(),
            winner: None,
            entries: 0,
            hand_for_hand: false,
            hfh_hands: Vec::new(),
            finished: Vec::new(),
            draws: 0,
            config,
        };
        for l in &t.config.levels {
            t.rules_for(*l).validate()?;
        }
        let n_tables = entrants.len().div_ceil(t.config.table_size);
        for _ in 0..n_tables {
            t.open_table()?;
        }
        let mut entrants = entrants;
        shuffle(&mut entrants, &mut t.rng());
        for (i, e) in entrants.into_iter().enumerate() {
            t.seat_entrant(i % n_tables, e)?;
        }
        Ok(t)
    }

    /// A generator for the next random choice (see `draws`).
    fn rng(&mut self) -> StdRng {
        self.draws += 1;
        StdRng::seed_from_u64(self.config.seed ^ self.draws.wrapping_mul(0xD1B5_4A32_D192_ED03))
    }

    fn rules_for(&self, level: Level) -> TableRules {
        TableRules {
            variant: self.config.variant,
            structure: self.config.structure,
            small_blind: level.small_blind,
            big_blind: level.big_blind,
            ante: level.ante,
        }
    }

    fn open_table(&mut self) -> Result<usize, PlayError> {
        let id = self.tables.len();
        let seats = (0..self.config.table_size)
            .map(|_| TableSeat::empty())
            .collect();
        let seed = self.config.seed ^ (id as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let rules = self.rules_for(self.current_level());
        // The buy-in only has to cover the big blind for Table::new; seats
        // get their real stacks as players sit down.
        let mut table = Table::new(
            rules,
            seats,
            self.config.starting_stack.max(rules.big_blind),
            seed,
        )?;
        table.set_top_up(false);
        for s in 0..self.config.table_size {
            table.set_stack(s, 0)?;
        }
        self.tables.push(Some(table));
        self.hfh_hands.push(0);
        self.finished.push(0);
        Ok(id)
    }

    /// Seats a new entry at a random empty seat of table `id`.
    fn seat_entrant(&mut self, id: usize, e: Entrant) -> Result<SeatRef, PlayError> {
        let stack = self.config.starting_stack;
        let seat = self
            .random_empty_seat(id)
            .ok_or(PlayError::InvalidPlayerCount)?;
        let table = self.tables[id].as_mut().expect("live table");
        *table.seat_mut(seat) = e.into_seat();
        table.set_stack(seat, stack)?;
        arrive(table, seat);
        self.entries += 1;
        Ok(SeatRef { table: id, seat })
    }

    fn random_empty_seat(&mut self, id: usize) -> Option<usize> {
        let table = self.tables[id].as_ref()?;
        let empty: Vec<usize> = (0..table.num_seats())
            .filter(|&s| table.seat(s).is_empty())
            .collect();
        if empty.is_empty() {
            None
        } else {
            Some(empty[self.rng().random_range(0..empty.len())])
        }
    }

    pub fn config(&self) -> &TournamentConfig {
        &self.config
    }

    /// The current level's index into the config's levels.
    pub fn level(&self) -> usize {
        self.level
    }

    pub fn current_level(&self) -> Level {
        self.config.levels[self.level.min(self.config.levels.len() - 1)]
    }

    /// Moves to blind level `level` (capped at the last). Each table uses it
    /// from its next hand.
    pub fn set_level(&mut self, level: usize) {
        self.level = level.min(self.config.levels.len() - 1);
    }

    /// The ids of the tables still in play.
    pub fn table_ids(&self) -> Vec<usize> {
        (0..self.tables.len())
            .filter(|&i| self.tables[i].is_some())
            .collect()
    }

    pub fn table(&self, id: usize) -> Option<&Table> {
        self.tables.get(id)?.as_ref()
    }

    /// For playing a hand (`act`, `advance`). Don't change seats or stacks
    /// through it; the tournament does that.
    pub fn table_mut(&mut self, id: usize) -> Option<&mut Table> {
        self.tables.get_mut(id)?.as_mut()
    }

    /// Players seated at table `id`.
    pub fn players_at(&self, id: usize) -> usize {
        self.table(id).map_or(0, |t| {
            (0..t.num_seats())
                .filter(|&s| !t.seat(s).is_empty())
                .count()
        })
    }

    /// Players still in the tournament.
    pub fn players_left(&self) -> usize {
        self.table_ids().iter().map(|&id| self.players_at(id)).sum()
    }

    /// Entries so far, re-entries included.
    pub fn entries(&self) -> usize {
        self.entries
    }

    /// Every chip in play: each entry's starting stack.
    pub fn total_chips(&self) -> u64 {
        self.entries as u64 * self.config.starting_stack
    }

    /// Knocked-out entries, worst place first.
    pub fn finishes(&self) -> &[Finish] {
        &self.finishes
    }

    pub fn winner(&self) -> Option<&Finish> {
        self.winner.as_ref()
    }

    pub fn is_over(&self) -> bool {
        self.winner.is_some()
    }

    /// Whether tables are playing hand-for-hand (on the bubble).
    pub fn hand_for_hand(&self) -> bool {
        self.hand_for_hand
    }

    /// Where player `id` sits, if they're still in.
    pub fn find(&self, id: &str) -> Option<SeatRef> {
        self.table_ids().into_iter().find_map(|t| {
            let table = self.table(t)?;
            (0..table.num_seats())
                .find(|&s| !table.seat(s).is_empty() && table.seat(s).id == id)
                .map(|seat| SeatRef { table: t, seat })
        })
    }

    /// Everyone still in, biggest stack first.
    pub fn standings(&self) -> Vec<Standing> {
        let mut v: Vec<Standing> = self
            .table_ids()
            .into_iter()
            .flat_map(|t| {
                let table = self.table(t).expect("live table");
                (0..table.num_seats())
                    .filter(|&s| !table.seat(s).is_empty())
                    .map(move |s| Standing {
                        id: table.seat(s).id.clone(),
                        name: table.seat(s).name.clone(),
                        stack: table.stack(s),
                        at: SeatRef { table: t, seat: s },
                        absent: table.seat(s).human && table.seat(s).away,
                    })
            })
            .collect();
        v.sort_by_key(|s| std::cmp::Reverse(s.stack));
        v
    }

    /// Marks a person as absent (blinded off) or back. Bots are never absent.
    pub fn set_absent(&mut self, id: &str, absent: bool) -> Result<(), PlayError> {
        let at = self.find(id).ok_or(PlayError::InvalidSetup)?;
        let seat = self.tables[at.table]
            .as_mut()
            .expect("live table")
            .seat_mut(at.seat);
        if seat.human {
            seat.away = absent;
        }
        Ok(())
    }

    /// A late registration or a re-entry: a new starting stack at a random
    /// table among those with the fewest players. Opens a new table if every
    /// seat is taken. Fails if `e.id` is still in or the tournament is over.
    pub fn add_entry(&mut self, e: Entrant) -> Result<SeatRef, PlayError> {
        if self.is_over() || self.find(&e.id).is_some() {
            return Err(PlayError::InvalidSetup);
        }
        let ids: Vec<usize> = self
            .table_ids()
            .into_iter()
            .filter(|&t| self.players_at(t) < self.config.table_size)
            .collect();
        let id = match ids.iter().map(|&t| self.players_at(t)).min() {
            Some(fewest) => {
                let shortest: Vec<usize> = ids
                    .into_iter()
                    .filter(|&t| self.players_at(t) == fewest)
                    .collect();
                shortest[self.rng().random_range(0..shortest.len())]
            }
            None => self.open_table()?,
        };
        self.seat_entrant(id, e)
    }

    /// Whether table `id` may deal its next hand now: it's between hands,
    /// has two players, and, hand-for-hand, every other table has caught up.
    pub fn can_deal(&self, id: usize) -> bool {
        let Some(table) = self.table(id) else {
            return false;
        };
        if self.is_over() || table.in_hand() || table.players_in() < 2 {
            return false;
        }
        if self.hand_for_hand {
            let others = self.table_ids().into_iter().filter(|&t| t != id);
            for t in others {
                let other = self.table(t).expect("live table");
                if other.in_hand()
                    || (other.players_in() >= 2 && self.hfh_hands[t] < self.hfh_hands[id])
                {
                    return false;
                }
            }
        }
        // The last hand here has been taken in.
        table.hand_number() == self.finished[id]
    }

    /// Deals the next hand at table `id` with the current level's blinds.
    pub fn new_hand(&mut self, id: usize) -> Result<(), PlayError> {
        if !self.can_deal(id) {
            return Err(PlayError::IllegalAction);
        }
        let rules = self.rules_for(self.current_level());
        let table = self.tables[id].as_mut().expect("live table");
        table.set_rules(rules)?;
        table.new_hand()?;
        if self.hand_for_hand {
            self.hfh_hands[id] += 1;
        }
        Ok(())
    }

    /// Takes in the finished hand at table `id`: knocks out players with no
    /// chips, then balances and breaks tables. Call it once per hand, after
    /// the hand is over.
    pub fn finish_hand(&mut self, id: usize) -> Result<HandReport, PlayError> {
        let table = self.table(id).ok_or(PlayError::InvalidSetup)?;
        let hand = table.hand().ok_or(PlayError::IllegalAction)?;
        let result = hand.result().ok_or(PlayError::IllegalAction)?;
        if table.hand_number() == self.finished[id] {
            return Err(PlayError::IllegalAction);
        }
        let mut report = HandReport::default();

        // Busted players, smallest starting stack first: they finish lowest.
        let mut busted: Vec<(u64, usize)> = table
            .dealt()
            .iter()
            .enumerate()
            .filter(|&(i, _)| result.final_stacks[i] == 0)
            .map(|(i, &seat)| ((result.final_stacks[i] as i64 - result.net[i]) as u64, seat))
            .collect();
        busted.sort();
        let hand_number = table.hand_number();
        let mut left = self.players_left();
        let table = self.tables[id].as_mut().expect("live table");
        let mut out = Vec::new();
        for (_, seat) in busted {
            let s = std::mem::replace(table.seat_mut(seat), TableSeat::empty());
            table.set_stack(seat, 0)?;
            out.push(Finish {
                id: s.id,
                name: s.name,
                place: left,
            });
            left -= 1;
        }
        self.finished[id] = hand_number;
        self.finishes.extend(out.iter().cloned());
        out.reverse();
        report.busted = out;

        if left == 1 {
            let last = self.standings().pop().expect("one player left");
            let f = Finish {
                id: last.id,
                name: last.name,
                place: 1,
            };
            self.winner = Some(f.clone());
            report.winner = Some(f);
            return Ok(report);
        }

        self.rebalance(&mut report)?;

        let multi = self.table_ids().len() > 1;
        if !self.hand_for_hand && multi && self.config.paid > 0 && left == self.config.paid + 1 {
            self.hand_for_hand = true;
            self.hfh_hands.iter_mut().for_each(|h| *h = 0);
        } else if self.hand_for_hand && (!multi || left <= self.config.paid) {
            self.hand_for_hand = false;
        }
        Ok(report)
    }

    /// Breaks tables while the field fits on fewer, then moves players from
    /// the fullest tables to the shortest until no two differ by more than
    /// one. Only tables between hands give up players.
    fn rebalance(&mut self, report: &mut HandReport) -> Result<(), PlayError> {
        loop {
            let ids = self.table_ids();
            let needed = self.players_left().div_ceil(self.config.table_size).max(1);
            if ids.len() > needed {
                // Break the shortest table that isn't mid-hand.
                let Some(&victim) = ids
                    .iter()
                    .filter(|&&t| !self.table(t).expect("live").in_hand())
                    .min_by_key(|&&t| (self.players_at(t), usize::MAX - t))
                else {
                    break;
                };
                let seats: Vec<usize> = {
                    let t = self.table(victim).expect("live");
                    (0..t.num_seats())
                        .filter(|&s| !t.seat(s).is_empty())
                        .collect()
                };
                // Mark it broken first so no one is moved back to it.
                let mut broken = self.tables[victim].take().expect("live");
                for seat in seats {
                    let dest = self
                        .shortest_table_with_room()
                        .expect("room after breaking");
                    let stack = broken.stack(seat);
                    let s = std::mem::replace(broken.seat_mut(seat), TableSeat::empty());
                    let to = self.place(dest, s, stack)?;
                    report.moves.push(Move {
                        id: self.table(to.table).expect("live").seat(to.seat).id.clone(),
                        from: SeatRef {
                            table: victim,
                            seat,
                        },
                        to,
                    });
                }
                report.broken.push(victim);
                continue;
            }
            // Balance: fullest table between hands → shortest table.
            let Some(&src) = ids
                .iter()
                .filter(|&&t| !self.table(t).expect("live").in_hand())
                .max_by_key(|&&t| (self.players_at(t), usize::MAX - t))
            else {
                break;
            };
            // Every table full is balanced too.
            let Some(dest) = self.shortest_table_with_room() else {
                break;
            };
            if self.players_at(src) <= self.players_at(dest) + 1 {
                break;
            }
            // Move the player who would post the big blind next.
            let table = self.tables[src].as_mut().expect("live");
            let seat = table
                .next_big_blind()
                .or_else(|| (0..table.num_seats()).find(|&s| !table.seat(s).is_empty()))
                .expect("a player to move");
            let stack = table.stack(seat);
            let s = std::mem::replace(table.seat_mut(seat), TableSeat::empty());
            table.set_stack(seat, 0)?;
            let id = s.id.clone();
            let to = self.place(dest, s, stack)?;
            report.moves.push(Move {
                id,
                from: SeatRef { table: src, seat },
                to,
            });
        }
        Ok(())
    }

    /// A live table with the fewest players and an empty seat (random among
    /// ties).
    fn shortest_table_with_room(&mut self) -> Option<usize> {
        let ids: Vec<usize> = self
            .table_ids()
            .into_iter()
            .filter(|&t| self.players_at(t) < self.config.table_size)
            .collect();
        let fewest = ids.iter().map(|&t| self.players_at(t)).min()?;
        let shortest: Vec<usize> = ids
            .into_iter()
            .filter(|&t| self.players_at(t) == fewest)
            .collect();
        Some(shortest[self.rng().random_range(0..shortest.len())])
    }

    /// Puts an existing player with `stack` chips in a random empty seat at
    /// table `dest`. They're dealt in from that table's next hand.
    fn place(&mut self, dest: usize, seat: TableSeat, stack: u64) -> Result<SeatRef, PlayError> {
        let s = self
            .random_empty_seat(dest)
            .ok_or(PlayError::InvalidPlayerCount)?;
        let table = self.tables[dest].as_mut().expect("live");
        *table.seat_mut(s) = seat;
        table.set_stack(s, stack)?;
        arrive(table, s);
        Ok(SeatRef {
            table: dest,
            seat: s,
        })
    }
}

impl Tournament {
    /// The whole tournament as saved data (JSON with the `serde` feature): its
    /// tables with the hands being played, stacks, the level, eliminations
    /// and what comes next. [`Self::restore`] carries on from it exactly as
    /// this one would. Bots aren't saved; `restore` asks for them again.
    pub fn snapshot(&self) -> TournamentSnapshot {
        TournamentSnapshot {
            version: TOURNAMENT_SNAPSHOT_VERSION,
            config: self.config.clone(),
            tables: self
                .tables
                .iter()
                .map(|t| t.as_ref().map(Table::snapshot))
                .collect(),
            level: self.level,
            finishes: self.finishes.clone(),
            winner: self.winner.clone(),
            entries: self.entries,
            hand_for_hand: self.hand_for_hand,
            hfh_hands: self.hfh_hands.clone(),
            finished: self.finished.clone(),
            draws: self.draws,
        }
    }

    /// Loads a saved tournament. `bot(id)` gives the bot for each entrant
    /// (by id) who had one; one that can't be given back fails the load, as
    /// does a snapshot of another version or one that doesn't fit together.
    pub fn restore(
        s: &TournamentSnapshot,
        mut bot: impl FnMut(&str) -> Option<Box<dyn Bot>>,
    ) -> Result<Self, PlayError> {
        let bad = PlayError::InvalidSnapshot;
        let c = &s.config;
        if s.version != TOURNAMENT_SNAPSHOT_VERSION
            || !(2..=MAX_PLAYERS).contains(&c.table_size)
            || c.levels.is_empty()
            || s.level >= c.levels.len()
            || s.tables.is_empty()
            || s.hfh_hands.len() != s.tables.len()
            || s.finished.len() != s.tables.len()
        {
            return Err(bad);
        }
        let mut tables = Vec::with_capacity(s.tables.len());
        for t in &s.tables {
            tables.push(match t {
                Some(t) => {
                    if t.seats.len() != c.table_size {
                        return Err(bad);
                    }
                    Some(Table::restore(t, |seat| bot(&t.seats[seat].id))?)
                }
                None => None,
            });
        }
        Ok(Self {
            config: c.clone(),
            tables,
            level: s.level,
            finishes: s.finishes.clone(),
            winner: s.winner.clone(),
            entries: s.entries,
            hand_for_hand: s.hand_for_hand,
            hfh_hands: s.hfh_hands.clone(),
            finished: s.finished.clone(),
            draws: s.draws,
        })
    }
}

/// Someone has just sat in `seat` (a move, a late entry or a re-entry). Once
/// the table has dealt, they aren't dealt in on the button or the small blind
/// (see [`Table::arrive`]), so a newcomer can't play an orbit without paying a
/// big blind; before the first hand everyone starts together.
fn arrive(table: &mut Table, seat: usize) {
    if table.hand_number() > 0 {
        table.arrive(seat);
    }
}

fn shuffle<T>(v: &mut [T], rng: &mut StdRng) {
    for i in (1..v.len()).rev() {
        let j = rng.random_range(0..=i);
        v.swap(i, j);
    }
}
