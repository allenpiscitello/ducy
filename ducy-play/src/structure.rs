//! Tournament blind structures: presets scaled to a starting stack, checking
//! a structure someone typed in, and about how long a tournament will take.
//!
//! A structure is a list of [`Step`]s: levels and the breaks between them.
//! The last level stays once it's reached ([`crate::Tournament`] does the
//! same with its [`Level`](crate::Level)s).
//!
//! The estimate is a simple model of players busting as stacks get short
//! against the blinds, fitted to bot-only tournaments this crate plays (see
//! `tests/structure.rs` and the `tourney_time` example).
//!
//! ```
//! use ducy_play::structure::{estimate, preset_structure, EstimateOptions};
//!
//! let steps = preset_structure("regular", 10_000).unwrap();
//! let e = estimate(&steps, &EstimateOptions { stack: 10_000, players: 27, ..Default::default() });
//! assert!(e.minutes > 120 && e.minutes < 360);
//! ```

/// One step of a structure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum Step {
    /// A blind level. `ante` is posted by every player dealt in.
    Level {
        /// The small blind.
        sb: u64,
        /// The big blind.
        bb: u64,
        /// The ante.
        ante: u64,
        /// Level length.
        minutes: u32,
    },
    /// A break between levels.
    Break {
        /// Always true; it tells a break from a level in JSON.
        #[cfg_attr(feature = "serde", serde(rename = "break"))]
        is_break: bool,
        /// Break length.
        minutes: u32,
    },
}

impl Step {
    /// A break of `minutes`.
    pub fn pause(minutes: u32) -> Self {
        Step::Break {
            is_break: true,
            minutes,
        }
    }

    /// Its length.
    pub fn minutes(&self) -> u32 {
        match *self {
            Step::Level { minutes, .. } | Step::Break { minutes, .. } => minutes,
        }
    }
}

/// At most this many levels.
pub const MAX_LEVELS: usize = 60;
/// A level or break lasts at most this many minutes.
pub const MAX_MINUTES: u32 = 240;
/// About how many hands a table plays an hour, for the estimate.
pub const HANDS_PER_HOUR: f64 = 30.0;
const BREAK_MINUTES: u32 = 10;

/// A preset: how deep the first level is, how fast blinds rise, how long
/// levels last, when antes start and how often there's a break.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preset {
    /// Its id, for [`preset_structure`].
    pub id: &'static str,
    /// Its name to show.
    pub label: &'static str,
    /// The first big blind as a share of the starting stack.
    pub start_bb: f64,
    /// About how much the big blind grows from one level to the next.
    pub growth: f64,
    /// Level length.
    pub minutes: u32,
    /// The level antes start at (counting from 1).
    pub antes_from: usize,
    /// A 10-minute break after every this many levels (0: none).
    pub break_every: usize,
}

/// Turbo, Regular and Deep.
pub const PRESETS: [Preset; 3] = [
    Preset {
        id: "turbo",
        label: "Turbo",
        start_bb: 1.0 / 50.0,
        growth: 1.5,
        minutes: 8,
        antes_from: 4,
        break_every: 0,
    },
    Preset {
        id: "regular",
        label: "Regular",
        start_bb: 1.0 / 100.0,
        growth: 1.35,
        minutes: 15,
        antes_from: 4,
        break_every: 6,
    },
    Preset {
        id: "deep",
        label: "Deep",
        start_bb: 1.0 / 200.0,
        growth: 1.25,
        minutes: 20,
        antes_from: 5,
        break_every: 5,
    },
];

// Round numbers blinds are made of: 1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6, 8 × 10^k.
const NICE: [f64; 10] = [1.0, 1.2, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0];

fn nice_at_least(x: f64) -> f64 {
    let mut p = 10f64.powf(x.max(1.0).log10().floor());
    loop {
        for m in NICE {
            if m * p >= x - 1e-9 {
                return m * p;
            }
        }
        p *= 10.0;
    }
}

// The largest whole round number at most x (0 below 1).
fn nice_at_most(x: f64) -> f64 {
    if x < 1.0 {
        return 0.0;
    }
    let mut best = 1.0;
    let mut p = 1.0;
    while p <= x {
        for m in NICE {
            let v = m * p;
            if v <= x + 1e-9 && v.fract() == 0.0 {
                best = v;
            }
        }
        p *= 10.0;
    }
    best
}

/// A preset's structure for a starting stack: levels until the big blind is
/// four starting stacks, so it lasts even for a big field. Errors for an
/// unknown preset or a stack under 20 chips.
pub fn preset_structure(id: &str, stack: u64) -> Result<Vec<Step>, String> {
    let p = PRESETS
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("no preset {id}"))?;
    if stack < 20 {
        return Err("The starting stack must be at least 20 chips.".into());
    }
    let mut bb = nice_at_least((stack as f64 * p.start_bb).max(2.0)).ceil();
    let mut steps = Vec::new();
    let mut levels = 0;
    while levels < MAX_LEVELS {
        let sb = (bb / 2.0).floor();
        let ante = if levels + 1 >= p.antes_from {
            nice_at_most(bb / 8.0)
        } else {
            0.0
        };
        steps.push(Step::Level {
            sb: sb as u64,
            bb: bb as u64,
            ante: ante as u64,
            minutes: p.minutes,
        });
        levels += 1;
        if bb >= stack as f64 * 4.0 {
            break;
        }
        if p.break_every > 0 && levels % p.break_every == 0 {
            steps.push(Step::pause(BREAK_MINUTES));
        }
        let next = nice_at_least(bb * p.growth * 0.97);
        bb = next.ceil().max(bb + 1.0);
    }
    Ok(steps)
}

/// A step as someone typed it into a form, before it's known to be whole
/// chips: for [`check_structure`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
pub struct StepInput {
    /// A break rather than a level.
    #[cfg_attr(feature = "serde", serde(default, rename = "break"))]
    pub is_break: bool,
    /// The small blind.
    #[cfg_attr(feature = "serde", serde(default))]
    pub sb: f64,
    /// The big blind.
    #[cfg_attr(feature = "serde", serde(default))]
    pub bb: f64,
    /// The ante.
    #[cfg_attr(feature = "serde", serde(default))]
    pub ante: f64,
    /// Its length.
    #[cfg_attr(feature = "serde", serde(default))]
    pub minutes: f64,
}

impl From<&Step> for StepInput {
    fn from(s: &Step) -> Self {
        match *s {
            Step::Level {
                sb,
                bb,
                ante,
                minutes,
            } => StepInput {
                is_break: false,
                sb: sb as f64,
                bb: bb as f64,
                ante: ante as f64,
                minutes: minutes as f64,
            },
            Step::Break { minutes, .. } => StepInput {
                is_break: true,
                minutes: minutes as f64,
                ..Default::default()
            },
        }
    }
}

fn whole(n: f64) -> bool {
    n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0
}

/// Problems with a structure, as messages to show; none means it's playable.
pub fn check_structure(steps: &[StepInput]) -> Vec<String> {
    let mut errors = Vec::new();
    if steps.is_empty() {
        return vec!["Add at least one level.".into()];
    }
    let mut prev: Option<&StepInput> = None;
    let mut levels = 0;
    for (i, s) in steps.iter().enumerate() {
        let at = format!("Step {}", i + 1);
        if !whole(s.minutes) || s.minutes < 1.0 || s.minutes > MAX_MINUTES as f64 {
            errors.push(format!("{at}: minutes must be 1 to {MAX_MINUTES}."));
        }
        if s.is_break {
            if prev.is_none() {
                errors.push(format!("{at}: a break can’t come before the first level."));
            }
            continue;
        }
        levels += 1;
        if ![s.sb, s.bb, s.ante].iter().all(|&n| whole(n)) || s.sb < 1.0 || s.ante < 0.0 {
            errors.push(format!(
                "{at}: blinds must be whole chips, the small blind at least 1."
            ));
        } else if s.sb > s.bb {
            errors.push(format!(
                "{at}: the small blind can’t be more than the big blind."
            ));
        } else if s.ante > s.bb {
            errors.push(format!("{at}: the ante can’t be more than the big blind."));
        }
        if let Some(p) = prev
            && whole(s.bb)
            && s.bb <= p.bb
        {
            errors.push(format!(
                "{at}: the big blind must be higher than the level before."
            ));
        }
        prev = Some(s);
    }
    if levels > MAX_LEVELS {
        errors.push(format!("At most {MAX_LEVELS} levels."));
    }
    if steps.last().is_some_and(|s| s.is_break) {
        errors.push("The structure can’t end with a break.".into());
    }
    errors
}

/// The estimate's model: each table loses `elim × (pot ÷ average stack) ^
/// power` players a hand, fitted to bot-only tournaments.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Model {
    /// The scale: players lost a hand when the pot equals the average stack.
    pub elim: f64,
    /// How fast busting speeds up as the pot grows against stacks.
    pub power: f64,
}

/// The fitted model.
pub const MODEL: Model = Model {
    elim: 0.9,
    power: 0.7,
};

/// What [`estimate`] needs besides the structure.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EstimateOptions {
    /// Starting stack.
    pub stack: u64,
    /// Entries.
    pub players: u32,
    /// Seats per table.
    pub table_size: u32,
    /// Hands a table plays an hour.
    pub hands_per_hour: f64,
    /// The bust-out model ([`MODEL`] by default).
    pub model: Model,
}

impl Default for EstimateOptions {
    fn default() -> Self {
        Self {
            stack: 10_000,
            players: 9,
            table_size: 9,
            hands_per_hour: HANDS_PER_HOUR,
            model: MODEL,
        }
    }
}

/// About how long a tournament takes to find a winner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Estimate {
    /// Minutes of play and breaks.
    pub minutes: u32,
    /// Hands each table plays.
    pub hands: u32,
    /// The level it ends in, from 1.
    pub level: usize,
}

/// About how long a tournament with this structure takes to find a winner.
pub fn estimate(steps: &[Step], o: &EstimateOptions) -> Estimate {
    // Each level's (sb, bb, ante) and the play time it starts at; each
    // break's play time and length.
    let mut levels: Vec<(f64, f64, f64, f64)> = Vec::new();
    let mut breaks: Vec<(f64, f64)> = Vec::new();
    let mut at = 0.0;
    for s in steps {
        match *s {
            Step::Break { minutes, .. } => breaks.push((at, minutes as f64)),
            Step::Level {
                sb,
                bb,
                ante,
                minutes,
            } => {
                levels.push((sb as f64, bb as f64, ante as f64, at));
                at += minutes as f64;
            }
        }
    }
    if levels.is_empty() || o.players < 2 {
        return Estimate {
            minutes: 0,
            hands: 0,
            level: 1,
        };
    }
    let per_hand = 60.0 / o.hands_per_hour;
    let total = o.players as f64 * o.stack as f64;
    let (mut left, mut played, mut hands, mut li) = (o.players as f64, 0.0, 0u32, 0usize);
    while left > 1.0 && hands < 100_000 {
        while li + 1 < levels.len() && levels[li + 1].3 <= played {
            li += 1;
        }
        let (sb, bb, ante, _) = levels[li];
        let tables = (left / o.table_size as f64).ceil();
        let pot = sb + bb + ante * (left / tables);
        let rate = o.model.elim * (pot / (total / left)).powf(o.model.power);
        left -= (rate * tables).min(left / 4.0 + 0.05);
        played += per_hand;
        hands += 1;
    }
    let break_minutes: f64 = breaks
        .iter()
        .filter(|&&(b, _)| b > 0.0 && b <= played)
        .map(|&(_, m)| m)
        .sum();
    Estimate {
        minutes: (played + break_minutes).round() as u32,
        hands,
        level: li + 1,
    }
}
