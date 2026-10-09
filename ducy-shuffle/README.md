# ducy-shuffle

A trustless ("mental poker") shuffle for [ducy](https://crates.io/crates/ducy):
the deck is encrypted and shuffled by every player and the host in turn, so
**the host never sees anyone's hole cards and no player sees the board
early**. It's the cryptographic core of ducy's trustless shuffle
([ducy#134](https://github.com/allenpiscitello/ducy/issues/134)). It has no
networking or table logic.

> **Status: not yet reviewed by a cryptographer**, including the proofs.
> Don't trust it with real stakes until it has been.
> [ducy#174](https://github.com/allenpiscitello/ducy/issues/174) lists what
> the review covers.

## Install

```toml
[dependencies]
ducy-shuffle = "0.1"
```

It builds natively and for WebAssembly (`wasm32-unknown-unknown`, with
`getrandom`'s `wasm_js` backend).

## How it works
- **Cards** are fixed points on Ristretto255 (`curve25519-dalek`), hashed from each card's name (`card_point`).
- **Keys:** each party makes a fresh `Secret` per hand and publishes its `PublicKey` (the secret times the base point). Locking a card multiplies it by the secret, unlocking by its inverse. Locks commute, so they come off in any order.
- **Shuffle:** players in seat order, then the host, each lock every card and apply a secret permutation (`shuffle_round`).
- **Setup:** everyone but a card's owner removes their lock from each hole card, and every player removes theirs from the board (`Layout::unlockers`). Each player then opens their own cards, and the board keeps only the host's lock.
- **Streets:** the host removes its last lock from one street at a time.
- **Showdown:** a player publishes their secret, which opens their hole cards for everyone. It must match their public key.
- **Audit:** after the hand, every party publishes its secret and permutation, and `audit` re-checks the whole deal. On a mismatch it names the party and the fault (`FaultKind`): a fake shuffle, a bad permutation, a wrong unlock, a substituted card, a card shown that wasn't dealt, or a missing round.

## Proofs
Every step comes with a zero-knowledge proof, checked before the deal goes on, so cheating is refused when it happens. The audit stays as a second check.
- **Unlocks** (`UnlockProof`): a batched Chaum–Pedersen proof that every lock a party removed was removed with the secret behind its public key. One 64-byte proof covers any number of cards.
- **Shuffles** (`ShuffleProof`): the deck a party sent on is the deck it got, permuted and locked with that same secret, without showing either. It's a cut-and-choose proof with 40 shadow shuffles (`SHUFFLE_ROUNDS`), made non-interactive with Fiat–Shamir, so a cheat gets through with probability 2^-40.

  This is simpler than a Bayer–Groth proof, at the cost of size (about 42 KB for 52 cards) and time.
- **Context:** proofs are bound to a context naming the table, hand, attempt and party, so they can't be replayed.

## Setting up the next deck
`DeckSetup` is the host's side of setting up a deck, run while the previous hand is still being played, so a disconnect never stops or replays a hand.
- **What it does:** it says whom to ask for what (`Request`: keys, shuffles, unlocks), checks each answer's proof, and ends with a deck ready to deal (`Ready`).
- **Dropouts:** a player who drops out or sends a bad proof before then is left out, and the setup starts again without them, with fresh keys (`Step::Restart`). Fewer than two players means it waits; the host leaving ends it.
- **The hand in play** has its own deck and is never touched.
- **The party side:** `SetupParty` is a party's own end: its secret, its answers, and opening its own cards.
- **Next-hand secrecy:** the host keeps the ready deck until the hand starts, then sends each player their hole cards. Nothing a player is sent during setup opens with their key, so no one learns their next cards before the current hand is over.

A simulation (`tests/setup.rs`) plays hands with players dropping out and coming back at random during setup. Every hand that has two players is dealt; a player who dropped out is out of that hand only; and the hand being played is never changed.

### Example

```rust
use std::collections::HashMap;
use ducy_shuffle::{DeckSetup, Request, SetupParty, Step};

let mut rng: rand::rngs::StdRng = rand::make_rng();
// Three players (by id) and the host (0); Hold'em: 2 hole cards each, 5 on the board.
let mut setup = DeckSetup::new(vec![1, 2, 3], 0, 2, 5, b"table T1/hand 1");
let mut parties: HashMap<u32, SetupParty> = HashMap::new();
let ready = loop {
    match setup.request() {
        Request::Keys => {
            for p in [1, 2, 3, 0] {
                let party = SetupParty::new(&setup.context_for(&p).unwrap(), &mut rng);
                setup.key(&p, party.key());
                parties.insert(p, party);
            }
        }
        Request::Shuffle { to, deck } => {
            let (out, proof) = parties.get_mut(&to).unwrap().shuffle(&deck, &mut rng);
            assert_eq!(setup.shuffled(&to, out, &proof), Step::Next);
        }
        Request::Unlock { to, cards, .. } => {
            let (out, proof) = parties[&to].unlock(&cards, &mut rng);
            assert_eq!(setup.unlocked(&to, out, &proof), Step::Next);
        }
        Request::Done => break setup.ready().unwrap(),
    }
};
// Each player opens only their own hole cards; the host opens the board.
for (i, p) in ready.players.iter().enumerate() {
    let mine: Vec<_> = ready.layout.hole(i).map(|pos| ready.deck[pos]).collect();
    assert_eq!(parties[p].open(&mine).unwrap().len(), 2);
}
let flop: Vec<_> = ready.layout.board().take(3).map(|pos| ready.deck[pos]).collect();
assert_eq!(parties[&0].open(&flop).unwrap().len(), 3);
```

## Trust model
- **The host** relays messages and checks every proof. It sees the board before it's dealt (the one accepted exposure: hosts have no seat at club tables), but never a hole card.
- **Players** can't see each other's cards, even if all the other players and the host pool their secrets: each player's own shuffle and lock stay secret.
- **A party that drops out** during setup is left out of that deck. After setup only the host is needed: a player who disconnects folds, and one who doesn't show at showdown can't win.
- **Known limits, for the review:** the shuffle proof's soundness is 2^-40 per proof with Fiat–Shamir. And a host that cheats could try to use players as oracles (asking them to remove locks from cards they shouldn't), unless players check its requests. ducy.cards has players check the host's shuffle and every lock removed before removing their own.

## In the browser
ducy-wasm exposes all of this to a web page:
- `ShuffleParty`: a party in its own browser;
- `ShuffleSetup`: the host's side;
- `auditHand`: the audit;
- the helpers `shuffleOpenDeck`, `shuffleVerifyShuffle`, `shuffleVerifyUnlock`, `shuffleSecretMatches` and `shuffleOpenWith`.

Cards, keys, secrets and proofs travel as hex strings.

## Performance
**Native**, release build:
- lock and shuffle 52 cards: about 5 ms per party;
- one player's setup unlocks at a 9-handed table: about 3 ms;
- a whole 9-player setup, every proof made and checked, all parties in one thread: about 2.2 s.

**WebAssembly** (Node), from ducy-wasm's `tests/shuffle.cjs`:
- lock, shuffle and prove 52 cards: about 250–340 ms per party;
- a whole setup with proofs, all parties in one thread: about 3.6–4.8 s at 6 players and 5.3–7 s at 9.

At a table each party proves on its own device, so a setup fits well inside one hand. Run `cargo run --release -p ducy-shuffle --example shuffle_timing` for the native figures.

## License
MIT
