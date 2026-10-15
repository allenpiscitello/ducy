// The GTO bot with blueprints at several depths, through the ducy-wasm
// JavaScript API: loadGtoDepth adds a blueprint trained at another depth,
// and "gto" then plays each hand with the one trained nearest its depth.
//
//   cd ducy-wasm && wasm-pack build --target nodejs --out-dir pkg-node && cd ..
//   cargo run -p ducy-gto --example tiny_model -- target/tiny-model
//   node ducy-wasm/tests/gto-depths.cjs
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
const read = name => fs.readFileSync(path.join(modelDir, name));

// A depth needs the main model (its card abstraction) first.
assert.throws(() => ducy.loadGtoDepth(read("blueprint-25.bin"), 25n), /loadGto first/);
ducy.loadGto(read("cards.bin"), read("blueprint.bin"));
assert.deepEqual(Array.from(ducy.gtoDepths()), []);

// A blueprint is checked against the depth it's given for.
assert.throws(() => ducy.loadGtoDepth(read("blueprint-25.bin"), 10n), /doesn't match 10 BB/);
ducy.loadGtoDepth(read("blueprint-25.bin"), 25n);
// Loading the same depth again replaces it.
ducy.loadGtoDepth(read("blueprint-25.bin"), 25n);
assert.deepEqual(Array.from(ducy.gtoDepths()).map(Number), [25]);

/// Plays `hands` hands against "gto" with `buyIn` chips at blinds 1/2,
/// checking or calling; every one must finish with legal play.
function play(buyIn, hands) {
  const table = new ducy.BotTable(["gto"], buyIn, 1n, 2n, 11n, undefined);
  for (let h = 0; h < hands; h++) {
    let state = table.newHand();
    for (let steps = 0; !state.complete; steps++) {
      assert.ok(steps < 200, "a hand that never ends");
      if (table.botToAct()) state = table.advance();
      else if (state.run_choice) state = table.runTwice(false);
      else state = table.act(state.legal.can_check ? "check" : "call", 0n);
    }
    // Stacks back to the buy-in, so every hand is played at that depth.
    if (table.resetStacks) table.resetStacks();
  }
}

// 25 BB hands go to the 25 BB blueprint, 100 BB ones to the main one; both
// play every hand through.
play(50n, 20);
play(200n, 20);

// A new main model drops the other depths (they went with its cards).
ducy.loadGto(read("cards.bin"), read("blueprint.bin"));
assert.deepEqual(Array.from(ducy.gtoDepths()), []);

console.log("gto-depths: ok");
