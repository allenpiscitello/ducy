// The PLO GTO bots through the ducy-wasm JavaScript API (#131, #200): load a
// model with loadGtoPlo, seat "gto-plo" at a plo4 table (and "gto-plo6" at
// a plo6 table), and play hands against it, timing its decisions.
//
//   cd ducy-wasm && wasm-pack build --target nodejs --out-dir pkg-node && cd ..
//   cargo run -p ducy-gto --example tiny_model -- target/tiny-model
//   node ducy-wasm/tests/gto-plo.cjs
//
// With a real model: DUCY_PLO_CARDS=plo-cards.bin DUCY_PLO_BLUEPRINT=plo-
// blueprint.bin DUCY_PLO_BB=100 DUCY_PLO_HANDS=200 node ducy-wasm/tests/gto-plo.cjs
// (DUCY_PLO6_CARDS, DUCY_PLO6_BLUEPRINT and DUCY_PLO6_BB for PLO6).
//
// Uses only Node built-ins so it runs without `npm install`.

"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const root = path.join(__dirname, "..", "..");
const pkgDir = process.env.DUCY_WASM_PKG || path.join(__dirname, "..", "pkg-node");
const modelDir = process.env.DUCY_TINY_MODEL || path.join(root, "target", "tiny-model");
const cardsPath = process.env.DUCY_PLO_CARDS || path.join(modelDir, "plo-cards.bin");
const blueprintPath = process.env.DUCY_PLO_BLUEPRINT || path.join(modelDir, "plo-blueprint.bin");
const bb = BigInt(process.env.DUCY_PLO_BB || 10);
const hands = Number(process.env.DUCY_PLO_HANDS || 30);
const ducy = require(path.join(pkgDir, "ducy_wasm.js"));

assert.ok(!ducy.gtoPloLoaded());
assert.throws(() => new ducy.BotTable(["gto-plo"], 200n, 1n, 2n, 1n, "plo4"), /loadGtoPlo/);
assert.throws(() => ducy.loadGtoPlo(new Uint8Array([1, 2, 3]), new Uint8Array(), bb), /PLO card abstraction/);

const cards = fs.readFileSync(cardsPath);
const blueprint = fs.readFileSync(blueprintPath);
let t = performance.now();
ducy.loadGtoPlo(cards, blueprint, bb);
const loadMs = performance.now() - t;
assert.ok(ducy.gtoPloLoaded());
// A blueprint for another depth is refused.
assert.throws(() => ducy.loadGtoPlo(cards, blueprint, bb + 1n), /doesn't match/);
ducy.loadGtoPlo(cards, blueprint, bb);

// Plays hands against `id` at a `game` table, timing the bot's decisions.
function play(id, game, bb, cards, blueprint, loadMs) {
  const table = new ducy.BotTable([id], 2n * bb, 1n, 2n, 7n, game);
  const times = [];
  let played = 0;
  for (let h = 0; h < hands; h++) {
    let state = table.newHand();
    while (!state.complete) {
      if (table.botToAct()) {
        t = performance.now();
        state = table.advance();
        times.push(performance.now() - t);
        continue;
      }
      if (state.run_choice) {
        state = table.runTwice(false);
        continue;
      }
      // The person bets the pot when they can, otherwise checks or calls.
      const legal = state.legal;
      const range = legal.bet || legal.raise;
      if (range && h % 3 === 0) {
        state = table.act(legal.bet ? "bet" : "raise", BigInt(range.max_to));
      } else {
        state = table.act(legal.can_check ? "check" : "call", 0n);
      }
    }
    played++;
  }
  times.sort((a, b) => a - b);
  const mean = times.reduce((a, b) => a + b, 0) / times.length;
  assert.ok(times.length > hands, "the bot made decisions");
  console.log(
    `${id}: ok (${played} hands, ${times.length} decisions; load ${loadMs.toFixed(0)} ms, ` +
      `${(cards.length / 1e3).toFixed(0)} KB + ${(blueprint.length / 1e6).toFixed(1)} MB; ` +
      `decisions mean ${mean.toFixed(1)} ms, median ${times[times.length >> 1].toFixed(1)} ms, ` +
      `slowest ${times[times.length - 1].toFixed(1)} ms)`,
  );
}

play("gto-plo", "plo4", bb, cards, blueprint, loadMs);

// PLO6: a six-card model loads alongside the four-card one, as "gto-plo6".
const cards6Path = process.env.DUCY_PLO6_CARDS || path.join(modelDir, "plo6-cards.bin");
const blueprint6Path =
  process.env.DUCY_PLO6_BLUEPRINT || path.join(modelDir, "plo6-blueprint.bin");
const bb6 = BigInt(process.env.DUCY_PLO6_BB || 10);
assert.ok(!ducy.gtoPlo6Loaded());
assert.throws(() => new ducy.BotTable(["gto-plo6"], 200n, 1n, 2n, 1n, "plo6"), /gto-plo6/);
const cards6 = fs.readFileSync(cards6Path);
const blueprint6 = fs.readFileSync(blueprint6Path);
// The four-card blueprint doesn't load with the six-card abstraction.
assert.throws(() => ducy.loadGtoPlo(cards6, blueprint, bb), /doesn't match/);
t = performance.now();
ducy.loadGtoPlo(cards6, blueprint6, bb6);
const load6Ms = performance.now() - t;
assert.ok(ducy.gtoPlo6Loaded() && ducy.gtoPloLoaded());
play("gto-plo6", "plo6", bb6, cards6, blueprint6, load6Ms);
