//! Real-time depth-limited solving of the turn.
//!
//! Solving the turn exactly means solving every river behind it as well,
//! too much to do while playing. Depth-limited solving (Brown, Sandholm and
//! Amos 2018, "Depth-Limited Solving for Imperfect-Information Games", the
//! method of Modicum) solves only the turn's betting and values what comes
//! after from the blueprint:
//!
//! - **The subgame** is the turn's betting from the real pot and stacks
//!   ([`RiverTree::build`] stops where the street ends), with any real
//!   action the menu lacks forced in, as on the river.
//! - **Leaves** are where the turn's betting closes with both players still
//!   in. Each is valued by dealing every river card and playing the river
//!   out with fixed *continuation strategies* ([`Continuation`]): the
//!   blueprint's river strategy carried over to the real chips. Showdowns
//!   and folds on each river are valued in O(n) as in
//!   [`RiverHands`], so a leaf costs one pass over its river tree per river
//!   card. An all-in leaf is the same with no betting left.
//! - **Several continuations.** With a single one the solver may assume the
//!   opponent keeps playing the blueprint on the river, which they needn't.
//!   So at each leaf the opponent (the *chooser*) picks, per hand, one of
//!   several ([`Bias`]): the blueprint, or the blueprint with folds, calls or
//!   raises made five times as likely. The turn strategy then has to hold up
//!   against every river plan, not rely on one.
//! - **Safety** comes from the same resolving gadget as on the river: each
//!   opponent hand may take what a best response gets against the
//!   blueprint's turn strategy (picking its continuation at each leaf)
//!   instead of playing.
//! - **Re-solving** after an off-tree bet freezes the bot's own earlier turn
//!   actions at what it played, as on the river.

use super::{
    abstraction::CardAbstraction,
    blueprint::Blueprint,
    cards::{Card, NUM_CARDS, NUM_HOLES, bit, hole_cards, hole_index, mask},
    hunl::{Betting, BettingTree, HunlAction, HunlConfig},
    river::{
        Discount, Gadget, RiverHands, RiverTree, Strategy, blueprint_strategy, fold_values,
        map_probs, match_blueprint, normalized, passive, regret_matching, same_spot,
    },
};

/// The hands that miss a turn board, in hole-index order.
#[derive(Clone, Debug)]
pub struct TurnHands {
    pub board: [Card; 4],
    /// Each hand's hole cards.
    pub cards: Vec<[Card; 2]>,
    /// The hand number of every hole index, `u16::MAX` if it uses a board
    /// card.
    pub index: Vec<u16>,
}

impl TurnHands {
    pub fn new(board: [Card; 4]) -> Self {
        let bm = mask(&board);
        let mut cards = Vec::new();
        let mut index = vec![u16::MAX; NUM_HOLES];
        for (h, i) in index.iter_mut().enumerate() {
            let (a, b) = hole_cards(h);
            if bm & (bit(a) | bit(b)) == 0 {
                *i = cards.len() as u16;
                cards.push([a, b]);
            }
        }
        Self {
            board,
            cards,
            index,
        }
    }

    pub fn len(&self) -> usize {
        self.cards.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }

    /// Converts weights by hole index to weights by hand number.
    pub fn by_hand(&self, by_hole: &[f64]) -> Vec<f32> {
        self.cards
            .iter()
            .map(|c| by_hole[hole_index(c[0], c[1])] as f32)
            .collect()
    }
}

/// One river card: its hands, numbered as on the river, where each sits
/// among the turn's hands, and each one's continuation bucket.
#[derive(Clone, Debug)]
struct River {
    hands: RiverHands,
    to_turn: Vec<u32>,
    buckets: Vec<u16>,
}

/// How both players play the river after a leaf: the river's betting in
/// real chips and, at each decision, action probabilities by bucket.
#[derive(Clone, Debug)]
pub struct Continuation {
    pub tree: RiverTree,
    /// For each node, a row of action probabilities per bucket (a single
    /// row serves every bucket); empty at nodes that end the hand.
    rows: Vec<Vec<Vec<f32>>>,
}

impl Continuation {
    /// `rows` as described on the field: per node, per bucket, the
    /// probability of each of the node's actions.
    pub fn new(tree: RiverTree, rows: Vec<Vec<Vec<f32>>>) -> Self {
        assert_eq!(tree.nodes.len(), rows.len());
        for (x, r) in tree.nodes.iter().zip(&rows) {
            assert_eq!(x.actions.is_empty(), r.is_empty());
            assert!(r.iter().all(|row| row.len() == x.actions.len()));
        }
        Self { tree, rows }
    }

    /// Both players check or call to the showdown.
    pub fn passive(leaf: &Betting, config: &HunlConfig) -> Self {
        let tree = RiverTree::build(leaf, config, &[]);
        let rows = tree
            .nodes
            .iter()
            .map(|x| {
                if x.actions.is_empty() {
                    return Vec::new();
                }
                let mut row = vec![0.0; x.actions.len()];
                row[passive(x)] = 1.0;
                vec![row]
            })
            .collect();
        Self::new(tree, rows)
    }

    /// Every action of the menu equally likely, whatever the hand.
    pub fn uniform(leaf: &Betting, config: &HunlConfig) -> Self {
        let tree = RiverTree::build(leaf, config, &[]);
        let rows = tree
            .nodes
            .iter()
            .map(|x| match x.actions.len() {
                0 => Vec::new(),
                k => vec![vec![1.0 / k as f32; k]],
            })
            .collect();
        Self::new(tree, rows)
    }

    /// The blueprint's river strategy from blueprint node `bp_leaf` (where
    /// its river starts), carried over to the real chips at `leaf` as in
    /// [`RiverSolver::blueprint_strategy`](super::river::RiverSolver::blueprint_strategy).
    /// Where the blueprint can't be followed, hands check or call.
    /// `buckets` is the number of river buckets.
    pub fn from_blueprint(
        leaf: &Betting,
        config: &HunlConfig,
        blueprint: &Blueprint,
        bp_tree: &BettingTree,
        bp_leaf: Option<u32>,
        buckets: usize,
    ) -> Self {
        let tree = RiverTree::build(leaf, config, &[]);
        let matched = match bp_leaf {
            Some(b) => match_blueprint(&tree, bp_tree, b),
            None => vec![None; tree.nodes.len()],
        };
        let rows = tree
            .nodes
            .iter()
            .zip(&matched)
            .map(|(x, bp)| {
                if x.actions.is_empty() {
                    return Vec::new();
                }
                match bp.filter(|&b| same_spot(x, &bp_tree.nodes[b as usize])) {
                    Some(b) => {
                        let y = &bp_tree.nodes[b as usize];
                        (0..buckets)
                            .map(|k| map_probs(x, y, &blueprint.probs(b, k as u16)))
                            .collect()
                    }
                    None => {
                        let mut row = vec![0.0; x.actions.len()];
                        row[passive(x)] = 1.0;
                        vec![row]
                    }
                }
            })
            .collect();
        Self::new(tree, rows)
    }

    /// Each hand's probabilities at `node` on river `r`, action-major, with
    /// the actions `bias` favours made more likely.
    fn strategy(&self, node: usize, r: &River, bias: Bias) -> Vec<f32> {
        let x = &self.tree.nodes[node];
        let rows = &self.rows[node];
        let (n, k) = (r.hands.len(), x.actions.len());
        let factors: Vec<f32> = x.actions.iter().map(|a| bias.factor(a)).collect();
        // Each bucket's row, biased, once.
        let mut biased: Vec<Option<Vec<f32>>> = vec![None; rows.len()];
        let mut s = vec![0f32; k * n];
        for h in 0..n {
            let b = (r.buckets[h] as usize).min(rows.len() - 1);
            let row = biased[b].get_or_insert_with(|| {
                let w: Vec<f32> = rows[b].iter().zip(&factors).map(|(p, f)| p * f).collect();
                let total: f32 = w.iter().sum();
                if total > 0.0 {
                    w.iter().map(|x| x / total).collect()
                } else {
                    vec![1.0 / k as f32; k]
                }
            });
            for a in 0..k {
                s[a * n + h] = row[a];
            }
        }
        s
    }

    /// Player `p`'s values on river `r` from `node`, by river hand, against
    /// `opp` (the other player's reach there), with both playing this
    /// continuation, each biased by `bias[player]`.
    fn values(&self, r: &River, node: usize, p: usize, opp: &[f32], bias: [Bias; 2]) -> Vec<f32> {
        let x = &self.tree.nodes[node];
        let n = r.hands.len();
        let mut out = vec![0f32; n];
        let b = &x.betting;
        if let Some(f) = b.folded {
            let amount = if f == p {
                -(b.contributed[p] as f32)
            } else {
                b.contributed[f] as f32
            };
            r.hands.fold(opp, amount, &mut out);
            return out;
        }
        if x.actions.is_empty() {
            let amount = b.contributed[0].min(b.contributed[1]) as f32;
            r.hands.showdown(opp, amount, &mut out);
            return out;
        }
        let q = b.to_act;
        let s = self.strategy(node, r, bias[q]);
        for (a, &child) in x.children.iter().enumerate() {
            let sa = &s[a * n..(a + 1) * n];
            if sa.iter().all(|&x| x == 0.0) {
                continue;
            }
            if q == p {
                let v = self.values(r, child as usize, p, opp, bias);
                for h in 0..n {
                    out[h] += sa[h] * v[h];
                }
            } else {
                let o: Vec<f32> = opp.iter().zip(sa).map(|(x, y)| x * y).collect();
                if o.iter().all(|&x| x == 0.0) {
                    continue;
                }
                let v = self.values(r, child as usize, p, &o, bias);
                out.iter_mut().zip(&v).for_each(|(o, x)| *o += x);
            }
        }
        out
    }
}

/// How much a biased continuation favours its kind of action.
pub const BIAS: f32 = 5.0;

/// A continuation the chooser may pick at a leaf: the continuation as given,
/// or with one kind of action made [`BIAS`] times as likely at their river
/// decisions (and the row renormalized).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bias {
    None,
    Fold,
    Call,
    Raise,
}

impl Bias {
    /// The four continuations of Modicum.
    pub const ALL: [Bias; 4] = [Bias::None, Bias::Fold, Bias::Call, Bias::Raise];

    fn factor(self, a: &HunlAction) -> f32 {
        let favoured = matches!(
            (self, a),
            (Bias::Fold, HunlAction::Fold)
                | (Bias::Call, HunlAction::Check | HunlAction::Call)
                | (Bias::Raise, HunlAction::Bet(_) | HunlAction::Raise(_))
        );
        if favoured { BIAS } else { 1.0 }
    }
}

/// A leaf: its continuation, and the chooser's regrets and strategy sums
/// over the continuations they may pick (choice-major, like actions).
#[derive(Clone, Debug)]
struct Leaf {
    cont: Continuation,
    /// Whether the river has a decision left; if not (all-in), there's
    /// nothing to choose.
    choice: bool,
    regret: Vec<f32>,
    sum: Vec<f32>,
}

/// A vectorized CFR solver for the turn's betting, with leaves valued by
/// playing out every river.
#[derive(Clone, Debug)]
pub struct TurnSolver {
    /// The turn's betting; nodes where the street ends are leaves.
    pub tree: RiverTree,
    pub hands: TurnHands,
    range: [Vec<f32>; 2],
    regret: Vec<Vec<f32>>,
    sum: Vec<Vec<f32>>,
    frozen: Vec<Option<Vec<f32>>>,
    gadget: Option<Gadget>,
    rivers: Vec<River>,
    leaves: Vec<Option<Leaf>>,
    chooser: Option<usize>,
    biases: Vec<Bias>,
    /// River cards dealt per iteration (0 for all of them), the seed that
    /// draws them, and this iteration's draw (indices into `rivers`).
    samples: usize,
    seed: u64,
    drawn: Vec<usize>,
    pub iterations: usize,
}

impl TurnSolver {
    /// A solver for the turn from `root` on `board`, with `ranges` (weights
    /// by hole index, for player 0 the button and 1) and `path` forced into
    /// the tree (see [`RiverTree::build`]). `continuation` gives the river
    /// play after each leaf (from its node number and betting), and
    /// `buckets` every hand's continuation bucket on a full board (by hole
    /// index).
    pub fn new(
        board: [Card; 4],
        root: &Betting,
        config: &HunlConfig,
        path: &[HunlAction],
        ranges: [&[f64]; 2],
        mut continuation: impl FnMut(u32, &Betting) -> Continuation,
        mut buckets: impl FnMut(&[Card; 5]) -> Vec<u16>,
    ) -> Self {
        assert_eq!(root.street, 2, "the turn starts on street 2");
        let tree = RiverTree::build(root, config, path);
        let hands = TurnHands::new(board);
        let range = [hands.by_hand(ranges[0]), hands.by_hand(ranges[1])];
        let bm = mask(&board);
        let rivers: Vec<River> = (0..NUM_CARDS as Card)
            .filter(|&c| bm & bit(c) == 0)
            .map(|c| {
                let full = [board[0], board[1], board[2], board[3], c];
                let rh = RiverHands::new(full);
                let to_turn = rh
                    .cards
                    .iter()
                    .map(|x| hands.index[hole_index(x[0], x[1])] as u32)
                    .collect();
                let by_hole = buckets(&full);
                let buckets = rh
                    .cards
                    .iter()
                    .map(|x| by_hole[hole_index(x[0], x[1])])
                    .collect();
                River {
                    hands: rh,
                    to_turn,
                    buckets,
                }
            })
            .collect();
        let leaves = tree
            .nodes
            .iter()
            .enumerate()
            .map(|(i, x)| {
                (x.actions.is_empty() && x.betting.folded.is_none()).then(|| {
                    let cont = continuation(i as u32, &x.betting);
                    let choice = cont.tree.nodes.iter().any(|y| !y.actions.is_empty());
                    Leaf {
                        cont,
                        choice,
                        regret: Vec::new(),
                        sum: Vec::new(),
                    }
                })
            })
            .collect();
        let n = hands.len();
        let size = |x: &super::river::RiverNode| x.actions.len() * n;
        Self {
            regret: tree.nodes.iter().map(|x| vec![0.0; size(x)]).collect(),
            sum: tree.nodes.iter().map(|x| vec![0.0; size(x)]).collect(),
            frozen: vec![None; tree.nodes.len()],
            tree,
            hands,
            range,
            gadget: None,
            drawn: (0..rivers.len()).collect(),
            rivers,
            leaves,
            chooser: None,
            biases: Vec::new(),
            samples: 0,
            seed: 0,
            iterations: 0,
        }
    }

    /// Deals only `per_iteration` river cards (drawn afresh each iteration
    /// from `seed`) when valuing leaves while solving, instead of all 48:
    /// an unbiased estimate of each leaf's values, so CFR still converges,
    /// at a fraction of the cost per iteration. Best responses and
    /// exploitability always use every river. 0 deals them all.
    pub fn set_river_samples(&mut self, per_iteration: usize, seed: u64) {
        self.samples = per_iteration.min(self.rivers.len());
        self.seed = seed;
    }

    /// The rivers to deal this iteration.
    fn draw(&mut self) {
        let all = self.rivers.len();
        self.drawn = (0..all).collect();
        if self.samples == 0 || self.samples == all {
            return;
        }
        let mut rng = crate::rng::Rng::for_iteration(self.seed, self.iterations as u64);
        // The first `samples` of a partial Fisher-Yates shuffle.
        for i in 0..self.samples {
            let j = i + (rng.next_u64() % (all - i) as u64) as usize;
            self.drawn.swap(i, j);
        }
        self.drawn.truncate(self.samples);
        self.drawn.sort_unstable();
    }

    /// A solver whose leaves continue with the blueprint's river strategy:
    /// `bp_root` is the blueprint node where the turn starts, and every
    /// subgame leaf continues from the blueprint node it maps to.
    #[allow(clippy::too_many_arguments)]
    pub fn from_blueprint(
        board: [Card; 4],
        root: &Betting,
        config: &HunlConfig,
        path: &[HunlAction],
        ranges: [&[f64]; 2],
        cards: &CardAbstraction,
        blueprint: &Blueprint,
        bp_tree: &BettingTree,
        bp_root: u32,
    ) -> Self {
        let matched = match_blueprint(&RiverTree::build(root, config, path), bp_tree, bp_root);
        let river_buckets = cards.num_buckets(5);
        Self::new(
            board,
            root,
            config,
            path,
            ranges,
            |node, leaf| {
                // The blueprint's river must start where the leaf does.
                let bp = matched[node as usize].filter(|&b| {
                    let y = &bp_tree.nodes[b as usize].betting;
                    y.street == 3 && !y.is_over()
                });
                Continuation::from_blueprint(leaf, config, blueprint, bp_tree, bp, river_buckets)
            },
            |full| cards.buckets(full),
        )
    }

    /// A player's range by hand number.
    pub fn range(&self, player: usize) -> &[f32] {
        &self.range[player]
    }

    /// Lets `player` pick, per hand and leaf, one of `biases` as their river
    /// strategy (see [`Bias`]); the other player keeps the continuation as
    /// given. Without a chooser, both play the continuation.
    pub fn set_chooser(&mut self, player: usize, biases: &[Bias]) {
        assert!(!biases.is_empty());
        self.chooser = Some(player);
        self.biases = biases.to_vec();
        let size = biases.len() * self.hands.len();
        for leaf in self.leaves.iter_mut().flatten() {
            leaf.regret = vec![0.0; size];
            leaf.sum = vec![0.0; size];
        }
    }

    /// Adds the resolving gadget for `player` (the opponent of whoever will
    /// play the solution): each of their hands may take `value` (by hand
    /// number) instead of playing. See
    /// [`RiverSolver::set_gadget`](super::river::RiverSolver::set_gadget).
    pub fn set_gadget(&mut self, player: usize, value: Vec<f32>) {
        self.gadget = Some(Gadget::new(player, &self.range[player], value));
    }

    /// Fixes the strategy at `node` (action-major, as in [`Strategy`]).
    pub fn freeze(&mut self, node: u32, strategy: Vec<f32>) {
        assert_eq!(strategy.len(), self.regret[node as usize].len());
        self.frozen[node as usize] = Some(strategy);
    }

    /// Runs `iterations` more iterations.
    pub fn run(&mut self, iterations: usize) {
        for _ in 0..iterations {
            self.iterate();
        }
    }

    /// One iteration: an update for each player in turn.
    pub fn iterate(&mut self) {
        self.iterations += 1;
        self.draw();
        let d = Discount::at(self.iterations);
        for p in 0..2 {
            let mut reach = self.range.clone();
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

    /// The fold values at `node` for `p`, if someone folded there.
    fn fold_at(&self, node: usize, p: usize, opp: &[f32]) -> Option<Vec<f32>> {
        let b = &self.tree.nodes[node].betting;
        let f = b.folded?;
        let amount = if f == p {
            -(b.contributed[p] as f32)
        } else {
            b.contributed[f] as f32
        };
        let mut out = vec![0f32; self.hands.len()];
        fold_values(&self.hands.cards, opp, amount, &mut out);
        Some(out)
    }

    /// Counterfactual values at `node` for player `p`'s hands, updating
    /// `p`'s regrets and average strategy below it.
    fn walk(&mut self, node: usize, p: usize, reach: [&[f32]; 2], d: &Discount) -> Vec<f32> {
        if let Some(v) = self.fold_at(node, p, reach[1 - p]) {
            return v;
        }
        if self.tree.nodes[node].actions.is_empty() {
            return self.walk_leaf(node, p, reach, d);
        }
        let n = self.hands.len();
        let mut out = vec![0f32; n];
        let q = self.tree.nodes[node].betting.to_act;
        let k = self.tree.nodes[node].actions.len();
        let sigma = self.current(node);
        let children = self.tree.nodes[node].children.clone();
        if q != p {
            for (a, &child) in children.iter().enumerate() {
                let s = &sigma[a * n..(a + 1) * n];
                let r: Vec<f32> = reach[q].iter().zip(s).map(|(x, y)| x * y).collect();
                // As on the river: no skipping branches the opponent stops
                // reaching, so `p`'s sums below keep being discounted.
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
        if self.frozen[node].is_none() {
            let (regret, sum) = (&mut self.regret[node], &mut self.sum[node]);
            for a in 0..k {
                for h in 0..n {
                    let i = a * n + h;
                    regret[i] = d.regret(regret[i]) + values[i] - out[h];
                    sum[i] = sum[i] * d.sum + reach[p][h] * sigma[i];
                }
            }
        }
        out
    }

    /// The chooser at leaf `node`, if they have a choice there.
    fn chooser_at(&self, node: usize) -> Option<usize> {
        let leaf = self.leaves[node].as_ref().expect("a leaf");
        self.chooser.filter(|_| leaf.choice)
    }

    /// Both players' biases when the chooser `c` picks continuation `i`.
    fn bias(&self, c: usize, i: usize) -> [Bias; 2] {
        let mut b = [Bias::None; 2];
        b[c] = self.biases[i];
        b
    }

    /// [`walk`](Self::walk) at a leaf: the chooser's regrets over the
    /// continuations are updated like any decision's.
    fn walk_leaf(&mut self, node: usize, p: usize, reach: [&[f32]; 2], d: &Discount) -> Vec<f32> {
        let Some(c) = self.chooser_at(node) else {
            return self.leaf_values(node, p, reach[1 - p], [Bias::None; 2], true);
        };
        let n = self.hands.len();
        let k = self.biases.len();
        let sigma = regret_matching(&self.leaves[node].as_ref().expect("a leaf").regret, n);
        let mut out = vec![0f32; n];
        if p != c {
            for i in 0..k {
                let s = &sigma[i * n..(i + 1) * n];
                let o: Vec<f32> = reach[c].iter().zip(s).map(|(x, y)| x * y).collect();
                let v = self.leaf_values(node, p, &o, self.bias(c, i), true);
                out.iter_mut().zip(&v).for_each(|(o, x)| *o += x);
            }
            return out;
        }
        let mut values = vec![0f32; k * n];
        for i in 0..k {
            let v = self.leaf_values(node, p, reach[1 - p], self.bias(c, i), true);
            for h in 0..n {
                out[h] += sigma[i * n + h] * v[h];
            }
            values[i * n..(i + 1) * n].copy_from_slice(&v);
        }
        let leaf = self.leaves[node].as_mut().expect("a leaf");
        for j in 0..k * n {
            let h = j % n;
            leaf.regret[j] = d.regret(leaf.regret[j]) + values[j] - out[h];
            leaf.sum[j] = leaf.sum[j] * d.sum + reach[p][h] * sigma[j];
        }
        out
    }

    /// Player `p`'s values at leaf `node`, by turn hand, against `opp` (the
    /// other player's reach, by turn hand): the river values on every river
    /// card, averaged over the cards each pair of hands leaves. With
    /// `sampled`, only this iteration's draw of rivers, scaled up to match.
    fn leaf_values(
        &self,
        node: usize,
        p: usize,
        opp: &[f32],
        bias: [Bias; 2],
        sampled: bool,
    ) -> Vec<f32> {
        let n = self.hands.len();
        let mut out = vec![0f32; n];
        if opp.iter().all(|&x| x == 0.0) {
            return out;
        }
        let all: Vec<usize>;
        let rivers = if sampled {
            &self.drawn
        } else {
            all = (0..self.rivers.len()).collect();
            &all
        };
        let cont = &self.leaves[node].as_ref().expect("a leaf").cont;
        let one = |&i: &usize| -> Vec<f32> {
            let r = &self.rivers[i];
            let o: Vec<f32> = r.to_turn.iter().map(|&t| opp[t as usize]).collect();
            cont.values(r, 0, p, &o, bias)
        };
        // Collected in card order and summed in that order, so the result
        // doesn't depend on the thread count.
        #[cfg(feature = "parallel")]
        let per_card: Vec<Vec<f32>> = {
            use rayon::prelude::*;
            rivers.par_iter().map(one).collect()
        };
        #[cfg(not(feature = "parallel"))]
        let per_card: Vec<Vec<f32>> = rivers.iter().map(one).collect();
        for (&i, v) in rivers.iter().zip(per_card) {
            for (&t, x) in self.rivers[i].to_turn.iter().zip(v) {
                out[t as usize] += x;
            }
        }
        // A pair of hands sees every river card neither holds: 44 of 48. A
        // draw of k of the 48 counts each card 48 / k times on average.
        let total = self.rivers.len() as f32;
        let scale = total / rivers.len() as f32 / (total - 4.0);
        out.iter_mut().for_each(|x| *x *= scale);
        out
    }

    /// The strategy this iteration plays at `node`: frozen, or regret
    /// matching.
    fn current(&self, node: usize) -> Vec<f32> {
        if let Some(f) = &self.frozen[node] {
            return f.clone();
        }
        regret_matching(&self.regret[node], self.hands.len())
    }

    /// The average strategy at `node`, action-major.
    pub fn average_at(&self, node: u32) -> Vec<f32> {
        if let Some(f) = &self.frozen[node as usize] {
            return f.clone();
        }
        normalized(&self.sum[node as usize], self.hands.len(), |x| x)
    }

    /// The average strategy everywhere.
    pub fn average(&self) -> Strategy {
        (0..self.tree.nodes.len() as u32)
            .map(|i| self.average_at(i))
            .collect()
    }

    /// The chooser's average pick at leaf `node`, choice-major (the order of
    /// the biases given to [`set_chooser`](Self::set_chooser)), or `None`
    /// where they have no choice.
    pub fn choice_at(&self, node: u32) -> Option<Vec<f32>> {
        self.chooser_at(node as usize)?;
        let leaf = self.leaves[node as usize].as_ref()?;
        Some(normalized(&leaf.sum, self.hands.len(), |x| x))
    }

    /// The average strategy at `node` for the hand with hole index `hole`,
    /// or `None` if the hand uses a board card.
    pub fn probs(&self, node: u32, hole: usize) -> Option<Vec<f64>> {
        let h = self.hands.index[hole];
        if h == u16::MAX {
            return None;
        }
        let n = self.hands.len();
        let k = self.tree.nodes[node as usize].actions.len();
        let s = match &self.frozen[node as usize] {
            Some(f) => f,
            None => &self.sum[node as usize],
        };
        let probs: Vec<f64> = (0..k).map(|a| s[a * n + h as usize] as f64).collect();
        let total: f64 = probs.iter().sum();
        Some(if total > 0.0 {
            probs.iter().map(|x| x / total).collect()
        } else {
            vec![1.0 / k as f64; k]
        })
    }

    /// Player `p`'s best-response counterfactual values, by hand number,
    /// against the other player playing `strategy` from their range. At the
    /// leaves a chooser `p` takes their best continuation for each hand;
    /// against a chooser, `p` faces their average pick.
    pub fn best_response(&self, p: usize, strategy: &Strategy) -> Vec<f32> {
        self.br(0, p, &self.range[1 - p], strategy)
    }

    fn br(&self, node: usize, p: usize, opp: &[f32], strategy: &Strategy) -> Vec<f32> {
        if let Some(v) = self.fold_at(node, p, opp) {
            return v;
        }
        let n = self.hands.len();
        let x = &self.tree.nodes[node];
        if x.actions.is_empty() {
            return self.leaf_br(node, p, opp);
        }
        let mut out = vec![0f32; n];
        if x.betting.to_act == p {
            out.fill(f32::NEG_INFINITY);
            for &child in &x.children {
                let v = self.br(child as usize, p, opp, strategy);
                out.iter_mut().zip(&v).for_each(|(o, x)| *o = o.max(*x));
            }
        } else {
            for (a, &child) in x.children.iter().enumerate() {
                let s = &strategy[node][a * n..(a + 1) * n];
                let r: Vec<f32> = opp.iter().zip(s).map(|(x, y)| x * y).collect();
                if r.iter().all(|&x| x == 0.0) {
                    continue;
                }
                let v = self.br(child as usize, p, &r, strategy);
                out.iter_mut().zip(&v).for_each(|(o, x)| *o += x);
            }
        }
        out
    }

    fn leaf_br(&self, node: usize, p: usize, opp: &[f32]) -> Vec<f32> {
        let Some(c) = self.chooser_at(node) else {
            return self.leaf_values(node, p, opp, [Bias::None; 2], false);
        };
        let n = self.hands.len();
        let k = self.biases.len();
        if c == p {
            let mut out = vec![f32::NEG_INFINITY; n];
            for i in 0..k {
                let v = self.leaf_values(node, p, opp, self.bias(c, i), false);
                out.iter_mut().zip(&v).for_each(|(o, x)| *o = o.max(*x));
            }
            return out;
        }
        let mix = self.choice_at(node as u32).expect("a choice");
        let mut out = vec![0f32; n];
        for i in 0..k {
            let s = &mix[i * n..(i + 1) * n];
            let o: Vec<f32> = opp.iter().zip(s).map(|(x, y)| x * y).collect();
            let v = self.leaf_values(node, p, &o, self.bias(c, i), false);
            out.iter_mut().zip(&v).for_each(|(o, x)| *o += x);
        }
        out
    }

    /// Weight of all pairs of hands the two ranges can hold together.
    fn pairs(&self) -> f64 {
        let mut v = vec![0f32; self.hands.len()];
        fold_values(&self.hands.cards, &self.range[1], 1.0, &mut v);
        v.iter()
            .zip(&self.range[0])
            .map(|(a, b)| (a * b) as f64)
            .sum()
    }

    /// How much a best response wins against `strategy` in this
    /// depth-limited subgame, averaged over both players, in chips per
    /// hand (0 at an equilibrium). It ignores the gadget.
    pub fn exploitability_of(&self, strategy: &Strategy) -> f64 {
        let total: f64 = (0..2)
            .map(|p| {
                let v = self.best_response(p, strategy);
                v.iter()
                    .zip(&self.range[p])
                    .map(|(a, b)| (a * b) as f64)
                    .sum::<f64>()
            })
            .sum();
        total / 2.0 / self.pairs().max(1e-12)
    }

    /// [`exploitability_of`](Self::exploitability_of) the average strategy.
    pub fn exploitability(&self) -> f64 {
        self.exploitability_of(&self.average())
    }

    /// The blueprint's turn strategy carried over to this subgame (see
    /// [`RiverSolver::blueprint_strategy`](super::river::RiverSolver::blueprint_strategy)),
    /// with `bp_root` the blueprint node the turn starts at and `buckets`
    /// every hand's turn bucket by hole index.
    pub fn blueprint_strategy(
        &self,
        blueprint: &Blueprint,
        bp_tree: &BettingTree,
        bp_root: u32,
        buckets: &[u16],
    ) -> Strategy {
        blueprint_strategy(
            &self.tree,
            &self.hands.cards,
            blueprint,
            bp_tree,
            bp_root,
            buckets,
        )
    }
}
