use criterion::{Criterion, criterion_group, criterion_main};
use ducy::deck::Deck;
use ducy::ranking::hand_rank::StandardHandRanker;

fn rank_5_card_hand(c: &mut Criterion) {
    let deck = Deck::parse("As Ks Qs Js 9s").unwrap();
    c.bench_function("rank_5_card_flush", |b| {
        b.iter(|| StandardHandRanker::get_rank(&deck))
    });
}

fn rank_7_card_hand(c: &mut Criterion) {
    let deck = Deck::parse("As Ks Qs Js 9s 2d 3c").unwrap();
    c.bench_function("rank_7_card_hand", |b| {
        b.iter(|| StandardHandRanker::get_rank(&deck))
    });
}

fn rank_5_card_straight(c: &mut Criterion) {
    let deck = Deck::parse("As Kd Qh Jc Ts").unwrap();
    c.bench_function("rank_5_card_straight", |b| {
        b.iter(|| StandardHandRanker::get_rank(&deck))
    });
}

fn rank_5_card_high_card(c: &mut Criterion) {
    let deck = Deck::parse("As Kd Qh Jc 9s").unwrap();
    c.bench_function("rank_5_card_high_card", |b| {
        b.iter(|| StandardHandRanker::get_rank(&deck))
    });
}

criterion_group!(
    benches,
    rank_5_card_hand,
    rank_7_card_hand,
    rank_5_card_straight,
    rank_5_card_high_card,
);
criterion_main!(benches);
