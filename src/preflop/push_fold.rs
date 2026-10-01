use super::{HandClass, PreflopEquityTable, compatible_pairs};

const N: usize = HandClass::COUNT;
// Opponent combos left after removing your two cards: C(50, 2).
const OPPONENT_COMBOS: f64 = 1225.0;

/// Heads-up push/fold equilibrium: how often the small blind shoves and the
/// big blind calls with each starting-hand class.
#[derive(Debug, Clone)]
pub struct PushFoldSolution {
    /// Effective stack in big blinds, before posting blinds and antes.
    pub stack_bb: f64,
    /// Ante each player posts, in big blinds.
    pub ante_bb: f64,
    /// Small blind shove frequency per class (index by `HandClass::index`).
    pub push: Vec<f64>,
    /// Big blind call frequency per class, facing a shove.
    pub call: Vec<f64>,
    /// How much (in BB per hand, averaged over both players) a perfect
    /// counter-strategy would gain against this solution; 0 is exact.
    pub exploitability_bb: f64,
}

impl PushFoldSolution {
    /// Small blind shove frequency for `class`.
    pub fn push_frequency(&self, class: HandClass) -> f64 {
        self.push[class.index()]
    }

    /// Big blind call frequency for `class`.
    pub fn call_frequency(&self, class: HandClass) -> f64 {
        self.call[class.index()]
    }

    /// Share of all 1326 starting combos the small blind shoves.
    pub fn push_range_fraction(&self) -> f64 {
        combo_weighted(&self.push)
    }

    /// Share of all 1326 starting combos the big blind calls with.
    pub fn call_range_fraction(&self) -> f64 {
        combo_weighted(&self.call)
    }
}

fn combo_weighted(freq: &[f64]) -> f64 {
    HandClass::all()
        .map(|c| freq[c.index()] * c.combo_count() as f64)
        .sum::<f64>()
        / 1326.0
}

struct Game {
    stack: f64,
    ante: f64,
    combos: Vec<f64>,
    // pairs[a * N + b]: combo pairs of classes a and b that share no card.
    pairs: Vec<f64>,
    // eq[a * N + b]: equity of class a against class b.
    eq: Vec<f64>,
}

impl Game {
    fn eq(&self, a: usize, b: usize) -> f64 {
        self.eq[a * N + b]
    }

    /// Small blind EV (chips, relative to its stack before posting) of
    /// shoving with class `a` against call frequencies `call`.
    fn push_ev(&self, a: usize, call: &[f64]) -> f64 {
        let (s, steal) = (self.stack, 1.0 + self.ante);
        let total: f64 = (0..N)
            .map(|b| {
                let w = self.pairs[a * N + b] / self.combos[a];
                w * ((1.0 - call[b]) * steal + call[b] * (self.eq(a, b) * 2.0 * s - s))
            })
            .sum();
        total / OPPONENT_COMBOS
    }

    fn fold_ev(&self) -> f64 {
        -(0.5 + self.ante)
    }

    /// Big blind's gain from calling (vs folding) with class `b`, summed over
    /// the small blind's shoving combos; positive means calling is better.
    fn call_gain(&self, b: usize, push: &[f64]) -> f64 {
        let s = self.stack;
        (0..N)
            .map(|a| {
                let w = self.pairs[a * N + b] / self.combos[b];
                w * push[a] * (self.eq(b, a) * 2.0 * s - s + 1.0 + self.ante)
            })
            .sum()
    }

    /// Big blind EV of class `b` against `push`, calling with frequency `c`.
    fn bb_ev(&self, b: usize, push: &[f64], c: f64) -> f64 {
        let s = self.stack;
        let total: f64 = (0..N)
            .map(|a| {
                let w = self.pairs[a * N + b] / self.combos[b];
                let vs_push = c * (self.eq(b, a) * 2.0 * s - s) + (1.0 - c) * -(1.0 + self.ante);
                w * ((1.0 - push[a]) * (0.5 + self.ante) + push[a] * vs_push)
            })
            .sum();
        total / OPPONENT_COMBOS
    }

    /// Exploitability: average of each player's best-response gain.
    fn exploitability(&self, push: &[f64], call: &[f64]) -> f64 {
        let (mut sb_value, mut sb_best) = (0.0, 0.0);
        for (a, &pushed) in push.iter().enumerate() {
            let (p, f) = (self.push_ev(a, call), self.fold_ev());
            let w = self.combos[a] / 1326.0;
            sb_value += w * (pushed * p + (1.0 - pushed) * f);
            sb_best += w * p.max(f);
        }
        let mut bb_best = 0.0;
        for b in 0..N {
            let w = self.combos[b] / 1326.0;
            bb_best += w * self.bb_ev(b, push, 1.0).max(self.bb_ev(b, push, 0.0));
        }
        // Zero-sum: the big blind's value under (push, call) is -sb_value.
        ((sb_best - sb_value) + (bb_best + sb_value)) / 2.0
    }
}

/// Solves heads-up push/fold: the small blind shoves all-in or folds, and the
/// big blind calls or folds. `stack_bb` is the effective stack before posting
/// (blinds 0.5/1 BB) and `ante_bb` is each player's ante. Runs `iterations`
/// rounds of fictitious play (a few thousand converge to well under 0.01 BB
/// of exploitability) using card-removal-aware weights.
pub fn solve_heads_up_push_fold(
    stack_bb: f64,
    ante_bb: f64,
    table: &PreflopEquityTable,
    iterations: u32,
) -> PushFoldSolution {
    let combos: Vec<f64> = HandClass::all().map(|c| c.combo_count() as f64).collect();
    let mut pairs = vec![0.0; N * N];
    let mut eq = vec![0.0; N * N];
    for a in HandClass::all() {
        for b in HandClass::all() {
            pairs[a.index() * N + b.index()] = compatible_pairs(a, b) as f64;
            eq[a.index() * N + b.index()] = table.equity(a, b);
        }
    }
    let game = Game {
        stack: stack_bb,
        ante: ante_bb,
        combos,
        pairs,
        eq,
    };

    let mut push = vec![1.0; N];
    let mut call = vec![0.0; N];
    for t in 1..=iterations.max(1) {
        let step = 1.0 / (t as f64 + 1.0);
        let br_call: Vec<f64> = (0..N)
            .map(|b| f64::from(game.call_gain(b, &push) > 0.0))
            .collect();
        for (c, br) in call.iter_mut().zip(&br_call) {
            *c += (br - *c) * step;
        }
        let br_push: Vec<f64> = (0..N)
            .map(|a| f64::from(game.push_ev(a, &call) > game.fold_ev()))
            .collect();
        for (p, br) in push.iter_mut().zip(&br_push) {
            *p += (br - *p) * step;
        }
    }

    let exploitability_bb = game.exploitability(&push, &call);
    PushFoldSolution {
        stack_bb,
        ante_bb,
        push,
        call,
        exploitability_bb,
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn solve(stack: f64) -> PushFoldSolution {
        solve_heads_up_push_fold(stack, 0.0, PreflopEquityTable::bundled(), 800)
    }

    fn class(s: &str) -> HandClass {
        s.parse().unwrap()
    }

    #[test]
    fn test_ten_bb_push_fold() {
        let sol = solve(10.0);
        assert!(
            sol.exploitability_bb < 0.01,
            "exploitability {}",
            sol.exploitability_bb
        );
        // Published heads-up Nash at 10 BB: SB shoves about 58%, BB calls about 37%.
        let (push, call) = (sol.push_range_fraction(), sol.call_range_fraction());
        assert!((0.52..0.64).contains(&push), "push {push}");
        assert!((0.31..0.43).contains(&call), "call {call}");
        for strong in ["AA", "KK", "AKs", "A2s", "K9o"] {
            assert!(sol.push_frequency(class(strong)) > 0.99, "{strong}");
        }
        for strong in ["AA", "KK", "AKo", "TT"] {
            assert!(sol.call_frequency(class(strong)) > 0.99, "{strong}");
        }
        assert!(sol.call_frequency(class("72o")) < 0.01);
    }

    #[test]
    fn test_ranges_tighten_as_stacks_grow() {
        let (short, mid, deep) = (solve(5.0), solve(10.0), solve(20.0));
        assert!(short.push_range_fraction() > mid.push_range_fraction());
        assert!(mid.push_range_fraction() > deep.push_range_fraction());
        assert!(short.call_range_fraction() > mid.call_range_fraction());
        assert!(deep.push_frequency(class("72o")) < 0.01);
    }

    #[test]
    fn test_antes_widen_ranges() {
        let table = PreflopEquityTable::bundled();
        let no_ante = solve_heads_up_push_fold(15.0, 0.0, table, 800);
        let ante = solve_heads_up_push_fold(15.0, 0.125, table, 800);
        assert!(ante.push_range_fraction() > no_ante.push_range_fraction());
    }
}
