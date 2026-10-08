# ducy-shuffle

A trustless ("mental poker") shuffle for [ducy](../README.md): the deck is encrypted and shuffled by every player and the host in turn, so **the host never sees anyone's hole cards and no player sees the board early**. It's the cryptographic core of the trustless-shuffle epic (#134). It has no networking or table logic.

## How it works
- **Cards** are fixed points on Ristretto255 (`curve25519-dalek`), hashed from each card's name.
- **Keys:** each party makes a fresh secret per hand and publishes its public key (the secret times the base point). Locking a card multiplies it by the secret, unlocking by its inverse. Locks commute, so they come off in any order.
- **Shuffle:** players in seat order, then the host, each lock every card and apply a secret permutation (`shuffle_round`).
- **Setup:** everyone but a card's owner removes their lock from each hole card, and every player removes theirs from the board (`Layout::unlockers`). Each player then opens their own cards, and the board keeps only the host's lock.
- **Streets:** the host removes its last lock from one street at a time.
- **Showdown:** a player publishes their secret, which opens their hole cards for everyone. It must match their public key.
- **Audit (#138):** after the hand, every party publishes its secret and permutation, and `audit` re-checks the whole deal. On a mismatch it names the party and the fault: a fake shuffle, a bad permutation, a wrong unlock, a substituted card, or a card shown that wasn't dealt.

## Proofs (#140)
Every step comes with a zero-knowledge proof, checked before the deal goes on, so cheating is refused when it happens. The audit stays as a second check.
- **Unlocks** (`UnlockProof`): a batched Chaum–Pedersen proof. Every lock a party removed was removed with the secret behind its public key. One 64-byte proof covers any number of cards.
- **Shuffles** (`ShuffleProof`): the deck a party sent on is the deck it got, permuted and locked with that same secret, without showing either. It's a cut-and-choose proof with 40 shadow shuffles, made non-interactive with Fiat–Shamir. A cheat gets through with probability 2^-40.

  This is simpler than a Bayer–Groth proof, at the cost of size (about 35 KB for 52 cards) and time.
- **Context:** proofs are bound to a context naming the table, hand, attempt and party, so they can't be replayed.

## Setting up the next deck (#137)
`DeckSetup` is the host's side of setting up a deck, run while the previous hand is still being played, so a disconnect never stops or replays a hand.
- **What it does:** it says whom to ask for what (keys, shuffles, unlocks), checks each answer's proof, and ends with a deck ready to deal (`Ready`).
- **Dropouts:** a player who drops out or sends a bad proof before then is left out, and the setup starts again without them, with fresh keys. Fewer than two players means it waits; the host leaving ends it.
- **The hand in play** has its own deck and is never touched.
- **The party side:** `SetupParty` is a party's own end: its secret, its answers, and opening its own cards.

A simulation (`tests/setup.rs`) plays hands with players dropping out and coming back at random during setup. Every hand that has two players is dealt; a player who dropped out is out of that hand only; and the hand being played is never changed.

## In the browser (#139)
ducy-wasm exposes all of this to the page:
- `ShuffleParty`: a party in its own browser;
- `ShuffleSetup`: the host's side;
- `auditHand`: the audit;
- the helpers `shuffleOpenDeck`, `shuffleVerifyUnlock`, `shuffleSecretMatches` and `shuffleOpenWith`.

Cards, keys, secrets and proofs travel as hex strings. `ducy-wasm/tests/shuffle.cjs` deals a hand through them.

## Performance
**Native**, release build:
- lock and shuffle 52 cards: about 5 ms per party;
- one player's setup unlocks at a 9-handed table: about 3 ms;
- a whole 9-player setup, every proof made and checked, all parties in one thread: about 2.2 s.

**WebAssembly** (Node), from `ducy-wasm/tests/shuffle.cjs`:
- lock, shuffle and prove 52 cards: about 340 ms per party;
- a whole setup with proofs, all parties in one thread: about 4.8 s at 6 players and 7 s at 9.

At a table each party proves on its own device, so a setup fits well inside one hand. Run `cargo run --release -p ducy-shuffle --example shuffle_timing` for the native figures.

## Status
**Not yet reviewed by a cryptographer**, including the proofs. Don't trust it with real stakes until it has been (#136, #140).
