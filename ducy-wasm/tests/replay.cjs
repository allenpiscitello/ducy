// Hand replays through the ducy-wasm JavaScript API (replayFrames): the
// frames ducy.cards' replayer made of real hands (#157).
//
//   cd ducy-wasm && wasm-pack build --target nodejs --out-dir pkg-node && cd ..
//   node ducy-wasm/tests/replay.cjs
//
// Uses only Node built-ins so it runs without `npm install`.

"use strict";

const assert = require("node:assert/strict");
const path = require("node:path");

const pkgDir = process.env.DUCY_WASM_PKG || path.join(__dirname, "..", "pkg-node");
const { replayFrames } = require(path.join(pkgDir, "ducy_wasm.js"));
const { hands } = require("../../ducy-play/tests/replay-fixture.json");

for (const [h, hand] of hands.entries()) {
  const frames = replayFrames(hand.events, BigUint64Array.from(hand.stacks.map(BigInt)), hand.names);
  assert.equal(frames.length, hand.frames.length, `hand ${h}`);
  frames.forEach((f, i) => {
    const { street, board, pot, text, seats } = f;
    assert.deepEqual({ street, board, pot, text, seats }, hand.frames[i], `hand ${h} (${hand.label}), frame ${i}`);
    assert.equal(f.event, i === 0 ? null : i - 1);
  });
}
console.log(`replay: ok (${hands.length} hands)`);
