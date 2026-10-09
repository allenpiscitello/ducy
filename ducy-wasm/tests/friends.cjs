// Smoke test for friends tables (MultiTable.friends) in the ducy-wasm
// JavaScript API: empty seats, chip requests the host approves, leaving.
//
//   cd ducy-wasm && wasm-pack build --target nodejs --out-dir pkg-node && cd ..
//   node ducy-wasm/tests/friends.cjs
//
// Uses only Node built-ins so it runs without `npm install`.

"use strict";

const assert = require("node:assert/strict");
const path = require("node:path");

const pkgDir = process.env.DUCY_WASM_PKG || path.join(__dirname, "..", "pkg-node");
const { MultiTable } = require(path.join(pkgDir, "ducy_wasm.js"));

let now = 1000;
const t = () => (now += 10);
// 6 seats, the host with 200 chips, buy-ins from 40 to 200, blinds 1/2.
const host = MultiTable.friends(6, "Host", 200n, 40n, 200n, 1n, 2n, 3n, undefined, 0n);
let r = host.state(t());
assert.equal(r.state.seats.length, 6);
assert.ok(r.state.seats.slice(1).every(s => s.empty), "empty seats");
assert.equal(r.playersIn, 1);
assert.deepEqual(r.buyIn, { min: 40, max: 200 });
assert.throws(() => host.newHand(t()), "no one to play");

// Ann joins with no chips and asks for some.
r = host.handle("ann", { type: "join", name: "Ann" }, t());
const toAnn = r.out.filter(m => m.to === "ann").map(m => m.data);
assert.ok(toAnn.some(d => d.type === "welcome" && d.seat === 1));
r = host.handle("ann", { type: "request_chips", amount: 500 }, t());
assert.ok(r.out.some(m => m.to === "ann" && m.data.type === "rejected"), "too many chips");
r = host.handle("ann", { type: "request_chips", amount: 120 }, t());
assert.deepEqual(r.chipRequests.map(q => [q.seat, q.name, q.amount]), [[1, "Ann", 120]]);
const annState = r.out.filter(m => m.to === "ann" && m.data.type === "state").pop().data;
assert.deepEqual(annState.chips, { min: 40, max: 200, requested: 120, approved: 0 });
r = host.approveChips(1, t());
assert.equal(r.chipRequests.length, 0);
assert.equal(r.playersIn, 2);

// Now they can play; the rest stay empty.
r = host.newHand(t());
assert.equal(r.state.seats[1].stack + r.state.seats[1].street_bet, 120);
assert.ok(r.state.seats.slice(2).every(s => s.empty));

// Ann leaves: she folds out, and her seat is empty again next hand.
r = host.handle("ann", { type: "leave" }, t());
while (!r.state.complete) {
  if (r.botToAct) r = host.advance(t());
  else r = host.act(r.state.legal.can_check ? "check" : "call", 0n, t());
}
assert.equal(r.playersIn, 1);
assert.throws(() => host.newHand(t()), "only the host is left");
assert.ok(host.state(t()).state.seats[1].empty);

// Saved mid-hand and restored, as after the host's browser restarts: the
// same hand, paused until people are back.
const club = MultiTable.club(4, 40n, 200n, 1n, 2n, 5n, undefined, 30000n);
for (const [seat, id] of ["ann", "bo"].entries()) {
  club.handle(id, { type: "join", name: id }, t());
  club.handle(id, { type: "request_chips", amount: 100 }, t());
  club.approveChips(seat, t());
}
const before = club.newHand(t()).state;
const saved = club.save(t());
assert.throws(() => MultiTable.restore(saved.replace('"version":1', '"version":99'), t()));
const back = MultiTable.restore(saved, t());
assert.ok(back.isPaused());
r = back.state(t());
assert.deepEqual([r.state.hand_number, r.state.pot, r.state.to_act], [before.hand_number, before.pot, before.to_act]);
assert.equal(r.turnMsLeft, 30000 - 10, "the clock stood still");
for (const id of ["ann", "bo"]) r = back.handle(id, { type: "join", name: id }, t());
const cards = id => r.out.filter(m => m.to === id && m.data.type === "state").pop().data.view.seats[0].cards;
assert.equal(cards("ann").length, 2, "they get their cards back");
r = back.resume(t());
assert.ok(!back.isPaused());

// The time bank: a minute more on the clock for whoever's turn it is.
const toAct = ["ann", "bo"][r.state.to_act];
const myStatus = res => res.out.filter(m => m.to === toAct && m.data.type === "state").pop().data.me;
const left = r.turnMsLeft;
r = back.handle(toAct, { type: "time_bank" }, t());
assert.equal(myStatus(r).time_bank_on, true);
assert.equal(myStatus(r).time_bank_ms, 0);
assert.ok(r.turnMsLeft >= left + 60000 - 20, `the clock got the bank (${left} → ${r.turnMsLeft})`);
console.log("ok");
