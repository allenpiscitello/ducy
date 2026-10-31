//! A styled player's least exploitable game (#265): one seat plays with a
//! style it must keep (plays a target share of its hands, its VPIP), and the
//! five others adapt to it, knowing which seat is styled. Everything else is
//! solved: the styled player loses as little as it can while keeping its
//! style, against opponents who exploit the style as well as they can (a
//! constrained equilibrium, not a fixed field).
//!
//! - **Who's styled:** each iteration the styled seat rotates through the six
//!   positions. The styled player has one strategy table (by decision, so by
//!   position); the opponents have a table for each position the styled
//!   player can be in, so they play differently against a styled UTG than a
//!   styled button.
//! - **Keeping the style:** a price `lambda` (big blinds) is added to the
//!   styled player's own payoff whenever it voluntarily plays a hand (any
//!   action but fold at its first decision that offers a fold), and only its
//!   payoff: its opponents see the real chips. `lambda` is tuned as training
//!   runs (`steer`) until the styled player's VPIP meets the target: positive
//!   to play more than it would, negative to play fewer.
//! - **The cost:** [`StyledSolver::styled_ev`] is the styled player's real
//!   result per hand, averaged over the six positions. Everyone at an all-GTO
//!   table averages zero, so this is what the style costs.
//!
//! Training as in [`super::solver`]: external-sampling MCCFR, regrets floored
//! at zero, the average weighted by iteration, lock-free atomic tables.

use std::{
    sync::atomic::{AtomicU32, AtomicU64, Ordering::Relaxed},
    thread,
};

use super::{
    equity::PreflopEquity,
    preflop::{Act, Node, PreflopGame, SEATS},
};
use crate::{
    Rng,
    holdem::{cards::Card, iso::NUM_PREFLOP_CLASSES, iso::preflop_class},
};

const C: usize = NUM_PREFLOP_CLASSES;
const MAGIC: &[u8; 8] = b"DUCY6ST\0";

fn load(a: &AtomicU32) -> f32 {
    f32::from_bits(a.load(Relaxed))
}
fn add(a: &AtomicU32, x: f32) {
    a.store((load(a) + x).to_bits(), Relaxed);
}
fn loadf(a: &AtomicU64) -> f64 {
    f64::from_bits(a.load(Relaxed))
}
fn addf(a: &AtomicU64, x: f64) {
    a.store((loadf(a) + x).to_bits(), Relaxed);
}

/// One strategy: regrets and the running average, by decision and class.
struct Table {
    regret: Vec<AtomicU32>,
    average: Vec<AtomicU32>,
}

impl Table {
    fn new(n: usize) -> Self {
        let zeros = |n: usize| (0..n).map(|_| AtomicU32::new(0)).collect();
        Self {
            regret: zeros(n),
            average: zeros(n),
        }
    }
}

pub struct StyledSolver<'a> {
    game: &'a PreflopGame,
    eq: &'a PreflopEquity,
    offsets: Vec<(usize, usize)>,
    /// Whether each decision id offers a fold (a VPIP decision).
    can_fold: Vec<Option<usize>>,
    styled: Table,
    /// The opponents' tables, one per position of the styled player.
    opponents: Vec<Table>,
    /// The price on playing a hand, in big blinds (f64 bits).
    lambda: AtomicU64,
    /// The styled player's VPIP since the last `steer`: hands played, and
    /// first decisions seen (f64 bits).
    played: AtomicU64,
    seen: AtomicU64,
    iterations: AtomicU64,
    seed: u64,
}

impl<'a> StyledSolver<'a> {
    pub fn new(game: &'a PreflopGame, eq: &'a PreflopEquity, seed: u64) -> Self {
        let mut offsets = vec![(0, 0); game.decisions as usize];
        let mut can_fold = vec![None; game.decisions as usize];
        let mut total = 0;
        for node in &game.nodes {
            if let Node::Decision { id, actions, .. } = node {
                offsets[*id as usize] = (total, actions.len());
                can_fold[*id as usize] = actions.iter().position(|(a, _)| *a == Act::Fold);
                total += actions.len() * C;
            }
        }
        Self {
            game,
            eq,
            offsets,
            can_fold,
            styled: Table::new(total),
            opponents: (0..SEATS).map(|_| Table::new(total)).collect(),
            lambda: AtomicU64::new(0f64.to_bits()),
            played: AtomicU64::new(0f64.to_bits()),
            seen: AtomicU64::new(0f64.to_bits()),
            iterations: AtomicU64::new(0),
            seed,
        }
    }

    pub fn iterations(&self) -> u64 {
        self.iterations.load(Relaxed)
    }
    pub fn lambda(&self) -> f64 {
        loadf(&self.lambda)
    }
    pub fn set_lambda(&self, x: f64) {
        self.lambda.store(x.to_bits(), Relaxed);
    }

    fn table(&self, seat: usize, styled_at: usize) -> &Table {
        if seat == styled_at {
            &self.styled
        } else {
            &self.opponents[styled_at]
        }
    }

    fn slots(&self, id: u32, class: usize) -> (usize, usize) {
        let (start, n) = self.offsets[id as usize];
        (start + class * n, n)
    }

    fn current(t: &Table, base: usize, n: usize, out: &mut [f64]) {
        let mut sum = 0.0;
        for (x, slot) in out.iter_mut().zip(&t.regret[base..base + n]) {
            *x = load(slot).max(0.0) as f64;
            sum += *x;
        }
        for x in out.iter_mut().take(n) {
            *x = if sum > 0.0 { *x / sum } else { 1.0 / n as f64 };
        }
    }

    /// The average strategy at a decision for a class: the styled player's
    /// (`styled_at` None) or, with the styled player at position
    /// `styled_at`, the opponents'.
    pub fn strategy(&self, id: u32, class: usize, styled_at: Option<usize>) -> Vec<f64> {
        let t = styled_at.map_or(&self.styled, |k| &self.opponents[k]);
        let (base, n) = self.slots(id, class);
        let w: Vec<f64> = (0..n).map(|a| load(&t.average[base + a]) as f64).collect();
        let sum: f64 = w.iter().sum();
        if sum > 0.0 {
            w.iter().map(|x| x / sum).collect()
        } else {
            vec![1.0 / n as f64; n]
        }
    }

    fn deal(rng: &mut Rng) -> [usize; SEATS] {
        let mut used = 0u64;
        let mut draw = || loop {
            let c = (rng.next_u64() % 52) as Card;
            if used & 1 << c == 0 {
                used |= 1 << c;
                return c;
            }
        };
        let mut classes = [0; SEATS];
        for c in &mut classes {
            let a = draw();
            let b = draw();
            *c = preflop_class(a, b);
        }
        classes
    }

    /// `first`: the styled player hasn't yet met a decision with a fold
    /// (where playing the hand earns the price).
    #[allow(clippy::too_many_arguments)]
    fn traverse(
        &self,
        node: usize,
        player: usize,
        styled_at: usize,
        classes: &[usize; SEATS],
        weight: f32,
        first: bool,
        rng: &mut Rng,
    ) -> f64 {
        match &self.game.nodes[node] {
            Node::End { .. } => self.game.payoffs(node, classes, self.eq)[player],
            Node::Decision { seat, id, actions } => {
                let n = actions.len();
                let t = self.table(*seat, styled_at);
                let (base, _) = self.slots(*id, classes[*seat]);
                let mut sigma = [0f64; 4];
                Self::current(t, base, n, &mut sigma);
                // The styled player's first decision with a fold: does it play?
                let vpip_here =
                    *seat == styled_at && first && self.can_fold[*id as usize].is_some();
                let fold = self.can_fold[*id as usize];
                let first_after = first && !vpip_here;
                if *seat == player {
                    if vpip_here {
                        let p = 1.0 - fold.map_or(0.0, |f| sigma[f]);
                        addf(&self.played, p);
                        addf(&self.seen, 1.0);
                    }
                    let mut values = [0f64; 4];
                    let mut value = 0.0;
                    for (a, &(_, child)) in actions.iter().enumerate() {
                        values[a] = self.traverse(
                            child as usize,
                            player,
                            styled_at,
                            classes,
                            weight,
                            first_after,
                            rng,
                        );
                        // The price on playing, in the styled player's own payoff.
                        if vpip_here && Some(a) != fold {
                            values[a] += self.lambda();
                        }
                        value += sigma[a] * values[a];
                    }
                    for (slot, v) in t.regret[base..base + n].iter().zip(values) {
                        let r = (load(slot) + (v - value) as f32).max(0.0);
                        slot.store(r.to_bits(), Relaxed);
                    }
                    value
                } else {
                    for (slot, p) in t.average[base..base + n].iter().zip(sigma) {
                        add(slot, weight * p as f32);
                    }
                    let a = rng.sample(&sigma[..n]);
                    // Past the styled player's first fold decision either way.
                    self.traverse(
                        actions[a].1 as usize,
                        player,
                        styled_at,
                        classes,
                        weight,
                        first_after,
                        rng,
                    )
                }
            }
        }
    }

    /// Runs `iterations` more iterations on `threads` threads: each deals six
    /// hands, puts the styled player in the next position round, and
    /// traverses once for every seat.
    pub fn run(&self, iterations: u64, threads: usize) {
        let start = self.iterations();
        let threads = threads.max(1) as u64;
        thread::scope(|scope| {
            for t in 0..threads {
                scope.spawn(move || {
                    let mut i = start + t;
                    while i < start + iterations {
                        let mut rng = Rng::for_iteration(self.seed, i);
                        let classes = Self::deal(&mut rng);
                        let styled_at = (i % SEATS as u64) as usize;
                        let weight = (i + 1) as f32;
                        for player in 0..SEATS {
                            self.traverse(
                                self.game.root(),
                                player,
                                styled_at,
                                &classes,
                                weight,
                                true,
                                &mut rng,
                            );
                        }
                        i += threads;
                    }
                });
            }
        });
        self.iterations.store(start + iterations, Relaxed);
    }

    /// The styled player's VPIP (current strategy) since the last call, and
    /// moves `lambda` toward the target: up by `rate` big blinds per point
    /// of VPIP short, down when over. Returns that VPIP.
    pub fn steer(&self, target: f64, rate: f64) -> f64 {
        let seen = loadf(&self.seen);
        let played = loadf(&self.played);
        self.seen.store(0f64.to_bits(), Relaxed);
        self.played.store(0f64.to_bits(), Relaxed);
        if seen <= 0.0 {
            return 0.0;
        }
        let vpip = played / seen;
        self.set_lambda(self.lambda() + rate * (target - vpip) * 100.0);
        vpip
    }

    /// The average strategy at node `node` for `seat`.
    fn avg_at(
        &self,
        node: usize,
        seat: usize,
        styled_at: usize,
        classes: &[usize; SEATS],
    ) -> Vec<f64> {
        let Node::Decision { id, .. } = &self.game.nodes[node] else {
            return Vec::new();
        };
        self.strategy(*id, classes[seat], (seat != styled_at).then_some(styled_at))
    }

    /// One play-out from `node` with the average strategies, actions
    /// sampled: the styled player's result, and the chance it plays the hand
    /// (exact at its first decision with a fold).
    fn value(
        &self,
        node: usize,
        styled_at: usize,
        classes: &[usize; SEATS],
        first: bool,
        rng: &mut Rng,
    ) -> (f64, f64) {
        let mut node = node;
        let mut first = first;
        let mut vpip = 0.0;
        loop {
            match &self.game.nodes[node] {
                Node::End { .. } => {
                    return (self.game.payoffs(node, classes, self.eq)[styled_at], vpip);
                }
                Node::Decision { seat, id, actions } => {
                    let sigma = self.avg_at(node, *seat, styled_at, classes);
                    if *seat == styled_at
                        && first
                        && let Some(f) = self.can_fold[*id as usize]
                    {
                        vpip = 1.0 - sigma[f];
                        first = false;
                    }
                    node = actions[rng.sample(&sigma)].1 as usize;
                }
            }
        }
    }

    /// The styled player's real result per hand (big blinds) and VPIP with
    /// the average strategies, over `deals` random deals in each position
    /// (play-outs sampled), on `threads` threads: (ev, ev's standard error,
    /// vpip).
    pub fn styled_ev(&self, deals: u64, seed: u64, threads: usize) -> (f64, f64, f64) {
        let sum = AtomicU64::new(0f64.to_bits());
        let sq = AtomicU64::new(0f64.to_bits());
        let vp = AtomicU64::new(0f64.to_bits());
        let threads = threads.max(1) as u64;
        thread::scope(|scope| {
            for t in 0..threads {
                let (sum, sq, vp) = (&sum, &sq, &vp);
                scope.spawn(move || {
                    let (mut s, mut q, mut v) = (0.0, 0.0, 0.0);
                    let mut i = t;
                    while i < deals {
                        let mut rng = Rng::for_iteration(seed, i);
                        let classes = Self::deal(&mut rng);
                        // One deal, the styled player in every position.
                        let mut hand = 0.0;
                        for k in 0..SEATS {
                            let (ev, p) = self.value(self.game.root(), k, &classes, true, &mut rng);
                            hand += ev / SEATS as f64;
                            v += p / SEATS as f64;
                        }
                        s += hand;
                        q += hand * hand;
                        i += threads;
                    }
                    addf(sum, s);
                    addf(sq, q);
                    addf(vp, v);
                });
            }
        });
        let n = deals as f64;
        let mean = loadf(&sum) / n;
        let var = (loadf(&sq) / n - mean * mean).max(0.0);
        (mean, (var / n).sqrt(), loadf(&vp) / n)
    }

    /// The tables, to resume training or read the strategies later.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend(self.game.stack.to_le_bytes());
        out.extend((self.styled.regret.len() as u64).to_le_bytes());
        out.extend(self.iterations().to_le_bytes());
        out.extend(self.lambda().to_le_bytes());
        for t in std::iter::once(&self.styled).chain(&self.opponents) {
            for x in t.regret.iter().chain(&t.average) {
                out.extend(x.load(Relaxed).to_le_bytes());
            }
        }
        out
    }

    /// Loads tables saved by [`Self::to_bytes`] for the same game.
    pub fn load(&self, bytes: &[u8]) -> Result<(), String> {
        let n = self.styled.regret.len();
        let head = 8 + 4 + 8 + 8 + 8;
        if bytes.len() != head + 4 * 2 * n * (SEATS + 1) || &bytes[..8] != MAGIC {
            return Err("not a styled-player checkpoint for this game".into());
        }
        let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().expect("4"));
        let u64_at = |i: usize| u64::from_le_bytes(bytes[i..i + 8].try_into().expect("8"));
        if u32_at(8) != self.game.stack || u64_at(12) as usize != n {
            return Err("checkpoint for another depth".into());
        }
        self.iterations.store(u64_at(20), Relaxed);
        self.set_lambda(f64::from_le_bytes(bytes[28..36].try_into().expect("8")));
        let mut at = head;
        for t in std::iter::once(&self.styled).chain(&self.opponents) {
            for slot in t.regret.iter().chain(&t.average) {
                slot.store(u32_at(at), Relaxed);
                at += 4;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_price_on_playing_moves_the_styled_vpip_and_costs_chips() {
        let eq = PreflopEquity::compute(200, 3, 4);
        let game = PreflopGame::new(40);
        // No price: about as loose as GTO.
        let free = StyledSolver::new(&game, &eq, 3);
        free.run(600_000, 8);
        let (ev0, se0, v0) = free.styled_ev(200_000, 9, 8);
        // A big price on playing: much looser, and it loses chips to
        // opponents who adapt.
        let loose = StyledSolver::new(&game, &eq, 3);
        loose.set_lambda(2.0);
        loose.run(600_000, 8);
        let (ev1, se1, v1) = loose.styled_ev(200_000, 9, 8);
        assert!(v1 > v0 + 0.15, "VPIP {v0:.2} -> {v1:.2}");
        assert!(ev1 + 2.0 * (se0 + se1) < ev0, "EV {ev0:.3} -> {ev1:.3}");
        // Saved and loaded, the same strategy.
        let again = StyledSolver::new(&game, &eq, 3);
        again.load(&loose.to_bytes()).expect("load");
        assert_eq!(again.lambda(), 2.0);
        assert_eq!(again.strategy(0, 0, None), loose.strategy(0, 0, None));
    }
}
