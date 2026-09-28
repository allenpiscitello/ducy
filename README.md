# ducy

A performant analysis library for poker, supporting Texas Hold'em and Omaha.

## Features

- **Bitfield-based deck representation** — cards and decks are stored as `u64` bitfields for fast set operations
- **Hand ranking** — standard poker hand evaluation with `Ord`-based comparison
- **Winner evaluation** — determine winners for a given board state
- **Equity calculation** — enumerate all possible runouts to compute each player's equity
- **Range support** — build weighted hand ranges using notation like `"AQo+"` and `"AJs+"`
- **Board analysis** — flush texture analysis for Omaha boards

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
println!("Winner: player {}", winners[0].player_index);
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

## Minimum supported Rust version

1.85.0 (edition 2024)

## License

MIT
