# ducy-play

Play out poker hands on top of [ducy](../README.md): antes and blinds, betting
rounds, side pots and the showdown.

- **Games:** Texas Hold'em and Omaha (high) with 4, 5 or 6 hole cards
- **Betting:** no-limit or pot-limit for either game
- **Cards:** shuffled from a seed (reproducible) or supplied exactly, e.g. to replay a hand
- **History:** every post, action, board card and award is recorded as an `Event`
- **Bots:** a `Bot` trait, simple built-in bots, a match runner with stack resets and duplicate deals, and `ProcessBot` for bots written in any language

## Example

```rust
use ducy_play::{Action, Deal, Hand, TableRules};

let rules = TableRules::pot_limit_omaha(1, 2);
let deal = Deal::random(rules.variant, 3, Some(42)).unwrap();
let mut hand = Hand::new(rules, &[200, 200, 200], 0, deal).unwrap();

// Button raises pot, then everyone calls or checks down.
let legal = hand.legal_actions().unwrap();
hand.act(Action::Raise(legal.raise.unwrap().max_to)).unwrap();
while let Some(legal) = hand.legal_actions() {
    hand.act(if legal.can_check { Action::Check } else { Action::Call }).unwrap();
}

let result = hand.result().unwrap();
for pot in &result.pots {
    println!("{} chips to {:?} ({:?})", pot.amount, pot.awards, pot.winning_hand);
}
```

## Bots

A bot is anything that implements `Bot`: given an `Observation` (its own
hole cards plus public information, never anyone else's cards) it returns an
`Action`.

```rust
use ducy_play::{Action, Bot, Observation};

struct PotOddsCaller;

impl Bot for PotOddsCaller {
    fn act(&mut self, obs: &Observation) -> Option<Action> {
        let legal = &obs.legal;
        Some(match legal.call {
            None => Action::Check,
            Some(call) if call * 3 <= obs.pot => Action::Call,
            Some(_) => Action::Fold,
        })
    }
}
```

**Respond if possible.** Returning `None` or an illegal action never breaks
the game: the bot checks if it can and folds otherwise, and the fallback is
counted so you can spot broken bots.

| Built-in bot | Plays |
|---|---|
| `CallingStation` | always checks or calls |
| `Raiser` | bets or raises the minimum whenever it can |
| `RandomBot` | random legal actions |
| `EquityBot` | estimates equity against random hands; bets about the pot when strong, calls with pot odds, otherwise checks or folds |

### Running hands and matches

- `play_hand(&mut hand, &mut [&mut bot_a, &mut bot_b])` plays one hand,
  bot `i` in seat `i`, and calls every bot's `hand_over` with a
  `HandSummary` afterwards.
- `run_match(&config, &mut bots)` plays many hands. Every seat starts every
  hand with `starting_stack` (no carry-over, so results don't depend on
  earlier hands) and the button moves each deal. With `duplicate()`, each
  deal is replayed with the bots rotated through every seat, so luck of the
  cards mostly cancels out. Results are chips and `bb_per_100` per bot.

```rust
use ducy_play::bots::{CallingStation, EquityBot};
use ducy_play::{Bot, MatchConfig, TableRules, run_match};

let config = MatchConfig::new(TableRules::no_limit_holdem(1, 2), 20, 7).duplicate();
let mut bots: Vec<Box<dyn Bot>> = vec![
    Box::new(EquityBot::new(50, Some(1))),
    Box::new(CallingStation),
];
let result = run_match(&config, &mut bots).unwrap();
println!("equity bot: {:+.1} bb/100", result.bb_per_100(0));
```

`cargo run --release -p ducy-play --example bot_match` runs the built-in bots
against each other.

### Bots in other languages

`ProcessBot` runs a bot as its own program (any language) and talks to it
with one JSON object per line over stdin/stdout:

| Direction | Message |
|---|---|
| to the bot | `{"type": "act", "id": 7, "observation": {...}}`: answer with one line |
| from the bot | `{"id": 7, "action": "raise", "amount": 12}`: `action` is `fold`, `check`, `call`, `bet`, `raise` or `all_in`; `bet` and `raise` need `amount` (street total). `id` is optional but lets late replies be ignored. |
| to the bot | `{"type": "hand_over", "summary": {...}}` after every hand; no reply |

If the program doesn't answer within the timeout (5 seconds by default,
`with_timeout` to change), answers with something unreadable, or exits, it
checks or folds and the game continues. Anything it prints to stderr is
passed through for debugging.

An observation looks like this (cards are strings such as `"As"`):

```json
{
  "seat": 0,
  "hole_cards": "As Ah",
  "rules": { "variant": "holdem", "structure": "no_limit",
             "small_blind": 1, "big_blind": 2, "ante": 0 },
  "street": "preflop",
  "board": [],
  "button": 0,
  "pot": 3,
  "current_bet": 2,
  "seats": [{ "stack": 100, "street_bet": 0, "contributed": 0,
              "folded": false, "all_in": false }, ...],
  "legal": { "seat": 0, "can_fold": true, "can_check": false, "call": 2,
             "bet": null, "raise": { "min_to": 4, "max_to": 100 } },
  "history": [{ "type": "small_blind", "seat": 1, "amount": 1 }, ...]
}
```

For Omaha, `variant` is `{"omaha": {"hole_cards": 4}}`. The summary has
`seat`, `result` (pots, payouts, final stacks, net), `shown` (hole cards
revealed at showdown, by seat), `board` and `history`.

[`examples/bots/simple_bot.py`](examples/bots/simple_bot.py) is a complete
bot in about 40 lines of Python:

```sh
cargo run --release -p ducy-play --example bot_match -- python3 ducy-play/examples/bots/simple_bot.py
```

Features: `serde` (JSON serialization) and `process` (`ProcessBot`, which
needs `serde`) are on by default. `ducy-play-wasm` turns them off.

## How a hand works

1. `Hand::new(rules, stacks, button, deal)` posts antes and blinds and finds
   the first player to act.
2. `legal_actions()` describes what that player may do: fold, check, the call
   amount, and the min/max **total** for a bet or raise.
3. `act(action)` plays it. Bets and raises are "to" amounts for the street.
   `Action::AllIn` is a shortcut for whichever call, bet or raise puts every
   chip in.
4. Streets advance automatically. When no more betting is possible (everyone
   left is all-in) the rest of the board is dealt.
5. `result()` gives the pots (main pot first), winners, payouts, final stacks
   and each seat's net result.

## Rules

| Rule | Behavior |
|---|---|
| Blinds | Small blind left of the button, big blind next. Heads-up the button posts the small blind. |
| Antes | Posted by everyone before the blinds; they go in the pot but don't count toward calling. |
| Action order | Preflop: left of the big blind (heads-up: the button). Later streets: first live seat left of the button. |
| Minimum bet | The big blind. |
| Minimum raise | The size of the previous bet or raise on the street. A smaller raise is only allowed all-in. |
| Short all-in raise | Doesn't reopen the betting: players who already acted may only call or fold. |
| Pot limit | Max raise is to the pot size after calling: `current bet + pot + amount to call`. |
| Folding | Only when facing a bet. |
| Short blinds | A player who can't cover a blind posts what they have; others still call the full big blind. |
| Side pots | Built from each player's total contribution. Folded chips stay in the pots they reached. |
| Uncalled chips | Chips nobody matched form a pot only their owner can win, so they come back. |
| Split pots | Even split; leftover chips go one at a time to winners starting left of the button. |
| Omaha | Exactly two hole cards and three board cards. |

Not yet supported: fixed-limit betting, Hi-Lo split games, stud and draw
games, straddles, and the rule where several short all-ins that add up to a
full raise reopen the betting.
