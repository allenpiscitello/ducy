# ducy-shuffle

A trustless ("mental poker") shuffle for [ducy](../README.md): the deck is encrypted and shuffled by every player and the host in turn, so **the host never sees anyone's hole cards and no player sees the board early**. It's the cryptographic core of the trustless-shuffle epic (#134). It has no networking or table logic.

## How it works
- **Cards** are fixed points on Ristretto255 (`curve25519-dalek`), hashed from each card's name.
- **Keys:** each party makes a fresh secret per hand. Locking a card multiplies it by the secret, unlocking by its inverse. Locks commute, so they come off in any order.
- **Shuffle:** players in seat order, then the host, each lock every card and apply a secret permutation (`shuffle_round`).
- **Setup:** everyone but a card's owner removes their lock from each hole card, and every player removes theirs from the board (`Layout::unlockers`). Each player then opens their own cards, and the board keeps only the host's lock.
- **Streets:** the host removes its last lock from one street at a time.
- **Showdown:** a player publishes their secret, which opens their hole cards for everyone.
- **Audit (#138):** after the hand, every party publishes its secret and permutation, and `audit` re-checks the whole deal. On a mismatch it names the party and the fault: a fake shuffle, a bad permutation, a wrong unlock, a substituted card, or a card shown that wasn't dealt.

## Performance
Native, release build:
- lock and shuffle 52 cards: about 5 ms per party;
- one player's setup unlocks at a 9-handed table: about 3 ms.

The WASM figure comes with the bindings (#139). Run `cargo run --release -p ducy-shuffle --example shuffle_timing` to measure.

## Status
**Not yet reviewed by a cryptographer.** Don't trust it with real stakes until it has been (#136). Zero-knowledge proofs that prevent cheating outright, rather than catching it afterwards, are phase 2 (#140).
