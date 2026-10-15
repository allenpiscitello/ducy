//! `DepthGtoBot`: blueprints trained at several stack depths, each hand
//! played by the one trained nearest that hand's depth.
//!
//! A blueprint knows only the stacks it was trained for: at another depth
//! [`GtoBot`] still follows its tree, but its plans (when to get all-in, how
//! big a raise commits it) are for the wrong stacks. So a 25 BB hand is better
//! played by a 25 BB blueprint than by the 100 BB one.
//!
//! The depth of a hand is the effective stack at its start, in big blinds:
//! the smaller of the two players' starting stacks. "Nearest" is by ratio,
//! not difference, since how a hand plays changes with the depth's scale:
//! 35 BB plays more like 25 BB than like 50, and 70 BB more like 50 than 100.

use std::sync::Arc;

use ducy_play::{Action, Bot, HandSummary, Observation};

use super::{
    abstraction::CardAbstraction, blueprint::BlueprintError, bot::GtoBot, hunl::HunlConfig,
};

/// Several [`GtoBot`]s, each with the depth its blueprint was trained at.
pub struct DepthGtoBot {
    /// (depth in big blinds, the bot), any order.
    bots: Vec<(u64, GtoBot)>,
    /// The bot playing the hand under way.
    current: Option<usize>,
}

impl DepthGtoBot {
    /// `bots` pairs each bot with the depth, in big blinds, its blueprint was
    /// trained at. Panics with no bots.
    pub fn new(bots: Vec<(u64, GtoBot)>) -> Self {
        assert!(!bots.is_empty(), "DepthGtoBot needs at least one blueprint");
        Self {
            bots,
            current: None,
        }
    }

    /// Loads blueprints, each given with the depth in big blinds it was
    /// trained at (`train_blueprint --bb`), on the one card abstraction.
    /// Loading checks each matches its depth.
    pub fn load(
        cards: Arc<CardAbstraction>,
        blueprints: &[(u64, &[u8])],
        seed: u64,
    ) -> Result<Self, BlueprintError> {
        let mut bots = Vec::new();
        for (i, &(depth, bytes)) in blueprints.iter().enumerate() {
            let config = depth_config(depth);
            bots.push((
                depth,
                GtoBot::new(config, cards.clone(), bytes, seed + i as u64)?,
            ));
        }
        Ok(Self::new(bots))
    }

    /// Applies `f` to every bot (turning on solving, say).
    pub fn map(self, f: impl Fn(GtoBot) -> GtoBot) -> Self {
        Self {
            bots: self.bots.into_iter().map(|(d, b)| (d, f(b))).collect(),
            current: None,
        }
    }

    /// The depths on offer, in the order given.
    pub fn depths(&self) -> Vec<u64> {
        self.bots.iter().map(|(d, _)| *d).collect()
    }

    /// Which bot plays a hand `depth_bb` deep: the one trained nearest it
    /// by ratio (ties go to the deeper one).
    pub fn nearest(&self, depth_bb: f64) -> usize {
        nearest(&self.depths(), depth_bb)
    }

    /// Each bot with its depth, in the order given (to read `off_tree` and
    /// the solve counts).
    pub fn bots(&self) -> impl Iterator<Item = (u64, &GtoBot)> {
        self.bots.iter().map(|(d, b)| (*d, b))
    }

    /// The depth of the blueprint playing the hand under way, if any.
    pub fn playing(&self) -> Option<u64> {
        self.current.map(|i| self.bots[i].0)
    }

    /// The bot for this hand's depth, for looking at its strategy.
    pub fn bot_for(&mut self, obs: &Observation) -> &mut GtoBot {
        let i = self.nearest(hand_depth_bb(obs));
        &mut self.bots[i].1
    }
}

/// The game a blueprint `depth_bb` deep was trained on: the default one
/// (blinds 1/2, the default menu) with stacks that deep, as `train_blueprint
/// --bb` makes it.
pub fn depth_config(depth_bb: u64) -> HunlConfig {
    let base = HunlConfig::default();
    HunlConfig {
        stack: depth_bb * base.big_blind,
        ..base
    }
}

/// The index in `depths` nearest `depth_bb` by ratio; ties go to the deeper.
pub fn nearest(depths: &[u64], depth_bb: f64) -> usize {
    let d = depth_bb.max(1e-9);
    let distance = |t: u64| (d / t.max(1) as f64).ln().abs();
    let mut best = 0;
    for (i, &t) in depths.iter().enumerate() {
        let (now, then) = (distance(t), distance(depths[best]));
        if now < then - 1e-12 || ((now - then).abs() <= 1e-12 && t > depths[best]) {
            best = i;
        }
    }
    best
}

/// The effective stack at the start of the hand, in big blinds: the bot's
/// own starting stack or the biggest opponent's, whichever is smaller. That
/// is what it can win or lose; a short stack elsewhere at the table doesn't
/// make its hand shallower.
pub fn hand_depth_bb(obs: &Observation) -> f64 {
    let start = |s: &ducy_play::SeatView| s.stack + s.contributed;
    let own = start(&obs.seats[obs.seat]);
    let biggest = obs
        .seats
        .iter()
        .enumerate()
        .filter(|&(i, _)| i != obs.seat)
        .map(|(_, s)| start(s))
        .max()
        .unwrap_or(own);
    own.min(biggest) as f64 / obs.rules.big_blind.max(1) as f64
}

impl Bot for DepthGtoBot {
    fn act(&mut self, obs: &Observation) -> Option<Action> {
        // Starting stacks don't change during a hand, so neither does the
        // bot playing it.
        let i = self.nearest(hand_depth_bb(obs));
        self.current = Some(i);
        self.bots[i].1.act(obs)
    }

    fn hand_over(&mut self, summary: &HandSummary) {
        if let Some(i) = self.current.take() {
            self.bots[i].1.hand_over(summary);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_is_by_ratio() {
        let depths = [10, 25, 50, 100];
        let at = |d: f64| depths[nearest(&depths, d)];
        assert_eq!(at(5.0), 10);
        assert_eq!(at(10.0), 10);
        assert_eq!(at(15.0), 10); // 1.5× from 10, 1.67× from 25
        assert_eq!(at(17.0), 25);
        assert_eq!(at(35.0), 25); // 1.4× from 25, 1.43× from 50
        assert_eq!(at(36.0), 50);
        assert_eq!(at(70.0), 50); // 1.4× from 50, 1.43× from 100
        assert_eq!(at(75.0), 100);
        assert_eq!(at(200.0), 100);
        // Exactly between by ratio: the deeper one.
        assert_eq!(at(50.0_f64.sqrt() * 100.0_f64.sqrt()), 100);
    }

    #[test]
    fn depth_is_against_the_biggest_opponent() {
        use ducy_play::{Deal, Hand, TableRules};
        // Blinds 1/2: 200 BB, 20 BB and 500 BB.
        let rules = TableRules::no_limit_holdem(1, 2);
        let deal = Deal::random(rules.variant, 3, Some(1)).expect("deal");
        let hand = Hand::new(rules, &[400, 40, 1000], 0, deal).expect("hand");
        // The seat to act's view, looked at from each seat in turn.
        let to_act = hand.legal_actions().expect("someone to act").seat;
        let base = hand.observation(to_act).expect("observation");
        let depth = |seat| {
            let mut obs = base.clone();
            obs.seat = seat;
            hand_depth_bb(&obs)
        };
        // The deep stack's biggest rival is the 200 BB one: 200 BB, not 20.
        assert_eq!(depth(2), 200.0);
        assert_eq!(depth(0), 200.0);
        // The short stack can only lose its 20 BB.
        assert_eq!(depth(1), 20.0);
    }

    #[test]
    fn order_given_does_not_matter() {
        let depths = [100, 10, 50, 25];
        assert_eq!(depths[nearest(&depths, 30.0)], 25);
        assert_eq!(depths[nearest(&depths, 400.0)], 100);
    }
}
