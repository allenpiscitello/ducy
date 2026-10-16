// The six-handed bot through the ducy-wasm JavaScript API: loadGto6 takes a
// compact preflop strategy, and "gto6" then sits at a table, playing the
// strategy before the flop six-handed and Doug Poker's game elsewhere.
//
//   cd ducy-wasm && wasm-pack build --target nodejs --out-dir pkg-node && cd ..
//   cargo run -p ducy-gto --example tiny_model -- target/tiny-model
//   node ducy-wasm/tests/gto6.cjs
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

assert.equal(ducy.gto6Loaded(), false);
assert.throws(() => new ducy.BotTable(["gto6"], 200n, 1n, 2n, 1n, undefined), /loadGto6 first/);
assert.throws(() => ducy.loadGto6(new Uint8Array([1, 2, 3])), /compact six-handed/);
ducy.loadGto6(fs.readFileSync(path.join(modelDir, "sixmax.bin")));
assert.equal(ducy.gto6Loaded(), true);

/// Plays `hands` hands with `bots` and `buyIn` chips at blinds 1/2, the
/// person checking or calling; every one must finish with legal play.
function play(bots, buyIn, hands) {
  const table = new ducy.BotTable(bots, buyIn, 1n, 2n, 7n, undefined);
  for (let h = 0; h < hands; h++) {
    let state = table.newHand();
    for (let steps = 0; !state.complete; steps++) {
      assert.ok(steps < 500, "a hand that never ends");
      if (table.botToAct()) state = table.advance();
      else if (state.run_choice) state = table.runTwice(false);
      else state = table.act(state.legal.can_check ? "check" : "call", 0n);
    }
    // Stacks back to the buy-in, so every hand is played at that depth.
    if (table.resetStacks) table.resetStacks();
  }
}

// Six-handed at 100 BB (the solve's depth), at 200 BB (Doug's game), and at
// a smaller table (always Doug's game).
play(["gto6", "doug_poker", "jonny_slow", "nik_airbag", "milk_king"], 200n, 30);
play(["gto6", "doug_poker", "jonny_slow", "nik_airbag", "milk_king"], 400n, 10);
play(["gto6", "doug_poker"], 200n, 10);
console.log("gto6: ok");
