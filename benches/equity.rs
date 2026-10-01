use criterion::{Criterion, criterion_group, criterion_main};
use ducy::deck::{Card, Deck};
use ducy::games::flop_game::FlopGame;
use ducy::games::holdem::{HoldemGameEvaluation, HoldemGameState, HoldemRange};
use ducy::games::omaha::{OmahaGameEvaluation, OmahaGameState};
use ducy::games::omaha_hilo::{OmahaHiLoGameEvaluation, OmahaHiLoGameState};
use ducy::games::{GameEquityEvaluation, GameEvaluation};

fn holdem_turn_equity(c: &mut Criterion) {
    let mut game = HoldemGameState::new();
    game.add_player(Deck::parse("As Ac").unwrap()).unwrap();
    game.add_player(Deck::parse("Ks Kd").unwrap()).unwrap();
    game.set_flop(Deck::parse("Kc Qd Js").unwrap()).unwrap();
    game.set_turn(Card::parse("Tc").unwrap()).unwrap();

    let evaluator = HoldemGameEvaluation {};

    c.bench_function("holdem_turn_equity", |b| {
        b.iter(|| evaluator.evaluate_equity(&game))
    });
}

fn holdem_flop_equity(c: &mut Criterion) {
    let mut game = HoldemGameState::new();
    game.add_player(Deck::parse("As Ac").unwrap()).unwrap();
    game.add_player(Deck::parse("Ks Kd").unwrap()).unwrap();
    game.set_flop(Deck::parse("Kc Qd Js").unwrap()).unwrap();

    let evaluator = HoldemGameEvaluation {};

    c.bench_function("holdem_flop_equity", |b| {
        b.iter(|| evaluator.evaluate_equity(&game))
    });
}

fn holdem_turn_evaluate_winners(c: &mut Criterion) {
    let mut game = HoldemGameState::new();
    game.add_player(Deck::parse("As Ac").unwrap()).unwrap();
    game.add_player(Deck::parse("Ks Kd").unwrap()).unwrap();
    game.set_flop(Deck::parse("Kc Qd Js").unwrap()).unwrap();
    game.set_turn(Card::parse("Tc").unwrap()).unwrap();
    game.set_river(Card::parse("2h").unwrap()).unwrap();

    let evaluator = HoldemGameEvaluation {};

    c.bench_function("holdem_evaluate_winners", |b| {
        b.iter(|| evaluator.evaluate_winners(&game))
    });
}

fn omaha_turn_equity(c: &mut Criterion) {
    let mut game = OmahaGameState::new(4);
    game.add_player(Deck::parse("As Ac Jc Ts").unwrap())
        .unwrap();
    game.add_player(Deck::parse("9h 8h 7d 6d").unwrap())
        .unwrap();
    game.set_flop(Deck::parse("Jh Th Qd").unwrap()).unwrap();
    game.set_turn(Card::parse("Jd").unwrap()).unwrap();

    let evaluator = OmahaGameEvaluation {};

    c.bench_function("omaha_turn_equity", |b| {
        b.iter(|| evaluator.evaluate_equity(&game))
    });
}

fn omaha_flop_equity(c: &mut Criterion) {
    let mut game = OmahaGameState::new(4);
    game.add_player(Deck::parse("As Ac Jc Ts").unwrap())
        .unwrap();
    game.add_player(Deck::parse("9h 8h 7d 6d").unwrap())
        .unwrap();
    game.set_flop(Deck::parse("Jh Th Qd").unwrap()).unwrap();

    let evaluator = OmahaGameEvaluation {};

    c.bench_function("omaha_flop_equity", |b| {
        b.iter(|| evaluator.evaluate_equity(&game))
    });
}

fn omaha_hilo_flop_equity(c: &mut Criterion) {
    let mut game = OmahaHiLoGameState::new(4);
    game.add_player(Deck::parse("As 2d Kc Kd").unwrap())
        .unwrap();
    game.add_player(Deck::parse("Ah 3h 4c Qs").unwrap())
        .unwrap();
    game.set_flop(Deck::parse("5h 6c Jd").unwrap()).unwrap();

    let evaluator = OmahaHiLoGameEvaluation {};

    c.bench_function("omaha_hilo_flop_equity", |b| {
        b.iter(|| evaluator.evaluate_equity(&game))
    });
}

fn holdem_range_sample_preflop(c: &mut Criterion) {
    let game = HoldemGameState::new();
    let ranges = [
        HoldemRange::parse("QQ+, AK").unwrap(),
        HoldemRange::parse("22+, ATs+, KQs").unwrap(),
    ];
    let evaluator = HoldemGameEvaluation {};

    c.bench_function("holdem_range_sample_preflop_10k", |b| {
        b.iter(|| evaluator.sample_range_equity(&game, &ranges, 10_000))
    });
}

fn holdem_range_exact_turn(c: &mut Criterion) {
    let mut game = HoldemGameState::new();
    game.set_flop(Deck::parse("Kc 8d 3s").unwrap()).unwrap();
    game.set_turn(Card::parse("2h").unwrap()).unwrap();
    let ranges = [
        HoldemRange::parse("QQ+, AK").unwrap(),
        HoldemRange::parse("88-TT, KQs").unwrap(),
    ];
    let evaluator = HoldemGameEvaluation {};

    c.bench_function("holdem_range_exact_turn", |b| {
        b.iter(|| evaluator.range_equity(&game, &ranges))
    });
}

criterion_group!(
    benches,
    holdem_range_sample_preflop,
    holdem_range_exact_turn,
    holdem_turn_evaluate_winners,
    holdem_turn_equity,
    holdem_flop_equity,
    omaha_turn_equity,
    omaha_flop_equity,
    omaha_hilo_flop_equity,
);
criterion_main!(benches);
