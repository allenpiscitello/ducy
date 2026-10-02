# ducy

A performant analysis library for poker, supporting Texas Hold'em and Omaha.

## Features

- **Bitfield-based deck representation** — cards and decks are stored as `u64` bitfields for fast set operations
- **Hand ranking** — standard poker hand evaluation with `Ord`-based comparison
- **Winner evaluation** — determine winners for a given board state
- **Equity calculation** — enumerate all possible runouts to compute each player's equity
- **Range support** — build weighted hand ranges using notation like `"AQo+"` and `"AJs+"`
- **Omaha ranges** — PPT-style hand classes (`AAxx$ds`, `$rd0-1`, `TT$3rd`) with preflop coverage, for PLO, PLO5 and PLO6
- **Board analysis** — flush texture analysis for Omaha boards
- **Playing hands** — the [`ducy-play`](ducy-play/README.md) crate plays out Hold'em and Omaha hands: blinds, no-limit and pot-limit betting, side pots and showdown

## Quick start

Add ducy to your `Cargo.toml`:

```toml
[dependencies]
ducy = "0.2.0"
```

### Evaluate a Hold'em hand

```rust
use ducy::deck::Deck;
use ducy::games::flop_game::FlopGame;
use ducy::games::holdem::{HoldemGameEvaluation, HoldemGameState};
use ducy::games::GameEvaluation;

let mut game = HoldemGameState::new();
game.add_player(Deck::parse("As Ac").unwrap()).unwrap();
game.add_player(Deck::parse("Ks Kd").unwrap()).unwrap();
game.set_flop(Deck::parse("Kc Qd Js").unwrap()).unwrap();

let evaluator = HoldemGameEvaluation {};
let winners = evaluator.evaluate_winners(&game);
println!("Winner: player {}", winners[0].player_index());
```

### Evaluate an Omaha hand

```rust
use ducy::deck::{Card, Deck};
use ducy::games::flop_game::FlopGame;
use ducy::games::omaha::{OmahaGameEvaluation, OmahaGameState};
use ducy::games::{GameEvaluation, GameEquityEvaluation};

let mut game = OmahaGameState::new(4);
game.add_player(Deck::parse("As Ac Jc Ts").unwrap()).unwrap();
game.add_player(Deck::parse("9h 8h 7d 6d").unwrap()).unwrap();
game.set_flop(Deck::parse("Jh Th Qd").unwrap()).unwrap();
game.set_turn(Card::parse("Jd").unwrap()).unwrap();

let evaluator = OmahaGameEvaluation {};
let equity = evaluator.evaluate_equity(&game);
println!("Player 0 equity: {:.1}%", equity[0] * rust_decimal::Decimal::from(100));
```

### Omaha ranges and preflop coverage

```rust
use ducy::games::omaha_range::OmahaRange;

let range = OmahaRange::parse("AAxx$ds, $rd$ds, TT$3rd", 4).unwrap();
println!("{} hands, {:.2}% of all PLO hands", range.combos(), 100.0 * range.coverage());
```

A range is a list of terms separated by commas or spaces; a hand is in the
range if it matches any term. A term is a card pattern followed by any number
of `$` conditions, all of which must hold.

**Card pattern** (up to the number of hole cards; missing cards are wildcards)

| Token | Meaning |
|---|---|
| `A` `K` `Q` `J` `T` `9`…`2` | a card of that rank; `AA` means two or more aces |
| `As`, `Td`, … | that exact card |
| `x` or `*` | any card |

**Conditions**

| Condition | Meaning |
|---|---|
| `$ts` | triple-suited: three suits with 2+ cards each (PLO6) |
| `$ds` | double-suited: exactly two suits with 2+ cards each |
| `$ss` | single-suited: exactly one suit with 2+ cards |
| `$r` | rainbow |
| `$np` / `$1p` / `$2p` | no pair / exactly one pair (no trips) / two or more pairs |
| `$rd` | rundown: all cards different ranks and consecutive (`JT98`) |
| `$rdG` | rundown with exactly `G` gaps (`$rd1`: `T986`; `$rd2`: `T975`) |
| `$rdG-H` | rundown with `G` to `H` gaps (`$rd0-1`) |
| `$Nrd`, `$NrdG`, `$NrdG-H` | contains an `N`-card rundown (`$3rd`: `TT98`, `KT98`) |
| `$!cond` | negation (`$3rd$!rd`: a 3-card rundown that isn't a full rundown) |

Gaps are the ranks missing inside the run, and the ace plays high or low
(`A234` and `AKQJ` are rundowns). `$ts`, `$ds`, `$ss` and `$r` split all hands
into disjoint classes. `coverage()` is the share of all starting hands in the
range (270,725 for PLO); `weighted_coverage()` counts hands by weight. See
the `OmahaRange` docs for more examples.

## Minimum supported Rust version

1.85.0 (edition 2024)

## License

MIT
