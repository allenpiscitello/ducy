//! What decision features (#214) cost per decision, against a personality
//! bot's own decision, in six-handed Hold'em, PLO4 and PLO6.
//!
//!     cargo run --release -p ducy-play --example features_cost
//!
//! Reports microseconds per decision: the bot's `act`, then the features
//! without equity (`samples: 0`) and with the bots' default 150 deals.

use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

use ducy_play::{
    Action, Bot, HandSummary, MatchConfig, Observation, TableRules, Variant,
    features::{FeatureConfig, decision_features},
    personality::{Personality, PersonalityBot},
    run_match,
    stats::OpponentModel,
};
use rand::{SeedableRng, rngs::StdRng};

#[derive(Default, Clone, Copy)]
struct Times {
    decisions: u32,
    act: Duration,
    plain: Duration,
    full: Duration,
}

struct Timed {
    bot: PersonalityBot,
    model: OpponentModel,
    rng: StdRng,
    t: Rc<Cell<Times>>,
}

impl Bot for Timed {
    fn act(&mut self, obs: &Observation) -> Option<Action> {
        let style = *self.bot.style();
        let mut t = self.t.get();
        let start = Instant::now();
        std::hint::black_box(decision_features(
            obs,
            &self.model,
            &style,
            &FeatureConfig { samples: 0 },
            &mut self.rng,
        ));
        t.plain += start.elapsed();
        let start = Instant::now();
        std::hint::black_box(decision_features(
            obs,
            &self.model,
            &style,
            &FeatureConfig::default(),
            &mut self.rng,
        ));
        t.full += start.elapsed();
        let start = Instant::now();
        let a = self.bot.act(obs);
        t.act += start.elapsed();
        t.decisions += 1;
        self.t.set(t);
        a
    }

    fn hand_over(&mut self, summary: &HandSummary) {
        self.model
            .record(&summary.history, summary.result.net.len());
        self.bot.hand_over(summary);
    }
}

fn main() {
    let games = [
        ("Hold'em", TableRules::no_limit_holdem(1, 2)),
        ("PLO4", TableRules::pot_limit_omaha(1, 2)),
        (
            "PLO6",
            TableRules::pot_limit_omaha(1, 2).with_variant(Variant::Omaha { hole_cards: 6 }),
        ),
    ];
    println!(
        "{:>8} {:>9} {:>12} {:>14} {:>14}",
        "game", "decisions", "bot act µs", "features µs", "+ equity µs"
    );
    for (name, rules) in games {
        let times: Vec<Rc<Cell<Times>>> = (0..6).map(|_| Rc::default()).collect();
        let mut bots: Vec<Box<dyn Bot>> = Personality::ALL[..6]
            .iter()
            .zip(&times)
            .enumerate()
            .map(|(i, (p, t))| {
                Box::new(Timed {
                    bot: p.bot(Some(i as u64)),
                    model: OpponentModel::new(),
                    rng: StdRng::seed_from_u64(i as u64),
                    t: t.clone(),
                }) as Box<dyn Bot>
            })
            .collect();
        run_match(&MatchConfig::new(rules, 300, 3), &mut bots).expect("match");
        let mut sum = Times::default();
        for t in &times {
            let t = t.get();
            sum.decisions += t.decisions;
            sum.act += t.act;
            sum.plain += t.plain;
            sum.full += t.full;
        }
        let us = |d: Duration| d.as_secs_f64() * 1e6 / sum.decisions.max(1) as f64;
        println!(
            "{name:>8} {:>9} {:>12.1} {:>14.1} {:>14.1}",
            sum.decisions,
            us(sum.act),
            us(sum.plain),
            us(sum.full)
        );
    }
}
