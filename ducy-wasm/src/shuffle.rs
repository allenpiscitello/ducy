//! The trustless shuffle (ducy-shuffle, allenpiscitello/ducy#134) for the
//! page: a party's side in a player's browser (`ShuffleParty`), the host's
//! side that runs and checks a deck's setup (`ShuffleSetup`), and the audit
//! after a hand (`auditHand`). Cards, keys, secrets and proofs travel as hex
//! strings; contexts are any string naming the table and hand.

use ducy::deck::Card;
use ducy_shuffle::{
    DeckSetup, Layout, Masked, PartyRound, PublicKey, Request, Secret, SetupParty, ShuffleProof,
    Step, Transcript, Unlock, UnlockProof, audit, open_deck,
};
use rand::rngs::StdRng;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

fn err(msg: impl std::fmt::Display) -> JsError {
    JsError::new(&msg.to_string())
}

fn to_js(value: &impl Serialize) -> Result<JsValue, JsError> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(err)
}

fn rng() -> StdRng {
    rand::make_rng()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Result<Vec<u8>, JsError> {
    if s.len() % 2 != 0 {
        return Err(err("not hex"));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| err("not hex")))
        .collect()
}

fn bytes32(s: &str) -> Result<[u8; 32], JsError> {
    unhex(s)?.try_into().map_err(|_| err("expected 32 bytes"))
}

fn card(s: &str) -> Result<Masked, JsError> {
    Masked::from_bytes(&bytes32(s)?).ok_or_else(|| err("not a card point"))
}

fn cards(list: &[String]) -> Result<Vec<Masked>, JsError> {
    list.iter().map(|s| card(s)).collect()
}

fn hexes(list: &[Masked]) -> Vec<String> {
    list.iter().map(|m| hex(&m.to_bytes())).collect()
}

fn key(s: &str) -> Result<PublicKey, JsError> {
    PublicKey::from_bytes(&bytes32(s)?).ok_or_else(|| err("not a public key"))
}

/// The 52 cards, unlocked: where a deck's first shuffle starts.
#[wasm_bindgen(js_name = shuffleOpenDeck)]
pub fn shuffle_open_deck() -> Vec<String> {
    hexes(&open_deck())
}

#[derive(Serialize)]
struct Shuffled {
    deck: Vec<String>,
    proof: String,
}

#[derive(Serialize)]
struct Unlocked {
    cards: Vec<String>,
    proof: String,
}

#[derive(Serialize)]
struct Revealed {
    secret: String,
    perm: Vec<u32>,
}

/// One party's side of a deck setup, in its own browser (a player) or the
/// host's: a fresh secret for this deck, never sent until the hand is over.
#[wasm_bindgen]
pub struct ShuffleParty {
    party: SetupParty,
}

#[wasm_bindgen]
impl ShuffleParty {
    /// A party for one deck. `context` is what the host's `contextFor` gave
    /// this party (hex).
    #[wasm_bindgen(constructor)]
    pub fn new(context: &str) -> Result<ShuffleParty, JsError> {
        Ok(ShuffleParty {
            party: SetupParty::new(&unhex(context)?, &mut rng()),
        })
    }

    /// The public key to send the host first (hex).
    pub fn key(&self) -> String {
        hex(&self.party.key().to_bytes())
    }

    /// Locks and shuffles `deck`: {deck, proof}.
    pub fn shuffle(&mut self, deck: Vec<String>) -> Result<JsValue, JsError> {
        let (out, proof) = self.party.shuffle(&cards(&deck)?, &mut rng());
        to_js(&Shuffled {
            deck: hexes(&out),
            proof: hex(&proof.to_bytes()),
        })
    }

    /// Removes this party's lock from `cards`: {cards, proof}.
    pub fn unlock(&self, cards_in: Vec<String>) -> Result<JsValue, JsError> {
        let (out, proof) = self.party.unlock(&cards(&cards_in)?, &mut rng());
        to_js(&Unlocked {
            cards: hexes(&out),
            proof: hex(&proof.to_bytes()),
        })
    }

    /// This player's own hole cards, opened ("As", "Kd", …). Throws if one
    /// isn't a card: someone cheated.
    pub fn open(&self, cards_in: Vec<String>) -> Result<Vec<String>, JsError> {
        let opened = self
            .party
            .open(&cards(&cards_in)?)
            .ok_or_else(|| err("a card that doesn't open: the deal was tampered with"))?;
        Ok(opened.iter().map(Card::to_string).collect())
    }

    /// The secret and permutation, published after the hand for the audit
    /// (and the secret at showdown): {secret, perm}.
    pub fn reveal(&self) -> Result<JsValue, JsError> {
        let (secret, perm) = self.party.reveal();
        to_js(&Revealed {
            secret: hex(&secret.to_bytes()),
            perm,
        })
    }
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RequestOut {
    Keys,
    Shuffle {
        to: String,
        deck: Vec<String>,
    },
    Unlock {
        to: String,
        positions: Vec<usize>,
        cards: Vec<String>,
    },
    Done,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum StepOut {
    Next,
    Restart { left: String },
    Waiting,
    Failed,
}

fn step(s: Step<String>) -> StepOut {
    match s {
        Step::Next => StepOut::Next,
        Step::Restart { left } => StepOut::Restart { left },
        Step::Waiting => StepOut::Waiting,
        Step::Failed => StepOut::Failed,
    }
}

#[derive(Serialize, Deserialize)]
struct UnlockRec {
    party: usize,
    position: usize,
    input: String,
    output: String,
}

#[derive(Serialize)]
struct ReadyOut {
    players: Vec<String>,
    hole: usize,
    board: usize,
    deck: Vec<String>,
    keys: Vec<String>,
    outputs: Vec<Vec<String>>,
    unlocks: Vec<UnlockRec>,
}

/// The host's side of setting up one deck (the players in seat order, then
/// the host): what to ask whom next, each answer's proof checked before
/// going on, and restarting without anyone who drops out.
#[wasm_bindgen]
pub struct ShuffleSetup {
    setup: DeckSetup<String>,
    order: Vec<String>,
}

#[wasm_bindgen]
impl ShuffleSetup {
    /// A deck for `players` (seat order) and `host`: `hole` cards each and
    /// `board` on the table. `context` names the table and hand.
    #[wasm_bindgen(constructor)]
    pub fn new(
        players: Vec<String>,
        host: String,
        hole: usize,
        board: usize,
        context: &str,
    ) -> Result<ShuffleSetup, JsError> {
        if players.len() * hole + board > 52 {
            return Err(err("not enough cards"));
        }
        Ok(ShuffleSetup {
            order: players.clone(),
            setup: DeckSetup::new(players, host, hole, board, context.as_bytes()),
        })
    }

    /// What to ask next: {kind: 'keys'}, {kind: 'shuffle', to, deck},
    /// {kind: 'unlock', to, positions, cards}, or {kind: 'done'}.
    pub fn request(&self) -> Result<JsValue, JsError> {
        to_js(&match self.setup.request() {
            Request::Keys => RequestOut::Keys,
            Request::Shuffle { to, deck } => RequestOut::Shuffle {
                to,
                deck: hexes(&deck),
            },
            Request::Unlock {
                to,
                positions,
                cards,
            } => RequestOut::Unlock {
                to,
                positions,
                cards: hexes(&cards),
            },
            Request::Done => RequestOut::Done,
        })
    }

    /// The players in this deck so far, in seat order.
    pub fn players(&self) -> Vec<String> {
        self.setup.players().to_vec()
    }

    /// The context (hex) party `id` makes its `ShuffleParty` with, for this
    /// attempt.
    #[wasm_bindgen(js_name = contextFor)]
    pub fn context_for(&self, id: &str) -> Option<String> {
        self.setup.context_for(&id.to_string()).map(|c| hex(&c))
    }

    /// Party `id` sent its public key. Returns a step ({kind: 'next' |
    /// 'restart' (with left) | 'waiting' | 'failed'}).
    pub fn key(&mut self, id: &str, public_key: &str) -> Result<JsValue, JsError> {
        let k = key(public_key)?;
        to_js(&step(self.setup.key(&id.to_string(), k)))
    }

    /// Party `id` sent its shuffled deck and proof: checked, and a bad one
    /// leaves them out.
    pub fn shuffled(
        &mut self,
        id: &str,
        deck: Vec<String>,
        proof: &str,
    ) -> Result<JsValue, JsError> {
        let id = id.to_string();
        let s = match (cards(&deck), ShuffleProof::from_bytes(&unhex(proof)?)) {
            (Ok(deck), Some(proof)) => self.setup.shuffled(&id, deck, &proof),
            _ => self.setup.drop_party(id),
        };
        to_js(&step(s))
    }

    /// Party `id` sent the cards with its lock removed, and the proof.
    pub fn unlocked(
        &mut self,
        id: &str,
        cards_out: Vec<String>,
        proof: &str,
    ) -> Result<JsValue, JsError> {
        let id = id.to_string();
        let s = match (cards(&cards_out), UnlockProof::from_bytes(&unhex(proof)?)) {
            (Ok(out), Some(proof)) => self.setup.unlocked(&id, out, &proof),
            _ => self.setup.drop_party(id),
        };
        to_js(&step(s))
    }

    /// Party `id` disconnected.
    #[wasm_bindgen(js_name = dropParty)]
    pub fn drop_party(&mut self, id: &str) -> Result<JsValue, JsError> {
        to_js(&step(self.setup.drop_party(id.to_string())))
    }

    /// Player `id` is back (or sat down): in, if the shuffle hasn't started.
    /// `seats` is every player's id in seat order.
    pub fn join(&mut self, id: &str, seats: Vec<String>) -> bool {
        let order = if seats.is_empty() {
            self.order.clone()
        } else {
            seats
        };
        let pos = |p: &String| order.iter().position(|q| q == p).unwrap_or(usize::MAX);
        self.setup.join(id.to_string(), |a, b| pos(a).cmp(&pos(b)))
    }

    /// The deck once ready, or null: {players, hole, board, deck, keys,
    /// outputs, unlocks}. Each player gets `deck` at their hole positions
    /// (player i: i×hole … (i+1)×hole − 1); the board follows them.
    pub fn ready(&self) -> Result<JsValue, JsError> {
        let Some(r) = self.setup.ready() else {
            return Ok(JsValue::NULL);
        };
        to_js(&ReadyOut {
            players: r.players,
            hole: r.layout.hole,
            board: r.layout.board,
            deck: hexes(&r.deck),
            keys: r.keys.iter().map(|k| hex(&k.to_bytes())).collect(),
            outputs: r.outputs.iter().map(|o| hexes(o)).collect(),
            unlocks: r
                .unlocks
                .iter()
                .map(|u| UnlockRec {
                    party: u.party,
                    position: u.position,
                    input: hex(&u.input.to_bytes()),
                    output: hex(&u.output.to_bytes()),
                })
                .collect(),
        })
    }
}

/// Removes the host's lock from board cards as a street is dealt, checked:
/// whether `outputs` are `inputs` unlocked by `publicKey`'s secret.
#[wasm_bindgen(js_name = shuffleVerifyUnlock)]
pub fn shuffle_verify_unlock(
    public_key: &str,
    inputs: Vec<String>,
    outputs: Vec<String>,
    proof: &str,
    context: &str,
) -> Result<bool, JsError> {
    let (k, i, o) = (key(public_key)?, cards(&inputs)?, cards(&outputs)?);
    if i.len() != o.len() {
        return Ok(false);
    }
    let pairs: Vec<_> = i.into_iter().zip(o).collect();
    let ctx = unhex(context)?;
    Ok(UnlockProof::from_bytes(&unhex(proof)?).is_some_and(|p| p.verify(&k, &pairs, &ctx)))
}

/// Whether a secret published at showdown is the one behind `publicKey`.
#[wasm_bindgen(js_name = shuffleSecretMatches)]
pub fn shuffle_secret_matches(public_key: &str, secret: &str) -> Result<bool, JsError> {
    let s = Secret::from_bytes(&bytes32(secret)?).ok_or_else(|| err("not a secret"))?;
    Ok(key(public_key)?.matches(&s))
}

/// Opens cards with published secrets (at showdown): `secrets` are the
/// parties whose locks are still on the cards. Null where a card doesn't open.
#[wasm_bindgen(js_name = shuffleOpenWith)]
pub fn shuffle_open_with(
    cards_in: Vec<String>,
    secrets: Vec<String>,
) -> Result<Vec<String>, JsError> {
    let secrets = secrets
        .iter()
        .map(|s| Secret::from_bytes(&bytes32(s)?).ok_or_else(|| err("not a secret")))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(cards(&cards_in)?
        .iter()
        .map(|m| {
            ducy_shuffle::decode(&secrets.iter().fold(*m, |m, s| s.unlock(&m)))
                .map_or(String::new(), |c| c.to_string())
        })
        .collect())
}

#[derive(Deserialize)]
struct RoundIn {
    secret: String,
    perm: Vec<u32>,
    output: Vec<String>,
}

#[derive(Deserialize)]
struct RevealIn {
    position: usize,
    card: String,
    by: usize,
}

#[derive(Deserialize)]
struct TranscriptIn {
    players: usize,
    hole: usize,
    board: usize,
    rounds: Vec<RoundIn>,
    unlocks: Vec<UnlockRec>,
    #[serde(default)]
    revealed: Vec<RevealIn>,
}

#[derive(Serialize)]
struct AuditOut {
    ok: bool,
    party: Option<usize>,
    fault: Option<String>,
    position: Option<usize>,
}

/// The audit after a hand (#138): re-runs the whole deal from what every
/// party published and checks it. {ok: true}, or {ok: false, party, fault,
/// position} naming who cheated and how.
#[wasm_bindgen(js_name = auditHand)]
pub fn audit_hand(transcript: JsValue) -> Result<JsValue, JsError> {
    let t: TranscriptIn = serde_wasm_bindgen::from_value(transcript).map_err(err)?;
    if t.players * t.hole + t.board > 52 {
        return Err(err("not enough cards"));
    }
    let rounds = t
        .rounds
        .iter()
        .map(|r| {
            Ok(PartyRound {
                secret: Secret::from_bytes(&bytes32(&r.secret)?)
                    .ok_or_else(|| err("not a secret"))?,
                perm: r.perm.clone(),
                output: cards(&r.output)?,
            })
        })
        .collect::<Result<Vec<_>, JsError>>()?;
    let unlocks = t
        .unlocks
        .iter()
        .map(|u| {
            Ok(Unlock {
                party: u.party,
                position: u.position,
                input: card(&u.input)?,
                output: card(&u.output)?,
            })
        })
        .collect::<Result<Vec<_>, JsError>>()?;
    let revealed = t
        .revealed
        .iter()
        .map(|r| Ok((r.position, Card::parse(&r.card).map_err(err)?, r.by)))
        .collect::<Result<Vec<_>, JsError>>()?;
    let transcript = Transcript {
        layout: Layout::new(t.players, t.hole, t.board),
        rounds,
        unlocks,
        revealed,
    };
    to_js(&match audit(&transcript) {
        Ok(()) => AuditOut {
            ok: true,
            party: None,
            fault: None,
            position: None,
        },
        Err(f) => AuditOut {
            ok: false,
            party: Some(f.party),
            fault: Some(format!("{:?}", f.kind)),
            position: f.position,
        },
    })
}
