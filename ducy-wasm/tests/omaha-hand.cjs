// Omaha starting-hand analysis through the ducy-wasm JavaScript API
// (omahaHand, omahaTagLabels): the same features, tags, blocking, score
// and percentile as ducy.cards' JavaScript had (#155).
//
//   cd ducy-wasm && wasm-pack build --target nodejs --out-dir pkg-node && cd ..
//   node ducy-wasm/tests/omaha-hand.cjs
//
// Uses only Node built-ins so it runs without `npm install`.

"use strict";

const assert = require("node:assert/strict");
const path = require("node:path");

const pkgDir = process.env.DUCY_WASM_PKG || path.join(__dirname, "..", "pkg-node");
const { omahaHand, omahaTagLabels } = require(path.join(pkgDir, "ducy_wasm.js"));
const fixture = require("../../src/games/omaha_analysis_fixture.json");

const close = (a, b) => Math.abs(a - b) <= 1e-9 * Math.max(Math.abs(a), 1);
for (const c of fixture.cases) {
  const h = omahaHand(c.cards.join(" "));
  assert.deepEqual(h.features, fixture.features);
  h.values.forEach((v, i) => assert.ok(close(v, c.features[i]), `${c.cards} ${h.features[i]}: ${v} vs ${c.features[i]}`));
  assert.deepEqual(h.tags, c.tags, String(c.cards));
  assert.ok(close(h.nut_flush_block, c.block) && close(h.score, c.score) && close(h.percentile, c.percentile), String(c.cards));
}
assert.deepEqual(omahaTagLabels(), fixture.tagLabels);
assert.throws(() => omahaHand("As Ad"), /4, 5 or 6/);

console.log(`omaha-hand: ok (${fixture.cases.length} hands)`);
