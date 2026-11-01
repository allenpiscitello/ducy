//! A club's tournaments (ducy-web #457): making one, registering, late
//! registration and re-entries, cancelling, results, and what they pay: the
//! default payouts by entries, a club's payout tables, and a tournament's own.
//!
//! Kept as ducy-web's JSON (`club.tourneys`, `club.payoutTables`), so a club
//! saved before these rules loads as it is. The tables themselves run on
//! [`ducy_play::tourney_runner`].
//!
//! A tournament's club chips move through the money model, which the caller
//! runs (`club_money`): these rules say what to move. A buy-in is taken
//! before the entry is made ([`Tourneys::check_register`], then
//! [`Tourneys::register`]); a refund or prize is paid after.
//!
//! - `buy_in` and `added` are club chips. The pool is every entry's buy-in
//!   plus the added chips, which the club issues when it pays.
//! - `stack` and the structure are tournament chips, nothing to do with
//!   club chips.
//! - Payouts are basis points by place adding up to exactly 10 000.

use std::collections::{BTreeMap, BTreeSet};

use rand::rngs::StdRng;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::rules::{Member, RuleError, Status, a_game, game, random36, refuse};
use ducy_play::structure::Step;

/// The longest tournament name.
pub const MAX_TOURNEY_NAME: usize = 40;
/// The most entries a tournament takes.
pub const MAX_ENTRIES: u64 = 1000;
/// The most chips in a buy-in or added to a pool.
pub const MAX_CHIPS: u64 = 1_000_000_000_000;
/// The most payout tables a club keeps.
pub const MAX_PAYOUT_TABLES: usize = 20;
/// The most rows in a payout table.
pub const MAX_PAYOUT_ROWS: usize = 50;
/// The longest payout table name.
pub const MAX_PAYOUT_NAME: usize = 30;
/// All of a pool, in basis points.
pub const FULL: u64 = 10_000;
/// How long a finished or cancelled tournament stays in members' lobby.
pub const SHOWN_AFTER_MS: u64 = 7 * 86_400_000;

/// Where a tournament is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TourneyStatus {
    /// Waiting to start; registration may be open.
    #[default]
    Scheduled,
    /// Playing.
    Running,
    /// Has a winner, and has paid.
    Finished,
    /// Called off, every buy-in back.
    Cancelled,
}

/// One entry: a member's buy-in. A member's last entry is their current one.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    /// Its number, from 1.
    pub n: u64,
    /// The member's id.
    pub member: String,
    /// When it was made.
    pub at: u64,
    /// Its place, once it's out (1: the winner).
    pub place: Option<u64>,
    /// The order it went out in, for final places.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out: Option<u64>,
    /// Anything else kept on it.
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

/// One row of a payout table: from `from` entries on, until the next row.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PayoutRow {
    /// The entries it starts at.
    pub from: u64,
    /// Each paid place's share, in basis points, first place first.
    pub places: Vec<u64>,
}

/// A tournament's own payouts: a table, or an older single list of shares
/// (a table of one row from 1 entry).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Payout {
    /// Its rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<Vec<PayoutRow>>,
    /// An older single list of shares.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub places: Option<Vec<u64>>,
}

/// A club's payout table (#424), which its tournaments can pay by.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PayoutTable {
    /// Its id.
    pub id: String,
    /// Its name.
    pub name: String,
    /// Its rows, by the entries they start at.
    pub rows: Vec<PayoutRow>,
}

/// A prize paid.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Paid {
    /// The member's id.
    pub member: String,
    /// Their place.
    pub place: u64,
    /// Club chips.
    pub amount: u64,
}

/// A tournament.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Tourney {
    /// Its id.
    pub id: String,
    /// Its name.
    pub name: String,
    /// The game.
    pub game: String,
    /// Each entry's buy-in, in club chips.
    pub buy_in: u64,
    /// Chips the club adds to the pool.
    pub added: u64,
    /// The starting stack, in tournament chips.
    pub stack: u64,
    /// The blind structure.
    pub structure: Vec<Step>,
    /// When registration opens.
    pub opens_at: u64,
    /// When it starts.
    pub starts_at: u64,
    /// The fewest players it starts with.
    pub min_players: u64,
    /// The most players it takes.
    pub max_players: u64,
    /// Seats at each table.
    pub table_size: u64,
    /// Registration and re-entry stay open until the end of this level (0:
    /// none after the start).
    pub late_level: u64,
    /// A player knocked out may buy in again while registration is open.
    pub reentry: bool,
    /// Re-entries per player (0: no limit).
    pub max_reentries: u64,
    /// Its own payouts, or none for the defaults (or a club table).
    pub payout: Option<Payout>,
    /// The club payout table it pays by.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payout_table: Option<String>,
    /// Where it is.
    pub status: TourneyStatus,
    /// Its entries, in order.
    pub entries: Vec<Entry>,
    /// Buy-ins it holds.
    pub held: u64,
    /// When it was made.
    pub created_at: u64,
    /// When it started.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    /// When it finished or was cancelled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
    /// Why it was cancelled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// Knockouts so far.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outs: Option<u64>,
    /// Once finished, the shares it paid by: later edits to its payout
    /// table don't rewrite what it paid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paid_shares: Option<Vec<u64>>,
    /// Once finished, the prizes paid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paid: Option<Vec<Paid>>,
    /// Anything else kept on it (a one-off tournament's, a promotion's).
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

impl Tourney {
    /// A member's current entry.
    pub fn entry_of(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().rev().find(|e| e.member == id)
    }

    /// Registered and not knocked out.
    pub fn is_in(&self, id: &str) -> bool {
        self.entry_of(id).is_some_and(|e| e.place.is_none())
    }

    /// Entries so far, re-entries too.
    pub fn entry_count(&self) -> u64 {
        self.entries.len() as u64
    }

    /// Different players registered.
    pub fn player_count(&self) -> u64 {
        self.entries
            .iter()
            .map(|e| e.member.as_str())
            .collect::<BTreeSet<_>>()
            .len() as u64
    }

    /// The prize pool: every entry's buy-in plus the added chips.
    pub fn pool(&self) -> u64 {
        self.entry_count() * self.buy_in + self.added
    }

    fn over(&self) -> bool {
        matches!(
            self.status,
            TourneyStatus::Finished | TourneyStatus::Cancelled
        )
    }

    fn not_over(&self) -> Result<(), RuleError> {
        if self.over() {
            return refuse("It’s over already.");
        }
        Ok(())
    }

    /// The rows it pays by: its club table's, its own, or none for the
    /// defaults.
    pub fn payout_rows<'a>(&'a self, tables: &'a [PayoutTable]) -> Option<Vec<PayoutRow>> {
        if let Some(id) = &self.payout_table {
            return tables.iter().find(|p| &p.id == id).map(|p| p.rows.clone());
        }
        let p = self.payout.as_ref()?;
        if let Some(rows) = &p.rows {
            return Some(rows.clone());
        }
        p.places.as_ref().map(|places| {
            vec![PayoutRow {
                from: 1,
                places: places.clone(),
            }]
        })
    }

    /// The shares in force: what it paid by once finished; else the row in
    /// force for the entries so far, or the defaults for them.
    pub fn payout_shares(&self, tables: &[PayoutTable]) -> Vec<u64> {
        if let Some(s) = &self.paid_shares {
            return s.clone();
        }
        match self.payout_rows(tables) {
            Some(rows) if !rows.is_empty() => payout_row(&rows, self.entry_count()).places.clone(),
            _ => default_payout(self.entry_count()),
        }
    }

    /// The prizes for the pool by place (first place first), adding up to
    /// exactly the pool: no more places than entries, and what's left over
    /// (rounding, or places beyond the entries) goes to first place.
    pub fn prizes(&self, tables: &[PayoutTable]) -> Vec<u64> {
        let total = self.pool();
        let shares = self.payout_shares(tables);
        let n = shares.len().min(self.entry_count().max(1) as usize).max(1);
        let mut out: Vec<u64> = shares
            .iter()
            .take(n)
            .map(|&bp| (u128::from(total) * u128::from(bp) / u128::from(FULL)) as u64)
            .collect();
        if out.is_empty() {
            out.push(0);
        }
        let given: u64 = out.iter().sum();
        out[0] += total.saturating_sub(given);
        out
    }
}

/// How many places the defaults pay for `entries` entries: about the top 15%.
pub fn paid_places(entries: u64) -> u64 {
    match entries {
        0..=3 => 1,
        4..=6 => 2,
        7..=10 => 3,
        // In floating point, as ducy-web always has (20 entries pay 4).
        n => (n as f64 * 0.15).ceil() as u64,
    }
}

/// The default shares for `entries` entries, in basis points adding up to
/// exactly 10 000: a falling curve, the rounding remainder to first place.
pub fn default_payout(entries: u64) -> Vec<u64> {
    let n = paid_places(entries.max(1)) as usize;
    let w: Vec<f64> = (0..n).map(|i| 1.0 / ((i + 1) as f64).powf(1.2)).collect();
    let sum: f64 = w.iter().sum();
    let mut bp: Vec<u64> = w
        .iter()
        .map(|x| (x / sum * FULL as f64).floor() as u64)
        .collect();
    let given: u64 = bp.iter().sum();
    bp[0] += FULL - given;
    bp
}

/// The row in force for `entries` entries: the last that starts at or
/// below them (below the first row's start, the first row).
pub fn payout_row(rows: &[PayoutRow], entries: u64) -> &PayoutRow {
    rows.iter()
        .rev()
        .find(|r| r.from <= entries)
        .unwrap_or(&rows[0])
}

fn whole(v: Option<&Value>) -> Option<u64> {
    let v = v?;
    v.as_u64().or_else(|| {
        v.as_f64()
            .filter(|f| f.fract() == 0.0 && *f >= 0.0 && *f <= 9e15)
            .map(|f| f as u64)
    })
}

/// Like `whole`, but a negative whole number is still a number (to refuse).
fn int(v: Option<&Value>) -> Option<i64> {
    let v = v?;
    v.as_i64().or_else(|| {
        v.as_f64()
            .filter(|f| f.fract() == 0.0 && f.abs() <= 9e15)
            .map(|f| f as i64)
    })
}

/// `v[k]` unless it's missing or null.
fn given<'a>(v: &'a Value, k: &str) -> Option<&'a Value> {
    v.get(k).filter(|x| !x.is_null())
}

/// Checks one row's shares: whole basis points above 0, none more than the
/// place above, adding up to exactly 10 000.
fn check_places(places: &Value, place: &str) -> Result<Vec<u64>, RuleError> {
    let bad = || refuse(format!("{place}Each paid place needs a share above 0."));
    let Some(list) = places.as_array() else {
        return bad();
    };
    if list.is_empty() || list.len() as u64 > MAX_ENTRIES {
        return bad();
    }
    let mut out = Vec::with_capacity(list.len());
    for p in list {
        match int(Some(p)) {
            Some(p) if p > 0 => out.push(p as u64),
            _ => return bad(),
        }
    }
    if out.windows(2).any(|w| w[1] > w[0]) {
        return refuse(format!(
            "{place}A place can’t pay more than the one above it."
        ));
    }
    let sum: u64 = out.iter().sum();
    if sum != FULL {
        return refuse(format!(
            "{place}The payouts add up to {}.{:02}%, not 100%.",
            sum / 100,
            sum % 100
        ));
    }
    Ok(out)
}

/// Checks a payout table's rows: at least one, each starting at more
/// entries than the one before (from 1), each row's shares as above.
pub fn check_payout_rows(rows: &Value) -> Result<Vec<PayoutRow>, RuleError> {
    let Some(list) = rows.as_array().filter(|l| !l.is_empty()) else {
        return refuse("Add at least one row.");
    };
    if list.len() > MAX_PAYOUT_ROWS {
        return refuse(format!("At most {MAX_PAYOUT_ROWS} rows."));
    }
    let mut out: Vec<PayoutRow> = Vec::with_capacity(list.len());
    for (i, r) in list.iter().enumerate() {
        let from = int(r.get("from"));
        let Some(from) = from.filter(|&f| f >= 1 && f as u64 <= MAX_ENTRIES) else {
            return refuse(format!(
                "Row {}: entries are a whole number from 1 to {MAX_ENTRIES}.",
                i + 1
            ));
        };
        let from = from as u64;
        if out.last().is_some_and(|p| from <= p.from) {
            return refuse("Each row starts at more entries than the one above it.");
        }
        let place = if list.len() > 1 {
            format!("From {from} entries: ")
        } else {
            String::new()
        };
        let places = check_places(r.get("places").unwrap_or(&Value::Null), &place)?;
        out.push(PayoutRow { from, places });
    }
    Ok(out)
}

/// A blind structure's shape (the structure module checks it fully).
fn check_structure(steps: &Value) -> Result<Vec<Step>, RuleError> {
    let none = || refuse("Add at least one blind level.");
    let Some(list) = steps.as_array().filter(|l| !l.is_empty() && l.len() <= 200) else {
        return none();
    };
    let mut out = Vec::with_capacity(list.len());
    for s in list {
        let minutes = int(s.get("minutes")).filter(|&m| m >= 1 && m <= i64::from(u32::MAX));
        let Some(minutes) = minutes else {
            return refuse("Every level needs its minutes.");
        };
        let minutes = minutes as u32;
        if s.get("break")
            .is_some_and(|b| b.as_bool() == Some(true) || b.as_u64() == Some(1))
        {
            out.push(Step::Break {
                is_break: true,
                minutes,
            });
            continue;
        }
        let ante = match given(s, "ante") {
            None => Some(0),
            v => int(v),
        };
        match (int(s.get("sb")), int(s.get("bb")), ante) {
            (Some(sb), Some(bb), Some(ante)) if sb >= 1 && bb >= sb && ante >= 0 => {
                out.push(Step::Level {
                    sb: sb as u64,
                    bb: bb as u64,
                    ante: ante as u64,
                    minutes,
                });
            }
            _ => return refuse("Blinds are whole chips, the big blind at least the small."),
        }
    }
    if !out.iter().any(|s| matches!(s, Step::Level { .. })) {
        return none();
    }
    Ok(out)
}

/// What the caller moves through the money model.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Move {
    /// The member's id.
    pub member: String,
    /// Club chips.
    pub amount: u64,
}

/// A finished tournament's payments.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finish {
    /// The prizes to pay, by place.
    pub paid: Vec<Paid>,
    /// What the club issues for them beyond what the tournament held: the
    /// added chips (less if fewer places were paid).
    pub issued: i64,
}

/// The part of a club these rules keep: its tournaments and payout tables,
/// with its members (read, never changed here).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Tourneys {
    /// The club's code.
    #[serde(skip_serializing)]
    pub code: String,
    /// The club's members, for who may enter and their names.
    #[serde(skip_serializing)]
    pub members: BTreeMap<String, Member>,
    /// Its tournaments.
    pub tourneys: Vec<Tourney>,
    /// Its payout tables.
    pub payout_tables: Vec<PayoutTable>,
}

impl Tourneys {
    /// A tournament by id.
    pub fn get(&self, id: &str) -> Result<&Tourney, RuleError> {
        match self.tourneys.iter().find(|t| t.id == id) {
            Some(t) => Ok(t),
            None => refuse("No such tournament."),
        }
    }

    fn get_mut(&mut self, id: &str) -> Result<&mut Tourney, RuleError> {
        match self.tourneys.iter_mut().find(|t| t.id == id) {
            Some(t) => Ok(t),
            None => refuse("No such tournament."),
        }
    }

    fn member(&self, id: &str) -> Result<(), RuleError> {
        match self.members.get(id) {
            Some(m) if m.is(Status::Member) => Ok(()),
            _ => refuse("Not a member of this club."),
        }
    }

    /// A payout table by id.
    pub fn payout_table(&self, id: &str) -> Option<&PayoutTable> {
        self.payout_tables.iter().find(|p| p.id == id)
    }

    /// The tournaments members can see: not finished or cancelled more than
    /// a week ago.
    pub fn visible(&self, now: u64) -> impl Iterator<Item = &Tourney> {
        self.tourneys.iter().filter(move |t| {
            !t.over() || now.saturating_sub(t.ended_at.unwrap_or(0)) < SHOWN_AFTER_MS
        })
    }

    /// The club's plan allows `most` tournaments a day (#265): at most that
    /// many, not cancelled, starting within any 24 hours. None: no cap.
    pub fn check_limit(&self, starts_at: u64, most: Option<u64>) -> Result<(), RuleError> {
        let Some(most) = most else { return Ok(()) };
        let near = self
            .tourneys
            .iter()
            .filter(|t| {
                t.status != TourneyStatus::Cancelled && t.starts_at.abs_diff(starts_at) < 86_400_000
            })
            .count() as u64;
        if near < most {
            return Ok(());
        }
        if most == 0 {
            return refuse("This club can’t run tournaments. Ask ducy about it.");
        }
        let (what, others) = if most == 1 {
            ("tournament", "another starts".to_string())
        } else {
            ("tournaments", format!("{most} others start"))
        };
        refuse(format!(
            "This club can run {most} {what} a day, and {others} within 24 hours of this one. Pick another time, or ask ducy for more."
        ))
    }

    /// A new tournament from its settings `s` (the form's), or why they
    /// won't do. `most`: tournaments a day the club's plan allows.
    pub fn new_tourney(
        &mut self,
        s: &Value,
        now: u64,
        most: Option<u64>,
        rng: &mut StdRng,
    ) -> Result<Tourney, RuleError> {
        let name: String = given(s, "name")
            .map(|n| match n {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .unwrap_or_default()
            .trim()
            .chars()
            .take(MAX_TOURNEY_NAME)
            .collect();
        let name = if name.is_empty() {
            format!("Tournament {}", self.tourneys.len() + 1)
        } else {
            name
        };
        let game_id = given(s, "game")
            .and_then(Value::as_str)
            .filter(|g| !g.is_empty())
            .unwrap_or("nlhe");
        let Some(g) = game(game_id) else {
            return refuse("Pick a game.");
        };
        let chips = |k: &str| match given(s, k) {
            None => Some(0),
            v => int(v)
                .filter(|&n| n >= 0 && n as u64 <= MAX_CHIPS)
                .map(|n| n as u64),
        };
        let (Some(buy_in), Some(added)) = (chips("buyIn"), chips("added")) else {
            return refuse("The buy-in and added prize pool are 0 or more chips.");
        };
        let Some(stack) = int(given(s, "stack")).filter(|&n| (20..=1_000_000_000).contains(&n))
        else {
            return refuse("The starting stack must be at least 20 chips.");
        };
        let opens_at = match given(s, "opensAt") {
            None => Some(now),
            v => whole(v),
        };
        let (Some(opens_at), Some(starts_at)) = (opens_at, whole(given(s, "startsAt"))) else {
            return refuse("Pick when registration opens and when it starts.");
        };
        if starts_at < opens_at {
            return refuse("It must start after registration opens.");
        }
        self.check_limit(starts_at, most)?;
        let or = |k: &str, d: i64| match given(s, k) {
            None => Some(d),
            v => int(v),
        };
        match (or("minPlayers", 2), or("maxPlayers", 100)) {
            (Some(lo), Some(hi)) if lo >= 2 && hi >= lo && hi as u64 <= MAX_ENTRIES => {}
            _ => return refuse("At least 2 players, and a maximum no smaller than the minimum."),
        }
        let (min_players, max_players) = (
            or("minPlayers", 2).unwrap_or(2) as u64,
            or("maxPlayers", 100).unwrap_or(100) as u64,
        );
        let table_size = or("tableSize", i64::from(g.default_seats));
        let Some(table_size) = table_size.filter(|&n| n >= 2 && n <= i64::from(g.seats)) else {
            return refuse(format!("{} table seats 2 to {}.", a_game(game_id), g.seats));
        };
        let (late_level, max_reentries) = match (or("lateLevel", 0), or("maxReentries", 0)) {
            (Some(l), Some(r)) if l >= 0 && r >= 0 => (l as u64, r as u64),
            _ => return refuse("Late registration is up to a level, from 0."),
        };
        let payout_table = given(s, "payoutTable")
            .and_then(Value::as_str)
            .filter(|p| !p.is_empty())
            .map(str::to_string);
        if let Some(p) = &payout_table
            && self.payout_table(p).is_none()
        {
            return refuse("No such payout table.");
        }
        let structure = check_structure(s.get("structure").unwrap_or(&Value::Null))?;
        let reentry = given(s, "reentry").is_some_and(|r| match r {
            Value::Bool(b) => *b,
            Value::Number(n) => n.as_f64() != Some(0.0),
            Value::String(s) => !s.is_empty(),
            _ => true,
        });
        let t = Tourney {
            id: random36(rng, 6),
            name,
            game: game_id.into(),
            buy_in,
            added,
            stack: stack as u64,
            structure,
            opens_at,
            starts_at,
            min_players,
            max_players,
            table_size: table_size as u64,
            late_level,
            reentry,
            max_reentries,
            payout: None,
            payout_table,
            status: TourneyStatus::Scheduled,
            entries: vec![],
            held: 0,
            created_at: now,
            ..Default::default()
        };
        self.tourneys.push(t.clone());
        Ok(t)
    }

    /// Whether `id` may register for `tid` before the start: registration
    /// open, not registered already, not full. The caller takes the buy-in,
    /// then [`Self::register`]s them.
    pub fn check_register(&self, tid: &str, id: &str, now: u64) -> Result<u64, RuleError> {
        let t = self.get(tid)?;
        self.member(id)?;
        if t.status != TourneyStatus::Scheduled {
            return refuse("Registration is closed.");
        }
        if now < t.opens_at {
            return refuse("Registration isn’t open yet.");
        }
        if t.is_in(id) {
            return refuse("You’re registered already.");
        }
        if t.player_count() >= t.max_players {
            return refuse("The tournament is full.");
        }
        Ok(t.buy_in)
    }

    fn enter(&mut self, tid: &str, id: &str, now: u64) -> Result<Entry, RuleError> {
        let t = self.get_mut(tid)?;
        t.held += t.buy_in;
        let e = Entry {
            n: t.entry_count() + 1,
            member: id.into(),
            at: now,
            ..Default::default()
        };
        t.entries.push(e.clone());
        Ok(e)
    }

    /// Registers `id` for `tid` (#77), its buy-in taken.
    pub fn register(&mut self, tid: &str, id: &str, now: u64) -> Result<Entry, RuleError> {
        self.check_register(tid, id, now)?;
        self.enter(tid, id, now)
    }

    /// Unregisters `id` before the start: the buy-in to give back.
    pub fn unregister(&mut self, tid: &str, id: &str) -> Result<Move, RuleError> {
        self.member(id)?;
        let t = self.get_mut(tid)?;
        if t.status != TourneyStatus::Scheduled {
            return refuse("It has started: you can’t unregister now.");
        }
        let Some(i) = t.entries.iter().position(|e| e.member == id) else {
            return refuse("You aren’t registered.");
        };
        t.entries.remove(i);
        for (k, e) in t.entries.iter_mut().enumerate() {
            e.n = k as u64 + 1;
        }
        t.held = t.held.saturating_sub(t.buy_in);
        Ok(Move {
            member: id.into(),
            amount: t.buy_in,
        })
    }

    /// Whether `id` may register late or re-enter (#80) while `tid` runs and
    /// `level` (from 0, the level being played) is before its last
    /// registration level. The buy-in to take.
    pub fn check_late_entry(&self, tid: &str, id: &str, level: u64) -> Result<u64, RuleError> {
        let t = self.get(tid)?;
        self.member(id)?;
        if t.status != TourneyStatus::Running || level >= t.late_level {
            return refuse("Late registration is closed.");
        }
        let mine = t.entries.iter().filter(|e| e.member == id).count() as u64;
        if mine > 0 {
            if t.is_in(id) {
                return refuse("You’re still in.");
            }
            if !t.reentry {
                return refuse("This tournament has no re-entries.");
            }
            // Re-entries so far: every entry but the first.
            if t.max_reentries > 0 && mine > t.max_reentries {
                let s = if t.max_reentries == 1 { "" } else { "s" };
                return refuse(format!(
                    "You can re-enter {} time{s} at most.",
                    t.max_reentries
                ));
            }
        } else if t.player_count() >= t.max_players {
            return refuse("The tournament is full.");
        }
        Ok(t.buy_in)
    }

    /// A late entry or re-entry, its buy-in taken.
    pub fn late_entry(
        &mut self,
        tid: &str,
        id: &str,
        level: u64,
        now: u64,
    ) -> Result<Entry, RuleError> {
        self.check_late_entry(tid, id, level)?;
        self.enter(tid, id, now)
    }

    /// What to do at the start time: "start", or "cancel" with fewer than
    /// the minimum registered; none before then.
    pub fn at_start(&self, tid: &str, now: u64) -> Result<Option<&'static str>, RuleError> {
        let t = self.get(tid)?;
        if t.status != TourneyStatus::Scheduled || now < t.starts_at {
            return Ok(None);
        }
        Ok(Some(if t.player_count() >= t.min_players {
            "start"
        } else {
            "cancel"
        }))
    }

    /// Cancels a tournament that hasn't finished (also when a running one
    /// can't go on): every buy-in back, to give. The added chips were never
    /// paid.
    pub fn cancel(&mut self, tid: &str, why: &str, now: u64) -> Result<Vec<Move>, RuleError> {
        let members: BTreeSet<String> = self.members.keys().cloned().collect();
        let t = self.get_mut(tid)?;
        t.not_over()?;
        let mut back = vec![];
        for e in &t.entries {
            if members.contains(&e.member) {
                back.push(Move {
                    member: e.member.clone(),
                    amount: t.buy_in,
                });
            }
        }
        t.held = t.held.saturating_sub(t.buy_in * back.len() as u64);
        t.status = TourneyStatus::Cancelled;
        t.why = Some(why.into());
        t.ended_at = Some(now);
        back.retain(|m| m.amount > 0);
        Ok(back)
    }

    /// It starts.
    pub fn start(&mut self, tid: &str, now: u64) -> Result<(), RuleError> {
        let t = self.get_mut(tid)?;
        if t.status != TourneyStatus::Scheduled {
            return refuse("It isn’t waiting to start.");
        }
        t.status = TourneyStatus::Running;
        t.started_at = Some(now);
        Ok(())
    }

    /// `id`'s entry was knocked out, in `place` as things stand. Knockouts
    /// are numbered as they happen: late entries change the count, so final
    /// places are set at the finish.
    pub fn knocked_out(
        &mut self,
        tid: &str,
        id: &str,
        place: u64,
    ) -> Result<Option<Entry>, RuleError> {
        let t = self.get_mut(tid)?;
        let outs = t.outs.unwrap_or(0);
        let Some(e) = t.entries.iter_mut().rev().find(|e| e.member == id) else {
            return Ok(None);
        };
        if e.place.is_none() {
            e.place = Some(place);
            e.out = Some(outs + 1);
            t.outs = Some(outs + 1);
        }
        Ok(Some(e.clone()))
    }

    /// The tournament has a winner: places are final, and the prizes to pay.
    pub fn finish(&mut self, tid: &str, winner: &str, now: u64) -> Result<Finish, RuleError> {
        let tables = self.payout_tables.clone();
        let members: BTreeSet<String> = self.members.keys().cloned().collect();
        let t = self.get_mut(tid)?;
        if t.status != TourneyStatus::Running {
            return refuse("It isn’t running.");
        }
        // Final places: the winner first, then everyone else in the reverse
        // of the order they went out, over all the entries.
        let w = t.entries.iter().rposition(|e| e.member == winner);
        let mut gone: Vec<usize> = (0..t.entries.len())
            .filter(|&i| Some(i) != w && t.entries[i].out.is_some())
            .collect();
        gone.sort_by_key(|&i| t.entries[i].out);
        let count = t.entries.len() as u64;
        for (k, &i) in gone.iter().enumerate() {
            t.entries[i].place = Some(count - k as u64);
        }
        if let Some(w) = w {
            t.entries[w].place = Some(1);
        }
        t.paid_shares = Some(t.payout_shares(&tables));
        let amounts = t.prizes(&tables);
        let mut paid = vec![];
        for (i, &amount) in amounts.iter().enumerate() {
            let place = i as u64 + 1;
            let Some(e) = t.entries.iter().find(|e| e.place == Some(place)) else {
                continue;
            };
            if amount == 0 || !members.contains(&e.member) {
                continue;
            }
            paid.push(Paid {
                member: e.member.clone(),
                place,
                amount,
            });
        }
        let out: u64 = paid.iter().map(|p| p.amount).sum();
        let issued = out as i64 - t.held as i64;
        t.held = 0;
        t.status = TourneyStatus::Finished;
        t.ended_at = Some(now);
        t.paid = Some(paid.clone());
        Ok(Finish { paid, issued })
    }

    /// The tournament's own payouts: a table `{rows}`, a list of shares (a
    /// table of one row from 1 entry), or null for the defaults. It then
    /// pays by no club table.
    pub fn set_payout(&mut self, tid: &str, payout: &Value) -> Result<(), RuleError> {
        let p = match payout {
            Value::Null => None,
            Value::Array(_) => Some(Payout {
                rows: Some(vec![PayoutRow {
                    from: 1,
                    places: check_places(payout, "")?,
                }]),
                places: None,
            }),
            other => Some(Payout {
                rows: Some(check_payout_rows(
                    other.get("rows").unwrap_or(&Value::Null),
                )?),
                places: None,
            }),
        };
        let t = self.get_mut(tid)?;
        t.not_over()?;
        t.payout_table = None;
        t.payout = p;
        Ok(())
    }

    /// The tournament pays by club table `id`, or (none) the defaults.
    pub fn set_payout_table(&mut self, tid: &str, id: Option<&str>) -> Result<(), RuleError> {
        if let Some(id) = id
            && self.payout_table(id).is_none()
        {
            self.get(tid)?.not_over()?;
            return refuse("No such payout table.");
        }
        let t = self.get_mut(tid)?;
        t.not_over()?;
        t.payout_table = id.map(str::to_string);
        t.payout = None;
        Ok(())
    }

    fn table_name(&self, name: &Value, except: Option<&str>) -> Result<String, RuleError> {
        let raw = match name {
            Value::Null => String::new(),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let n: String = raw
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(MAX_PAYOUT_NAME)
            .collect();
        let n = n.trim().to_string();
        if n.is_empty() {
            return refuse("Name the payout table.");
        }
        if n.to_lowercase() == "default" {
            return refuse("“Default” is ducy’s payouts: pick another name.");
        }
        let taken = self
            .payout_tables
            .iter()
            .any(|p| Some(p.id.as_str()) != except && p.name.to_lowercase() == n.to_lowercase());
        if taken {
            return refuse(format!("There’s a payout table called {n} already."));
        }
        Ok(n)
    }

    /// A new club payout table from `{name, rows}`.
    pub fn add_payout_table(
        &mut self,
        s: &Value,
        rng: &mut StdRng,
    ) -> Result<PayoutTable, RuleError> {
        if self.payout_tables.len() >= MAX_PAYOUT_TABLES {
            return refuse(format!(
                "A club has at most {MAX_PAYOUT_TABLES} payout tables."
            ));
        }
        let name = self.table_name(s.get("name").unwrap_or(&Value::Null), None)?;
        let rows = check_payout_rows(s.get("rows").unwrap_or(&Value::Null))?;
        let p = PayoutTable {
            id: random36(rng, 6),
            name,
            rows,
        };
        self.payout_tables.push(p.clone());
        Ok(p)
    }

    /// Renames a club payout table and/or changes its rows. Tournaments
    /// still to finish that pay by it follow; finished ones keep what they
    /// paid.
    pub fn edit_payout_table(&mut self, id: &str, s: &Value) -> Result<PayoutTable, RuleError> {
        if self.payout_table(id).is_none() {
            return refuse("No such payout table.");
        }
        let name = match given(s, "name") {
            None => None,
            Some(n) => Some(self.table_name(n, Some(id))?),
        };
        let rows = match given(s, "rows") {
            None => None,
            Some(r) => Some(check_payout_rows(r)?),
        };
        let p = self
            .payout_tables
            .iter_mut()
            .find(|p| p.id == id)
            .expect("found above");
        if let Some(n) = name {
            p.name = n;
        }
        if let Some(r) = rows {
            p.rows = r;
        }
        Ok(p.clone())
    }

    /// The tournaments still to finish that pay by club table `id`.
    pub fn tourneys_using(&self, id: &str) -> Vec<&Tourney> {
        self.tourneys
            .iter()
            .filter(|t| t.payout_table.as_deref() == Some(id) && !t.over())
            .collect()
    }

    /// Deletes a club payout table, unless a tournament still to finish
    /// pays by it.
    pub fn delete_payout_table(&mut self, id: &str) -> Result<PayoutTable, RuleError> {
        let Some(p) = self.payout_table(id).cloned() else {
            return refuse("No such payout table.");
        };
        let using = self.tourneys_using(id);
        if !using.is_empty() {
            let names: Vec<&str> = using.iter().map(|t| t.name.as_str()).collect();
            let (pays, them) = if using.len() == 1 {
                ("pays", "it")
            } else {
                ("pay", "them")
            };
            return refuse(format!(
                "{} {pays} by {}: pick another payout for {them} first.",
                names.join(", "),
                p.name
            ));
        }
        self.payout_tables.retain(|x| x.id != id);
        Ok(p)
    }

    /// What members see of the club's tournaments (the lobby), with how
    /// running ones are going (`live`, by tournament id, from the host).
    pub fn body(&self, live: &Map<String, Value>, now: u64) -> Value {
        let name = |id: &str| {
            self.members
                .get(id)
                .map_or("?".to_string(), |m| m.name.clone())
        };
        let list: Vec<Value> = self
            .visible(now)
            .map(|t| {
                let mut players: Vec<&str> = vec![];
                for e in &t.entries {
                    if !players.contains(&e.member.as_str()) {
                        players.push(&e.member);
                    }
                }
                let n = t.entry_count().max(1) as usize;
                let shares: Vec<u64> = t.payout_shares(&self.payout_tables).into_iter().take(n).collect();
                let mut v = json!({
                    "id": t.id, "name": t.name, "game": t.game, "buyIn": t.buy_in, "added": t.added, "stack": t.stack,
                    "opensAt": t.opens_at, "startsAt": t.starts_at, "minPlayers": t.min_players, "maxPlayers": t.max_players,
                    "tableSize": t.table_size, "lateLevel": t.late_level, "reentry": t.reentry, "maxReentries": t.max_reentries,
                    "status": t.status, "why": t.why.clone().unwrap_or_default(),
                    "entries": t.entry_count(),
                    "players": players.iter().map(|id| json!({"id": id, "name": name(id)})).collect::<Vec<_>>(),
                    "in": t.entries.iter().filter(|e| e.place.is_none()).map(|e| e.member.clone()).collect::<Vec<_>>(),
                    "places": t.entries.iter().filter_map(|e| e.place.map(|p| json!({"id": e.member, "name": name(&e.member), "place": p}))).collect::<Vec<_>>(),
                    "pool": t.pool(), "prizes": t.prizes(&self.payout_tables), "shares": shares, "structure": t.structure,
                });
                if let Some(l) = live.get(&t.id) {
                    v["live"] = l.clone();
                }
                v
            })
            .collect();
        json!({"type": "tourneys", "club": self.code, "tourneys": list})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn rng() -> StdRng {
        StdRng::seed_from_u64(7)
    }

    fn levels() -> Value {
        json!([{"sb": 10, "bb": 20, "ante": 0, "minutes": 10}, {"sb": 20, "bb": 40, "minutes": 10}, {"break": true, "minutes": 5}, {"sb": 50, "bb": 100, "ante": 10, "minutes": 10}])
    }

    fn club(n: usize) -> Tourneys {
        let mut c = Tourneys {
            code: "ABCD-EFGH".into(),
            ..Default::default()
        };
        for i in 0..n {
            let id = format!("p{i}");
            let m: Member = serde_json::from_value(
                json!({"id": id, "name": id.to_uppercase(), "status": "member"}),
            )
            .unwrap();
            c.members.insert(id, m);
        }
        c
    }

    fn make(c: &mut Tourneys, s: Value) -> String {
        let mut s = s;
        if s.get("structure").is_none() {
            s["structure"] = levels();
        }
        if s.get("stack").is_none() {
            s["stack"] = json!(1500);
        }
        c.new_tourney(&s, 0, None, &mut rng()).unwrap().id
    }

    fn err(r: Result<impl std::fmt::Debug, RuleError>) -> String {
        r.unwrap_err().0
    }

    #[test]
    fn registering_unregistering_and_the_pool() {
        let mut c = club(3);
        let t = make(
            &mut c,
            json!({"name": "Sunday", "buyIn": 1000, "added": 500, "opensAt": 100, "startsAt": 1000}),
        );
        assert!(err(c.register(&t, "p0", 50)).contains("isn’t open yet"));
        for id in ["p0", "p1", "p2"] {
            assert_eq!(c.check_register(&t, id, 200), Ok(1000));
            c.register(&t, id, 200).unwrap();
        }
        assert!(err(c.register(&t, "p0", 200)).contains("registered already"));
        assert_eq!(c.get(&t).unwrap().pool(), 3500);
        assert_eq!(c.get(&t).unwrap().held, 3000);
        let back = c.unregister(&t, "p2").unwrap();
        assert_eq!(
            back,
            Move {
                member: "p2".into(),
                amount: 1000
            }
        );
        let tt = c.get(&t).unwrap();
        assert_eq!((tt.pool(), tt.entry_count(), tt.held), (2500, 2, 2000));
        assert_eq!(
            tt.entries.iter().map(|e| e.n).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert!(err(c.register(&t, "p9", 200)).contains("Not a member"));
    }

    #[test]
    fn below_the_minimum_its_cancelled_with_every_buy_in_back() {
        let mut c = club(3);
        let t = make(
            &mut c,
            json!({"buyIn": 700, "opensAt": 0, "startsAt": 1000, "minPlayers": 4}),
        );
        for id in ["p0", "p1", "p2"] {
            c.register(&t, id, 10).unwrap();
        }
        assert_eq!(c.at_start(&t, 999).unwrap(), None);
        assert_eq!(c.at_start(&t, 1000).unwrap(), Some("cancel"));
        let back = c.cancel(&t, "not enough players", 1000).unwrap();
        assert_eq!(back.len(), 3);
        assert!(back.iter().all(|m| m.amount == 700));
        let tt = c.get(&t).unwrap();
        assert_eq!((tt.status, tt.held), (TourneyStatus::Cancelled, 0));
        assert!(err(c.cancel(&t, "", 1000)).contains("over already"));
    }

    #[test]
    fn registration_closes_at_the_maximum() {
        let mut c = club(5);
        let t = make(
            &mut c,
            json!({"opensAt": 0, "startsAt": 1000, "maxPlayers": 4}),
        );
        for id in ["p0", "p1", "p2", "p3"] {
            c.register(&t, id, 10).unwrap();
        }
        assert!(err(c.register(&t, "p4", 10)).contains("full"));
        assert_eq!(c.at_start(&t, 1000).unwrap(), Some("start"));
    }

    #[test]
    fn default_payouts_add_up_exactly_for_any_field() {
        for n in 1..=300u64 {
            let bp = default_payout(n);
            assert_eq!(bp.iter().sum::<u64>(), FULL, "{n} entries");
            assert!(bp.windows(2).all(|w| w[1] <= w[0]), "{n}: falling");
            for buy_in in [0, 1, 333, 1000, 12345] {
                let t = Tourney {
                    entries: vec![Entry::default(); n as usize],
                    buy_in,
                    added: 77,
                    ..Default::default()
                };
                assert_eq!(
                    t.prizes(&[]).iter().sum::<u64>(),
                    n * buy_in + 77,
                    "{n} at {buy_in}"
                );
            }
        }
        // The curve, as ducy-web has always paid it.
        assert_eq!(default_payout(3), vec![10000]);
        assert_eq!(default_payout(5), vec![6968, 3032]);
        assert_eq!(default_payout(10), vec![5873, 2556, 1571]);
        assert_eq!(
            default_payout(50),
            vec![4290, 1865, 1146, 812, 621, 499, 414, 353]
        );
    }

    #[test]
    fn a_custom_payout_that_doesnt_add_up_cant_be_saved() {
        let mut c = club(2);
        let t = make(&mut c, json!({"opensAt": 0, "startsAt": 1000}));
        assert!(err(c.set_payout(&t, &json!([6000, 3000]))).contains("90.00%"));
        assert!(err(c.set_payout(&t, &json!([3000, 7000]))).contains("more than the one above"));
        assert!(err(c.set_payout(&t, &json!([10000, 0]))).contains("above 0"));
        c.set_payout(&t, &json!([7000, 3000])).unwrap();
        let rows = c.get(&t).unwrap().payout.clone().unwrap().rows.unwrap();
        assert_eq!(
            rows,
            vec![PayoutRow {
                from: 1,
                places: vec![7000, 3000]
            }]
        );
        c.set_payout(&t, &Value::Null).unwrap();
        assert_eq!(c.get(&t).unwrap().payout, None);
    }

    #[test]
    fn prizes_go_to_the_winners_and_the_added_chips_are_issued() {
        let mut c = club(10);
        let t = make(
            &mut c,
            json!({"buyIn": 1000, "added": 2000, "opensAt": 0, "startsAt": 100}),
        );
        let ids: Vec<String> = (0..10).map(|i| format!("p{i}")).collect();
        for id in &ids {
            c.register(&t, id, 10).unwrap();
        }
        c.set_payout(&t, &json!([5000, 3000, 2000])).unwrap();
        c.start(&t, 100).unwrap();
        for (k, id) in ids[1..].iter().rev().enumerate() {
            c.knocked_out(&t, id, 10 - k as u64).unwrap();
        }
        let f = c.finish(&t, "p0", 500).unwrap();
        let got: Vec<(&str, u64, u64)> = f
            .paid
            .iter()
            .map(|p| (p.member.as_str(), p.place, p.amount))
            .collect();
        assert_eq!(got, vec![("p0", 1, 6000), ("p1", 2, 3600), ("p2", 3, 2400)]);
        assert_eq!(f.issued, 2000);
        let tt = c.get(&t).unwrap();
        assert_eq!((tt.status, tt.held), (TourneyStatus::Finished, 0));
        assert_eq!(tt.paid_shares, Some(vec![5000, 3000, 2000]));
        let body = c.body(&Map::new(), 600);
        let first = body["tourneys"][0]["places"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["place"] == 1)
            .unwrap()
            .clone();
        assert_eq!(first["id"], "p0");
    }

    #[test]
    fn late_registration_and_re_entries_close_after_the_level() {
        let mut c = club(4);
        let t = make(
            &mut c,
            json!({"buyIn": 100, "opensAt": 0, "startsAt": 100, "lateLevel": 2, "reentry": true, "maxReentries": 1}),
        );
        for id in ["p0", "p1", "p2"] {
            c.register(&t, id, 10).unwrap();
        }
        c.start(&t, 100).unwrap();
        c.late_entry(&t, "p3", 1, 200).unwrap();
        assert_eq!(c.get(&t).unwrap().entry_count(), 4);
        assert!(err(c.late_entry(&t, "p0", 1, 200)).contains("still in"));
        c.knocked_out(&t, "p0", 4).unwrap();
        c.late_entry(&t, "p0", 1, 300).unwrap();
        assert_eq!(c.get(&t).unwrap().pool(), 500);
        c.knocked_out(&t, "p0", 4).unwrap();
        assert!(err(c.late_entry(&t, "p0", 1, 400)).contains("1 time at most"));
        c.knocked_out(&t, "p1", 3).unwrap();
        assert!(err(c.late_entry(&t, "p1", 2, 500)).contains("closed"));
    }

    #[test]
    fn settings_are_checked() {
        let mut c = club(1);
        let mut e = |s: Value| err(c.new_tourney(&s, 0, None, &mut rng()));
        assert!(
            e(json!({"opensAt": 0, "startsAt": 5, "stack": 10, "structure": levels()}))
                .contains("starting stack")
        );
        assert!(
            e(json!({"opensAt": 0, "startsAt": 5, "stack": 1500, "structure": []}))
                .contains("blind level")
        );
        assert!(
            e(json!({"opensAt": 10, "startsAt": 5, "stack": 1500, "structure": levels()}))
                .contains("after registration opens")
        );
        assert!(e(json!({"opensAt": 0, "startsAt": 5, "stack": 1500, "structure": levels(), "minPlayers": 5, "maxPlayers": 3})).contains("maximum"));
        assert!(e(json!({"opensAt": 0, "startsAt": 5, "stack": 1500, "structure": levels(), "buyIn": -1})).contains("buy-in"));
        assert!(e(json!({"opensAt": 0, "startsAt": 5, "stack": 1500, "structure": levels(), "game": "plo6", "tableSize": 9})).contains("An Omaha 6 table seats 2 to 6."));
    }

    #[test]
    fn a_plan_limits_tournaments_a_day() {
        let mut c = club(0);
        let h = 3_600_000u64;
        let day = 86_400_000u64;
        let mk = |c: &mut Tourneys, at: u64, most: Option<u64>| {
            c.new_tourney(
                &json!({"stack": 1500, "structure": levels(), "opensAt": 0, "startsAt": at}),
                0,
                most,
                &mut rng(),
            )
        };
        let first = mk(&mut c, 10 * day, Some(1)).unwrap().id;
        assert!(err(mk(&mut c, 10 * day + 23 * h, Some(1))).contains("1 tournament a day"));
        assert!(err(mk(&mut c, 10 * day - 5 * h, Some(1))).contains("1 tournament a day"));
        mk(&mut c, 12 * day, Some(1)).unwrap();
        c.cancel(&first, "test", 0).unwrap();
        mk(&mut c, 10 * day + 2 * h, Some(1)).unwrap();
        mk(&mut c, 20 * day, Some(2)).unwrap();
        mk(&mut c, 20 * day + h, Some(2)).unwrap();
        assert!(err(mk(&mut c, 20 * day + 2 * h, Some(2))).contains("2 tournaments a day"));
        assert!(err(mk(&mut c, 40 * day, Some(0))).contains("can’t run tournaments"));
        mk(&mut c, 20 * day + 3 * h, None).unwrap();
    }

    #[test]
    fn payout_tables_by_entries() {
        let mut c = club(0);
        let rows = json!([{"from": 1, "places": [10000]}, {"from": 10, "places": [6000, 4000]}]);
        let p = c
            .add_payout_table(
                &json!({"name": "  Club   standard ", "rows": rows}),
                &mut rng(),
            )
            .unwrap();
        assert_eq!(p.name, "Club standard");
        assert!(
            err(c.add_payout_table(&json!({"name": "club STANDARD", "rows": rows}), &mut rng()))
                .contains("called club STANDARD already")
        );
        assert!(
            err(c.add_payout_table(&json!({"name": "Default", "rows": rows}), &mut rng()))
                .contains("ducy’s payouts")
        );
        assert!(err(c.add_payout_table(&json!({"name": "X", "rows": [{"from": 5, "places": [10000]}, {"from": 5, "places": [10000]}]}), &mut rng())).contains("more entries than the one above"));
        assert!(err(c.add_payout_table(&json!({"name": "X", "rows": [{"from": 1, "places": [10000]}, {"from": 5, "places": [5000]}]}), &mut rng())).contains("From 5 entries: The payouts add up to 50.00%"));
        let t = make(
            &mut c,
            json!({"opensAt": 0, "startsAt": 1000, "payoutTable": p.id}),
        );
        let tt = c.get(&t).unwrap();
        assert_eq!(tt.payout_shares(&c.payout_tables), vec![10000]);
        assert_eq!(payout_row(&p.rows, 9).from, 1);
        assert_eq!(payout_row(&p.rows, 10).from, 10);
        // A tournament still to finish pays by it: it can't go.
        assert!(
            err(c.delete_payout_table(&p.id))
                .contains("pays by Club standard: pick another payout for it first.")
        );
        c.set_payout_table(&t, None).unwrap();
        c.edit_payout_table(&p.id, &json!({"name": "Renamed"}))
            .unwrap();
        assert_eq!(c.delete_payout_table(&p.id).unwrap().name, "Renamed");
        assert!(err(c.set_payout_table(&t, Some("nope"))).contains("No such payout table."));
    }

    #[test]
    fn a_saved_tourney_loads_and_saves_the_same() {
        let saved = json!({
            "id": "abc123", "name": "Sunday", "game": "nlhe", "buyIn": 1000, "added": 0, "stack": 1500,
            "structure": [{"sb": 10, "bb": 20, "ante": 0, "minutes": 10}, {"break": true, "minutes": 5}],
            "opensAt": 0, "startsAt": 100, "minPlayers": 2, "maxPlayers": 100, "tableSize": 9, "lateLevel": 0,
            "reentry": false, "maxReentries": 0, "payout": null, "status": "finished",
            "entries": [{"n": 1, "member": "p0", "at": 5, "place": 1}, {"n": 2, "member": "p1", "at": 6, "place": 2, "out": 1}],
            "held": 0, "createdAt": 0, "startedAt": 100, "endedAt": 900, "outs": 1, "paidShares": [10000],
            "paid": [{"member": "p0", "place": 1, "amount": 2000}], "promo": {"x": 1}
        });
        let t: Tourney = serde_json::from_value(saved.clone()).unwrap();
        assert_eq!(serde_json::to_value(&t).unwrap(), saved);
    }
}
