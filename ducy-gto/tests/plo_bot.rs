use std::{cell::Cell, rc::Rc, sync::Arc};

use ducy_gto::{
    Config, Discount, Mccfr,
    holdem::{
        blueprint::Blueprint,
        hunl::{HuPlo, HunlConfig},
    },
    omaha::{
        abstraction::{PloAbstraction, PloAbstractionConfig},
        bot::PloGtoBot,
    },
};
use ducy_play::{
    Action, Bot, HandSummary, MatchConfig, Observation, TableRules,
    bots::{CallingStation, RandomBot},
    personality::Personality,
    run_match,
};

/// The bot, sharing its off-tree count.
struct Watched(PloGtoBot, Rc<Cell<u32>>);

impl Bot for Watched {
    fn act(&mut self, obs: &Observation) -> Option<Action> {
        let a = self.0.act(obs);
        self.1.set(self.0.off_tree);
        a
    }

    fn hand_over(&mut self, summary: &HandSummary) {
        self.0.hand_over(summary);
    }
}

/// A small abstraction and a briefly trained 10 big blind blueprint.
fn setup() -> (HunlConfig, Arc<PloAbstraction>, Vec<u8>) {
    let cards = Arc::new(PloAbstraction::build(
        PloAbstractionConfig {
            preflop: 0,
            flop: 8,
            turn: 8,
            river: 8,
            fit_hands: 800,
            equity_samples: 100,
            ..PloAbstractionConfig::default()
        },
        |_| {},
    ));
    let config = HunlConfig::pot_limit_omaha_lean(10);
    let game = HuPlo::with_cards(config.clone(), Some(&*cards));
    let mut m = Mccfr::new(
        &game,
        Config {
            seed: 1,
            batch: 256,
            discount: Discount::DCFR,
            prune: None,
        },
    );
    m.run(3000);
    let bp = Blueprint::from_strategy(&game, &*cards, |info, n| {
        m.average_at(&info)
            .unwrap_or_else(|| vec![1.0 / n as f64; n])
    });
    (config, cards, bp.save())
}

#[test]
fn plays_legal_plo_against_every_kind_of_bet() {
    let (config, cards, bytes) = setup();
    // A blueprint for another depth is refused.
    let deeper = HunlConfig::pot_limit_omaha_lean(20);
    assert!(PloGtoBot::new(deeper, cards.clone(), &bytes, 1).is_err());
    let rules = TableRules::pot_limit_omaha(1, 2);
    let opponents: Vec<Box<dyn Bot>> = vec![
        Box::new(RandomBot::new(Some(3))),
        Box::new(CallingStation),
        Box::new(Personality::NikAirbag.bot(Some(4))),
        Box::new(PloGtoBot::new(config.clone(), cards.clone(), &bytes, 5).unwrap()),
    ];
    for (i, other) in opponents.into_iter().enumerate() {
        let off_tree = Rc::new(Cell::new(0));
        let me = PloGtoBot::new(config.clone(), cards.clone(), &bytes, 7).expect("loads");
        let mut bots: Vec<Box<dyn Bot>> = vec![Box::new(Watched(me, off_tree.clone())), other];
        // Deeper than the blueprint's 10 BB sometimes, so real stacks don't
        // always match the tree's.
        let stack = if i % 2 == 0 { 20 } else { 33 };
        let r = run_match(
            &MatchConfig::new(rules, 30, 11 + i as u64)
                .with_starting_stack(stack)
                .duplicate(),
            &mut bots,
        )
        .unwrap();
        assert_eq!(r.fallbacks[0], 0, "opponent {i}: an illegal action");
        assert_eq!(r.hands, 60);
        assert_eq!(off_tree.get(), 0, "opponent {i}: off the tree");
    }
}
