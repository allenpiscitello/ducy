use std::collections::HashSet;

use rand::{SeedableRng, rngs::StdRng};

use super::*;

/// A whole deal between `layout.players` players and the host, as the
/// parties would run it, recorded for the audit: shuffle rounds, setup lock
/// removals, each player opening their cards, and the host dealing the
/// board. Returns the transcript, each player's hole cards, and the values
/// each player and the host saw at positions that aren't theirs.
struct Dealt {
    transcript: Transcript,
    hole: Vec<Vec<Card>>,
    board: Vec<Card>,
    /// Hole positions after setup: only the owner's lock is left.
    hole_after_setup: Vec<Masked>,
    /// Board positions after setup: only the host's lock is left.
    board_after_setup: Vec<Masked>,
    secrets: Vec<Secret>,
}

fn deal(layout: Layout, seed: u64) -> Dealt {
    let mut rng = StdRng::seed_from_u64(seed);
    let parties = layout.players + 1;
    let secrets: Vec<Secret> = (0..parties).map(|_| Secret::from_rng(&mut rng)).collect();
    let mut deck = open_deck();
    let mut rounds = Vec::new();
    for s in &secrets {
        let (out, perm) = shuffle_round(&deck, s, &mut rng);
        rounds.push(PartyRound {
            secret: *s,
            perm,
            output: out.clone(),
        });
        deck = out;
    }
    let mut unlocks = Vec::new();
    let mut current = deck.clone();
    let mut remove = |party: usize, pos: usize, current: &mut Vec<Masked>| {
        let input = current[pos];
        let output = secrets[party].unlock(&input);
        unlocks.push(Unlock {
            party,
            position: pos,
            input,
            output,
        });
        current[pos] = output;
    };
    // Setup: everyone else off each hole card, every player off the board.
    for pos in 0..layout.board().end {
        for party in layout.unlockers(pos) {
            remove(party, pos, &mut current);
        }
    }
    let hole_after_setup: Vec<Masked> = (0..layout.players)
        .flat_map(|p| layout.hole(p))
        .map(|pos| current[pos])
        .collect();
    let board_after_setup: Vec<Masked> = layout.board().map(|pos| current[pos]).collect();
    // Each player opens their own (privately: not part of the record).
    let hole = (0..layout.players)
        .map(|p| {
            layout
                .hole(p)
                .map(|pos| decode(&secrets[p].unlock(&current[pos])).expect("own card"))
                .collect()
        })
        .collect();
    // The host deals the board, street by street.
    let mut revealed = Vec::new();
    let mut board = Vec::new();
    for pos in layout.board() {
        remove(layout.host(), pos, &mut current);
        let card = decode(&current[pos]).expect("a board card");
        revealed.push((pos, card, layout.host()));
        board.push(card);
    }
    Dealt {
        transcript: Transcript {
            layout,
            rounds,
            unlocks,
            revealed,
        },
        hole,
        board,
        hole_after_setup,
        board_after_setup,
        secrets,
    }
}

#[test]
fn nine_players_and_the_host_each_player_sees_only_their_own_cards() {
    let layout = Layout::new(9, 2, 5);
    let d = deal(layout, 1);
    // Every card dealt is a different card.
    let all: Vec<Card> = d
        .hole
        .iter()
        .flatten()
        .chain(d.board.iter())
        .copied()
        .collect();
    assert_eq!(all.len(), 23);
    assert_eq!(
        all.iter()
            .map(|c| c.to_string())
            .collect::<HashSet<_>>()
            .len(),
        23
    );
    // The host can't open any hole card: only the owner's lock is left, and
    // the host has removed its own.
    for (i, m) in d.hole_after_setup.iter().enumerate() {
        assert_eq!(decode(m), None, "hole card {i} readable as it travels");
        assert_eq!(
            decode(&d.secrets[layout.host()].unlock(m)),
            None,
            "the host can't open hole card {i}"
        );
    }
    // No player can open another's hole cards, or a board card before the host deals it.
    for p in 0..layout.players {
        for (i, m) in d.hole_after_setup.iter().enumerate() {
            if !layout.hole(p).contains(&i) {
                assert_eq!(
                    decode(&d.secrets[p].unlock(m)),
                    None,
                    "player {p} opened hole card {i}"
                );
            }
        }
        for (i, m) in d.board_after_setup.iter().enumerate() {
            assert_eq!(decode(m), None);
            assert_eq!(
                decode(&d.secrets[p].unlock(m)),
                None,
                "player {p} saw board card {i} early"
            );
        }
    }
}

#[test]
fn every_card_appears_exactly_once_after_any_number_of_rounds() {
    for parties in [1, 2, 5, 10] {
        let mut rng = StdRng::seed_from_u64(parties as u64);
        let secrets: Vec<Secret> = (0..parties).map(|_| Secret::from_rng(&mut rng)).collect();
        let mut deck = open_deck();
        for s in &secrets {
            deck = shuffle_round(&deck, s, &mut rng).0;
        }
        // Locks come off in any order: here, backwards.
        let open: HashSet<String> = deck
            .iter()
            .map(|m| secrets.iter().rev().fold(*m, |m, s| s.unlock(&m)))
            .map(|m| decode(&m).expect("a card").to_string())
            .collect();
        assert_eq!(open.len(), 52, "{parties} parties");
    }
}

#[test]
fn at_showdown_a_published_secret_shows_that_players_cards_to_everyone() {
    let layout = Layout::new(3, 2, 5);
    let d = deal(layout, 7);
    let published = Secret::from_bytes(&d.secrets[1].to_bytes()).unwrap();
    let shown: Vec<Card> = layout
        .hole(1)
        .map(|pos| decode(&published.unlock(&d.hole_after_setup[pos])).unwrap())
        .collect();
    assert_eq!(shown, d.hole[1]);
}

#[test]
fn omaha_layouts_work_too() {
    let layout = Layout::new(6, 4, 5);
    let d = deal(layout, 3);
    assert!(d.hole.iter().all(|h| h.len() == 4));
    assert_eq!(audit(&d.transcript), Ok(()));
}

#[test]
fn bytes_round_trip_and_reject_garbage() {
    let m = open_deck()[5];
    assert_eq!(Masked::from_bytes(&m.to_bytes()), Some(m));
    assert_eq!(Masked::from_bytes(&[0xff; 32]), None);
    let s = Secret::random();
    assert_eq!(Secret::from_bytes(&s.to_bytes()), Some(s));
    assert_eq!(Secret::from_bytes(&[0; 32]), None);
    assert_eq!(Secret::from_bytes(&[0xff; 32]), None, "not canonical");
}

#[test]
fn layout_says_who_unlocks_what() {
    let l = Layout::new(3, 2, 5);
    assert_eq!(l.host(), 3);
    assert_eq!(
        l.unlockers(0),
        vec![1, 2, 3],
        "player 0's card: everyone else, host too"
    );
    assert_eq!(l.unlockers(5), vec![0, 1, 3]);
    assert_eq!(l.unlockers(6), vec![0, 1, 2], "a board card: players only");
    assert!(l.unlockers(11).is_empty(), "not dealt");
}

// ---- the audit (#138) ----

#[test]
fn an_honest_hand_passes_the_audit() {
    for seed in 0..5 {
        assert_eq!(audit(&deal(Layout::new(4, 2, 5), seed).transcript), Ok(()));
    }
}

fn fault_of(t: &Transcript) -> (usize, FaultKind) {
    let f = audit(t).expect_err("cheating should be caught");
    (f.party, f.kind)
}

#[test]
fn a_fake_shuffle_is_caught_and_names_the_shuffler() {
    let mut t = deal(Layout::new(4, 2, 5), 11).transcript;
    // Party 2 claims a permutation but sends the deck in another order.
    t.rounds[2].output.swap(0, 1);
    assert_eq!(fault_of(&t), (2, FaultKind::WrongShuffle));
}

#[test]
fn a_permutation_that_isnt_one_is_caught() {
    let mut t = deal(Layout::new(4, 2, 5), 12).transcript;
    t.rounds[1].perm[0] = t.rounds[1].perm[1];
    assert_eq!(fault_of(&t), (1, FaultKind::NotAPermutation));
}

#[test]
fn a_substituted_card_is_caught() {
    // The host swaps in a card from another round: its shuffle output no
    // longer matches its secret and permutation.
    let mut t = deal(Layout::new(4, 2, 5), 13).transcript;
    let host = t.layout.host();
    let stolen = t.rounds[host - 1].output[0];
    t.rounds[host].output[3] = stolen;
    assert_eq!(fault_of(&t), (host, FaultKind::WrongShuffle));
}

#[test]
fn a_wrong_unlock_is_caught_and_names_who_did_it() {
    let mut t = deal(Layout::new(4, 2, 5), 14).transcript;
    // The 5th removal was done with some other key.
    let other = Secret::from_rng(&mut StdRng::seed_from_u64(99));
    let u = &mut t.unlocks[4];
    u.output = other.unlock(&u.input);
    let who = u.party;
    let (party, kind) = fault_of(&t);
    assert_eq!((party, kind), (who, FaultKind::WrongUnlock));
}

#[test]
fn unlocking_a_different_card_than_the_one_at_that_position_is_caught() {
    let mut t = deal(Layout::new(4, 2, 5), 15).transcript;
    // A party removes its lock from a card it took from elsewhere and passes
    // that off as position 0.
    let i = t.unlocks.iter().position(|u| u.position == 0).unwrap();
    let party = t.unlocks[i].party;
    let elsewhere = t.rounds.last().unwrap().output[40];
    t.unlocks[i].input = elsewhere;
    t.unlocks[i].output = t.rounds[party].secret.unlock(&elsewhere);
    assert_eq!(fault_of(&t), (party, FaultKind::SubstitutedCard));
}

#[test]
fn a_host_revealing_a_board_card_that_wasnt_dealt_is_caught() {
    let mut t = deal(Layout::new(4, 2, 5), 16).transcript;
    let host = t.layout.host();
    // The host shows the turn as a card from the undealt rest of the deck.
    let undealt = {
        let shown: HashSet<String> = t.revealed.iter().map(|r| r.1.to_string()).collect();
        Deck::all_cards()
            .iter(false)
            .find(|c| !shown.contains(&c.to_string()))
            .unwrap()
    };
    t.revealed[3].1 = undealt;
    let (party, kind) = fault_of(&t);
    assert_eq!((party, kind), (host, FaultKind::WrongReveal));
}

#[test]
fn a_missing_round_is_caught() {
    let mut t = deal(Layout::new(3, 2, 5), 17).transcript;
    t.rounds.pop();
    assert_eq!(fault_of(&t), (3, FaultKind::MissingRound));
}
