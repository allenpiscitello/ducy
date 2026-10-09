//! Real-time river solving for heads-up PLO (#132).
//!
//! Hold'em's river solver keeps every one of the 1,081 hands the board
//! allows. A PLO range has over 100,000, so here each range is **sampled**:
//!
//! - **Ranges** ([`sample_range`]): candidate hands are drawn from the
//!   cards the player can hold, each weighted by how likely the blueprint
//!   was to take that player's actions before the river with it, and then
//!   resampled in proportion to those weights (importance resampling), so
//!   the hands kept are the likely ones. The bot's own hand is always kept,
//!   as hand 0 of its range.
//! - **The solve** ([`PloRiverSolver`]) is Discounted CFR over the river's
//!   betting in real chips (Hold'em's [`RiverTree`]), on vectors over each
//!   range's hands. Sampled hands share cards with each other often (about a
//!   third of random pairs on a river), so showdowns and folds use a
//!   precomputed table of which hand wins each pair and which pairs can't
//!   both be dealt.
//! - **Safety** comes from Hold'em's resolving gadget: each opponent hand
//!   may take what a best response gets against the blueprint's own river
//!   strategy instead of playing, so solving shouldn't make the bot more
//!   exploitable on the river than the blueprint, as far as the samples and
//!   the solve's convergence allow.
//!
//! [`best_response_value`] measures any river strategy against a separate,
//! larger sample of the opponent's range, which is how solving is compared
//! with the blueprint (see the `plo_river` example).

use std::{collections::HashMap, time::Duration};

use super::{
    abstraction::{BoardView, PloAbstraction},
    showdown::{PairTable, draw},
};
use crate::{
    holdem::{
        blueprint::Blueprint,
        cards::{Card, mask},
        hunl::{Betting, BettingTree, HunlAction, HunlConfig},
        river::{
            Discount, Gadget, RiverTree, Strategy, map_probs, match_blueprint, normalized, passive,
            regret_matching, same_spot,
        },
    },
    rng::Rng,
};

/// How hard the PLO bot solves the river.
#[derive(Clone, Debug, PartialEq)]
pub struct PloRiverSolving {
    /// Iterations per solve.
    pub iterations: usize,
    /// Hands sampled for each range.
    pub hands: usize,
    /// Stop early after this long (not on WebAssembly, which has no clock).
    pub time: Option<Duration>,
}

impl PloRiverSolving {
    /// `iterations` per solve, 256 hands per range.
    pub fn new(iterations: usize) -> Self {
        Self {
            iterations,
            hands: 256,
            time: None,
        }
    }
}

impl Default for PloRiverSolving {
    /// 200 iterations, 256 hands per range.
    fn default() -> Self {
        Self::new(200)
    }
}

/// A sampled river range: four-card hands and their weights (summing to 1).
#[derive(Clone, Debug, PartialEq)]
pub struct RiverRange {
    pub hands: Vec<[Card; 4]>,
    pub weights: Vec<f32>,
}

/// The blueprint's probability that `player` took its actions in `steps`
/// (`(node, action)`, before the river) holding `hole`, with `views` the
/// flop and turn boards' views.
pub fn reach_weight(
    cards: &PloAbstraction,
    blueprint: &Blueprint,
    tree: &BettingTree,
    steps: &[(u32, usize)],
    player: usize,
    hole: &[Card],
    views: &[BoardView; 2],
) -> f64 {
    let mut w = 1.0;
    let mut pre = None;
    for &(node, a) in steps {
        let b = &tree.nodes[node as usize].betting;
        if b.to_act != player || b.street > 2 {
            continue;
        }
        let bucket = match b.street {
            0 => *pre.get_or_insert_with(|| cards.preflop_bucket(hole)),
            s => cards.bucket_on(hole, &views[s - 1]),
        };
        w *= blueprint.probs(node, bucket)[a];
        if w == 0.0 {
            break;
        }
    }
    w
}

/// Player `player`'s range at the start of the river on `board`, sampled:
/// `4 * n` candidates from the cards outside `blocked`, weighted by
/// [`reach_weight`], resampled to `n` hands of equal weight. `first`, if
/// given, is kept as hand 0 (the bot's own).
#[allow(clippy::too_many_arguments)]
pub fn sample_range(
    cards: &PloAbstraction,
    blueprint: &Blueprint,
    tree: &BettingTree,
    steps: &[(u32, usize)],
    player: usize,
    board: &[Card; 5],
    blocked: u64,
    n: usize,
    first: Option<[Card; 4]>,
    rng: &mut Rng,
) -> RiverRange {
    let views = [BoardView::new(&board[..3]), BoardView::new(&board[..4])];
    let mut hands: Vec<[Card; 4]> = Vec::with_capacity(n);
    hands.extend(first);
    let want = n.saturating_sub(hands.len());
    let candidates: Vec<([Card; 4], f64)> = (0..4 * want.max(1))
        .map(|_| {
            let mut used = blocked | mask(board);
            let h: [Card; 4] = std::array::from_fn(|_| draw(&mut used, rng));
            let w = reach_weight(cards, blueprint, tree, steps, player, &h, &views);
            (h, w)
        })
        .collect();
    let total: f64 = candidates.iter().map(|c| c.1).sum();
    if total > 0.0 {
        // Systematic resampling: `want` evenly spaced points through the
        // cumulative weights.
        let step = total / want.max(1) as f64;
        let mut at = rng.next_f64() * step;
        let mut acc = 0.0;
        for (h, w) in &candidates {
            acc += w;
            while at < acc && hands.len() < n {
                hands.push(*h);
                at += step;
            }
        }
    }
    while hands.len() < n {
        // No weight anywhere (a line the blueprint never takes): uniform.
        let mut used = blocked | mask(board);
        hands.push(std::array::from_fn(|_| draw(&mut used, rng)));
    }
    let weights = vec![1.0 / n as f32; n];
    RiverRange { hands, weights }
}

/// For two lists of hands on a river: which hand wins each pair, from the
/// first list's side (1, 0 tie, -1), and 0 where they share a card.
#[derive(Clone, Debug)]
struct Matchups {
    rows: usize,
    cols: usize,
    sign: Vec<i8>,
    compat: Vec<i8>,
}

impl Matchups {
    fn new(board: &[Card; 5], a: &[[Card; 4]], b: &[[Card; 4]]) -> Self {
        let table = PairTable::new(board);
        let score = |h: &[Card; 4]| table.score(h);
        let sa: Vec<u32> = a.iter().map(score).collect();
        let sb: Vec<u32> = b.iter().map(score).collect();
        let ma: Vec<u64> = a.iter().map(|h| mask(h)).collect();
        let mb: Vec<u64> = b.iter().map(|h| mask(h)).collect();
        let mut sign = vec![0i8; a.len() * b.len()];
        let mut compat = vec![0i8; a.len() * b.len()];
        for i in 0..a.len() {
            for j in 0..b.len() {
                if ma[i] & mb[j] == 0 {
                    compat[i * b.len() + j] = 1;
                    sign[i * b.len() + j] = sa[i].cmp(&sb[j]) as i8;
                }
            }
        }
        Self {
            rows: a.len(),
            cols: b.len(),
            sign,
            compat,
        }
    }

    /// `out[i] = amount * Σ_j m[i][j] * x[j]` for the first list's hands
    /// (`transpose` false), or the second's with signs flipped for showdowns
    /// (`transpose` true).
    fn apply(&self, showdown: bool, transpose: bool, x: &[f32], amount: f32, out: &mut [f32]) {
        let m = if showdown { &self.sign } else { &self.compat };
        let flip = if showdown && transpose { -1.0 } else { 1.0 };
        out.fill(0.0);
        if transpose {
            for i in 0..self.rows {
                let xi = x[i];
                if xi == 0.0 {
                    continue;
                }
                let row = &m[i * self.cols..(i + 1) * self.cols];
                for (o, &s) in out.iter_mut().zip(row) {
                    *o += s as f32 * xi;
                }
            }
        } else {
            for (i, o) in out.iter_mut().enumerate() {
                let row = &m[i * self.cols..(i + 1) * self.cols];
                *o = row.iter().zip(x).map(|(&s, &v)| s as f32 * v).sum();
            }
        }
        for o in out.iter_mut() {
            *o *= amount * flip;
        }
    }
}

/// The value `p`'s hands get at a terminal node against `opp` (the other
/// player's reach), or `None` if the node isn't terminal. `p` is list 0 of
/// `m` when `p_first`.
fn terminal(
    b: &Betting,
    p: usize,
    m: &Matchups,
    p_first: bool,
    opp: &[f32],
    out: &mut [f32],
) -> bool {
    if let Some(f) = b.folded {
        let amount = if f == p {
            -(b.contributed[p] as f32)
        } else {
            b.contributed[f] as f32
        };
        m.apply(false, !p_first, opp, amount, out);
        return true;
    }
    if b.is_over() {
        let amount = b.contributed[0].min(b.contributed[1]) as f32;
        m.apply(true, !p_first, opp, amount, out);
        return true;
    }
    false
}

/// A vectorized CFR solver for one PLO river subgame.
#[derive(Clone, Debug)]
pub struct PloRiverSolver {
    pub tree: RiverTree,
    pub ranges: [RiverRange; 2],
    /// Player 0's hands against player 1's.
    matchups: Matchups,
    regret: Vec<Vec<f32>>,
    sum: Vec<Vec<f32>>,
    gadget: Option<Gadget>,
    pub iterations: usize,
}

impl PloRiverSolver {
    /// A solver for the river from `root` on `board`, with both players'
    /// sampled ranges (player 0 the button) and `path` forced into the tree.
    pub fn new(
        board: &[Card; 5],
        root: &Betting,
        config: &HunlConfig,
        path: &[HunlAction],
        ranges: [RiverRange; 2],
    ) -> Self {
        let tree = RiverTree::build(root, config, path);
        let matchups = Matchups::new(board, &ranges[0].hands, &ranges[1].hands);
        let size = |x: &crate::holdem::river::RiverNode| {
            if x.actions.is_empty() {
                0
            } else {
                x.actions.len() * ranges[x.betting.to_act].hands.len()
            }
        };
        Self {
            regret: tree.nodes.iter().map(|x| vec![0.0; size(x)]).collect(),
            sum: tree.nodes.iter().map(|x| vec![0.0; size(x)]).collect(),
            tree,
            ranges,
            matchups,
            gadget: None,
            iterations: 0,
        }
    }

    fn hands(&self, p: usize) -> usize {
        self.ranges[p].hands.len()
    }

    /// Adds the resolving gadget for `player`: each of their hands may take
    /// `value` (counterfactual, as [`PloRiverSolver::best_response`] gives)
    /// instead of playing.
    pub fn set_gadget(&mut self, player: usize, value: Vec<f32>) {
        self.gadget = Some(Gadget::new(player, &self.ranges[player].weights, value));
    }

    /// Runs up to `iterations` more iterations, stopping early after `time`.
    pub fn run(&mut self, iterations: usize, time: Option<Duration>) {
        #[cfg(not(target_arch = "wasm32"))]
        let start = std::time::Instant::now();
        for _ in 0..iterations {
            self.iterate();
            #[cfg(not(target_arch = "wasm32"))]
            if time.is_some_and(|t| start.elapsed() >= t) {
                break;
            }
        }
        #[cfg(target_arch = "wasm32")]
        let _ = time;
    }

    /// One iteration: an update for each player in turn.
    pub fn iterate(&mut self) {
        self.iterations += 1;
        let d = Discount::at(self.iterations);
        for p in 0..2 {
            let mut reach = [
                self.ranges[0].weights.clone(),
                self.ranges[1].weights.clone(),
            ];
            let enter = self.gadget.as_ref().map(|g| {
                let e = g.enter();
                reach[g.player] = g.reach(&e);
                e
            });
            let v = self.walk(0, p, [&reach[0], &reach[1]], &d);
            if let (Some(g), Some(e)) = (self.gadget.as_mut(), enter)
                && g.player == p
            {
                g.update(&v, &e, &d);
            }
        }
    }

    fn walk(&mut self, node: usize, p: usize, reach: [&[f32]; 2], d: &Discount) -> Vec<f32> {
        let n = self.hands(p);
        let mut out = vec![0f32; n];
        let b = &self.tree.nodes[node].betting;
        if terminal(b, p, &self.matchups, p == 0, reach[1 - p], &mut out) {
            return out;
        }
        let q = b.to_act;
        let nq = self.hands(q);
        let k = self.tree.nodes[node].actions.len();
        let sigma = regret_matching(&self.regret[node], nq);
        let children = self.tree.nodes[node].children.clone();
        if q != p {
            for (a, &child) in children.iter().enumerate() {
                let s = &sigma[a * nq..(a + 1) * nq];
                let r: Vec<f32> = reach[q].iter().zip(s).map(|(x, y)| x * y).collect();
                let mut next = reach;
                next[q] = &r;
                let v = self.walk(child as usize, p, next, d);
                out.iter_mut().zip(&v).for_each(|(o, x)| *o += x);
            }
            return out;
        }
        let mut values = vec![0f32; k * n];
        for (a, &child) in children.iter().enumerate() {
            let s = &sigma[a * n..(a + 1) * n];
            let r: Vec<f32> = reach[p].iter().zip(s).map(|(x, y)| x * y).collect();
            let mut next = reach;
            next[p] = &r;
            let v = self.walk(child as usize, p, next, d);
            for h in 0..n {
                out[h] += s[h] * v[h];
            }
            values[a * n..(a + 1) * n].copy_from_slice(&v);
        }
        let (regret, sum) = (&mut self.regret[node], &mut self.sum[node]);
        for a in 0..k {
            for h in 0..n {
                let i = a * n + h;
                regret[i] = d.regret(regret[i]) + values[i] - out[h];
                sum[i] = sum[i] * d.sum + reach[p][h] * sigma[i];
            }
        }
        out
    }

    /// The average strategy at `node`, action-major over the acting
    /// player's hands.
    pub fn average_at(&self, node: u32) -> Vec<f32> {
        let x = &self.tree.nodes[node as usize];
        if x.actions.is_empty() {
            return Vec::new();
        }
        normalized(
            &self.sum[node as usize],
            self.hands(x.betting.to_act),
            |v| v,
        )
    }

    /// The average strategy everywhere.
    pub fn average(&self) -> Strategy {
        (0..self.tree.nodes.len() as u32)
            .map(|i| self.average_at(i))
            .collect()
    }

    /// The average strategy at `node` for hand `h` of the acting player.
    pub fn probs(&self, node: u32, h: usize) -> Vec<f64> {
        let x = &self.tree.nodes[node as usize];
        let n = self.hands(x.betting.to_act);
        let k = x.actions.len();
        let s = &self.sum[node as usize];
        let p: Vec<f64> = (0..k).map(|a| s[a * n + h] as f64).collect();
        let total: f64 = p.iter().sum();
        if total > 0.0 {
            p.iter().map(|x| x / total).collect()
        } else {
            vec![1.0 / k as f64; k]
        }
    }

    /// Player `p`'s best-response counterfactual values, by hand, against
    /// the other player playing `strategy` from their range.
    pub fn best_response(&self, p: usize, strategy: &Strategy) -> Vec<f32> {
        br(
            &self.tree,
            0,
            p,
            &self.matchups,
            p == 0,
            [self.hands(0), self.hands(1)],
            &self.ranges[1 - p].weights,
            strategy,
        )
    }

    /// The blueprint's river strategy carried over to this subgame for
    /// player `p`'s hands (see [`blueprint_strategy`]).
    pub fn blueprint_strategy(
        &self,
        cards: &PloAbstraction,
        board: &[Card; 5],
        blueprint: &Blueprint,
        bp_tree: &BettingTree,
        bp_root: u32,
    ) -> Strategy {
        blueprint_strategy(
            &self.tree,
            [&self.ranges[0].hands, &self.ranges[1].hands],
            cards,
            board,
            blueprint,
            bp_tree,
            bp_root,
        )
    }
}

/// Best-response values for player `p` (list 0 of `m` when `p_first`)
/// against the other player's `strategy`, reaching with `opp`.
#[allow(clippy::too_many_arguments)]
fn br(
    tree: &RiverTree,
    node: usize,
    p: usize,
    m: &Matchups,
    p_first: bool,
    sizes: [usize; 2],
    opp: &[f32],
    strategy: &Strategy,
) -> Vec<f32> {
    let mut out = vec![0f32; sizes[p]];
    let x = &tree.nodes[node];
    if terminal(&x.betting, p, m, p_first, opp, &mut out) {
        return out;
    }
    if x.betting.to_act == p {
        out.fill(f32::NEG_INFINITY);
        for &child in &x.children {
            let v = br(tree, child as usize, p, m, p_first, sizes, opp, strategy);
            out.iter_mut().zip(&v).for_each(|(o, x)| *o = o.max(*x));
        }
    } else {
        let nq = sizes[1 - p];
        for (a, &child) in x.children.iter().enumerate() {
            let s = &strategy[node][a * nq..(a + 1) * nq];
            let r: Vec<f32> = opp.iter().zip(s).map(|(x, y)| x * y).collect();
            if r.iter().all(|&x| x == 0.0) {
                continue;
            }
            let v = br(tree, child as usize, p, m, p_first, sizes, &r, strategy);
            out.iter_mut().zip(&v).for_each(|(o, x)| *o += x);
        }
    }
    out
}

/// The blueprint's river strategy carried over to subgame `tree`, for each
/// player's hands: each subgame node is matched to a blueprint node by the
/// nearest pot fractions from `bp_root`, and each hand plays its river
/// bucket's probabilities. Where the blueprint has no match, hands check or
/// call.
pub fn blueprint_strategy(
    tree: &RiverTree,
    hands: [&[[Card; 4]]; 2],
    cards: &PloAbstraction,
    board: &[Card; 5],
    blueprint: &Blueprint,
    bp_tree: &BettingTree,
    bp_root: u32,
) -> Strategy {
    let view = BoardView::new(board);
    let buckets: [Vec<u16>; 2] =
        [0, 1].map(|p| hands[p].iter().map(|h| cards.bucket_on(h, &view)).collect());
    let matched = match_blueprint(tree, bp_tree, bp_root);
    tree.nodes
        .iter()
        .zip(&matched)
        .map(|(x, bp)| {
            if x.actions.is_empty() {
                return Vec::new();
            }
            let p = x.betting.to_act;
            let n = hands[p].len();
            let mut s = vec![0f32; x.actions.len() * n];
            match bp.filter(|&b| same_spot(x, &bp_tree.nodes[b as usize])) {
                Some(b) => {
                    let y = &bp_tree.nodes[b as usize];
                    let mut rows: HashMap<u16, Vec<f32>> = HashMap::new();
                    for (h, &bucket) in buckets[p].iter().enumerate() {
                        let row = rows
                            .entry(bucket)
                            .or_insert_with(|| map_probs(x, y, &blueprint.probs(b, bucket)));
                        for (a, &v) in row.iter().enumerate() {
                            s[a * n + h] = v;
                        }
                    }
                }
                None => {
                    let a = passive(x);
                    s[a * n..(a + 1) * n].fill(1.0);
                }
            }
            s
        })
        .collect()
}

/// How much player `br_player`, holding hands from `br_range`, wins on
/// average by best-responding on the river to the other player playing
/// `strategy` from `range` (the strategy's hands), in chips per pair of
/// hands that can be dealt together.
pub fn best_response_value(
    tree: &RiverTree,
    board: &[Card; 5],
    br_player: usize,
    range: &RiverRange,
    strategy: &Strategy,
    br_range: &RiverRange,
) -> f64 {
    let bot = 1 - br_player;
    // List 0 is player 0's hands.
    let (l0, l1) = if bot == 0 {
        (&range.hands, &br_range.hands)
    } else {
        (&br_range.hands, &range.hands)
    };
    let m = Matchups::new(board, l0, l1);
    let sizes = [l0.len(), l1.len()];
    let v = br(
        tree,
        0,
        br_player,
        &m,
        br_player == 0,
        sizes,
        &range.weights,
        strategy,
    );
    let value: f64 = v
        .iter()
        .zip(&br_range.weights)
        .map(|(a, b)| (a * b) as f64)
        .sum();
    // The weight of all compatible pairs.
    let mut c = vec![0f32; br_range.hands.len()];
    m.apply(false, br_player == 1, &range.weights, 1.0, &mut c);
    let pairs: f64 = c
        .iter()
        .zip(&br_range.weights)
        .map(|(a, b)| (a * b) as f64)
        .sum();
    value / pairs.max(1e-12)
}
