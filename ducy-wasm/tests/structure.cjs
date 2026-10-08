// Tournament blind structures through the ducy-wasm JavaScript API
// (blindPresets, blindPreset, checkBlindStructure, estimateTournament):
// the same presets, checks and estimates as ducy.cards had (#156).
//
//   cd ducy-wasm && wasm-pack build --target nodejs --out-dir pkg-node && cd ..
//   node ducy-wasm/tests/structure.cjs
//
// Uses only Node built-ins so it runs without `npm install`.

"use strict";

const assert = require("node:assert/strict");
const path = require("node:path");

const pkgDir = process.env.DUCY_WASM_PKG || path.join(__dirname, "..", "pkg-node");
const { blindPresets, blindPreset, checkBlindStructure, estimateTournament } = require(path.join(pkgDir, "ducy_wasm.js"));
const fixture = require("../../ducy-play/tests/structure-fixture.json");

assert.deepEqual(blindPresets().map(p => p.id), ["turbo", "regular", "deep"]);
for (const c of fixture.presets) {
  // The page's shape: plain objects, breaks as {break: true, minutes}.
  assert.deepEqual(blindPreset(c.preset, BigInt(c.stack)), c.steps, `${c.preset} ${c.stack}`);
  for (const e of c.estimates) {
    const got = estimateTournament(c.steps, BigInt(c.stack), e.players, e.tableSize, e.handsPerHour);
    assert.deepEqual(got, { minutes: e.minutes, hands: e.hands, level: e.level }, `${c.preset} ${c.stack} ${e.players}p`);
  }
}
for (const c of fixture.checks) assert.deepEqual(checkBlindStructure(c.steps), c.errors, JSON.stringify(c.steps));
assert.throws(() => blindPreset("regular", 10n), /at least 20/);

console.log(`structure: ok (${fixture.presets.length} preset cases, ${fixture.checks.length} checks)`);
