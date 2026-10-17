//! Six-handed postflop spots as heads-up games (#267): a pot that two
//! players took to the flop, from the flop on, with each player's hole
//! cards dealt from their range on that preflop line in the six-handed
//! solve. Trained with [`crate::Mccfr`] on Hold'em's card abstraction and
//! betting menu, as the heads-up blueprint is.
//!
//! - **Players:** 0 is in position after the flop (acts last), 1 out of
//!   position, as in the heads-up game's postflop streets.
//! - **The pot:** both players' preflop bets, plus the blinds of players who
//!   folded (`dead`). The game stays zero-sum: whoever wins the pot gets
//!   half the dead money from the other's side of the books, so each
//!   player's payoff is their winnings less half the dead money.
//! - **Ranges** come from the preflop strategy, card removal between the
//!   two players included (hands are dealt without overlap) but not with
//!   the players who folded.

use super::{
    bot::PreflopStrategy,
    chart::spots,
    preflop::{Act, Node, POSITIONS, SEATS},
};
use crate::{
    Game, Rng, Turn,
    holdem::{
        abstraction::CardAbstraction,
        cards::{Card, NUM_CARDS, NUM_HOLES, bit, hole_cards, showdown_score},
        hunl::{Betting, BettingTree, Buckets, HunlConfig},
        iso::{NUM_PREFLOP_CLASSES, preflop_class},
    },
};

/// Postflop order: who acts first after the flop, by preflop position
/// (UTG 0 … BB 5): the blinds, then UTG round to the button.
fn postflop_order(position: usize) -> usize {
    (position + 2) % SEATS
}

/// A heads-up postflop spot of the six-handed game.
pub struct SpotGame<'a> {
    pub name: String,
    pub config: HunlConfig,
    pub cards: &'a CardAbstraction,
    pub tree: BettingTree,
    /// Chips of players who folded preflop, in the pot.
    pub dead: u64,
    /// Each player's range: per hand class, how likely they hold it on this
    /// line (not normalized).
    pub ranges: [Vec<f64>; 2],
    /// Per player, the running total of hand weights over the 1,326 hole
    /// card pairs, for drawing one.
    cumulative: [Vec<f64>; 2],
    /// The preflop positions of player 0 and 1.
    pub positions: [usize; 2],
}

impl<'a> SpotGame<'a> {
    /// A single-raised pot: `opener` (a position, 0 UTG … 4 SB) raised first
    /// in, `caller` called, the rest folded. None if the strategy has no
    /// such line.
    pub fn single_raised(
        strategy: &PreflopStrategy,
        opener: usize,
        caller: usize,
        cards: &'a CardAbstraction,
    ) -> Option<Self> {
        let game = strategy.game();
        let all = spots(game);
        let open = all
            .iter()
            .find(|s| s.id == format!("open-{}", POSITIONS[opener]))?;
        let facing = all
            .iter()
            .find(|s| s.id == format!("vs-open-{}-{}", POSITIONS[caller], POSITIONS[opener]))?;
        let Node::Decision {
            id: open_id,
            actions: open_actions,
            ..
        } = &game.nodes[open.node]
        else {
            return None;
        };
        let Node::Decision {
            id: face_id,
            actions: face_actions,
            ..
        } = &game.nodes[facing.node]
        else {
            return None;
        };
        let raise = open_actions
            .iter()
            .position(|(a, _)| matches!(a, Act::Raise(_)))?;
        let Act::Raise(to) = open_actions[raise].0 else {
            return None;
        };
        let call = face_actions.iter().position(|(a, _)| *a == Act::Call)?;
        let weights = |id: u32, a: usize| -> Option<Vec<f64>> {
            let w: Vec<f64> = (0..NUM_PREFLOP_CLASSES)
                .map(|c| strategy.probs(id, c).get(a).copied().unwrap_or(0.0) as f64)
                .collect();
            (w.iter().sum::<f64>() > 0.0).then_some(w)
        };
        let opener_range = weights(*open_id, raise)?;
        let caller_range = weights(*face_id, call)?;
        // Player 0 acts last after the flop.
        let (p0, p1, r0, r1) = if postflop_order(opener) > postflop_order(caller) {
            (opener, caller, opener_range, caller_range)
        } else {
            (caller, opener, caller_range, opener_range)
        };
        // The tree's chips are half big blinds: blinds 1/2.
        let to = to as u64;
        let config = HunlConfig {
            stack: game.stack as u64,
            ..HunlConfig::default()
        };
        let blind = |p: usize| match p {
            4 => config.small_blind,
            5 => config.big_blind,
            _ => 0,
        };
        let dead = [4, 5]
            .iter()
            .filter(|&&p| p != opener && p != caller)
            .map(|&p| blind(p))
            .sum();
        let start = Betting::street_start(
            1,
            [config.stack - to, config.stack - to],
            [to, to],
            config.big_blind,
        );
        let tree = BettingTree::build_from(&config, start);
        let name = format!("SRP {} v {}", POSITIONS[opener], POSITIONS[caller]);
        Some(Self::new(
            name,
            config,
            cards,
            tree,
            dead,
            [r0, r1],
            [p0, p1],
        ))
    }

    fn new(
        name: String,
        config: HunlConfig,
        cards: &'a CardAbstraction,
        tree: BettingTree,
        dead: u64,
        ranges: [Vec<f64>; 2],
        positions: [usize; 2],
    ) -> Self {
        let cumulative = std::array::from_fn(|p| {
            let mut total = 0.0;
            (0..NUM_HOLES)
                .map(|i| {
                    let (a, b) = hole_cards(i);
                    total += ranges[p][preflop_class(a, b)];
                    total
                })
                .collect()
        });
        Self {
            name,
            config,
            cards,
            tree,
            dead,
            ranges,
            cumulative,
            positions,
        }
    }

    /// A hole-card pair for `player` from their range, avoiding `used`.
    fn draw_hole(&self, player: usize, used: u64, rng: &mut Rng) -> [Card; 2] {
        let cum = &self.cumulative[player];
        let total = *cum.last().expect("holes");
        loop {
            let x = rng.next_f64() * total;
            let i = cum.partition_point(|&c| c <= x).min(NUM_HOLES - 1);
            let (a, b) = hole_cards(i);
            if used & (bit(a) | bit(b)) == 0 {
                return [a, b];
            }
        }
    }

    pub fn node(&self, s: &SpotState) -> &crate::holdem::hunl::Node {
        &self.tree.nodes[s.node as usize]
    }

    /// The betting at `s`.
    pub fn betting(&self, s: &SpotState) -> &Betting {
        &self.node(s).betting
    }
}

/// A hand in a spot: the deal, each player's buckets, and the betting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpotState {
    pub hole: [[Card; 2]; 2],
    pub board: [Card; 5],
    pub buckets: [[u16; 4]; 2],
    pub node: u32,
    /// Player 0's showdown result: 1 win, -1 loss, 0 split.
    pub showdown: i8,
    dealt: bool,
}

impl Game for SpotGame<'_> {
    type State = SpotState;
    /// `node << 16 | bucket`, as in the heads-up game.
    type Info = u64;

    fn root(&self) -> SpotState {
        SpotState {
            hole: [[0, 1], [2, 3]],
            board: [4, 5, 6, 7, 8],
            buckets: [[0; 4]; 2],
            node: 0,
            showdown: 0,
            dealt: false,
        }
    }

    fn turn(&self, s: &SpotState) -> Turn {
        if !s.dealt {
            return Turn::Chance;
        }
        let b = self.betting(s);
        if b.is_over() {
            Turn::Terminal
        } else {
            Turn::Player(b.to_act)
        }
    }

    fn utility(&self, s: &SpotState) -> f64 {
        let b = self.betting(s);
        let half_dead = self.dead as f64 / 2.0;
        if let Some(p) = b.folded {
            return if p == 0 {
                -(b.contributed[0] as f64) - half_dead
            } else {
                b.contributed[1] as f64 + half_dead
            };
        }
        s.showdown as f64 * (b.contributed[0].min(b.contributed[1]) as f64 + half_dead)
    }

    fn chance_outcomes(&self, _: &SpotState) -> Vec<(SpotState, f64)> {
        Vec::new()
    }

    /// The out-of-position hand, then the other, then the board.
    fn sample_chance(&self, _: &SpotState, rng: &mut Rng) -> SpotState {
        let h1 = self.draw_hole(1, 0, rng);
        let mut used = bit(h1[0]) | bit(h1[1]);
        let h0 = self.draw_hole(0, used, rng);
        used |= bit(h0[0]) | bit(h0[1]);
        let mut board = [0; 5];
        for c in board.iter_mut() {
            *c = loop {
                let x = (rng.next_u64() % NUM_CARDS as u64) as Card;
                if used & bit(x) == 0 {
                    used |= bit(x);
                    break x;
                }
            };
        }
        let buckets = self.cards.deal_buckets([&h0, &h1], &board);
        let showdown = showdown_score(&h0, &board).cmp(&showdown_score(&h1, &board)) as i8;
        SpotState {
            hole: [h0, h1],
            board,
            buckets,
            node: 0,
            showdown,
            dealt: true,
        }
    }

    fn num_actions(&self, s: &SpotState) -> usize {
        self.node(s).actions.len()
    }

    fn apply(&self, s: &SpotState, action: usize) -> SpotState {
        let mut n = s.clone();
        n.node = self.node(s).children[action];
        n
    }

    fn info(&self, s: &SpotState) -> u64 {
        let b = self.betting(s);
        (s.node as u64) << 16 | s.buckets[b.to_act][b.street] as u64
    }
}

/// The preflop line of a hand that went to the flop as a single-raised pot
/// between two players, as (opener, caller) positions (0 UTG … 5 BB): one
/// raise, no limps before it, everyone else folded. None for any other line.
pub fn single_raised_line(events: &[ducy_play::Event], button: usize) -> Option<(usize, usize)> {
    use ducy_play::Event;
    let mut raiser = None;
    let mut callers = Vec::new();
    for e in events {
        match *e {
            Event::Call { seat, .. } => {
                raiser?;
                callers.push(seat);
            }
            Event::Bet { seat, .. } | Event::Raise { seat, .. } => {
                if raiser.is_some() {
                    return None;
                }
                raiser = Some(seat);
            }
            Event::Board { .. } => {
                let r = raiser?;
                return match callers.as_slice() {
                    [c] => Some((
                        super::bot::position(r, button),
                        super::bot::position(*c, button),
                    )),
                    _ => None,
                };
            }
            _ => {}
        }
    }
    None
}

/// Plays the trained six-handed spots after the flop: a hand that reached
/// the flop as one of them, at its depth, follows the spot's tree
/// ([`crate::holdem::follow::Follower`], off-menu sizes mapped to the
/// nearest) and plays the trained strategy, turned into real chips the way
/// the heads-up bot does. Anything else (another line, a multiway pot, a
/// hand off the tree) is None, for the caller's fallback.
pub struct SpotBot {
    spots: Vec<(
        std::sync::Arc<SpotGame<'static>>,
        std::sync::Arc<crate::Profile<u64>>,
    )>,
    rng: Rng,
}

impl SpotBot {
    /// `spots`: each trained spot with its average strategy.
    pub fn new(
        spots: Vec<(
            std::sync::Arc<SpotGame<'static>>,
            std::sync::Arc<crate::Profile<u64>>,
        )>,
        seed: u64,
    ) -> Self {
        Self {
            spots,
            rng: Rng::new(seed),
        }
    }

    /// The trained action for this decision, or None.
    pub fn act(&mut self, obs: &ducy_play::Observation) -> Option<ducy_play::Action> {
        use crate::holdem::{bot::realize, cards::from_ducy, follow::Follower};
        if obs.street == ducy_play::Street::Preflop || obs.seats.len() != SEATS {
            return None;
        }
        let (opener, caller) = single_raised_line(&obs.history, obs.button)?;
        let depth = crate::holdem::depths::hand_depth_bb(obs);
        let (game, profile) = self.spots.iter().find(|(g, _)| {
            let bb = g.config.stack as f64 / g.config.big_blind as f64;
            g.positions.contains(&opener)
                && g.positions.contains(&caller)
                && depth >= bb * 0.7
                && depth <= bb * 1.4
        })?;
        let side_of = |seat: usize| -> usize {
            if super::bot::position(seat, obs.button) == game.positions[0] {
                0
            } else {
                1
            }
        };
        // Follow the hand from the flop: the two players' preflop chips as
        // the starting pot (the tree's pot leaves the dead blinds out, too).
        let flop_at = obs
            .history
            .iter()
            .position(|e| matches!(e, ducy_play::Event::Board { .. }))?;
        let mut follow = Follower::new();
        let mut pre = [0u64; 2];
        for e in &obs.history[..flop_at] {
            match *e {
                ducy_play::Event::SmallBlind { seat, amount }
                | ducy_play::Event::BigBlind { seat, amount }
                | ducy_play::Event::Call { seat, amount, .. } => {
                    if !obs.seats[seat].folded {
                        pre[side_of(seat)] += amount;
                    }
                }
                ducy_play::Event::Raise { seat, to, .. }
                | ducy_play::Event::Bet { seat, to, .. } => {
                    // A raise's total for the street replaces what was in.
                    pre[side_of(seat)] = to;
                }
                _ => {}
            }
        }
        follow.real.contributed = pre;
        follow.street = 1;
        // The same mapping draws for every decision of the hand.
        let mut map_rng =
            Rng::new(u64::from(obs.hole_cards) ^ (obs.button as u64) << 56 ^ flop_at as u64);
        for e in &obs.history[flop_at + 1..] {
            follow.apply(&game.tree, e, side_of, &mut map_rng);
        }
        let node = follow.node?;
        let n = &game.tree.nodes[node as usize];
        let me = side_of(obs.seat);
        let street = crate::holdem::follow::street_index(obs.street);
        if n.actions.is_empty() || n.betting.to_act != me || n.betting.street != street {
            return None;
        }
        let hole: Vec<Card> = obs.hole_cards.iter(false).map(from_ducy).collect();
        let board: Vec<Card> = obs.board.iter().copied().map(from_ducy).collect();
        let bucket = game.cards.bucket([hole[0], hole[1]], &board);
        let probs = profile.probs(&((node as u64) << 16 | bucket as u64), n.actions.len());
        let mut x = self.rng.next_f64();
        let mut pick = n.actions.len() - 1;
        for (i, p) in probs.iter().enumerate() {
            if x < *p {
                pick = i;
                break;
            }
            x -= p;
        }
        Some(realize(&game.tree, node, n.actions[pick], obs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, Mccfr, sixmax::preflop::PreflopGame};

    #[test]
    fn a_single_raised_pot_from_the_flop() {
        let game = PreflopGame::new(20);
        let mut n_of = vec![0; game.decisions as usize];
        for x in &game.nodes {
            if let Node::Decision { id, actions, .. } = x {
                n_of[*id as usize] = actions.len();
            }
        }
        // Every action equally likely, except the big blind never calls with 72o.
        let seventy_two = (0..NUM_PREFLOP_CLASSES)
            .find(|&c| crate::holdem::chart::class_name(c) == "72o")
            .expect("72o");
        let facing = spots(&game)
            .into_iter()
            .find(|s| s.id == "vs-open-BB-BTN")
            .expect("BB vs BTN");
        let Node::Decision {
            id: facing_id,
            actions: facing_actions,
            ..
        } = &game.nodes[facing.node]
        else {
            panic!("decision");
        };
        let call = facing_actions
            .iter()
            .position(|(a, _)| *a == Act::Call)
            .expect("call");
        let strategy = PreflopStrategy::from_fn(&game, |id, class| {
            let n = n_of[id as usize];
            if id == *facing_id && class == seventy_two {
                (0..n)
                    .map(|a| if a == call { 0.0 } else { 1.0 / (n - 1) as f32 })
                    .collect()
            } else {
                vec![1.0 / n as f32; n]
            }
        });
        let cards = CardAbstraction::quick(4);
        let spot = SpotGame::single_raised(&strategy, 3, 5, &cards).expect("BTN v BB");
        // The button is in position; the small blind's chip is dead.
        assert_eq!(spot.positions, [3, 5]);
        assert_eq!(spot.dead, 1);
        assert_eq!(spot.name, "SRP BTN v BB");
        let mut rng = Rng::new(3);
        let root = spot.root();
        assert_eq!(spot.turn(&root), Turn::Chance);
        for _ in 0..2000 {
            let s = spot.sample_chance(&root, &mut rng);
            assert_eq!(spot.turn(&s), Turn::Player(1), "out of position acts first");
            let mut seen = 0u64;
            for c in s.hole.iter().flatten().chain(&s.board) {
                assert_eq!(seen & bit(*c), 0, "no card twice");
                seen |= bit(*c);
            }
            assert_ne!(
                preflop_class(s.hole[1][0], s.hole[1][1]),
                seventy_two,
                "out of the big blind's range"
            );
        }
        // Zero-sum with the dead money: the big blind folding to a bet loses
        // its preflop bet and half the dead chip.
        let s = spot.sample_chance(&root, &mut rng);
        let check = spot.apply(&s, 0);
        let after_bet = spot.apply(
            &check,
            spot.node(&check)
                .actions
                .iter()
                .position(|a| matches!(a, crate::holdem::hunl::HunlAction::Bet(_)))
                .expect("bet"),
        );
        let fold = spot
            .node(&after_bet)
            .actions
            .iter()
            .position(|a| *a == crate::holdem::hunl::HunlAction::Fold)
            .expect("fold");
        let end = spot.apply(&after_bet, fold);
        assert_eq!(spot.turn(&end), Turn::Terminal);
        let preflop_bet = spot.tree.nodes[0].betting.contributed[1] as f64;
        assert_eq!(spot.utility(&end), preflop_bet + 0.5);
        // It trains.
        let mut solver = Mccfr::new(&spot, Config::default());
        solver.run(2000);
        assert!(solver.num_infosets() > 0);
    }
}
