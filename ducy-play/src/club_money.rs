//! A club's money: who backs whom, for how much, and every chip that moves
//! between them (ducy-web #454, the club money model).
//!
//! Each member plays through a backer: the club, or an agent who is a member
//! too. The pair is a [`Link`] with one balance and one credit limit. Above
//! zero the backer owes the member; below zero the member owes the backer.
//! Every chain of links leads up to the club, and a member's path is the
//! chain they play through, from them up to [`CLUB`].
//!
//! - **Giving and taking back** move one link: a backer gives chips (a
//!   promise to pay) or takes them back, with a note saying why and how it
//!   was paid. That's how links are settled.
//! - **Buy-ins, top-ups and tournament buy-ins** move every link on the path
//!   down by the same amount, all or nothing. None may take a link below
//!   minus its credit, or go through a link that is overdue.
//! - **Cash-outs, refunds and prizes** move every link on the path back up.
//! - **Rake and time** come off the table, so no link moves for them. What a
//!   player contributed is shared up their path when they stand up: each
//!   link gets its [share](Link::share) of it.
//! - **Agencies** are agents run by several people. Their players are
//!   served by whoever is on shift, through a link with that person; at a
//!   shift change a seated player's session is [handed over](Money::handover).
//!
//! The [ledger](Money::ledger) has one [`Entry`] per link per move. A move
//! along a path is one transaction, so its entries share a `tx`.
//!
//! Amounts are whole hundredths of a chip, as in ducy-web. There's one
//! currency and no exchange.

use std::collections::BTreeMap;
use std::fmt;

/// The club's id as a backer.
pub const CLUB: &str = "club";
/// The most chips one move can be.
pub const MAX_CHIPS: u64 = 1_000_000_000_000;
/// The longest note kept on an entry, in characters.
pub const MAX_NOTE: usize = 80;
/// A share of all of a player's rake, in hundredths of a percent.
pub const FULL_SHARE: u32 = 10_000;

const DAY_MS: i64 = 86_400_000;
const MAX_DEPTH: usize = 64;

/// When a backer expects a link below zero to be paid back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "camelCase"))]
pub enum Terms {
    /// Within this long of going below zero.
    Within {
        /// Milliseconds.
        ms: u64,
    },
    /// By the end of the next settle day after going below zero.
    Weekly {
        /// The settle day, 0 for Sunday to 6 for Saturday, in the club's time.
        day: u8,
    },
}

/// A member and their backer.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct Link {
    /// The member.
    pub member: String,
    /// Their backer: the club or an agent.
    pub backer: String,
    /// Above zero the backer owes the member; below zero the member owes.
    pub balance: i64,
    /// How far below zero buy-ins may take the balance.
    pub credit: u64,
    /// When what's owed must be paid back. None: whenever the two choose.
    pub terms: Option<Terms>,
    /// The member's share of the rake from everyone they play for and
    /// everyone under them, in hundredths of a percent. None: the backer's
    /// default.
    pub share: Option<u32>,
    /// When the balance went below zero, while it is.
    pub below_since: Option<u64>,
}

impl Link {
    fn new(member: &str, backer: &str) -> Self {
        Self {
            member: member.into(),
            backer: backer.into(),
            balance: 0,
            credit: 0,
            terms: None,
            share: None,
            below_since: None,
        }
    }

    /// How much lower buy-ins may take the balance.
    pub fn room(&self) -> u64 {
        (self.balance + self.credit as i64).max(0) as u64
    }
}

/// An agent run by several people.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct Agency {
    /// The people who run it, each with their own link to it.
    pub people: Vec<String>,
    /// Who serves its players now. None: the agency's own links do.
    pub on_shift: Option<String>,
}

/// A member's or agent's settings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct Account {
    /// The backer they play through.
    pub through: Option<String>,
    /// The rake share for links under them that don't set one.
    pub default_share: Option<u32>,
    /// Set when this is an agency, not a person.
    pub agency: Option<Agency>,
}

/// A seated player's chips at a table.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct Session {
    /// The table.
    pub about: String,
    /// The path the chips came down, from the player up to the club.
    pub path: Vec<String>,
    /// Chips in front of them.
    pub stack: u64,
    /// Chips brought to the table in all.
    pub put: u64,
}

/// What an entry was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum EntryKind {
    /// A backer gave chips.
    Give,
    /// A backer took chips back.
    TakeBack,
    /// Chips to a table.
    BuyIn,
    /// More chips to a table.
    TopUp,
    /// Part of a top-up the table handed back, or a tournament buy-in back.
    Refund,
    /// Chips back from a table.
    CashOut,
    /// A tournament buy-in.
    TourneyBuyIn,
    /// A prize from the club: a tournament's or a promotion's.
    Prize,
    /// A share of the rake a player contributed.
    RakeShare,
    /// A prize taken back: up the path, as the prize came down it.
    Void,
    /// A link's balance when the club moved to links.
    Opening,
}

impl EntryKind {
    /// A move along a whole path.
    pub fn is_path_move(self) -> bool {
        !matches!(
            self,
            Self::Give | Self::TakeBack | Self::RakeShare | Self::Opening
        )
    }
}

/// One change to one link.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct Entry {
    /// Its number, from 1.
    pub n: u64,
    /// When, in milliseconds.
    pub at: u64,
    /// The transaction it belongs to.
    pub tx: u64,
    /// The link's member.
    pub member: String,
    /// The link's backer.
    pub backer: String,
    /// What it was.
    pub kind: EntryKind,
    /// The change to the balance.
    pub amount: i64,
    /// The balance after.
    pub balance: i64,
    /// Why, and how it was paid.
    pub note: Option<String>,
    /// What it was for: a table, a tournament, a promotion.
    pub about: Option<String>,
}

/// Why a move was refused. As JSON, `{code, …}`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(tag = "code", rename_all = "camelCase"))]
pub enum MoneyError {
    /// Not a whole number of chips from 1 to [`MAX_CHIPS`].
    Amount,
    /// A share over [`FULL_SHARE`].
    Share,
    /// `club` is the club's id, or a member was paired with themselves.
    BadId,
    /// There's no such link.
    NoLink {
        /// The member.
        member: String,
        /// The backer.
        backer: String,
    },
    /// The link is there already.
    LinkExists,
    /// The link would make someone their own agent's agent.
    Loop,
    /// They have no backer to play through.
    NoBacker {
        /// Who.
        member: String,
    },
    /// Agencies don't play.
    Agency {
        /// The agency.
        member: String,
    },
    /// Not one of the agency's people.
    NotInAgency,
    /// A link on the path has too little room.
    Room {
        /// The link's member.
        member: String,
        /// The link's backer.
        backer: String,
        /// How much lower it may go.
        room: u64,
    },
    /// A link on the path is overdue.
    Overdue {
        /// The link's member.
        member: String,
        /// The link's backer.
        backer: String,
        /// When it was due.
        since: u64,
    },
    /// Taking back more than the member is owed.
    TakeBack {
        /// What the backer owes them.
        owed: u64,
    },
    /// Already seated.
    Seated,
    /// Not seated.
    NotSeated,
    /// Not something the club pays.
    Kind,
}

impl fmt::Display for MoneyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Amount => write!(f, "Enter a whole number of chips above 0."),
            Self::Share => write!(f, "A share is at most 100%."),
            Self::BadId => write!(f, "Pick someone else."),
            Self::NoLink { member, backer } => write!(f, "{member} doesn't play through {backer}."),
            Self::LinkExists => write!(f, "They're linked already."),
            Self::Loop => write!(f, "That would make them their own agent's agent."),
            Self::NoBacker { member } => write!(f, "{member} has no one to play through."),
            Self::Agency { member } => write!(f, "{member} is an agency."),
            Self::NotInAgency => write!(f, "Not one of the agency's people."),
            Self::Room {
                member,
                backer,
                room,
            } => {
                write!(f, "{member}'s link with {backer} has room for {room}.")
            }
            Self::Overdue { member, backer, .. } => {
                write!(f, "{member}'s link with {backer} is overdue.")
            }
            Self::TakeBack { owed } => write!(f, "They're owed {owed}."),
            Self::Seated => write!(f, "Already seated."),
            Self::NotSeated => write!(f, "Not seated."),
            Self::Kind => write!(f, "The club pays prizes and refunds."),
        }
    }
}

impl std::error::Error for MoneyError {}

/// A club's links, sessions and ledger.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct Money {
    /// The club's offset from UTC, in minutes, for settle days.
    pub utc_offset_min: i32,
    links: Vec<Link>,
    accounts: BTreeMap<String, Account>,
    sessions: BTreeMap<String, Session>,
    ledger: Vec<Entry>,
    next_tx: u64,
}

fn amount(a: u64) -> Result<i64, MoneyError> {
    if a == 0 || a > MAX_CHIPS {
        return Err(MoneyError::Amount);
    }
    Ok(a as i64)
}

fn note(s: &str) -> Option<String> {
    let s: String = s.trim().chars().take(MAX_NOTE).collect();
    (!s.is_empty()).then_some(s)
}

fn share(s: Option<u32>) -> Result<Option<u32>, MoneyError> {
    match s {
        Some(s) if s > FULL_SHARE => Err(MoneyError::Share),
        _ => Ok(s),
    }
}

impl Money {
    /// No links yet; settle days in the club's time, `utc_offset_min` from UTC.
    pub fn new(utc_offset_min: i32) -> Self {
        Self {
            utc_offset_min,
            ..Self::default()
        }
    }

    /// Every link.
    pub fn links(&self) -> &[Link] {
        &self.links
    }

    /// The link between `member` and `backer`.
    pub fn link(&self, member: &str, backer: &str) -> Option<&Link> {
        self.links
            .iter()
            .find(|l| l.member == member && l.backer == backer)
    }

    fn link_mut(&mut self, member: &str, backer: &str) -> Result<&mut Link, MoneyError> {
        self.links
            .iter_mut()
            .find(|l| l.member == member && l.backer == backer)
            .ok_or_else(|| MoneyError::NoLink {
                member: member.into(),
                backer: backer.into(),
            })
    }

    /// `member`'s links up to their backers.
    pub fn backers_of(&self, member: &str) -> impl Iterator<Item = &Link> {
        self.links.iter().filter(move |l| l.member == member)
    }

    /// The links of everyone `backer` backs.
    pub fn members_of(&self, backer: &str) -> impl Iterator<Item = &Link> {
        self.links.iter().filter(move |l| l.backer == backer)
    }

    /// A member's or agent's settings.
    pub fn account(&self, id: &str) -> Option<&Account> {
        self.accounts.get(id)
    }

    /// A seated player's session.
    pub fn session(&self, member: &str) -> Option<&Session> {
        self.sessions.get(member)
    }

    /// Every entry, oldest first.
    pub fn ledger(&self) -> &[Entry] {
        &self.ledger
    }

    /// The entries of one link after entry `since`.
    pub fn entries(&self, member: &str, backer: &str, since: u64) -> Vec<&Entry> {
        self.ledger
            .iter()
            .filter(|e| e.n > since && e.member == member && e.backer == backer)
            .collect()
    }

    fn reaches(&self, from: &str, to: &str) -> bool {
        let mut todo = vec![from];
        let mut seen = Vec::new();
        while let Some(at) = todo.pop() {
            if at == to {
                return true;
            }
            if seen.contains(&at) {
                continue;
            }
            seen.push(at);
            todo.extend(self.backers_of(at).map(|l| l.backer.as_str()));
        }
        false
    }

    /// Links `member` to `backer`, with no credit. Their first backer is the
    /// one they play through. Refused if it would make a loop.
    pub fn add_link(&mut self, member: &str, backer: &str) -> Result<&Link, MoneyError> {
        if member == CLUB || member == backer || member.is_empty() || backer.is_empty() {
            return Err(MoneyError::BadId);
        }
        if self.link(member, backer).is_some() {
            return Err(MoneyError::LinkExists);
        }
        if self.reaches(backer, member) {
            return Err(MoneyError::Loop);
        }
        self.links.push(Link::new(member, backer));
        let a = self.accounts.entry(member.into()).or_default();
        a.through.get_or_insert_with(|| backer.into());
        if backer != CLUB {
            self.accounts.entry(backer.into()).or_default();
        }
        Ok(self.links.last().unwrap())
    }

    /// The backer sets how far below zero the link may go. Lowering it below
    /// what's owed is allowed: there's just no room until it's paid.
    pub fn set_credit(
        &mut self,
        member: &str,
        backer: &str,
        credit: u64,
    ) -> Result<(), MoneyError> {
        if credit > MAX_CHIPS {
            return Err(MoneyError::Amount);
        }
        self.link_mut(member, backer)?.credit = credit;
        Ok(())
    }

    /// The backer sets when what's owed must be paid back.
    pub fn set_terms(
        &mut self,
        member: &str,
        backer: &str,
        terms: Option<Terms>,
    ) -> Result<(), MoneyError> {
        if let Some(Terms::Weekly { day }) = terms {
            if day > 6 {
                return Err(MoneyError::BadId);
            }
        }
        self.link_mut(member, backer)?.terms = terms;
        Ok(())
    }

    /// The backer sets the member's rake share. None: the backer's default.
    pub fn set_share(
        &mut self,
        member: &str,
        backer: &str,
        s: Option<u32>,
    ) -> Result<(), MoneyError> {
        self.link_mut(member, backer)?.share = share(s)?;
        Ok(())
    }

    /// The rake share for links under `backer` that set none.
    pub fn set_default_share(&mut self, backer: &str, s: Option<u32>) -> Result<(), MoneyError> {
        let s = share(s)?;
        self.accounts
            .entry(backer.into())
            .or_default()
            .default_share = s;
        Ok(())
    }

    /// Which backer `member` plays through. Not while seated.
    pub fn set_through(&mut self, member: &str, backer: &str) -> Result<(), MoneyError> {
        if self.link(member, backer).is_none() {
            return Err(MoneyError::NoLink {
                member: member.into(),
                backer: backer.into(),
            });
        }
        if self.sessions.contains_key(member) {
            return Err(MoneyError::Seated);
        }
        self.accounts.entry(member.into()).or_default().through = Some(backer.into());
        Ok(())
    }

    /// Makes `id` an agency. It must be linked up already.
    pub fn make_agency(&mut self, id: &str) -> Result<(), MoneyError> {
        if id == CLUB || self.backers_of(id).next().is_none() {
            return Err(MoneyError::NoBacker { member: id.into() });
        }
        let a = self.accounts.entry(id.into()).or_default();
        a.agency.get_or_insert_with(Agency::default);
        Ok(())
    }

    /// Adds `person` to `agency`'s people, linking them to it.
    pub fn add_to_agency(&mut self, agency: &str, person: &str) -> Result<(), MoneyError> {
        if self
            .accounts
            .get(agency)
            .and_then(|a| a.agency.as_ref())
            .is_none()
        {
            return Err(MoneyError::NotInAgency);
        }
        if self.link(person, agency).is_none() {
            self.add_link(person, agency)?;
        }
        let ag = self
            .accounts
            .get_mut(agency)
            .unwrap()
            .agency
            .as_mut()
            .unwrap();
        if !ag.people.iter().any(|p| p == person) {
            ag.people.push(person.into());
        }
        Ok(())
    }

    /// Who serves `agency`'s players now (the shift schedule's pick).
    /// Seated players move over at their next [handover](Self::handover).
    pub fn set_on_shift(&mut self, agency: &str, person: Option<&str>) -> Result<(), MoneyError> {
        let ag = self
            .accounts
            .get_mut(agency)
            .and_then(|a| a.agency.as_mut())
            .ok_or(MoneyError::NotInAgency)?;
        if let Some(p) = person {
            if !ag.people.iter().any(|x| x == p) {
                return Err(MoneyError::NotInAgency);
            }
        }
        ag.on_shift = person.map(Into::into);
        Ok(())
    }

    fn is_agency(&self, id: &str) -> bool {
        self.accounts.get(id).is_some_and(|a| a.agency.is_some())
    }

    // Who `at` plays through next: their backer, or the person on shift when
    // that's an agency and `at` isn't one of its people.
    fn next_up(&self, at: &str) -> Result<String, MoneyError> {
        let through = self
            .accounts
            .get(at)
            .and_then(|a| a.through.clone())
            .ok_or_else(|| MoneyError::NoBacker { member: at.into() })?;
        if let Some(ag) = self.accounts.get(&through).and_then(|a| a.agency.as_ref()) {
            if let Some(p) = &ag.on_shift {
                if p != at && !ag.people.iter().any(|x| x == at) {
                    return Ok(p.clone());
                }
            }
        }
        Ok(through)
    }

    /// The path `member` plays through now, from them up to the club.
    pub fn path(&self, member: &str) -> Result<Vec<String>, MoneyError> {
        if self.is_agency(member) {
            return Err(MoneyError::Agency {
                member: member.into(),
            });
        }
        let mut path = vec![member.to_string()];
        while path.last().unwrap() != CLUB {
            if path.len() > MAX_DEPTH {
                return Err(MoneyError::Loop);
            }
            let next = self.next_up(path.last().unwrap())?;
            if path.contains(&next) {
                return Err(MoneyError::Loop);
            }
            path.push(next);
        }
        Ok(path)
    }

    /// When the link became overdue, if it is at `now`.
    pub fn overdue(&self, link: &Link, now: u64) -> Option<u64> {
        let due = self.due(link.below_since?, link.terms?);
        (now >= due).then_some(due)
    }

    fn due(&self, since: u64, terms: Terms) -> u64 {
        match terms {
            Terms::Within { ms } => since.saturating_add(ms),
            Terms::Weekly { day } => {
                let off = self.utc_offset_min as i64 * 60_000;
                let d = (since as i64 + off).div_euclid(DAY_MS);
                // Day 0, 1 January 1970, was a Thursday.
                let ahead = (day as i64 - (d + 4).rem_euclid(7)).rem_euclid(7);
                ((d + ahead + 1) * DAY_MS - off).max(0) as u64
            }
        }
    }

    /// How much `member` can bring to a table now: the least room on their
    /// path, or 0 when a link on it is overdue.
    pub fn room(&self, member: &str, now: u64) -> Result<u64, MoneyError> {
        let path = self.path(member)?;
        Ok(self.check(&path, 0, now).unwrap_or(0))
    }

    // The least room on `path`, if `want` fits it and nothing is overdue.
    fn check(&self, path: &[String], want: u64, now: u64) -> Result<u64, MoneyError> {
        let mut least = u64::MAX;
        for w in path.windows(2) {
            let fresh = Link::new(&w[0], &w[1]);
            let l = self.link(&w[0], &w[1]).unwrap_or(&fresh);
            if let Some(since) = self.overdue(l, now) {
                return Err(MoneyError::Overdue {
                    member: w[0].clone(),
                    backer: w[1].clone(),
                    since,
                });
            }
            if l.room() < want {
                return Err(MoneyError::Room {
                    member: w[0].clone(),
                    backer: w[1].clone(),
                    room: l.room(),
                });
            }
            least = least.min(l.room());
        }
        Ok(least)
    }

    fn record(
        &mut self,
        (member, backer): (&str, &str),
        kind: EntryKind,
        change: i64,
        (tx, now): (u64, u64),
        why: Option<String>,
        about: Option<&str>,
    ) -> Entry {
        if self.link(member, backer).is_none() {
            // A player's first time served by someone on an agency's shift.
            self.links.push(Link::new(member, backer));
        }
        let n = self.ledger.len() as u64 + 1;
        let l = self.link_mut(member, backer).unwrap();
        l.balance += change;
        if l.balance < 0 {
            l.below_since.get_or_insert(now);
        } else {
            l.below_since = None;
        }
        let e = Entry {
            n,
            at: now,
            tx,
            member: member.into(),
            backer: backer.into(),
            kind,
            amount: change,
            balance: l.balance,
            note: why,
            about: about.map(Into::into),
        };
        self.ledger.push(e.clone());
        e
    }

    fn tx(&mut self) -> u64 {
        self.next_tx += 1;
        self.next_tx
    }

    // Moves every link on `path` by `change`.
    fn move_path(
        &mut self,
        path: &[String],
        kind: EntryKind,
        change: i64,
        now: u64,
        about: Option<&str>,
    ) -> Vec<Entry> {
        let tx = self.tx();
        path.windows(2)
            .map(|w| self.record((&w[0], &w[1]), kind, change, (tx, now), None, about))
            .collect()
    }

    /// The backer gives `member` chips: a promise to pay, or what the member
    /// paid them, noted. The person on an agency's shift can give its
    /// players chips on a link of their own.
    pub fn give(
        &mut self,
        backer: &str,
        member: &str,
        amt: u64,
        why: &str,
        now: u64,
    ) -> Result<Entry, MoneyError> {
        let a = amount(amt)?;
        let serving = self.path(member).is_ok_and(|p| p[1] == backer);
        if !serving {
            self.link_mut(member, backer)?;
        }
        let tx = self.tx();
        Ok(self.record(
            (member, backer),
            EntryKind::Give,
            a,
            (tx, now),
            note(why),
            None,
        ))
    }

    /// The backer takes back chips they owe `member`, as they pay them, noted.
    pub fn take_back(
        &mut self,
        backer: &str,
        member: &str,
        amt: u64,
        why: &str,
        now: u64,
    ) -> Result<Entry, MoneyError> {
        let a = amount(amt)?;
        let owed = self.link_mut(member, backer)?.balance.max(0);
        if a > owed {
            return Err(MoneyError::TakeBack { owed: owed as u64 });
        }
        let tx = self.tx();
        Ok(self.record(
            (member, backer),
            EntryKind::TakeBack,
            -a,
            (tx, now),
            note(why),
            None,
        ))
    }

    /// Sits `member` at a table with `amt` club chips, down their path.
    pub fn buy_in(
        &mut self,
        member: &str,
        table: &str,
        amt: u64,
        now: u64,
    ) -> Result<Vec<Entry>, MoneyError> {
        let a = amount(amt)?;
        if self.sessions.contains_key(member) {
            return Err(MoneyError::Seated);
        }
        let path = self.path(member)?;
        self.check(&path, amt, now)?;
        let out = self.move_path(&path, EntryKind::BuyIn, -a, now, Some(table));
        self.sessions.insert(
            member.into(),
            Session {
                about: table.into(),
                path,
                stack: amt,
                put: amt,
            },
        );
        Ok(out)
    }

    /// Adds `amt` to a seated player's stack, down the session's path.
    pub fn top_up(&mut self, member: &str, amt: u64, now: u64) -> Result<Vec<Entry>, MoneyError> {
        let a = amount(amt)?;
        let s = self.sessions.get(member).ok_or(MoneyError::NotSeated)?;
        let (path, table) = (s.path.clone(), s.about.clone());
        self.check(&path, amt, now)?;
        let out = self.move_path(&path, EntryKind::TopUp, -a, now, Some(&table));
        let s = self.sessions.get_mut(member).unwrap();
        s.stack += amt;
        s.put += amt;
        Ok(out)
    }

    /// The table handed back `amt` of a top-up; it goes back up the path.
    pub fn refund_top_up(
        &mut self,
        member: &str,
        amt: u64,
        now: u64,
    ) -> Result<Vec<Entry>, MoneyError> {
        let a = amount(amt)?;
        let s = self.sessions.get_mut(member).ok_or(MoneyError::NotSeated)?;
        s.stack = s.stack.saturating_sub(amt);
        s.put = s.put.saturating_sub(amt);
        let (path, table) = (s.path.clone(), s.about.clone());
        Ok(self.move_path(&path, EntryKind::Refund, a, now, Some(&table)))
    }

    /// The table reports a seated player's stack.
    pub fn set_stack(&mut self, member: &str, stack: u64) -> Result<(), MoneyError> {
        self.sessions
            .get_mut(member)
            .ok_or(MoneyError::NotSeated)?
            .stack = stack;
        Ok(())
    }

    /// Stands `member` up: their stack goes back up the session's path, and
    /// each link gets its share of the rake and time they `contributed`.
    /// With nothing left, the cash-out is still recorded, at 0, so the
    /// ledger shows the session ended.
    pub fn cash_out(
        &mut self,
        member: &str,
        contributed: u64,
        now: u64,
    ) -> Result<Vec<Entry>, MoneyError> {
        let s = self.sessions.remove(member).ok_or(MoneyError::NotSeated)?;
        if s.stack == 0 {
            let mut out = self.move_path(&s.path, EntryKind::CashOut, 0, now, Some(&s.about));
            out.extend(self.rake_shares(&s.path, contributed, now, &s.about));
            return Ok(out);
        }
        Ok(self.close(&s, contributed, now))
    }

    fn close(&mut self, s: &Session, contributed: u64, now: u64) -> Vec<Entry> {
        let mut out = Vec::new();
        if s.stack > 0 {
            out = self.move_path(
                &s.path,
                EntryKind::CashOut,
                s.stack as i64,
                now,
                Some(&s.about),
            );
        }
        out.extend(self.rake_shares(&s.path, contributed, now, &s.about));
        out
    }

    /// Each link's share of a player's rake on `path`: the link's share, or
    /// its backer's default, and never more than the backer's own.
    pub fn shares(&self, path: &[String]) -> Vec<u32> {
        let mut cap = FULL_SHARE;
        let mut out: Vec<u32> = path
            .windows(2)
            .rev()
            .map(|w| {
                let set = self.link(&w[0], &w[1]).and_then(|l| l.share);
                let dflt = self.accounts.get(&w[1]).and_then(|a| a.default_share);
                cap = set.or(dflt).unwrap_or(0).min(cap);
                cap
            })
            .collect();
        out.reverse();
        out
    }

    fn rake_shares(
        &mut self,
        path: &[String],
        contributed: u64,
        now: u64,
        about: &str,
    ) -> Vec<Entry> {
        if contributed == 0 {
            return Vec::new();
        }
        let shares = self.shares(path);
        let tx = self.tx();
        let mut out = Vec::new();
        for (w, s) in path.windows(2).zip(shares) {
            let a = (contributed as u128 * s as u128 / FULL_SHARE as u128) as i64;
            if a > 0 {
                out.push(self.record(
                    (&w[0], &w[1]),
                    EntryKind::RakeShare,
                    a,
                    (tx, now),
                    None,
                    Some(about),
                ));
            }
        }
        out
    }

    /// After the hand in play at a shift change: when `member`'s path has
    /// changed, closes the session on the old path at their stack and opens
    /// it on the new one at the same stack. Never refused for room: the
    /// chips are on the table already. Empty when nothing changed.
    pub fn handover(
        &mut self,
        member: &str,
        contributed: u64,
        now: u64,
    ) -> Result<Vec<Entry>, MoneyError> {
        let old = self
            .sessions
            .get(member)
            .ok_or(MoneyError::NotSeated)?
            .clone();
        let path = self.path(member)?;
        if path == old.path {
            return Ok(Vec::new());
        }
        let mut out = self.close(&old, contributed, now);
        if old.stack > 0 {
            out.extend(self.move_path(
                &path,
                EntryKind::BuyIn,
                -(old.stack as i64),
                now,
                Some(&old.about),
            ));
        }
        let s = self.sessions.get_mut(member).unwrap();
        s.path = path;
        s.put = s.stack;
        Ok(out)
    }

    /// A tournament buy-in or re-entry: `amt` down `member`'s path.
    pub fn tourney_buy_in(
        &mut self,
        member: &str,
        tourney: &str,
        amt: u64,
        now: u64,
    ) -> Result<Vec<Entry>, MoneyError> {
        let a = amount(amt)?;
        let path = self.path(member)?;
        self.check(&path, amt, now)?;
        Ok(self.move_path(&path, EntryKind::TourneyBuyIn, -a, now, Some(tourney)))
    }

    /// Something the club pays `member` (a [`Prize`](EntryKind::Prize) or a
    /// tournament's [`Refund`](EntryKind::Refund)): `amt` up their path.
    pub fn pay(
        &mut self,
        member: &str,
        kind: EntryKind,
        about: &str,
        amt: u64,
        now: u64,
    ) -> Result<Vec<Entry>, MoneyError> {
        if !matches!(kind, EntryKind::Prize | EntryKind::Refund) {
            return Err(MoneyError::Kind);
        }
        let a = amount(amt)?;
        let path = self.path(member)?;
        Ok(self.move_path(&path, kind, a, now, Some(about)))
    }

    /// Takes back a prize the club paid `member`: `amt` down their path, as
    /// long as their own link still has it.
    pub fn void(
        &mut self,
        member: &str,
        about: &str,
        amt: u64,
        now: u64,
    ) -> Result<Vec<Entry>, MoneyError> {
        let a = amount(amt)?;
        let path = self.path(member)?;
        let owed = self
            .link(&path[0], &path[1])
            .map_or(0, |l| l.balance.max(0));
        if a > owed {
            return Err(MoneyError::TakeBack { owed: owed as u64 });
        }
        Ok(self.move_path(&path, EntryKind::Void, -a, now, Some(about)))
    }

    /// Moving a club to links: a link's balance carried over, as its first
    /// entry. Refused once the link has entries.
    pub fn open_balance(
        &mut self,
        member: &str,
        backer: &str,
        balance: i64,
        now: u64,
    ) -> Result<Option<Entry>, MoneyError> {
        self.link_mut(member, backer)?;
        if self
            .ledger
            .iter()
            .any(|e| e.member == member && e.backer == backer)
        {
            return Err(MoneyError::LinkExists);
        }
        if balance.unsigned_abs() > MAX_CHIPS {
            return Err(MoneyError::Amount);
        }
        if balance == 0 {
            return Ok(None);
        }
        let tx = self.tx();
        Ok(Some(self.record(
            (member, backer),
            EntryKind::Opening,
            balance,
            (tx, now),
            None,
            None,
        )))
    }

    /// Moving a club to links: a player seated with `stack` at `table`
    /// (`put` brought in all), on their path now. No link moves: the chips
    /// went to the table before.
    pub fn open_session(
        &mut self,
        member: &str,
        table: &str,
        stack: u64,
        put: u64,
    ) -> Result<(), MoneyError> {
        if self.sessions.contains_key(member) {
            return Err(MoneyError::Seated);
        }
        let path = self.path(member)?;
        self.sessions.insert(
            member.into(),
            Session {
                about: table.into(),
                path,
                stack,
                put,
            },
        );
        Ok(())
    }

    /// What doesn't add up, if anything: a link whose entries don't sum to
    /// its balance, or a move along a path whose links moved by different
    /// amounts.
    pub fn audit(&self) -> Vec<String> {
        let mut bad = Vec::new();
        for l in &self.links {
            let sum: i64 = self
                .ledger
                .iter()
                .filter(|e| e.member == l.member && e.backer == l.backer)
                .map(|e| e.amount)
                .sum();
            if sum != l.balance {
                bad.push(format!(
                    "{} with {}: entries {sum}, balance {}",
                    l.member, l.backer, l.balance
                ));
            }
        }
        let mut txs: BTreeMap<u64, i64> = BTreeMap::new();
        for e in self.ledger.iter().filter(|e| e.kind.is_path_move()) {
            if *txs.entry(e.tx).or_insert(e.amount) != e.amount {
                bad.push(format!(
                    "transaction {}: links moved by different amounts",
                    e.tx
                ));
            }
        }
        bad
    }
}
