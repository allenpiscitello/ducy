use ducy_gto::{
    Config, Discount, Mccfr,
    holdem::{
        blueprint::{Blueprint, BlueprintError},
        cards::{Card, rank},
        hunl::{Buckets, HuPlo, Hunl, HunlConfig},
    },
    sampled::sampled_exploitability,
};

/// A stand-in abstraction, cheap even in a debug build: preflop by the top
/// card's rank, after the flop by how many board ranks the hand pairs.
struct Coarse;

impl Buckets for Coarse {
    fn hand_bucket(&self, hole: &[Card], board: &[Card]) -> u16 {
        if board.is_empty() {
            return hole.iter().map(|&c| rank(c) as u16).max().unwrap() / 4;
        }
        let hits = board
            .iter()
            .filter(|&&b| hole.iter().any(|&h| rank(h) == rank(b)))
            .count();
        hits.min(2) as u16
    }

    fn bucket_count(&self, board_len: usize) -> usize {
        if board_len == 0 { 4 } else { 3 }
    }

    fn settings(&self) -> String {
        "coarse".into()
    }
}

fn train<'a>(game: &'a HuPlo<'a, Coarse>, iterations: u64) -> Mccfr<'a, HuPlo<'a, Coarse>> {
    let mut m = Mccfr::new(
        game,
        Config {
            seed: 3,
            batch: 256,
            discount: Discount::DCFR,
            prune: None,
        },
    );
    m.run(iterations);
    m
}

#[test]
fn a_tiny_plo_game_gets_less_exploitable_with_training() {
    // Six big blinds deep with the lean pot-limit menu.
    let game = HuPlo::with_cards(HunlConfig::pot_limit_omaha_lean(6), Some(&Coarse));
    let measure = |m: &Mccfr<HuPlo<Coarse>>| {
        let s = |i: &u64, n: usize| m.average_at(i).unwrap_or_else(|| vec![1.0 / n as f64; n]);
        sampled_exploitability(&game, &s, 3000, 3000, 5)
    };
    let early = measure(&train(&game, 300));
    let late = measure(&train(&game, 30_000));
    assert!(
        late.value + 3.0 * late.std_error < early.value - 3.0 * early.std_error,
        "300 iterations: {early:?}; 30,000: {late:?}"
    );
}

#[test]
fn plo_blueprints_save_load_and_check_their_settings() {
    let config = HunlConfig::pot_limit_omaha_lean(6);
    let game = HuPlo::with_cards(config.clone(), Some(&Coarse));
    let m = train(&game, 2000);
    let strategy = |info: u64, n: usize| {
        m.average_at(&info)
            .unwrap_or_else(|| vec![1.0 / n as f64; n])
    };
    let bp = Blueprint::from_strategy(&game, &Coarse, strategy);
    let bytes = bp.save();
    let loaded = Blueprint::load(&bytes, &game, &Coarse).expect("loads");
    assert_eq!(loaded, bp);
    // The stored strategy is the solver's, to within a byte's rounding.
    let root_bucket = 3u64;
    let want = strategy(root_bucket, game.tree.nodes[0].actions.len());
    let got = loaded.probs(0, root_bucket as u16);
    for (w, g) in want.iter().zip(&got) {
        assert!((w - g).abs() < 0.01, "{want:?} vs {got:?}");
    }
    // Another depth, or Hold'em with the same betting, is refused.
    let deeper = HuPlo::with_cards(HunlConfig::pot_limit_omaha_lean(8), Some(&Coarse));
    assert_eq!(
        Blueprint::load(&bytes, &deeper, &Coarse),
        Err(BlueprintError::WrongSettings)
    );
    let holdem = Hunl::<Coarse, 2>::with_cards(config, Some(&Coarse));
    assert_eq!(
        Blueprint::load(&bytes, &holdem, &Coarse),
        Err(BlueprintError::WrongSettings)
    );
}
