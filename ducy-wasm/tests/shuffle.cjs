// The trustless shuffle through ducy-wasm (#134): a deck set up for three
// players and the host with every proof checked, each player opening only
// their own cards, the audit passing and catching a cheat, a bad proof
// refused, and timings in WebAssembly (#136: one lock-and-shuffle; #140: a
// whole setup with proofs at 6 and 9 players).
// Run: wasm-pack build --target nodejs --out-dir pkg-node (in ducy-wasm), then
// node ducy-wasm/tests/shuffle.cjs
const assert = require('node:assert/strict');
const path = require('node:path');
const w = require(path.join(__dirname, '..', 'pkg-node', 'ducy_wasm.js'));

function setup(players, ctx) {
  const host = 'host';
  const s = new w.ShuffleSetup(players, host, 2, 5, ctx);
  const parties = {};
  for (let guard = 0; guard < 1000; guard++) {
    const r = s.request();
    if (r.kind === 'keys') {
      for (const id of [...s.players(), host]) {
        parties[id] = new w.ShuffleParty(s.contextFor(id));
        assert.equal(s.key(id, parties[id].key()).kind, 'next');
      }
    } else if (r.kind === 'shuffle') {
      const out = parties[r.to].shuffle(r.deck);
      assert.equal(s.shuffled(r.to, out.deck, out.proof).kind, 'next');
    } else if (r.kind === 'unlock') {
      const out = parties[r.to].unlock(r.cards);
      assert.equal(s.unlocked(r.to, out.cards, out.proof).kind, 'next');
    } else return {s, parties, ready: s.ready()};
  }
  throw new Error('setup didn’t finish');
}

// Three players: each opens only their own two cards; the host can't.
const {parties, ready} = setup(['ann', 'bob', 'cy'], 'table T1/hand 1');
const holes = ready.players.map((id, i) => parties[id].open(ready.deck.slice(i * 2, i * 2 + 2)));
assert.throws(() => parties.host.open(ready.deck.slice(0, 2)), /tampered/, 'the host can’t open hole cards');
assert.throws(() => parties.bob.open(ready.deck.slice(0, 2)), /tampered/, 'a player can’t open another’s');
const board = ready.deck.slice(6, 11).map(c => parties.host.open([c])[0]);
const all = new Set([...holes.flat(), ...board]);
assert.equal(all.size, 11, 'eleven different cards');

// The audit after the hand: honest passes; a lie about a secret is caught.
const ids = [...ready.players, 'host'];
const rounds = ids.map((id, i) => ({...parties[id].reveal(), output: ready.outputs[i]}));
const transcript = {players: 3, hole: 2, board: 5, rounds, unlocks: ready.unlocks,
  revealed: [{position: 6, card: board[0], by: 3}]};
assert.deepEqual(w.auditHand(transcript), {ok: true, party: null, fault: null, position: null});
const lying = structuredClone(transcript);
lying.revealed[0].card = board[0] === 'As' ? 'Kd' : 'As';
const caught = w.auditHand(lying);
assert.equal(caught.ok, false);
assert.equal(caught.party, 3, 'names the host');

// A tampered shuffle is refused on the spot, and that player is left out.
{
  const s = new w.ShuffleSetup(['ann', 'bob', 'cy'], 'host', 2, 5, 'table T1/hand 2');
  const ps = {};
  for (const id of ['ann', 'bob', 'cy', 'host']) { ps[id] = new w.ShuffleParty(s.contextFor(id)); s.key(id, ps[id].key()); }
  const r = s.request();
  assert.equal(r.kind, 'shuffle');
  const out = ps[r.to].shuffle(r.deck);
  [out.deck[0], out.deck[1]] = [out.deck[1], out.deck[0]];
  assert.deepEqual(s.shuffled(r.to, out.deck, out.proof), {kind: 'restart', left: 'ann'});
  assert.deepEqual(s.players(), ['bob', 'cy']);
}

// Timings in WebAssembly.
const p = new w.ShuffleParty('00');
let t = performance.now();
const reps = 5;
for (let i = 0; i < reps; i++) p.shuffle(w.shuffleOpenDeck());
const perShuffle = (performance.now() - t) / reps;
const times = {};
for (const n of [6, 9]) {
  t = performance.now();
  setup(Array.from({length: n}, (_, i) => `p${i}`), `table T9/hand ${n}`);
  times[n] = performance.now() - t;
}
console.log(`shuffle: ok · lock, shuffle and prove 52 cards: ${perShuffle.toFixed(0)} ms · whole setup with proofs: ${(times[6] / 1000).toFixed(1)} s at 6 players, ${(times[9] / 1000).toFixed(1)} s at 9 (every party in one thread)`);
