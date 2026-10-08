//! The audit after a hand (allenpiscitello/ducy#138): once the hand is over,
//! every party publishes its secret and permutation, and anyone can re-run
//! the whole deal from the record and check that no one cheated.

use ducy::deck::Card;

use crate::{Layout, Masked, Secret, apply_round, decode, open_deck};

/// One party's shuffle round as published after the hand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartyRound {
    pub secret: Secret,
    pub perm: Vec<u32>,
    /// The deck this party sent on.
    pub output: Vec<Masked>,
}

/// One lock removal during the hand: `party` turned `input` (the card at
/// `position` as they received it) into `output`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unlock {
    pub party: usize,
    pub position: usize,
    pub input: Masked,
    pub output: Masked,
}

/// Everything a hand's deal published: the rounds in shuffle order (party
/// `i` is `rounds[i]`: players, then the host), the lock removals in the
/// order they happened, and the cards shown during the hand (board streets
/// and showdowns) with who showed them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transcript {
    pub layout: Layout,
    pub rounds: Vec<PartyRound>,
    pub unlocks: Vec<Unlock>,
    pub revealed: Vec<(usize, Card, usize)>,
}

/// Who cheated, and how.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fault {
    pub party: usize,
    pub kind: FaultKind,
    /// The deck position involved, if it's about one card.
    pub position: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultKind {
    /// A party's round is missing from the record.
    MissingRound,
    /// The published permutation isn't a permutation of the deck.
    NotAPermutation,
    /// The deck a party sent on isn't its secret and permutation applied
    /// to the deck it got (a fake shuffle, or cards swapped in).
    WrongShuffle,
    /// A lock removal that doesn't match the party's secret.
    WrongUnlock,
    /// A lock removal on a card that isn't the one at that position.
    SubstitutedCard,
    /// A card shown that isn't the one dealt at that position.
    WrongReveal,
}

/// Re-checks a hand's deal. `Ok` if everyone played it straight; otherwise
/// the first fault found, naming the party.
pub fn audit(t: &Transcript) -> Result<(), Fault> {
    let fault = |party, kind, position| {
        Err(Fault {
            party,
            kind,
            position,
        })
    };
    let parties = t.layout.players + 1;
    if t.rounds.len() < parties {
        return fault(t.rounds.len(), FaultKind::MissingRound, None);
    }
    // The shuffle: each round from the deck before it.
    let mut deck = open_deck();
    for (party, r) in t.rounds.iter().take(parties).enumerate() {
        match apply_round(&deck, &r.secret, &r.perm) {
            None => return fault(party, FaultKind::NotAPermutation, None),
            Some(expected) if expected != r.output => {
                return fault(party, FaultKind::WrongShuffle, None);
            }
            Some(_) => {}
        }
        deck = r.output.clone();
    }
    // Lock removals, in order: each must start from the card as it stood at
    // that position, and match the party's secret.
    let mut current = deck.clone();
    for u in &t.unlocks {
        if u.party >= parties || u.position >= current.len() {
            return fault(
                u.party.min(parties),
                FaultKind::WrongUnlock,
                Some(u.position),
            );
        }
        if u.input != current[u.position] {
            return fault(u.party, FaultKind::SubstitutedCard, Some(u.position));
        }
        if u.output != t.rounds[u.party].secret.unlock(&u.input) {
            return fault(u.party, FaultKind::WrongUnlock, Some(u.position));
        }
        current[u.position] = u.output;
    }
    // Shown cards: the position with every lock off must be that card.
    for &(pos, card, by) in &t.revealed {
        let open = deck.get(pos).map(|m| {
            t.rounds
                .iter()
                .take(parties)
                .fold(*m, |m, r| r.secret.unlock(&m))
        });
        if open.and_then(|m| decode(&m)) != Some(card) {
            return fault(by, FaultKind::WrongReveal, Some(pos));
        }
    }
    Ok(())
}
