// PLO bomb pots on two boards through ducy-wasm ("plo4-bomb"): everyone
// antes the big blind, the hand starts on the flop with both boards, and
// every chip is awarded at the end.
// Run: wasm-pack build --target nodejs --out-dir pkg-node (in ducy-wasm), then
// node ducy-wasm/tests/bomb-pot.cjs
const assert = require('node:assert/strict');
const path = require('node:path');
const w = require(path.join(__dirname, '..', 'pkg-node', 'ducy_wasm.js'));

// $5 antes with $500 stacks, in cents: an ante of 500 and a buy-in of 50,000.
const table = new w.BotTable(['doug_poker', 'jen_silly', 'rampart'], 50000n, 200n, 500n, 7n, 'plo4-bomb');
let showdowns = 0;
for (let hand = 0; hand < 30; hand++) {
  let s = table.newHand();
  const antes = s.events.filter(e => e.type === 'ante');
  assert.equal(antes.length, 4, 'everyone antes');
  assert.ok(antes.every(e => Number(e.amount) === 500), 'the ante is the big blind');
  assert.ok(!s.events.some(e => e.type === 'small_blind' || e.type === 'big_blind'), 'no blinds');
  assert.equal(s.board.length, 3, 'starts on the flop');
  assert.equal(s.second_board.length, 3, 'with both boards');
  assert.ok(s.seats[0].cards.length === 4, 'four hole cards');
  for (let guard = 0; guard < 200 && !s.complete; guard++) {
    if (table.botToAct()) s = table.advance();
    else if (s.legal) s = table.act(s.legal.can_check ? 'check' : 'call', 0n);
    else break;
  }
  assert.equal(s.complete, true);
  if (s.showdown) {
    showdowns++;
    assert.equal(s.board.length, 5);
    assert.equal(s.second_board.length, 5, 'both boards run out');
  }
  const won = s.seats.reduce((a, x) => a + Number(x.won), 0);
  assert.equal(won, s.pots.reduce((a, p) => a + Number(p.amount), 0), 'every chip awarded');
}
assert.ok(showdowns > 0, 'some hands went to showdown');
console.log(`bomb-pot: ok (${showdowns} showdowns in 30 hands)`);
