//! Setting up the next hand's deck while a hand is played (#137), with the
//! proofs checked as it goes (#140): a simulation of many hands with players
//! dropping out and coming back at random during setup.

use std::collections::{HashMap, HashSet};

use ducy_shuffle::{
    DeckSetup, Layout, PartyRound, Ready, Request, SetupParty, Step, Transcript, audit, decode,
};
use rand::{RngExt, SeedableRng, rngs::StdRng};

const HOST: u32 = 99;

/// Runs one setup to the end, answering every request honestly, except that
/// `drop_at` (step number → player) players disconnect along the way. Returns
/// the ready deck (or None if too few were left) and every party's side.
fn run_setup(
    players: Vec<u32>,
    drop_at: &HashMap<usize, u32>,
    rng: &mut StdRng,
) -> (Option<Ready<u32>>, HashMap<u32, SetupParty>, Vec<u32>) {
    let mut setup = DeckSetup::new(players, HOST, 2, 5, b"table T1/hand 12");
    let mut parties: HashMap<u32, SetupParty> = HashMap::new();
    let mut dropped = Vec::new();
    for step in 0..10_000 {
        // Ready: a later disconnect is the hand's business, not the setup's.
        if let Some(ready) = setup.ready() {
            return (Some(ready), parties, dropped);
        }
        if let Some(&p) = drop_at.get(&step)
            && setup.players().contains(&p)
        {
            dropped.push(p);
            match setup.drop_party(p) {
                Step::Waiting => return (None, parties, dropped),
                Step::Failed => panic!("the host didn't leave"),
                _ => {}
            }
            continue;
        }
        match setup.request() {
            Request::Keys => {
                parties.clear();
                let everyone: Vec<u32> = setup.players().iter().copied().chain([HOST]).collect();
                for p in everyone {
                    let party = SetupParty::new(&setup.context_for(&p).unwrap(), rng);
                    assert_eq!(setup.key(&p, party.key()), Step::Next);
                    parties.insert(p, party);
                }
            }
            Request::Shuffle { to, deck } => {
                let (out, proof) = parties.get_mut(&to).unwrap().shuffle(&deck, rng);
                assert_eq!(
                    setup.shuffled(&to, out, &proof),
                    Step::Next,
                    "an honest shuffle is accepted"
                );
            }
            Request::Unlock { to, cards, .. } => {
                let (out, proof) = parties[&to].unlock(&cards, rng);
                assert_eq!(
                    setup.unlocked(&to, out, &proof),
                    Step::Next,
                    "honest unlocks are accepted"
                );
            }
            Request::Done => return (setup.ready(), parties, dropped),
        }
    }
    panic!("setup didn't finish");
}

/// Deals a ready deck: each player opens their own hole cards, and the host
/// opens the board.
fn deal(
    ready: &Ready<u32>,
    parties: &HashMap<u32, SetupParty>,
) -> (Vec<Vec<ducy::deck::Card>>, Vec<ducy::deck::Card>) {
    let layout = ready.layout;
    let holes = ready
        .players
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let cards: Vec<_> = layout.hole(i).map(|pos| ready.deck[pos]).collect();
            parties[p]
                .open(&cards)
                .expect("only the owner's lock is left")
        })
        .collect();
    let board = layout
        .board()
        .map(|pos| parties[&HOST].open(&[ready.deck[pos]]).unwrap()[0])
        .collect();
    (holes, board)
}

#[test]
fn a_full_setup_deals_each_player_their_own_cards_only() {
    let mut rng = StdRng::seed_from_u64(1);
    let (ready, parties, _) = run_setup(vec![1, 2, 3, 4, 5, 6], &HashMap::new(), &mut rng);
    let ready = ready.expect("ready");
    let layout = ready.layout;
    // The host can't open anyone's hole cards, and players can't open the board.
    for (i, _) in ready.players.iter().enumerate() {
        for pos in layout.hole(i) {
            assert!(decode(&ready.deck[pos]).is_none());
            assert!(
                parties[&HOST].open(&[ready.deck[pos]]).is_none(),
                "the host never sees hole cards"
            );
        }
    }
    for pos in layout.board() {
        assert!(
            decode(&ready.deck[pos]).is_none(),
            "the board stays locked until the host deals it"
        );
        assert!(parties[&1].open(&[ready.deck[pos]]).is_none());
    }
    let (holes, board) = deal(&ready, &parties);
    let all: HashSet<String> = holes
        .iter()
        .flatten()
        .chain(&board)
        .map(|c| c.to_string())
        .collect();
    assert_eq!(all.len(), 6 * 2 + 5, "every card is different");
    // And the audit afterwards agrees with what the proofs allowed.
    let rounds = ready
        .players
        .iter()
        .chain([&HOST])
        .zip(&ready.outputs)
        .map(|(p, output)| {
            let (secret, perm) = parties[p].reveal();
            PartyRound {
                secret,
                perm,
                output: output.clone(),
            }
        })
        .collect();
    let t = Transcript {
        layout,
        rounds,
        unlocks: ready.unlocks.clone(),
        revealed: vec![],
    };
    assert_eq!(audit(&t), Ok(()));
}

/// The next hand's deck is set up while the current hand is played, so no
/// player may learn their next-hand cards before it's dealt. The host holds
/// the finished deck and sends each player their hole cards (carrying only
/// that player's lock) when the hand starts. Until then nothing a player has
/// been sent opens with their key: they never see their own hole cards with
/// the other locks off.
#[test]
fn nothing_a_player_is_sent_during_setup_opens_their_next_hand_cards() {
    let mut rng = StdRng::seed_from_u64(5);
    let players: Vec<u32> = (1..=6).collect();
    let mut setup = DeckSetup::new(players.clone(), HOST, 2, 5, b"table T3/hand 7");
    let mut parties: HashMap<u32, SetupParty> = HashMap::new();
    let mut sent: HashMap<u32, Vec<ducy_shuffle::Masked>> = HashMap::new();
    while setup.ready().is_none() {
        match setup.request() {
            Request::Keys => {
                for p in players.iter().copied().chain([HOST]) {
                    let party = SetupParty::new(&setup.context_for(&p).unwrap(), &mut rng);
                    setup.key(&p, party.key());
                    parties.insert(p, party);
                }
            }
            Request::Shuffle { to, deck } => {
                sent.entry(to).or_default().extend(&deck);
                let (out, proof) = parties.get_mut(&to).unwrap().shuffle(&deck, &mut rng);
                assert_eq!(setup.shuffled(&to, out, &proof), Step::Next);
            }
            Request::Unlock { to, cards, .. } => {
                sent.entry(to).or_default().extend(&cards);
                let (out, proof) = parties[&to].unlock(&cards, &mut rng);
                assert_eq!(setup.unlocked(&to, out, &proof), Step::Next);
            }
            Request::Done => break,
        }
    }
    let ready = setup.ready().unwrap();
    for (i, p) in ready.players.iter().enumerate() {
        let got = &sent[p];
        let mine: Vec<_> = ready.layout.hole(i).map(|pos| ready.deck[pos]).collect();
        for card in &mine {
            assert!(
                !got.contains(card),
                "player {p} was never sent their hole cards"
            );
        }
        for card in got {
            assert!(
                parties[p].open(&[*card]).is_none(),
                "nothing player {p} was sent opens with their key"
            );
        }
        // Once the hand starts and the host sends them, they open.
        assert!(parties[p].open(&mine).is_some());
    }
}

#[test]
fn a_bad_proof_leaves_that_player_out_and_starts_again() {
    let mut rng = StdRng::seed_from_u64(2);
    let mut setup = DeckSetup::new(vec![1, 2, 3], HOST, 2, 5, b"table T2/hand 1");
    let mut parties = HashMap::new();
    for p in [1, 2, 3, HOST] {
        let party = SetupParty::new(&setup.context_for(&p).unwrap(), &mut rng);
        setup.key(&p, party.key());
        parties.insert(p, party);
    }
    let Request::Shuffle { to: 1, deck } = setup.request() else {
        panic!("player 1 shuffles first")
    };
    let (mut out, proof) = parties.get_mut(&1).unwrap().shuffle(&deck, &mut rng);
    out.swap(0, 1); // not what the proof is about
    assert_eq!(setup.shuffled(&1, out, &proof), Step::Restart { left: 1 });
    assert_eq!(setup.players(), &[2, 3]);
    assert_eq!(
        setup.request(),
        Request::Keys,
        "everyone left makes new keys"
    );
}

#[test]
fn the_host_leaving_ends_it_and_too_few_players_wait() {
    let mut s = DeckSetup::new(vec![1, 2, 3], HOST, 2, 5, b"t");
    assert_eq!(s.drop_party(2), Step::Restart { left: 2 });
    assert_eq!(s.drop_party(1), Step::Waiting);
    assert!(
        s.join(1, |a, b| a.cmp(b)),
        "a player back before the shuffle is in"
    );
    assert_eq!(s.players(), &[1, 3]);
    assert_eq!(s.drop_party(HOST), Step::Failed);
}

/// Many hands in a row: each hand is played from the deck set up during the
/// one before. Players drop out at random points of the setup and come back
/// later. A hand being played is never touched by the next deck's setup, a
/// player who drops during setup is out of that hand only, and every hand
/// that has two players is dealt.
#[test]
fn random_disconnects_during_setup_never_touch_the_hand_in_play() {
    let mut rng = StdRng::seed_from_u64(3);
    let seated: Vec<u32> = (1..=5).collect();
    let mut away: HashSet<u32> = HashSet::new();
    let mut in_play: Option<(Ready<u32>, Vec<Vec<ducy::deck::Card>>)> = None;
    let mut dealt = 0;
    for _hand in 0..12 {
        // Who's connected when this deck's setup starts.
        let players: Vec<u32> = seated
            .iter()
            .copied()
            .filter(|p| !away.contains(p))
            .collect();
        // Some of them drop out during setup.
        let mut drop_at = HashMap::new();
        for &p in &players {
            if rng.random_range(0..5) == 0 {
                drop_at.insert(rng.random_range(0..40), p);
            }
        }
        let before = in_play.clone();
        let (ready, parties, dropped) = run_setup(players.clone(), &drop_at, &mut rng);
        // The hand being played wasn't affected by any of it.
        assert_eq!(in_play, before, "the hand in play is never touched");
        for p in &dropped {
            away.insert(*p);
        }
        match ready {
            Some(ready) => {
                // In: exactly those connected who didn't drop out.
                let expect: Vec<u32> = players
                    .iter()
                    .copied()
                    .filter(|p| !dropped.contains(p))
                    .collect();
                assert_eq!(ready.players, expect);
                let (holes, board) = deal(&ready, &parties);
                assert_eq!(holes.len(), expect.len());
                let all: HashSet<String> = holes
                    .iter()
                    .flatten()
                    .chain(&board)
                    .map(|c| c.to_string())
                    .collect();
                assert_eq!(all.len(), expect.len() * 2 + 5);
                in_play = Some((ready, holes));
                dealt += 1;
            }
            None => assert!(
                players.len() - dropped.len() < 2,
                "only waits with fewer than two players"
            ),
        }
        // Players who were away come back before the next setup, at random.
        away.retain(|_| rng.random_range(0..2) == 0);
    }
    assert!(dealt >= 8, "most hands are dealt ({dealt})");
}

#[test]
fn a_nine_player_setup_takes() {
    // Timing for the issue (#140): a whole setup, proofs made and checked, for
    // 9 players and the host (native, debug or release as built).
    let mut rng = StdRng::seed_from_u64(4);
    let t = std::time::Instant::now();
    let (ready, _, _) = run_setup((1..=9).collect(), &HashMap::new(), &mut rng);
    assert!(ready.is_some());
    eprintln!("9-player setup with proofs: {:?}", t.elapsed());
    let _ = Layout::new(9, 2, 5);
}
