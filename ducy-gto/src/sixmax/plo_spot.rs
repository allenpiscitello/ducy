//! Six-handed PLO postflop spots (#290), as [`super::spot`] is for hold'em:
//! a pot that two players took to the flop, from the flop on, as a heads-up
//! pot-limit game. Each player's hand is dealt from their range on that
//! preflop line of the six-handed PLO solve (per preflop bucket of the PLO
//! abstraction); the postflop buckets and the pot-limit betting menu are the
//! heads-up PLO blueprint's. Trained with [`crate::Mccfr`].
//!
//! - **Players:** 0 is in position after the flop (acts last), 1 out of
//!   position.
//! - **The pot:** both players' preflop bets, plus the chips of players who
//!   folded (`dead`); zero-sum as in [`super::spot`]: the winner gets half
//!   the dead money from the other's side of the books.
//! - **Dealing from a range:** a random hand is drawn and kept with chance
//!   its bucket's weight over the largest weight, so hands come up as often
//!   as the range plays them, card removal between the two players included.
//! - **Lines** are named by their preflop actions in order from UTG, one
//!   letter each (`f` fold, `c` call or check, `r` raise): `fffrfc` is the
//!   button opening and the big blind calling.

use super::{
    bot::PreflopStrategy,
    preflop::{Act, Node, POSITIONS, SEATS},
};
use crate::{
    Game, Rng, Turn,
    holdem::{
        cards::{Card, NUM_CARDS, bit},
        hunl::{Betting, BettingTree, Buckets, HunlConfig},
    },
    omaha::{abstraction::PloAbstraction, showdown::showdown},
};

/// Postflop order by preflop position (UTG 0 … BB 5): the blinds first.
fn postflop_order(position: usize) -> usize {
    (position + 2) % SEATS
}

/// The heads-up lines of six-handed PLO trained by default: a single-raised
/// pot for every opener and caller (15; at tables of Regulars the caller is
/// often in position, not the big blind), the small blind's limp checked by
/// the big blind, and the big blind 3-betting the button.
pub fn lines() -> Vec<String> {
    let mut out = Vec::new();
    for opener in 0..SEATS - 1 {
        for caller in opener + 1..SEATS {
            out.push(
                (0..SEATS)
                    .map(|s| match s {
                        s if s == opener => 'r',
                        s if s == caller => 'c',
                        _ => 'f',
                    })
                    .collect(),
            );
        }
    }
    out.push("ffffcc".to_string());
    out.push("fffrfrc".to_string());
    out
}

/// A heads-up postflop spot of six-handed PLO.
pub struct PloSpotGame<'a> {
    pub name: String,
    pub line: String,
    pub config: HunlConfig,
    pub cards: &'a PloAbstraction,
    pub tree: BettingTree,
    pub dead: u64,
    /// Per player, the weight of each preflop bucket on this line.
    pub ranges: [Vec<f64>; 2],
    max_weight: [f64; 2],
    /// The preflop positions of players 0 and 1.
    pub positions: [usize; 2],
    /// Raises preflop: 0 limped, 1 single-raised, 2 3-bet, …
    pub raises: u32,
    /// The position that raised last preflop, None if limped.
    pub aggressor: Option<usize>,
}

impl<'a> PloSpotGame<'a> {
    /// The spot after preflop `line` in the six-handed PLO `strategy`. None
    /// if the line leaves the tree, isn't two players to the flop with equal
    /// bets and no all-in, or a range is empty.
    pub fn from_line(
        strategy: &PreflopStrategy,
        line: &str,
        cards: &'a PloAbstraction,
    ) -> Option<Self> {
        let game = strategy.game();
        let n = game.classes;
        if n != cards.bucket_count(0) {
            return None;
        }
        let mut node = 0usize;
        let mut weights = vec![vec![1.0f64; n]; SEATS];
        let (mut raises, mut first_in, mut last_raiser) = (0u32, None, None);
        for ch in line.chars() {
            let Node::Decision { seat, id, actions } = &game.nodes[node] else {
                return None;
            };
            let a = actions.iter().position(|(a, _)| {
                matches!(
                    (ch, a),
                    ('f', Act::Fold) | ('c', Act::Call) | ('r', Act::Raise(_))
                )
            })?;
            if ch == 'r' {
                raises += 1;
                last_raiser = Some(*seat);
            }
            if ch != 'f' && first_in.is_none() {
                first_in = Some(*seat);
            }
            for (b, w) in weights[*seat].iter_mut().enumerate() {
                *w *= strategy.probs(*id, b).get(a).copied().unwrap_or(0.0) as f64;
            }
            node = actions[a].1 as usize;
        }
        let Node::End {
            put_in,
            live,
            all_in,
        } = &game.nodes[node]
        else {
            return None;
        };
        let players: Vec<usize> = (0..SEATS).filter(|&s| live[s]).collect();
        let [a, b] = players[..] else {
            return None;
        };
        if *all_in || put_in[a] != put_in[b] {
            return None;
        }
        if [a, b]
            .iter()
            .any(|&p| weights[p].iter().sum::<f64>() <= 0.0)
        {
            return None;
        }
        let dead = (0..SEATS)
            .filter(|&s| !live[s])
            .map(|s| put_in[s] as u64)
            .sum();
        let first = last_raiser
            .or(first_in)
            .filter(|p| *p == a || *p == b)
            .unwrap_or(a);
        let second = if first == a { b } else { a };
        let kind = ["limped", "SRP", "3BP", "4BP", "5BP"]
            .get(raises as usize)
            .copied()
            .unwrap_or("raised");
        let name = format!("{kind} {} v {}", POSITIONS[first], POSITIONS[second]);
        // Player 0 acts last after the flop.
        let (p0, p1) = if postflop_order(a) > postflop_order(b) {
            (a, b)
        } else {
            (b, a)
        };
        let to = put_in[a] as u64;
        let config = HunlConfig::pot_limit_omaha_lean(game.stack as u64 / 2);
        let start = Betting::street_start(
            1,
            [config.stack - to, config.stack - to],
            [to, to],
            config.big_blind,
        );
        let tree = BettingTree::build_from(&config, start);
        let ranges = [weights[p0].clone(), weights[p1].clone()];
        let max_weight = [0, 1].map(|p| ranges[p].iter().copied().fold(0.0, f64::max));
        Some(Self {
            name,
            line: line.to_string(),
            config,
            cards,
            tree,
            dead,
            ranges,
            max_weight,
            positions: [p0, p1],
            raises,
            aggressor: last_raiser,
        })
    }

    /// A hand for `player` from their range, avoiding `used`.
    fn draw_hand(&self, player: usize, used: u64, rng: &mut Rng) -> [Card; 6] {
        let k = self.cards.config.hole_cards;
        loop {
            let mut taken = used;
            let mut hole = [0 as Card; 6];
            for c in hole.iter_mut().take(k) {
                *c = loop {
                    let x = (rng.next_u64() % NUM_CARDS as u64) as Card;
                    if taken & bit(x) == 0 {
                        taken |= bit(x);
                        break x;
                    }
                };
            }
            let b = self.cards.preflop_bucket(&hole[..k]) as usize;
            if rng.next_f64() * self.max_weight[player] < self.ranges[player][b] {
                return hole;
            }
        }
    }

    pub fn node(&self, s: &PloSpotState) -> &crate::holdem::hunl::Node {
        &self.tree.nodes[s.node as usize]
    }

    pub fn betting(&self, s: &PloSpotState) -> &Betting {
        &self.node(s).betting
    }
}

/// A hand in a PLO spot: the deal, each player's buckets, the betting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PloSpotState {
    pub hole: [[Card; 6]; 2],
    pub board: [Card; 5],
    pub buckets: [[u16; 4]; 2],
    pub node: u32,
    /// Player 0's showdown result: 1 win, -1 loss, 0 split.
    pub showdown: i8,
    dealt: bool,
}

impl Game for PloSpotGame<'_> {
    type State = PloSpotState;
    type Info = u64;

    fn root(&self) -> PloSpotState {
        PloSpotState {
            hole: [[0; 6]; 2],
            board: [0; 5],
            buckets: [[0; 4]; 2],
            node: 0,
            showdown: 0,
            dealt: false,
        }
    }

    fn turn(&self, s: &PloSpotState) -> Turn {
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

    fn utility(&self, s: &PloSpotState) -> f64 {
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

    fn chance_outcomes(&self, _: &PloSpotState) -> Vec<(PloSpotState, f64)> {
        Vec::new()
    }

    fn sample_chance(&self, _: &PloSpotState, rng: &mut Rng) -> PloSpotState {
        let k = self.cards.config.hole_cards;
        let mask = |h: &[Card]| h.iter().fold(0u64, |m, &c| m | bit(c));
        let h1 = self.draw_hand(1, 0, rng);
        let mut used = mask(&h1[..k]);
        let h0 = self.draw_hand(0, used, rng);
        used |= mask(&h0[..k]);
        let board: [Card; 5] = std::array::from_fn(|_| {
            loop {
                let x = (rng.next_u64() % NUM_CARDS as u64) as Card;
                if used & bit(x) == 0 {
                    used |= bit(x);
                    break x;
                }
            }
        });
        let buckets = self.cards.deal_buckets([&h0[..k], &h1[..k]], &board);
        let showdown = showdown(&h0[..k], &h1[..k], &board);
        PloSpotState {
            hole: [h0, h1],
            board,
            buckets,
            node: 0,
            showdown,
            dealt: true,
        }
    }

    fn num_actions(&self, s: &PloSpotState) -> usize {
        self.node(s).actions.len()
    }

    fn apply(&self, s: &PloSpotState, action: usize) -> PloSpotState {
        let mut n = s.clone();
        n.node = self.node(s).children[action];
        n
    }

    fn info(&self, s: &PloSpotState) -> u64 {
        let b = self.betting(s);
        (s.node as u64) << 16 | s.buckets[b.to_act][b.street] as u64
    }
}

/// Plays trained six-handed PLO spots after the flop: a hand that reached
/// the flop on one of the spots' lines (the same two positions, raises and
/// last raiser) follows that spot's tree and plays its strategy. Anything
/// else is None, for the caller's fallback.
pub struct PloSpotBot {
    spots: Vec<(
        std::sync::Arc<PloSpotGame<'static>>,
        std::sync::Arc<crate::Profile<u64>>,
    )>,
    rng: Rng,
    /// Postflop decisions seen, and why those not played were left: not a
    /// heads-up line, no spot for it, off the spot's tree, played.
    pub stats: SpotStats,
}

/// How a [`PloSpotBot`]'s postflop decisions went.
#[derive(Clone, Copy, Debug, Default)]
pub struct SpotStats {
    pub seen: u64,
    pub not_heads_up: u64,
    pub no_spot: u64,
    pub off_tree: u64,
    pub played: u64,
}

impl PloSpotBot {
    pub fn new(
        spots: Vec<(
            std::sync::Arc<PloSpotGame<'static>>,
            std::sync::Arc<crate::Profile<u64>>,
        )>,
        seed: u64,
    ) -> Self {
        Self {
            spots,
            rng: Rng::new(seed),
            stats: SpotStats::default(),
        }
    }

    /// The trained action for this decision, or None.
    pub fn act(&mut self, obs: &ducy_play::Observation) -> Option<ducy_play::Action> {
        use crate::holdem::{
            bot::realize,
            cards::from_ducy,
            follow::{Follower, street_index},
        };
        if obs.street == ducy_play::Street::Preflop || obs.seats.len() != SEATS {
            return None;
        }
        self.stats.seen += 1;
        let Some(([a, b], raises, aggressor)) =
            super::spot::heads_up_line(&obs.history, obs.button)
        else {
            self.stats.not_heads_up += 1;
            return None;
        };
        let depth = crate::holdem::depths::hand_depth_bb(obs);
        let Some((game, profile)) = self.spots.iter().find(|(g, _)| {
            let bb = g.config.stack as f64 / g.config.big_blind as f64;
            g.raises == raises
                && g.aggressor == aggressor
                && g.positions.contains(&a)
                && g.positions.contains(&b)
                && depth >= bb * 0.7
                && depth <= bb * 1.4
        }) else {
            self.stats.no_spot += 1;
            return None;
        };
        let side_of = |seat: usize| -> usize {
            if super::bot::position(seat, obs.button) == game.positions[0] {
                0
            } else {
                1
            }
        };
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
                | ducy_play::Event::Bet { seat, to, .. }
                    if !obs.seats[seat].folded =>
                {
                    pre[side_of(seat)] = to;
                }
                _ => {}
            }
        }
        follow.real.contributed = pre;
        follow.street = 1;
        let mut map_rng =
            Rng::new(u64::from(obs.hole_cards) ^ (obs.button as u64) << 56 ^ flop_at as u64);
        for e in &obs.history[flop_at + 1..] {
            follow.apply(&game.tree, e, side_of, &mut map_rng);
        }
        let Some(node) = follow.node else {
            self.stats.off_tree += 1;
            return None;
        };
        let n = &game.tree.nodes[node as usize];
        let me = side_of(obs.seat);
        if n.actions.is_empty()
            || n.betting.to_act != me
            || n.betting.street != street_index(obs.street)
        {
            self.stats.off_tree += 1;
            return None;
        }
        self.stats.played += 1;
        let hole: Vec<Card> = obs.hole_cards.iter(false).map(from_ducy).collect();
        let board: Vec<Card> = obs.board.iter().copied().map(from_ducy).collect();
        if hole.len() != game.cards.config.hole_cards {
            return None;
        }
        let bucket = game.cards.hand_bucket(&hole, &board);
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
    use crate::{
        Config, Mccfr,
        omaha::abstraction::PloAbstractionConfig,
        sixmax::{preflop::PreflopGame, realize::Realization},
    };

    #[test]
    fn a_plo_spot_deals_from_its_ranges_and_trains() {
        let cards = PloAbstraction::build(
            PloAbstractionConfig {
                preflop: 12,
                flop: 6,
                turn: 6,
                river: 6,
                fit_hands: 400,
                equity_samples: 40,
                ..PloAbstractionConfig::default()
            },
            |_| {},
        );
        let n = cards.bucket_count(0);
        let game = PreflopGame::pot_limit(100, Realization::uniform(n));
        // Everyone mixes evenly, except the big blind never calls with
        // bucket 0.
        let mut n_of = vec![0; game.decisions as usize];
        let mut bb_facing = Vec::new();
        for x in &game.nodes {
            if let Node::Decision { id, actions, seat } = x {
                n_of[*id as usize] = actions.len();
                if *seat == 5 {
                    bb_facing.push(*id);
                }
            }
        }
        let strategy = PreflopStrategy::from_fn(&game, |id, b| {
            let k = n_of[id as usize];
            if bb_facing.contains(&id) && b == 0 {
                let Node::Decision { actions, .. } = game
                    .nodes
                    .iter()
                    .find(|x| matches!(x, Node::Decision { id: i, .. } if *i == id))
                    .expect("decision")
                else {
                    unreachable!()
                };
                actions
                    .iter()
                    .map(|(a, _)| if *a == Act::Fold { 1.0 } else { 0.0 })
                    .collect()
            } else {
                vec![1.0 / k as f32; k]
            }
        });
        let spot = PloSpotGame::from_line(&strategy, "fffrfc", &cards).expect("BTN v BB");
        assert_eq!(spot.positions, [3, 5], "the button is in position");
        assert_eq!(spot.dead, 1, "the small blind's chip");
        assert_eq!(spot.name, "SRP BTN v BB");
        let mut rng = Rng::new(5);
        let root = spot.root();
        for _ in 0..300 {
            let s = spot.sample_chance(&root, &mut rng);
            assert_eq!(spot.turn(&s), Turn::Player(1), "out of position first");
            let mut seen = 0u64;
            for c in s.hole.iter().flat_map(|h| &h[..4]).chain(&s.board) {
                assert_eq!(seen & bit(*c), 0, "no card twice");
                seen |= bit(*c);
            }
            assert_ne!(
                cards.preflop_bucket(&s.hole[1][..4]),
                0,
                "out of the big blind's range"
            );
        }
        let mut solver = Mccfr::new(&spot, Config::default());
        solver.run(3000);
        assert!(solver.num_infosets() > 0);
    }
}
