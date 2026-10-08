//! How long one party's turn takes: locking and shuffling 52 cards, and the
//! lock removals one party does during setup at a 9-handed table.
//!
//! cargo run --release -p ducy-shuffle --example shuffle_timing

use std::time::Instant;

use ducy_shuffle::{Layout, Secret, open_deck, shuffle_round};

fn main() {
    let runs = 200;
    let mut rng: rand::rngs::StdRng = rand::make_rng();
    let secret = Secret::random();
    let deck = open_deck();
    let start = Instant::now();
    for _ in 0..runs {
        std::hint::black_box(shuffle_round(&deck, &secret, &mut rng));
    }
    let shuffle_ms = start.elapsed().as_secs_f64() * 1000.0 / runs as f64;

    // A player at a 9-handed Hold'em table removes their lock from 16 other
    // hole cards and the 5 board cards.
    let layout = Layout::new(9, 2, 5);
    let mine = (0..layout.board().end)
        .filter(|&p| layout.unlockers(p).contains(&0))
        .count();
    let start = Instant::now();
    for _ in 0..runs {
        for m in deck.iter().take(mine) {
            std::hint::black_box(secret.unlock(m));
        }
    }
    let unlock_ms = start.elapsed().as_secs_f64() * 1000.0 / runs as f64;
    println!("lock and shuffle 52 cards: {shuffle_ms:.2} ms");
    println!("setup lock removals for one player at 9 players ({mine} cards): {unlock_ms:.2} ms");
}
