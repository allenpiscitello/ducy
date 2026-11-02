//! `NetBot` (#215): plays a personality from a network. At each decision it
//! builds the features (#214) with the personality's style for this table,
//! runs the network, and samples one of the possible action classes.
//!
//! Hard rules stay on top of the network: holding the nuts on the river, it
//! always bets or raises (#212). The network still picks the size, small to
//! all-in: a small bet is sometimes the one that gets paid.

use std::sync::Arc;

use ducy::deck::Card;
use rand::{RngExt, SeedableRng, rngs::StdRng};

use super::{ActionClass, Mlp, NUM_CLASSES};
use ducy_play::{Action, Bot, HandSummary, Observation, Street};

use crate::{
    features::{FeatureConfig, NUM_FEATURES, decision_features},
    personality::{Personality, PersonalityBot, Style},
    ranges::has_nuts,
    stats::OpponentModel,
};

/// A bot that plays from a network, as one personality.
pub struct NetBot {
    net: Arc<Mlp>,
    /// The personality: its style for each table, and its mood.
    style: PersonalityBot,
    model: OpponentModel,
    rng: StdRng,
    config: FeatureConfig,
}

impl NetBot {
    /// Plays `personality` from `net`, which must take the decision features
    /// and give one output per action class. A `seed` makes it reproducible.
    pub fn new(personality: Personality, net: Arc<Mlp>, seed: Option<u64>) -> Self {
        Self::with_style(
            PersonalityBot::new(personality.name(), personality.style(), seed),
            net,
            seed,
        )
    }

    /// Plays the style of `bot` (any [`Style`], through
    /// [`PersonalityBot::new`]) from `net`.
    pub fn with_style(bot: PersonalityBot, net: Arc<Mlp>, seed: Option<u64>) -> Self {
        let sizes = net.sizes();
        assert_eq!(
            sizes[0], NUM_FEATURES,
            "the network takes the decision features"
        );
        assert_eq!(
            *sizes.last().unwrap(),
            NUM_CLASSES,
            "the network gives an output per action class"
        );
        Self {
            net,
            style: bot,
            model: OpponentModel::new(),
            rng: match seed {
                Some(s) => StdRng::seed_from_u64(s ^ 0x6e65_7462_6f74),
                None => rand::make_rng(),
            },
            config: FeatureConfig::default(),
        }
    }

    /// Its features configuration (equity deals per estimate).
    pub fn with_features(mut self, config: FeatureConfig) -> Self {
        self.config = config;
        self
    }

    /// The probability of each action class for `obs`, 0 for impossible
    /// ones, and for checking, calling and folding with the nuts on the
    /// river when it can bet or raise.
    pub fn policy(&mut self, obs: &Observation) -> [f32; NUM_CLASSES] {
        let style: Style = self.style.style_for(obs);
        let f = decision_features(obs, &self.model, &style, &self.config, &mut self.rng);
        let mut mask = ActionClass::mask(obs);
        if must_bet(obs, &mask) {
            mask[ActionClass::Fold.index()] = false;
            mask[ActionClass::CheckCall.index()] = false;
        }
        let p = self.net.probs(&f, &mask);
        std::array::from_fn(|i| p[i])
    }
}

/// On the river with the nuts, and able to bet or raise: it must (#212).
fn must_bet(obs: &Observation, mask: &[bool; NUM_CLASSES]) -> bool {
    if obs.street != Street::River || !mask[ActionClass::Third.index()..].iter().any(|&m| m) {
        return false;
    }
    let hole: Vec<Card> = obs.hole_cards.iter(false).collect();
    !hole.is_empty() && has_nuts(obs.rules.variant, &hole, &obs.board)
}

impl Bot for NetBot {
    fn act(&mut self, obs: &Observation) -> Option<Action> {
        let p = self.policy(obs);
        let total: f32 = p.iter().sum();
        if total <= 0.0 {
            return None;
        }
        let mut r = self.rng.random::<f32>() * total;
        let mut pick = None;
        for (i, &q) in p.iter().enumerate() {
            if q > 0.0 {
                pick = Some(i);
                if r < q {
                    break;
                }
                r -= q;
            }
        }
        ActionClass::from_index(pick?)?.action(obs)
    }

    fn hand_over(&mut self, summary: &HandSummary) {
        self.model
            .record(&summary.history, summary.result.net.len());
        self.style.hand_over(summary);
    }
}
