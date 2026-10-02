# ducy-play

Play out poker hands on top of [ducy](../README.md): antes and blinds, betting
rounds, side pots and the showdown.

- **Games:** Texas Hold'em and Omaha (high) with 4, 5 or 6 hole cards
- **Betting:** no-limit or pot-limit for either game
- **Cards:** shuffled from a seed (reproducible) or supplied exactly, e.g. to replay a hand
- **History:** every post, action, board card and award is recorded as an `Event`

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
