//! Hand review: grading a player's decisions against the blueprint.
//!
//! After a heads-up hand against `GtoBot`, each of the player's decisions is
//! compared with what the blueprint would do in the same spot, holding the
//! same cards:
//!
//! - **Replay** ([`replay`]): the hand is followed on the blueprint's
//!   betting tree exactly as `GtoBot` follows it ([`Follower`]), and both
//!   players' ranges are tracked: each action multiplies every hand's weight
//!   by the blueprint's probability of that action for the hand's bucket. A
//!   bet size between two menu sizes counts as a mix of both, with the
//!   pseudo-harmonic mapping's weights.
//! - **Action values** ([`Evaluator`]): at a decision, the expected value of
//!   every action on the menu for the player's actual hand, against the
//!   opponent's range (never its actual cards), with both players following
//!   the blueprint afterwards. River and turn decisions are exact (every
//!   river card); flop decisions sample turn and river cards and report a
//!   standard error. Preflop decisions are only valued when flops may be sampled
//!   ([`ReviewConfig::preflop_flops`]), which is slow without the full card
//!   abstraction tables; otherwise they're graded on the blueprint's mix
//!   alone.
//! - **Grading** ([`Reviewer`]): the blueprint's main action (its most
//!   likely one) is always fine. An action it mixes in at least
//!   [`ReviewConfig::variant`] of the time is fine too, with a note that
//!   it's the less common choice. Anything else is graded by the big blinds
//!   it gives up against the best action: fine, inaccuracy, mistake or
//!   blunder.
//!
//! What it measures is play against this blueprint within its abstraction,
//! not against perfect play: a strong, consistent yardstick, not an oracle.

use std::collections::HashMap;

use ducy_play::{BettingStructure, Event, Hand, HandSummary, Variant};

use super::{
    abstraction::CardAbstraction,
    blueprint::Blueprint,
    cards::{
        Card, NUM_CARDS, NUM_HOLES, bit, from_ducy, hole_cards, hole_index, mask, score, to_string,
    },
    follow::{Follower, RealBetting, Step},
    hunl::{Betting, BettingTree, HunlAction},
    range::{BucketCache, Range, board_for},
};
use crate::rng::Rng;

/// How a review is done.
#[derive(Clone, Debug, PartialEq)]
pub struct ReviewConfig {
    /// Turn and river cards sampled for a flop decision: a quarter as many
    /// turn cards, with several rivers each, each pair at most once (all
    /// 2,162 makes it exact). 0 leaves flop decisions graded on the mix
    /// alone.
    pub flop_runouts: usize,
    /// Flop and turn cards sampled for a preflop decision. Each needs every
    /// hand's flop bucket, about 0.2 s with a compact abstraction, so the
    /// default 0 grades preflop decisions on the mix alone.
    pub preflop_flops: usize,
    /// An action the blueprint plays at least this often counts as one of
    /// its choices: fine, with a note when it isn't the main one.
    pub variant: f64,
    /// Big blinds lost from which a decision is an inaccuracy, a mistake and
    /// a blunder.
    pub thresholds: [f64; 3],
    /// Seeds the sampling and the mapping of off-menu sizes, so a review is
    /// reproducible.
    pub seed: u64,
}

impl Default for ReviewConfig {
    fn default() -> Self {
        Self {
            flop_runouts: 32,
            preflop_flops: 0,
            variant: 0.05,
            thresholds: [0.25, 1.0, 4.0],
            seed: 1,
        }
    }
}

/// A finished heads-up no-limit hand, as the reviewer needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct HandRecord {
    /// The reviewed player's seat.
    pub seat: usize,
    pub button: usize,
    /// The reviewed player's hole cards.
    pub hole: [Card; 2],
    /// The board as far as it was dealt.
    pub board: Vec<Card>,
    pub history: Vec<Event>,
    pub big_blind: u64,
    /// The reviewed player's result in chips.
    pub net: i64,
    /// Both seats' stacks at the start of the hand, by seat.
    pub stacks: [u64; 2],
}

impl HandRecord {
    /// The hand `summary` describes, from the point of view of its seat.
    /// `None` unless it's heads-up no-limit Hold'em with that seat's cards.
    pub fn from_summary(summary: &HandSummary) -> Option<Self> {
        if summary.result.net.len() != 2
            || summary.rules.variant != Variant::Holdem
            || summary.rules.structure != BettingStructure::NoLimit
        {
            return None;
        }
        let hole: Vec<Card> = summary
            .shown
            .get(summary.seat)?
            .as_ref()?
            .iter(false)
            .map(from_ducy)
            .collect();
        let button = summary.history.iter().find_map(|e| match *e {
            Event::SmallBlind { seat, .. } => Some(seat),
            _ => None,
        })?;
        Some(Self {
            seat: summary.seat,
            button,
            hole: hole.try_into().ok()?,
            board: summary.board.iter().copied().map(from_ducy).collect(),
            history: summary.history.clone(),
            big_blind: summary.rules.big_blind,
            net: summary.result.net[summary.seat],
            stacks: std::array::from_fn(|i| {
                (summary.result.final_stacks[i] as i64 - summary.result.net[i]).max(0) as u64
            }),
        })
    }

    /// The hand being played, as `seat` has seen it so far: their own cards,
    /// the board dealt so far and every action so far. Nothing else of the
    /// deal, so neither the other player's cards nor the cards to come can
    /// affect a review. `None` unless it's heads-up no-limit Hold'em.
    pub fn in_progress(hand: &Hand, seat: usize) -> Option<Self> {
        let rules = hand.rules();
        if hand.num_seats() != 2
            || seat >= 2
            || rules.variant != Variant::Holdem
            || rules.structure != BettingStructure::NoLimit
        {
            return None;
        }
        let hole: Vec<Card> = hand.deal().hole_cards()[seat]
            .iter(false)
            .map(from_ducy)
            .collect();
        let history = hand.events().to_vec();
        let button = history.iter().find_map(|e| match *e {
            Event::SmallBlind { seat, .. } => Some(seat),
            _ => None,
        })?;
        let result = hand.result();
        Some(Self {
            seat,
            button,
            hole: hole.try_into().ok()?,
            board: hand.board().iter().copied().map(from_ducy).collect(),
            history,
            big_blind: rules.big_blind,
            net: result.map_or(0, |r| r.net[seat]),
            stacks: std::array::from_fn(|i| match result {
                Some(r) => (r.final_stacks[i] as i64 - r.net[i]).max(0) as u64,
                None => hand.stack(i) + hand.contributed(i),
            }),
        })
    }

    /// The reviewed player's side on the tree: 0 is the button.
    pub fn side(&self) -> usize {
        usize::from(self.seat != self.button)
    }
}

/// One decision in a replayed hand.
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    /// Who acted: 0 is the button.
    pub player: usize,
    /// The real street: 0 preflop to 3 river.
    pub street: usize,
    /// The tree node the player acted at, or `None` if the hand had left
    /// the tree.
    pub node: Option<u32>,
    /// The real chips before the action.
    pub real: RealBetting,
    /// The real action, amounts in real chips (street totals).
    pub action: HunlAction,
    /// How the action was taken on the tree, if it was.
    pub step: Option<Step>,
    /// Both players' public ranges before the action, by hole index.
    pub ranges: [Vec<f64>; 2],
}

/// Replays a finished hand on the tree and tracks both ranges.
pub fn replay(
    rec: &HandRecord,
    tree: &BettingTree,
    cards: &CardAbstraction,
    blueprint: &Blueprint,
    cache: &mut BucketCache,
    seed: u64,
) -> Vec<Decision> {
    let mut rng = Rng::new(seed);
    let mut f = Follower::new();
    let mut ranges = [Range::new(0), Range::new(0)];
    let side = |seat: usize| usize::from(seat != rec.button);
    let mut out = Vec::new();
    for e in &rec.history {
        let acted = match *e {
            Event::Fold { seat } => Some((seat, HunlAction::Fold)),
            Event::Check { seat } => Some((seat, HunlAction::Check)),
            Event::Call { seat, .. } => Some((seat, HunlAction::Call)),
            Event::Bet { seat, to, .. } => Some((seat, HunlAction::Bet(to))),
            Event::Raise { seat, to, .. } => Some((seat, HunlAction::Raise(to))),
            _ => None,
        };
        let (node, real, street, before) = (f.node, f.real, f.street, f.steps.len());
        f.apply(tree, e, side, &mut rng);
        // Board cards leave both ranges.
        if let Event::Board { street: s, .. } = e {
            let n = [0, 3, 4, 5][super::follow::street_index(*s)];
            let dealt = mask(&rec.board[..n.min(rec.board.len())]);
            for r in &mut ranges {
                r.remove(dealt);
            }
        }
        if let Some((seat, action)) = acted {
            let step = f
                .steps
                .get(before)
                .filter(|s| Some(s.node) == node)
                .cloned();
            out.push(Decision {
                player: side(seat),
                street,
                node,
                real,
                action,
                step,
                ranges: [ranges[0].weight.clone(), ranges[1].weight.clone()],
            });
        }
        // Every new step on the tree updates the actor's range.
        for s in &f.steps[before..] {
            let n = &tree.nodes[s.node as usize];
            let p = n.betting.to_act;
            let bk = cache.get(cards, board_for(n.betting.street, &rec.board));
            let mut rows: HashMap<u16, Vec<f64>> = HashMap::new();
            ranges[p].update_by(|h| {
                let b = bk[h];
                if b == u16::MAX {
                    return 0.0;
                }
                let probs = rows.entry(b).or_insert_with(|| blueprint.probs(s.node, b));
                match s.mapped {
                    Some(m) => m.iter().map(|&(i, w)| w * probs[i]).sum(),
                    None => probs[s.action],
                }
            });
        }
    }
    out
}

/// Every action's value at one decision.
#[derive(Clone, Debug, PartialEq)]
pub struct ActionValues {
    /// Expected value of each action at the node, in big blinds, from the
    /// decision on (folding is 0).
    pub ev: Vec<f64>,
    /// Standard error of each, 0 when exact.
    pub stderr: Vec<f64>,
    /// Runouts the values average over (1 on the river).
    pub runouts: usize,
}

/// Values actions for one player's hand against the other's range, with
/// both following the blueprint afterwards.
pub struct Evaluator<'a> {
    pub tree: &'a BettingTree,
    pub blueprint: &'a Blueprint,
    pub cards: &'a CardAbstraction,
    /// The tree's big blind (chips per big blind in the tree's units).
    pub tree_big_blind: u64,
    /// Every hand's buckets per board, kept across samples and decisions:
    /// with a compact abstraction a flop's take about 0.1 s to compute.
    buckets: std::cell::RefCell<HashMap<Vec<Card>, Vec<u16>>>,
}

/// What one walk needs: the hero's hand and side, the opponent's possible
/// hands, the cards planned for this runout, and per-board caches.
struct Walk<'b> {
    hero: [Card; 2],
    side: usize,
    opp: &'b [(usize, Card, Card)],
    /// Board cards for this runout beyond the current board, in order; the
    /// river is enumerated when it isn't among them.
    planned: Vec<Card>,
    buckets: &'b mut HashMap<Vec<Card>, Vec<u16>>,
    scores: HashMap<Vec<Card>, (u32, Vec<u32>)>,
}

impl<'a> Evaluator<'a> {
    pub fn new(
        tree: &'a BettingTree,
        blueprint: &'a Blueprint,
        cards: &'a CardAbstraction,
        tree_big_blind: u64,
    ) -> Self {
        Self {
            tree,
            blueprint,
            cards,
            tree_big_blind,
            buckets: Default::default(),
        }
    }

    /// Hands it every hand's buckets on `board`, already worked out
    /// elsewhere (e.g. while replaying the hand).
    pub fn know_buckets(&self, board: &[Card], buckets: &[u16]) {
        self.buckets
            .borrow_mut()
            .entry(board.to_vec())
            .or_insert_with(|| buckets.to_vec());
    }

    /// The values of every action at `node` (where `side` acts) for `hero`
    /// with `board`, against the opponent's range `opp_range` (by hole
    /// index). `None` when the opponent's range is empty or the street can't
    /// be valued with `config`.
    #[allow(clippy::too_many_arguments)]
    pub fn values(
        &self,
        node: u32,
        side: usize,
        hero: [Card; 2],
        board: &[Card],
        opp_range: &[f64],
        config: &ReviewConfig,
        rng: &mut Rng,
    ) -> Option<ActionValues> {
        let n = &self.tree.nodes[node as usize];
        if n.actions.is_empty() || n.betting.to_act != side {
            return None;
        }
        let used = mask(board) | bit(hero[0]) | bit(hero[1]);
        let opp: Vec<(usize, Card, Card)> = (0..NUM_HOLES)
            .filter(|&h| opp_range[h] > 0.0)
            .map(|h| {
                let (a, b) = hole_cards(h);
                (h, a, b)
            })
            .filter(|&(_, a, b)| used & (bit(a) | bit(b)) == 0)
            .collect();
        if opp.is_empty() {
            return None;
        }
        let reach: Vec<f64> = opp.iter().map(|o| opp_range[o.0]).collect();
        let deck: Vec<Card> = (0..NUM_CARDS as Card)
            .filter(|&c| used & bit(c) == 0)
            .collect();
        // Cards fixed per sample: none from the turn on (the river is
        // enumerated), a turn and river on the flop, a flop and turn
        // preflop.
        let samples: Vec<Vec<Card>> = match board.len() {
            4 | 5 => vec![Vec::new()],
            3 if config.flop_runouts > 0 => sample(&deck, 2, config.flop_runouts, rng),
            0 if config.preflop_flops > 0 => sample(&deck, 4, config.preflop_flops, rng),
            _ => return None,
        };
        let k = n.actions.len();
        let mut per = vec![Vec::with_capacity(samples.len()); k];
        let mut sums = vec![(0f64, 0f64); k];
        let mut buckets = self.buckets.borrow_mut();
        for planned in samples {
            let mut w = Walk {
                hero,
                side,
                opp: &opp,
                planned,
                buckets: &mut buckets,
                scores: HashMap::new(),
            };
            for (a, &child) in n.children.iter().enumerate() {
                let (v, m) = self.walk(child, board, &reach, &mut w);
                if m > 0.0 {
                    per[a].push(v / m);
                }
                sums[a].0 += v;
                sums[a].1 += m;
            }
        }
        // From the decision on: add back what the player has put in.
        let base = n.betting.contributed[side] as f64;
        let bb = self.tree_big_blind as f64;
        let ev: Vec<f64> = sums
            .iter()
            .map(|&(v, m)| if m > 0.0 { (v / m + base) / bb } else { 0.0 })
            .collect();
        let stderr = per
            .iter()
            .map(|xs| {
                let n = xs.len() as f64;
                if n < 2.0 {
                    return 0.0;
                }
                let mean = xs.iter().sum::<f64>() / n;
                let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
                (var / n).sqrt() / bb
            })
            .collect();
        Some(ActionValues {
            ev,
            stderr,
            runouts: per.iter().map(Vec::len).max().unwrap_or(0),
        })
    }

    /// The hero's total value and the opponent reach it's over, below
    /// `node` with `board` dealt so far.
    fn walk(&self, node: u32, board: &[Card], reach: &[f64], w: &mut Walk) -> (f64, f64) {
        let mass: f64 = reach.iter().sum();
        if mass <= 0.0 {
            return (0.0, 0.0);
        }
        let n = &self.tree.nodes[node as usize];
        let b = &n.betting;
        if n.actions.is_empty() {
            if let Some(f) = b.folded {
                let u = if f == w.side {
                    -(b.contributed[w.side] as f64)
                } else {
                    b.contributed[1 - w.side] as f64
                };
                return (u * mass, mass);
            }
            if board.len() < 5 {
                return self.deal(node, board, reach, w);
            }
            let amount = b.contributed[0].min(b.contributed[1]) as f64;
            let (me, theirs) = self.scores(board, w);
            let v: f64 = reach
                .iter()
                .zip(theirs)
                .map(|(&r, s)| r * (me.cmp(s) as i32) as f64)
                .sum();
            return (v * amount, mass);
        }
        let need = [0, 3, 4, 5][b.street];
        if board.len() < need {
            return self.deal(node, board, reach, w);
        }
        let (side, hero, opp) = (w.side, w.hero, w.opp);
        let bk = self.buckets(&board[..need], w);
        let hero_bucket = bk[hole_index(hero[0], hero[1])];
        let opp_bk: Vec<u16> = opp.iter().map(|o| bk[o.0]).collect();
        if b.to_act == side {
            let probs = self.blueprint.probs(node, hero_bucket);
            let (mut v, mut m) = (0.0, 0.0);
            for (&p, &child) in probs.iter().zip(&n.children) {
                if p > 0.0 {
                    let (cv, cm) = self.walk(child, board, reach, w);
                    v += p * cv;
                    m += p * cm;
                }
            }
            return (v, m);
        }
        // Every action's reach in one pass, with each bucket's row of
        // probabilities looked up once.
        let k = n.children.len();
        let mut rows = vec![f64::NAN; self.cards.num_buckets(need) * k];
        let mut next = vec![vec![0.0; reach.len()]; k];
        for (i, (&r, &bkt)) in reach.iter().zip(&opp_bk).enumerate() {
            if r == 0.0 {
                continue;
            }
            let row = &mut rows[bkt as usize * k..(bkt as usize + 1) * k];
            if row[0].is_nan() {
                row.copy_from_slice(&self.blueprint.probs(node, bkt));
            }
            for (x, &p) in next.iter_mut().zip(row.iter()) {
                x[i] = r * p;
            }
        }
        let (mut v, mut m) = (0.0, 0.0);
        for (&child, reach) in n.children.iter().zip(&next) {
            let (cv, cm) = self.walk(child, board, reach, w);
            v += cv;
            m += cm;
        }
        (v, m)
    }

    /// Deals the next street's cards (or all the rest before a showdown):
    /// planned cards first, then every possible river.
    fn deal(&self, node: u32, board: &[Card], reach: &[f64], w: &mut Walk) -> (f64, f64) {
        let next_len = match board.len() {
            0 => 3,
            3 => 4,
            _ => 5,
        };
        // Every opponent hand leaves `unseen - 2` of the unseen cards: each
        // card it allows comes with probability 1 / (unseen - 2) given the
        // hand, which keeps paths through a deal and paths ending before one
        // on the same footing.
        let unseen = (NUM_CARDS - board.len() - 2) as f64;
        let have = board.len() + w.planned.len();
        if have >= next_len {
            let k = next_len - board.len();
            let mut nb = board.to_vec();
            nb.extend_from_slice(&w.planned[..k]);
            // Sampled uniformly from the unseen cards: weight each by how much
            // likelier it is given the opponent's hand.
            let factor: f64 = (0..k)
                .map(|i| (unseen - i as f64) / (unseen - 2.0 - i as f64))
                .product();
            let next: Vec<f64> = without(reach, w.opp, mask(&nb))
                .into_iter()
                .map(|r| r * factor)
                .collect();
            let planned = w.planned.split_off(k);
            let saved = std::mem::replace(&mut w.planned, planned);
            let r = self.walk(node, &nb, &next, w);
            let mut restored = saved;
            restored.extend(std::mem::take(&mut w.planned));
            w.planned = restored;
            return r;
        }
        // Enumerate the river.
        let used = mask(board) | bit(w.hero[0]) | bit(w.hero[1]);
        let (mut v, mut m) = (0.0, 0.0);
        for c in 0..NUM_CARDS as Card {
            if used & bit(c) != 0 {
                continue;
            }
            let mut nb = board.to_vec();
            nb.push(c);
            let (cv, cm) = self.walk(node, &nb, &without(reach, w.opp, bit(c)), w);
            v += cv;
            m += cm;
        }
        (v / (unseen - 2.0), m / (unseen - 2.0))
    }

    fn buckets<'w>(&self, board: &[Card], w: &'w mut Walk) -> &'w [u16] {
        w.buckets
            .entry(board.to_vec())
            .or_insert_with(|| self.cards.buckets(board))
    }

    fn scores<'w>(&self, board: &[Card], w: &'w mut Walk) -> (u32, &'w [u32]) {
        let (hero, opp) = (w.hero, w.opp);
        let e = w.scores.entry(board.to_vec()).or_insert_with(|| {
            let bm = mask(board);
            let me = score(bm | bit(hero[0]) | bit(hero[1]));
            let theirs = opp
                .iter()
                .map(|&(_, a, b)| score(bm | bit(a) | bit(b)))
                .collect();
            (me, theirs)
        });
        (e.0, &e.1)
    }
}

/// `reach` with the hands that hold a card of `cards` left out.
fn without(reach: &[f64], opp: &[(usize, Card, Card)], cards: u64) -> Vec<f64> {
    reach
        .iter()
        .zip(opp)
        .map(|(&r, &(_, a, b))| {
            if cards & (bit(a) | bit(b)) != 0 {
                0.0
            } else {
                r
            }
        })
        .collect()
}

/// `count` draws of `k` distinct cards from `deck`, in order, each distinct.
/// Two cards (a turn and a river) come as a quarter as many turn cards with
/// several rivers each: every pair is as likely as any other, and only that
/// many turn boards need every hand's bucket, the slow part with a compact
/// abstraction. Asking for every pair gets every pair.
fn sample(deck: &[Card], k: usize, count: usize, rng: &mut Rng) -> Vec<Vec<Card>> {
    let mut d = deck.to_vec();
    let n = d.len();
    if k == 1 {
        shuffle(&mut d, rng);
        return d.iter().take(count).map(|&c| vec![c]).collect();
    }
    if k == 2 && n >= 2 {
        let turns = count.div_ceil(4).clamp(1, n);
        let rivers = count.div_ceil(turns).clamp(1, n - 1);
        shuffle(&mut d, rng);
        let mut out = Vec::with_capacity(turns * rivers);
        for &t in &d[..turns] {
            let mut rest: Vec<Card> = deck.iter().copied().filter(|&c| c != t).collect();
            shuffle(&mut rest, rng);
            out.extend(rest[..rivers].iter().map(|&r| vec![t, r]));
        }
        return out;
    }
    (0..count)
        .map(|_| {
            shuffle(&mut d, rng);
            d[..k].to_vec()
        })
        .collect()
}

fn shuffle(d: &mut [Card], rng: &mut Rng) {
    for i in (1..d.len()).rev() {
        let j = (rng.next_u64() % (i as u64 + 1)) as usize;
        d.swap(i, j);
    }
}

/// How a decision is graded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Grade {
    /// The blueprint's play, or close enough in value.
    Fine,
    Inaccuracy,
    Mistake,
    Blunder,
    /// Not something the blueprint does, but its cost couldn't be valued
    /// (preflop without sampling).
    Deviation,
    /// The hand had left the blueprint's tree, so there's nothing to compare.
    Unknown,
}

/// One action of the blueprint's at a decision.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ActionReview {
    /// Like "bet 4.5bb (75% pot)".
    pub action: String,
    /// How often the blueprint plays it with this hand.
    pub frequency: f64,
    /// Its value in big blinds from the decision on, when valued.
    pub ev: Option<f64>,
}

/// The review of one decision.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DecisionReview {
    /// "preflop", "flop", "turn" or "river".
    pub street: String,
    /// Like "Ks 9d 4h".
    pub board: String,
    /// The pot before the action, in big blinds.
    pub pot: f64,
    /// To call, in big blinds.
    pub to_call: f64,
    /// What the player did, like "raise to 9bb (80% pot)".
    pub action: String,
    /// The blueprint's actions here, with how often it plays each.
    pub gto: Vec<ActionReview>,
    /// How often the blueprint makes the player's play (a bet between two
    /// menu sizes counts as a mix of both).
    pub frequency: f64,
    /// The blueprint's main (most frequent) action.
    pub main: String,
    /// The most valuable action, when valued.
    pub best: Option<String>,
    /// The player's play's value, in big blinds from the decision on.
    pub ev: Option<f64>,
    /// Value given up against the best action, in big blinds.
    pub ev_loss: Option<f64>,
    /// What's charged to the player: the loss, or 0 for a play the blueprint
    /// makes (mixed strategies leave small differences between them).
    pub bb_lost: f64,
    /// Standard error of the values (0 when exact).
    pub stderr: f64,
    pub grade: Grade,
    /// An explanation, e.g. for a less common mixed play.
    pub note: Option<String>,
}

/// The review of one hand.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct HandReview {
    /// The player's cards, like "Ah Kd".
    pub hole: String,
    /// The board as far as it was dealt.
    pub board: String,
    pub decisions: Vec<DecisionReview>,
    /// Big blinds charged over the hand's decisions.
    pub bb_lost: f64,
    /// The decision that cost the most, if any cost anything.
    pub worst: Option<usize>,
    /// What the player actually won or lost, in big blinds: luck included,
    /// so kept apart from the grading.
    pub result: f64,
    /// A caveat for the whole hand, e.g. stacks deeper or shorter than the
    /// blueprint was trained for, which makes its grades approximate.
    pub note: Option<String>,
}

/// Everything needed to review hands against a blueprint.
pub struct Reviewer<'a> {
    pub tree: &'a BettingTree,
    pub blueprint: &'a Blueprint,
    pub cards: &'a CardAbstraction,
    /// The blueprint game's big blind, in tree chips.
    pub tree_big_blind: u64,
    pub config: ReviewConfig,
}

impl Reviewer<'_> {
    /// Reviews the decisions of `rec`'s player.
    pub fn review(&self, rec: &HandRecord, cache: &mut BucketCache) -> HandReview {
        let decisions = replay(
            rec,
            self.tree,
            self.cards,
            self.blueprint,
            cache,
            self.config.seed,
        );
        let me = rec.side();
        let eval = self.evaluator(rec, cache);
        let mut rng = Rng::new(self.config.seed ^ 0x7e1e);
        let bb = rec.big_blind.max(1) as f64;
        let mut out = Vec::new();
        for d in decisions.iter().filter(|d| d.player == me) {
            out.push(self.decision(rec, d, &eval, &mut rng, bb));
        }
        let bb_lost: f64 = out.iter().map(|d| d.bb_lost).sum();
        let worst = out
            .iter()
            .enumerate()
            .filter(|(_, d)| d.bb_lost > 0.0)
            .max_by(|a, b| a.1.bb_lost.total_cmp(&b.1.bb_lost))
            .map(|(i, _)| i);
        HandReview {
            hole: cards_text(&rec.hole),
            board: cards_text(&rec.board),
            decisions: out,
            bb_lost,
            worst,
            result: rec.net as f64 / bb,
            note: self.depth_note(rec),
        }
    }

    /// Reviews only the player's most recent decision in `rec`, e.g. a hand
    /// still being played ([`HandRecord::in_progress`]), so feedback can
    /// follow each action. `None` before the player's first decision. The
    /// grade matches [`Self::review`]'s for the same decision, within the
    /// sampling error.
    pub fn review_last_decision(
        &self,
        rec: &HandRecord,
        cache: &mut BucketCache,
    ) -> Option<DecisionReview> {
        let decisions = replay(
            rec,
            self.tree,
            self.cards,
            self.blueprint,
            cache,
            self.config.seed,
        );
        let d = decisions.iter().rev().find(|d| d.player == rec.side())?;
        let eval = self.evaluator(rec, cache);
        let mut rng = Rng::new(self.config.seed ^ 0x7e1e);
        Some(self.decision(rec, d, &eval, &mut rng, rec.big_blind.max(1) as f64))
    }

    /// An evaluator that knows every street's buckets the replay worked out.
    fn evaluator(&self, rec: &HandRecord, cache: &mut BucketCache) -> Evaluator<'_> {
        let eval = Evaluator::new(self.tree, self.blueprint, self.cards, self.tree_big_blind);
        for n in [0, 3, 4, 5] {
            if n <= rec.board.len() {
                let board = &rec.board[..n];
                eval.know_buckets(board, cache.get(self.cards, board));
            }
        }
        eval
    }

    /// A note when the hand's effective stack is more than 10% off the
    /// blueprint's: its all-ins and big bets then mean something else.
    fn depth_note(&self, rec: &HandRecord) -> Option<String> {
        let root = &self.tree.nodes[0].betting;
        let tree_bb = self.tree_big_blind.max(1) as f64;
        let trained = (root.stack[0] + root.contributed[0]) as f64 / tree_bb;
        let real = rec.stacks[0].min(rec.stacks[1]) as f64 / rec.big_blind.max(1) as f64;
        ((real - trained).abs() > 0.1 * trained).then(|| {
            format!(
                "stacks were {real:.0} bb, but the bot plays {trained:.0} bb stacks, so these grades are approximate"
            )
        })
    }

    fn decision(
        &self,
        rec: &HandRecord,
        d: &Decision,
        eval: &Evaluator,
        rng: &mut Rng,
        bb: f64,
    ) -> DecisionReview {
        let board = &rec.board[..[0, 3, 4, 5][d.street].min(rec.board.len())];
        let pot = d.real.pot() as f64 / bb;
        let to_call = d
            .real
            .current_bet
            .saturating_sub(d.real.street_bet[d.player]) as f64
            / bb;
        let action = real_text(d, bb);
        let mut r = DecisionReview {
            street: ["preflop", "flop", "turn", "river"][d.street].to_string(),
            board: cards_text(board),
            pot,
            to_call,
            action,
            gto: Vec::new(),
            frequency: 0.0,
            main: String::new(),
            best: None,
            ev: None,
            ev_loss: None,
            bb_lost: 0.0,
            stderr: 0.0,
            grade: Grade::Unknown,
            note: Some("the hand had left the blueprint's betting tree".to_string()),
        };
        let (Some(node), Some(step)) = (d.node, d.step.as_ref()) else {
            return r;
        };
        let n = &self.tree.nodes[node as usize];
        if n.betting.street != d.street {
            return r;
        }
        let bucket = self.cards.bucket(rec.hole, board);
        let probs = self.blueprint.probs(node, bucket);
        let taken: Vec<(usize, f64)> = match step.mapped {
            Some(m) => m.to_vec(),
            None => vec![(step.action, 1.0)],
        };
        let values = eval.values(
            node,
            d.player,
            rec.hole,
            board,
            &d.ranges[1 - d.player],
            &self.config,
            rng,
        );
        // Values in the tree's chips, scaled to the real pot.
        let scale = pot / (n.betting.pot() as f64 / self.tree_big_blind as f64).max(1e-9);
        let ev: Option<Vec<f64>> = values
            .as_ref()
            .map(|v| v.ev.iter().map(|x| x * scale).collect());
        let main = (0..probs.len())
            .max_by(|&a, &b| probs[a].total_cmp(&probs[b]))
            .unwrap_or(0);
        r.gto = n
            .actions
            .iter()
            .enumerate()
            .map(|(i, a)| ActionReview {
                action: tree_text(&n.betting, a, self.tree_big_blind),
                frequency: probs[i],
                ev: ev.as_ref().map(|e| e[i]),
            })
            .collect();
        r.main = r.gto[main].action.clone();
        r.frequency = taken.iter().map(|&(i, w)| w * probs[i]).sum();
        if let (Some(ev), Some(v)) = (&ev, &values) {
            let best = (0..ev.len())
                .max_by(|&a, &b| ev[a].total_cmp(&ev[b]))
                .unwrap_or(0);
            let mine: f64 = taken.iter().map(|&(i, w)| w * ev[i]).sum();
            r.best = Some(r.gto[best].action.clone());
            r.ev = Some(mine);
            r.ev_loss = Some((ev[best] - mine).max(0.0));
            r.stderr = taken.iter().map(|&(i, w)| w * v.stderr[i] * scale).sum();
        }
        r.note = None;
        let is_main = taken.iter().any(|&(i, w)| i == main && w >= 0.5);
        let c = &self.config;
        if is_main {
            r.grade = Grade::Fine;
        } else if r.frequency >= c.variant {
            r.grade = Grade::Fine;
            r.note = Some(format!(
                "a less common choice: the blueprint plays this {:.0}% of the time; its main play is {} ({:.0}%)",
                100.0 * r.frequency,
                r.main,
                100.0 * probs[main]
            ));
        } else if let Some(loss) = r.ev_loss {
            // Losses within the sampling error aren't charged.
            let charged = if loss <= 2.0 * r.stderr { 0.0 } else { loss };
            r.bb_lost = charged;
            r.grade = if charged < c.thresholds[0] {
                Grade::Fine
            } else if charged < c.thresholds[1] {
                Grade::Inaccuracy
            } else if charged < c.thresholds[2] {
                Grade::Mistake
            } else {
                Grade::Blunder
            };
            if r.grade == Grade::Fine {
                r.note = Some(format!(
                    "not the blueprint's play, but close in value; its main play is {} ({:.0}%)",
                    r.main,
                    100.0 * probs[main]
                ));
            }
        } else {
            r.grade = Grade::Deviation;
            r.note = Some(format!(
                "the blueprint rarely plays this ({:.0}%); its main play is {} ({:.0}%)",
                100.0 * r.frequency,
                r.main,
                100.0 * probs[main]
            ));
        }
        r
    }
}

/// Totals over several reviewed hands.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SessionReview {
    pub hands: usize,
    pub decisions: usize,
    /// Big blinds charged in all.
    pub bb_lost: f64,
    /// Per 100 hands.
    pub bb_lost_per_100: f64,
    /// Decisions by grade: fine, inaccuracy, mistake, blunder, deviation,
    /// unknown.
    pub fine: usize,
    pub inaccuracies: usize,
    pub mistakes: usize,
    pub blunders: usize,
    pub deviations: usize,
    pub unknown: usize,
    /// Decisions made with a less common mixed play.
    pub variants: usize,
    /// What the player actually won, in big blinds (luck included).
    pub result: f64,
    /// The costliest decisions, worst first (at most 5).
    pub worst: Vec<WorstDecision>,
}

/// A costly decision in a session.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct WorstDecision {
    /// Which hand, in the order reviewed.
    pub hand: usize,
    /// Which of the player's decisions in it.
    pub decision: usize,
    pub bb_lost: f64,
}

impl SessionReview {
    pub fn of(hands: &[HandReview]) -> Self {
        let mut s = Self {
            hands: hands.len(),
            ..Self::default()
        };
        let mut all = Vec::new();
        for (h, r) in hands.iter().enumerate() {
            s.result += r.result;
            s.bb_lost += r.bb_lost;
            for (i, d) in r.decisions.iter().enumerate() {
                s.decisions += 1;
                match d.grade {
                    Grade::Fine => s.fine += 1,
                    Grade::Inaccuracy => s.inaccuracies += 1,
                    Grade::Mistake => s.mistakes += 1,
                    Grade::Blunder => s.blunders += 1,
                    Grade::Deviation => s.deviations += 1,
                    Grade::Unknown => s.unknown += 1,
                }
                if d.grade == Grade::Fine
                    && d.note
                        .as_deref()
                        .is_some_and(|n| n.starts_with("a less common"))
                {
                    s.variants += 1;
                }
                if d.bb_lost > 0.0 {
                    all.push(WorstDecision {
                        hand: h,
                        decision: i,
                        bb_lost: d.bb_lost,
                    });
                }
            }
        }
        all.sort_by(|a, b| b.bb_lost.total_cmp(&a.bb_lost));
        all.truncate(5);
        s.worst = all;
        s.bb_lost_per_100 = if s.hands > 0 {
            100.0 * s.bb_lost / s.hands as f64
        } else {
            0.0
        };
        s
    }
}

fn cards_text(cards: &[Card]) -> String {
    cards
        .iter()
        .map(|&c| to_string(c))
        .collect::<Vec<_>>()
        .join(" ")
}

fn bb_text(x: f64) -> String {
    let s = format!("{x:.1}");
    s.strip_suffix(".0").map_or(s.clone(), str::to_string)
}

/// A tree action, sized in big blinds and as a share of the pot.
fn tree_text(b: &Betting, a: &HunlAction, tree_bb: u64) -> String {
    let bb = tree_bb.max(1) as f64;
    match *a {
        HunlAction::Fold => "fold".into(),
        HunlAction::Check => "check".into(),
        HunlAction::Call => "call".into(),
        HunlAction::Bet(to) | HunlAction::Raise(to) => {
            let verb = if matches!(a, HunlAction::Bet(_)) {
                "bet"
            } else {
                "raise to"
            };
            if to >= b.all_in_to() {
                "all-in".into()
            } else if b.street == 0 {
                format!("{verb} {}bb", bb_text(to as f64 / bb))
            } else {
                format!("{verb} {:.0}% pot", 100.0 * b.pot_fraction(to))
            }
        }
    }
}

/// The real action, in big blinds and as a share of the pot.
fn real_text(d: &Decision, bb: f64) -> String {
    let r = &d.real;
    match d.action {
        HunlAction::Fold => "fold".into(),
        HunlAction::Check => "check".into(),
        HunlAction::Call => format!(
            "call {}bb",
            bb_text(r.current_bet.saturating_sub(r.street_bet[d.player]) as f64 / bb)
        ),
        HunlAction::Bet(to) | HunlAction::Raise(to) => {
            let verb = if matches!(d.action, HunlAction::Bet(_)) {
                "bet"
            } else {
                "raise to"
            };
            let to_call = r.current_bet.saturating_sub(r.street_bet[d.player]);
            let frac = super::hunl::pot_fraction(to, r.current_bet, to_call, r.pot());
            if d.street == 0 {
                format!("{verb} {}bb", bb_text(to as f64 / bb))
            } else {
                format!(
                    "{verb} {}bb ({:.0}% pot)",
                    bb_text(to as f64 / bb),
                    100.0 * frac
                )
            }
        }
    }
}

/// The hands a player has finished against GtoBot, and their reviews: each
/// hand is recorded as it ends, and reviewed when first asked for, so a
/// player can play several hands and then look at them all.
#[derive(Default)]
pub struct ReviewLog {
    records: Vec<HandRecord>,
    reviews: Vec<Option<HandReview>>,
    cache: BucketCache,
}

impl ReviewLog {
    /// Records a finished hand from its summary (for the summary's seat).
    /// Returns false, recording nothing, unless it's heads-up no-limit
    /// Hold'em with that seat's cards.
    pub fn record(&mut self, summary: &HandSummary) -> bool {
        match HandRecord::from_summary(summary) {
            Some(r) => {
                self.records.push(r);
                self.reviews.push(None);
                true
            }
            None => false,
        }
    }

    /// Hands recorded.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Forgets every hand.
    pub fn clear(&mut self) {
        self.records.clear();
        self.reviews.clear();
    }

    fn ensure(&mut self, i: usize, reviewer: &Reviewer) {
        if self.reviews[i].is_none() {
            self.reviews[i] = Some(reviewer.review(&self.records[i], &mut self.cache));
        }
    }

    /// The last hand's review.
    pub fn last(&mut self, reviewer: &Reviewer) -> Option<&HandReview> {
        let i = self.records.len().checked_sub(1)?;
        self.ensure(i, reviewer);
        self.reviews[i].as_ref()
    }

    /// Every hand's review, and the session's totals.
    pub fn all(&mut self, reviewer: &Reviewer) -> (Vec<HandReview>, SessionReview) {
        for i in 0..self.records.len() {
            self.ensure(i, reviewer);
        }
        let hands: Vec<HandReview> = self.reviews.iter().flatten().cloned().collect();
        let session = SessionReview::of(&hands);
        (hands, session)
    }
}
