// Provably fair deals through ducy-wasm (#141): secrets, commitments, the
// seed, and rebuilding a deal from it.
// wasm-pack build --target nodejs --out-dir pkg-node, then node tests/fair.cjs
const assert = require('node:assert/strict');
const w = require('../pkg-node/ducy_wasm.js');

const a = w.fairSecret(), b = w.fairSecret();
assert.match(a.value, /^[0-9a-f]{64}$/);
assert.notEqual(a.value, b.value, 'fresh randomness');
assert.equal(w.fairCommitment(a.value, a.nonce), a.commitment, 'a reveal opens its commitment');
assert.notEqual(w.fairCommitment(b.value, a.nonce), a.commitment, 'another value does not');

// The same numbers as ducy-play's pinned test and the module docs.
const one = '01'.repeat(32), two = '02'.repeat(32);
assert.equal(w.fairCommitment(one, 'fe'.repeat(32)), '0a4bfa03e04cffca04aec9dd0878a0fede90eed815dfd9d9304c6603d6eb2aa8');
const seed = w.fairSeed([one, two]);
assert.equal(seed, 'd1619638c15945404b1d271a1204a400710801bb4af274e77ac3aa64d7cd20db');

const deal = w.fairDeal('plo4', 3, seed);
assert.deepEqual(w.fairDeal('plo4', 3, seed), deal, 'anyone gets the same deal');
assert.equal(deal.hole_cards.length, 3);
assert.ok(deal.hole_cards.every(h => h.length === 4));
assert.equal(deal.board.length, 5);
const all = [...deal.hole_cards.flat(), ...deal.board];
assert.equal(new Set(all).size, all.length, 'no card twice');

assert.throws(() => w.fairSeed(['xyz']), /hex/);
console.log('fair deals ok');
