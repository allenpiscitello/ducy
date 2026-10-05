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

use super::{
    abstraction::CardAbstraction,
    blueprint::Blueprint,
    cards::{Card, NUM_CARDS, NUM_HOLES, bit, hole_cards, mask, score},
    hunl::{Hunl, HunlAction, HunlState},
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
    let one = |i: usize| -> f64 {
        let mut rng = Rng::for_iteration(seed, i as u64);
        let seat = i % 2;
        play_hand(game, cards, blueprint, seat, &mut rng) / game.config.big_blind as f64 * 1000.0
    };
    #[cfg(feature = "parallel")]
    let results: Vec<f64> = {
        use rayon::prelude::*;
        (0..hands).into_par_iter().map(one).collect()
    };
    #[cfg(not(feature = "parallel"))]
    let results: Vec<f64> = (0..hands).map(one).collect();
    let n = results.len().max(1) as f64;
    let mean = results.iter().sum::<f64>() / n;
    let var = results.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0);
    LbrResult {
        hands,
        mbb_per_hand: mean,
        ci95: 1.96 * (var / n).sqrt(),
    }
}

/// The blueprint's range: a weight per hole-card hand, and each hand's
/// bucket on the current street.
struct Range {
    weight: Vec<f64>,
    bucket: Vec<u16>,
    street: usize,
}

impl Range {
    fn new(blocked: u64) -> Self {
        let weight = (0..NUM_HOLES)
            .map(|h| {
                let (a, b) = hole_cards(h);
                if blocked & (bit(a) | bit(b)) != 0 {
                    0.0
                } else {
                    1.0
                }
            })
            .collect();
        Self {
            weight,
            bucket: vec![0; NUM_HOLES],
            street: usize::MAX,
        }
    }

    /// Recomputes buckets for `board` when the street changes, and drops
    /// hands that collide with new board cards.
    fn on_street(&mut self, cards: &CardAbstraction, street: usize, board: &[Card]) {
        if self.street == street {
            return;
        }
        self.street = street;
        let bm = mask(board);
        for h in 0..NUM_HOLES {
            if self.weight[h] == 0.0 {
                continue;
            }
            let (a, b) = hole_cards(h);
            if bm & (bit(a) | bit(b)) != 0 {
                self.weight[h] = 0.0;
                continue;
            }
            self.bucket[h] = cards.bucket([a, b], board);
        }
    }
}

fn board_for(street: usize, board: &[Card; 5]) -> &[Card] {
    &board[..[0, 3, 4, 5][street]]
}

/// One hand, LBR in `seat` (0 is the button). Returns LBR's chip result.
fn play_hand(
    game: &Hunl,
    cards: &CardAbstraction,
    blueprint: &Blueprint,
    seat: usize,
    rng: &mut Rng,
) -> f64 {
    let mut s = game.sample_chance(&game.root(), rng);
    let me = s.hole[seat];
    let mut range = Range::new(bit(me[0]) | bit(me[1]));
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
                range.on_street(cards, b.street, board);
                let a = if p == seat {
                    lbr_action(game, blueprint, &s, &range, me, board, rng)
                } else {
                    // The blueprint acts with its real hand; LBR updates the
                    // range with every hand's probability of that action.
                    let probs = blueprint.probs(s.node, s.buckets[p][b.street]);
                    let a = rng.sample(&probs);
                    for h in 0..NUM_HOLES {
                        if range.weight[h] > 0.0 {
                            range.weight[h] *= blueprint.probs(s.node, range.bucket[h])[a];
                        }
                    }
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

fn lbr_action(
    game: &Hunl,
    blueprint: &Blueprint,
    s: &HunlState,
    range: &Range,
    me: [Card; 2],
    board: &[Card],
    rng: &mut Rng,
) -> usize {
    let node = &game.tree.nodes[s.node as usize];
    let b = &node.betting;
    let pot = b.pot() as f64;
    let call = b.to_call() as f64;
    let wp = equity(me, board, &range.weight, rng);
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
                let mut calling = range.weight.clone();
                let (mut folded, mut total) = (0.0, 0.0);
                for (c, (&w, &bucket)) in calling
                    .iter_mut()
                    .zip(range.weight.iter().zip(&range.bucket))
                {
                    if w <= 0.0 {
                        continue;
                    }
                    let f = fold.map_or(0.0, |k| blueprint.probs(child, bucket)[k]);
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
