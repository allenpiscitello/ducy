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
    /// Raises preflop: 0 a limped pot, 1 single-raised, 2 3-bet, …
    pub raises: u32,
    /// The position (0 UTG … 5 BB) that raised last preflop, None in a
    /// limped pot: it tells a small blind open from a big blind raising a
    /// small blind limp.
    pub aggressor: Option<usize>,
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
        let blind = |p: usize| match p {
            4 => 1,
            5 => 2,
            _ => 0,
        };
        let dead = [4, 5]
            .iter()
            .filter(|&&p| p != opener && p != caller)
            .map(|&p| blind(p))
            .sum();
        let name = format!("SRP {} v {}", POSITIONS[opener], POSITIONS[caller]);
        Some(Self::two_player(
            name,
            game.stack as u64,
            to as u64,
            dead,
            [(opener, opener_range), (caller, caller_range)],
            (1, Some(opener)),
            cards,
        ))
    }

    /// The spot at the end of a preflop line of the strategy's tree, given
    /// as one letter per decision from the start (UTG first): `f` fold, `c`
    /// call or check, `r` raise. The line must end with two players to the
    /// flop, not all-in. Each player's range is the product of their
    /// actions' probabilities along the line. None if the line isn't in the
    /// tree, or a player never takes it.
    ///
    /// For example `"ffffcc"` is the small blind limping and the big
    /// blind checking, and `"fffrfrc"` the big blind 3-betting the button,
    /// who calls.
    pub fn from_line(
        strategy: &PreflopStrategy,
        line: &str,
        cards: &'a CardAbstraction,
    ) -> Option<Self> {
        let game = strategy.game();
        let mut node = 0usize;
        let mut weights = vec![vec![1.0f64; NUM_PREFLOP_CLASSES]; SEATS];
        let mut raises = 0;
        let mut first_in = None;
        let mut last_raiser = None;
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
            let probs: Vec<f64> = (0..NUM_PREFLOP_CLASSES)
                .map(|c| strategy.probs(*id, c).get(a).copied().unwrap_or(0.0) as f64)
                .collect();
            for (w, p) in weights[*seat].iter_mut().zip(probs) {
                *w *= p;
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
            .get(raises)
            .copied()
            .unwrap_or("raised");
        let name = format!("{kind} {} v {}", POSITIONS[first], POSITIONS[second]);
        Some(Self::two_player(
            name,
            game.stack as u64,
            put_in[a] as u64,
            dead,
            [(a, weights[a].clone()), (b, weights[b].clone())],
            (raises as u32, last_raiser),
            cards,
        ))
    }

    /// A spot by name: `<opener>-<caller>` for a single-raised pot (`BTN-BB`)
    /// or `line-<letters>` for any line ([`Self::from_line`]). A spot file's
    /// name is `spot-<bb>-<name>.bin` (or `spot-floor-<bb>-<name>.bin`);
    /// [`Self::name_of_file`] takes the name back out.
    pub fn from_name(
        strategy: &PreflopStrategy,
        name: &str,
        cards: &'a CardAbstraction,
    ) -> Option<Self> {
        if let Some(line) = name.strip_prefix("line-") {
            return Self::from_line(strategy, line, cards);
        }
        let (o, c) = name.split_once('-')?;
        let pos = |n: &str| POSITIONS.iter().position(|p| *p == n);
        Self::single_raised(strategy, pos(o)?, pos(c)?, cards)
    }

    /// The spot name in a spot file's path: what follows `spot-<bb>-` or
    /// `spot-floor-<bb>-`, without `.bin`.
    pub fn name_of_file(path: &str) -> Option<String> {
        let stem = std::path::Path::new(path).file_stem()?.to_str()?;
        let rest = stem.strip_prefix("spot-")?;
        let rest = rest.strip_prefix("floor-").unwrap_or(rest);
        let (bb, name) = rest.split_once('-')?;
        bb.parse::<u32>().ok()?;
        Some(name.to_string())
    }

    /// A spot from its parts: stacks and each player's preflop bet `to` in
    /// half big blinds, the dead chips, and the two (position, range) pairs.
    fn two_player(
        name: String,
        stack: u64,
        to: u64,
        dead: u64,
        players: [(usize, Vec<f64>); 2],
        (raises, aggressor): (u32, Option<usize>),
        cards: &'a CardAbstraction,
    ) -> Self {
        let [(pa, ra), (pb, rb)] = players;
        // Player 0 acts last after the flop.
        let (p0, p1, r0, r1) = if postflop_order(pa) > postflop_order(pb) {
            (pa, pb, ra, rb)
        } else {
            (pb, pa, rb, ra)
        };
        // The tree's chips are half big blinds: blinds 1/2.
        let config = HunlConfig {
            stack,
            ..HunlConfig::default()
        };
        let start = Betting::street_start(
            1,
            [config.stack - to, config.stack - to],
            [to, to],
            config.big_blind,
        );
        let tree = BettingTree::build_from(&config, start);
        let mut spot = Self::new(name, config, cards, tree, dead, [r0, r1], [p0, p1]);
        spot.raises = raises;
        spot.aggressor = aggressor;
        spot
    }

    /// Deals every hand class at least `floor` of the time (a probability,
    /// like the range weights), so training also learns to play hands the
    /// preflop ranges rarely or never bring here. The postflop value tables
    /// (`share_table`) need those: the preflop solve tries every hand.
    pub fn with_floor(self, floor: f64) -> Self {
        let ranges = self
            .ranges
            .clone()
            .map(|r| r.into_iter().map(|w| w.max(floor)).collect());
        let (raises, aggressor) = (self.raises, self.aggressor);
        let mut spot = Self::new(
            self.name,
            self.config,
            self.cards,
            self.tree,
            self.dead,
            ranges,
            self.positions,
        );
        spot.raises = raises;
        spot.aggressor = aggressor;
        spot
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
            raises: 1,
            aggressor: None,
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

    /// Each player's preflop bet, in chips (half big blinds).
    pub fn preflop_bet(&self) -> u64 {
        self.tree.nodes[0].betting.contributed[0]
    }

    /// Player 0's expected payoff on this deal when both play `profile`,
    /// exactly over its mixed actions: a walk of the whole betting tree.
    fn value(&self, s: &SpotState, profile: &crate::Profile<u64>) -> f64 {
        match self.turn(s) {
            Turn::Terminal => self.utility(s),
            Turn::Player(_) => {
                let n = self.num_actions(s);
                let probs = profile.probs(&self.info(s), n);
                (0..n)
                    .filter(|&a| probs[a] > 0.0)
                    .map(|a| probs[a] * self.value(&self.apply(s, a), profile))
                    .sum()
            }
            Turn::Chance => panic!("the deal is at the root"),
        }
    }

    /// The postflop value table ([`super::preflop::PostflopTable`]) of this
    /// spot played with `profile`: for each pair of hand classes (in
    /// position, out of position), the share of the flop pot the player in
    /// position ends up with, averaged over `samples` deals of those classes
    /// (random combos and boards). Runs on `threads` threads.
    pub fn share_table(
        &self,
        profile: &crate::Profile<u64>,
        samples: usize,
        seed: u64,
        threads: usize,
    ) -> Vec<f32> {
        use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
        let combos: Vec<Vec<[Card; 2]>> = {
            let mut by: Vec<Vec<[Card; 2]>> = vec![Vec::new(); NUM_PREFLOP_CLASSES];
            for i in 0..NUM_HOLES {
                let (a, b) = hole_cards(i);
                by[preflop_class(a, b)].push([a, b]);
            }
            by
        };
        let to = self.preflop_bet() as f64;
        let pot = 2.0 * to + self.dead as f64;
        let half_dead = self.dead as f64 / 2.0;
        let pairs = NUM_PREFLOP_CLASSES * NUM_PREFLOP_CLASSES;
        let out: Vec<std::sync::atomic::AtomicU32> = (0..pairs)
            .map(|_| std::sync::atomic::AtomicU32::new(0))
            .collect();
        let next = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..threads.max(1) {
                scope.spawn(|| {
                    loop {
                        let k = next.fetch_add(1, Relaxed);
                        if k >= pairs {
                            break;
                        }
                        let (ci, cj) = (k / NUM_PREFLOP_CLASSES, k % NUM_PREFLOP_CLASSES);
                        let mut rng =
                            Rng::new(seed ^ (k as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
                        // A deal of these two classes: the combos, the board and
                        // who wins a showdown (None when the combos collide).
                        let deal = |rng: &mut Rng| {
                            let h0 =
                                combos[ci][(rng.next_u64() % combos[ci].len() as u64) as usize];
                            let h1 =
                                combos[cj][(rng.next_u64() % combos[cj].len() as u64) as usize];
                            let mut used = bit(h0[0]) | bit(h0[1]);
                            if used & (bit(h1[0]) | bit(h1[1])) != 0 {
                                return None;
                            }
                            used |= bit(h1[0]) | bit(h1[1]);
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
                            let showdown =
                                showdown_score(&h0, &board).cmp(&showdown_score(&h1, &board)) as i8;
                            Some((h0, h1, board, showdown))
                        };
                        // Each played deal's share, beside its showdown result
                        // (1 a win, 0.5 a tie, 0 a loss).
                        let mut played: Vec<(f64, f64)> = Vec::with_capacity(samples);
                        let mut tries = 0;
                        while played.len() < samples && tries < samples * 20 {
                            tries += 1;
                            let Some((h0, h1, board, showdown)) = deal(&mut rng) else {
                                continue;
                            };
                            let buckets = self.cards.deal_buckets([&h0, &h1], &board);
                            let s = SpotState {
                                hole: [h0, h1],
                                board,
                                buckets,
                                node: 0,
                                showdown,
                                dealt: true,
                            };
                            // Player 0's real result: the zero-sum payoff plus
                            // their half of the dead money.
                            let net = self.value(&s, profile) + half_dead;
                            played.push(((net + to) / pot, (showdown as f64 + 1.0) / 2.0));
                        }
                        let n = played.len() as f64;
                        let share = if played.is_empty() {
                            0.5
                        } else {
                            // The showdown result as a control variate: how lucky
                            // the sampled boards ran is known from many cheap
                            // deals (no betting walked), and a deal's share
                            // follows its luck closely, so the share is corrected
                            // by that luck. Unbiased; far less noise per pair.
                            let mx = played.iter().map(|p| p.0).sum::<f64>() / n;
                            let my = played.iter().map(|p| p.1).sum::<f64>() / n;
                            let cov = played.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum::<f64>();
                            let var = played.iter().map(|p| (p.1 - my).powi(2)).sum::<f64>();
                            let beta = if var > 0.0 { cov / var } else { 0.0 };
                            let (mut luck, mut m) = (0.0, 0usize);
                            let mut cheap_tries = 0;
                            while m < 64 * samples && cheap_tries < 64 * samples * 20 {
                                cheap_tries += 1;
                                if let Some((_, _, _, showdown)) = deal(&mut rng) {
                                    luck += (showdown as f64 + 1.0) / 2.0;
                                    m += 1;
                                }
                            }
                            if m > 0 {
                                mx - beta * (my - luck / m as f64)
                            } else {
                                mx
                            }
                        };
                        out[k].store((share as f32).to_bits(), Relaxed);
                    }
                });
            }
        });
        out.iter()
            .map(|x| f32::from_bits(x.load(Relaxed)))
            .collect()
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

/// The preflop line of a hand that went to the flop between two players:
/// their positions (0 UTG … 5 BB) and the raises before the flop (0 a
/// limped pot, 1 single-raised, 2 3-bet, …) and the position that raised
/// last. None for a multiway pot or before the flop.
pub fn heads_up_line(
    events: &[ducy_play::Event],
    button: usize,
) -> Option<([usize; 2], u32, Option<usize>)> {
    use ducy_play::Event;
    let mut raises = 0;
    let mut last_raiser = None;
    let mut folded = [false; SEATS];
    for e in events {
        match *e {
            Event::Bet { seat, .. } | Event::Raise { seat, .. } => {
                raises += 1;
                last_raiser = Some(super::bot::position(seat, button));
            }
            Event::Fold { seat } if seat < SEATS => folded[seat] = true,
            Event::Board { .. } => {
                let live: Vec<usize> = (0..SEATS).filter(|&s| !folded[s]).collect();
                return match live[..] {
                    [a, b] => Some((
                        [
                            super::bot::position(a, button),
                            super::bot::position(b, button),
                        ],
                        raises,
                        last_raiser,
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
        let ([a, b], raises, aggressor) = heads_up_line(&obs.history, obs.button)?;
        let depth = crate::holdem::depths::hand_depth_bb(obs);
        let (game, profile) = self.spots.iter().find(|(g, _)| {
            let bb = g.config.stack as f64 / g.config.big_blind as f64;
            g.raises == raises
                && g.aggressor == aggressor
                && g.positions.contains(&a)
                && g.positions.contains(&b)
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
