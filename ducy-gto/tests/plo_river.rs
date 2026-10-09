use std::{cell::Cell, rc::Rc, sync::Arc};

use ducy_gto::{
    Config, Discount, Game, Mccfr, Rng, Turn,
    holdem::{
        blueprint::Blueprint,
        cards::{Card, mask},
        hunl::{HuPlo, HunlConfig},
    },
    omaha::{
        abstraction::{PloAbstraction, PloAbstractionConfig},
        bot::PloGtoBot,
        river::{PloRiverSolver, PloRiverSolving, RiverRange, best_response_value, sample_range},
    },
};
use ducy_play::{
    Bot, MatchConfig, TableRules,
    bots::{CallingStation, RandomBot},
    run_match,
};

/// A small abstraction and a briefly trained 10 big blind blueprint.
fn setup() -> (HunlConfig, Arc<PloAbstraction>, Blueprint) {
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
    (config, cards, bp)
}

#[test]
fn river_solves_converge_and_beat_the_blueprint_river() {
    let (config, cards, blueprint) = setup();
    let game = HuPlo::with_cards(config.clone(), Some(&*cards));
    let mut rng = Rng::new(5);
    let mut checked = 0;
    let (mut solved_total, mut plain_total, mut converged) = (0.0, 0.0, 0.0);
    while checked < 4 {
        // The blueprint plays itself to a river decision.
        let mut s = game.sample_chance(&game.root(), &mut rng);
        let mut steps = Vec::new();
        let root = loop {
            match game.turn(&s) {
                Turn::Player(p) => {
                    let b = game.betting(&s);
                    if b.street == 3 {
                        break Some(s.node);
                    }
                    let a = rng.sample(&blueprint.probs(s.node, s.buckets[p][b.street]));
                    steps.push((s.node, a));
                    s = game.apply(&s, a);
                }
                _ => break None,
            }
        };
        let Some(bp_root) = root else { continue };
        checked += 1;
        let bot = 1;
        let mut sample = |player: usize, blocked: u64, first: Option<[Card; 4]>, n: usize| {
            sample_range(
                &cards, &blueprint, &game.tree, &steps, player, &s.board, blocked, n, first,
                &mut rng,
            )
        };
        let mine = sample(bot, 0, Some(s.hole[bot]), 40);
        let theirs = sample(1 - bot, mask(&s.hole[bot]), None, 40);
        assert_eq!(mine.hands[0], s.hole[bot], "our own hand is hand 0");
        let fresh = sample(1 - bot, 0, None, 80);
        let betting = game.tree.nodes[bp_root as usize].betting.clone();
        let mut solver =
            PloRiverSolver::new(&s.board, &betting, &config, &[], [theirs, mine.clone()]);
        let reference =
            solver.blueprint_strategy(&cards, &s.board, &blueprint, &game.tree, bp_root);
        let target = solver.best_response(1 - bot, &reference);
        solver.set_gadget(1 - bot, target);
        // Exploitability within the solve's own ranges falls as it runs.
        solver.run(1, None);
        let early = solver.average();
        solver.run(150, None);
        let late = solver.average();
        let ranges: [RiverRange; 2] = solver.ranges.clone();
        let br = |strategy| {
            best_response_value(
                &solver.tree,
                &s.board,
                1 - bot,
                &ranges[1],
                strategy,
                &ranges[0],
            )
        };
        assert!(
            br(&late) <= br(&early) + 1e-3,
            "{} vs {}",
            br(&late),
            br(&early)
        );
        converged += br(&early) - br(&late);
        // Against fresh opponent hands: no worse than the blueprint's river.
        solved_total += best_response_value(&solver.tree, &s.board, 1 - bot, &mine, &late, &fresh);
        plain_total +=
            best_response_value(&solver.tree, &s.board, 1 - bot, &mine, &reference, &fresh);
    }
    assert!(converged > 0.05, "the solves gained {converged} chips");
    assert!(
        solved_total <= plain_total + 0.5,
        "solved {solved_total} vs blueprint {plain_total}"
    );
}

#[test]
fn the_bot_solves_rivers_and_plays_legally() {
    let (config, cards, blueprint) = setup();
    let game = HuPlo::with_cards(config.clone(), Some(&*cards));
    let tree = Arc::new(game.tree);
    let blueprint = Arc::new(blueprint);
    let mut solving = PloRiverSolving::new(30);
    solving.hands = 32;
    let me = PloGtoBot::from_parts(config, cards, blueprint, tree, 3).with_river_solving(solving);
    let solves = Rc::new(Cell::new(0));
    let rules = TableRules::pot_limit_omaha(1, 2);
    let mut bots: Vec<Box<dyn Bot>> = vec![
        Box::new(Watched(me, solves.clone())),
        Box::new(CallingStation),
    ];
    let r = run_match(
        &MatchConfig::new(rules, 20, 9)
            .with_starting_stack(20)
            .duplicate(),
        &mut bots,
    )
    .unwrap();
    assert_eq!(r.fallbacks[0], 0);
    bots[1] = Box::new(RandomBot::new(Some(2)));
    let r = run_match(
        &MatchConfig::new(rules, 20, 10)
            .with_starting_stack(33)
            .duplicate(),
        &mut bots,
    )
    .unwrap();
    assert_eq!(r.fallbacks[0], 0);
    assert!(solves.get() > 5, "{} river solves", solves.get());
}

/// The bot, sharing its river-solve count.
struct Watched(PloGtoBot, Rc<Cell<u32>>);

impl Bot for Watched {
    fn act(&mut self, obs: &ducy_play::Observation) -> Option<ducy_play::Action> {
        let a = self.0.act(obs);
        self.1.set(self.0.river_solves);
        a
    }

    fn hand_over(&mut self, summary: &ducy_play::HandSummary) {
        self.0.hand_over(summary);
    }
}
