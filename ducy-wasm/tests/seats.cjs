// Changing who sits at a bot table between hands (ducy-web #225, the
// Regulars): setBot swaps one bot for another, and with setTopUp(false) a
// seat out of chips sits out until setStack gives it some.
// Run: wasm-pack build --target nodejs --out-dir pkg-node (in ducy-wasm), then
// node ducy-wasm/tests/seats.cjs
const assert = require('node:assert/strict');
const path = require('node:path');
const w = require(path.join(__dirname, '..', 'pkg-node', 'ducy_wasm.js'));

const finish = (table, s) => {
  for (let guard = 0; guard < 200 && !s.complete; guard++) {
    if (table.botToAct()) s = table.advance();
    else if (s.run_choice) s = table.runTwice(false);
    else if (s.legal) s = table.act(s.legal.check ? 'check' : 'fold', 0n);
    else break;
  }
  return s;
};

// setBot: Milk King takes Doug Poker's seat with his own chips.
{
  const table = new w.BotTable(['doug_poker', 'jen_silly'], 200n, 1n, 2n, 7n, undefined);
  let s = finish(table, table.newHand());
  assert.throws(() => table.setBot(0, 'milk_king', 200n, 1n), /no such bot seat/);
  assert.throws(() => table.setBot(3, 'milk_king', 200n, 1n), /no such bot seat/);
  assert.throws(() => table.setBot(1, 'nobody', 200n, 1n), /unknown bot/);
  table.setBot(1, 'milk_king', 350n, 9n);
  s = table.newHand();
  const milk = s.seats.find(x => x.id === 'milk_king');
  assert.ok(milk, 'Milk King sits down');
  assert.equal(milk.name, 'Milk King');
  assert.ok(!s.seats.some(x => x.id === 'doug_poker'), 'Doug Poker left');
  assert.equal(Number(milk.stack) + Number(milk.street_bet), 350, 'with the chips given');
  // Not in the middle of a hand.
  if (!s.complete) assert.throws(() => table.setBot(1, 'doug_poker', 200n, 1n), /isn't over/);
  finish(table, s);
}

// setTopUp(false): the person out of chips sits out until they rebuy.
{
  const table = new w.BotTable(['doug_poker', 'jen_silly'], 200n, 1n, 2n, 3n, undefined);
  table.setTopUp(false);
  finish(table, table.newHand());
  table.setStack(0, 0n);
  let s = table.newHand();
  const me = s.seats.find(x => x.human);
  assert.equal(Number(me.stack), 0);
  assert.equal(me.sitting_out, true, 'not dealt in');
  assert.equal(me.cards, null);
  finish(table, s);
  table.setStack(0, 200n);
  s = table.newHand();
  const back = s.seats.find(x => x.human);
  assert.equal(back.sitting_out, false, 'dealt in after the rebuy');
  assert.equal(back.cards.length, 2);
  finish(table, s);
}
// With top-up on (the default), the person is topped up as before.
{
  const table = new w.BotTable(['doug_poker'], 200n, 1n, 2n, 3n, undefined);
  finish(table, table.newHand());
  table.setStack(0, 0n);
  const s = table.newHand();
  assert.equal(s.seats.find(x => x.human).cards.length, 2);
}
console.log('seats: ok');
