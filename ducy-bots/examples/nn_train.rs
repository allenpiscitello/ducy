//! Stage-1 training (#216): a network that copies the rule-based
//! personalities, from a dataset `nn_dataset` wrote.
//!
//!     cargo run --release -p ducy-bots --example nn_train -- \
//!         --data imitation.bin --out net.bin --epochs 12 --traits 4000
//!
//! The last `--val` share of the decisions (default 5%, whole tables since
//! the file is in table order) is held out. Each epoch prints the training
//! and held-out loss and accuracy; the network with the lowest held-out loss
//! is kept. Then the held-out accuracy per class and per personality, and
//! with `--traits N` each personality played by the network for N Hold'em
//! hands (`traits::measure`), its drift from the rule-based bot's targets
//! (under 1.0 is within tolerance on average).
//!
//! Options: `--hidden 64,64`, `--batch 256`, `--lr 0.001`, `--cw 0` (how
//! much rare classes are weighted up), `--seed 1`.

use std::{sync::Arc, time::Instant};

use ducy_bots::{
    net::{
        ActionClass, Mlp, NUM_CLASSES, NetBot,
        train::{TrainConfig, Trainer, class_counts},
    },
    net_data::read_records,
    personality::Personality,
    traits::{MeasureConfig, drift, measure, out_of_tolerance},
};

fn main() {
    let mut data_path = String::new();
    let mut out = "net.bin".to_string();
    let mut epochs = 10usize;
    let mut val = 0.05f64;
    let mut hidden = vec![64usize, 64];
    let mut config = TrainConfig::default();
    let mut traits = 0usize;
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut v = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--data" => data_path = v(),
            "--out" => out = v(),
            "--epochs" => epochs = v().parse().expect("--epochs"),
            "--val" => val = v().parse().expect("--val"),
            "--hidden" => {
                hidden = v()
                    .split(',')
                    .map(|s| s.parse().expect("--hidden"))
                    .collect()
            }
            "--batch" => config.batch = v().parse().expect("--batch"),
            "--lr" => config.lr = v().parse().expect("--lr"),
            "--cw" => config.class_weight_power = v().parse().expect("--cw"),
            "--seed" => config.seed = v().parse().expect("--seed"),
            "--traits" => traits = v().parse().expect("--traits"),
            f => panic!("unknown option {f}"),
        }
    }
    let t0 = Instant::now();
    let bytes = std::fs::read(&data_path).expect("read --data");
    let data = read_records(&bytes).expect("a dataset");
    drop(bytes);
    let cut = data.len() - ((data.len() as f64 * val) as usize).min(data.len() - 1);
    let (train, held) = data.split_at(cut);
    let counts = class_counts(train);
    println!(
        "{} decisions: {} to train on, {} held out ({:.1}s to load)",
        data.len(),
        train.len(),
        held.len(),
        t0.elapsed().as_secs_f64()
    );

    let mut sizes = vec![ducy_bots::features::NUM_FEATURES];
    sizes.extend(&hidden);
    sizes.push(NUM_CLASSES);
    let net = Mlp::random(&sizes, config.seed);
    println!("network {sizes:?}: {} weights", net.num_params());
    let mut trainer = Trainer::new(net, config, &counts);
    let w = trainer.class_weights();
    println!(
        "class weights: {}",
        ActionClass::ALL
            .map(|c| format!("{c:?} {:.2}", w[c.index()]))
            .join(", ")
    );

    let mut best = (f64::INFINITY, trainer.net.clone());
    println!(
        "{:>5} {:>10} {:>9} {:>10} {:>9} {:>8}",
        "epoch", "train loss", "train acc", "held loss", "held acc", "secs"
    );
    for e in 1..=epochs {
        let t = Instant::now();
        let tr = trainer.epoch(train);
        let he = trainer.evaluate(held);
        println!(
            "{e:>5} {:>10.4} {:>8.1}% {:>10.4} {:>8.1}% {:>8.1}",
            tr.loss,
            100.0 * tr.accuracy,
            he.loss,
            100.0 * he.accuracy,
            t.elapsed().as_secs_f64()
        );
        if he.loss < best.0 {
            best = (he.loss, trainer.net.clone());
        }
    }
    trainer.net = best.1;
    std::fs::write(&out, trainer.net.save()).expect("write --out");
    println!("kept held-out loss {:.4}; wrote {out}", best.0);

    // Held out: per class, then per personality.
    let he = trainer.evaluate(held);
    println!("\nheld-out recall by class (decisions):");
    for c in ActionClass::ALL {
        let (_, n) = he.per_class[c.index()];
        println!(
            "  {:<10} {:>6.1}% ({n})",
            format!("{c:?}"),
            100.0 * he.recall(c.index())
        );
    }
    println!("\nheld-out accuracy by personality:");
    for (i, p) in Personality::ALL.iter().enumerate() {
        let mine: Vec<_> = held
            .iter()
            .filter(|r| r.personality as usize == i)
            .cloned()
            .collect();
        if mine.is_empty() {
            continue;
        }
        let m = trainer.evaluate(&mine);
        println!(
            "  {:<20} {:>6.1}%  loss {:.3} ({} decisions)",
            p.name(),
            100.0 * m.accuracy,
            m.loss,
            m.n
        );
    }

    if traits > 0 {
        println!(
            "\ntraits over {traits} Hold'em hands each (drift from the rule-based bot's targets; < 1 within tolerance):"
        );
        let net = Arc::new(trainer.net.clone());
        for (i, p) in Personality::ALL.into_iter().enumerate() {
            let t = Instant::now();
            let mut bot = NetBot::new(p, net.clone(), Some(i as u64 + 1));
            let measured = measure(&mut bot, Some(p), &MeasureConfig::new(traits, 1));
            let off: Vec<String> = out_of_tolerance(p, &measured)
                .into_iter()
                .map(|(tr, v)| format!("{} {v:.2}", tr.name()))
                .collect();
            println!(
                "  {:<20} drift {:.2}{} ({:.0}s)",
                p.name(),
                drift(p, &measured),
                if off.is_empty() {
                    String::new()
                } else {
                    format!("  off: {}", off.join(", "))
                },
                t.elapsed().as_secs_f64()
            );
        }
    }
}
