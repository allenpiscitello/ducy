//! Charts from the six-handed preflop solve (#250): for each position, how
//! often each starting hand opens when everyone before it folded, and how
//! the big blind defends against each position's open.

use super::{
    preflop::{Act, Node, POSITIONS, PreflopGame, SEATS},
    solver::PreflopSolver,
};
use crate::holdem::{
    chart::{class_of, combos},
    iso::NUM_PREFLOP_CLASSES,
};

const RANKS: &[u8; 13] = b"23456789TJQKA";

/// Per-class action frequencies at one decision.
#[derive(Clone, Debug, PartialEq)]
pub struct SpotChart {
    pub seat: usize,
    pub actions: Vec<Act>,
    /// `[class][action]`
    pub probs: Vec<Vec<f64>>,
}

impl SpotChart {
    fn read(game: &PreflopGame, solver: &PreflopSolver, node: usize) -> Self {
        let Node::Decision { seat, id, actions } = &game.nodes[node] else {
            panic!("not a decision");
        };
        Self {
            seat: *seat,
            actions: actions.iter().map(|(a, _)| *a).collect(),
            probs: (0..NUM_PREFLOP_CLASSES)
                .map(|c| solver.strategy(*id, c))
                .collect(),
        }
    }

    /// How often a class raises (any size, all-in included).
    pub fn raise(&self, class: usize) -> f64 {
        self.actions
            .iter()
            .zip(&self.probs[class])
            .filter(|(a, _)| matches!(a, Act::Raise(_) | Act::AllIn))
            .map(|(_, p)| p)
            .sum()
    }

    /// How often a class goes all-in.
    pub fn shove(&self, class: usize) -> f64 {
        self.actions
            .iter()
            .zip(&self.probs[class])
            .filter(|(a, _)| matches!(a, Act::AllIn))
            .map(|(_, p)| p)
            .sum()
    }

    /// How often a class calls (or checks).
    pub fn call(&self, class: usize) -> f64 {
        self.actions
            .iter()
            .zip(&self.probs[class])
            .filter(|(a, _)| matches!(a, Act::Call))
            .map(|(_, p)| p)
            .sum()
    }

    /// How often a class folds.
    pub fn fold(&self, class: usize) -> f64 {
        self.actions
            .iter()
            .zip(&self.probs[class])
            .filter(|(a, _)| matches!(a, Act::Fold))
            .map(|(_, p)| p)
            .sum()
    }

    /// A frequency over all 1,326 hands, by combos.
    pub fn overall(&self, f: impl Fn(&Self, usize) -> f64) -> f64 {
        (0..NUM_PREFLOP_CLASSES)
            .map(|c| f(self, c) * combos(c))
            .sum::<f64>()
            / 1326.0
    }

    /// A 13 × 13 grid of a frequency in percent: pairs on the diagonal,
    /// suited hands above it, offsuit below, aces first.
    pub fn grid(&self, f: impl Fn(&Self, usize) -> f64) -> String {
        let mut out = String::from("    ");
        for &r in RANKS.iter().rev() {
            out.push_str(&format!("{:>4}", r as char));
        }
        out.push('\n');
        for row in (0..13u8).rev() {
            out.push_str(&format!("{:>4}", RANKS[row as usize] as char));
            for col in (0..13u8).rev() {
                let class = if row == col {
                    class_of(row, col, false)
                } else {
                    class_of(row.max(col), row.min(col), col < row)
                };
                out.push_str(&format!("{:>4.0}", 100.0 * f(self, class)));
            }
            out.push('\n');
        }
        out
    }
}

/// Follows one action at a decision.
fn follow(game: &PreflopGame, node: usize, want: impl Fn(&Act) -> bool) -> Option<usize> {
    let Node::Decision { actions, .. } = &game.nodes[node] else {
        return None;
    };
    actions
        .iter()
        .find(|(a, _)| want(a))
        .map(|&(_, child)| child as usize)
}

/// The decision of `seat` when everyone before it folded.
pub fn first_in(game: &PreflopGame, seat: usize) -> usize {
    let mut node = game.root();
    for _ in 0..seat {
        node = follow(game, node, |a| *a == Act::Fold).expect("a fold");
    }
    node
}

/// The big blind's decision after `seat` opens (the standard raise) and the
/// rest fold.
pub fn big_blind_defence(game: &PreflopGame, seat: usize) -> Option<usize> {
    let mut node = follow(game, first_in(game, seat), |a| matches!(a, Act::Raise(_)))?;
    while let Node::Decision { seat: s, .. } = &game.nodes[node] {
        if *s == SEATS - 1 {
            return Some(node);
        }
        node = follow(game, node, |a| *a == Act::Fold)?;
    }
    None
}

/// Opening charts for every position but the big blind, and the big blind's
/// defence against each.
pub struct Charts {
    pub opens: Vec<SpotChart>,
    pub defences: Vec<Option<SpotChart>>,
}

impl Charts {
    pub fn read(game: &PreflopGame, solver: &PreflopSolver) -> Self {
        let seats = 0..SEATS - 1;
        Self {
            opens: seats
                .clone()
                .map(|s| SpotChart::read(game, solver, first_in(game, s)))
                .collect(),
            defences: seats
                .map(|s| big_blind_defence(game, s).map(|n| SpotChart::read(game, solver, n)))
                .collect(),
        }
    }

    /// One line per position: how much it opens first in, and how the big
    /// blind answers.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        for (s, open) in self.opens.iter().enumerate() {
            out.push_str(&format!(
                "{:>4} opens {:>5.1}% (shoves {:>4.1}%)",
                POSITIONS[s],
                100.0 * open.overall(SpotChart::raise),
                100.0 * open.overall(SpotChart::shove),
            ));
            if let Some(d) = &self.defences[s] {
                out.push_str(&format!(
                    "   BB vs it: folds {:>5.1}%, calls {:>5.1}%, raises {:>5.1}%",
                    100.0 * d.overall(SpotChart::fold),
                    100.0 * d.overall(SpotChart::call),
                    100.0 * d.overall(SpotChart::raise),
                ));
            }
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{holdem::chart::class_name, sixmax::equity::PreflopEquity};

    fn class(name: &str) -> usize {
        (0..NUM_PREFLOP_CLASSES)
            .find(|&c| class_name(c) == name)
            .expect("class")
    }

    #[test]
    fn charts_make_sense() {
        let eq = PreflopEquity::compute(300, 3, 4);
        let game = PreflopGame::new(100);
        let solver = PreflopSolver::new(&game, &eq, 9);
        solver.run(400_000, 4);
        let charts = Charts::read(&game, &solver);
        let open: Vec<f64> = charts
            .opens
            .iter()
            .map(|c| c.overall(SpotChart::raise))
            .collect();
        // Opening ranges widen from UTG to the button.
        assert!(open[0] < open[2] && open[2] < open[3], "{open:?}");
        // No dominated actions: aces always play, 72o never opens early,
        // and the big blind never folds aces.
        for chart in &charts.opens {
            assert!(chart.fold(class("AA")) < 0.05, "{}", POSITIONS[chart.seat]);
        }
        assert!(charts.opens[0].raise(class("72o")) < 0.1);
        let bb = charts.defences[3].as_ref().expect("BB vs the button");
        assert!(bb.fold(class("AA")) < 0.05);
        // The big blind defends wider against the button than UTG.
        let utg = charts.defences[0].as_ref().expect("BB vs UTG");
        assert!(bb.overall(SpotChart::fold) < utg.overall(SpotChart::fold));
        assert!(charts.summary().contains("BTN opens"));
    }
}
