// Running it twice through ducy-wasm: at a bot table the person chooses and
// the bots go along; at a club table (MultiTable) both players must say yes.
// Run: wasm-pack build --target nodejs --out-dir pkg-node (in ducy-wasm), then
// node ducy-wasm/tests/run-twice.cjs
const assert = require('node:assert/strict');
const path = require('node:path');
const w = require(path.join(__dirname, '..', 'pkg-node', 'ducy_wasm.js'));

// A bot table: the person shoves every hand until a bot calls.
{
  const table = new w.BotTable(['doug_poker'], 200n, 1n, 2n, 11n, undefined);
  let found = false;
  for (let hand = 0; hand < 60 && !found; hand++) {
    let s = table.newHand();
    for (let guard = 0; guard < 50 && !s.complete && !s.run_choice; guard++) {
      if (table.botToAct()) s = table.advance();
      else if (s.legal) s = table.act(s.legal.raise || s.legal.bet ? 'allin' : 'call', 0n);
      else break;
    }
    if (!s.run_choice) continue;
    found = true;
    assert.equal(s.board.length < 5, true, 'cards still to come');
    s = table.runTwice(true);
    assert.equal(s.complete, true);
    assert.equal(s.runs, 2);
    assert.equal(s.second_board.length, 5, 'the second board');
    assert.ok(s.events.some(e => e.type === 'second_board'));
    const won = s.seats.reduce((a, x) => a + Number(x.won), 0);
    assert.equal(won, Number(s.pots.reduce((a, p) => a + Number(p.amount), 0)), 'every chip awarded');
  }
  assert.ok(found, 'a bot called an all-in');
}

// A club table: Ann and Bo all-in; twice only when both say yes.
function clubAllIn() {
  let now = 1000;
  const t = () => (now += 10);
  const table = w.MultiTable.club(2, 40n, 200n, 1n, 2n, 5n, undefined, 30000n);
  ['ann', 'bo'].forEach((id, seat) => {
    table.handle(id, {type: 'join', name: id}, t());
    table.handle(id, {type: 'request_chips', amount: 100}, t());
    table.approveChips(seat, t());
  });
  let r = table.newHand(t());
  for (let k = 0; k < 2; k++) {
    const id = ['ann', 'bo'][r.state.to_act];
    const view = r.out.filter(m => m.to === id && m.data.type === 'state').pop().data.view;
    r = table.handle(id, {type: 'act', seq: r.seq, kind: view.legal.raise ? 'allin' : 'call', amount: 0}, t());
  }
  assert.equal(r.awaiting?.kind, 'run_choice');
  assert.equal(r.state.run_choice, true);
  return {table, t, r};
}
{
  const {table, t} = clubAllIn();
  let r = table.handle('ann', {type: 'run_twice', yes: true}, t());
  const mine = r.out.filter(m => m.to === 'ann' && m.data.type === 'state').pop().data;
  assert.equal(mine.me.run_twice, true, 'Ann sees her answer');
  assert.equal(r.state.complete, false, 'waiting for Bo');
  r = table.handle('bo', {type: 'run_twice', yes: true}, t());
  assert.equal(r.state.complete, true);
  assert.equal(r.state.runs, 2);
  assert.equal(r.state.second_board.length, 5);
}
{
  const {table, t} = clubAllIn();
  table.handle('ann', {type: 'run_twice', yes: true}, t());
  const r = table.handle('bo', {type: 'run_twice', yes: false}, t());
  assert.equal(r.state.runs, 1, 'Bo said no: once');
  assert.deepEqual(r.state.second_board, []);
}
console.log('run-twice: ok');
