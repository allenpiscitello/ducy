use ducy_gto::{
    Cfr, Game, Variant, best_response_value,
    games::{kuhn::Kuhn, leduc::Leduc},
    sampled::sampled_best_response,
};

/// On a game small enough for exact best responses, the sampled one finds
/// the same values (every information set is reached by many deals, so the
/// fitted policy is the true best response) within sampling error.
fn agrees_with_exact<G>(game: &G, iterations: u64, deals: usize)
where
    G: Game + Sync,
    G::State: Send + Sync,
    G::Info: Send + Sync,
{
    let mut cfr = Cfr::new(game, Variant::Plus);
    cfr.run(iterations);
    let profile = cfr.average();
    let strategy = |i: &G::Info, n: usize| profile.probs(i, n);
    for br in 0..2 {
        let exact = best_response_value(game, &profile, br);
        let s = sampled_best_response(game, &strategy, br, deals, deals, 7 + br as u64);
        assert!(
            (s.value - exact).abs() < 4.0 * s.std_error + 0.01,
            "player {br}: sampled {s:?}, exact {exact}"
        );
    }
}

#[test]
fn matches_exact_best_responses_on_kuhn() {
    // A barely trained strategy, so best responses win a lot.
    agrees_with_exact(&Kuhn, 3, 20_000);
}

#[test]
fn matches_exact_best_responses_on_leduc() {
    // Chance also moves mid-hand here (the board card).
    agrees_with_exact(&Leduc, 5, 20_000);
}
