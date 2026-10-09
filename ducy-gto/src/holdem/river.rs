//! Real-time solving of the river.
//!
//! The blueprint plays the river with buckets and a betting tree whose chip
//! amounts can be off from the real hand's. Once the river card is out, the
//! remaining game is small enough to solve exactly while playing: every
//! hand kept separate, the bet sizes worked out from the real pot and stacks.
//!
//! - **The subgame** ([`RiverTree`]) starts at the river's first decision
//!   with the real chips, and offers a menu of sizes (by default the
//!   blueprint's postflop menu: a third, three quarters and 1.25× the pot,
//!   then pot, then all-in). A path of actions already taken can be forced
//!   into it, so a re-solve contains the opponent's real bet size.
//! - **The solver** ([`RiverSolver`]) runs Discounted CFR (Brown and
//!   Sandholm 2019, α = 1.5, β = 0, γ = 2) on vectors over the 1,081 hands
//!   the board allows: regrets per node, hand and action. A showdown is
//!   valued in O(n) from hands sorted by strength, with running sums per card
//!   to leave out opponent hands that share a card; a fold likewise.
//! - **Safety** comes from the resolving gadget of Burch, Johanson and
//!   Bowling (2014), as described by Brown and Sandholm (2017): at the root
//!   the opponent may, for each hand, take a fixed value instead of playing
//!   the subgame. The value is what a best response gets against the
//!   blueprint's own river strategy, mapped onto the subgame tree. At an
//!   equilibrium of the gadget game, no opponent hand can win more against
//!   the solved strategy than it could against the blueprint's, so solving
//!   never makes the bot more exploitable on the river than the blueprint
//!   (up to how far the solve has converged).
//! - **Re-solving** after an off-tree opponent bet solves the whole river
//!   again with that size added, with the bot's own earlier river actions
//!   frozen at the strategy it actually played (as in Brown and Sandholm
//!   2019, "Superhuman AI for multiplayer poker"), under the same gadget.

use std::collections::HashMap;

use super::{
    blueprint::Blueprint,
    cards::{Card, NUM_CARDS, NUM_HOLES, bit, hole_cards, hole_index, mask, score},
    hunl::{Betting, BettingTree, HunlAction, HunlConfig, Node},
};

/// The hands that miss the board, numbered from weakest to strongest so
/// that every showdown is valued in one pass each way.
#[derive(Clone, Debug)]
pub struct RiverHands {
    pub board: [Card; 5],
    /// Each hand's hole cards.
    pub cards: Vec<[Card; 2]>,
    /// Each hand's strength with the board (higher is better), ascending.
    pub strength: Vec<u32>,
    /// The hand number of every hole index, `u16::MAX` if it uses a board
    /// card.
    pub index: Vec<u16>,
}

impl RiverHands {
    pub fn new(board: [Card; 5]) -> Self {
        let bm = mask(&board);
        let mut hands: Vec<(u32, [Card; 2])> = (0..NUM_HOLES)
            .map(hole_cards)
            .filter(|&(a, b)| bm & (bit(a) | bit(b)) == 0)
            .map(|(a, b)| (score(bm | bit(a) | bit(b)), [a, b]))
            .collect();
        hands.sort_by_key(|h| h.0);
        let mut index = vec![u16::MAX; NUM_HOLES];
        for (i, (_, c)) in hands.iter().enumerate() {
            index[hole_index(c[0], c[1])] = i as u16;
        }
        Self {
            board,
            cards: hands.iter().map(|h| h.1).collect(),
            strength: hands.iter().map(|h| h.0).collect(),
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

    /// Showdown values: for each hand, `amount` times the opponent reach it
    /// beats minus the reach it loses to, over opponent hands that share no
    /// card with it.
    pub fn showdown(&self, opp: &[f32], amount: f32, out: &mut [f32]) {
        let n = self.len();
        let (cards, strength) = (&self.cards, &self.strength);
        // Weaker hands first: what each hand beats. Ties are a run of equal
        // strength, valued before any of the run is added.
        let (mut total, mut per_card) = (0f64, [0f64; NUM_CARDS]);
        let mut i = 0;
        while i < n {
            let mut j = i;
            while j < n && strength[j] == strength[i] {
                let [a, b] = cards[j];
                out[j] = (total - per_card[a as usize] - per_card[b as usize]) as f32;
                j += 1;
            }
            for h in i..j {
                let ([a, b], w) = (cards[h], opp[h] as f64);
                total += w;
                per_card[a as usize] += w;
                per_card[b as usize] += w;
            }
            i = j;
        }
        // Then stronger hands: what each hand loses to.
        let (mut total, mut per_card) = (0f64, [0f64; NUM_CARDS]);
        let mut j = n;
        while j > 0 {
            let mut i = j;
            while i > 0 && strength[i - 1] == strength[j - 1] {
                i -= 1;
                let [a, b] = cards[i];
                let lose = total - per_card[a as usize] - per_card[b as usize];
                out[i] = (out[i] as f64 - lose) as f32 * amount;
            }
            for h in i..j {
                let ([a, b], w) = (cards[h], opp[h] as f64);
                total += w;
                per_card[a as usize] += w;
                per_card[b as usize] += w;
            }
            j = i;
        }
    }

    /// Fold values: `amount` times the opponent reach that shares no card
    /// with each hand.
    pub fn fold(&self, opp: &[f32], amount: f32, out: &mut [f32]) {
        fold_values(&self.cards, opp, amount, out);
    }
}

/// Fold values over any list of hands: `amount` times the opponent reach
/// (by position in `cards`) that shares no card with each hand.
pub fn fold_values(cards: &[[Card; 2]], opp: &[f32], amount: f32, out: &mut [f32]) {
    let (mut total, mut per_card) = (0f64, [0f64; NUM_CARDS]);
    for (c, &w) in cards.iter().zip(opp) {
        total += w as f64;
        per_card[c[0] as usize] += w as f64;
        per_card[c[1] as usize] += w as f64;
    }
    for (h, c) in cards.iter().enumerate() {
        // The hand itself is in both card sums: add it back once.
        let other = total - per_card[c[0] as usize] - per_card[c[1] as usize] + opp[h] as f64;
        out[h] = other as f32 * amount;
    }
}

/// One point in a river subgame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiverNode {
    pub betting: Betting,
    /// Empty once the hand is over.
    pub actions: Vec<HunlAction>,
    pub children: Vec<u32>,
}

/// The betting of one street of a subgame in real chips; node 0 is the
/// root. Used for the river, and for the turn up to where the river card
/// comes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiverTree {
    pub nodes: Vec<RiverNode>,
}

impl RiverTree {
    /// The subgame from `root` (a betting state after the flop) to the end
    /// of `root`'s street, with the sizes of `config.menu.postflop`, plus
    /// every action of `path` (actions taken since `root`, in order) where
    /// the menu lacks it. Nodes where the hand ends or the street does have
    /// no actions.
    pub fn build(root: &Betting, config: &HunlConfig, path: &[HunlAction]) -> Self {
        fn add(
            c: &HunlConfig,
            b: Betting,
            street: usize,
            path: Option<&[HunlAction]>,
            nodes: &mut Vec<RiverNode>,
        ) -> u32 {
            let id = nodes.len() as u32;
            let mut actions = if b.is_over() || b.street != street {
                Vec::new()
            } else {
                b.actions(c)
            };
            let forced = path.and_then(|p| p.first().copied());
            if let Some(f) = forced
                && !b.is_over()
                && b.street == street
                && !actions.contains(&f)
            {
                let at = actions
                    .iter()
                    .position(|a| amount(a) > amount(&f) && amount(a) > 0)
                    .unwrap_or(actions.len());
                actions.insert(at, f);
            }
            nodes.push(RiverNode {
                betting: b.clone(),
                actions: actions.clone(),
                children: Vec::new(),
            });
            let children = actions
                .iter()
                .map(|&a| {
                    let rest = path.filter(|_| forced == Some(a)).map(|p| &p[1..]);
                    add(c, b.play(a), street, rest, nodes)
                })
                .collect();
            nodes[id as usize].children = children;
            id
        }
        let mut nodes = Vec::new();
        add(config, root.clone(), root.street, Some(path), &mut nodes);
        Self { nodes }
    }

    /// The node reached by `path` from the root.
    pub fn follow(&self, path: &[HunlAction]) -> Option<u32> {
        let mut at = 0u32;
        for a in path {
            let n = &self.nodes[at as usize];
            at = n.children[n.actions.iter().position(|x| x == a)?];
        }
        Some(at)
    }

    /// Nodes along `path`: the node before each action and the action's
    /// index there.
    pub fn along(&self, path: &[HunlAction]) -> Option<Vec<(u32, usize)>> {
        let mut at = 0u32;
        let mut out = Vec::with_capacity(path.len());
        for a in path {
            let n = &self.nodes[at as usize];
            let i = n.actions.iter().position(|x| x == a)?;
            out.push((at, i));
            at = n.children[i];
        }
        Some(out)
    }
}

/// The chips an action puts the street total at (0 for fold and check), to
/// keep actions in menu order.
fn amount(a: &HunlAction) -> u64 {
    match *a {
        HunlAction::Bet(to) | HunlAction::Raise(to) => to,
        HunlAction::Call => 1,
        _ => 0,
    }
}

/// A strategy for every node: action probabilities stored action-major
/// (`a * hands + hand`), empty at terminal nodes.
pub type Strategy = Vec<Vec<f32>>;

/// Discounted CFR's settings.
const ALPHA: f64 = 1.5;
const GAMMA: f64 = 2.0;

/// Weight of the gadget's floor: hands the opponent's range rules out still
/// get this share of the largest weight, so the gadget protects them too.
const GADGET_FLOOR: f32 = 1e-3;

/// The resolving gadget: the opponent's alternative value per hand and its
/// own regrets for taking it (index 0) or playing (1).
#[derive(Clone, Debug)]
pub(crate) struct Gadget {
    pub(crate) player: usize,
    value: Vec<f32>,
    weight: Vec<f32>,
    regret: [Vec<f32>; 2],
}

impl Gadget {
    /// The gadget for `player`, whose range is `range`, with alternative
    /// values `value` (both by hand number).
    pub(crate) fn new(player: usize, range: &[f32], value: Vec<f32>) -> Self {
        let n = range.len();
        let top = range.iter().fold(0f32, |m, &x| m.max(x));
        let weight = range
            .iter()
            .map(|&w| if top > 0.0 { w / top } else { 0.0 } + GADGET_FLOOR)
            .collect();
        Self {
            player,
            value,
            weight,
            regret: [vec![0.0; n], vec![0.0; n]],
        }
    }

    /// The probability each hand plays the subgame.
    pub(crate) fn enter(&self) -> Vec<f32> {
        let [t, f] = &self.regret;
        t.iter()
            .zip(f)
            .map(|(&t, &f)| {
                let (t, f) = (t.max(0.0), f.max(0.0));
                if t + f > 0.0 { f / (t + f) } else { 0.5 }
            })
            .collect()
    }

    /// The gadget player's reach into the subgame: `enter` per hand.
    pub(crate) fn reach(&self, enter: &[f32]) -> Vec<f32> {
        self.weight.iter().zip(enter).map(|(w, e)| w * e).collect()
    }

    /// Updates the regrets for taking the value or playing, given the
    /// subgame's values `v` and this iteration's `enter`.
    pub(crate) fn update(&mut self, v: &[f32], enter: &[f32], d: &Discount) {
        for h in 0..v.len() {
            let mixed = (1.0 - enter[h]) * self.value[h] + enter[h] * v[h];
            for (r, x) in self.regret.iter_mut().zip([self.value[h], v[h]]) {
                r[h] = d.regret(r[h]) + x - mixed;
            }
        }
    }
}

/// A vectorized CFR solver for one river subgame.
#[derive(Clone, Debug)]
pub struct RiverSolver {
    pub tree: RiverTree,
    pub hands: RiverHands,
    /// Each player's range by hand number.
    range: [Vec<f32>; 2],
    regret: Vec<Vec<f32>>,
    sum: Vec<Vec<f32>>,
    frozen: Vec<Option<Vec<f32>>>,
    gadget: Option<Gadget>,
    pub iterations: usize,
}

impl RiverSolver {
    /// A solver for the river from `root` on `board`, with `ranges` (weights
    /// by hole index, for player 0 the button and 1) and `path` forced into
    /// the tree (see [`RiverTree::build`]).
    pub fn new(
        board: [Card; 5],
        root: &Betting,
        config: &HunlConfig,
        path: &[HunlAction],
        ranges: [&[f64]; 2],
    ) -> Self {
        let tree = RiverTree::build(root, config, path);
        let hands = RiverHands::new(board);
        let n = hands.len();
        let range = [hands.by_hand(ranges[0]), hands.by_hand(ranges[1])];
        let size = |node: &RiverNode| node.actions.len() * n;
        Self {
            regret: tree.nodes.iter().map(|x| vec![0.0; size(x)]).collect(),
            sum: tree.nodes.iter().map(|x| vec![0.0; size(x)]).collect(),
            frozen: vec![None; tree.nodes.len()],
            tree,
            hands,
            range,
            gadget: None,
            iterations: 0,
        }
    }

    /// A player's range by hand number.
    pub fn range(&self, player: usize) -> &[f32] {
        &self.range[player]
    }

    /// Adds the resolving gadget for `player` (the opponent of whoever will
    /// play the solution): each of their hands may take `value` (a
    /// counterfactual value by hand number, against the other player's
    /// range) instead of playing.
    pub fn set_gadget(&mut self, player: usize, value: Vec<f32>) {
        self.gadget = Some(Gadget::new(player, &self.range[player], value));
    }

    /// Fixes the strategy at `node` (action-major, as in [`Strategy`]): for
    /// actions already taken, so a re-solve keeps what was really played.
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

    /// Counterfactual values at `node` for player `p`'s hands, updating
    /// `p`'s regrets and average strategy below it.
    fn walk(&mut self, node: usize, p: usize, reach: [&[f32]; 2], d: &Discount) -> Vec<f32> {
        let n = self.hands.len();
        let mut out = vec![0f32; n];
        if self.terminal(node, p, reach[1 - p], &mut out) {
            return out;
        }
        let q = self.tree.nodes[node].betting.to_act;
        let k = self.tree.nodes[node].actions.len();
        let sigma = self.current(node);
        let children = self.tree.nodes[node].children.clone();
        if q != p {
            for (a, &child) in children.iter().enumerate() {
                let s = &sigma[a * n..(a + 1) * n];
                let r: Vec<f32> = reach[q].iter().zip(s).map(|(x, y)| x * y).collect();
                // No skipping a branch the opponent no longer reaches: `p`'s
                // strategy sums below it still need their discounting and
                // updates, or a branch abandoned early keeps the average
                // strategy it had then, and a best response exploits it.
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

    /// Fills `out` with `p`'s values if `node` ends the hand.
    fn terminal(&self, node: usize, p: usize, opp: &[f32], out: &mut [f32]) -> bool {
        let b = &self.tree.nodes[node].betting;
        if let Some(f) = b.folded {
            let amount = if f == p {
                -(b.contributed[p] as f32)
            } else {
                b.contributed[f] as f32
            };
            self.hands.fold(opp, amount, out);
            return true;
        }
        if b.is_over() {
            let amount = b.contributed[0].min(b.contributed[1]) as f32;
            self.hands.showdown(opp, amount, out);
            return true;
        }
        false
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
        let node = node as usize;
        if let Some(f) = &self.frozen[node] {
            return f.clone();
        }
        normalized(&self.sum[node], self.hands.len(), |x| x)
    }

    /// The average strategy everywhere.
    pub fn average(&self) -> Strategy {
        (0..self.tree.nodes.len() as u32)
            .map(|i| self.average_at(i))
            .collect()
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
        let sum = &self.sum[node as usize];
        let probs: Vec<f64> = match &self.frozen[node as usize] {
            Some(f) => (0..k).map(|a| f[a * n + h as usize] as f64).collect(),
            None => (0..k).map(|a| sum[a * n + h as usize] as f64).collect(),
        };
        let total: f64 = probs.iter().sum();
        Some(if total > 0.0 {
            probs.iter().map(|x| x / total).collect()
        } else {
            vec![1.0 / k as f64; k]
        })
    }

    /// Player `p`'s best-response counterfactual values, by hand number,
    /// against the other player playing `strategy` from their range.
    pub fn best_response(&self, p: usize, strategy: &Strategy) -> Vec<f32> {
        self.br(0, p, &self.range[1 - p], strategy)
    }

    fn br(&self, node: usize, p: usize, opp: &[f32], strategy: &Strategy) -> Vec<f32> {
        let n = self.hands.len();
        let mut out = vec![0f32; n];
        if self.terminal(node, p, opp, &mut out) {
            return out;
        }
        let x = &self.tree.nodes[node];
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

    /// Counterfactual values for player `p` when both play `strategy`.
    pub fn values(&self, p: usize, strategy: &Strategy) -> Vec<f32> {
        self.ev(0, p, [&self.range[0], &self.range[1]], strategy)
    }

    fn ev(&self, node: usize, p: usize, reach: [&[f32]; 2], strategy: &Strategy) -> Vec<f32> {
        let n = self.hands.len();
        let mut out = vec![0f32; n];
        if self.terminal(node, p, reach[1 - p], &mut out) {
            return out;
        }
        let x = &self.tree.nodes[node];
        let q = x.betting.to_act;
        for (a, &child) in x.children.iter().enumerate() {
            let s = &strategy[node][a * n..(a + 1) * n];
            let r: Vec<f32> = reach[q].iter().zip(s).map(|(x, y)| x * y).collect();
            let mut next = reach;
            next[q] = &r;
            let v = self.ev(child as usize, p, next, strategy);
            if q == p {
                out.iter_mut()
                    .zip(v.iter().zip(s))
                    .for_each(|(o, (x, s))| *o += s * x);
            } else {
                out.iter_mut().zip(&v).for_each(|(o, x)| *o += x);
            }
        }
        out
    }

    /// Weight of all pairs of hands the two ranges can hold together.
    fn pairs(&self) -> f64 {
        let mut v = vec![0f32; self.hands.len()];
        self.hands.fold(&self.range[1], 1.0, &mut v);
        v.iter()
            .zip(&self.range[0])
            .map(|(a, b)| (a * b) as f64)
            .sum()
    }

    /// How much a best response wins against `strategy` in this subgame,
    /// averaged over both players, in chips per hand (0 at an equilibrium).
    /// It ignores the gadget: the ranges are taken as given.
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

    /// The blueprint's river strategy carried over to this subgame: each
    /// subgame node is matched to a blueprint node by following actions
    /// with the nearest pot fraction from `bp_root` (the blueprint node the
    /// river starts at), and each hand plays its bucket's probabilities
    /// (`buckets` by hole index). Where the blueprint tree has no match, the
    /// hand checks or calls.
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

/// The blueprint node each subgame node corresponds to, following actions
/// from `bp_root` (matched to the root) with the nearest pot fraction.
/// `None` below a node the blueprint can't follow: a decision node matches
/// only a blueprint decision on the same street with the same player to
/// act. A node where the street ends gets the blueprint node after the
/// matching action, whatever it is.
pub fn match_blueprint(tree: &RiverTree, bp_tree: &BettingTree, bp_root: u32) -> Vec<Option<u32>> {
    let mut out = vec![None; tree.nodes.len()];
    let mut stack = vec![(0u32, Some(bp_root))];
    while let Some((node, bp)) = stack.pop() {
        out[node as usize] = bp;
        let x = &tree.nodes[node as usize];
        let y = bp
            .map(|b| &bp_tree.nodes[b as usize])
            .filter(|y| same_spot(x, y));
        for (a, &child) in x.actions.iter().zip(&x.children) {
            let next = y.map(|y| y.children[nearest(&x.betting, a, &y.betting, &y.actions)]);
            stack.push((child, next));
        }
    }
    out
}

/// Whether blueprint node `y` is a decision like subgame node `x`.
pub fn same_spot(x: &RiverNode, y: &Node) -> bool {
    !x.actions.is_empty()
        && !y.actions.is_empty()
        && y.betting.street == x.betting.street
        && y.betting.to_act == x.betting.to_act
}

/// Blueprint probabilities at `y` moved onto the actions of subgame node
/// `x`: each blueprint action's mass goes to the nearest subgame action.
pub fn map_probs(x: &RiverNode, y: &Node, probs: &[f64]) -> Vec<f32> {
    let mut out = vec![0f32; x.actions.len()];
    for (a, &p) in y.actions.iter().zip(probs) {
        out[nearest(&y.betting, a, &x.betting, &x.actions)] += p as f32;
    }
    out
}

/// The index of the check or call at `x` (the first action if neither).
pub fn passive(x: &RiverNode) -> usize {
    x.actions
        .iter()
        .position(|a| matches!(a, HunlAction::Check | HunlAction::Call))
        .unwrap_or(0)
}

/// The blueprint's strategy carried over to subgame `tree` for the hands
/// `cards`: see [`RiverSolver::blueprint_strategy`].
pub fn blueprint_strategy(
    tree: &RiverTree,
    cards: &[[Card; 2]],
    blueprint: &Blueprint,
    bp_tree: &BettingTree,
    bp_root: u32,
    buckets: &[u16],
) -> Strategy {
    let n = cards.len();
    let matched = match_blueprint(tree, bp_tree, bp_root);
    tree.nodes
        .iter()
        .zip(&matched)
        .map(|(x, bp)| {
            let mut s = vec![0f32; x.actions.len() * n];
            if x.actions.is_empty() {
                return s;
            }
            match bp.filter(|&b| same_spot(x, &bp_tree.nodes[b as usize])) {
                Some(b) => {
                    let y = &bp_tree.nodes[b as usize];
                    let mut rows: HashMap<u16, Vec<f32>> = HashMap::new();
                    for (h, c) in cards.iter().enumerate() {
                        let bucket = buckets[hole_index(c[0], c[1])];
                        let row = rows
                            .entry(bucket)
                            .or_insert_with(|| map_probs(x, y, &blueprint.probs(b, bucket)));
                        for (a, &p) in row.iter().enumerate() {
                            s[a * n + h] = p;
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

/// The index in `to` (actions at betting `tb`) of the action closest to `a`
/// at betting `fb`: the same kind, and for bets the nearest pot fraction
/// (all-in matches all-in).
fn nearest(fb: &Betting, a: &HunlAction, tb: &Betting, to: &[HunlAction]) -> usize {
    let passive = |x: &HunlAction| matches!(x, HunlAction::Check | HunlAction::Call);
    let find = |pred: &dyn Fn(&HunlAction) -> bool| to.iter().position(pred);
    let fallback = || find(&passive).unwrap_or(0);
    match *a {
        HunlAction::Fold => find(&|x| *x == HunlAction::Fold).unwrap_or_else(fallback),
        HunlAction::Check | HunlAction::Call => fallback(),
        HunlAction::Bet(x) | HunlAction::Raise(x) => {
            let all_in = x >= fb.all_in_to();
            let frac = fb.pot_fraction(x);
            let mut best: Option<(usize, f64)> = None;
            for (i, y) in to.iter().enumerate() {
                if let HunlAction::Bet(t) | HunlAction::Raise(t) = *y {
                    let d = if all_in && t >= tb.all_in_to() {
                        -1.0
                    } else {
                        (tb.pot_fraction(t) - frac).abs()
                    };
                    if best.is_none_or(|b| d < b.1) {
                        best = Some((i, d));
                    }
                }
            }
            best.map_or_else(fallback, |b| b.0)
        }
    }
}

/// Regret matching over `k` actions per hand, action-major: each hand plays
/// its actions in proportion to positive regret, or uniformly if none.
pub(crate) fn regret_matching(regret: &[f32], n: usize) -> Vec<f32> {
    normalized(regret, n, |r| r.max(0.0))
}

/// Each hand's row of `x` (action-major over `n` hands) scaled by `f` to
/// sum to 1, uniform where it sums to 0.
pub(crate) fn normalized(x: &[f32], n: usize, f: impl Fn(f32) -> f32) -> Vec<f32> {
    let k = x.len() / n.max(1);
    let mut s = vec![0f32; k * n];
    for h in 0..n {
        let total: f32 = (0..k).map(|a| f(x[a * n + h])).sum();
        for a in 0..k {
            s[a * n + h] = if total > 0.0 {
                f(x[a * n + h]) / total
            } else {
                1.0 / k as f32
            };
        }
    }
    s
}

/// Discounted CFR's factors for one iteration.
pub(crate) struct Discount {
    positive: f32,
    negative: f32,
    pub(crate) sum: f32,
}

impl Discount {
    /// The factors for iteration `t` (from 1).
    pub(crate) fn at(t: usize) -> Self {
        let t = t as f64;
        Self {
            positive: (t.powf(ALPHA) / (t.powf(ALPHA) + 1.0)) as f32,
            negative: 0.5,
            sum: ((t / (t + 1.0)).powf(GAMMA)) as f32,
        }
    }

    pub(crate) fn regret(&self, r: f32) -> f32 {
        r * if r > 0.0 {
            self.positive
        } else {
            self.negative
        }
    }
}
