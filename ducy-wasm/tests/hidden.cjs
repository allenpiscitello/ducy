// A club table dealt by the trustless shuffle (#139), all through ducy-wasm,
// as the page will run it: the deck is set up (ShuffleSetup and each party's
// ShuffleParty), the host deals the hand hidden (MultiTable.newHandHidden),
// sends each player their locked hole cards, unlocks the board street by
// street (dealBoard), and at showdown checks each published secret against
// the player's key and opens their cards with it (reveal). The host's view
// never holds a hole card before showdown; a bad secret forfeits.
// Run: wasm-pack build --target nodejs --out-dir pkg-node (in ducy-wasm), then
// node ducy-wasm/tests/hidden.cjs
const assert = require('node:assert/strict');
const path = require('node:path');
const w = require(path.join(__dirname, '..', 'pkg-node', 'ducy_wasm.js'));

let now = 1000;
const t = () => (now += 10);
const ids = ['ann', 'bo', 'cy'];

function setup(players, ctx) {
  const s = new w.ShuffleSetup(players, 'host', 2, 5, ctx);
  const parties = {};
  for (;;) {
    const r = s.request();
    if (r.kind === 'keys') {
      for (const id of [...s.players(), 'host']) {
        parties[id] = new w.ShuffleParty(s.contextFor(id));
        s.key(id, parties[id].key());
      }
    } else if (r.kind === 'shuffle') {
      const out = parties[r.to].shuffle(r.deck);
      assert.equal(s.shuffled(r.to, out.deck, out.proof).kind, 'next');
    } else if (r.kind === 'unlock') {
      const out = parties[r.to].unlock(r.cards);
      assert.equal(s.unlocked(r.to, out.cards, out.proof).kind, 'next');
    } else return {parties, ready: s.ready()};
  }
}

const table = w.MultiTable.club(4, 40n, 200n, 1n, 2n, 5n, undefined, 30000n);
ids.forEach((id, seat) => {
  table.handle(id, {type: 'join', name: id}, t());
  table.handle(id, {type: 'request_chips', amount: 100}, t());
  table.approveChips(seat, t());
});

// The next hand's deck, set up while nothing else is going on here.
const {parties, ready} = setup(ids, 'table T1/hand 1');
let r = table.newHandHidden(t(), ready.players);
const noHoleCards = r => r.state.seats.forEach(s => assert.equal(s.cards ?? null, null, 'the host sees no hole cards'));
noHoleCards(r);
// Each player gets their locked cards from the host and opens them.
const holes = ready.players.map((id, i) => parties[id].open(ready.deck.slice(i * 2, i * 2 + 2)));
const board = ready.deck.slice(6, 11);

// A host restarting mid-hand picks its party up again from what it saved.
{
  const saved = parties.host.reveal();
  const again = w.ShuffleParty.restore('00', saved.secret, saved.perm);
  assert.equal(again.key(), parties.host.key());
  assert.deepEqual(again.open([board[0]]), parties.host.open([board[0]]));
}

// Everyone checks or calls; the host deals each street when it's waited for.
let dealt = 0;
for (let guard = 0; guard < 100 && !r.awaiting?.seats; guard++) {
  if (r.awaiting?.kind === 'board') {
    const cards = board.slice(dealt, dealt + r.awaiting.cards).map(c => parties.host.open([c])[0]);
    dealt += r.awaiting.cards;
    r = table.dealBoard(cards, t());
    noHoleCards(r);
    continue;
  }
  const seat = r.state.to_act ?? r.state.toAct;
  const id = ids[seat];
  const view = r.out.filter(m => m.to === id && m.data.type === 'state').pop().data;
  const kind = view.view.legal.can_check ? 'check' : 'call';
  r = table.handle(id, {type: 'act', seq: r.seq, kind, amount: 0}, t());
}
assert.equal(dealt, 5, 'the whole board, street by street');
assert.deepEqual(r.awaiting, {kind: 'reveals', seats: [0, 1, 2]});
assert.ok(r.turnMsLeft > 0, 'showing is on the clock');

// Showdown: Ann and Bo publish their secrets; the host checks each against
// the key they set up with and opens their cards with it. Cy sends a secret
// that isn't hers: she forfeits.
const keyOf = id => parties[id].key();
for (const [i, id] of ids.entries()) {
  const secret = id === 'cy' ? parties.ann.reveal().secret : parties[id].reveal().secret;
  if (!w.shuffleSecretMatches(keyOf(id), secret)) { r = table.forfeit(i, t()); continue; }
  const cards = w.shuffleOpenWith(ready.deck.slice(i * 2, i * 2 + 2), [secret]);
  assert.deepEqual(cards, holes[i], `${id}'s cards open with her secret`);
  r = table.reveal(i, cards, t());
}
assert.equal(r.awaiting, null, 'the hand is over');
const shown = r.state.seats.filter(s => s.cards);
assert.equal(shown.length, 2, 'Ann’s and Bo’s hands are shown; Cy’s isn’t');
assert.equal(r.state.seats.reduce((a, s) => a + Number(s.stack), 0), 300, 'no chips made or lost');

// Survives a save and restore mid-hand.
{
  const t2 = w.MultiTable.club(4, 40n, 200n, 1n, 2n, 5n, undefined, 30000n);
  ids.forEach((id, seat) => {
    t2.handle(id, {type: 'join', name: id}, t());
    t2.handle(id, {type: 'request_chips', amount: 100}, t());
    t2.approveChips(seat, t());
  });
  const before = t2.newHandHidden(t(), ['ann', 'cy']);
  const back = w.MultiTable.restore(t2.save(t()), t());
  const after = back.state(t()).state;
  assert.deepEqual(after.events, before.state.events, 'the same hand, Bo sitting out');
  assert.equal(after.pot, 3);
  noHoleCards({state: after});
  assert.throws(() => back.newHandHidden(t(), ['ann', 'cy']), 'a hand is on');
}
console.log('hidden: ok');
