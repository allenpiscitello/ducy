# ducy-gto

Game-theory-optimal poker strategies with counterfactual regret minimization
(CFR), built on [ducy](../README.md). The goal is an example bot for
[ducy-play](../ducy-play/README.md) that plays heads-up no-limit Hold'em very
close to a Nash equilibrium. The plan is tracked in issue #84.

In a two-player zero-sum game an equilibrium strategy can't lose in
expectation to any opponent. How far a strategy is from one is its
**exploitability**: what a best-responding opponent wins against it. This
crate finds equilibria and measures exploitability exactly.

## What's here (step 1 of 8)

- `Game`: the interface a game implements (chance, players, payoffs, information sets).
- `Cfr`: vanilla CFR and CFR+ over the whole game tree, for games small
  enough to enumerate.
- `exploitability`, `best_response_value`, `expected_value`: exact evaluation of a `Profile`.
- `games::kuhn` and `games::leduc`: Kuhn poker and Leduc hold'em, small games
  with known equilibrium values, for checking the solver.

```rust
use ducy_gto::{Cfr, Variant, exploitability, expected_value, games::leduc::Leduc};

let mut cfr = Cfr::new(&Leduc, Variant::Plus);
cfr.run(1000);
let strategy = cfr.average();
println!("exploitability {:.5}, value {:.4}", exploitability(&Leduc, &strategy), expected_value(&Leduc, &strategy));
```

## Convergence

`cargo run --release -p ducy-gto --example toy_games`. Exploitability is in
milli-chips per hand (the ante is 1 chip). The value is player 0's, against
equilibrium values of −1/18 ≈ −0.0556 for Kuhn and −0.0856 for Leduc.

| Game | Variant | Iterations | Exploitability | Value | Time |
|---|---|---|---|---|---|
| Kuhn (12 infosets) | CFR | 10,000 | 2.318 | −0.0555 | 0.05 s |
| Kuhn | CFR+ | 1,000 | 0.087 | −0.0556 | 0.01 s |
| Leduc (288 infosets) | CFR | 3,000 | 19.716 | −0.0885 | 3.0 s |
| Leduc | CFR+ | 1,000 | 0.255 | −0.0856 | 2.0 s |
| Leduc | CFR+ | 3,000 | 0.035 | −0.0856 | 5.8 s |

## Next steps

1. **This:** CFR engine, exact exploitability on Kuhn and Leduc (#85)
2. Monte Carlo CFR with linear/discounted weighting, pruning and parallel training (#86)
3. Card abstraction: suit isomorphism, equity-distribution buckets (#87)
4. Action abstraction and the abstract heads-up no-limit tree (#88)
5. Train and store the blueprint strategy (#89)
6. `GtoBot`: play the blueprint in ducy-play, with action translation (#90)
7. Evaluation: Local Best Response and duplicate matches against the built-in bots (#91)
8. Real-time depth-limited subgame solving (#92)
