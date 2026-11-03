//! The six-handed blueprint bot (#338): legal actions in Hold'em and PLO,
//! on-tree self-play that matches the tree chip for chip, and off-tree
//! counts.

use std::sync::{Arc, Mutex, atomic::Ordering::Relaxed};

use ducy_bots::{
    MatchConfig,
    bots::{CallingStation, RandomBot},
    personality::Personality,
    run_match,
};
use ducy_gto::{
    Config, Discount, Mccfr,
    holdem::{abstraction::CardAbstraction, hunl::Buckets},
    omaha::abstraction::{PloAbstraction, PloAbstractionConfig},
    sixmax::{
        multiway::{MultiwayBlueprint, MultiwayConfig, MultiwayGame},
        multiway_bot::{MultiwayBot, MultiwayPlayer, MwCounts, OffTree, Settings},
    },
};
use ducy_play::{Action, Bot, HandSummary, Observation, TableRules, Variant};

/// The tree's depth: 20 big blinds, so training is quick.
const BB: u32 = 20;

/// A briefly trained blueprint on `menu` at 20 big blinds, written and
/// loaded as `train_sixmax` and the match tool do.
fn train<A: Buckets + Sync, const H: usize>(
    menu: &str,
    cards: &A,
    card_bytes: &[u8],
) -> Arc<MultiwayPlayer> {
    let config = MultiwayConfig::preset(menu, BB).expect("a menu");
    let game = MultiwayGame::<_, H>::new(config, Some(cards));
    let mut m = Mccfr::new(
        &game,
        Config {
            seed: 1,
            batch: 256,
            discount: Discount::DCFR,
            prune: None,
        },
    );
    m.run(2000);
    let settings = format!("{menu} {BB} BB, {H} hole cards | {}", cards.settings());
    let mut bytes = Vec::new();
    MultiwayBlueprint::write(
        &settings,
        m.num_infosets(),
        |f| m.average_each(f),
        &mut bytes,
    )
    .expect("write");
    Arc::new(MultiwayPlayer::load(&bytes, card_bytes).expect("loads"))
}

fn holdem() -> Arc<MultiwayPlayer> {
    let cards = CardAbstraction::quick(4);
    train::<_, 2>("holdem-lean", &cards, &cards.save())
}

fn plo<const H: usize>() -> Arc<MultiwayPlayer> {
    let cards = PloAbstraction::build(
        PloAbstractionConfig {
            hole_cards: H,
            preflop: if H == 4 { 0 } else { 8 },
            flop: 6,
            turn: 6,
            river: 6,
            fit_hands: 800,
            equity_samples: 100,
            ..PloAbstractionConfig::default()
        },
        |_| {},
    );
    train::<_, H>("plo-lean", &cards, &cards.save())
}

fn rules(hole_cards: usize) -> TableRules {
    if hole_cards == 2 {
        TableRules::no_limit_holdem(1, 2)
    } else {
        TableRules::pot_limit_omaha(1, 2).with_variant(Variant::Omaha {
            hole_cards: hole_cards as u32,
        })
    }
}

/// The bot, checking at every decision on the tree that the tree's chips
/// are the real ones.
struct Checked(MultiwayBot, Arc<Mutex<u32>>);

impl Bot for Checked {
    fn act(&mut self, obs: &Observation) -> Option<Action> {
        if let Some(b) = self.0.tree_betting(obs) {
            assert_eq!(b.pot() as u64, obs.pot, "pot");
            assert_eq!(b.current_bet as u64, obs.current_bet, "bet to call");
            let me = &obs.seats[obs.seat];
            assert_eq!(b.stack[b.to_act as usize] as u64, me.stack, "stack");
            *self.1.lock().unwrap() += 1;
        }
        self.0.act(obs)
    }

    fn hand_over(&mut self, summary: &HandSummary) {
        self.0.hand_over(summary);
    }
}

/// Six copies of the bot at the tree's own depth never leave the tree, and
/// the tree's chips match the table's at every decision.
fn self_play_stays_on_the_tree(player: &Arc<MultiwayPlayer>) {
    let counts = Arc::new(MwCounts::default());
    let checked = Arc::new(Mutex::new(0));
    let mut bots: Vec<Box<dyn Bot>> = (0..6)
        .map(|s| {
            let bot = MultiwayBot::new(player.clone(), s).with_counts(counts.clone());
            Box::new(Checked(bot, checked.clone())) as Box<dyn Bot>
        })
        .collect();
    let r = run_match(
        &MatchConfig::new(rules(player.hole_cards), 30, 5)
            .with_starting_stack(2 * BB as u64)
            .duplicate(),
        &mut bots,
    )
    .unwrap();
    assert_eq!(r.hands, 180);
    assert!(r.fallbacks.iter().all(|&f| f == 0), "{:?}", r.fallbacks);
    let decisions = counts.decisions.load(Relaxed);
    assert!(decisions > 180, "{decisions}");
    assert_eq!(counts.off_tree.load(Relaxed), 0);
    assert_eq!(*checked.lock().unwrap() as u64, decisions);
    assert!(r.net.iter().sum::<i64>() == 0);
}

/// Against bots betting any size, at depths other than the tree's, the bot
/// only ever acts legally, and counts what it couldn't follow.
fn plays_legal_against_anything(player: &Arc<MultiwayPlayer>) {
    for (i, stack) in [(0u64, 200u64), (1, 2 * BB as u64), (2, 333)] {
        let counts = Arc::new(MwCounts::default());
        let mut bots: Vec<Box<dyn Bot>> = vec![
            Box::new(MultiwayBot::new(player.clone(), 7).with_counts(counts.clone())),
            Box::new(RandomBot::new(Some(3 + i))),
            Box::new(CallingStation),
            Box::new(Personality::NikAirbag.bot(Some(4 + i))),
            Box::new(Personality::DougPoker.bot(Some(5 + i))),
            Box::new(MultiwayBot::new(player.clone(), 8).with_counts(counts.clone())),
        ];
        let r = run_match(
            &MatchConfig::new(rules(player.hole_cards), 20, 11 + i)
                .with_starting_stack(stack)
                .duplicate(),
            &mut bots,
        )
        .unwrap();
        assert!(r.fallbacks.iter().all(|&f| f == 0), "{:?}", r.fallbacks);
        let (decisions, off) = (
            counts.decisions.load(Relaxed),
            counts.off_tree.load(Relaxed),
        );
        assert!(decisions > 0 && off <= decisions);
        // Random bets and limps take some hands off the tree, not all.
        assert!(off > 0, "stack {stack}: never off the tree");
        assert!(off < decisions, "stack {stack}: always off the tree");
        let (by_decision, by_hand) = counts.rates();
        assert!(by_decision > 0.0 && by_decision < 1.0);
        assert!(by_hand > 0.0 && by_hand <= 1.0);
        assert!(counts.off_hands.load(Relaxed) <= counts.hands.load(Relaxed));
        let by_reason: u64 = counts.by_reason.iter().map(|c| c.load(Relaxed)).sum();
        assert_eq!(by_reason, off);
    }

    // Not six-handed: every decision falls back, legally, and is counted.
    let counts = Arc::new(MwCounts::default());
    let mut bots: Vec<Box<dyn Bot>> = vec![
        Box::new(MultiwayBot::new(player.clone(), 9).with_counts(counts.clone())),
        Box::new(CallingStation),
        Box::new(RandomBot::new(Some(2))),
    ];
    let r = run_match(
        &MatchConfig::new(rules(player.hole_cards), 20, 3),
        &mut bots,
    )
    .unwrap();
    assert_eq!(r.fallbacks[0], 0);
    let decisions = counts.decisions.load(Relaxed);
    assert!(decisions > 0);
    assert_eq!(counts.off_tree.load(Relaxed), decisions);
    assert_eq!(
        counts.by_reason[OffTree::Table as usize].load(Relaxed),
        decisions
    );
}

#[test]
fn holdem_blueprint_plays_on_the_tree_and_legally() {
    let player = holdem();
    assert_eq!(player.hole_cards, 2);
    assert!(!player.blueprint.is_empty());
    self_play_stays_on_the_tree(&player);
    plays_legal_against_anything(&player);
}

#[test]
fn plo4_blueprint_plays_on_the_tree_and_legally() {
    let player = plo::<4>();
    self_play_stays_on_the_tree(&player);
    plays_legal_against_anything(&player);
}

#[test]
fn plo6_blueprint_plays_on_the_tree_and_legally() {
    let player = plo::<6>();
    assert_eq!(player.hole_cards, 6);
    self_play_stays_on_the_tree(&player);
    plays_legal_against_anything(&player);
}

#[test]
fn settings_name_the_game_and_check_the_cards() {
    let s = Settings::parse("plo-lean 100 BB, 6 hole cards | PloAbstractionConfig { x }").unwrap();
    assert_eq!(s.menu, "plo-lean");
    assert_eq!(s.big_blinds, 100);
    assert_eq!(s.hole_cards, 6);
    assert_eq!(s.cards, "PloAbstractionConfig { x }");
    assert!(Settings::parse("test").is_none());

    // A blueprint loads only with the abstraction it was trained on.
    let cards = CardAbstraction::quick(4);
    let other = CardAbstraction::quick(5);
    let settings = format!("holdem-lean {BB} BB, 2 hole cards | {}", cards.settings());
    let mut bytes = Vec::new();
    MultiwayBlueprint::write(&settings, 0, |_| {}, &mut bytes).unwrap();
    assert!(MultiwayPlayer::load(&bytes, &cards.save()).is_ok());
    assert!(MultiwayPlayer::load(&bytes, &other.save()).is_err());
    let bad = settings.replace("holdem-lean", "no-such-menu");
    let mut bytes = Vec::new();
    MultiwayBlueprint::write(&bad, 0, |_| {}, &mut bytes).unwrap();
    assert!(MultiwayPlayer::load(&bytes, &cards.save()).is_err());
}
