use ducy_gto::{
    Profile,
    holdem::{
        abstraction::CardAbstraction,
        blueprint::Blueprint,
        hunl::{Hunl, HunlAction, HunlConfig},
        lbr::local_best_response,
    },
};

/// A blueprint that folds whenever it can, and otherwise checks or calls.
fn always_fold(game: &Hunl, cards: &CardAbstraction) -> Blueprint {
    let mut p = Profile::new();
    for (id, n) in game.tree.nodes.iter().enumerate() {
        if n.actions.is_empty() {
            continue;
        }
        let pick = n
            .actions
            .iter()
            .position(|a| *a == HunlAction::Fold)
            .unwrap_or_else(|| {
                n.actions
                    .iter()
                    .position(|a| matches!(a, HunlAction::Check | HunlAction::Call))
                    .unwrap()
            });
        let mut probs = vec![0.0; n.actions.len()];
        probs[pick] = 1.0;
        for b in 0..cards.num_buckets([0, 3, 4, 5][n.betting.street]) {
            p.set((id as u64) << 16 | b as u64, probs.clone());
        }
    }
    Blueprint::from_profile(game, cards, &p)
}

#[test]
fn lbr_exploits_a_strategy_that_always_folds() {
    let cards = CardAbstraction::quick(6);
    let game = Hunl::new(HunlConfig::default(), Some(&cards));
    let bp = always_fold(&game, &cards);
    let r = local_best_response(&game, &cards, &bp, 40, 3);
    // As the button LBR raises and the big blind folds (+1 big blind); as
    // the big blind, the button folds at once (+0.5). 750 mbb per hand.
    assert!((r.mbb_per_hand - 750.0).abs() < 1e-6, "{r:?}");
}

#[test]
fn lbr_beats_a_random_strategy() {
    let cards = CardAbstraction::quick(6);
    let game = Hunl::new(HunlConfig::default(), Some(&cards));
    let uniform = Blueprint::from_profile(&game, &cards, &Profile::new());
    // Each hand has its own seeded stream, so this is the same run every
    // time; stacks are 100 big blinds deep, so the spread is wide.
    let r = local_best_response(&game, &cards, &uniform, 200, 4);
    println!("{r:?}");
    assert!(r.mbb_per_hand > 1000.0, "{r:?}");
}
