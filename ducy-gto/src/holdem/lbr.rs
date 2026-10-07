//! Local Best Response (Lisý and Bowling 2017): a lower bound on how much a
//! strategy can be exploited, for games too big to compute a best response
//! exactly.
//!
//! LBR plays one seat against the blueprint with exact cards and the same
//! bet menu. It tracks the blueprint's range: every hand it could hold,
//! weighted by how likely its actions so far were with that hand. At each of
//! its own decisions LBR assumes it will check or call to the end
//! afterwards, and picks the action with the best expected value:
//!
//! - check or call: `wp · (pot + call) − call`
//! - bet or raise `b`: `fp · pot + (1 − fp) · (wp' · (pot + 2b) − b)`, where `fp`
//!   is how often the range folds to it (from the blueprint itself) and
//!   `wp'` is LBR's equity against the hands that don't fold
//! - fold: 0
//!
//! Equity against the range is exact on the river and sampled over runouts
//! before it. What LBR wins on average, over both seats, is how much the
//! blueprint can be exploited at the least; a true best response wins more.
//!
//! [`local_best_response_with`] measures the bot that solves the river in
//! real time instead: on reaching the river it solves the subgame the way
//! the bot does (both ranges, the gadget against the blueprint's river), and
//! LBR plays against that solution there. LBR's own bets are on the menu, so
//! the bot never needs to re-solve.
//!
//! [`lbr_hands_solving`] adds depth-limited turn solving the same way: on
//! reaching the turn the bot solves its betting (LBR picking among the
//! biased river continuations, the gadget against the blueprint's turn), and
//! the river solve then starts from the ranges that solution played.

use std::collections::HashMap;

use super::{
    abstraction::CardAbstraction,
    blueprint::Blueprint,
    cards::{Card, NUM_CARDS, bit, hole_cards, hole_index, mask, score},
    hunl::{Hunl, HunlAction, HunlState},
    range::{Range, StreetBuckets, board_for},
    river::{RiverSolver, RiverTree},
    turn::{Bias, TurnSolver},
};
use crate::{
    game::{Game, Turn},
    rng::Rng,
};

/// Runouts sampled for equity before the river.
const RUNOUTS: usize = 24;

/// The result of an LBR run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LbrResult {
    pub hands: usize,
    /// LBR's average winnings in milli-big-blinds per hand (both seats).
    pub mbb_per_hand: f64,
    /// 95% confidence half-width of that.
    pub ci95: f64,
}

/// Runs LBR for `hands` hands (half in each seat) against `blueprint`.
pub fn local_best_response(
    game: &Hunl,
    cards: &CardAbstraction,
    blueprint: &Blueprint,
    hands: usize,
    seed: u64,
) -> LbrResult {
    local_best_response_with(game, cards, blueprint, hands, seed, 0)
}

/// LBR against the blueprint with the river solved in real time,
/// `river_iterations` per solve (0 plays the blueprint's river).
pub fn local_best_response_with(
    game: &Hunl,
    cards: &CardAbstraction,
    blueprint: &Blueprint,
    hands: usize,
    seed: u64,
    river_iterations: usize,
) -> LbrResult {
    LbrResult::of(&lbr_hands(
        game,
        cards,
        blueprint,
        hands,
        seed,
        river_iterations,
    ))
}

/// LBR's result in each hand, in milli-big-blinds. Hand `i` is dealt and
/// played the same way up to the river whatever `river_iterations` is, so
/// runs with and without river solving can be compared hand by hand.
pub fn lbr_hands(
    game: &Hunl,
    cards: &CardAbstraction,
    blueprint: &Blueprint,
    hands: usize,
    seed: u64,
    river_iterations: usize,
) -> Vec<f64> {
    lbr_hands_solving(game, cards, blueprint, hands, seed, 0, river_iterations)
}

/// [`lbr_hands`] against a bot that also solves the turn, depth-limited,
/// with `turn_iterations` per solve (0 plays the blueprint's turn) and 8
/// rivers dealt per iteration, as `TurnSolving::new` does. Hand `i` is
/// dealt and played the same way up to the turn whatever the iterations.
pub fn lbr_hands_solving(
    game: &Hunl,
    cards: &CardAbstraction,
    blueprint: &Blueprint,
    hands: usize,
    seed: u64,
    turn_iterations: usize,
    river_iterations: usize,
) -> Vec<f64> {
    let one = |i: usize| -> f64 {
        let mut rng = Rng::for_iteration(seed, i as u64);
        let seat = i % 2;
        let mut opp = Opponent {
            blueprint,
            cards,
            turn: None,
            river: None,
            turn_iterations,
            iterations: river_iterations,
            // Not drawn from `rng`, so the hand plays the same up to the turn.
            seed: Rng::for_iteration(seed ^ 0x7475_726e, i as u64).next_u64(),
        };
        play_hand(game, cards, &mut opp, seat, &mut rng) / game.config.big_blind as f64 * 1000.0
    };
    #[cfg(feature = "parallel")]
    let results: Vec<f64> = {
        use rayon::prelude::*;
        (0..hands).into_par_iter().map(one).collect()
    };
    #[cfg(not(feature = "parallel"))]
    let results: Vec<f64> = (0..hands).map(one).collect();
    results
}

impl LbrResult {
    /// The mean and its 95% interval over per-hand results.
    pub fn of(results: &[f64]) -> Self {
        let n = results.len().max(1) as f64;
        let mean = results.iter().sum::<f64>() / n;
        let var = results.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0);
        Self {
            hands: results.len(),
            mbb_per_hand: mean,
            ci95: 1.96 * (var / n).sqrt(),
        }
    }
}

/// The strategy LBR plays against: the blueprint, or on the turn and river
/// a solve of it with the tree's nodes mapped to the subgame's.
struct Opponent<'a> {
    blueprint: &'a Blueprint,
    cards: &'a CardAbstraction,
    turn: Option<(TurnSolver, HashMap<u32, u32>)>,
    river: Option<(RiverSolver, HashMap<u32, u32>)>,
    /// Iterations per turn and river solve (0 for none).
    turn_iterations: usize,
    iterations: usize,
    /// Draws the turn solver's sampled rivers.
    seed: u64,
}

impl Opponent<'_> {
    /// Whether the bot solves any street, so both ranges must be tracked.
    fn solving(&self) -> bool {
        self.turn_iterations > 0 || self.iterations > 0
    }

    fn solved(&self, node: u32) -> bool {
        self.river.as_ref().is_some_and(|r| r.1.contains_key(&node))
            || self.turn.as_ref().is_some_and(|t| t.1.contains_key(&node))
    }

    /// Action probabilities at tree node `node` for hole index `hole` (in
    /// `bucket`).
    fn probs(&self, node: u32, hole: usize, bucket: u16) -> Vec<f64> {
        if let Some((s, map)) = &self.river
            && let Some(&sub) = map.get(&node)
            && let Some(p) = s.probs(sub, hole)
        {
            return p;
        }
        if let Some((s, map)) = &self.turn
            && let Some(&sub) = map.get(&node)
            && let Some(p) = s.probs(sub, hole)
        {
            return p;
        }
        self.blueprint.probs(node, bucket)
    }

    /// Updates `range` for action `a` at `node`.
    fn update(&self, range: &mut Range, node: u32, a: usize, buckets: &[u16]) {
        if self.solved(node) {
            range.update_by(|h| match buckets[h] {
                u16::MAX => 0.0,
                b => self.probs(node, h, b)[a],
            });
        } else {
            range.update(self.blueprint, node, a, buckets);
        }
    }

    /// Solves the river from tree node `node`, the way the bot does with
    /// `bot` playing the solution.
    fn solve(
        &mut self,
        game: &Hunl,
        s: &HunlState,
        ranges: &[Range; 2],
        bot: usize,
        buckets: &[u16],
    ) {
        if self.iterations == 0 || ranges.iter().any(|r| r.total() <= 0.0) {
            return;
        }
        let root = &game.tree.nodes[s.node as usize].betting;
        let mut solver = RiverSolver::new(
            s.board,
            root,
            &game.config,
            &[],
            [&ranges[0].weight, &ranges[1].weight],
        );
        let reference = solver.blueprint_strategy(self.blueprint, &game.tree, s.node, buckets);
        let target = solver.best_response(1 - bot, &reference);
        solver.set_gadget(1 - bot, target);
        solver.run(self.iterations);
        let map = node_map(game, s.node, &solver.tree);
        self.river = Some((solver, map));
    }

    /// Solves the turn from tree node `node`, the way the bot does with
    /// `bot` playing the solution and the other player choosing among the
    /// river continuations.
    fn solve_turn(
        &mut self,
        game: &Hunl,
        s: &HunlState,
        ranges: &[Range; 2],
        bot: usize,
        buckets: &[u16],
    ) {
        if self.turn_iterations == 0 || ranges.iter().any(|r| r.total() <= 0.0) {
            return;
        }
        let root = &game.tree.nodes[s.node as usize].betting;
        let board: [Card; 4] = s.board[..4].try_into().expect("a turn board");
        let mut solver = TurnSolver::from_blueprint(
            board,
            root,
            &game.config,
            &[],
            [&ranges[0].weight, &ranges[1].weight],
            self.cards,
            self.blueprint,
            &game.tree,
            s.node,
        );
        solver.set_chooser(1 - bot, &Bias::ALL);
        solver.set_river_samples(8, self.seed);
        let reference = solver.blueprint_strategy(self.blueprint, &game.tree, s.node, buckets);
        let target = solver.best_response(1 - bot, &reference);
        solver.set_gadget(1 - bot, target);
        solver.run(self.turn_iterations);
        let map = node_map(game, s.node, &solver.tree);
        self.turn = Some((solver, map));
    }
}

/// The blueprint tree's nodes from `root` mapped to a subgame tree's: with
/// the same betting and menu they match node for node, down to the
/// subgame's leaves.
fn node_map(game: &Hunl, root: u32, sub: &RiverTree) -> HashMap<u32, u32> {
    let mut map = HashMap::new();
    let mut stack = vec![(root, 0u32)];
    while let Some((t, r)) = stack.pop() {
        let (x, y) = (&game.tree.nodes[t as usize], &sub.nodes[r as usize]);
        if x.actions != y.actions {
            continue;
        }
        map.insert(t, r);
        stack.extend(x.children.iter().copied().zip(y.children.iter().copied()));
    }
    map
}

/// One hand, LBR in `seat` (0 is the button). Returns LBR's chip result.
fn play_hand(
    game: &Hunl,
    cards: &CardAbstraction,
    opp: &mut Opponent,
    seat: usize,
    rng: &mut Rng,
) -> f64 {
    let mut s = game.sample_chance(&game.root(), rng);
    let me = s.hole[seat];
    // Public ranges: both players' as the other sees them.
    let mut ranges = [Range::new(0), Range::new(0)];
    let mut buckets = StreetBuckets::default();
    let mut street = usize::MAX;
    loop {
        match game.turn(&s) {
            Turn::Terminal => {
                let u = game.utility(&s);
                return if seat == 0 { u } else { -u };
            }
            Turn::Chance => unreachable!("only the root deals"),
            Turn::Player(p) => {
                let b = game.betting(&s).clone();
                let board = board_for(b.street, &s.board);
                let bk = buckets.on(cards, b.street, &s.board);
                if b.street != street {
                    street = b.street;
                    for r in &mut ranges {
                        r.remove(mask(board));
                    }
                    match street {
                        2 => opp.solve_turn(game, &s, &ranges, 1 - seat, bk),
                        3 => opp.solve(game, &s, &ranges, 1 - seat, bk),
                        _ => {}
                    }
                }
                let a = if p == seat {
                    // LBR's view of the opponent's range leaves out its cards.
                    let mut view = ranges[1 - seat].clone();
                    view.remove(bit(me[0]) | bit(me[1]));
                    let a = lbr_action(game, opp, &s, &view.weight, bk, me, board, rng);
                    if opp.solving() && street < 3 {
                        opp.update(&mut ranges[seat], s.node, a, bk);
                    }
                    a
                } else {
                    // The bot acts with its real hand; LBR updates the range
                    // with every hand's probability of that action.
                    let hole = hole_index(s.hole[p][0], s.hole[p][1]);
                    let probs = opp.probs(s.node, hole, s.buckets[p][b.street]);
                    let a = rng.sample(&probs);
                    opp.update(&mut ranges[p], s.node, a, bk);
                    a
                };
                s = game.apply(&s, a);
            }
        }
    }
}

/// LBR's equity against the range (weights `w`), exact on the river and
/// sampled over runouts before it.
fn equity(me: [Card; 2], board: &[Card], w: &[f64], rng: &mut Rng) -> f64 {
    let known = mask(board) | bit(me[0]) | bit(me[1]);
    let runs = if board.len() == 5 { 1 } else { RUNOUTS };
    let free: Vec<Card> = (0..NUM_CARDS as Card)
        .filter(|&c| known & bit(c) == 0)
        .collect();
    let (mut won, mut total) = (0.0, 0.0);
    let mut full = [0u8; 5];
    full[..board.len()].copy_from_slice(board);
    for _ in 0..runs {
        // Deal the rest of the board.
        let mut used = known;
        for c in full.iter_mut().skip(board.len()) {
            loop {
                let x = free[(rng.next_u64() % free.len() as u64) as usize];
                if used & bit(x) == 0 {
                    used |= bit(x);
                    *c = x;
                    break;
                }
            }
        }
        let bm = mask(&full);
        let mine = score(bm | bit(me[0]) | bit(me[1]));
        for (h, &wt) in w.iter().enumerate() {
            if wt <= 0.0 {
                continue;
            }
            let (a, b) = hole_cards(h);
            if bm & (bit(a) | bit(b)) != 0 {
                continue;
            }
            let theirs = score(bm | bit(a) | bit(b));
            won += wt
                * match mine.cmp(&theirs) {
                    std::cmp::Ordering::Greater => 1.0,
                    std::cmp::Ordering::Equal => 0.5,
                    std::cmp::Ordering::Less => 0.0,
                };
            total += wt;
        }
    }
    if total > 0.0 { won / total } else { 0.5 }
}

#[allow(clippy::too_many_arguments)]
fn lbr_action(
    game: &Hunl,
    opp: &Opponent,
    s: &HunlState,
    range: &[f64],
    buckets: &[u16],
    me: [Card; 2],
    board: &[Card],
    rng: &mut Rng,
) -> usize {
    let node = &game.tree.nodes[s.node as usize];
    let b = &node.betting;
    let pot = b.pot() as f64;
    let call = b.to_call() as f64;
    let wp = equity(me, board, range, rng);
    let mut best = (0usize, f64::NEG_INFINITY);
    for (i, a) in node.actions.iter().enumerate() {
        let ev = match *a {
            HunlAction::Fold => 0.0,
            HunlAction::Check | HunlAction::Call => wp * (pot + call) - call,
            HunlAction::Bet(to) | HunlAction::Raise(to) => {
                let added = (to - b.street_bet[b.to_act]) as f64;
                // How the range answers: its fold probability at the child.
                let child = node.children[i];
                let reply = &game.tree.nodes[child as usize];
                let fold = reply.actions.iter().position(|x| *x == HunlAction::Fold);
                let mut calling = range.to_vec();
                let (mut folded, mut total) = (0.0, 0.0);
                for (h, c) in calling.iter_mut().enumerate() {
                    let w = range[h];
                    if w <= 0.0 {
                        continue;
                    }
                    let f = fold.map_or(0.0, |k| opp.probs(child, h, buckets[h])[k]);
                    folded += w * f;
                    total += w;
                    *c = w * (1.0 - f);
                }
                let fp = if total > 0.0 { folded / total } else { 0.0 };
                let wp_call = equity(me, board, &calling, rng);
                // The opponent calls our full raise: `added` from us and the
                // same increase on top of its call from them.
                fp * pot + (1.0 - fp) * (wp_call * (pot + 2.0 * added - call) - added)
            }
        };
        if ev > best.1 {
            best = (i, ev);
        }
    }
    best.0
}
