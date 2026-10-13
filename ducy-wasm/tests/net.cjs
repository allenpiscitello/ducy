// Network bots in the browser (#215): loadNet takes a weight file, and
// "net:<personality>" seats that personality played from it.
// Run: wasm-pack build --target nodejs --out-dir pkg-node (in ducy-wasm), then
// node ducy-wasm/tests/net.cjs
const assert = require('node:assert/strict');
const path = require('node:path');
const w = require(path.join(__dirname, '..', 'pkg-node', 'ducy_wasm.js'));

// A weight file (ducy_play::net::Mlp): the magic, the layer sizes, then each
// layer's weights and biases. All zeros: every possible action equally likely.
function netFile(sizes) {
  const params = sizes.slice(1).reduce((n, out, i) => n + sizes[i] * out + out, 0);
  const buf = new ArrayBuffer(8 + 4 + 4 * sizes.length + 4 * params);
  const bytes = new Uint8Array(buf);
  bytes.set(new TextEncoder().encode('DUCYNET'), 0);
  bytes[7] = 1;
  const view = new DataView(buf);
  view.setUint32(8, sizes.length, true);
  sizes.forEach((s, i) => view.setUint32(12 + 4 * i, s, true));
  return bytes;
}

const finish = (table, s) => {
  for (let guard = 0; guard < 400 && !s.complete; guard++) {
    if (table.botToAct()) s = table.advance();
    else if (s.run_choice) s = table.runTwice(false);
    else if (s.legal) s = table.act(s.legal.can_check ? 'check' : 'fold', 0n);
    else break;
  }
  return s;
};

assert.equal(w.netLoaded(), false);
assert.throws(() => new w.BotTable(['net:doug_poker'], 200n, 1n, 2n, 1n, undefined), /loadNet/);
assert.throws(() => w.loadNet(new Uint8Array([1, 2, 3])), /not a ducy network file/);
assert.throws(() => w.loadNet(netFile([10, 8, 7])), /75 inputs and 7 outputs/);
w.loadNet(netFile([75, 16, 7]));
assert.equal(w.netLoaded(), true);
assert.throws(() => new w.BotTable(['net:nobody'], 200n, 1n, 2n, 1n, undefined), /unknown bot/);

// Two network bots play twenty hands to the end, legally.
const table = new w.BotTable(['net:doug_poker', 'net:nik_airbag'], 200n, 1n, 2n, 5n, undefined);
let s = table.newHand();
assert.deepEqual(s.seats.filter(x => !x.human).map(x => x.name), ['Doug Poker', 'Nik Airbag']);
for (let hand = 0; hand < 20; hand++) {
  s = finish(table, hand ? table.newHand() : s);
  assert.ok(s.complete, `hand ${hand} finishes`);
}
console.log('net: ok');
