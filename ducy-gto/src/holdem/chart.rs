//! Preflop charts from a strategy: how often the button opens each starting
//! hand, and how the big blind answers a limp or an open. For checking a
//! blueprint by eye and in tests.

use super::{
    hunl::{Hunl, HunlAction},
    iso::NUM_PREFLOP_CLASSES,
};

const RANKS: &[u8; 13] = b"23456789TJQKA";

/// The preflop class of a hand given by ranks (0 = deuce .. 12 = ace).
pub fn class_of(hi: u8, lo: u8, suited: bool) -> usize {
    let (hi, lo) = if hi >= lo { (hi, lo) } else { (lo, hi) };
    if hi == lo {
        return hi as usize;
    }
    let idx = hi as usize * (hi as usize - 1) / 2 + lo as usize;
    if suited { 13 + idx } else { 91 + idx }
}

/// The name of a class, e.g. "AKs", "T9o", "77".
pub fn class_name(class: usize) -> String {
    let r = |x: u8| RANKS[x as usize] as char;
    for hi in 0..13u8 {
        if class_of(hi, hi, false) == class {
            return format!("{}{}", r(hi), r(hi));
        }
        for lo in 0..hi {
            for (suited, tag) in [(true, 's'), (false, 'o')] {
                if class_of(hi, lo, suited) == class {
                    return format!("{}{}{tag}", r(hi), r(lo));
                }
            }
        }
    }
    String::new()
}

/// Two-card combinations in a class: 6 for a pair, 4 suited, 12 offsuit.
pub fn combos(class: usize) -> f64 {
    match class {
        0..13 => 6.0,
        13..91 => 4.0,
        _ => 12.0,
    }
}

/// Per-class action frequencies at one preflop node.
#[derive(Clone, Debug, PartialEq)]
pub struct Spot {
    pub node: u32,
    pub actions: Vec<HunlAction>,
    /// `[class][action]`
    pub probs: Vec<Vec<f64>>,
}

impl Spot {
    /// Reads every class's strategy at `node` from `strategy(node, bucket)`.
    pub fn read(game: &Hunl, node: u32, strategy: impl Fn(u32, u16) -> Vec<f64>) -> Self {
        let actions = game.tree.nodes[node as usize].actions.clone();
        let probs = (0..NUM_PREFLOP_CLASSES)
            .map(|c| strategy(node, c as u16))
            .collect();
        Self {
            node,
            actions,
            probs,
        }
    }

    /// How often `class` folds.
    pub fn fold(&self, class: usize) -> f64 {
        self.actions
            .iter()
            .position(|a| *a == HunlAction::Fold)
            .map_or(0.0, |i| self.probs[class][i])
    }

    /// How often `class` bets or raises (any size).
    pub fn raise(&self, class: usize) -> f64 {
        self.actions
            .iter()
            .zip(&self.probs[class])
            .filter(|(a, _)| matches!(a, HunlAction::Bet(_) | HunlAction::Raise(_)))
            .map(|(_, p)| p)
            .sum()
    }

    /// Share of all 1,326 hands that `f` picks, weighting each class by its
    /// combinations.
    pub fn overall(&self, f: impl Fn(&Self, usize) -> f64) -> f64 {
        (0..NUM_PREFLOP_CLASSES)
            .map(|c| combos(c) * f(self, c))
            .sum::<f64>()
            / 1326.0
    }

    /// A 13×13 grid of `f` in percent: pairs on the diagonal, suited hands
    /// above it and offsuit below, aces first.
    pub fn grid(&self, f: impl Fn(&Self, usize) -> f64) -> String {
        let mut out = String::from("     ");
        for c in (0..13).rev() {
            out.push_str(&format!("{:>4}", RANKS[c] as char));
        }
        out.push('\n');
        for row in (0..13u8).rev() {
            out.push_str(&format!("{:>4} ", RANKS[row as usize] as char));
            for col in (0..13u8).rev() {
                let class = if row == col {
                    class_of(row, col, false)
                } else if col < row {
                    class_of(row, col, true)
                } else {
                    class_of(col, row, false)
                };
                out.push_str(&format!("{:>4.0}", 100.0 * f(self, class)));
            }
            out.push('\n');
        }
        out
    }
}

/// The preflop spots worth looking at: the button's first decision, and the
/// big blind's answer to a limp and to the smallest open.
pub struct PreflopReport {
    pub button: Spot,
    pub vs_limp: Option<Spot>,
    pub vs_open: Option<Spot>,
}

impl PreflopReport {
    pub fn read(game: &Hunl, strategy: impl Fn(u32, u16) -> Vec<f64>) -> Self {
        let root = &game.tree.nodes[0];
        let child = |pred: fn(&HunlAction) -> bool| {
            root.actions.iter().position(pred).map(|i| root.children[i])
        };
        let limp = child(|a| *a == HunlAction::Call);
        let open = child(|a| matches!(a, HunlAction::Raise(_)));
        Self {
            button: Spot::read(game, 0, &strategy),
            vs_limp: limp.map(|n| Spot::read(game, n, &strategy)),
            vs_open: open.map(|n| Spot::read(game, n, &strategy)),
        }
    }

    /// A few lines of headline numbers.
    pub fn summary(&self) -> String {
        let mut s = format!(
            "button: open {:.1}%, limp {:.1}%, fold {:.1}%",
            100.0 * self.button.overall(Spot::raise),
            100.0 * self.button.overall(|sp, c| 1.0 - sp.raise(c) - sp.fold(c)),
            100.0 * self.button.overall(Spot::fold),
        );
        if let Some(o) = &self.vs_open {
            s.push_str(&format!(
                "; BB vs open: defend {:.1}%, 3-bet {:.1}%",
                100.0 * o.overall(|sp, c| 1.0 - sp.fold(c)),
                100.0 * o.overall(Spot::raise)
            ));
        }
        if let Some(l) = &self.vs_limp {
            s.push_str(&format!(
                "; BB vs limp: raise {:.1}%",
                100.0 * l.overall(Spot::raise)
            ));
        }
        s
    }
}
