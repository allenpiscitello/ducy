//! Decision features for neural-network bots (#214).
//!
//! [`decision_features`] turns any bot decision in Hold'em or Omaha (four to
//! six cards) into a fixed-length vector: the network's input. Every value
//! is scaled to roughly 0 to 1, and each index is documented on its constant
//! below.
//!
//! - **Cards** ([`CARDS`]): equity against random hands and against what the
//!   opponents probably hold, the starting hand's rank, made strength and
//!   closeness to the nuts (exact, against every two-card holding), the made
//!   hand's category, outs, flush draws and the board's texture.
//! - **Spot** ([`SPOT`]): street, position, players, pot odds, stack-to-pot
//!   ratio, bet sizes as fractions of the pot, and the betting so far.
//! - **Opponents** ([`OPPONENTS`]): the [`OpponentModel`] stats of the
//!   players still in.
//! - **Style** ([`STYLE`]): the [`Style`] numbers, so one network can play
//!   every personality.
//!
//! The card features use each game's own rules ([`made_score`]): any five of
//! seven cards in Hold'em, exactly two hole cards and three board cards in
//! Omaha. Strength and nut closeness compare the hand with every two-card
//! holding an opponent could hold (as [`crate::ranges::has_nuts`] does), which covers every
//! hand an Omaha opponent can make too.
//!
//! The equity features cost most: they're Monte Carlo estimates with
//! [`FeatureConfig::samples`] deals each, as the personality bots' own.

use ducy::deck::{Card, Deck};
use rand::rngs::StdRng;

use crate::{
    bot::Observation,
    hand::{Event, Street},
    personality::Style,
    ranges::{made_score, preflop_top, range_equity},
    rules::{BettingStructure, Variant},
    stats::OpponentModel,
    strength::observation_equity,
};

/// How the features are computed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeatureConfig {
    /// Monte Carlo deals per equity estimate (two estimates per decision).
    pub samples: usize,
}

impl Default for FeatureConfig {
    /// 50 deals: a third of [`Style::default`]'s, so both estimates together
    /// cost about a third of a personality bot's decision. The exact
    /// strength features carry much of the same information without noise.
    fn default() -> Self {
        Self { samples: 50 }
    }
}

/// Card features: indices `CARDS.start..CARDS.end`.
pub const CARDS: std::ops::Range<usize> = 0..21;
/// Equity against random hands for every opponent still in.
pub const EQUITY: usize = 0;
/// Equity against the opponents' likely ranges after the flop (the same as
/// [`EQUITY`] preflop).
pub const RANGE_EQUITY: usize = 1;
/// The starting hand's rank: 1 for the best hand, 0 for the worst.
pub const PREFLOP: usize = 2;
/// Made strength after the flop: the share of two-card holdings the hand
/// beats on this board, ties half (0 preflop).
pub const STRENGTH: usize = 3;
/// Closeness to the nuts: 1 for the nuts, falling to 0 once 3% of two-card
/// holdings beat it.
pub const NUTNESS: usize = 4;
/// 1 when no two-card holding beats it.
pub const IS_NUTS: usize = 5;
/// The made hand's category from 0 (high card) to 1 (straight flush), in
/// eighths: pair, two pair, trips, straight, flush, full house, quads.
pub const CATEGORY: usize = 6;
/// Share of the unseen cards that improve the category on the next street.
pub const IMPROVE_OUTS: usize = 7;
/// Share of the unseen cards that make a straight or better from below one.
pub const STRONG_OUTS: usize = 8;
/// A flush draw: one more card of a suit makes a flush.
pub const FLUSH_DRAW: usize = 9;
/// A flush draw to the best flush (the hand holds the suit's highest card
/// not on the board).
pub const NUT_FLUSH_DRAW: usize = 10;
/// The board is paired.
pub const BOARD_PAIRED: usize = 11;
/// Two board cards share a suit (a flush draw is possible), and no more.
pub const BOARD_TWO_TONE: usize = 12;
/// Three or more board cards share a suit (a flush is possible).
pub const BOARD_FLUSH: usize = 13;
/// Four or more share a suit.
pub const BOARD_FOUR_FLUSH: usize = 14;
/// Some five-rank window holds three board ranks (a straight is possible).
pub const BOARD_STRAIGHT: usize = 15;
/// Some window holds four.
pub const BOARD_FOUR_STRAIGHT: usize = 16;
/// The board's highest rank, 0 for a deuce to 1 for an ace (0 preflop).
pub const BOARD_HIGH: usize = 17;
/// The hole cards hold a pair.
pub const HOLE_PAIR: usize = 18;
/// Suits held twice or more in the hole, over two.
pub const HOLE_SUITED: usize = 19;
/// The hole cards' highest rank, 0 to 1.
pub const HOLE_HIGH: usize = 20;

/// Spot features: indices `SPOT.start..SPOT.end`.
pub const SPOT: std::ops::Range<usize> = 21..42;
/// The street, one-hot: preflop, flop, turn, river.
pub const STREET: usize = 21;
/// Position after the flop among the players still in: 0 acts first, 1
/// last.
pub const POSITION: usize = 25;
/// Players still in, over the table's seats.
pub const PLAYERS_IN: usize = 26;
/// Players still in, beyond two, over seven (0 heads-up).
pub const MULTIWAY: usize = 27;
/// Players still in who act after this one (in post-flop order) and aren't
/// all-in, over the others still in.
pub const BEHIND: usize = 28;
/// Pot odds: the bet to call over the pot after calling.
pub const POT_ODDS: usize = 29;
/// The bet to call over the pot, capped at 2, over 2.
pub const TO_CALL: usize = 30;
/// Stack-to-pot ratio (the smaller of its stack and the biggest opponent
/// stack, over the pot), as `spr / (1 + spr)`.
pub const SPR: usize = 31;
/// The effective stack in big blinds, as `bb / (bb + 100)`.
pub const DEPTH: usize = 32;
/// The pot in big blinds, as `bb / (bb + 20)`.
pub const POT: usize = 33;
/// Its own bet this street over the pot, capped at 1.
pub const MY_BET: usize = 34;
/// Bets and raises this street, over 4, capped at 1.
pub const STREET_RAISES: usize = 35;
/// Bets and raises in the hand, over 8, capped at 1.
pub const HAND_RAISES: usize = 36;
/// It made the last bet or raise of an earlier street.
pub const AGGRESSOR: usize = 37;
/// It faces a bet or raise.
pub const FACING_BET: usize = 38;
/// Pot-limit rather than no-limit.
pub const POT_LIMIT: usize = 39;
/// Omaha rather than Hold'em.
pub const OMAHA: usize = 40;
/// Hole cards, from 0 for two to 1 for six.
pub const HOLE_CARDS: usize = 41;

/// Opponent features: indices `OPPONENTS.start..OPPONENTS.end`, averaged over
/// the players still in.
pub const OPPONENTS: std::ops::Range<usize> = 42..47;
/// Share of hands played voluntarily (VPIP).
pub const OPP_VPIP: usize = 42;
/// Share of hands raised preflop (PFR).
pub const OPP_PFR: usize = 43;
/// Bets and raises per call after the flop, as `af / (1 + af)`.
pub const OPP_AGGRESSION: usize = 44;
/// How often they fold to a bet after the flop.
pub const OPP_FOLD_TO_BET: usize = 45;
/// How much the stats can be trusted: hands seen, as `n / (n + 30)`.
pub const OPP_HANDS: usize = 46;

/// Style features: indices `STYLE.start..STYLE.end`, in the order of
/// [`style_features`].
pub const STYLE: std::ops::Range<usize> = 47..75;

/// Features per decision.
pub const NUM_FEATURES: usize = STYLE.end;

/// The [`Style`] numbers, each scaled to about 0 to 1: `vpip`, `pfr`,
/// `three_bet`, `four_bet`, `defend`, `position_bonus`, `any_suited`,
/// `any_ace`, whether `always_play` names hands, `push_fold_bb` (as
/// `x / (x + 20)`, 1 for always), `open_size / 5`, `value_margin`,
/// `aggression`, `trap`, `check_raise`, `backwards`, `bluff`, `bluff_raise`,
/// `call_factor / 2`, `pair_call_factor / 2`, `caution`, `bet_size / 2`,
/// `tilt`, `heater`, `recovery`, `omaha`, `exploit` and `ranges`.
pub fn style_features(s: &Style) -> [f32; STYLE.end - STYLE.start] {
    let flag = |b: bool| if b { 1.0 } else { 0.0 };
    let push = if s.push_fold_bb.is_infinite() {
        1.0
    } else {
        s.push_fold_bb / (s.push_fold_bb + 20.0)
    };
    [
        s.vpip,
        s.pfr,
        s.three_bet,
        s.four_bet,
        s.defend,
        s.position_bonus,
        flag(s.any_suited),
        flag(s.any_ace),
        flag(!s.always_play.is_empty()),
        push,
        s.open_size / 5.0,
        s.value_margin,
        s.aggression,
        s.trap,
        s.check_raise,
        flag(s.backwards),
        s.bluff,
        s.bluff_raise,
        s.call_factor / 2.0,
        s.pair_call_factor / 2.0,
        s.caution,
        s.bet_size / 2.0,
        s.tilt,
        s.heater,
        s.recovery,
        s.omaha,
        flag(s.exploit),
        flag(s.ranges),
    ]
    .map(|x| x as f32)
}

/// The features of the decision in `obs` for a player of style `style`,
/// with `model` holding what's known of the other seats.
pub fn decision_features(
    obs: &Observation,
    model: &OpponentModel,
    style: &Style,
    config: &FeatureConfig,
    rng: &mut StdRng,
) -> [f32; NUM_FEATURES] {
    let mut f = [0f32; NUM_FEATURES];
    card_features(obs, model, config, rng, &mut f);
    spot_features(obs, &mut f);
    opponent_features(obs, model, &mut f);
    f[STYLE].copy_from_slice(&style_features(style));
    f
}

/// Lowest scores of each made-hand category from pair to straight flush, as
/// `ducy`'s `StandardHandRanker::score` counts them.
const CATEGORY_BASES: [u32; 8] = {
    const FIVE: u32 = 13 * 13 * 13 * 13 * 13;
    const FOUR: u32 = 13 * 13 * 13 * 13;
    const THREE: u32 = 13 * 13 * 13;
    const TWO: u32 = 13 * 13;
    let pair = FIVE;
    let two_pair = pair + FOUR;
    let trips = two_pair + THREE;
    let straight = trips + THREE;
    let flush = straight + 13;
    let full = flush + FIVE;
    let quads = full + TWO;
    let straight_flush = quads + TWO;
    [
        pair,
        two_pair,
        trips,
        straight,
        flush,
        full,
        quads,
        straight_flush,
    ]
};

/// A score's category: 0 high card to 8 straight flush.
fn category(score: u32) -> usize {
    CATEGORY_BASES.iter().filter(|&&b| score >= b).count()
}

const STRAIGHT: usize = 4;

fn rank(c: &Card) -> usize {
    c.rank() as usize
}

fn suit(c: &Card) -> usize {
    c.suit() as usize
}

/// The share of two-card holdings `hole` loses to on `board`, ties half,
/// leaving out holdings that use one of the known cards.
fn beaten_share(variant: Variant, hole: &[Card], board: &[Card], mine: u32) -> f64 {
    let mut known = Deck::empty();
    for &c in hole.iter().chain(board) {
        known |= c;
    }
    let unseen: Vec<Card> = (Deck::all_cards() - known).iter(false).collect();
    let pair_variant = match variant {
        Variant::Holdem => Variant::Holdem,
        Variant::Omaha { .. } => Variant::Omaha { hole_cards: 2 },
    };
    let (mut above, mut ties, mut n) = (0u32, 0u32, 0u32);
    for (i, &a) in unseen.iter().enumerate() {
        for &b in &unseen[i + 1..] {
            let s = made_score(pair_variant, &[a, b], board);
            n += 1;
            if s > mine {
                above += 1;
            } else if s == mine {
                ties += 1;
            }
        }
    }
    (above as f64 + ties as f64 / 2.0) / n.max(1) as f64
}

fn card_features(
    obs: &Observation,
    model: &OpponentModel,
    config: &FeatureConfig,
    rng: &mut StdRng,
    f: &mut [f32; NUM_FEATURES],
) {
    let variant = obs.rules.variant;
    let hole: Vec<Card> = obs.hole_cards.iter(false).collect();
    let board = &obs.board;
    if config.samples > 0 && !hole.is_empty() {
        let eq = observation_equity(obs, config.samples, rng);
        f[EQUITY] = eq as f32;
        f[RANGE_EQUITY] = if board.is_empty() {
            eq as f32
        } else {
            range_equity(obs, model, config.samples, rng) as f32
        };
    }
    if hole.len() == variant.hole_cards() {
        f[PREFLOP] = (1.0 - preflop_top(variant, &hole)) as f32;
    }
    // The hole cards' own shape.
    let mut hole_ranks = [0u8; 13];
    let mut hole_suits = [0u8; 4];
    for c in &hole {
        hole_ranks[rank(c)] += 1;
        hole_suits[suit(c)] += 1;
    }
    f[HOLE_PAIR] = if hole_ranks.iter().any(|&n| n >= 2) {
        1.0
    } else {
        0.0
    };
    f[HOLE_SUITED] = (hole_suits.iter().filter(|&&n| n >= 2).count() as f32 / 2.0).min(1.0);
    f[HOLE_HIGH] = hole.iter().map(rank).max().unwrap_or(0) as f32 / 12.0;
    if board.len() < 3 || hole.len() != variant.hole_cards() {
        return;
    }

    // The board's texture.
    let mut board_ranks = [0u8; 13];
    let mut board_suits = [0u8; 4];
    for c in board {
        board_ranks[rank(c)] += 1;
        board_suits[suit(c)] += 1;
    }
    let max_suit = board_suits.iter().copied().max().unwrap_or(0);
    let rank_bits = |counts: &[u8; 13]| -> u16 {
        counts.iter().enumerate().fold(0, |m, (r, &n)| {
            if n == 0 {
                m
            } else if r == 12 {
                m | 1 << 13 | 1
            } else {
                m | 1 << (r + 1)
            }
        })
    };
    let bits = rank_bits(&board_ranks);
    let window = (0..10)
        .map(|w| (bits & 0b11111 << w).count_ones())
        .max()
        .unwrap_or(0);
    let flag = |b: bool| if b { 1.0 } else { 0.0 };
    f[BOARD_PAIRED] = flag(board_ranks.iter().any(|&n| n >= 2));
    f[BOARD_TWO_TONE] = flag(max_suit == 2);
    f[BOARD_FLUSH] = flag(max_suit >= 3);
    f[BOARD_FOUR_FLUSH] = flag(max_suit >= 4);
    f[BOARD_STRAIGHT] = flag(window >= 3);
    f[BOARD_FOUR_STRAIGHT] = flag(window >= 4);
    f[BOARD_HIGH] = board.iter().map(rank).max().unwrap_or(0) as f32 / 12.0;

    // The made hand, against every two-card holding.
    let mine = made_score(variant, &hole, board);
    let cat = category(mine);
    let beaten = beaten_share(variant, &hole, board, mine);
    f[STRENGTH] = (1.0 - beaten) as f32;
    f[NUTNESS] = (1.0 - beaten / 0.03).max(0.0) as f32;
    f[IS_NUTS] = flag(beaten == 0.0);
    f[CATEGORY] = cat as f32 / 8.0;

    // Draws: what the next card does.
    if board.len() < 5 {
        let mut known = Deck::empty();
        for &c in hole.iter().chain(board.iter()) {
            known |= c;
        }
        let unseen: Vec<Card> = (Deck::all_cards() - known).iter(false).collect();
        let mut next = board.clone();
        next.push(board[0]);
        let (mut improve, mut strong) = (0u32, 0u32);
        for &c in &unseen {
            *next.last_mut().expect("a card") = c;
            let after = category(made_score(variant, &hole, &next));
            improve += u32::from(after > cat);
            strong += u32::from(cat < STRAIGHT && after >= STRAIGHT);
        }
        let n = unseen.len().max(1) as f32;
        f[IMPROVE_OUTS] = improve as f32 / n;
        f[STRONG_OUTS] = strong as f32 / n;
        // Flush draws: one more card of the suit completes the flush.
        for s in 0..4 {
            let (on_board, held) = (board_suits[s], hole_suits[s]);
            let draw = match variant {
                Variant::Holdem => held >= 1 && on_board + held == 4,
                Variant::Omaha { .. } => held >= 2 && on_board == 2,
            };
            if !draw || cat >= 5 {
                continue;
            }
            f[FLUSH_DRAW] = 1.0;
            // The suit's best card not on the board.
            let top = (0..13)
                .rev()
                .find(|&r| !board.iter().any(|c| suit(c) == s && rank(c) == r));
            let mine_top = hole.iter().filter(|c| suit(c) == s).map(rank).max();
            if top.is_some() && mine_top == top {
                f[NUT_FLUSH_DRAW] = 1.0;
            }
        }
    }
}

fn spot_features(obs: &Observation, f: &mut [f32; NUM_FEATURES]) {
    let street = match obs.street {
        Street::Preflop => 0,
        Street::Flop => 1,
        Street::Turn => 2,
        Street::River => 3,
    };
    f[STREET + street] = 1.0;
    let n = obs.seats.len();
    // Post-flop order: from the seat after the button round to the button.
    let order: Vec<usize> = (1..=n).map(|k| (obs.button + k) % n).collect();
    let live: Vec<usize> = order
        .iter()
        .copied()
        .filter(|&s| !obs.seats[s].folded)
        .collect();
    let me = live.iter().position(|&s| s == obs.seat).unwrap_or(0);
    f[POSITION] = if live.len() > 1 {
        me as f32 / (live.len() - 1) as f32
    } else {
        1.0
    };
    f[PLAYERS_IN] = live.len() as f32 / n.max(1) as f32;
    f[MULTIWAY] = (live.len().saturating_sub(2) as f32 / 7.0).min(1.0);
    let behind = live[me + 1..]
        .iter()
        .filter(|&&s| !obs.seats[s].all_in)
        .count();
    f[BEHIND] = if live.len() > 1 {
        behind as f32 / (live.len() - 1) as f32
    } else {
        0.0
    };

    let pot = obs.pot.max(1) as f64;
    let mine = &obs.seats[obs.seat];
    let to_call = obs
        .current_bet
        .saturating_sub(mine.street_bet)
        .min(mine.stack) as f64;
    f[POT_ODDS] = (to_call / (pot + to_call)) as f32;
    f[TO_CALL] = ((to_call / pot).min(2.0) / 2.0) as f32;
    let biggest = obs
        .seats
        .iter()
        .enumerate()
        .filter(|&(s, v)| s != obs.seat && !v.folded)
        .map(|(_, v)| v.stack + v.street_bet)
        .max()
        .unwrap_or(0);
    let effective = (mine.stack + mine.street_bet).min(biggest) as f64;
    let spr = effective / pot;
    f[SPR] = (spr / (1.0 + spr)) as f32;
    let bb = obs.rules.big_blind.max(1) as f64;
    f[DEPTH] = (effective / bb / (effective / bb + 100.0)) as f32;
    f[POT] = (pot / bb / (pot / bb + 20.0)) as f32;
    f[MY_BET] = (mine.street_bet as f64 / pot).min(1.0) as f32;
    f[FACING_BET] = if to_call > 0.0 { 1.0 } else { 0.0 };

    // The betting so far.
    let (mut street_raises, mut hand_raises) = (0u32, 0u32);
    let (mut last, mut earlier) = (None, None);
    for e in &obs.history {
        match e {
            Event::Bet { seat, .. } | Event::Raise { seat, .. } => {
                street_raises += 1;
                hand_raises += 1;
                last = Some(*seat);
            }
            Event::Board { .. } => {
                street_raises = 0;
                earlier = last.or(earlier);
                last = None;
            }
            _ => {}
        }
    }
    f[STREET_RAISES] = (street_raises as f32 / 4.0).min(1.0);
    f[HAND_RAISES] = (hand_raises as f32 / 8.0).min(1.0);
    f[AGGRESSOR] = if earlier == Some(obs.seat) { 1.0 } else { 0.0 };
    f[POT_LIMIT] = if obs.rules.structure == BettingStructure::PotLimit {
        1.0
    } else {
        0.0
    };
    let cards = obs.rules.variant.hole_cards();
    f[OMAHA] = if cards > 2 { 1.0 } else { 0.0 };
    f[HOLE_CARDS] = (cards - 2) as f32 / 4.0;
}

fn opponent_features(obs: &Observation, model: &OpponentModel, f: &mut [f32; NUM_FEATURES]) {
    let others: Vec<_> = obs
        .seats
        .iter()
        .enumerate()
        .filter(|&(s, v)| s != obs.seat && !v.folded)
        .map(|(s, _)| model.seat(s))
        .collect();
    if others.is_empty() {
        return;
    }
    let mean = |g: &dyn Fn(&crate::stats::SeatStats) -> f64| {
        (others.iter().map(g).sum::<f64>() / others.len() as f64) as f32
    };
    f[OPP_VPIP] = mean(&|s| s.vpip());
    f[OPP_PFR] = mean(&|s| s.pfr());
    f[OPP_AGGRESSION] = mean(&|s| {
        let af = s.aggression();
        af / (1.0 + af)
    });
    f[OPP_FOLD_TO_BET] = mean(&|s| s.fold_to_bet());
    f[OPP_HANDS] = mean(&|s| s.hands as f64 / (s.hands as f64 + 30.0));
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn categories_follow_the_ranker() {
        let score = |s: &str| {
            let mut d = Deck::empty();
            for c in s.split(' ') {
                d |= c.parse::<Card>().unwrap();
            }
            ducy::ranking::hand_rank::StandardHandRanker::score(&d)
        };
        assert_eq!(category(score("Ah Kd 9c 7s 2h")), 0);
        assert_eq!(category(score("Ah Ad 9c 7s 2h")), 1);
        assert_eq!(category(score("Ah Ad 9c 9s 2h")), 2);
        assert_eq!(category(score("Ah Ad Ac 9s 2h")), 3);
        assert_eq!(category(score("5h 4d 3c 2s Ah")), 4);
        assert_eq!(category(score("Ah 9h 7h 4h 2h")), 5);
        assert_eq!(category(score("Ah Ad Ac 9s 9h")), 6);
        assert_eq!(category(score("Ah Ad Ac As 9h")), 7);
        assert_eq!(category(score("9h 8h 7h 6h 5h")), 8);
    }
}
