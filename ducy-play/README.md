# ducy-play

Play out poker hands on top of [ducy](https://crates.io/crates/ducy): antes and blinds, betting
rounds, side pots and the showdown.

- **Games:** Texas Hold'em and Omaha (high) with 4, 5 or 6 hole cards
- **Betting:** no-limit or pot-limit for either game
- **Cards:** shuffled from a seed (reproducible) or supplied exactly, e.g. to replay a hand
- **History:** every post, action, board card and award is recorded as an `Event`
- **Bots:** a `Bot` trait, simple built-in bots, 15 personality bots (Doug Poker, Phil Bigmouth, Rampart, Milk King, ...), a match runner with stack resets and duplicate deals, and `ProcessBot` for bots written in any language
- **Tables:** `Table` keeps seats over many hands: the button moves, players sit out and come back posting missed blinds (or wait for the big blind), and bots play their seats
- **Hosting:** `TableHost` runs a table for remote players, with commands in and per-player views out (no one sees another's cards before showdown), turn clocks, chip requests and sit-out limits, and snapshots to resume a hand after a restart
- **Tournaments:** `Tournament` runs several tables with blind levels, eliminations, table balancing and hand-for-hand play

## Install

```toml
[dependencies]
ducy-play = "0.1"
```

Features: `process` (default) adds `ProcessBot` and `serde`; `serde` alone
serializes rules, actions, events, views and snapshots.

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

### Personalities

Ready-made characters, each a `PersonalityBot` playing a `Style`. Any
resemblance to real players is purely coincidental. They are tuned for
**no-limit Hold'em**: they also play Omaha legally, but Omaha needs very
different hand selection, so their Omaha play isn't tuned yet. Measured over
1,500 no-limit Hold'em hands at 6-handed tables (VPIP = hands played,
PFR = hands raised preflop). Their range traits are written for a full
9-handed table and widen automatically at shorter tables and in position
(see *Table size and position* below):

| Personality | Plays | VPIP / PFR | Folds to bets |
|---|---|---|---|
| **Doug Poker** | Balanced, near-GTO: solid positional ranges, 2/3-pot bets with about one bluff per two value bets, pot-odds defense. An approximation, not a solver. | 23% / 16% | 28% |
| **Old Man Coffee** | A rock: few hands, limps and check-calls rather than raising, folds to pressure, never bluffs. | 11% / 3% | 58% |
| **Mister Cheating** | Loose-aggressive and exploitative: lots of hands, bold plays, adapts to each opponent's leaks. | 44% / 33% | 21% |
| **Milk King** | Loose-passive: plays most hands, rarely raises, calls far too much. | 60% / 3% | 5% |
| **Phil Bigmouth** | Tight and proud until a big loss puts him on tilt; then he loosens up and spews for a while. | 17% / 12% | 35% |
| **Rampart** | Splashy loose-aggressive vlogger: lots of hands, big bluffs, hero calls, bolder on a heater. | 44% / 34% | 9% |
| **Danny Smallball** | Many hands, small pots, 1/3-pot bets, sticky calls in position. | 36% / 22% | 27% |
| **Chris Moneybags** | Lucky amateur: loose-passive, gets bolder with every pot he wins. | 55% / 7% | 11% |
| **Uncle Gary** | Loose-passive, and never folds a pair. | 47% / 3% | 18% |
| **Gus Bluffsen** | Fearless bluffer: bets his air and checks his monsters. | 33% / 23% | 22% |
| **Lady Luck Linda** | Plays any suited hand and any ace, because they're pretty. | 37% / 6% | 33% |
| **Michael Miserable** | Disciplined, relentless grinder who never looks happy about it. | 23% / 18% | 31% |
| **Brad Owned** | Fit-or-fold recreational: sees lots of flops, bets what he hits, gives up when he misses. Always plays pocket jacks. | 33% / 5% | 70% |
| **Nik Airbag** | Maniac: raises almost everything, 3-bets wide, bluffs huge. | 65% / 46% | 12% |
| **Bungleman** | Wild card: plays any suited hand, traps one hand and fires huge bluffs the next. | 55% / 29% | 16% |

```rust
use ducy_play::{Bot, MatchConfig, Personality, TableRules, run_match};

let config = MatchConfig::new(TableRules::no_limit_holdem(1, 2), 5, 1).duplicate();
let mut bots: Vec<Box<dyn Bot>> = vec![
    Box::new(Personality::DougPoker.bot(Some(1))),
    Box::new(Personality::from_name("milk_king").unwrap().bot(Some(2))),
];
let result = run_match(&config, &mut bots).unwrap();
```

`Personality::ALL`, `name()`, `id()` (e.g. `"old_man_coffee"`), `description()`, `catchphrase()`
and `from_name()` make it easy to list them in an app.

**How they decide.**
- *Preflop:* each bot ranks its hand among all starting hands of the game
  (`strength::preflop_percentile`, by heads-up equity against a random hand)
  and plays, raises or re-raises the top shares its style allows.
- *Table size and position:* the style's range shares are baselines for a
  full 9-handed table. Shorter tables widen them with
  `scale_for_table(share, players) = 1 - (1 - share)^(9 / players)`, so a 20%
  9-handed range is about 28% 6-handed and 63% heads-up. Then position scales
  them by `1 ± position_bonus`: widest on the button, narrowest in the small
  blind (first to act after the flop), with the big blind treated as middle
  position. Even the recreational personalities have a little positional
  awareness (`position_bonus` 0.15).
- *After the flop:* it estimates its equity against random hands for the
  opponents still in (`strength::equity_vs_random`), bets for value above a
  margin, bluffs weak hands at its bluff rate, and calls when its equity
  beats the pot odds times its call factor plus some caution about the bet's
  size.
- *Exploiting:* with `exploit` on (Mister Cheating), it tracks every seat's
  VPIP, PFR, fold-to-bet and aggression from hand histories
  (`stats::OpponentModel`). After 10 hands it bluffs more into players who
  fold a lot, stops bluffing and bets bigger into calling stations, steals
  more from tight players, and calls aggressive players down lighter.
  `style_for(observation)` shows the adjusted style for a spot. Stats follow
  seats, since observations don't name players.

**Your own personality.** Every trait is a field on `Style`;
`Style::default()` is a solid, balanced regular:

| Trait | Effect |
|---|---|
| `vpip`, `pfr`, `three_bet`, `four_bet` | **9-handed baselines** (see below) for the share of starting hands it plays, opens with a raise, re-raises, and raises again against a re-raise. It calls a re-raise with up to 2.5 × `four_bet`, so a tiny value folds to 4-bets. |
| `defend` | Share of its playing range that calls a single raise. |
| `position_bonus` | How much position moves its ranges: `1 + bonus` times as wide on the button, `1 - bonus` in the small blind, scaled in between (the big blind counts as middle position). |
| `any_suited`, `any_ace` | Also plays every suited hand or every hand with an ace. |
| `always_play` | Hands it always raises, in range syntax (`"T2"`, `"AAxx"`); works for Hold'em and Omaha. |
| `push_fold_bb` | At or below this many big blinds it only shoves or folds preflop (`f64::INFINITY` for every hand). |
| `open_size` | Opening raise in big blinds. |
| `value_margin`, `aggression` | How strong a hand must be to bet for value, and how often it then bets rather than checks or calls. |
| `trap` | Chance it slow-plays a monster: checks to check-raise. |
| `backwards` | Bets weak hands and checks strong ones (calls still use real strength). |
| `bluff`, `bluff_raise` | How often it bets or raises with a weak hand. |
| `call_factor`, `caution`, `pair_call_factor` | How much equity it wants to call: a multiple of the pot odds, extra per pot-sized bet faced, and a multiplier when it holds any pair. |
| `bet_size` | Bets as a fraction of the pot (above 1 overbets in no-limit). |
| `tilt`, `heater`, `recovery` | How much big losses or wins loosen it up, and how fast that wears off. `mood()` shows the current level. |
| `exploit`, `exploit_after` | Whether it adapts to opponents, and after how many hands. |
| `samples` | Monte Carlo deals per equity estimate. |

Start from a preset or the default and change what you like:

```rust
use ducy_play::{Personality, PersonalityBot, Style};

let maniac = PersonalityBot::new(
    "Maniac",
    Style { vpip: 0.8, pfr: 0.6, bluff: 0.7, ..Personality::MisterCheating.style() },
    Some(5),
);
```

`cargo run --release -p ducy-play --example personalities` measures every
personality's stats and win rate.

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

[`examples/bots/simple_bot.py`](https://github.com/allenpiscitello/ducy/blob/master/ducy-play/examples/bots/simple_bot.py) is a complete
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
