//! The README's example, so it keeps compiling and working.

use std::collections::HashMap;

use ducy_shuffle::{DeckSetup, Request, SetupParty, Step};

#[test]
fn readme_example() {
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
    let flop: Vec<_> = ready
        .layout
        .board()
        .take(3)
        .map(|pos| ready.deck[pos])
        .collect();
    assert_eq!(parties[&0].open(&flop).unwrap().len(), 3);
}
