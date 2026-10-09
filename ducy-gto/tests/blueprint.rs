use ducy_gto::{
    Config, Discount, Mccfr, Profile,
    holdem::{
        abstraction::CardAbstraction,
        blueprint::{Blueprint, BlueprintError, settings_hash},
        chart::{PreflopReport, class_name, class_of, combos},
        hunl::{BetMenu, Hunl, HunlConfig, Size},
    },
};

#[test]
fn chart_classes() {
    assert_eq!(class_name(class_of(12, 12, false)), "AA");
    assert_eq!(class_name(class_of(12, 11, true)), "AKs");
    assert_eq!(class_name(class_of(5, 0, false)), "72o");
    let total: f64 = (0..169).map(combos).sum();
    assert_eq!(total, 1326.0);
}

#[test]
fn blueprint_round_trip_and_settings_check() {
    let cards = CardAbstraction::quick(8);
    let game = Hunl::new(HunlConfig::default(), Some(&cards));
    // A made-up strategy at the root: AA always raises small, 72o folds.
    let mut profile = Profile::new();
    let n = game.tree.nodes[0].actions.len();
    let mut raise = vec![0.0; n];
    raise[2] = 1.0;
    let mut fold = vec![0.0; n];
    fold[0] = 1.0;
    profile.set(class_of(12, 12, false) as u64, raise.clone());
    profile.set(class_of(5, 0, false) as u64, fold.clone());
    profile.set(7, vec![0.2, 0.3, 0.5, 0.0]);
    let bp = Blueprint::from_profile(&game, &cards, &profile);
    assert_eq!(bp.probs(0, class_of(12, 12, false) as u16), raise);
    assert_eq!(bp.probs(0, class_of(5, 0, false) as u16), fold);
    // Bytes: probabilities survive to within 1/255, and still sum to 1.
    let p = bp.probs(0, 7);
    assert!((p[0] - 0.2).abs() < 0.005 && (p[2] - 0.5).abs() < 0.005);
    assert!((p.iter().sum::<f64>() - 1.0).abs() < 1e-9);
    // Unseen information sets play uniformly.
    let u = bp.probs(0, 50);
    assert!(u.iter().all(|&x| (x - 0.25).abs() < 0.01));

    let bytes = bp.save();
    assert_eq!(Blueprint::load(&bytes, &game, &cards).unwrap(), bp);
    assert_eq!(
        Blueprint::load(&bytes[..bytes.len() - 1], &game, &cards),
        Err(BlueprintError::Corrupt)
    );
    assert_eq!(
        Blueprint::load(b"junk", &game, &cards),
        Err(BlueprintError::Corrupt)
    );
    // A different stack, menu or abstraction is refused.
    let deeper = Hunl::new(
        HunlConfig {
            stack: 400,
            ..HunlConfig::default()
        },
        Some(&cards),
    );
    assert_eq!(
        Blueprint::load(&bytes, &deeper, &cards),
        Err(BlueprintError::WrongSettings)
    );
    let other = CardAbstraction::quick(9);
    let game9 = Hunl::new(HunlConfig::default(), Some(&other));
    assert_eq!(
        Blueprint::load(&bytes, &game9, &other),
        Err(BlueprintError::WrongSettings)
    );
}

#[test]
fn published_blueprints_keep_their_settings_hash() {
    // The hash a blueprint records for the default game, as it was before
    // pot-limit betting was added (#188 changed it by accident, so blueprints
    // trained earlier stopped loading). It must never change for an
    // existing no-limit game.
    let cards = CardAbstraction::quick(8);
    let game = Hunl::new(HunlConfig::default(), Some(&cards));
    assert_eq!(settings_hash(&game, &cards), 0xea93_5854_3de6_a8a4);
    // A pot-limit game hashes differently, so neither loads as the other.
    let plo = Hunl::new(
        HunlConfig {
            pot_limit: true,
            ..HunlConfig::default()
        },
        Some(&cards),
    );
    assert_ne!(settings_hash(&plo, &cards), settings_hash(&game, &cards));
}

/// Shove, limp or fold with 10 big blinds: small enough to train in
/// seconds. The button always plays strong hands (shoving or trapping with
/// a limp), folds trash more often, and the big blind calls a shove with
/// aces.
#[test]
fn push_fold_training_learns_sensible_ranges() {
    let cards = CardAbstraction::quick(4);
    let config = HunlConfig {
        small_blind: 1,
        big_blind: 2,
        stack: 20,
        menu: BetMenu {
            preflop: vec![vec![Size::AllIn]],
            postflop: vec![vec![Size::AllIn]],
        },
        pot_limit: false,
    };
    let game = Hunl::new(config, Some(&cards));
    let mut m = Mccfr::new(
        &game,
        Config {
            seed: 5,
            batch: 512,
            discount: Discount::DCFR,
            prune: None,
        },
    );
    m.run(if cfg!(debug_assertions) {
        10_000
    } else {
        200_000
    });
    let report = PreflopReport::read(&game, |n, b| {
        let k = game.tree.nodes[n as usize].actions.len();
        m.average_at(&((n as u64) << 16 | b as u64))
            .unwrap_or(vec![1.0 / k as f64; k])
    });
    let plays = |c: usize| 1.0 - report.button.fold(c);
    let aa = class_of(12, 12, false);
    let kk = class_of(11, 11, false);
    let seven_two = class_of(5, 0, false);
    println!("{}", report.summary());
    assert!(
        plays(aa) > 0.97 && plays(kk) > 0.97,
        "AA plays {}, KK plays {}",
        plays(aa),
        plays(kk)
    );
    assert!(
        report.button.fold(seven_two) > report.button.fold(aa) + 0.2,
        "72o folds {}",
        report.button.fold(seven_two)
    );
    let vs = report.vs_open.expect("the big blind faces a shove");
    // A short run still carries some early noise in the average.
    assert!(vs.fold(aa) < 0.05, "AA calls a shove");
    assert!(vs.fold(seven_two) > 0.5, "72o folds to a shove");
}
