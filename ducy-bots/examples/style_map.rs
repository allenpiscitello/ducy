//! Measures each personality's style for the bot style map: VPIP, PFR and
//! aggression factor over six-handed No-limit Hold'em hands against mixed
//! tables of the other personalities. Prints CSV.
//!
//! cargo run --release -p ducy-bots --example style_map [hands]

use ducy_bots::Personality;
use ducy_bots::stats::OpponentModel;
use ducy_play::{Bot, Deal, Hand, TableRules, play_hand};

fn measure(p: Personality, hands: usize) -> (f64, f64, f64) {
    let rules = TableRules::no_limit_holdem(1, 2);
    let all = Personality::ALL;
    let others: Vec<Personality> = all.iter().copied().filter(|&o| o != p).collect();
    let n = 6;
    let mut model = OpponentModel::new();
    let mut me = p.bot(Some(7));
    let per_table = 500;
    for block in 0..hands.div_ceil(per_table) {
        // A new mix of five opponents every block.
        let mut table: Vec<Box<dyn Bot>> = (0..n - 1)
            .map(|i| {
                let o = others[(block * (n - 1) + i) % others.len()];
                Box::new(o.bot(Some((block * n + i) as u64))) as Box<dyn Bot>
            })
            .collect();
        for h in 0..per_table {
            let id = (block * per_table + h) as u64;
            let deal = Deal::random(rules.variant, n, Some(id)).unwrap();
            let mut hand = Hand::new(rules, &vec![200; n], h % n, deal).unwrap();
            let mut seated: Vec<&mut dyn Bot> = vec![&mut me];
            for bot in table.iter_mut() {
                seated.push(&mut **bot);
            }
            play_hand(&mut hand, &mut seated).unwrap();
            model.record(hand.events(), n);
        }
    }
    let s = model.seat(0);
    (
        s.vpip_hands as f64 / s.hands as f64,
        s.pfr_hands as f64 / s.hands as f64,
        s.aggressive as f64 / s.calls.max(1) as f64,
    )
}

fn main() {
    let hands: usize = std::env::args()
        .nth(1)
        .map_or(6_000, |s| s.parse().expect("hands"));
    let rows: Vec<_> = std::thread::scope(|scope| {
        let handles: Vec<_> = Personality::ALL
            .into_iter()
            .map(|p| scope.spawn(move || (p, measure(p, hands))))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    println!("name,hands,vpip,pfr,af");
    for (p, (vpip, pfr, af)) in rows {
        println!("{},{hands},{vpip:.4},{pfr:.4},{af:.3}", p.name());
    }
}
