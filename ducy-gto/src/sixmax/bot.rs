//! A six-handed bot that plays the preflop solve (#245, stage 2): before the
//! flop it follows the solved strategy for its position and hand; after the
//! flop, and wherever the preflop leaves the solved tree, it plays as
//! `post`, another bot (Doug Poker's style to start with, so the gain over
//! Doug is the preflop's).
//!
//! The table's real actions are mapped onto the tree by kind, not size:
//! every raise is the tree's raise at that level, every all-in its all-in.
//! Its own raises are the tree's sizes in the table's chips.
//!
//! For pot-limit Omaha (#290) the strategy is a `PreflopGame::pot_limit`
//! solve over a PLO abstraction's preflop buckets, and the bot looks hands up
//! by bucket (`with_plo_buckets`); after the flop, heads-up pots go to a
//! heads-up PLO bot (`with_heads_up`), as they go to GtoBot in hold'em.

use std::sync::Arc;

use ducy_play::{Action, Bot, Event, HandSummary, Observation, Street};

use super::{
    preflop::{Act, Node, PreflopGame, SEATS},
    solver::PreflopSolver,
};
use crate::{
    Rng,
    holdem::{
        cards::from_ducy, depths::hand_depth_bb, iso::NUM_PREFLOP_CLASSES, iso::preflop_class,
    },
    omaha::abstraction::PloAbstraction,
};

const C: usize = NUM_PREFLOP_CLASSES;

/// The solve's average strategy, ready to play: per decision and class, the
/// chance of each action.
pub struct PreflopStrategy {
    game: PreflopGame,
    /// Per decision id: where its probabilities start, and how many actions.
    offsets: Vec<(usize, usize)>,
    probs: Vec<f32>,
}

impl PreflopStrategy {
    /// The average strategy of a solve.
    pub fn from_solver(game: &PreflopGame, solver: &PreflopSolver) -> Self {
        Self::from_fn(game, |id, class| {
            solver
                .strategy(id, class)
                .iter()
                .map(|&p| p as f32)
                .collect()
        })
    }

    /// A strategy from a function giving each decision id and class its
    /// action probabilities, in the order of the node's actions.
    pub fn from_fn(game: &PreflopGame, mut f: impl FnMut(u32, usize) -> Vec<f32>) -> Self {
        let mut offsets = vec![(0, 0); game.decisions as usize];
        let mut probs = Vec::new();
        for node in &game.nodes {
            if let Node::Decision { id, actions, .. } = node {
                offsets[*id as usize] = (probs.len(), actions.len());
                for class in 0..game.classes {
                    let p = f(*id, class);
                    assert_eq!(p.len(), actions.len(), "one probability per action");
                    probs.extend(p);
                }
            }
        }
        Self {
            game: game.clone(),
            offsets,
            probs,
        }
    }

    /// The tree the strategy is for.
    pub fn game(&self) -> &PreflopGame {
        &self.game
    }

    /// The chance of each action at decision `id` with hand class `class`;
    /// empty for a decision a compact strategy left out.
    pub fn probs(&self, id: u32, class: usize) -> &[f32] {
        let (start, n) = self.offsets[id as usize];
        &self.probs[start + class * n..start + (class + 1) * n]
    }

    /// The strategy for the decisions `keep` names, small enough to ship to
    /// a web page: each probability in a byte. The tree is rebuilt from its
    /// depth when it's read (`from_compact`); a decision left out has no
    /// strategy there, and `SixMaxBot` leaves it to its `post` bot.
    pub fn to_compact(&self, keep: impl Fn(u32) -> bool) -> Vec<u8> {
        let ids: Vec<u32> = (0..self.offsets.len() as u32)
            .filter(|&id| keep(id) && self.offsets[id as usize].1 > 0)
            .collect();
        let mut out = COMPACT_MAGIC.to_vec();
        out.extend(COMPACT_VERSION.to_le_bytes());
        out.extend(self.game.stack.to_le_bytes());
        out.extend((ids.len() as u32).to_le_bytes());
        for id in ids {
            out.extend(id.to_le_bytes());
            for class in 0..self.game.classes {
                out.extend(
                    self.probs(id, class)
                        .iter()
                        .map(|&p| (p.clamp(0.0, 1.0) * 255.0).round() as u8),
                );
            }
        }
        out
    }

    /// Reads `to_compact`'s bytes: the tree for their depth, with the
    /// decisions they hold (each class's probabilities summing to 1 again).
    pub fn from_compact(bytes: &[u8]) -> Result<Self, String> {
        let bad = || "not a compact six-handed preflop strategy".to_string();
        let u32_at = |i: usize| -> Result<u32, String> {
            Ok(u32::from_le_bytes(
                bytes.get(i..i + 4).ok_or_else(bad)?.try_into().expect("4"),
            ))
        };
        if bytes.get(..8) != Some(COMPACT_MAGIC) || u32_at(8)? != COMPACT_VERSION {
            return Err(bad());
        }
        let stack = u32_at(12)?;
        if stack == 0 || stack % 2 != 0 || stack > 2 * 1000 {
            return Err(bad());
        }
        let game = PreflopGame::new(stack / 2);
        // Each decision's number of actions.
        let mut actions_of = vec![0; game.decisions as usize];
        for node in &game.nodes {
            if let Node::Decision { id, actions, .. } = node {
                actions_of[*id as usize] = actions.len();
            }
        }
        let mut table: Vec<Option<Vec<f32>>> = vec![None; game.decisions as usize];
        let mut at = 20;
        for _ in 0..u32_at(16)? {
            let id = u32_at(at)? as usize;
            at += 4;
            let n = *actions_of.get(id).ok_or_else(bad)?;
            let raw = bytes.get(at..at + n * C).ok_or_else(bad)?;
            at += n * C;
            let mut probs = Vec::with_capacity(n * C);
            for class in raw.chunks(n) {
                let sum: f32 = class.iter().map(|&b| b as f32).sum();
                // A class rounded to all zeros: fold-or-check stays first.
                probs.extend(class.iter().enumerate().map(|(i, &b)| {
                    if sum > 0.0 {
                        b as f32 / sum
                    } else if i == 0 {
                        1.0
                    } else {
                        0.0
                    }
                }));
            }
            table[id] = Some(probs);
        }
        if at != bytes.len() {
            return Err(bad());
        }
        let mut offsets = vec![(0, 0); game.decisions as usize];
        let mut probs = Vec::new();
        for node in &game.nodes {
            if let Node::Decision { id, actions, .. } = node {
                if let Some(p) = table[*id as usize].take() {
                    offsets[*id as usize] = (probs.len(), actions.len());
                    probs.extend(p);
                } else {
                    offsets[*id as usize] = (probs.len(), 0);
                }
            }
        }
        Ok(Self {
            game,
            offsets,
            probs,
        })
    }
}

const COMPACT_MAGIC: &[u8; 8] = b"DUCY6PS\0";
const COMPACT_VERSION: u32 = 1;

/// Follows a six-handed hand's preflop actions down the tree, calling
/// `visit(node, table seat, action index)` at each decision taken. Actions
/// map by kind, as `SixMaxBot` maps them. Returns the node reached (a
/// decision, or the end of the betting at the flop), or None if the hand
/// left the tree.
pub fn follow(
    game: &PreflopGame,
    events: &[Event],
    button: usize,
    mut visit: impl FnMut(usize, usize, usize),
) -> Option<usize> {
    let mut node = game.root();
    for e in events {
        let (seat, act) = match *e {
            Event::Fold { seat } => (seat, Act::Fold),
            Event::Check { seat } | Event::Call { seat, .. } => (seat, Act::Call),
            Event::Bet { seat, all_in, .. } | Event::Raise { seat, all_in, .. } => {
                (seat, if all_in { Act::AllIn } else { Act::Raise(0) })
            }
            Event::Board { .. } => break,
            _ => continue,
        };
        let Node::Decision {
            seat: pos, actions, ..
        } = &game.nodes[node]
        else {
            return None;
        };
        if *pos != position(seat, button) {
            return None;
        }
        let next = actions.iter().position(|(a, _)| match (a, act) {
            (Act::Raise(_), Act::Raise(_)) => true,
            (a, b) => *a == b,
        });
        // A raise the tree only has as an all-in (short stacks).
        let next = next.or_else(|| {
            matches!(act, Act::Raise(_))
                .then(|| actions.iter().position(|(a, _)| *a == Act::AllIn))
                .flatten()
        })?;
        visit(node, seat, next);
        node = actions[next].1 as usize;
    }
    Some(node)
}

/// The solve's position of a table seat: 0 UTG, 1 HJ, 2 CO, 3 BTN, 4 SB, 5 BB.
pub fn position(seat: usize, button: usize) -> usize {
    (seat + SEATS - button + 3) % SEATS
}

/// The six-handed bot.
pub struct SixMaxBot<B: Bot> {
    /// Each strategy with the hand depths, in big blinds (`hand_depth_bb`),
    /// it's played at: a solve is for one depth. A hand at no strategy's
    /// depths goes to `post`.
    strategies: Vec<(Arc<PreflopStrategy>, f64, f64)>,
    post: B,
    /// Plays heads-up pots after the flop, seeing them as a heads-up hand
    /// (`heads_up_view`): a heads-up GTO bot. Without one, `post` does.
    heads_up: Option<Box<dyn Bot>>,
    /// PLO: hands are looked up by this abstraction's preflop buckets
    /// rather than hold'em's 169 classes.
    plo: Option<Arc<PloAbstraction>>,
    rng: Rng,
    /// Preflop decisions taken from the solve, and those left to `post`.
    pub solved: u64,
    pub off_tree: u64,
    /// Decisions after the flop the heads-up bot made.
    pub heads_up_decisions: u64,
}

impl<B: Bot> SixMaxBot<B> {
    /// Plays `strategy` at every depth.
    pub fn new(strategy: Arc<PreflopStrategy>, post: B, seed: u64) -> Self {
        Self {
            strategies: vec![(strategy, 0.0, f64::INFINITY)],
            post,
            heads_up: None,
            plo: None,
            rng: Rng::new(seed),
            solved: 0,
            off_tree: 0,
            heads_up_decisions: 0,
        }
    }

    /// Plays the first strategy only in hands from `lo` to `hi` big blinds
    /// deep (the effective stack, `hand_depth_bb`).
    pub fn with_depths(mut self, lo: f64, hi: f64) -> Self {
        self.strategies[0].1 = lo;
        self.strategies[0].2 = hi;
        self
    }

    /// Adds `strategy` for hands from `lo` to `hi` big blinds deep (a solve
    /// at another depth). Where depths overlap, the first added plays.
    pub fn with_strategy(mut self, strategy: Arc<PreflopStrategy>, lo: f64, hi: f64) -> Self {
        self.strategies.push((strategy, lo, hi));
        self
    }

    /// Plays heads-up pots after the flop with `bot`.
    pub fn with_heads_up(mut self, bot: Box<dyn Bot>) -> Self {
        self.heads_up = Some(bot);
        self
    }

    /// Pot-limit Omaha: the strategies are `PreflopGame::pot_limit` solves
    /// over `cards`' preflop buckets, and hands are looked up by bucket.
    pub fn with_plo_buckets(mut self, cards: Arc<PloAbstraction>) -> Self {
        self.plo = Some(cards);
        self
    }

    fn preflop(&mut self, obs: &Observation) -> Option<Action> {
        if obs.seats.len() != SEATS || obs.seats.iter().any(|s| s.stack + s.contributed == 0) {
            return None;
        }
        if obs.history.iter().any(|e| matches!(e, Event::Board { .. })) {
            return None;
        }
        let depth = hand_depth_bb(obs);
        let strategy = self
            .strategies
            .iter()
            .find(|(_, lo, hi)| (*lo..=*hi).contains(&depth))?
            .0
            .clone();
        // The tree node the preflop has reached, or None if it left the tree.
        let node = follow(&strategy.game, &obs.history, obs.button, |_, _, _| {})?;
        let Node::Decision { seat, id, actions } = &strategy.game.nodes[node] else {
            return None;
        };
        if *seat != position(obs.seat, obs.button) {
            return None;
        }
        let cards: Vec<_> = obs.hole_cards.iter(false).map(from_ducy).collect();
        let class = match &self.plo {
            Some(plo) if cards.len() == plo.config.hole_cards => {
                plo.preflop_bucket(&cards) as usize
            }
            Some(_) => return None,
            None => preflop_class(cards[0], cards[1]),
        };
        if class >= strategy.game.classes {
            return None;
        }
        let probs = strategy.probs(*id, class);
        // A decision a compact strategy left out.
        if probs.is_empty() {
            return None;
        }
        let mut x = self.rng.next_f64() as f32;
        let mut pick = actions.len() - 1;
        for (i, &p) in probs.iter().enumerate() {
            if x < p {
                pick = i;
                break;
            }
            x -= p;
        }
        let legal = &obs.legal;
        // Tree chips are half big blinds.
        let chips = |half_bb: u32| half_bb as u64 * obs.rules.big_blind / 2;
        Some(match actions[pick].0 {
            Act::Fold if legal.can_check => Action::Check,
            Act::Fold => Action::Fold,
            Act::Call if legal.can_check => Action::Check,
            Act::Call => Action::Call,
            Act::Raise(to) => match legal.raise.or(legal.bet) {
                Some(r) => {
                    let to = chips(to).clamp(r.min_to, r.max_to);
                    if to == r.max_to {
                        Action::AllIn
                    } else {
                        Action::Raise(to)
                    }
                }
                None if legal.can_check => Action::Check,
                None => Action::Call,
            },
            Act::AllIn => Action::AllIn,
        })
    }
}

impl<B: Bot> Bot for SixMaxBot<B> {
    fn act(&mut self, obs: &Observation) -> Option<Action> {
        if obs.street == Street::Preflop {
            if let Some(action) = self.preflop(obs) {
                self.solved += 1;
                return Some(action);
            }
            self.off_tree += 1;
        } else if let Some(bot) = self.heads_up.as_mut() {
            if let Some(view) = heads_up_view(obs) {
                if let Some(action) = bot.act(&view) {
                    self.heads_up_decisions += 1;
                    return Some(action);
                }
            }
        }
        self.post.act(obs)
    }

    fn hand_over(&mut self, summary: &HandSummary) {
        self.post.hand_over(summary);
    }
}

/// A multiway hand down to two players after the flop, seen as a heads-up
/// one, for a heads-up bot: the player in position (last to act after the
/// flop) is the heads-up button, seat 0, and the other the big blind, seat 1.
/// The two players' preflop actions become the nearest heads-up line: the
/// blinds, then their raises and calls in turn, the button first (so when
/// the player out of position opened, the button limps, the big blind
/// raises, the button calls). Amounts are the real ones; the pot is the real
/// pot, folded players' chips included. None when the hand doesn't fit: more
/// than two players left, or preflop actions that don't alternate.
pub fn heads_up_view(obs: &Observation) -> Option<Observation> {
    let live: Vec<usize> = (0..obs.seats.len())
        .filter(|&s| !obs.seats[s].folded)
        .collect();
    let &[a, b] = live.as_slice() else {
        return None;
    };
    let n = obs.seats.len();
    // After the flop, action starts left of the button: the last of the two
    // in that order is in position.
    let order = |s: usize| (s + n - obs.button - 1) % n;
    let (ip, oop) = if order(a) > order(b) { (a, b) } else { (b, a) };
    let hu = |s: usize| if s == ip { 0 } else { 1 };
    let bb = obs.rules.big_blind;
    let sb = obs.rules.small_blind;

    // The two players' preflop actions, in order.
    let mut acts = Vec::new();
    let mut flop_at = obs.history.len();
    for (i, e) in obs.history.iter().enumerate() {
        match *e {
            Event::Board { .. } => {
                flop_at = i;
                break;
            }
            Event::Call { seat, .. } | Event::Check { seat } if seat == a || seat == b => {
                acts.push((hu(seat), None))
            }
            Event::Bet { seat, to, all_in } | Event::Raise { seat, to, all_in }
                if seat == a || seat == b =>
            {
                acts.push((hu(seat), Some((to, all_in))))
            }
            _ => {}
        }
    }
    // Their actions as a heads-up line: one action each in turn.
    let mut line: Vec<(usize, Option<(u64, bool)>)> = Vec::new();
    for act in acts {
        if line.last().is_some_and(|l| l.0 == act.0) {
            // Two in a row from one player (a third player acted between):
            // keep the later.
            line.pop();
        }
        line.push(act);
    }
    if line.first().is_some_and(|l| l.0 == 1) {
        // The big blind (out of position) acted first: the button limped.
        line.insert(0, (0, None));
    }
    if line.windows(2).any(|w| w[0].0 == w[1].0) {
        return None;
    }

    let mut history = vec![
        Event::SmallBlind {
            seat: 0,
            amount: sb,
        },
        Event::BigBlind {
            seat: 1,
            amount: bb,
        },
    ];
    let mut put = [sb, bb];
    let mut bet = bb;
    for (seat, raise) in line {
        match raise {
            Some((to, all_in)) => {
                let to = to.max(bet);
                history.push(Event::Raise { seat, to, all_in });
                put[seat] = to;
                bet = to;
            }
            None if put[seat] == bet => history.push(Event::Check { seat }),
            None => {
                history.push(Event::Call {
                    seat,
                    amount: bet - put[seat],
                    all_in: false,
                });
                put[seat] = bet;
            }
        }
    }
    // The preflop must end with the bets matched.
    if put[0] != put[1] {
        return None;
    }
    // After the flop: the real events, seats mapped.
    for e in &obs.history[flop_at..] {
        let mapped = match *e {
            Event::Board { .. } => e.clone(),
            Event::Fold { seat } => Event::Fold { seat: hu(seat) },
            Event::Check { seat } => Event::Check { seat: hu(seat) },
            Event::Call {
                seat,
                amount,
                all_in,
            } => Event::Call {
                seat: hu(seat),
                amount,
                all_in,
            },
            Event::Bet { seat, to, all_in } => Event::Bet {
                seat: hu(seat),
                to,
                all_in,
            },
            Event::Raise { seat, to, all_in } => Event::Raise {
                seat: hu(seat),
                to,
                all_in,
            },
            _ => continue,
        };
        history.push(mapped);
    }

    let mut view = obs.clone();
    view.seat = hu(obs.seat);
    view.button = 0;
    view.seats = vec![obs.seats[ip], obs.seats[oop]];
    view.history = history;
    Some(view)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ducy_play::{Deal, Hand, TableRules};

    #[test]
    fn a_six_handed_pot_down_to_two_reads_as_heads_up() {
        // Blinds 1/2, button seat 0: seat 3 is UTG. UTG opens to 5, everyone
        // folds to the big blind (seat 2), who calls; the flop comes.
        let rules = TableRules::no_limit_holdem(1, 2);
        let deal = Deal::random(rules.variant, 6, Some(3)).expect("deal");
        let mut hand = Hand::new(rules, &[200; 6], 0, deal).expect("hand");
        hand.act(Action::Raise(5)).expect("UTG opens");
        for _ in 0..4 {
            hand.act(Action::Fold).expect("fold");
        }
        hand.act(Action::Call).expect("big blind calls");
        // The big blind acts first after the flop.
        let obs = hand.observation(2).expect("big blind to act");
        assert_eq!(obs.street, Street::Flop);
        let view = heads_up_view(&obs).expect("two players left");
        // UTG is in position: the heads-up button, seat 0; the big blind
        // seat 1, and it's the big blind to act.
        assert_eq!((view.button, view.seat, view.seats.len()), (0, 1, 2));
        assert_eq!(view.seats[0].stack, obs.seats[3].stack);
        // The line: the heads-up blinds, then UTG (the button here) opens
        // and the big blind calls.
        let line: Vec<_> = view
            .history
            .iter()
            .take_while(|e| !matches!(e, Event::Board { .. }))
            .cloned()
            .collect();
        assert_eq!(
            line,
            vec![
                Event::SmallBlind { seat: 0, amount: 1 },
                Event::BigBlind { seat: 1, amount: 2 },
                Event::Raise {
                    seat: 0,
                    to: 5,
                    all_in: false
                },
                Event::Call {
                    seat: 1,
                    amount: 3,
                    all_in: false
                },
            ]
        );
        // Three or more players left: no heads-up view.
        let deal = Deal::random(rules.variant, 6, Some(4)).expect("deal");
        let mut hand = Hand::new(rules, &[200; 6], 0, deal).expect("hand");
        hand.act(Action::Raise(5)).expect("UTG opens");
        hand.act(Action::Call).expect("HJ calls");
        for _ in 0..3 {
            hand.act(Action::Fold).expect("fold");
        }
        hand.act(Action::Call).expect("big blind calls");
        let obs = hand.observation(2).expect("big blind to act");
        assert!(heads_up_view(&obs).is_none());
    }

    /// A `post` bot that always calls, to tell its decisions apart.
    struct Calls;
    impl Bot for Calls {
        fn act(&mut self, _: &Observation) -> Option<Action> {
            Some(Action::Call)
        }
        fn hand_over(&mut self, _: &HandSummary) {}
    }

    #[test]
    fn a_compact_strategy_keeps_its_decisions_and_leaves_the_rest_to_post() {
        let game = PreflopGame::new(100);
        let Node::Decision {
            id: root, actions, ..
        } = &game.nodes[game.root()]
        else {
            panic!("root");
        };
        let fold = actions
            .iter()
            .position(|(a, _)| *a == Act::Fold)
            .expect("fold");
        // Folds everything at the root; elsewhere every action is equally likely.
        let mut n_of = vec![0; game.decisions as usize];
        for n in &game.nodes {
            if let Node::Decision { id, actions, .. } = n {
                n_of[*id as usize] = actions.len();
            }
        }
        let full = PreflopStrategy::from_fn(&game, |id, _| {
            let n = n_of[id as usize];
            if id == *root {
                (0..n).map(|i| if i == fold { 1.0 } else { 0.0 }).collect()
            } else {
                vec![1.0 / n as f32; n]
            }
        });
        let bytes = full.to_compact(|id| id == *root);
        let compact = PreflopStrategy::from_compact(&bytes).expect("reads back");
        assert_eq!(compact.game().decisions, game.decisions);
        assert_eq!(compact.probs(*root, 0)[fold], 1.0);
        let other = (0..game.decisions)
            .find(|&id| id != *root)
            .expect("another decision");
        assert!(compact.probs(other, 0).is_empty(), "left out");
        assert!(PreflopStrategy::from_compact(&bytes[..bytes.len() - 1]).is_err());
        assert!(PreflopStrategy::from_compact(b"not one").is_err());

        // UTG with the strategy folds; outside its depths, it plays as `post`.
        let rules = TableRules::no_limit_holdem(1, 2);
        let deal = Deal::random(rules.variant, 6, Some(5)).expect("deal");
        let hand = Hand::new(rules, &[200; 6], 0, deal).expect("hand");
        let obs = hand.observation(3).expect("UTG to act");
        let strategy = Arc::new(compact);
        let mut bot = SixMaxBot::new(strategy.clone(), Calls, 1).with_depths(70.0, 140.0);
        assert_eq!(bot.act(&obs), Some(Action::Fold));
        assert_eq!(bot.solved, 1);
        let deal = Deal::random(rules.variant, 6, Some(5)).expect("deal");
        let deep = Hand::new(rules, &[400; 6], 0, deal).expect("hand");
        let obs = deep.observation(3).expect("UTG to act");
        assert_eq!(
            bot.act(&obs),
            Some(Action::Call),
            "200 BB deep: post plays it"
        );
        assert_eq!(bot.off_tree, 1);

        // With a strategy for 140 to 280 BB too, that one plays 200 BB hands:
        // it folds UTG there, where `post` would call.
        let deep_game = PreflopGame::new(200);
        let Node::Decision {
            id: deep_root,
            actions: deep_actions,
            ..
        } = &deep_game.nodes[deep_game.root()]
        else {
            panic!("root");
        };
        let deep_fold = deep_actions
            .iter()
            .position(|(a, _)| *a == Act::Fold)
            .expect("fold");
        let mut deep_n = vec![0; deep_game.decisions as usize];
        for x in &deep_game.nodes {
            if let Node::Decision { id, actions, .. } = x {
                deep_n[*id as usize] = actions.len();
            }
        }
        let deep_bytes = PreflopStrategy::from_fn(&deep_game, |id, _| {
            let n = deep_n[id as usize];
            if id == *deep_root {
                (0..n)
                    .map(|i| if i == deep_fold { 1.0 } else { 0.0 })
                    .collect()
            } else {
                vec![1.0 / n as f32; n]
            }
        })
        .to_compact(|id| id == *deep_root);
        let deep_strategy = Arc::new(PreflopStrategy::from_compact(&deep_bytes).expect("200 BB"));
        let mut both = SixMaxBot::new(strategy, Calls, 1)
            .with_depths(70.0, 140.0)
            .with_strategy(deep_strategy, 140.0, 280.0);
        assert_eq!(
            both.act(&obs),
            Some(Action::Fold),
            "the 200 BB strategy plays it"
        );
        assert_eq!(both.solved, 1);
    }
}
