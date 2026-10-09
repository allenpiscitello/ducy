// Tournaments for people through ducy-wasm (#142): ClubTournament runs 12
// simulated players over 2 tables to a winner. Chips are conserved, no
// player's view ever has another's unshown cards, a moved player gets a view
// of their new table at once, and p11, who never joins, is blinded off.
// The host restarts every so often (save, restore, everyone joins again),
// and the tournament carries on where it was.
// Run: wasm-pack build --target nodejs --out-dir pkg-node (in ducy-wasm), then
// node ducy-wasm/tests/tournament.cjs
const assert = require('node:assert/strict');
const path = require('node:path');
const w = require(path.join(__dirname, '..', 'pkg-node', 'ducy_wasm.js'));

let seed = 7;
const rand = () => ((seed = (seed * 1103515245 + 12345) % 2147483648) / 2147483648);

const config = {
  game: 'nlhe', tableSize: 6, startingStack: 1000, paid: 3, seed: 5,
  levels: [[10, 20, 0], [25, 50, 5], [50, 100, 10], [100, 200, 25], [200, 400, 50]].map(([sb, bb, ante]) => ({sb, bb, ante})),
};
const players = Array.from({length: 12}, (_, i) => ({id: `p${i}`, name: `P${i}`}));
let t = new w.ClubTournament(config, players, 30000);

let now = 0;
let r = t.state(now);
assert.equal(r.tables.length, 2, 'two tables');
assert.equal(r.totalChips, 12000);

// No view shows anyone else's cards before they're shown down.
const private_ = r => {
  for (const m of r.out) {
    if (m.data.type !== 'state') continue;
    m.data.view.seats.forEach((s, i) => {
      if (i > 0 && s.cards) assert.ok(m.data.view.showdown && !s.folded, `${m.to} sees seat ${i}'s cards`);
    });
  }
};

for (let i = 0; i < 11; i++) {
  r = t.handle(`p${i}`, {type: 'join', name: ''}, now);
  assert.equal(r.out[0].data.type, 'welcome');
  private_(r);
}

const last = new Map(); // each player's latest state
const busted = [];
let hands = 0, moved = 0, winner = null, steps = 0, restarts = 0;
while (!r.over) {
  now += 100;
  assert.ok(now < 50_000_000, 'the tournament ends');
  if (++steps % 15 === 0) {
    // The host restarts: the same tournament, and everyone still in who was
    // here joins again and is sent their view.
    const before = t.state(now);
    t = w.ClubTournament.restore(t.save(now), now);
    restarts++;
    r = t.state(now);
    assert.deepEqual(r.standings, before.standings, 'the same standings after a restart');
    assert.equal(r.level, before.level);
    last.clear();
    for (const p of players.slice(0, 11)) {
      if (!r.standings.some(s => s.id === p.id)) continue;
      r = t.handle(p.id, {type: 'join', name: ''}, now);
      for (const m of r.out) if (m.data.type === 'state') last.set(m.to, m.data);
    }
    continue;
  }
  if (r.canDeal) {
    if (r.tables.every(x => !x.inHand)) {
      const chips = r.standings.reduce((a, s) => a + s.stack, 0);
      assert.equal(chips, r.totalChips, 'chips are conserved');
    }
    r = t.newHands(now);
    hands++;
    if (hands % 8 === 0) r = Object.assign(t.setLevel(r.level + 1, now), {out: r.out, events: r.events});
  } else if (r.autoToAct) {
    r = t.advance(now);
  } else {
    // Whoever has a turn, from their latest view.
    const [who, s] = [...last].find(([, s]) => s.view.legal && s.seq === r.seq) || [];
    assert.ok(who, `someone to act at seq ${r.seq}`);
    const L = s.view.legal;
    const roll = rand();
    const kind = roll < 0.1 ? (L.can_check ? 'check' : 'fold') : roll < 0.2 ? 'allin' : L.can_check ? 'check' : 'call';
    r = t.handle(who, {type: 'act', seq: s.seq, kind, amount: 0}, now);
  }
  private_(r);
  assert.ok(r.out.every(m => m.to !== 'p11'), 'p11 isn’t connected');
  for (const m of r.out) if (m.data.type === 'state') last.set(m.to, m.data);
  // A moved player is sent their new seat at once.
  for (const mv of r.events.moves) {
    if (mv.id === 'p11') continue;
    moved++;
    const sent = r.out.filter(m => m.to === mv.id && m.data.type === 'state').pop();
    assert.equal(sent?.data.view.seat, mv.to.seat, `${mv.id} sees their new seat`);
    assert.ok(t.tableView(mv.to.table), 'the host can watch that table');
  }
  busted.push(...r.events.busted);
  for (const b of r.events.busted) last.delete(b.id);
  if (r.events.winner) winner = r.events.winner;
}

assert.equal(busted.length, 11);
assert.deepEqual(busted.map(b => b.place).sort((a, b) => a - b), Array.from({length: 11}, (_, i) => i + 2));
assert.equal(winner.place, 1);
assert.ok(busted.some(b => b.id === 'p11'), 'p11 was blinded off');
assert.ok(moved > 0, 'players were moved');
assert.ok(restarts > 5, `restarted ${restarts} times`);
assert.throws(() => w.ClubTournament.restore('{"version": 999}', now), 'not a saved tournament');
assert.equal(t.viewFor(busted[0].id), null, 'out: no view');
// The host's view of a table hides everyone's cards until shown down.
const [table] = r.tables;
const hv = t.tableView(table.id);
assert.equal(hv.legal, null);
console.log(`tournament: ok (${hands} deals, ${moved} moves, ${restarts} restarts, won by ${winner.name})`);
