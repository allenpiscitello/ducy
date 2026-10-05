use ducy_gto::{
    Config, Discount, Mccfr, Prune, exploitability,
    games::{kuhn::Kuhn, leduc::Leduc},
};

fn config(discount: Discount) -> Config {
    Config {
        seed: 3,
        batch: 64,
        discount,
        prune: None,
    }
}

#[test]
fn kuhn_converges_with_every_discount() {
    for d in [
        Discount::None,
        Discount::Linear { until: u64::MAX },
        Discount::DCFR,
    ] {
        let mut m = Mccfr::new(&Kuhn, config(d));
        m.run(200_000);
        assert_eq!(m.num_infosets(), 12);
        let e = exploitability(&Kuhn, &m.average());
        assert!(e < 0.005, "{d:?}: exploitability {e}");
    }
}

#[test]
fn leduc_exploitability_falls_with_training() {
    let mut m = Mccfr::new(&Leduc, config(Discount::DCFR));
    m.run(20_000);
    let early = exploitability(&Leduc, &m.average());
    m.run(300_000);
    let late = exploitability(&Leduc, &m.average());
    assert_eq!(m.num_infosets(), 288);
    assert!(late < early / 2.0, "{early} -> {late}");
    assert!(late < 0.06, "exploitability {late}");
}

#[test]
fn same_seed_same_run() {
    let mut a = Mccfr::new(&Leduc, config(Discount::DCFR));
    let mut b = Mccfr::new(&Leduc, config(Discount::DCFR));
    a.run(5_000);
    b.run(5_000);
    assert_eq!(a.save(), b.save());
    let mut c = Mccfr::new(
        &Leduc,
        Config {
            seed: 4,
            ..config(Discount::DCFR)
        },
    );
    c.run(5_000);
    assert_ne!(a.save(), c.save());
}

#[cfg(feature = "parallel")]
#[test]
fn thread_count_does_not_change_the_result() {
    let run = |threads| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                let mut m = Mccfr::new(
                    &Leduc,
                    Config {
                        batch: 256,
                        ..config(Discount::DCFR)
                    },
                );
                m.run(10_000);
                m.save()
            })
    };
    assert_eq!(run(1), run(4));
}

#[test]
fn resuming_from_a_checkpoint_matches_an_unbroken_run() {
    let prune = Some(Prune {
        after: 2_000,
        threshold: -5.0,
        full_every: 10,
    });
    let cfg = Config {
        prune,
        ..config(Discount::DCFR)
    };
    let mut whole = Mccfr::new(&Leduc, cfg);
    whole.run(8_000);

    let mut first = Mccfr::new(&Leduc, cfg);
    first.run(3_200);
    let bytes = first.save();
    let mut resumed = Mccfr::load(&Leduc, cfg, &bytes).expect("valid checkpoint");
    assert_eq!(resumed.iterations(), 3_200);
    resumed.run(4_800);
    assert_eq!(resumed.save(), whole.save());

    // The wrong seed, or garbage, is refused.
    assert!(Mccfr::load(&Leduc, Config { seed: 99, ..cfg }, &bytes).is_none());
    assert!(Mccfr::load(&Leduc, cfg, &bytes[..bytes.len() - 1]).is_none());
    assert!(Mccfr::load(&Leduc, cfg, b"nonsense").is_none());
}

#[test]
fn pruning_skips_bad_actions_but_still_converges() {
    let prune = Some(Prune {
        after: 10_000,
        threshold: -10.0,
        full_every: 20,
    });
    let mut m = Mccfr::new(
        &Leduc,
        Config {
            prune,
            ..config(Discount::DCFR)
        },
    );
    m.run(200_000);
    let e = exploitability(&Leduc, &m.average());
    assert!(e < 0.08, "exploitability {e}");
}
