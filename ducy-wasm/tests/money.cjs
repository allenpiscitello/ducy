// A club's money through ducy-wasm (ducy-web #454): ClubMoney takes ops as
// plain objects and answers {ok, entries} or {ok: false, error, message}.
// The example club from the money model doc: Ann → Marco → Priya → club.
// Run: wasm-pack build --target nodejs --out-dir pkg-node (in ducy-wasm), then
// node ducy-wasm/tests/money.cjs
const assert = require('node:assert/strict');
const path = require('node:path');
const w = require(path.join(__dirname, '..', 'pkg-node', 'ducy_wasm.js'));

const m = new w.ClubMoney(0);
const ok = (op, now = 0) => {
  const r = m.run(op, now);
  assert.equal(r.ok, true, JSON.stringify(r));
  return r.entries;
};

ok({op: 'addLink', member: 'priya', backer: 'club'});
ok({op: 'addLink', member: 'marco', backer: 'priya'});
ok({op: 'addLink', member: 'ann', backer: 'marco'});
ok({op: 'setCredit', member: 'priya', backer: 'club', credit: 5000});
ok({op: 'setCredit', member: 'marco', backer: 'priya', credit: 2000});
ok({op: 'setCredit', member: 'ann', backer: 'marco', credit: 500});
ok({op: 'setTerms', member: 'ann', backer: 'marco', terms: {kind: 'within', ms: 1000}});
ok({op: 'setShare', member: 'priya', backer: 'club', share: 4000});
ok({op: 'setShare', member: 'marco', backer: 'priya', share: 2500});

assert.deepEqual(m.path('ann'), ['ann', 'marco', 'priya', 'club']);
assert.equal(m.room('ann', 0), 500);

// Refused: a structured error, nothing moved.
const loop = m.run({op: 'addLink', member: 'priya', backer: 'ann'}, 0);
assert.deepEqual(loop.error, {code: 'loop'});
const room = m.run({op: 'buyIn', member: 'ann', table: 't1', amount: 600}, 0);
assert.deepEqual(room.error, {code: 'room', member: 'ann', backer: 'marco', room: 500});
assert.match(room.message, /room for 500/);
assert.deepEqual(m.ledger(0), []);

// Buy in for 500, stand up with 800 having contributed 10 in rake.
const buy = ok({op: 'buyIn', member: 'ann', table: 't1', amount: 500}, 1);
assert.equal(buy.length, 3);
assert.ok(buy.every(e => e.kind === 'buy_in' && e.amount === -500 && e.about === 't1'));
assert.deepEqual(m.session('ann'), {about: 't1', path: ['ann', 'marco', 'priya', 'club'], stack: 500, put: 500});
ok({op: 'setStack', member: 'ann', stack: 800});
const out = ok({op: 'cashOut', member: 'ann', contributed: 1000}, 2);
assert.deepEqual(out.filter(e => e.kind === 'rake_share').map(e => [e.member, e.amount]), [['marco', 250], ['priya', 400]]);
assert.equal(m.link('ann', 'marco').balance, 300);
assert.equal(m.link('marco', 'priya').balance, 550);

// Settling with a note; overdue once the terms run out.
ok({op: 'buyIn', member: 'ann', table: 't1', amount: 800}, 3);
ok({op: 'setStack', member: 'ann', stack: 0});
ok({op: 'cashOut', member: 'ann'}, 4);
assert.equal(m.link('ann', 'marco').balance, -500);
// Below zero since the buy-in at 3, so due at 1003.
assert.equal(m.overdue('ann', 'marco', 1002), undefined);
assert.equal(m.overdue('ann', 'marco', 1004), 1003);
assert.equal(m.run({op: 'buyIn', member: 'ann', table: 't1', amount: 1}, 1004).error.code, 'overdue');
const paid = ok({op: 'give', backer: 'marco', member: 'ann', amount: 500, note: 'Cash'}, 1005);
assert.equal(paid[0].note, 'Cash');
assert.equal(m.room('ann', 1005), 500);

// Prizes and the ledger by link.
ok({op: 'pay', member: 'ann', kind: 'prize', about: 'sunday', amount: 2000}, 1006);
assert.equal(m.entries('ann', 'marco', 0).at(-1).balance, 2000);
assert.equal(m.membersOf('marco').length, 1);
assert.equal(m.backersOf('ann')[0].backer, 'marco');
assert.equal(m.account('ann').through, 'marco');
assert.deepEqual(m.audit(), []);

// Saved and restored, the same books.
const back = w.ClubMoney.restore(m.save());
assert.deepEqual(back.links(), m.links());
assert.deepEqual(back.ledger(0), m.ledger(0));
assert.throws(() => m.run({op: 'nope'}, 0));
// As a plain object, and back.
const data = m.data();
assert.equal(data.links.length, 3);
assert.deepEqual(w.ClubMoney.fromData(JSON.parse(JSON.stringify(data))).ledger(0), m.ledger(0));

// A club moving to links: balances (negative too) and a seated player.
const c = new w.ClubMoney(0);
for (const op of [{op: 'addLink', member: 'bob', backer: 'club'}, {op: 'openBalance', member: 'bob', backer: 'club', balance: -250}, {op: 'openSession', member: 'bob', table: 't9', stack: 400, put: 300}]) {
  const r = c.run(op, 0);
  assert.equal(r.ok, true, JSON.stringify(r));
}
assert.equal(c.link('bob', 'club').balance, -250);
assert.equal(c.session('bob').stack, 400);
assert.equal(c.run({op: 'cashOut', member: 'bob'}, 1).ok, true);
assert.equal(c.link('bob', 'club').balance, 150);
assert.equal(c.run({op: 'pay', member: 'bob', kind: 'prize', about: 'hh', amount: 100}, 2).ok, true);
assert.equal(c.run({op: 'void', member: 'bob', about: 'hh', amount: 300}, 3).error.code, 'takeBack');
assert.equal(c.run({op: 'void', member: 'bob', about: 'hh', amount: 100}, 3).entries[0].kind, 'void');

console.log('money ok:', m.ledger(0).length, 'entries');
