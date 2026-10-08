// Range text through the ducy-wasm JavaScript API (holdemRangeReport,
// omahaRangeReport): every ducy.cards preset and example reads the same as
// the page's own parser did (ranges-fixture.json), and bad text names the
// term at fault (#153).
//
//   cd ducy-wasm && wasm-pack build --target nodejs --out-dir pkg-node && cd ..
//   node ducy-wasm/tests/ranges.cjs
//
// Uses only Node built-ins so it runs without `npm install`.

"use strict";

const assert = require("node:assert/strict");
const path = require("node:path");

const pkgDir = process.env.DUCY_WASM_PKG || path.join(__dirname, "..", "pkg-node");
const { holdemRangeReport, omahaRangeReport } = require(path.join(pkgDir, "ducy_wasm.js"));
const fixture = require("./ranges-fixture.json");

for (const c of fixture.cases) {
  const r = holdemRangeReport(c.text);
  assert.equal(r.ok, true, `${c.text}: ${JSON.stringify(r.error)}`);
  assert.deepEqual([...r.classes].sort(), c.classes, c.text);
  assert.equal(r.combos, c.combos, c.text);
}

// Dead cards come off the combo count.
assert.equal(holdemRangeReport("AA, AKs", "As").combos, 3 + 3);
// Bad text: the term and where it starts.
const bad = holdemRangeReport("QQ+, AKx");
assert.equal(bad.ok, false);
assert.deepEqual([bad.error.term, bad.error.start], ["AKx", 5]);
assert.match(bad.error.message, /AKx/);

// Omaha: the same answers through omahaRangeReport.
const plo = omahaRangeReport("AAxx$ds", 4);
assert.equal(plo.ok, true);
assert.ok(plo.combos > 0 && plo.combos < plo.total && plo.coverage > 0 && plo.coverage < 1);
assert.equal(plo.total, 270725);
const ploBad = omahaRangeReport("AAxx, $nonsense", 4);
assert.equal(ploBad.ok, false);
assert.deepEqual([ploBad.error.term, ploBad.error.start], ["$nonsense", 6]);

console.log(`ranges: ok (${fixture.cases.length} preset and example texts)`);
