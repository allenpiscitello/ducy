// Smoke test for hand review in the ducy-wasm JavaScript API.
//
// Build the Node.js package and a tiny model first, then run this file:
//
//   cd ducy-wasm && wasm-pack build --target nodejs --out-dir pkg-node && cd ..
//   cargo run -p ducy-gto --example tiny_model -- target/tiny-model
//   node ducy-wasm/tests/review.cjs
//
// Uses only Node built-ins so it runs without `npm install`.

"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const root = path.join(__dirname, "..", "..");
const pkgDir = process.env.DUCY_WASM_PKG || path.join(__dirname, "..", "pkg-node");
const modelDir = process.env.DUCY_TINY_MODEL || path.join(root, "target", "tiny-model");
const ducy = require(path.join(pkgDir, "ducy_wasm.js"));

ducy.loadGto(
  fs.readFileSync(path.join(modelDir, "cards.bin")),
  fs.readFileSync(path.join(modelDir, "blueprint.bin")),
);
assert.ok(ducy.gtoLoaded());

/// Plays one hand: the bot acts when it's its turn; the person bets half
/// the pot when they can bet, and otherwise checks or calls.
function playHand(table) {
  let state = table.newHand();
  while (!state.complete) {
    if (table.botToAct()) {
      state = table.advance();
      continue;
    }
    const legal = state.legal;
    if (legal.bet) {
      const to = Math.min(
        Number(legal.bet.max_to),
        Math.max(Number(legal.bet.min_to), Math.round(Number(state.pot) / 2)),
      );
      state = table.act("bet", BigInt(to));
    } else {
      state = table.act(legal.can_check ? "check" : "call", 0n);
    }
  }
}

const table = new ducy.BotTable(["gto"], 200n, 1n, 2n, 7n, undefined);
assert.equal(table.reviewLastHand(), null);

const hands = 5;
const started = Date.now();
for (let i = 0; i < hands; i++) {
  playHand(table);
  assert.equal(table.reviewableHands(), i + 1);
}

const last = table.reviewLastHand();
assert.ok(last.decisions.length > 0, "the person made decisions");
for (const d of last.decisions) {
  assert.ok(["preflop", "flop", "turn", "river"].includes(d.street));
  assert.ok(
    ["fine", "inaccuracy", "mistake", "blunder", "deviation", "unknown"].includes(d.grade),
    d.grade,
  );
  assert.equal(typeof d.action, "string");
  assert.ok(d.bb_lost >= 0);
  if (d.grade !== "unknown") {
    assert.ok(d.gto.length > 0);
    const total = d.gto.reduce((a, x) => a + x.frequency, 0);
    assert.ok(Math.abs(total - 1) < 1e-6, `frequencies sum to ${total}`);
  }
}

const session = table.reviewSession();
assert.equal(session.hands.length, hands);
assert.deepEqual(session.hands[hands - 1], last);
assert.equal(session.summary.hands, hands);
assert.equal(
  session.summary.decisions,
  session.hands.reduce((a, h) => a + h.decisions.length, 0),
);
const ms = (Date.now() - started) / hands;
console.log(`${hands} hands played and reviewed, ${ms.toFixed(0)} ms per hand`);

// Resetting the stacks puts both players back to the buy-in for the next
// hand, whatever the last one did.
table.resetStacks();
const fresh = table.newHand();
const behind = s => Number(s.stack) + Number(s.street_bet);
assert.deepEqual(fresh.seats.map(behind), [200, 200]);
while (!table.state().complete) {
  if (table.botToAct()) table.advance();
  else table.act(table.state().legal.can_check ? "check" : "call", 0n);
}

table.clearReviews();
assert.equal(table.reviewableHands(), 0);
assert.equal(table.reviewLastHand(), null);

// Hands against other bots aren't kept.
const other = new ducy.BotTable(["doug_poker"], 200n, 1n, 2n, 7n, undefined);
playHand(other);
assert.equal(other.reviewableHands(), 0);

console.log("ok");
