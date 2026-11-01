//! Practising Ducybot's pre-flop charts (ducy-web #249, #464): a spot from
//! his charts is a skill, practised two ways:
//! - **fixed:** pick the action. A hand he plays one way has one right
//!   answer; a hand he mixes accepts any action he takes often enough.
//! - **mixed:** set how often he takes each action, scored by how close the
//!   mix is.

use serde::{Deserialize, Serialize};

use crate::{fixed, js_number};

/// An action used less than this often in a spot overall (in %) isn't asked
/// about: the solve's noise folds into the spot's main aggressive action,
/// else its call.
pub const MIN_ACTION: f64 = 4.0;
/// Open-limping from a 6-max seat other than the small blind needs this much.
pub const OPEN_LIMP_MIN: f64 = 10.0;
/// In fixed mode, besides a hand's most frequent action, any action it takes
/// at least this often (in %) is also right.
pub const FIXED_OK: f64 = 30.0;
/// Mixed mode: a guess within this distance of the true mix is right.
pub const RIGHT_MIX: f64 = 12.0;
/// Mixed mode: within this, close (no points either way).
pub const CLOSE_MIX: f64 = 25.0;

const RANKS: &[u8; 13] = b"AKQJT98765432";

/// A raise size in a chart: to so many big blinds.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Size {
    /// Raise to, in big blinds.
    pub to: f64,
}

/// A spot from a chart file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Spot {
    /// Its id: "open-UTG", "hu-vs-open-BB-BTN"…
    pub id: String,
    /// "open", "vs-open", "vs-3bet", "vs-4bet", "vs-limp" or "vs-iso".
    pub kind: String,
    /// The player practising's seat.
    pub hero: String,
    /// Who acted before them, if anyone.
    pub villain: Option<String>,
    /// The raise they face, in big blinds.
    pub facing: Option<f64>,
    /// The raise sizes the chart carries.
    pub sizes: Vec<Size>,
    /// How often each action (fold, call, raise, all-in) is taken overall, in %.
    pub overall: Vec<f64>,
    /// Each hand class's mix over the four actions (169 cells, by row).
    pub cells: Vec<Vec<f64>>,
}

/// The hand in chart cell `i` (0–168): "AA", "AKs", "AKo"…
pub fn cell_hand(i: usize) -> String {
    let (r, c) = (i / 13, i % 13);
    let k = |x: usize| RANKS[x] as char;
    if r == c {
        format!("{}{}", k(r), k(r))
    } else if r < c {
        format!("{}{}s", k(r), k(c))
    } else {
        format!("{}{}o", k(c), k(r))
    }
}

/// What "call" and "raise" mean at each kind of spot.
pub fn verbs(kind: &str) -> [&'static str; 4] {
    match kind {
        "open" => ["Fold", "Limp", "Raise", "All-in"],
        "vs-open" | "vs-iso" => ["Fold", "Call", "3-bet", "All-in"],
        "vs-3bet" => ["Fold", "Call", "4-bet", "All-in"],
        "vs-4bet" => ["Fold", "Call", "5-bet", "All-in"],
        "vs-limp" => ["Fold", "Check", "Raise", "All-in"],
        _ => ["Fold", "Call", "Raise", "All-in"],
    }
}

/// Big blinds as written in a chart: 2.5, 7.5, 3.
fn bb_text1(x: f64) -> String {
    js_number(fixed(x, 1))
}

/// The action names for a spot, with its raise size when the chart gives one:
/// "Raise to 2.5 BB", "3-bet to 7.5 or 10 BB".
pub fn sized_verbs(spot: &Spot) -> Vec<String> {
    let v: Vec<String> = verbs(&spot.kind).iter().map(|s| s.to_string()).collect();
    let sizes: Vec<f64> = spot.sizes.iter().map(|s| s.to).collect();
    let Some((last, rest)) = sizes.split_last() else {
        return v;
    };
    let to = if rest.is_empty() {
        bb_text1(*last)
    } else {
        let head: Vec<String> = rest.iter().map(|x| bb_text1(*x)).collect();
        format!("{} or {}", head.join(", "), bb_text1(*last))
    };
    v.into_iter()
        .enumerate()
        .map(|(a, w)| if a == 2 { format!("{w} to {to} BB") } else { w })
        .collect()
}

fn villain(spot: &Spot) -> &str {
    spot.villain.as_deref().unwrap_or("")
}

/// A short name for a spot: "BTN open", "BB vs BTN open", "BTN vs 3-bet".
pub fn spot_name(spot: &Spot) -> String {
    let (h, v) = (&spot.hero, villain(spot));
    match spot.kind.as_str() {
        "open" => format!("{h} open"),
        "vs-open" => format!("{h} vs {v} open"),
        "vs-3bet" => format!("{h} vs {v} 3-bet"),
        "vs-4bet" => format!("{h} vs {v} 4-bet"),
        "vs-limp" => format!("{h} vs {v} limp"),
        "vs-iso" => format!("{h} limped, {v} raised"),
        _ => spot.id.clone(),
    }
}

/// A short name for a spot from its id alone, for lists drawn before the
/// chart loads.
pub fn spot_name_from_id(id: &str) -> String {
    let parts: Vec<&str> = id.strip_prefix("hu-").unwrap_or(id).split('-').collect();
    let at = |i: usize| parts.get(i).copied().unwrap_or("").to_string();
    if parts.first() == Some(&"open") {
        return spot_name(&Spot {
            kind: "open".into(),
            hero: at(1),
            ..Spot::default()
        });
    }
    spot_name(&Spot {
        id: id.into(),
        kind: format!("vs-{}", at(1)),
        hero: at(2),
        villain: Some(at(3)),
        ..Spot::default()
    })
}

/// The question for a spot: "You're on the button, first to act."
pub fn spot_question(spot: &Spot, hu: bool) -> String {
    let at = |p: &str| -> String {
        match p {
            "BTN" => "the button".into(),
            "SB" => "the small blind".into(),
            "BB" => "the big blind".into(),
            "UTG" => "UTG".into(),
            "HJ" => "the hijack".into(),
            "CO" => "the cutoff".into(),
            p => p.into(),
        }
    };
    let facing = spot
        .facing
        .filter(|f| *f != 0.0)
        .map_or(String::new(), |f| format!(" to {} BB", bb_text1(f)));
    let (h, v) = (spot.hero.as_str(), villain(spot));
    match spot.kind.as_str() {
        "open" if hu => format!("You’re on {}, first to act.", at(h)),
        "open" if h == "UTG" => "You’re UTG, first to act.".into(),
        "open" => format!("You’re on {}. Everyone before you folded.", at(h)),
        "vs-open" => format!("You’re on {}. {v} raises{facing}.", at(h)),
        "vs-3bet" => format!("You opened from {}. {v} 3-bets{facing}.", at(h)),
        "vs-4bet" => format!("You 3-bet from {}. {v} 4-bets{facing}.", at(h)),
        "vs-limp" => format!("You’re on {}. {v} limps.", at(h)),
        "vs-iso" => format!("You limped from {}. {v} raises{facing}.", at(h)),
        _ => String::new(),
    }
}

/// An action asked about in a spot: its number (0 fold, 1 call, 2 raise,
/// 3 all-in) and what it's called there.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Asked {
    /// The action.
    pub a: usize,
    /// Its name in this spot.
    pub verb: String,
}

/// The actions asked about in a spot, and where each action's share goes
/// (itself, or the kept action it folds into).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TrainingActions {
    /// Those used at least [`MIN_ACTION`]% overall.
    pub actions: Vec<Asked>,
    /// For each of the four actions, the kept one its share goes to.
    pub into: Vec<usize>,
}

/// The actions asked about in a spot (see [`TrainingActions`]).
pub fn training_actions(spot: &Spot) -> TrainingActions {
    let hu = spot.id.starts_with("hu-");
    // The 6-max charts don't carry raise sizes; their opens are the solve's
    // 2.5 BB, or 3 BB from the small blind.
    let verbs = if spot.sizes.is_empty() && spot.kind == "open" && !hu {
        let to = if spot.hero == "SB" { 3.0 } else { 2.5 };
        sized_verbs(&Spot {
            sizes: vec![Size { to }],
            ..spot.clone()
        })
    } else {
        sized_verbs(spot)
    };
    // Open-limping is only real strategy from the small blind and heads-up.
    let limp_noise = spot.kind == "open" && spot.hero != "SB" && !hu;
    let overall = |a: usize| spot.overall.get(a).copied().unwrap_or(0.0);
    let kept: Vec<usize> = (0..4)
        .filter(|&a| {
            overall(a)
                >= if a == 1 && limp_noise {
                    OPEN_LIMP_MIN
                } else {
                    MIN_ACTION
                }
        })
        .collect();
    // A dropped action's share goes to the nearest kept action of the same
    // leaning: all-in to raise (else call), raise to all-in (else call), call
    // to raise (else fold), fold to call.
    let pref = |a: usize| -> &'static [usize] {
        match a {
            0 => &[1, 2],
            1 => &[2, 3, 0],
            2 => &[3, 1, 0],
            _ => &[2, 1, 0],
        }
    };
    let into = (0..4)
        .map(|a| {
            if kept.contains(&a) {
                a
            } else {
                pref(a)
                    .iter()
                    .copied()
                    .find(|b| kept.contains(b))
                    .or(kept.first().copied())
                    .unwrap_or(0)
            }
        })
        .collect();
    TrainingActions {
        actions: kept
            .iter()
            .map(|&a| Asked {
                a,
                verb: verbs[a].clone(),
            })
            .collect(),
        into,
    }
}

/// Hand class `i`'s mix over the kept actions, in % adding to 100.
pub fn hand_mix(spot: &Spot, i: usize, ta: &TrainingActions) -> Vec<f64> {
    let mut out = vec![0.0; ta.actions.len()];
    if let Some(cell) = spot.cells.get(i) {
        for (a, p) in cell.iter().enumerate() {
            let to = ta.into.get(a).copied();
            if let Some(k) = ta.actions.iter().position(|x| Some(x.a) == to) {
                out[k] += p;
            }
        }
    }
    let sum: f64 = out.iter().sum();
    let sum = if sum == 0.0 { 1.0 } else { sum };
    out.iter().map(|x| 100.0 * x / sum).collect()
}

fn max(mix: &[f64]) -> f64 {
    mix.iter().copied().fold(f64::NEG_INFINITY, f64::max)
}

/// Fixed mode: the actions (indexes into the kept ones) that count as right.
pub fn fixed_answers(mix: &[f64]) -> Vec<usize> {
    let top = max(mix);
    (0..mix.len())
        .filter(|&k| mix[k] == top || mix[k] >= FIXED_OK)
        .collect()
}

/// Fixed mode: how bad picking action `k` is, from 0 (right) to 1.
pub fn fixed_severity(mix: &[f64], k: usize) -> f64 {
    if fixed_answers(mix).contains(&k) {
        return 0.0;
    }
    ((max(mix) - mix.get(k).copied().unwrap_or(0.0)) / 100.0).min(1.0)
}

/// Mixed mode: the distance between two mixes, 0 to 100.
pub fn mix_distance(guess: &[f64], truth: &[f64]) -> f64 {
    guess
        .iter()
        .enumerate()
        .map(|(k, g)| (g - truth.get(k).copied().unwrap_or(0.0)).abs())
        .sum::<f64>()
        / 2.0
}

/// How hard a hand is to get right: 0 for one action always, 1 for an even
/// split of the top two.
pub fn difficulty(mix: &[f64]) -> f64 {
    let mut s = mix.to_vec();
    s.sort_by(|a, b| b.total_cmp(a));
    let a = s.first().copied().unwrap_or(0.0);
    let b = s.get(1).copied().unwrap_or(0.0);
    1.0 - (a - b) / 100.0
}

/// The main action (index into the kept ones) of a mix.
pub fn main_action(mix: &[f64]) -> usize {
    let top = max(mix);
    mix.iter().position(|x| *x == top).unwrap_or(0)
}

/// Hands too easy to ask about except as an occasional probe: always played
/// one way (95%+), with every neighbour on the grid played the same way.
pub fn obvious_cells(spot: &Spot, ta: &TrainingActions) -> Vec<bool> {
    let mixes: Vec<Vec<f64>> = (0..spot.cells.len())
        .map(|i| hand_mix(spot, i, ta))
        .collect();
    let main: Vec<usize> = mixes.iter().map(|m| main_action(m)).collect();
    (0..mixes.len())
        .map(|i| {
            if max(&mixes[i]) < 95.0 {
                return false;
            }
            let (r, c) = ((i / 13) as i64, (i % 13) as i64);
            for dr in -1..=1 {
                for dc in -1..=1 {
                    let (rr, cc) = (r + dr, c + dc);
                    if (dr != 0 || dc != 0)
                        && (0..13).contains(&rr)
                        && (0..13).contains(&cc)
                        && main.get((rr * 13 + cc) as usize) != Some(&main[i])
                    {
                        return false;
                    }
                }
            }
            true
        })
        .collect()
}

/// How many combos hand class `i` has: 6 for a pair, 4 suited, 12 offsuit.
fn combos(i: usize) -> f64 {
    let (r, c) = (i / 13, i % 13);
    if r == c {
        6.0
    } else if r < c {
        4.0
    } else {
        12.0
    }
}

/// Picks a hand class for practice: not obvious (unless `probe`), within the
/// difficulty `band` ([min, max)), weighted by its combos, never `avoid`
/// (the hand just played) unless it's all there is. Relaxes the band, then
/// obviousness, rather than give up. `r`: a random draw in `0..1`.
pub fn pick_cell(spot: &Spot, band: [f64; 2], probe: bool, r: f64, avoid: Option<usize>) -> usize {
    let ta = training_actions(spot);
    let obvious = obvious_cells(spot, &ta);
    let mixes: Vec<Vec<f64>> = (0..spot.cells.len())
        .map(|i| hand_mix(spot, i, &ta))
        .collect();
    let in_band = |i: usize| {
        let d = difficulty(&mixes[i]);
        !obvious[i] && d >= band[0] && d < band[1]
    };
    let tries: Vec<Box<dyn Fn(usize) -> bool>> = if probe {
        vec![Box::new(|i| obvious[i])]
    } else {
        vec![
            Box::new(in_band),
            Box::new(|i| !obvious[i]),
            Box::new(|_| true),
        ]
    };
    for ok in tries {
        let mut pool: Vec<usize> = (0..mixes.len()).filter(|&i| ok(i)).collect();
        if let Some(a) = avoid
            && pool.len() > 1
        {
            pool.retain(|&i| i != a);
        }
        if pool.is_empty() {
            continue;
        }
        let mut x = r * pool.iter().map(|&i| combos(i)).sum::<f64>();
        for &i in &pool {
            x -= combos(i);
            if x < 0.0 {
                return i;
            }
        }
        return *pool.last().expect("not empty");
    }
    0
}

/// Two cards of hand class `i` (random suits that fit it), high card first.
/// `r`: three random draws in `0..1`, for shuffling the suits.
pub fn cards_for(i: usize, r: [f64; 3]) -> [String; 2] {
    let h = cell_hand(i);
    let mut s = ['s', 'h', 'd', 'c'];
    for (n, k) in (1..4).rev().enumerate() {
        let j = ((r[n] * (k as f64 + 1.0)).floor() as usize).min(k);
        s.swap(k, j);
    }
    let b: Vec<char> = h.chars().collect();
    if b.len() == 3 && b[2] == 's' {
        return [format!("{}{}", b[0], s[0]), format!("{}{}", b[1], s[0])];
    }
    [format!("{}{}", b[0], s[0]), format!("{}{}", b[1], s[1])]
}

/// A fingerprint of a spot's chart, so progress earned on an older chart
/// starts over: djb2 of its cells as JSON, in base 36.
pub fn spot_fingerprint(spot: &Spot) -> String {
    let rows: Vec<String> = spot
        .cells
        .iter()
        .map(|c| {
            let v: Vec<String> = c.iter().map(|x| js_number(*x)).collect();
            format!("[{}]", v.join(","))
        })
        .collect();
    let text = format!("[{}]", rows.join(","));
    let mut h: i32 = 5381;
    for u in text.encode_utf16() {
        h = h.wrapping_shl(5).wrapping_add(h).wrapping_add(i32::from(u));
    }
    base36(h as u32)
}

fn base36(mut n: u32) -> String {
    const D: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".into();
    }
    let mut out = vec![];
    while n > 0 {
        out.push(D[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).expect("ascii")
}

/// Points lost for a wrong fixed answer: 2 for a near miss up to 16; 20 on
/// a probe.
pub fn fixed_penalty(severity: f64, probe: bool) -> u32 {
    if probe {
        return 20;
    }
    ((16.0 * severity).round() as u32).max(2)
}

/// Points lost for a mixed answer that's off: 0 inside [`CLOSE_MIX`], then
/// up to 16.
pub fn mixed_penalty(distance: f64) -> u32 {
    if distance <= CLOSE_MIX {
        return 0;
    }
    ((16.0 * ((distance - CLOSE_MIX) / 50.0).min(1.0)).round() as u32).max(2)
}

/// A slider change in mixed mode: action `k` set to `v` (0–100), the others
/// scaled to make up the rest, keeping their proportions (or splitting it
/// evenly when they're all 0). Whole numbers adding to 100.
pub fn set_share(mix: &[f64], k: usize, v: f64) -> Vec<i64> {
    let v = v.round().clamp(0.0, 100.0);
    let rest = 100.0 - v;
    let others: Vec<f64> = mix
        .iter()
        .enumerate()
        .map(|(j, p)| if j == k { 0.0 } else { *p })
        .collect();
    let sum: f64 = others.iter().sum();
    let n = mix.len().saturating_sub(1) as f64;
    let mut out: Vec<i64> = mix
        .iter()
        .enumerate()
        .map(|(j, p)| {
            let x = if j == k {
                v
            } else if sum != 0.0 {
                p * rest / sum
            } else if n != 0.0 {
                rest / n
            } else {
                0.0
            };
            js_round(x)
        })
        .collect();
    let diff = 100 - out.iter().sum::<i64>();
    if diff != 0 {
        // Settle the difference on the largest other share (the first, on a tie).
        let j = (0..out.len())
            .filter(|&j| j != k)
            .max_by(|&a, &b| out[a].cmp(&out[b]).then(b.cmp(&a)));
        match j {
            Some(j) => out[j] += diff,
            None => {
                if let Some(x) = out.get_mut(k) {
                    *x += diff;
                }
            }
        }
    }
    out
}

/// JavaScript's `Math.round`: halves go up.
fn js_round(x: f64) -> i64 {
    (x + 0.5).floor() as i64
}

/// A seat at the table as the hand reaches the player.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SceneSeat {
    /// Its position.
    pub pos: String,
    /// "hero", "fold", "raise", "call", "blind" or "waiting".
    pub state: String,
    /// What it shows: "Raise 2.5", "Fold", "0.5"…
    pub label: String,
}

/// The table when it's the player's turn in a spot.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Scene {
    /// Who sits where, and what each did.
    pub seats: Vec<SceneSeat>,
    /// The button's position.
    pub button: String,
    /// The pot, in big blinds.
    pub pot: f64,
    /// The action in words: "UTG folds · HJ raises to 2.5".
    pub story: String,
}

/// Raises a spot's chart doesn't carry, in big blinds.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SceneSizes {
    /// The open (2.5 by default).
    pub open: Option<f64>,
    /// The small blind's open six-handed (3 by default).
    pub sb_open: Option<f64>,
    /// The 3-bet made before facing a 4-bet.
    pub three_bet: Option<f64>,
}

/// The table when it's the player's turn in `spot`: who sits where, what each
/// did before them, and the pot.
pub fn table_scene(spot: &Spot, hu: bool, sizes: &SceneSizes) -> Scene {
    let seats: &[&str] = if hu {
        &["BTN", "BB"]
    } else {
        &["UTG", "HJ", "CO", "BTN", "SB", "BB"]
    };
    let bb2 = |x: f64| js_number(fixed(x, 2));
    let open = sizes.open.unwrap_or(2.5);
    let open_from = |p: &str| {
        if p == "SB" && !hu {
            sizes.sb_open.unwrap_or(3.0)
        } else {
            open
        }
    };
    let blind_of = |p: &str| -> f64 {
        match (hu, p) {
            (true, "BTN") => 0.5,
            (true, _) => 1.0,
            (false, "SB") => 0.5,
            (false, "BB") => 1.0,
            _ => 0.0,
        }
    };
    let idx = |p: &str| seats.iter().position(|x| *x == p).unwrap_or(0);
    let mut put: Vec<f64> = seats.iter().map(|_| 0.0).collect();
    let mut state: Vec<String> = seats.iter().map(|_| String::new()).collect();
    let mut label: Vec<String> = seats.iter().map(|_| String::new()).collect();
    for (i, p) in seats.iter().enumerate() {
        let b = blind_of(p);
        if b != 0.0 {
            put[i] = b;
            state[i] = "blind".into();
            label[i] = bb2(b);
        }
    }
    let mut story: Vec<String> = vec![];
    let mut act = |put: &mut Vec<f64>,
                   state: &mut Vec<String>,
                   label: &mut Vec<String>,
                   p: &str,
                   s: &str,
                   amount: Option<f64>,
                   words: &str| {
        let i = idx(p);
        state[i] = s.into();
        if let Some(a) = amount {
            put[i] = a;
        }
        let a = amount.unwrap_or(0.0);
        label[i] = match s {
            "fold" => "Fold".into(),
            "call" if a == 1.0 && words == "limps" => "Limp".into(),
            "call" => format!("Call {}", bb2(a)),
            _ => format!(
                "{} {}",
                if words == "raises" { "Raise" } else { words },
                bb2(a)
            ),
        };
        story.push(match s {
            "fold" => format!("{p} folds"),
            "call" if words == "limps" => format!("{p} limps"),
            "call" => format!("{p} calls {}", bb2(a)),
            _ => format!("{p} {} to {}", words.to_lowercase(), bb2(a)),
        });
    };
    let before = |p: &str| seats[..idx(p)].to_vec();
    let between = |a: &str, b: &str| {
        let (i, j) = (idx(a) + 1, idx(b));
        if i < j { seats[i..j].to_vec() } else { vec![] }
    };
    macro_rules! folds {
        ($ps:expr) => {
            for p in $ps {
                if state[idx(p)] != "fold" {
                    act(&mut put, &mut state, &mut label, p, "fold", None, "");
                }
            }
        };
    }
    let hero = spot.hero.as_str();
    let v = villain(spot);
    let after = |p: &str, others: &[&str]| -> Vec<&'static str> {
        seats
            .iter()
            .copied()
            .filter(|x| !others.contains(x) && idx(x) > idx(p))
            .collect()
    };
    match spot.kind.as_str() {
        "open" => folds!(before(hero)),
        "vs-open" => {
            folds!(before(v));
            act(
                &mut put,
                &mut state,
                &mut label,
                v,
                "raise",
                Some(spot.facing.unwrap_or(open_from(v))),
                "raises",
            );
            folds!(between(v, hero));
        }
        "vs-limp" => {
            folds!(before(v));
            act(
                &mut put,
                &mut state,
                &mut label,
                v,
                "call",
                Some(1.0),
                "limps",
            );
            folds!(between(v, hero));
        }
        "vs-iso" => {
            folds!(before(hero));
            act(
                &mut put,
                &mut state,
                &mut label,
                hero,
                "call",
                Some(1.0),
                "limps",
            );
            folds!(between(hero, v));
            act(
                &mut put,
                &mut state,
                &mut label,
                v,
                "raise",
                Some(spot.facing.unwrap_or(3.0)),
                "raises",
            );
        }
        "vs-3bet" => {
            folds!(before(hero));
            act(
                &mut put,
                &mut state,
                &mut label,
                hero,
                "raise",
                Some(open_from(hero)),
                "raises",
            );
            folds!(between(hero, v));
            act(
                &mut put,
                &mut state,
                &mut label,
                v,
                "raise",
                Some(spot.facing.unwrap_or(3.0 * open_from(hero))),
                "3-bets",
            );
            folds!(after(v, &[hero, v]));
        }
        "vs-4bet" => {
            folds!(before(v));
            act(
                &mut put,
                &mut state,
                &mut label,
                v,
                "raise",
                Some(open_from(v)),
                "raises",
            );
            folds!(between(v, hero));
            let three = sizes.three_bet.unwrap_or(3.0 * open_from(v));
            act(
                &mut put,
                &mut state,
                &mut label,
                hero,
                "raise",
                Some(three),
                "3-bets",
            );
            folds!(after(hero, &[hero, v]));
            act(
                &mut put,
                &mut state,
                &mut label,
                v,
                "raise",
                Some(spot.facing.unwrap_or(2.3 * three)),
                "4-bets",
            );
        }
        _ => {}
    }
    // The player, and whoever is still to act after them (blinds keep their chips).
    let h = idx(hero);
    let facing = state[h].clone();
    state[h] = "hero".into();
    if facing.is_empty() || facing == "blind" {
        let b = blind_of(hero);
        label[h] = if b != 0.0 { bb2(b) } else { String::new() };
    }
    for s in state.iter_mut().skip(h + 1) {
        if s.is_empty() {
            *s = "waiting".into();
        }
    }
    let pot = fixed(put.iter().sum(), 2);
    Scene {
        seats: seats
            .iter()
            .enumerate()
            .map(|(i, p)| SceneSeat {
                pos: (*p).into(),
                state: if state[i].is_empty() {
                    "waiting".into()
                } else {
                    state[i].clone()
                },
                label: label[i].clone(),
            })
            .collect(),
        button: "BTN".into(),
        pot,
        story: story.join(" · "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A spot whose cells are all `mix`, with `overall` actions.
    fn spot(kind: &str, hero: &str, overall: [f64; 4], mix: [f64; 4]) -> Spot {
        Spot {
            id: format!("{kind}-{hero}"),
            kind: kind.into(),
            hero: hero.into(),
            villain: Some("BTN".into()),
            overall: overall.to_vec(),
            cells: vec![mix.to_vec(); 169],
            ..Spot::default()
        }
    }

    #[test]
    fn hands_names_and_questions() {
        assert_eq!(
            (cell_hand(0), cell_hand(1), cell_hand(13), cell_hand(168)),
            ("AA".into(), "AKs".into(), "AKo".into(), "22".into())
        );
        assert_eq!(spot_name_from_id("hu-vs-open-BB-BTN"), "BB vs BTN open");
        assert_eq!(spot_name_from_id("open-UTG"), "UTG open");
        let mut s = spot("vs-open", "BB", [50.0, 30.0, 20.0, 0.0], [0.0; 4]);
        s.facing = Some(2.5);
        assert_eq!(
            spot_question(&s, false),
            "You’re on the big blind. BTN raises to 2.5 BB."
        );
        assert_eq!(
            spot_question(&spot("open", "UTG", [0.0; 4], [0.0; 4]), false),
            "You’re UTG, first to act."
        );
        s.sizes = vec![Size { to: 7.5 }, Size { to: 10.0 }];
        assert_eq!(sized_verbs(&s)[2], "3-bet to 7.5 or 10 BB");
    }

    #[test]
    fn actions_kept_and_folded_in() {
        // A 2% all-in folds into the raise; a 6-max limp under 10% into the raise too.
        let s = spot("open", "UTG", [60.0, 5.0, 33.0, 2.0], [0.0, 5.0, 90.0, 5.0]);
        let ta = training_actions(&s);
        assert_eq!(ta.actions.iter().map(|a| a.a).collect::<Vec<_>>(), [0, 2]);
        assert_eq!(ta.actions[1].verb, "Raise to 2.5 BB");
        assert_eq!(ta.into, [0, 2, 2, 2]);
        assert_eq!(hand_mix(&s, 0, &ta), [0.0, 100.0]);
    }

    #[test]
    fn scoring_answers() {
        let mix = [60.0, 35.0, 5.0];
        assert_eq!(fixed_answers(&mix), [0, 1]);
        assert_eq!(fixed_severity(&mix, 2), 0.55);
        assert_eq!(mix_distance(&[50.0, 50.0, 0.0], &mix), 15.0);
        assert!((difficulty(&mix) - 0.75).abs() < 1e-9);
        assert_eq!(main_action(&mix), 0);
        assert_eq!(
            (
                fixed_penalty(0.05, false),
                fixed_penalty(1.0, false),
                fixed_penalty(0.0, true)
            ),
            (2, 16, 20)
        );
        assert_eq!(
            (
                mixed_penalty(20.0),
                mixed_penalty(30.0),
                mixed_penalty(100.0)
            ),
            (0, 2, 16)
        );
        assert_eq!(set_share(&[50.0, 30.0, 20.0], 0, 80.0), [80, 12, 8]);
        assert_eq!(set_share(&[100.0, 0.0, 0.0], 0, 40.0), [40, 30, 30]);
        // 33.5 each rounds up: the extra point comes off the first.
        assert_eq!(set_share(&[0.0, 50.0, 50.0], 0, 33.0), [33, 33, 34]);
    }

    #[test]
    fn picking_hands_and_their_cards() {
        // Every hand raised: all obvious; a probe picks one, weighted by combos.
        let s = spot(
            "open",
            "BTN",
            [0.0, 0.0, 100.0, 0.0],
            [0.0, 0.0, 100.0, 0.0],
        );
        let ta = training_actions(&s);
        assert!(obvious_cells(&s, &ta).iter().all(|x| *x));
        assert_eq!(pick_cell(&s, [0.0, 1.01], true, 0.0, None), 0);
        assert_eq!(pick_cell(&s, [0.0, 1.01], true, 0.0, Some(0)), 1);
        // Not a probe, nothing in the band: still a hand.
        assert!(pick_cell(&s, [2.0, 3.0], false, 0.99, None) < 169);
        let [a, b] = cards_for(1, [0.1, 0.5, 0.9]);
        assert_eq!(
            (&a[..1], &b[..1], &a[1..], &b[1..]),
            ("A", "K", &b[1..], &a[1..])
        );
        let [c, d] = cards_for(13, [0.1, 0.5, 0.9]);
        assert_ne!(c[1..], d[1..]);
    }

    #[test]
    fn fingerprints_match_the_pages() {
        // As the page's JS made it: JSON.stringify of [[0,50.5,49.5,0]].
        let s = Spot {
            cells: vec![vec![0.0, 50.5, 49.5, 0.0]],
            ..Spot::default()
        };
        let text = "[[0,50.5,49.5,0]]";
        let mut h: i32 = 5381;
        for u in text.encode_utf16() {
            h = h.wrapping_shl(5).wrapping_add(h).wrapping_add(i32::from(u));
        }
        assert_eq!(spot_fingerprint(&s), base36(h as u32));
        assert_eq!(base36(35), "z");
    }

    #[test]
    fn the_table_as_the_hand_reaches_you() {
        let mut s = spot("vs-open", "BB", [0.0; 4], [0.0; 4]);
        s.villain = Some("CO".into());
        let sc = table_scene(&s, false, &SceneSizes::default());
        assert_eq!(
            sc.story,
            "UTG folds · HJ folds · CO raises to 2.5 · BTN folds · SB folds"
        );
        // The blinds, the folded small blind's too, and the open.
        assert_eq!(sc.pot, 4.0);
        let bb = sc.seats.iter().find(|x| x.pos == "BB").unwrap();
        assert_eq!((bb.state.as_str(), bb.label.as_str()), ("hero", "1"));
        let hu = table_scene(
            &spot("open", "BTN", [0.0; 4], [0.0; 4]),
            true,
            &SceneSizes::default(),
        );
        assert_eq!(hu.story, "");
        assert_eq!(hu.seats[1].state, "blind");
    }
}
