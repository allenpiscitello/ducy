//! A six-handed bot that plays the preflop solve (#245, stage 2): before the
//! flop it follows the solved strategy for its position and hand; after the
//! flop, and wherever the preflop leaves the solved tree, it plays as
//! `post`, another bot (Doug Poker's style to start with, so the gain over
//! Doug is the preflop's).
//!
//! The table's real actions are mapped onto the tree by kind, not size:
//! every raise is the tree's raise at that level, every all-in its all-in.
//! Its own raises are the tree's sizes in the table's chips.

use std::sync::Arc;

use ducy_play::{Action, Bot, Event, HandSummary, Observation, Street};

use super::{
    preflop::{Act, Node, PreflopGame, SEATS},
    solver::PreflopSolver,
};
use crate::{
    Rng,
    holdem::{cards::from_ducy, iso::NUM_PREFLOP_CLASSES, iso::preflop_class},
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
        let mut offsets = vec![(0, 0); game.decisions as usize];
        let mut probs = Vec::new();
        for node in &game.nodes {
            if let Node::Decision { id, actions, .. } = node {
                offsets[*id as usize] = (probs.len(), actions.len());
                for class in 0..C {
                    probs.extend(solver.strategy(*id, class).iter().map(|&p| p as f32));
                }
            }
        }
        Self {
            game: game.clone(),
            offsets,
            probs,
        }
    }

    fn probs(&self, id: u32, class: usize) -> &[f32] {
        let (start, n) = self.offsets[id as usize];
        &self.probs[start + class * n..start + (class + 1) * n]
    }
}

/// The six-handed bot.
pub struct SixMaxBot<B: Bot> {
    strategy: Arc<PreflopStrategy>,
    post: B,
    /// Plays heads-up pots after the flop, seeing them as a heads-up hand
    /// (`heads_up_view`): a heads-up GTO bot. Without one, `post` does.
    heads_up: Option<Box<dyn Bot>>,
    rng: Rng,
    /// Preflop decisions taken from the solve, and those left to `post`.
    pub solved: u64,
    pub off_tree: u64,
    /// Decisions after the flop the heads-up bot made.
    pub heads_up_decisions: u64,
}

impl<B: Bot> SixMaxBot<B> {
    pub fn new(strategy: Arc<PreflopStrategy>, post: B, seed: u64) -> Self {
        Self {
            strategy,
            post,
            heads_up: None,
            rng: Rng::new(seed),
            solved: 0,
            off_tree: 0,
            heads_up_decisions: 0,
        }
    }

    /// Plays heads-up pots after the flop with `bot`.
    pub fn with_heads_up(mut self, bot: Box<dyn Bot>) -> Self {
        self.heads_up = Some(bot);
        self
    }

    /// The solve's position of a seat: 0 UTG, 1 HJ, 2 CO, 3 BTN, 4 SB, 5 BB.
    fn position(seat: usize, button: usize) -> usize {
        (seat + SEATS - button + 3) % SEATS
    }

    /// The tree node the preflop has reached, following the hand's actions,
    /// or None if it left the tree.
    fn node(&self, obs: &Observation) -> Option<usize> {
        let game = &self.strategy.game;
        let mut node = game.root();
        for e in &obs.history {
            let (seat, act) = match *e {
                Event::Fold { seat } => (seat, Some(Act::Fold)),
                Event::Check { seat } | Event::Call { seat, .. } => (seat, Some(Act::Call)),
                Event::Bet { seat, all_in, .. } | Event::Raise { seat, all_in, .. } => {
                    (seat, Some(if all_in { Act::AllIn } else { Act::Raise(0) }))
                }
                Event::Board { .. } => return None,
                _ => continue,
            };
            let Node::Decision {
                seat: pos, actions, ..
            } = &game.nodes[node]
            else {
                return None;
            };
            if *pos != Self::position(seat, obs.button) {
                return None;
            }
            let act = act?;
            let next = actions.iter().find(|(a, _)| match (a, act) {
                (Act::Raise(_), Act::Raise(_)) => true,
                (a, b) => *a == b,
            });
            // A raise the tree only has as an all-in (short stacks).
            let next = next.or_else(|| {
                matches!(act, Act::Raise(_))
                    .then(|| actions.iter().find(|(a, _)| *a == Act::AllIn))
                    .flatten()
            });
            node = next?.1 as usize;
        }
        Some(node)
    }

    fn preflop(&mut self, obs: &Observation) -> Option<Action> {
        if obs.seats.len() != SEATS || obs.seats.iter().any(|s| s.stack + s.contributed == 0) {
            return None;
        }
        let node = self.node(obs)?;
        let Node::Decision { seat, id, actions } = &self.strategy.game.nodes[node] else {
            return None;
        };
        if *seat != Self::position(obs.seat, obs.button) {
            return None;
        }
        let cards: Vec<_> = obs.hole_cards.iter(false).map(from_ducy).collect();
        let class = preflop_class(cards[0], cards[1]);
        let probs = self.strategy.probs(*id, class);
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
}
