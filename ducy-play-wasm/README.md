# ducy-play-wasm

WebAssembly bindings for [ducy-play](../ducy-play/README.md): play Hold'em and
Omaha hands from JavaScript. It's separate from `ducy-wasm` (equity and
analysis), so an app that only plays hands doesn't load the analysis code.

```sh
cd ducy-play-wasm && wasm-pack build --target web
```

## Example

```js
import init, { PokerHand } from "./pkg/ducy_play_wasm.js";
await init();

const hand = new PokerHand({
  game: "holdem",          // "holdem" | "omaha"
  small_blind: 1,
  big_blind: 2,
  stacks: [200, 200, 200], // one per seat
  button: 0,
  seed: 42,                // optional: reproducible shuffle
});

while (!hand.is_complete()) {
  const legal = hand.legal_actions();
  if (legal.raise) hand.act("raise", legal.raise.min_to);
  else hand.act(legal.can_check ? "check" : "call");
}
console.log(hand.result());
```

## Config

| Field | Meaning |
|---|---|
| `game` | `"holdem"` or `"omaha"` |
| `hole_cards` | Omaha only: `4` (default), `5` or `6` |
| `betting` | `"no_limit"` or `"pot_limit"`; defaults to no-limit Hold'em and pot-limit Omaha |
| `small_blind`, `big_blind`, `ante` | whole chips; `ante` defaults to 0 |
| `stacks` | one stack per seat, 2–10 seats |
| `button` | the button's seat |
| `seed` | optional seed for the shuffle |
| `cards` + `board` | an exact deal instead of a shuffle, e.g. `["As Ah", "Kd Kh"]` and `"2c 7d 9h Jc 3s"` |

## Methods

| Method | Returns |
|---|---|
| `to_act()` | seat to act, or `undefined` when the hand is over |
| `legal_actions()` | `{ seat, can_fold, can_check, call?, bet?, raise? }` where `bet` / `raise` are `{ min_to, max_to }` street totals; `null` when over |
| `act(action, amount?)` | plays `"fold"`, `"check"`, `"call"`, `"bet"`, `"raise"` or `"all_in"`; `bet` and `raise` take the street total. Throws if illegal. |
| `state()` | `{ street, board, pot, current_bet, button, to_act, complete, seats: [{ stack, street_bet, contributed, folded, all_in }] }` |
| `hole_cards(seat)` | e.g. `"As Ah"`. Your app decides who may see which cards. |
| `events()` | hand history: objects with `type` of `ante`, `small_blind`, `big_blind`, `fold`, `check`, `call`, `bet`, `raise`, `board` or `award` |
| `result()` | `{ showdown, pots: [{ amount, eligible, awards: [{ seat, amount }], winning_hand }], payouts, final_stacks, net }`, or `null` until the hand is over |
| `is_complete()` | whether the hand is over |

Betting rules (min raises, short all-ins, pot limit, side pots) are listed in
the [ducy-play README](../ducy-play/README.md#rules).

## Testing

`tests/smoke.cjs` drives the JS API through a few complete hands. It needs
only Node (no `npm install`) and loads a Node.js build from `pkg-node/`:

```sh
cd ducy-play-wasm
wasm-pack build --target nodejs --out-dir pkg-node
node tests/smoke.cjs
```

Node 18+ runs it as is; Node 16 needs
`node --experimental-wasm-reftypes tests/smoke.cjs`. CI runs it on Node 20.
