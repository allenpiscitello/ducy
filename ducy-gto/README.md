# ducy-gto

Game-theory-optimal poker strategies with counterfactual regret minimization
(CFR), built on [ducy](../README.md). The goal is an example bot for
[ducy-play](../ducy-play/README.md) that plays heads-up no-limit Hold'em very
close to a Nash equilibrium. The plan is tracked in issue #84.

In a two-player zero-sum game an equilibrium strategy can't lose in
expectation to any opponent. How far a strategy is from one is its
**exploitability**: what a best-responding opponent wins against it. This
crate finds equilibria and measures exploitability exactly.

## What's here (steps 1–6 of 8)

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
- `holdem::river`: an exact river solver with safe re-solving, and
  `holdem::range`: ranges tracked through a hand, for solving the river in
  real time (see [below](#solving-the-river-in-real-time)).
- `holdem::review`: grading a player's decisions in a hand against the
  blueprint, in big blinds lost (see [below](#reviewing-hands-against-the-blueprint)).
- `holdem::turn`: a depth-limited turn solver whose leaves play out every
  river with the blueprint (see [below](#solving-the-turn-depth-limited)).
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

## Playing the blueprint: `GtoBot`

`holdem::bot::GtoBot` is a ducy-play `Bot`, so it sits at any table, in
`run_match`, or anywhere else a bot goes.

- **Following the hand:** at each decision it replays the hand's betting on the blueprint's tree.
  - Its own actions are on the tree already.
  - The opponent's off-menu bets are mapped to the neighboring menu sizes with
    the randomized **pseudo-harmonic mapping** (Ganzfried and Sandholm). Each
    mapping is drawn once per hand and kept.
- **Choosing an action:** it looks up its bucket, **samples** the blueprint's
  mixed strategy, and turns the chosen size back into chips as the same
  fraction of the real pot, clamped to the legal range. Sizes are pot
  fractions, so it works at any blinds; stacks other than the trained depth
  clamp to what's legal.
- **When the tree is all-in early:** a large real bet can be mapped to
  all-in while the real hand still has chips behind. The tree's plan is
  then to get every chip in, so the bot does.
- **More than two players, or a hand it can't follow:** a pot-odds fallback
  that never makes an illegal move. `off_tree` counts those decisions.
- **Stress test:** `cargo run --release -p ducy-gto --example gto_stress -- 50000`
  plays 100,000 duplicate hands against each of `RandomBot` (every bet size),
  `EquityBot` and a personality bot: **0 illegal actions, 0 off-tree
  decisions**.

## Shipping to the browser

The full card abstraction is 565 MB, almost all of it the turn table. But a
bucket is only "this hand's equity histogram, nearest cluster centre", so it
can be computed instead of looked up. Builds now keep the centres (file
format version 2), and `CardAbstraction::compact()` drops the tables:

- **Size:** **65 KB** instead of 565 MB (`build_abstraction` writes
  `<out>.compact` alongside the full file).
- **Same buckets:** it computes the same buckets as the tables. The build
  checks 2,000 random flop and turn hands, and the settings hash is
  unchanged, so a blueprint trained on the full table loads and plays the
  same on the compact one.
- **Cost:** about 13 ms per flop or turn lookup natively.
- **The build is deterministic:** a rebuild with the same settings assigns
  every hand identically (`COMPARE=old.bin` checks it), so older files can
  be upgraded by rebuilding.

Training keeps using the full tables for speed.

In ducy-wasm, `loadGto(cards, blueprint)` takes the two files (the compact
abstraction and the 6 MB blueprint, about 2.8 MB gzipped), after which `"gto"`
works as a bot id in `BotTable` and `MultiTable`. In Node, loading takes 14 ms,
and decisions average 6 ms (57 ms at worst, a flop bucket computed on the fly).
Table seats now accept any bot (`TableSeat::with_bot`), not only personality
bots.

## Solving the river in real time

The blueprint plays the river with 200 buckets and a betting tree whose chip
amounts can be off from the real hand's. Once the river card is out, the rest
of the hand is small enough to solve exactly while playing.
`GtoBot::with_river_solving(RiverSolving::new(200))` does that for every
river decision:

- **Ranges** (`holdem::range`): both players' ranges are tracked through the
  hand as the blueprint plays them. Each action multiplies every hand's
  weight by the blueprint's probability of that action for the hand's
  bucket. That takes every hand's bucket on each street
  (`CardAbstraction::buckets`): instant with the full tables, and with the
  compact abstraction one pass over the runouts builds every hand's equity
  histogram at once (about 0.2 s for a flop on one core, spread over the
  cores with the `parallel` feature, instead of 1,081 × 13 ms).
- **The subgame** (`holdem::river`) starts at the river with the real pot and
  stacks and the blueprint's post-flop sizes, plus any real bet already made
  on the river, at its real size.
- **The solver** runs Discounted CFR on vectors over the 1,081 hands the
  board allows, every hand kept separate. Showdowns and folds are valued in
  O(n) from hands sorted by strength, with running per-card sums to leave
  out opponent hands that share a card.
- **Safe re-solving:** a resolving gadget (Burch, Johanson and Bowling 2014)
  lets each opponent hand take, instead of playing the subgame, what a best
  response gets against the blueprint's own river strategy. At the gadget's
  equilibrium no opponent hand gains against the solution compared with the
  blueprint, so solving can't make the bot more exploitable on the river.
- **Off-tree bets:** when the opponent bets a size the solution doesn't
  have, the bot solves again with that size added, its own earlier river
  actions frozen at the strategy it played them with.

`cargo run --release -p ducy-gto --example river_solve` times it with random
ranges for both players (one core, 200 iterations):

| Board | Pot | Behind | Decisions | 100 iterations | 200 iterations | 1,000 iterations |
|---|---|---|---|---|---|---|
| Qs Td 7h 4c 2s | 20 | 190 | 28 | 0.53% (170 ms) | 0.21% (288 ms) | 0.018% (1.3 s) |
| Ah Kh 8d 8c 3h | 60 | 170 | 20 | 0.29% (122 ms) | 0.10% (215 ms) | 0.008% (1.1 s) |
| 9s 8s 7d 2c 2h | 120 | 140 | 12 | 0.09% (47 ms) | 0.04% (90 ms) | 0.003% (0.4 s) |
| Kc Jd 5s 4h 3d | 200 | 100 | 8 | 0.03% (28 ms) | 0.01% (56 ms) | 0.001% (0.3 s) |

Exploitability is in percent of the pot, with the time so far in brackets.
Deep stacks and a small pot leave the most betting, so they take longest.

The tests check it on known spots:

- **Polar range against a bluff catcher:** for a pot-sized bet, the polar
  player bluffs one hand for every two value bets, and the bluff catcher
  calls half the time; exploitability is under 0.05% of the pot.
- **Exploitability falls toward 0** with random ranges (under 0.25% of the
  pot after 200 iterations).
- **The gadget:** no opponent hand does better against the re-solved
  strategy than against the reference it was given.
- **Showdown and fold values** match brute force over every pair of hands,
  ties and shared cards included.

`gto_match --river-solve N` measures it with the real blueprint: a duplicate
match of the river-solving bot against the plain one (`--only self`), and
with `--lbr` a paired LBR comparison on the same deals.
`gto_stress -- DEALS N` checks legality: **0 illegal actions** against every
opponent with river solving on.

### Measuring it

Measuring river solving against the trained blueprint takes long runs, best
done on a local machine rather than a cloud session. The trained model is
in the `gto-hunl-300m` release (`cards.bin` is the compact abstraction):

```sh
gh release download gto-hunl-300m -p cards.bin -p blueprint.bin
# Paired LBR on the same deals: the blueprint, then river solving.
cargo run --release -p ducy-gto --example gto_match -- \
    --cards cards.bin --blueprint blueprint.bin --only self --deals 10 \
    --lbr 20000 --river-solve 200
# Duplicate match: the river-solving bot against the plain blueprint bot.
cargo run --release -p ducy-gto --example gto_match -- \
    --cards cards.bin --blueprint blueprint.bin --only self --deals 20000 \
    --river-solve 200
```

A short run so far (4 cores) is far too small to tell the two apart: LBR
wins −237 ± 1,082 mbb/hand against the blueprint and −251 ± 1,077 against
river solving over 2,000 hands, a paired change of −14 ± 293. With the
compact abstraction a river solve costs about 0.25 s in all (ranges, then
200 iterations), so the duplicate match plays about 7 hands a second.

## Reviewing hands against the blueprint

After a heads-up hand against `GtoBot`, `holdem::review` grades each of the
player's decisions against the blueprint, holding the same cards (#107):

- **Replay:** the hand is followed on the blueprint's tree exactly as
  `GtoBot` follows it (`holdem::follow`), and both players' ranges are
  tracked. A bet between two menu sizes counts as a mix of both, with the
  pseudo-harmonic mapping's weights.
- **Values:** at each decision, the expected value of every action on the
  menu for the player's actual hand against the bot's *range* (never its
  actual cards, so luck doesn't count), with both players following the
  blueprint afterwards. Turn and river decisions are exact (every river
  card), flop decisions average 32 sampled turn and river cards with a
  standard error, and preflop decisions are graded on the blueprint's mix
  alone (valuing them means sampling flops, which needs every hand's flop
  bucket: too slow with the compact abstraction).
- **Grades:**
  - the blueprint's main (most frequent) action is always **fine**;
  - an action it mixes in at least 5% of the time is **fine** too, with a
    note that it's the less common choice and what the main play is;
  - anything else is graded by the big blinds it gives up against the best
    action: **fine** under 0.25 bb (or within twice the sampling error),
    then **inaccuracy**, **mistake** from 1 bb and **blunder** from 4 bb;
  - preflop, a play the blueprint (almost) never makes is a **deviation**,
    with no cost attached.
- **Sessions:** `ReviewLog` keeps the finished hands so a player can play
  several and then review them all; `SessionReview` totals the big blinds
  lost (per 100 hands too), the grades and the costliest decisions, with
  the actual result kept apart.

In ducy-wasm, a `BotTable` heads-up against `"gto"` keeps each finished
hand: `reviewLastHand()`, `reviewSession()` (`{hands, summary}`),
`reviewableHands()` and `clearReviews()`. `ducy-wasm/tests/review.cjs`
plays and reviews hands under Node in CI, with the tiny model from
`cargo run -p ducy-gto --example tiny_model -- target/tiny-model`.

The tests check it against brute force (ranges, and turn, river and flop
values), on known spots (folding the nuts to a shove costs the 101 bb pot,
calling off with the worst hand costs 99 bb, checking the nuts gives up
value), that the replay reproduces the real pot at every decision, that the
same hand reviews the same way twice, and that `GtoBot` reviewing its own
play is charged nothing.

### Calibrating it

```sh
gh release download gto-hunl-300m -p cards.bin -p blueprint.bin
cargo run --release -p ducy-gto --example review_calibration -- \
    --cards cards.bin --blueprint blueprint.bin --hands 500
```

This plays `GtoBot` against itself, EquityBot, a calling station and every
personality, reviews the opponent's decisions, and prints the big blinds
charged per 100 hands and per decision, the grades, the actual result, and
the review time per decision on each street. `GtoBot` should be charged
about nothing, and the bots it beats most (`gto_match`) should be charged
the most.

#### Results

With the `gto-hunl-300m` model, 200 hands per opponent (`--hands 200`).
"Charged" is what the review says each opponent's decisions cost, in big
blinds. "GtoBot's match result" is what GtoBot actually won against the same
bot over 10,000 duplicate deals with `gto_match`, on the same model, ± its
95% interval:

| Opponent | Decisions | Charged (bb/100) | Charged (bb/decision) | Graded fine | Mistakes + blunders | GtoBot's match result (bb/100) |
|---|---:|---:|---:|---:|---:|---:|
| GtoBot itself | 558 | 15.4 | 0.055 | 99% | 3 | 2.5 ± 18.2 |
| calling station | 901 | 275.9 | 0.612 | 82% | 67 | 258.8 ± 49.2 |
| milk_king | 832 | 250.3 | 0.602 | 84% | 63 | 202.8 ± 44.0 |
| chris_moneybags | 700 | 195.3 | 0.558 | 86% | 41 | 162.8 ± 36.5 |
| nik_airbag | 382 | 191.6 | 1.003 | 71% | 46 | 116.0 ± 60.0 |
| mister_cheating | 461 | 179.7 | 0.780 | 83% | 35 | 92.9 ± 35.8 |
| rampart | 419 | 168.7 | 0.805 | 77% | 40 | 190.0 ± 51.6 |
| danny_smallball | 573 | 138.9 | 0.485 | 88% | 27 | 51.3 ± 27.2 |
| uncle_gary | 793 | 114.4 | 0.289 | 88% | 33 | 79.6 ± 34.1 |
| bungleman | 471 | 110.0 | 0.467 | 81% | 38 | 170.6 ± 41.5 |
| michael_miserable | 459 | 98.5 | 0.429 | 85% | 17 | 62.6 ± 26.2 |
| doug_poker | 454 | 79.4 | 0.350 | 85% | 21 | 64.0 ± 26.2 |
| EquityBot | 812 | 68.3 | 0.168 | 86% | 37 | 86.1 ± 33.7 |
| gus_bluffsen | 491 | 67.0 | 0.273 | 86% | 27 | 70.2 ± 27.3 |
| phil_bigmouth | 435 | 59.9 | 0.275 | 82% | 18 | 61.6 ± 23.3 |
| brad_owned | 598 | 51.1 | 0.171 | 89% | 27 | 38.6 ± 21.6 |
| lady_luck_linda | 474 | 25.2 | 0.106 | 84% | 9 | 83.5 ± 26.8 |
| old_man_coffee | 419 | 17.4 | 0.083 | 77% | 6 | 28.5 ± 11.2 |

What it shows:
- **GtoBot reviewing itself** is charged 0.055 bb per decision, with 99% of its decisions graded fine. Every bot is charged more per decision: from 1.5 times as much (old_man_coffee) to 18 times (nik_airbag).
  - Much of its 15.4 bb/100 comes from 3 blunders in 558 decisions, each at least 4 bb. Most likely these are rare branches of its mixed strategy that the review scores lower, because it values the rest of the hand by the blueprint. Those hands weren't inspected (`--only self --show N` prints them).
  - The cautious old_man_coffee is charged about as little per hand (17.4 bb/100), because it makes fewer decisions, but 1.5 times as much per decision (0.083).
- **The ranking matches the matches.** Across the 17 bots, the charge and GtoBot's match result agree with a rank correlation of 0.76 (Spearman) and a linear correlation of 0.82 (Pearson):
  - the bots charged most (the calling station, then milk_king) are the two GtoBot beats by the most;
  - the bot charged least, old_man_coffee, is the one it beats by the least.
- **Where they disagree,** the review charges more than the match shows (mister_cheating, danny_smallball, nik_airbag) or less (bungleman, lady_luck_linda). The causes:
  - 200 hands is a small sample;
  - the review charges each decision against GtoBot's continuation, not against what each bot goes on to do;
  - a bot can play badly in ways GtoBot doesn't fully exploit.
- **Luck stays out of it.** What the opponents actually won over these 200 hands ranged from −257 to +141 bb/100, unrelated to their play. That's why the review charges decisions, not results.
- **Speed:** on one core, a flop decision takes about 0.7 s to review, a turn decision about 0.09 s and a river decision under a millisecond.

## Solving the turn, depth-limited

Solving the turn exactly would mean solving all 48 rivers behind it as well.
`GtoBot::with_turn_solving(TurnSolving::new(N))` instead solves only the
turn's betting and values what comes after from the blueprint (Brown,
Sandholm and Amos 2018, the method of Modicum):

- **The subgame** (`holdem::turn`) is the turn's betting from the real pot
  and stacks, with the opponent's real bet sizes added, as on the river.
- **Leaves** are where the turn's betting closes. Each one deals every river
  card and plays the river out with a *continuation strategy*: the
  blueprint's river strategy carried over to the real chips. Showdowns and
  folds are valued in O(n) per river, as in the river solver.
- **The opponent picks a continuation** at each leaf, per hand: the
  blueprint's, or the blueprint with folds, calls or raises made five times
  as likely. So the turn strategy has to hold up whatever the opponent does
  on the river, rather than assume they keep playing the blueprint.
- **Safety:** the same resolving gadget as on the river, with targets from a
  best response to the blueprint's turn strategy.
- **Re-solving** after an off-tree bet freezes the bot's own earlier turn
  actions. With river solving on too, the river's ranges follow the turn
  solution instead of the blueprint.
- **Sampled rivers:** valuing a leaf deals all 48 rivers, the bulk of the
  cost. `TurnSolver::set_river_samples(k, seed)` deals `k` random rivers per
  iteration instead (8 by default in `TurnSolving`), an unbiased estimate, so
  it still converges; best responses always use every river.

`cargo run --release -p ducy-gto --example turn_solve -- 50 8` times it with
random ranges, uniform continuations and the default menu, 12 cores, 8
rivers per iteration:

| Board | Pot | Behind | Leaves | 25 iterations | 50 iterations | 50, opponent choosing |
|---|---|---|---|---|---|---|
| Qs Td 7h 4c | 20 | 190 | 27 | 2.6% (5.5 s) | 0.54% (10.6 s) | 0.71% (28.9 s) |
| Ah Kh 8d 8c | 60 | 170 | 19 | 1.6% (2.7 s) | 0.50% (5.8 s) | 0.48% (15.9 s) |
| Kc Jd 5s 4h | 120 | 140 | 11 | 0.46% (1.3 s) | 0.21% (2.4 s) | 0.28% (8.2 s) |

Exploitability is in the depth-limited game, in percent of the pot. Those
times are from before leaves were valued from per-bucket tables: as the bot
uses it (the real model, 200 river buckets, the opponent choosing, 8
rivers), an iteration now takes 60–70 ms, down from 180–330 ms
(`examples/turn_bench`).

To measure the bot, `gto_match --turn-solve N` (with `--river-solve`) runs
LBR against turn and river solving paired with river solving alone
(`lbr::lbr_hands_solving`), and adds a `river-solve` opponent for a
duplicate match of the two:

```sh
cargo run --release -p ducy-gto --example gto_match -- \
    --cards cards.bin --blueprint blueprint.bin --only river-solve \
    --deals 2000 --lbr 4000 --turn-solve 50 --river-solve 200
```

### What it's worth

Measured against river solving alone, with 50 turn and 200 river
iterations, the `gto-hunl-300m` model, and the LBR hands paired up to the
turn:

| 70,000 paired hands | LBR wins (mbb/hand, lower is better) |
|---|---|
| River solving only | 186 ± 164 |
| Turn and river solving | −11 ± 162 |
| **Change** | **−197 ± 97** |

So turn solving makes the bot clearly harder to exploit: the whole 95%
interval of the change is below zero. In a duplicate match of 3,000 deals
the two bots were even (−6.8 ± 24.9 bb/100), as two near-equilibrium
strategies should be. The cost is about 3–4 s a turn decision on 12 cores,
and about 150 MB.

So `GtoBot::with_solving()` turns on both, at `TurnSolving::default()` (50
iterations) and `RiverSolving::default()` (200). In the page (ducy-wasm),
the GTO bot solves both too, with much less turn work: WebAssembly runs on
one core, where 50 iterations take about 6.5 s a turn decision and 20 about
3.7 s, too slow to play against. The page's bot does 10 iterations of 4
rivers each, about a third of the time of 20 of 8, still with the
opponent's river styles. `setGtoSolving(turn, river)` changes the
iterations (0 plays the blueprint on that street), and
`setGtoTurnOptions(riverSamples, biases)` the rest.

The tests check that check-down leaf values match brute force over every
pair of hands and river, that exploitability falls toward 0 (with all rivers
and with 8 sampled), that a chooser's best response gains from its
continuations, that the gadget holds, and that the bot plays legally with
turn and river solving on.

## Heads-up pot-limit Omaha (#125)

The same pipeline for heads-up PLO. The abstract game is `Hunl` with four
hole cards (`holdem::hunl::HuPlo`) and pot-limit betting
(`HunlConfig::pot_limit_omaha`); what differs from Hold'em lives in `omaha`.

### Showdowns and equity (#129)

- **Showdowns** (`omaha::showdown`): an Omaha hand plays exactly two hole
  cards and three from the board, so it's the best of 60 five-card hands.
  `RiverBoard` keeps the board's ten triples and scores each with ducy's
  bit-twiddling scorer. Results match ducy's exact Omaha evaluator on 12,000
  random PLO4/5/6 deals (`tests/omaha.rs`).
- **Equity** (`equity_vs_random`): sampled against a random hand, a random
  runout and opponent per sample, with its standard error. That error is at
  most 0.5/√samples: ±1% at 2,500 samples, ±0.5% at 10,000. A test checks it
  against exact turn equity, enumerating every river and all 123,410
  opponent hands.
- **River tables** (`PairTable`): every two-card pair's best hand on one
  river, after which a hand's strength is 6 lookups.

`cargo run --release -p ducy-gto --example plo_bench`, 12 cores:

| | Speed |
|---|---|
| Showdown, two four-card hands (one core) | 2.3 µs (430,000/s) |
| Equity sample vs a random hand, preflop to turn (one core) | 2.4–2.7 µs |
| Equity sample on the river, after a 213 µs pair table (one core) | 0.23 µs |
| MCCFR, PLO 100 BB, every hand in one bucket (tree and showdowns only) | 13,000 iterations/s |
| MCCFR, 16 buckets from the top two ranks | 6,700 iterations/s |
| MCCFR, buckets from 25 equity samples per hand per street | 1,800 iterations/s |
| MCCFR, buckets from 100 equity samples | 1,000 iterations/s |

So the card abstraction can't sample equity during training: 25 samples
(±10%) already cost three quarters of the speed. It has to be computed from
cheap features (#128).

The pot-limit tree with `BetMenu::pot_limit` is large: 263,364 nodes and
84,428 river betting sequences, against Hold'em's 9,236. Pot-sized raises
take four or five rounds to get 100 BB in, where no-limit ends in an all-in.
Training (#130) needs a leaner menu.

## Next steps

1. **Done:** CFR engine, exact exploitability on Kuhn and Leduc (#85)
2. **Done:** Monte Carlo CFR with linear/discounted weighting, pruning and parallel training (#86)
3. **Done:** Card abstraction: suit isomorphism, equity-distribution buckets (#87)
4. **Done:** Action abstraction and the abstract heads-up no-limit tree (#88)
5. **Done:** Train and store the blueprint strategy (#89)
6. **Done:** `GtoBot`: play the blueprint in ducy-play, with action translation (#90)
7. Evaluation: Local Best Response and duplicate matches against the built-in bots (#91)
8. **Done:** Real-time subgame solving (#92): the river exactly, the turn
   depth-limited, both on by default
