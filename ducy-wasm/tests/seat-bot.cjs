// Bots that play a club table as its players do (SeatBot): each sees only
// its own seat's view, as sent over the wire, and sends its moves as a
// player would. Every move is one the table takes, and chips are conserved.
// wasm-pack build --target nodejs --out-dir pkg-node, then node tests/seat-bot.cjs
const assert = require('node:assert/strict');
const w = require('../pkg-node/ducy_wasm.js');

for (const game of [undefined, 'plo4']) {
  let now = 1000;
  const t = () => (now += 10);
  // A generic player, two Regulars and an empty seat.
  const ids = ['g-rock-12', 'phil_bigmouth', 'g-maniac-7', 'doug_poker'];
  const bots = Object.fromEntries(ids.map((id, i) => [id, new w.SeatBot(id, BigInt(i + 1))]));
  const table = w.MultiTable.club(5, 40n, 200n, 1n, 2n, 9n, game, 30000n);
  ids.forEach((id, seat) => {
    table.handle(id, {type: 'join', name: id}, t());
    table.handle(id, {type: 'request_chips', amount: 200}, t());
    table.approveChips(seat, t());
  });
  const last = {};
  const take = r => {
    for (const m of r.out) {
      assert.notEqual(m.data.type, 'rejected', `${m.to}: ${m.data.reason}`);
      if (m.data.type === 'state') last[m.to] = JSON.parse(JSON.stringify(m.data));
    }
    return r;
  };
  let moves = 0, hands = 0;
  const kinds = new Set();
  for (; hands < 30; hands++) {
    let r = take(table.newHand(t()));
    for (let guard = 0; guard < 200 && !r.state.complete; guard++) {
      const id = ids.find(x => last[x]?.view.legal);
      assert.ok(id, 'someone to act');
      const {seq, view} = last[id];
      // No one else is asked.
      for (const x of ids) if (x !== id) assert.equal(bots[x].act(last[x].view), null);
      const m = bots[id].act(view);
      kinds.add(m.kind);
      moves++;
      r = take(table.handle(id, {type: 'act', seq, kind: m.kind, amount: m.amount}, t()));
    }
    assert.ok(r.state.complete, 'the hand finished');
    if (r.players.filter(p => p.chips > 0).length < 2) break;
  }
  const chips = table.state(t()).players.reduce((a, p) => a + p.chips, 0);
  assert.equal(chips, ids.length * 200, `${game || 'nlhe'}: chips conserved`);
  assert.ok(moves > 40 && kinds.size >= 3, `${game || 'nlhe'}: ${moves} moves, ${[...kinds]}`);
}
assert.throws(() => new w.SeatBot('nobody', 1n), /unknown bot/);
console.log('seat-bot ok');
