# ducy-gto

Game-theory-optimal poker strategies with counterfactual regret minimization
(CFR), built on [ducy](../README.md). The goal is an example bot for
[ducy-play](../ducy-play/README.md) that plays heads-up no-limit Hold'em very
close to a Nash equilibrium. The plan is tracked in issue #84.

In a two-player zero-sum game an equilibrium strategy can't lose in
expectation to any opponent. How far a strategy is from one is its
**exploitability**: what a best-responding opponent wins against it. This
crate finds equilibria and measures exploitability exactly.

## What's here (steps 1–5 of 8)

- `Game`: the interface a game implements (chance, players, payoffs, information sets).
- `Cfr`: vanilla CFR and CFR+ over the whole game tree, for games small
  enough to enumerate.
- `Mccfr`: Monte Carlo CFR with external sampling, for games too big to walk
  in full. Each iteration explores all of the traverser's actions and samples
  chance and the opponent. On top of that:
  - **Discounting:** `Discount::Linear` (Linear CFR) and `Discount::DCFR`
    (DCFR, α = 1.5, β = 0, γ = 2).
  - **Regret-based pruning** (`Prune`), with periodic full passes.
  - **Parallel batches with rayon:** reproducible for a seed and batch size,
    whatever the thread count.
  - **Compact storage:** `f32` regrets and strategy sums in flat arrays.
  - **Checkpoints:** `save` / `load`. A resumed run is identical to an unbroken one.
- `exploitability`, `best_response_value`, `expected_value`: exact evaluation of a `Profile`.
- `holdem::abstraction::CardAbstraction`: maps hole cards and a board to a bucket per street:
  - **Preflop:** 169 lossless classes.
  - **Flop and turn:** k-means under earth mover's distance on each hand's
    river-equity histogram over all runouts. Hands are canonicalized by suit
    isomorphism first.
  - **River:** equal-population buckets of equity against a random hand.
- `holdem::hunl::Hunl`: the abstract heads-up no-limit game, with a
  configurable bet menu (`BetMenu`) and the card abstraction for information
  sets. Its betting follows ducy-play's rules exactly (fuzz-tested against
  `ducy_play::Hand`).
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

## Monte Carlo CFR

`cargo run --release -p ducy-gto --example mccfr_leduc`, on 4 cores, batches
of 256, exploitability in milli-chips per hand:

| Iterations | Plain | Linear | DCFR | DCFR + pruning |
|---|---|---|---|---|
| 10,000 | 456.4 | 387.5 | 277.6 | 277.6 |
| 100,000 | 81.2 | 73.4 | 51.7 | 51.6 |
| 1,000,000 | 21.8 | 25.6 | 18.7 | 17.0 |
| 4,000,000 (≈17 s) | 10.8 | 11.2 | 10.4 | 8.8 |

On a game as small as Leduc, full-tree CFR+ is far better (0.26 in 2 s): a
full walk costs barely more than a sample. Sampling pays off in big games,
where one iteration costs the same however large the tree is.

`cargo bench -p ducy-gto`: about 95,000 Leduc iterations per second on one
core (batches of 16) and 273,000 with 4 cores (batches of 1,024), with 18.7
bytes of regrets and strategy sums per information set.

## Card abstraction

`cargo run --release -p ducy-gto --example build_abstraction -- abstraction.bin`
builds the default abstraction (169 / 200 / 200 / 200 buckets) and checks it.
It's one pass over every runout of all 1,755 distinct flops (2.06 million
river boards) to sample equity histograms and fit the clusters, then a second
pass to assign all 1,286,792 canonical flop hands and 55,190,538 canonical
turn hands. Those are the known counts of distinct hands up to suit
isomorphism, and the build checks it finds exactly that many.

The speed comes from scoring each river board once for every hand
(`river_equities`): score the 1,081 hands the board allows, sort them, and
count each hand's wins and ties with binary searches, correcting for shared
cards. That takes about 120 µs per board, against 17 µs per hand (18 ms per
board) one hand at a time.

On 4 cores (default config):

| Stage | Time |
|---|---|
| Sample equity histograms (every runout of every flop) | 79 s |
| Cluster flop hands (k-means, EMD, 200 clusters) | 22 s |
| Cluster turn hands | 12 s |
| Assign all flop and turn hands (second full pass) | 160 s |
| Sort the tables | 3 s |
| **Total** | **4.6 min** |

The file is 565 MB. Sanity checks from the build:
- Isomorphic hands share a bucket.
- A set, a flush draw and air land in different flop buckets.
- On the river, the royal flush is in the top bucket (199), the second-nut
  flush in bucket 196, and a busted hand in bucket 0.

The turn table is most of the file: an 8-byte canonical key and a 2-byte
bucket per hand. A dense hand index (Waugh's hand isomorphism) would drop the
keys and shrink it about tenfold, if the size becomes a problem.

## The abstract game

With the default bet menu, the betting tree has these information sets with
the default buckets:

| Street | Betting sequences | Information sets | Actions |
|---|---|---|---|
| Preflop | 16 | 2,704 | 45 per bucket |
| Flop | 180 | 36,000 | 512 per bucket |
| Turn | 1,532 | 306,400 | 4,264 per bucket |
| River | 9,236 | 1,847,200 | 24,976 per bucket |

That's about 2.2 million information sets and 48 MB of `f32` regrets and
strategy sums, small enough to train on one machine.

## Training the blueprint

```sh
cargo run --release -p ducy-gto --example build_abstraction -- abstraction.bin
cargo run --release -p ducy-gto --example train_blueprint -- \
    --cards abstraction.bin --out blueprint.bin --checkpoint checkpoint.bin --iters 300000000
```

- **Training:** DCFR-discounted external-sampling MCCFR on the default abstract
  game (100 big blinds, the default bet menu, 169/200/200/200 buckets).
- **Progress:** every `--every` iterations it prints the speed and the preflop
  headline numbers (button open, limp and fold rates; big blind defend and
  3-bet rates vs an open), and saves the checkpoint. A stopped run resumes from
  the checkpoint exactly.
- **Speed:** on a 12-core machine, about 28,000 iterations per second. All 2.2
  million information sets are reached within a few million iterations.
- **Output:** the blueprint (`holdem::blueprint::Blueprint`), with one byte per
  action and about 6 MB for the default game. It records a hash of the game
  and abstraction settings and refuses to load against anything else.
- **Chart:** the run ends by printing the button's opening chart and the big
  blind's defending chart (`holdem::chart`).

The betting tree is compiled once (`BettingTree`): a hand in training is the
deal plus a node number, and an information set is `node << 16 | bucket`.

For tests there's `CardAbstraction::quick`, which buckets flop and turn hands
by current hand strength instead of a precomputed table, so it needs no
build. A 10 big blind shove/limp/fold game trains in seconds on it and learns
sensible ranges (`tests/blueprint.rs`).

## Next steps

1. **Done:** CFR engine, exact exploitability on Kuhn and Leduc (#85)
2. **Done:** Monte Carlo CFR with linear/discounted weighting, pruning and parallel training (#86)
3. **Done:** Card abstraction: suit isomorphism, equity-distribution buckets (#87)
4. **Done:** Action abstraction and the abstract heads-up no-limit tree (#88)
5. **Done:** Train and store the blueprint strategy (#89)
6. `GtoBot`: play the blueprint in ducy-play, with action translation (#90)
7. Evaluation: Local Best Response and duplicate matches against the built-in bots (#91)
8. Real-time depth-limited subgame solving (#92)
