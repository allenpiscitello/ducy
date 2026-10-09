// Player stats through the ducy-wasm JavaScript API (handCounts, botReads):
// the same counting rules the bots use (ducy_play::stats), for the page's
// player stats (#152).
//
//   cd ducy-wasm && wasm-pack build --target nodejs --out-dir pkg-node && cd ..
//   node ducy-wasm/tests/stats.cjs
//
// Uses only Node built-ins so it runs without `npm install`.

"use strict";

const assert = require("node:assert/strict");
const path = require("node:path");

const pkgDir = process.env.DUCY_WASM_PKG || path.join(__dirname, "..", "pkg-node");
const { BotTable, handCounts, botReads } = require(path.join(pkgDir, "ducy_wasm.js"));

// A scripted hand, as table views give events: seat 0 opens, seat 2 (big
// blind) folds, seat 1 3-bets, seat 0 calls; flop: seat 1 bets, seat 0 folds.
const events = [
  { type: "small_blind", seat: 1, amount: 1 },
  { type: "big_blind", seat: 2, amount: 2 },
  { type: "raise", seat: 0, to: 6, all_in: false },
  { type: "fold", seat: 2 },
  { type: "raise", seat: 1, to: 18, all_in: false },
  { type: "call", seat: 0, amount: 12, all_in: false },
  { type: "board", street: "flop", cards: ["As", "Kd", "2c"] },
  { type: "bet", seat: 1, to: 20, all_in: false },
  { type: "fold", seat: 0 },
  { type: "award", seat: 1, pot: 0, amount: 56 },
];
const c = handCounts(events, 3);
assert.equal(c.length, 3);
assert.deepEqual(
  [c[0].vpip, c[0].pfr, c[0].three_bet_chance, c[0].folds, c[0].folds_to_bets, c[0].saw_flop],
  [true, true, false, 1, 1, true],
);
assert.deepEqual([c[1].three_bet_chance, c[1].three_bet, c[1].aggressive], [true, true, 1]);
assert.deepEqual([c[2].three_bet_chance, c[2].three_bet, c[2].saw_flop, c[2].folded], [true, false, false, true]);
assert.throws(() => handCounts([{ type: "nonsense" }], 2), "unknown events are refused");

// Real hands from a bot table: every hand's events count without error, and
// the totals make sense.
const table = new BotTable(["doug_poker", "rampart", "milk_king"], 200n, 1n, 2n, 7n);
const sums = Array.from({ length: 4 }, () => ({ hands: 0, vpip: 0, pfr: 0, faced: 0, foldsTo: 0, aggressive: 0, calls: 0, sawFlop: 0 }));
for (let h = 0; h < 60; h++) {
  let s = table.newHand();
  for (let i = 0; i < 200 && !s.complete; i++) {
    if (table.botToAct()) s = table.advance();
    else if (s.run_choice) s = table.runTwice(false);
    else {
      try { s = table.act("check", 0n); } catch { s = table.act("call", 0n); }
    }
  }
  assert.ok(s.complete, "hand finished");
  const counts = handCounts(s.events, s.seats.length);
  counts.forEach((x, i) => {
    const t = sums[i];
    t.hands++;
    t.vpip += +x.vpip;
    t.pfr += +x.pfr;
    t.faced += x.faced_bets;
    t.foldsTo += x.folds_to_bets;
    t.aggressive += x.aggressive;
    t.calls += x.calls;
    t.sawFlop += +x.saw_flop;
    assert.ok(!x.pfr || x.vpip, "a preflop raise is voluntary");
    assert.ok(x.folds_to_bets <= x.faced_bets && x.folds_to_bets <= x.folds);
    assert.ok(!x.three_bet || x.three_bet_chance);
  });
}
// Seat 0 only checks or calls: it never raises.
assert.equal(sums[0].pfr, 0);
assert.ok(sums.every(t => t.hands === 60));
assert.ok(sums.some(t => t.pfr > 0), "the bots raise sometimes");

// The bots' smoothed reads: no hands gives the priors.
const fresh = botReads(0, 0, 0, 0, 0, 0, 0);
assert.deepEqual(fresh, { vpip: 0.25, pfr: 0.15, fold_to_bet: 0.4, aggression: 1 });
const t = sums[1];
const r = botReads(t.hands, t.vpip, t.pfr, t.faced, t.foldsTo, t.aggressive, t.calls);
assert.ok(Math.abs(r.vpip - (t.vpip + 1) / (t.hands + 4)) < 1e-12);

console.log("stats: ok");
