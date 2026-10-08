// Monte Carlo with one or two known flop cards through the ducy-wasm
// JavaScript API (#154): set_flop takes a partial flop, and sampling deals
// the rest of the board on every sample, so a page can run large batches
// instead of completing flops itself. Sampled equity matches exact equity
// over every completion, and seeded runs repeat.
//
//   cd ducy-wasm && wasm-pack build --target nodejs --out-dir pkg-node && cd ..
//   node ducy-wasm/tests/partial-board.cjs
//
// Uses only Node built-ins so it runs without `npm install`.

"use strict";

const assert = require("node:assert/strict");
const path = require("node:path");

const pkgDir = process.env.DUCY_WASM_PKG || path.join(__dirname, "..", "pkg-node");
const { HoldemGame, OmahaGame, OmahaHiLoGame, OmahaBombPotGame } = require(path.join(pkgDir, "ducy_wasm.js"));

const close = (a, b, what) => a.forEach((x, i) => assert.ok(Math.abs(x - b[i]) < 0.01, `${what}: exact ${a} sampled ${b}`));

// Omaha, two known flop cards.
let g = new OmahaGame(4);
g.add_player("As Ad Kh Qh");
g.add_player("Jc Tc 9d 8d");
g.set_flop("Jh 7c");
close(g.evaluate_equity(), g.sample_equity(40000, 5), "Omaha");
assert.deepEqual(g.sample_equity(5000, 9), g.sample_equity(5000, 9), "seeded runs repeat");
assert.throws(() => g.set_turn("2c"), "no turn before a full flop");

// Omaha hi-lo, one known flop card.
g = new OmahaHiLoGame(4);
g.add_player("As 2d 3h Kc");
g.add_player("Ks Kd Qs Jh");
g.set_flop("4c");
close(g.evaluate_equity(), g.sample_equity(40000, 6), "Omaha hi-lo");

// Hold'em ranges, one known flop card.
const h = new HoldemGame();
h.add_player("As Ah");
h.set_flop("Ks");
const r = h.sample_range_equity(["KK, QQ"], 40000, 7);
close(h.range_equity(["KK, QQ"]), r.equity_sum.map(e => e / r.samples), "Hold'em ranges");

// Bomb pot: one board with a partial flop, one with none.
const b = new OmahaBombPotGame(2, 4);
b.add_player("As Ad Kh Qh");
b.add_player("Jc Tc 9d 8d");
b.set_flop(0, "Jh 7c");
const s = b.sample(20000, [], 8);
assert.equal(s.samples, 20000);
assert.ok(Math.abs(s.equity_sum.reduce((x, y) => x + y, 0) / s.samples - 1) < 1e-9, "shares add up to 1");

console.log("partial-board: ok");
