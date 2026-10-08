use std::sync::Arc;

use ducy_gto::{
    Profile,
    holdem::{
        abstraction::CardAbstraction,
        blueprint::Blueprint,
        bot::{GtoBot, RiverSolving, TurnSolving, pseudo_harmonic},
        hunl::{BetMenu, Hunl, HunlConfig, Size},
    },
};
use ducy_play::{
    Bot, MatchConfig, TableRules, bots::RandomBot, personality::Personality, run_match,
};

/// A bot on the quick abstraction with an untrained (uniform) blueprint:
/// enough to check that it always plays legally and follows the tree.
fn bot(seed: u64) -> GtoBot {
    let cards = Arc::new(CardAbstraction::quick(8));
    let config = HunlConfig::default();
    let game = Hunl::new(config.clone(), Some(&cards));
    let bp = Blueprint::from_profile(&game, &cards, &Profile::new());
    GtoBot::new(config, cards, &bp.save(), seed).expect("matching blueprint")
}

#[test]
fn pseudo_harmonic_mapping() {
    // On a menu size, it maps to itself.
    assert_eq!(pseudo_harmonic(0.5, 1.0, 0.5), 1.0);
    assert_eq!(pseudo_harmonic(0.5, 1.0, 1.0), 0.0);
    // Halfway between half pot and pot: the formula's value.
    let p = pseudo_harmonic(0.5, 1.0, 0.75);
    assert!((p - (0.25 * 1.5) / (0.5 * 1.75)).abs() < 1e-12);
    // Monotone: bigger bets map to the bigger size more often.
    assert!(pseudo_harmonic(0.5, 1.0, 0.6) > pseudo_harmonic(0.5, 1.0, 0.9));
}

fn play(mut a: GtoBot, other: Box<dyn Bot>, deals: usize, rules: TableRules) -> (u32, Vec<u32>) {
    let off = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    struct Counting(GtoBot, Arc<std::sync::atomic::AtomicU32>);
    impl Bot for Counting {
        fn act(&mut self, obs: &ducy_play::Observation) -> Option<ducy_play::Action> {
            let before = self.0.off_tree;
            let a = self.0.act(obs);
            self.1.fetch_add(
                self.0.off_tree - before,
                std::sync::atomic::Ordering::Relaxed,
            );
            a
        }
        fn hand_over(&mut self, s: &ducy_play::HandSummary) {
            self.0.hand_over(s);
        }
    }
    a.off_tree = 0;
    let mut bots: Vec<Box<dyn Bot>> = vec![Box::new(Counting(a, off.clone())), other];
    let r = run_match(&MatchConfig::new(rules, deals, 11).duplicate(), &mut bots).unwrap();
    (off.load(std::sync::atomic::Ordering::Relaxed), r.fallbacks)
}

#[test]
fn always_legal_and_on_the_tree_against_wild_bets() {
    // RandomBot bets every size; the bot must map them all onto the tree.
    let (off, fallbacks) = play(
        bot(1),
        Box::new(RandomBot::new(Some(2))),
        400,
        TableRules::no_limit_holdem(1, 2),
    );
    assert_eq!(fallbacks[0], 0, "illegal actions");
    assert_eq!(off, 0, "lost the tree");
}

#[test]
fn always_legal_against_itself_and_a_personality() {
    let (off, fallbacks) = play(
        bot(3),
        Box::new(bot(4)),
        300,
        TableRules::no_limit_holdem(1, 2),
    );
    assert_eq!(fallbacks, vec![0, 0]);
    assert_eq!(off, 0);
    let p = Personality::ALL[0].bot(Some(5));
    let (off, fallbacks) = play(bot(6), Box::new(p), 300, TableRules::no_limit_holdem(1, 2));
    assert_eq!(fallbacks[0], 0);
    assert_eq!(off, 0);
}

#[test]
fn other_blinds_and_stacks_still_play_legally() {
    // Trained for 100 big blinds at 1/2; sizes are pot fractions, so other
    // blinds work, and deeper or shorter stacks clamp to what's legal.
    for (sb, bb, stack_bbs) in [(5, 10, 100), (1, 2, 40), (1, 2, 250)] {
        let rules = TableRules::no_limit_holdem(sb, bb);
        let mut bots: Vec<Box<dyn Bot>> = vec![Box::new(bot(7)), Box::new(RandomBot::new(Some(8)))];
        let config = MatchConfig::new(rules, 150, 3)
            .with_starting_stack(bb * stack_bbs)
            .duplicate();
        let r = run_match(&config, &mut bots).unwrap();
        assert_eq!(r.fallbacks[0], 0, "{sb}/{bb} at {stack_bbs}bb");
    }
}

#[test]
fn same_seed_same_play() {
    let rules = TableRules::no_limit_holdem(1, 2);
    let run = || {
        let mut bots: Vec<Box<dyn Bot>> =
            vec![Box::new(bot(9)), Box::new(RandomBot::new(Some(10)))];
        run_match(&MatchConfig::new(rules, 100, 4), &mut bots)
            .unwrap()
            .net
    };
    assert_eq!(run(), run());
}

/// Plays `deals` duplicate deals with a river-solving bot in seat 0 and
/// returns (illegal actions, river solves).
fn play_solving(other: Box<dyn Bot>, deals: usize, rules: TableRules, stack: u64) -> (u32, u32) {
    use std::sync::Mutex;
    struct Shared(Arc<Mutex<GtoBot>>);
    impl Bot for Shared {
        fn act(&mut self, o: &ducy_play::Observation) -> Option<ducy_play::Action> {
            self.0.lock().unwrap().act(o)
        }
        fn hand_over(&mut self, s: &ducy_play::HandSummary) {
            self.0.lock().unwrap().hand_over(s)
        }
    }
    let gto = Arc::new(Mutex::new(bot(21).with_river_solving(RiverSolving::new(6))));
    let mut bots: Vec<Box<dyn Bot>> = vec![Box::new(Shared(gto.clone())), other];
    let config = MatchConfig::new(rules, deals, 17)
        .with_starting_stack(stack)
        .duplicate();
    let r = run_match(&config, &mut bots).unwrap();
    let solves = gto.lock().unwrap().river_solves;
    (r.fallbacks[0], solves)
}

#[test]
fn river_solving_plays_legally_against_wild_bets() {
    // RandomBot bets every size, so the bot re-solves for off-tree bets.
    let (illegal, solves) = play_solving(
        Box::new(RandomBot::new(Some(22))),
        150,
        TableRules::no_limit_holdem(1, 2),
        200,
    );
    assert_eq!(illegal, 0);
    assert!(solves > 0, "never solved a river");
}

#[test]
fn river_solving_plays_legally_at_other_stakes() {
    let (illegal, solves) = play_solving(
        Box::new(ducy_play::bots::CallingStation),
        40,
        TableRules::no_limit_holdem(5, 10),
        400,
    );
    assert_eq!(illegal, 0);
    assert!(solves > 0, "never solved a river");
}

/// Turn solving (with river solving after it) plays legally, against wild
/// bets that force re-solves, and at other stakes.
#[test]
fn turn_solving_plays_legally() {
    use std::sync::Mutex;
    struct Shared(Arc<Mutex<GtoBot>>);
    impl Bot for Shared {
        fn act(&mut self, o: &ducy_play::Observation) -> Option<ducy_play::Action> {
            self.0.lock().unwrap().act(o)
        }
        fn hand_over(&mut self, s: &ducy_play::HandSummary) {
            self.0.lock().unwrap().hand_over(s)
        }
    }
    // A small menu keeps each solve quick in a debug build.
    let small = HunlConfig {
        menu: BetMenu {
            preflop: BetMenu::default().preflop,
            postflop: vec![vec![Size::Pot(0.75), Size::AllIn], vec![Size::AllIn]],
        },
        ..HunlConfig::default()
    };
    let turn = TurnSolving {
        config: small.clone(),
        ..TurnSolving::new(2)
    };
    let river = RiverSolving {
        config: small,
        ..RiverSolving::new(4)
    };
    let cases: Vec<(Box<dyn Bot>, TableRules, u64)> = vec![
        (
            Box::new(RandomBot::new(Some(31))),
            TableRules::no_limit_holdem(1, 2),
            200,
        ),
        (
            Box::new(ducy_play::bots::CallingStation),
            TableRules::no_limit_holdem(5, 10),
            400,
        ),
    ];
    for (other, rules, stack) in cases {
        let gto = bot(32)
            .with_turn_solving(turn.clone())
            .with_river_solving(river.clone());
        let gto = Arc::new(Mutex::new(gto));
        let mut bots: Vec<Box<dyn Bot>> = vec![Box::new(Shared(gto.clone())), other];
        let config = MatchConfig::new(rules, 20, 19)
            .with_starting_stack(stack)
            .duplicate();
        let r = run_match(&config, &mut bots).unwrap();
        let g = gto.lock().unwrap();
        assert_eq!(r.fallbacks[0], 0, "illegal actions");
        assert!(g.turn_solves > 0, "never solved a turn");
        assert!(g.river_solves > 0, "never solved a river");
    }
}

#[test]
fn more_than_two_players_falls_back_legally() {
    let mut bots: Vec<Box<dyn Bot>> = vec![
        Box::new(bot(12)),
        Box::new(RandomBot::new(Some(13))),
        Box::new(RandomBot::new(Some(14))),
    ];
    let r = run_match(
        &MatchConfig::new(TableRules::no_limit_holdem(1, 2), 100, 5),
        &mut bots,
    )
    .unwrap();
    assert_eq!(r.fallbacks[0], 0);
}
