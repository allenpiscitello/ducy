//! What opponents probably hold, from what they did this hand, and equity
//! against that instead of against random hands (#205).
//!
//! Each opponent still in gets a [`Read`]:
//!
//! - **Preflop:** a raise puts them in the top of their raising range (their
//!   PFR from [`OpponentModel`], a prior until it has seen them), a re-raise
//!   in a third of that, a call or limp in their playing range (VPIP), a
//!   call of a raise in most of it. Checking the big blind says nothing.
//!   Starting hands are ranked by heads-up equity against a random hand
//!   (Hold'em, a table built once) or by ducy's Omaha model
//!   ([`omaha_analysis::percentile`]).
//! - **Each street after the flop:** a bet keeps the hands in the top
//!   [`BET_SHARE`] by made strength on that street's board, a raise the top
//!   [`RAISE_SHARE`], a call of a bet the top [`CALL_SHARE`]. A player seen
//!   betting and raising far more than it calls (past [`AGGRESSIVE`]) gets
//!   those shares widened in proportion (up to [`MAX_WIDEN`] times), so
//!   bluffers' bets count for less. Hands that miss are still kept [`BLUFF_ALLOWANCE`] of the time,
//!   for draws and bluffs, so a range never collapses to the nuts.
//!
//! [`range_equity`] then deals opponents' hands from their reads by
//! rejection sampling, runs out the board, and scores showdowns with a fast
//! scorer (ducy's [`StandardHandRanker::score`]).

use std::sync::OnceLock;

use ducy::{
    deck::{Card, Deck},
    games::omaha_analysis,
    ranking::hand_rank::StandardHandRanker,
};
use rand::{RngExt, SeedableRng, rngs::StdRng};

use ducy_play::{Event, Observation, Variant};

use crate::stats::OpponentModel;

/// Share of hands, by made strength on the board, that bet a street.
pub const BET_SHARE: f64 = 0.45;
/// ... that raise.
pub const RAISE_SHARE: f64 = 0.25;
/// ... that call a bet.
pub const CALL_SHARE: f64 = 0.7;
/// Chance a hand outside a street's share is kept anyway (draws, bluffs).
pub const BLUFF_ALLOWANCE: f64 = 0.25;
/// The most an aggressive player's betting and raising shares are widened.
pub const MAX_WIDEN: f64 = 2.0;
/// Bets and raises per call after the flop beyond which a player's bets are
/// read as wider (an average player makes about one per call).
pub const AGGRESSIVE: f64 = 2.0;
/// Random hands scored per street to find the strength cut-offs.
const CUTOFF_HANDS: usize = 240;
/// Tries to deal an opponent a hand that fits its read before taking any.
const MAX_TRIES: usize = 80;

/// What one opponent's actions say about its hand.
#[derive(Clone, Debug, PartialEq)]
pub struct Read {
    /// The seat.
    pub seat: usize,
    /// Its hand is in the top `preflop` share of starting hands (1 = any).
    pub preflop: f64,
    /// For each street it acted on after the flop: board cards then (3, 4
    /// or 5) and the top share of hands by made strength it keeps.
    pub streets: Vec<(usize, f64)>,
}

/// The reads of every opponent still in the hand, from this hand's history.
pub fn reads(obs: &Observation, model: &OpponentModel) -> Vec<Read> {
    let live: Vec<usize> = (0..obs.seats.len())
        .filter(|&s| s != obs.seat && !obs.seats[s].folded)
        .collect();
    let mut out: Vec<Read> = live
        .iter()
        .map(|&seat| Read {
            seat,
            preflop: 1.0,
            streets: Vec::new(),
        })
        .collect();
    let mut board = 0usize;
    let mut raises = 0usize; // on this street
    let mut bet_open = false; // a bet or raise to call on this street
    for e in &obs.history {
        let (seat, kind) = match e {
            Event::Board { cards, .. } => {
                board += cards.len();
                raises = 0;
                bet_open = false;
                continue;
            }
            Event::BigBlind { .. } if board == 0 => {
                bet_open = true;
                continue;
            }
            Event::Call { seat, .. } => (*seat, 'c'),
            Event::Bet { seat, .. } => (*seat, 'b'),
            Event::Raise { seat, .. } => (*seat, 'r'),
            _ => continue,
        };
        if let Some(r) = out.iter_mut().find(|r| r.seat == seat) {
            let stats = model.seat(seat);
            if board == 0 {
                let share = match (kind, raises) {
                    ('r', 0) => stats.pfr(),
                    ('r', _) => stats.pfr() / 3.0,
                    ('c', 0) => stats.vpip(),
                    _ => stats.vpip() * 0.7,
                };
                r.preflop = r.preflop.min(share.clamp(0.02, 1.0));
            } else {
                // A player who bets and raises far more than it calls bets
                // weaker hands too: past `AGGRESSIVE` bets and raises per
                // call after the flop, widen its betting range in proportion.
                let widen = (stats.aggression() / AGGRESSIVE).clamp(1.0, MAX_WIDEN);
                let share = match kind {
                    'b' => (BET_SHARE * widen).min(0.9),
                    'r' => (RAISE_SHARE * widen).min(0.9),
                    _ if bet_open => CALL_SHARE,
                    _ => 1.0,
                };
                match r.streets.iter_mut().find(|(b, _)| *b == board) {
                    Some((_, s)) => *s = s.min(share),
                    None => r.streets.push((board, share)),
                }
            }
        }
        if matches!(kind, 'b' | 'r') {
            raises += 1;
            bet_open = true;
        }
    }
    out
}

/// The best five-card score `hole` makes with `board` (3 to 5 cards): any
/// five of the seven in Hold'em, exactly two hole cards and three board
/// cards in Omaha.
pub fn made_score(variant: Variant, hole: &[Card], board: &[Card]) -> u32 {
    let deck = |cards: &mut dyn Iterator<Item = &Card>| {
        let mut d = Deck::empty();
        for &c in cards {
            d |= c;
        }
        d
    };
    match variant {
        Variant::Holdem => StandardHandRanker::score(&deck(&mut hole.iter().chain(board))),
        Variant::Omaha { .. } => {
            let mut best = 0;
            for i in 0..hole.len() {
                for j in i + 1..hole.len() {
                    for a in 0..board.len() {
                        for b in a + 1..board.len() {
                            for c in b + 1..board.len() {
                                let five = [hole[i], hole[j], board[a], board[b], board[c]];
                                best = best.max(StandardHandRanker::score(&deck(&mut five.iter())));
                            }
                        }
                    }
                }
            }
            best
        }
    }
}

/// Whether `hole` holds the nuts on `board` (3 to 5 cards) right now: no
/// hand an opponent could hold makes a better five-card hand (a tie still
/// counts). In Hold'em an opponent holds two unseen cards; in Omaha its best
/// hand uses exactly two of its cards. Either way, checking every pair of
/// unseen cards with the board covers every hand it could make.
pub fn has_nuts(variant: Variant, hole: &[Card], board: &[Card]) -> bool {
    if board.len() < 3 {
        return false;
    }
    let mine = made_score(variant, hole, board);
    let mut known = Deck::empty();
    for &c in hole.iter().chain(board) {
        known |= c;
    }
    let unseen: Vec<Card> = (Deck::all_cards() - known).iter(false).collect();
    // A pair of cards scored as a two-card hand of the same game.
    let pair_variant = match variant {
        Variant::Holdem => Variant::Holdem,
        Variant::Omaha { .. } => Variant::Omaha { hole_cards: 2 },
    };
    for (i, &a) in unseen.iter().enumerate() {
        for &b in &unseen[i + 1..] {
            if made_score(pair_variant, &[a, b], board) > mine {
                return false;
            }
        }
    }
    true
}

/// Share of starting hands at least as good as `hole`: 0 is the best hand,
/// 1 the worst.
pub fn preflop_top(variant: Variant, hole: &[Card]) -> f64 {
    match variant {
        Variant::Holdem => holdem_top(hole),
        Variant::Omaha { .. } => omaha_analysis::percentile(hole).map_or(1.0, |p| p / 100.0),
    }
}

/// Hold'em's 169 starting-hand classes: (high rank, low rank, suited).
fn class(hole: &[Card]) -> usize {
    let (a, b) = (hole[0].rank() as usize, hole[1].rank() as usize);
    let (hi, lo) = (a.max(b), a.min(b));
    let suited = hole[0].suit() == hole[1].suit();
    if hi == lo {
        hi
    } else if suited {
        13 + hi * 13 + lo
    } else {
        13 + 169 + hi * 13 + lo
    }
}

/// Share of hands at least as good as each class's, by heads-up equity
/// against a random hand: built once, from a fixed seed.
fn holdem_top(hole: &[Card]) -> f64 {
    static TABLE: OnceLock<Vec<f64>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let cards: Vec<Card> = Deck::all_cards().iter(false).collect();
        let mut rng = StdRng::seed_from_u64(0x0205);
        let mut classes: Vec<(usize, f64, f64)> = Vec::new(); // (class, equity, combos)
        let mut seen = vec![false; 13 + 2 * 169];
        for (i, &x) in cards.iter().enumerate() {
            for &y in &cards[i + 1..] {
                let k = class(&[x, y]);
                if seen[k] {
                    continue;
                }
                seen[k] = true;
                let combos = if x.rank() == y.rank() {
                    6.0
                } else if x.suit() == y.suit() {
                    4.0
                } else {
                    12.0
                };
                let e = equity(Variant::Holdem, &[x, y], &[], &[], 1, 600, &mut rng);
                classes.push((k, e, combos));
            }
        }
        classes.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut top = vec![1.0; 13 + 2 * 169];
        let mut above = 0.0;
        for (k, _, combos) in classes {
            above += combos;
            top[k] = above / 1326.0;
        }
        top
    });
    table[class(hole)]
}

/// Equity of `hole` on `board` against `opponents` hands, each dealt from
/// `reads` (one per opponent, or none for random hands), from `samples`
/// Monte Carlo deals.
fn equity(
    variant: Variant,
    hole: &[Card],
    board: &[Card],
    reads: &[Read],
    opponents: usize,
    samples: usize,
    rng: &mut StdRng,
) -> f64 {
    let k = variant.hole_cards();
    let mut known = Deck::empty();
    for &c in hole.iter().chain(board) {
        known |= c;
    }
    let unknown: Vec<Card> = (Deck::all_cards() - known).iter(false).collect();
    let need = 5 - board.len();
    if opponents * k + need > unknown.len() || samples == 0 {
        return 0.0;
    }
    // Strength cut-offs per street an opponent acted on.
    let cutoffs: Vec<(usize, Vec<u32>)> = {
        let mut lens: Vec<usize> = reads
            .iter()
            .flat_map(|r| r.streets.iter().map(|s| s.0))
            .collect();
        lens.sort_unstable();
        lens.dedup();
        lens.into_iter()
            .map(|len| {
                let mut scores: Vec<u32> = (0..CUTOFF_HANDS)
                    .map(|_| {
                        let h = deal(&unknown, Deck::empty(), k, rng);
                        made_score(variant, &h, &board[..len])
                    })
                    .collect();
                scores.sort_unstable_by(|a, b| b.cmp(a));
                (len, scores)
            })
            .collect()
    };
    let fits = |read: &Read, h: &[Card], rng: &mut StdRng| {
        if read.preflop < 1.0 && preflop_top(variant, h) > read.preflop {
            return false;
        }
        read.streets.iter().all(|&(len, share)| {
            if share >= 1.0 {
                return true;
            }
            let scores = &cutoffs
                .iter()
                .find(|c| c.0 == len)
                .expect("cut-offs per street")
                .1;
            let cut = scores[((share * scores.len() as f64) as usize).min(scores.len() - 1)];
            made_score(variant, h, &board[..len]) >= cut
                || rng.random_range(0.0..1.0) < BLUFF_ALLOWANCE
        })
    };
    let (mut won, mut counted) = (0.0, 0usize);
    for _ in 0..samples {
        let mut used = Deck::empty();
        let mut hands: Vec<Vec<Card>> = Vec::with_capacity(opponents);
        for i in 0..opponents {
            let mut h = deal(&unknown, used, k, rng);
            if let Some(read) = reads.get(i) {
                for _ in 0..MAX_TRIES {
                    if fits(read, &h, rng) {
                        break;
                    }
                    h = deal(&unknown, used, k, rng);
                }
            }
            for &c in &h {
                used |= c;
            }
            hands.push(h);
        }
        let mut full = board.to_vec();
        full.extend(deal(&unknown, used, need, rng));
        let mine = made_score(variant, hole, &full);
        let best = hands
            .iter()
            .map(|h| made_score(variant, h, &full))
            .max()
            .unwrap_or(0);
        counted += 1;
        if mine > best {
            won += 1.0;
        } else if mine == best {
            let ties = 1 + hands
                .iter()
                .filter(|h| made_score(variant, h, &full) == best)
                .count();
            won += 1.0 / ties as f64;
        }
    }
    won / counted.max(1) as f64
}

/// `n` cards from `pool` not in `used`, at random.
fn deal(pool: &[Card], used: Deck, n: usize, rng: &mut StdRng) -> Vec<Card> {
    let mut out: Vec<Card> = Vec::with_capacity(n);
    let mut taken = used;
    while out.len() < n {
        let c = pool[rng.random_range(0..pool.len())];
        if !taken.has_card(&c) {
            taken |= c;
            out.push(c);
        }
    }
    out
}

/// The acting player's equity against what each opponent still in probably
/// holds (see the module docs), from `samples` Monte Carlo deals.
pub fn range_equity(
    obs: &Observation,
    model: &OpponentModel,
    samples: usize,
    rng: &mut StdRng,
) -> f64 {
    let reads = reads(obs, model);
    let hole: Vec<Card> = obs.hole_cards.iter(false).collect();
    equity(
        obs.rules.variant,
        &hole,
        &obs.board,
        &reads,
        reads.len(),
        samples,
        rng,
    )
}
