//! Setting up a hand's deck, run by the host while the previous hand is
//! still being played (allenpiscitello/ducy#137), so a disconnect never stops
//! or replays a hand: whoever drops out during setup just isn't in the next
//! hand.
//!
//! [`DeckSetup`] is the host's side. It says what to ask of whom
//! ([`Request`]), checks every answer's proof before going on
//! ([`proof`](crate::proof)), and ends with a deck ready to deal:
//!
//! 1. **Keys:** each party (the players in seat order, then the host) sends
//!    its [`PublicKey`].
//! 2. **Shuffle:** each party in turn locks and shuffles the deck, with a
//!    [`ShuffleProof`].
//! 3. **Lock removal:** each party in turn removes its lock from the cards
//!    [`Layout::unlockers`] gives it, with an [`UnlockProof`]: other players'
//!    hole cards, and (players only) the board.
//! 4. **Ready:** each player's hole cards carry only their own lock, and the
//!    board only the host's ([`Ready`]).
//!
//! **The host keeps the ready deck to itself until the hand starts,** then
//! sends each player their hole cards. Setup runs while the previous hand is
//! still played, and no player may know their next cards before it's over.
//! Nothing a player is sent during setup opens with their key: their own
//! hole cards never pass through them with the other locks off.
//!
//! If a player drops out or sends a bad proof before the deck is ready, their
//! lock is on every card, so the setup starts again without them, with fresh
//! keys ([`Step::Restart`]). Fewer than two players left: [`Step::Waiting`].
//! The host dropping out ends it ([`Step::Failed`]). None of this touches a
//! hand already being played: that hand has its own deck.
//!
//! [`SetupParty`] is a party's side: its secret, its answers to each request,
//! and opening its own hole cards.

use ducy::deck::Card;
use rand::Rng;

use crate::proof::{PublicKey, ShuffleProof, UnlockProof};
use crate::{Layout, Masked, Secret, Unlock, decode, open_deck, shuffle_round};

/// What the host should ask next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request<P> {
    /// Ask every party for a fresh public key for this deck.
    Keys,
    /// Ask `to` to lock and shuffle `deck`.
    Shuffle {
        /// The party to ask.
        to: P,
        /// The deck as it is now.
        deck: Vec<Masked>,
    },
    /// Ask `to` to remove its lock from the cards at `positions`, as they are
    /// now (`cards`).
    Unlock {
        /// The party to ask.
        to: P,
        /// The deck positions.
        positions: Vec<usize>,
        /// The cards at those positions.
        cards: Vec<Masked>,
    },
    /// The deck is ready: see [`DeckSetup::ready`].
    Done,
}

/// What a setup event led to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step<P> {
    /// Go on: ask what [`DeckSetup::request`] says.
    Next,
    /// Start again without `left` (dropped out, or a bad answer): ask everyone
    /// left for new keys.
    Restart {
        /// The player left out.
        left: P,
    },
    /// Fewer than two players left; wait until more are connected.
    Waiting,
    /// The host left; this deck can't be finished.
    Failed,
}

/// A deck ready to deal: who's in (in seat order), the layout, and the deck
/// with only each player's lock on their hole cards and only the host's on
/// the board.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ready<P> {
    /// Who's dealt in, in seat order.
    pub players: Vec<P>,
    /// Whose positions are whose.
    pub layout: Layout,
    /// The deck: each player's hole cards under only their own lock, the
    /// board under only the host's.
    pub deck: Vec<Masked>,
    /// Each party's public key, in party order (players, then the host).
    pub keys: Vec<PublicKey>,
    /// What the audit needs besides the secrets and permutations: each
    /// round's output deck, and the lock removals in order.
    pub outputs: Vec<Vec<Masked>>,
    /// Every lock removed, in order.
    pub unlocks: Vec<Unlock>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Stage {
    Keys,
    Shuffle(usize),
    Unlock(usize),
    Ready,
    Waiting,
    Failed,
}

/// The host's side of setting up one deck.
#[derive(Clone, Debug)]
pub struct DeckSetup<P> {
    players: Vec<P>,
    host: P,
    hole: usize,
    board: usize,
    context: Vec<u8>,
    attempt: u32,
    stage: Stage,
    keys: Vec<Option<PublicKey>>,
    deck: Vec<Masked>,
    outputs: Vec<Vec<Masked>>,
    unlocks: Vec<Unlock>,
}

impl<P: Clone + PartialEq> DeckSetup<P> {
    /// A deck for `players` (seat order) and `host`, `hole` cards each and
    /// `board` on the table. `context` names the table and hand, so proofs
    /// can't be reused elsewhere.
    pub fn new(players: Vec<P>, host: P, hole: usize, board: usize, context: &[u8]) -> Self {
        let mut s = DeckSetup {
            players,
            host,
            hole,
            board,
            context: context.to_vec(),
            attempt: 0,
            stage: Stage::Keys,
            keys: Vec::new(),
            deck: Vec::new(),
            outputs: Vec::new(),
            unlocks: Vec::new(),
        };
        s.reset();
        s
    }

    fn reset(&mut self) {
        self.attempt += 1;
        self.keys = vec![None; self.players.len() + 1];
        self.deck = open_deck();
        self.outputs.clear();
        self.unlocks.clear();
        self.stage = if self.players.len() < 2 {
            Stage::Waiting
        } else {
            Stage::Keys
        };
    }

    /// The players in this deck so far, in seat order.
    pub fn players(&self) -> &[P] {
        &self.players
    }

    fn layout(&self) -> Layout {
        Layout::new(self.players.len(), self.hole, self.board)
    }

    fn parties(&self) -> impl Iterator<Item = &P> {
        self.players.iter().chain(std::iter::once(&self.host))
    }

    fn party(&self, i: usize) -> &P {
        if i < self.players.len() {
            &self.players[i]
        } else {
            &self.host
        }
    }

    fn index(&self, p: &P) -> Option<usize> {
        self.parties().position(|q| q == p)
    }

    /// The context each party's proofs in this attempt are bound to: the
    /// table's, the attempt, and the party's place.
    pub fn context_for(&self, p: &P) -> Option<Vec<u8>> {
        let i = self.index(p)?;
        Some(party_context(&self.context, self.attempt, i))
    }

    /// What to ask next.
    pub fn request(&self) -> Request<P> {
        match self.stage {
            Stage::Keys => Request::Keys,
            Stage::Shuffle(i) => Request::Shuffle {
                to: self.party(i).clone(),
                deck: self.deck.clone(),
            },
            Stage::Unlock(i) => {
                let positions = self.positions_for(i);
                Request::Unlock {
                    to: self.party(i).clone(),
                    cards: positions.iter().map(|&p| self.deck[p]).collect(),
                    positions,
                }
            }
            Stage::Ready => Request::Done,
            Stage::Waiting | Stage::Failed => Request::Keys,
        }
    }

    fn positions_for(&self, party: usize) -> Vec<usize> {
        let layout = self.layout();
        (0..self.deck.len())
            .filter(|&pos| layout.unlockers(pos).contains(&party))
            .collect()
    }

    /// Party `p` sent its public key.
    pub fn key(&mut self, p: &P, key: PublicKey) -> Step<P> {
        let Some(i) = self.index(p) else {
            return Step::Next;
        };
        if self.stage != Stage::Keys {
            return Step::Next;
        }
        self.keys[i] = Some(key);
        if self.keys.iter().all(Option::is_some) {
            self.stage = Stage::Shuffle(0);
        }
        Step::Next
    }

    /// Party `p` sent its shuffled deck: checked against its proof.
    pub fn shuffled(&mut self, p: &P, output: Vec<Masked>, proof: &ShuffleProof) -> Step<P> {
        let Stage::Shuffle(i) = self.stage else {
            return Step::Next;
        };
        if self.index(p) != Some(i) {
            return Step::Next;
        }
        let key = self.keys[i].expect("every key is in");
        if output.len() != self.deck.len()
            || !proof.verify(
                &key,
                &self.deck,
                &output,
                &party_context(&self.context, self.attempt, i),
            )
        {
            return self.drop_party(p.clone());
        }
        self.deck = output.clone();
        self.outputs.push(output);
        self.stage = if i + 1 < self.players.len() + 1 {
            Stage::Shuffle(i + 1)
        } else {
            self.first_unlock(0)
        };
        Step::Next
    }

    fn first_unlock(&self, from: usize) -> Stage {
        (from..=self.players.len())
            .find(|&q| !self.positions_for(q).is_empty())
            .map_or(Stage::Ready, Stage::Unlock)
    }

    /// Party `p` removed its lock from the cards it was asked to: checked
    /// against its proof.
    pub fn unlocked(&mut self, p: &P, outputs: Vec<Masked>, proof: &UnlockProof) -> Step<P> {
        let Stage::Unlock(i) = self.stage else {
            return Step::Next;
        };
        if self.index(p) != Some(i) {
            return Step::Next;
        }
        let positions = self.positions_for(i);
        let key = self.keys[i].expect("every key is in");
        let pairs: Vec<(Masked, Masked)> = positions
            .iter()
            .map(|&pos| self.deck[pos])
            .zip(outputs.iter().copied())
            .collect();
        if outputs.len() != positions.len()
            || !proof.verify(&key, &pairs, &party_context(&self.context, self.attempt, i))
        {
            return self.drop_party(p.clone());
        }
        for (&pos, (input, output)) in positions.iter().zip(pairs) {
            self.unlocks.push(Unlock {
                party: i,
                position: pos,
                input,
                output,
            });
            self.deck[pos] = output;
        }
        self.stage = self.first_unlock(i + 1);
        Step::Next
    }

    /// Party `p` disconnected (or must be left out). Before the deck is ready
    /// a player's lock is on every card, so the setup starts again without
    /// them; the host leaving ends it. Once the deck is ready, nothing
    /// changes: they're in that hand, and the table's rules deal with them.
    pub fn drop_party(&mut self, p: P) -> Step<P> {
        if matches!(self.stage, Stage::Ready | Stage::Failed) {
            return Step::Next;
        }
        if p == self.host {
            self.stage = Stage::Failed;
            return Step::Failed;
        }
        let Some(i) = self.players.iter().position(|q| *q == p) else {
            return Step::Next;
        };
        self.players.remove(i);
        self.reset();
        if self.stage == Stage::Waiting {
            Step::Waiting
        } else {
            Step::Restart { left: p }
        }
    }

    /// A player connected again (or newly sat down) and wants in. Only before
    /// the shuffle starts; otherwise they're in the next deck.
    pub fn join(&mut self, p: P, seat_order: impl Fn(&P, &P) -> std::cmp::Ordering) -> bool {
        if !matches!(self.stage, Stage::Keys | Stage::Waiting)
            || self.players.contains(&p)
            || p == self.host
        {
            return false;
        }
        self.players.push(p);
        self.players.sort_by(&seat_order);
        self.reset();
        true
    }

    /// The deck, once ready.
    pub fn ready(&self) -> Option<Ready<P>> {
        (self.stage == Stage::Ready).then(|| Ready {
            players: self.players.clone(),
            layout: self.layout(),
            deck: self.deck.clone(),
            keys: self
                .keys
                .iter()
                .map(|k| k.expect("every key is in"))
                .collect(),
            outputs: self.outputs.clone(),
            unlocks: self.unlocks.clone(),
        })
    }
}

fn party_context(table: &[u8], attempt: u32, party: usize) -> Vec<u8> {
    let mut c = table.to_vec();
    c.extend_from_slice(b"/attempt/");
    c.extend_from_slice(&attempt.to_be_bytes());
    c.extend_from_slice(b"/party/");
    c.extend_from_slice(&(party as u32).to_be_bytes());
    c
}

/// One party's side of a deck setup: its secret for this deck, the
/// permutation it used (both published after the hand, for the audit), and
/// its answers to the host's requests.
#[derive(Clone, Debug)]
pub struct SetupParty {
    secret: Secret,
    perm: Vec<u32>,
    context: Vec<u8>,
}

impl SetupParty {
    /// A party with a fresh secret, for proofs bound to `context`
    /// ([`DeckSetup::context_for`]).
    pub fn new<R: Rng + ?Sized>(context: &[u8], rng: &mut R) -> Self {
        SetupParty {
            secret: Secret::from_rng(rng),
            perm: Vec::new(),
            context: context.to_vec(),
        }
    }

    /// A party saved with [`SetupParty::reveal`] (and its context), e.g. a
    /// host restarting mid-hand that still has the board to unlock. Keep
    /// the saved secret where only this party can read it.
    pub fn restore(context: &[u8], secret: Secret, perm: Vec<u32>) -> Self {
        SetupParty {
            secret,
            perm,
            context: context.to_vec(),
        }
    }

    /// The public key to send first.
    pub fn key(&self) -> PublicKey {
        self.secret.public()
    }

    /// Locks and shuffles `deck`: the new deck and its proof.
    pub fn shuffle<R: Rng + ?Sized>(
        &mut self,
        deck: &[Masked],
        rng: &mut R,
    ) -> (Vec<Masked>, ShuffleProof) {
        let (out, perm) = shuffle_round(deck, &self.secret, rng);
        let proof = ShuffleProof::prove(&self.secret, &perm, deck, &out, &self.context, rng);
        self.perm = perm;
        (out, proof)
    }

    /// Removes this party's lock from `cards`: the results and their proof.
    pub fn unlock<R: Rng + ?Sized>(
        &self,
        cards: &[Masked],
        rng: &mut R,
    ) -> (Vec<Masked>, UnlockProof) {
        let out: Vec<Masked> = cards.iter().map(|c| self.secret.unlock(c)).collect();
        let pairs: Vec<(Masked, Masked)> = cards.iter().copied().zip(out.iter().copied()).collect();
        let proof = UnlockProof::prove(&self.secret, &pairs, &self.context, rng);
        (out, proof)
    }

    /// Opens this player's own hole cards (only their lock left on them);
    /// `None` if one isn't a card (someone cheated).
    pub fn open(&self, cards: &[Masked]) -> Option<Vec<Card>> {
        cards
            .iter()
            .map(|c| decode(&self.secret.unlock(c)))
            .collect()
    }

    /// The secret and permutation, to publish after the hand.
    pub fn reveal(&self) -> (Secret, Vec<u32>) {
        (self.secret, self.perm.clone())
    }
}
