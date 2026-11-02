use ducy_bots::traits::{
    MeasureConfig, Trait, drift, measure, measure_counts, out_of_tolerance, targets,
};
use ducy_bots::{Personality, PersonalityBot};

/// A personality with fewer Monte Carlo samples, to keep tests quick.
fn quick(p: Personality, seed: u64) -> PersonalityBot {
    let mut style = p.style();
    style.samples = 40;
    PersonalityBot::new(p.name(), style, Some(seed))
}

/// A few hundred hands: the counts add up, and characters far apart on a
/// trait already measure apart.
#[test]
fn traits_tell_characters_apart() {
    let config = MeasureConfig {
        opponent_samples: Some(40),
        ..MeasureConfig::new(300, 3)
    };
    let measured = |p: Personality| {
        let mut bot = quick(p, 1);
        measure_counts(&mut bot, Some(p), &config)
    };
    let jonny = measured(Personality::JonnySlow);
    let airbag = measured(Personality::NikAirbag);
    for c in [&jonny, &airbag] {
        assert_eq!(c.hands, 300);
        assert!(c.pfr <= c.vpip && c.vpip <= c.hands, "{c:?}");
        assert!(c.three_bets <= c.three_bet_chances && c.check_raises <= c.check_raise_chances);
        assert!(c.folds_to_bets <= c.faced_bets && c.showdowns <= c.saw_flop);
        assert!(c.bluffs <= c.aggressive.iter().sum::<u32>());
    }
    let (j, a) = (jonny.traits(), airbag.traits());
    for t in [Trait::Vpip, Trait::Pfr, Trait::Aggression] {
        assert!(j.get(t) < a.get(t), "{t:?}: {j:?} vs {a:?}");
    }
    assert!(j.get(Trait::FoldToBet) > a.get(Trait::FoldToBet));
    // Measuring is repeatable.
    let mut again = quick(Personality::JonnySlow, 1);
    assert_eq!(
        measure_counts(&mut again, Some(Personality::JonnySlow), &config),
        jonny
    );
}

/// Every personality as it plays now is within tolerance of its own
/// targets, on a sample other than the one they were measured from. Takes a
/// few minutes in release:
/// cargo test --release -p ducy-play --test traits -- --ignored
#[test]
#[ignore = "30,000 hands per personality; run in release"]
fn personalities_stay_within_tolerance_of_their_targets() {
    let config = MeasureConfig::new(30_000, 2);
    let rows: Vec<_> = std::thread::scope(|scope| {
        let handles: Vec<_> = Personality::ALL
            .into_iter()
            .map(|p| {
                scope.spawn(move || {
                    let mut bot = p.bot(Some(7));
                    (p, measure(&mut bot, Some(p), &config))
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut bad = Vec::new();
    for (p, t) in rows {
        let out = out_of_tolerance(p, &t);
        if !out.is_empty() || drift(p, &t) >= 1.0 {
            bad.push(format!(
                "{}: drift {:.2}, {out:?}, target {:?}",
                p.name(),
                drift(p, &t),
                targets(p)
            ));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
