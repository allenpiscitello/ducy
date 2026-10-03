// Smoke test for the ducy-play-wasm JavaScript API.
//
// Build the Node.js package first, then run this file:
//
//   cd ducy-play-wasm
//   wasm-pack build --target nodejs --out-dir pkg-node
//   node tests/smoke.cjs
//
// Node 16 needs `node --experimental-wasm-reftypes tests/smoke.cjs`.
// Uses only Node built-ins so it runs without `npm install`.

"use strict";

const assert = require("node:assert/strict");
const path = require("node:path");

const pkgDir = process.env.DUCY_PLAY_WASM_PKG || path.join(__dirname, "..", "pkg-node");
const { PokerHand } = require(path.join(pkgDir, "ducy_play_wasm.js"));

const tests = [];
function test(name, fn) {
  tests.push({ name, fn });
}

function sum(xs) {
  return xs.reduce((a, b) => a + b, 0);
}

/// Checks or calls until the hand is over.
function checkDown(hand) {
  while (!hand.is_complete()) {
    const legal = hand.legal_actions();
    hand.act(legal.can_check ? "check" : "call");
  }
}

test("seeded 3-handed no-limit Hold'em with a raise plays to completion", () => {
  const stacks = [200, 200, 200];
  const hand = new PokerHand({
    game: "holdem",
    small_blind: 1,
    big_blind: 2,
    stacks,
    button: 0,
    seed: 42,
  });

  assert.equal(hand.is_complete(), false);
  const legal = hand.legal_actions();
  assert.equal(legal.seat, hand.to_act());
  assert.ok(legal.raise, "first player can raise");
  assert.equal(legal.raise.min_to, 4);
  assert.equal(legal.raise.max_to, 200);
  hand.act("raise", legal.raise.min_to);

  const state = hand.state();
  assert.equal(state.street, "preflop");
  assert.equal(state.current_bet, 4);
  assert.equal(state.seats.length, 3);

  checkDown(hand);

  assert.equal(hand.is_complete(), true);
  assert.equal(hand.legal_actions(), null);
  assert.equal(hand.to_act(), undefined);

  const result = hand.result();
  assert.ok(result);
  assert.equal(result.final_stacks.length, 3);
  assert.equal(sum(result.final_stacks), sum(stacks), "chips are conserved");
  assert.equal(sum(result.net), 0);
  assert.equal(sum(result.payouts), sum(result.pots.map((p) => p.amount)));

  const events = hand.events();
  assert.equal(events[0].type, "small_blind");
  assert.equal(events[1].type, "big_blind");
  assert.ok(events.some((e) => e.type === "raise" && e.to === 4));
  assert.ok(events.some((e) => e.type === "award"));
  assert.equal(hand.state().board.length, 5);
});

test("exact-deal pot-limit Omaha checked down", () => {
  const hand = new PokerHand({
    game: "omaha",
    small_blind: 1,
    big_blind: 2,
    stacks: [100, 100],
    button: 0,
    cards: ["Ah Kc Kd Qs", "3h 2h 4c 5d"],
    board: "6h 9h Th Jh 8c",
  });

  // Hole cards come back sorted, so compare them as sets.
  const cardSet = (s) => s.split(" ").sort();
  assert.deepEqual(cardSet(hand.hole_cards(0)), cardSet("Ah Kc Kd Qs"));
  assert.deepEqual(cardSet(hand.hole_cards(1)), cardSet("3h 2h 4c 5d"));

  const legal = hand.legal_actions();
  assert.equal(legal.seat, 0);
  assert.equal(legal.call, 1);
  assert.equal(legal.raise.max_to, 6, "pot-limit max raise");

  checkDown(hand);

  const result = hand.result();
  assert.equal(result.showdown, true);
  assert.deepEqual(result.final_stacks, [98, 102]);
  assert.deepEqual(result.net, [-2, 2]);
  assert.equal(result.pots.length, 1);
  assert.deepEqual(result.pots[0].awards, [{ seat: 1, amount: 4 }]);
});

test("illegal actions and bad configs throw", () => {
  const config = {
    game: "holdem",
    small_blind: 1,
    big_blind: 2,
    stacks: [200, 200, 200],
    button: 0,
    seed: 7,
  };
  const hand = new PokerHand(config);

  assert.throws(() => hand.act("raise", 3), "raise below the minimum");
  assert.throws(() => hand.act("raise", 1000), "raise above the stack");
  assert.throws(() => hand.act("raise"), /need an amount/);
  assert.throws(() => hand.act("raise", 4.5), /whole number/);
  assert.throws(() => hand.act("dance"), /unknown action/);
  assert.throws(() => new PokerHand({ ...config, game: "stud" }), /unknown game/);

  // Failed actions leave the hand untouched.
  assert.equal(hand.to_act(), 0);
  assert.equal(hand.state().current_bet, 2);

  hand.act("fold");
  hand.act("fold");
  assert.equal(hand.is_complete(), true);
  assert.throws(() => hand.act("check"));
  assert.throws(() => hand.act("fold"));
});

test("all-in and call finish the hand", () => {
  const stacks = [50, 80];
  const hand = new PokerHand({
    game: "holdem",
    small_blind: 1,
    big_blind: 2,
    stacks,
    button: 0,
    seed: 1,
  });

  hand.act("all_in");
  assert.equal(hand.state().seats[0].all_in, true);
  hand.act("call");

  assert.equal(hand.is_complete(), true);
  assert.equal(hand.legal_actions(), null);
  assert.equal(hand.state().board.length, 5);
  const result = hand.result();
  assert.equal(result.showdown, true);
  assert.equal(sum(result.final_stacks), sum(stacks));
  assert.equal(sum(result.pots.map((p) => p.amount)), 100);
});

let failed = 0;
for (const { name, fn } of tests) {
  try {
    fn();
    console.log(`ok - ${name}`);
  } catch (e) {
    failed++;
    console.log(`not ok - ${name}`);
    console.log(e);
  }
}
console.log(`${tests.length - failed}/${tests.length} passed`);
process.exit(failed ? 1 : 0);
