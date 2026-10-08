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

// A club table (MultiTable) dealing from the seed (#170): each player gets
// exactly the cards fairDeal gives their seat, through a save and restore.
let now = 1000;
const t = () => (now += 10);
const ids = ['ann', 'bo', 'cy'];
const table = w.MultiTable.club(4, 40n, 200n, 1n, 2n, 5n, undefined, 30000n);
ids.forEach((id, seat) => {
  table.handle(id, {type: 'join', name: id}, t());
  table.handle(id, {type: 'request_chips', amount: 100}, t());
  table.approveChips(seat, t());
});
// A hand is a set of cards: fairDeal lists them in deck order, views as dealt.
const myCards = (r, id) => [...r.out.filter(m => m.to === id && m.data.type === 'state').pop().data.view.seats[0].cards].sort();
const expected = w.fairDeal('nlhe', 3, seed).hole_cards.map(h => [...h].sort());
let r = table.newHand(t(), seed);
assert.equal(r.seed, seed, 'the result names the seed');
ids.forEach((id, i) => assert.deepEqual(myCards(r, id), expected[i], `${id} holds the seed's cards`));

const back = w.MultiTable.restore(table.save(t()), t());
for (const id of ids) r = back.handle(id, {type: 'join', name: id}, t());
ids.forEach((id, i) => assert.deepEqual(myCards(r, id), expected[i], `${id} keeps them after a restore`));

// Without a seed, a secure deal as before, and the result says so.
const plain = w.MultiTable.club(4, 40n, 200n, 1n, 2n, 5n, undefined, 30000n);
ids.slice(0, 2).forEach((id, seat) => {
  plain.handle(id, {type: 'join', name: id}, t());
  plain.handle(id, {type: 'request_chips', amount: 100}, t());
  plain.approveChips(seat, t());
});
r = plain.newHand(t());
assert.equal(r.seed, null, 'no seed: a secure deal');
assert.equal(plain.state(t()).seed, undefined, 'other results carry no seed');
assert.throws(() => w.MultiTable.club(4, 40n, 200n, 1n, 2n, 5n, undefined, 30000n).newHand(t(), 'nothex'), /hex/);
console.log('fair deals ok');
