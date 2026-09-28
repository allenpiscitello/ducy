use criterion::{Criterion, criterion_group, criterion_main};
use ducy::deck::Deck;

fn enumerate_combinations_of_2_from_47(c: &mut Criterion) {
    let deck = Deck::all_cards() - Deck::parse("As Ac Ks Kd Kc").unwrap();
    c.bench_function("combinations_C(47,2)", |b| {
        b.iter(|| {
            let mut count = 0u32;
            for _ in deck.enumerate_combinations(2) {
                count += 1;
            }
            count
        })
    });
}

fn enumerate_combinations_of_5_from_10(c: &mut Criterion) {
    let deck = Deck::parse("As Ks Qs Js Ts 9s 8s 7s 6s 5s").unwrap();
    c.bench_function("combinations_C(10,5)", |b| {
        b.iter(|| {
            let mut count = 0u32;
            for _ in deck.enumerate_combinations(5) {
                count += 1;
            }
            count
        })
    });
}

fn iterate_full_deck(c: &mut Criterion) {
    let deck = Deck::all_cards();
    c.bench_function("iterate_52_cards", |b| {
        b.iter(|| {
            let mut count = 0u32;
            for _ in deck.iter(true) {
                count += 1;
            }
            count
        })
    });
}

criterion_group!(
    benches,
    enumerate_combinations_of_2_from_47,
    enumerate_combinations_of_5_from_10,
    iterate_full_deck,
);
criterion_main!(benches);
