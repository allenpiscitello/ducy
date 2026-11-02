//! Tunes the personalities' style numbers to win more (#206), keeping each
//! one's character.
//!
//! For each personality, a coordinate search: every knob is nudged up and
//! down, and the three versions play the same six-handed No-limit Hold'em
//! deals against tables of the other Regulars ([`run_variants`], duplicate).
//! A nudge is kept when it wins more by more than its 95% interval and the
//! bot still plays like itself: its signature traits within tolerance and
//! its drift from its trait targets ([`traits::drift`]) at most the leash.
//! Each pass halves the steps. At the end the tuned bot plays the original on
//! fresh deals, to measure the gain without the luck of the search.
//!
//! The bots are made afresh for every hand, so moods (tilt, heaters) and
//! reads on opponents play no part: those knobs aren't tuned.
//!
//!     cargo run --release -p ducy-bots --example tune -- --only gus_bluffsen
//!
//! Options: `--only ID` (repeatable; default every personality), `--deals N`
//! per comparison (default 600, six hands each), `--passes N` (default 2),
//! `--leash X` (default 2; Gus Bluffsen gets twice it), `--threads N`
//! (default 10), `--verify N` deals for the final match (default 2000),
//! `--seed N`.

use std::{
    io::Write,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

use ducy_bots::{
    MatchConfig, Personality, PersonalityBot, Style, run_variants,
    traits::{self, MeasureConfig, Traits},
};
use ducy_play::{Bot, TableRules};

/// One style number the search may change.
struct Knob {
    name: &'static str,
    get: fn(&Style) -> f64,
    set: fn(&mut Style, f64),
    /// First step; halved every pass.
    step: f64,
    min: f64,
    max: f64,
}

macro_rules! knob {
    ($field:ident, $step:expr, $min:expr, $max:expr) => {
        Knob {
            name: stringify!($field),
            get: |s| s.$field,
            set: |s, v| s.$field = v,
            step: $step,
            min: $min,
            max: $max,
        }
    };
}

fn knobs() -> Vec<Knob> {
    vec![
        knob!(vpip, 0.03, 0.03, 0.9),
        knob!(pfr, 0.03, 0.0, 0.9),
        knob!(three_bet, 0.02, 0.0, 0.4),
        knob!(four_bet, 0.01, 0.0, 0.2),
        knob!(defend, 0.1, 0.1, 1.0),
        knob!(position_bonus, 0.1, 0.0, 0.8),
        knob!(open_size, 0.5, 2.0, 5.0),
        knob!(value_margin, 0.03, 0.0, 0.4),
        knob!(aggression, 0.1, 0.05, 1.0),
        knob!(trap, 0.1, 0.0, 1.0),
        knob!(check_raise, 0.1, 0.0, 1.0),
        knob!(bluff, 0.07, 0.0, 1.0),
        knob!(bluff_raise, 0.05, 0.0, 0.6),
        knob!(call_factor, 0.1, 0.3, 2.0),
        knob!(pair_call_factor, 0.1, 0.1, 2.0),
        knob!(caution, 0.05, 0.0, 0.6),
        knob!(bet_size, 0.15, 0.25, 2.0),
    ]
}

/// `style` with `knob` set to `v` (clamped, rounded to 3 places, raises
/// kept inside the hands it plays), or `None` if that changes nothing.
fn with(style: &Style, knob: &Knob, v: f64) -> Option<Style> {
    let v = (v.clamp(knob.min, knob.max) * 1000.0).round() / 1000.0;
    if (v - (knob.get)(style)).abs() < 1e-9 {
        return None;
    }
    let mut s = *style;
    (knob.set)(&mut s, v);
    s.pfr = s.pfr.min(s.vpip);
    (s != *style).then_some(s)
}

struct Opts {
    only: Vec<Personality>,
    deals: usize,
    passes: usize,
    leash: f64,
    threads: usize,
    verify: usize,
    seed: u64,
    trait_hands: usize,
    opponent_samples: usize,
}

fn mix(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// Opponent `k` for a hand: five different Regulars other than `p`, a new
/// table every hand, all with `samples` equity samples (quicker).
fn opponent(p: Personality, samples: usize, k: usize, seed: u64) -> Box<dyn Bot> {
    let others: Vec<Personality> = Personality::ALL.into_iter().filter(|&o| o != p).collect();
    // run_variants seeds opponent k with the hand's seed plus k.
    let start = mix(seed.wrapping_sub(k as u64)) as usize % others.len();
    let o = others[(start + k) % others.len()];
    let mut style = o.style();
    style.samples = samples;
    Box::new(PersonalityBot::new(o.name(), style, Some(seed)))
}

/// Plays `styles` of `p` on the same deals; returns each one's gain over the
/// first, (bb/100, ± 95%).
fn compare(p: Personality, styles: &[Style], deals: usize, seed: u64, o: &Opts) -> Vec<(f64, f64)> {
    let config = MatchConfig::new(TableRules::no_limit_holdem(1, 2), deals, seed).duplicate();
    type Make = dyn Fn(u64) -> Box<dyn Bot>;
    let makers: Vec<Box<Make>> = styles
        .iter()
        .map(|&s| {
            Box::new(move |seed| {
                Box::new(PersonalityBot::new(p.name(), s, Some(seed))) as Box<dyn Bot>
            }) as Box<Make>
        })
        .collect();
    let refs: Vec<&Make> = makers.iter().map(|m| &**m).collect();
    let samples = o.opponent_samples;
    let r = run_variants(&config, &refs, &|k, s| opponent(p, samples, k, s), 6).expect("match");
    (0..styles.len())
        .map(|v| {
            let c = r.compare(v, 0);
            (c.bb_per_100, c.ci95)
        })
        .collect()
}

fn measure(p: Personality, style: Style, seed: u64, o: &Opts) -> Traits {
    let mut bot = PersonalityBot::new(p.name(), style, Some(seed));
    let config = MeasureConfig {
        hands: o.trait_hands,
        seed,
        opponent_samples: Some(o.opponent_samples),
    };
    traits::measure(&mut bot, Some(p), &config)
}

/// Whether `t` is still `p`: signature traits within tolerance, drift at
/// most the leash.
fn leashed(p: Personality, t: &Traits, leash: f64) -> bool {
    let sig_ok = traits::signature(p)
        .iter()
        .all(|&tr| (t.get(tr) - traits::targets(p).get(tr)).abs() <= traits::tolerance(p, tr));
    sig_ok && traits::drift(p, t) <= leash
}

fn tune(p: Personality, o: &Opts, out: &Mutex<std::io::Stdout>) -> String {
    let say = |line: String| {
        let mut w = out.lock().unwrap();
        writeln!(w, "{:18} {line}", p.name()).ok();
        w.flush().ok();
    };
    let leash = if p == Personality::GusBluffsen {
        o.leash * 2.0
    } else {
        o.leash
    };
    let original = p.style();
    let mut style = original;
    let knobs = knobs();
    let mut seed = mix(o.seed ^ p.id().len() as u64 ^ (p as u64) << 32);
    let start = Instant::now();
    for pass in 0..o.passes {
        let scale = 0.5f64.powi(pass as i32);
        for knob in &knobs {
            let cur = (knob.get)(&style);
            let step = knob.step * scale;
            let cands: Vec<Style> = [cur + step, cur - step]
                .into_iter()
                .filter_map(|v| with(&style, knob, v))
                .collect();
            if cands.is_empty() {
                continue;
            }
            seed = seed.wrapping_add(1);
            let mut all = vec![style];
            all.extend(&cands);
            let gains = compare(p, &all, o.deals, seed, o);
            let Some((best, &(gain, ci))) = gains
                .iter()
                .enumerate()
                .skip(1)
                .max_by(|a, b| a.1.0.total_cmp(&b.1.0))
            else {
                continue;
            };
            if gain <= ci {
                continue;
            }
            let cand = all[best];
            let t = measure(p, cand, seed, o);
            let drift = traits::drift(p, &t);
            let new = (knob.get)(&cand);
            if leashed(p, &t, leash) {
                say(format!(
                    "pass {} {:16} {cur:.3} → {new:.3}: {gain:+.1} ± {ci:.1} bb/100, drift {drift:.2}",
                    pass + 1,
                    knob.name
                ));
                style = cand;
            } else {
                say(format!(
                    "pass {} {:16} {cur:.3} → {new:.3}: {gain:+.1} ± {ci:.1}, off the leash (drift {drift:.2})",
                    pass + 1,
                    knob.name
                ));
            }
        }
    }
    // The gain on fresh deals, and how far it moved.
    let verify = compare(p, &[original, style], o.verify, mix(seed ^ 0x5eed), o);
    let (gain, ci) = verify[1];
    let t = measure(p, style, mix(seed ^ 0x7a17), o);
    let changes: Vec<String> = knobs
        .iter()
        .filter(|k| ((k.get)(&original) - (k.get)(&style)).abs() > 1e-9)
        .map(|k| {
            format!(
                "{}: {:.3} → {:.3}",
                k.name,
                (k.get)(&original),
                (k.get)(&style)
            )
        })
        .collect();
    let summary = format!(
        "{}: tuned beats original by {gain:+.1} ± {ci:.1} bb/100 ({} hands), drift {:.2}, {:.0} min\n    {}",
        p.name(),
        o.verify * 6,
        traits::drift(p, &t),
        start.elapsed().as_secs_f64() / 60.0,
        if changes.is_empty() {
            "no change".to_string()
        } else {
            changes.join(", ")
        }
    );
    say(format!("done. {summary}"));
    summary
}

fn main() {
    let mut o = Opts {
        only: Vec::new(),
        deals: 600,
        passes: 2,
        leash: 2.0,
        threads: 10,
        verify: 2000,
        seed: 1,
        trait_hands: 2000,
        opponent_samples: 60,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut val = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--only" => o
                .only
                .push(Personality::from_name(&val()).expect("unknown personality")),
            "--deals" => o.deals = val().parse().expect("--deals"),
            "--passes" => o.passes = val().parse().expect("--passes"),
            "--leash" => o.leash = val().parse().expect("--leash"),
            "--threads" => o.threads = val().parse().expect("--threads"),
            "--verify" => o.verify = val().parse().expect("--verify"),
            "--seed" => o.seed = val().parse().expect("--seed"),
            "--trait-hands" => o.trait_hands = val().parse().expect("--trait-hands"),
            f => panic!("unknown option {f}"),
        }
    }
    let todo: Vec<Personality> = if o.only.is_empty() {
        Personality::ALL.to_vec()
    } else {
        o.only.clone()
    };
    let next = AtomicUsize::new(0);
    let out = Mutex::new(std::io::stdout());
    let results = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..o.threads.min(todo.len()) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&p) = todo.get(i) else { break };
                    let summary = tune(p, &o, &out);
                    results.lock().unwrap().push((i, summary));
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort();
    println!("\nSummary");
    for (_, s) in results {
        println!("{s}");
    }
}
