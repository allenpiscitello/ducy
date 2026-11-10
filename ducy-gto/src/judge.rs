//! Scoring players' decisions at real tables against the heads-up models,
//! for fair play checks: how often each player makes the model's plays after
//! the flop, and, in Hold'em, what their plays give up by its values. Cards
//! go in; only numbers come out.
//!
//! - **Which hands:** pots that two players took to the flop, at a table of
//!   any size. A multiway hand is seen as the heads-up hand between those two
//!   ([`heads_up_history`]: the player in position is the heads-up button);
//!   the chips of players who folded before the flop stay in the pot, as
//!   antes split between the two, so bet sizes keep their real share of it.
//! - **Hold'em:** each of the two players' decisions is reviewed with the
//!   hand review ([`Reviewer`]): how often the blueprint makes their play
//!   with their hand, its most frequent play's frequency, and the big blinds
//!   their play gives up against the best one, valued against the other
//!   player's range with both following the blueprint after.
//! - **Omaha:** the heads-up PLO blueprint's frequencies alone (there is no
//!   PLO action valuation yet): the hand is followed on its tree as
//!   [`PloGtoBot`](crate::omaha::bot::PloGtoBot) follows it, and each
//!   decision looked up in the player's bucket.
//!
//! Only decisions from the flop on are scored: preflop, a heads-up model
//! says little about a six-handed table. Like the hand review, this measures
//! play against one blueprint within its abstraction, not perfect play.

use ducy_play::Event;

use crate::{
    Rng,
    holdem::{
        blueprint::Blueprint,
        cards::Card,
        follow::Follower,
        hunl::{BettingTree, Buckets},
        range::BucketCache,
        review::{Grade, HandRecord, Reviewer},
    },
    omaha::abstraction::PloAbstraction,
    sixmax::bot::heads_up_history,
};

/// A finished hand at a table of any size. Seats are indexes into `stacks`,
/// as in the hand's events.
#[derive(Clone, Debug, PartialEq)]
pub struct TableHand {
    pub history: Vec<Event>,
    pub button: usize,
    pub small_blind: u64,
    pub big_blind: u64,
    /// Every seat's stack when the hand began.
    pub stacks: Vec<u64>,
    /// Every seat's hole cards, where known.
    pub holes: Vec<Option<Vec<Card>>>,
    /// The board as far as it was dealt.
    pub board: Vec<Card>,
}

/// A hand as the heads-up hand between the two players who saw the flop.
#[derive(Clone, Debug, PartialEq)]
pub struct HeadsUp {
    /// The real seats of heads-up seat 0 (in position, the button) and 1.
    pub seats: [usize; 2],
    pub history: Vec<Event>,
    /// Their stacks when the hand began, as heads-up seats.
    pub stacks: [u64; 2],
}

impl HeadsUp {
    /// The smaller of the two stacks, in big blinds.
    pub fn depth(&self, big_blind: u64) -> f64 {
        self.stacks[0].min(self.stacks[1]) as f64 / big_blind.max(1) as f64
    }
}

/// `hand` as a heads-up hand, if exactly two players saw the flop (or it was
/// heads-up from the start) and a flop was dealt. None otherwise.
pub fn heads_up(hand: &TableHand) -> Option<HeadsUp> {
    let n = hand.stacks.len();
    let flop = hand
        .history
        .iter()
        .position(|e| matches!(e, Event::Board { .. }))?;
    let mut folded = vec![false; n];
    // Chips put in before the flop by players who then folded.
    let mut put = vec![0u64; n];
    let mut street_bet = vec![0u64; n];
    for e in &hand.history[..flop] {
        match *e {
            Event::Fold { seat } if seat < n => folded[seat] = true,
            Event::Ante { seat, amount } if seat < n => put[seat] += amount,
            Event::Post { seat, dead, live } if seat < n => {
                put[seat] += dead + live;
                street_bet[seat] += live;
            }
            Event::SmallBlind { seat, amount }
            | Event::BigBlind { seat, amount }
            | Event::Call { seat, amount, .. }
                if seat < n =>
            {
                put[seat] += amount;
                street_bet[seat] += amount;
            }
            Event::Bet { seat, to, .. } | Event::Raise { seat, to, .. } if seat < n => {
                put[seat] += to.saturating_sub(street_bet[seat]);
                street_bet[seat] = to;
            }
            _ => {}
        }
    }
    let live: Vec<usize> = (0..n).filter(|&s| !folded[s]).collect();
    let &[a, b] = live.as_slice() else {
        return None;
    };
    let ([ip, oop], mut history) = if n == 2 {
        // Heads-up from the start: the button is seat 0 already.
        let ip = hand.button % 2;
        let hu = |s: usize| usize::from(s != ip);
        let history = hand.history.iter().map(|e| with_seat(e, hu)).collect();
        ([ip, 1 - ip], history)
    } else {
        heads_up_history(
            &hand.history,
            n,
            hand.button,
            hand.small_blind,
            hand.big_blind,
            [a, b],
        )?
    };
    if n > 2 {
        let dead: u64 = (0..n).filter(|&s| folded[s]).map(|s| put[s]).sum();
        if dead > 0 {
            history.splice(
                0..0,
                [
                    Event::Ante {
                        seat: 0,
                        amount: dead / 2,
                    },
                    Event::Ante {
                        seat: 1,
                        amount: dead - dead / 2,
                    },
                ],
            );
        }
    }
    Some(HeadsUp {
        seats: [ip, oop],
        history,
        stacks: [hand.stacks[ip], hand.stacks[oop]],
    })
}

/// `e` with its seat renumbered by `f`.
fn with_seat(e: &Event, f: impl Fn(usize) -> usize) -> Event {
    let mut e = e.clone();
    match &mut e {
        Event::Ante { seat, .. }
        | Event::SmallBlind { seat, .. }
        | Event::BigBlind { seat, .. }
        | Event::Post { seat, .. }
        | Event::Fold { seat }
        | Event::Check { seat }
        | Event::Call { seat, .. }
        | Event::Bet { seat, .. }
        | Event::Raise { seat, .. } => *seat = f(*seat),
        _ => {}
    }
    e
}

/// One scored decision. No cards.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Scored {
    /// The real seat that decided.
    pub seat: usize,
    /// 1 flop, 2 turn, 3 river.
    pub street: usize,
    /// How often the model makes the play they made, with their hand (a bet
    /// between two menu sizes counts as a mix of both).
    pub frequency: f64,
    /// How often the model makes its most frequent play there.
    pub top: f64,
    /// Big blinds the play gives up against the model's best one, when
    /// valued (Hold'em).
    pub ev_loss: Option<f64>,
    /// The standard error of that (0 when exact).
    pub stderr: f64,
    /// What the hand review charges: the loss, or 0 for a play the model
    /// makes or one within the sampling error.
    pub bb_lost: f64,
}

/// Every decision from the flop on by the two players who saw it, in a
/// Hold'em hand, reviewed with `reviewer` (the heads-up blueprint). Players
/// without known hole cards are left out.
pub fn holdem(reviewer: &Reviewer, hand: &TableHand, cache: &mut BucketCache) -> Vec<Scored> {
    let Some(hu) = heads_up(hand) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for side in 0..2 {
        let seat = hu.seats[side];
        let Some(hole) = hand.holes.get(seat).and_then(|h| h.as_ref()) else {
            continue;
        };
        let Ok(hole) = <[Card; 2]>::try_from(hole.as_slice()) else {
            continue;
        };
        let rec = HandRecord {
            seat: side,
            button: 0,
            hole,
            board: hand.board.clone(),
            history: hu.history.clone(),
            big_blind: hand.big_blind,
            net: 0,
            stacks: hu.stacks,
        };
        let review = reviewer.review(&rec, cache);
        for d in review.decisions {
            let street = match d.street.as_str() {
                "flop" => 1,
                "turn" => 2,
                "river" => 3,
                _ => continue,
            };
            if d.grade == Grade::Unknown {
                continue;
            }
            out.push(Scored {
                seat,
                street,
                frequency: d.frequency,
                top: d.gto.iter().map(|a| a.frequency).fold(0.0, f64::max),
                ev_loss: d.ev_loss,
                stderr: d.stderr,
                bb_lost: d.bb_lost,
            });
        }
    }
    out
}

/// Every decision from the flop on by the two players who saw it, in a
/// pot-limit Omaha hand, against a heads-up PLO blueprint (`blueprint` on
/// `tree`, bucketed with `cards`): frequencies only. Players whose hole
/// cards aren't the abstraction's number are left out.
pub fn plo(
    cards: &PloAbstraction,
    blueprint: &Blueprint,
    tree: &BettingTree,
    hand: &TableHand,
    seed: u64,
) -> Vec<Scored> {
    let Some(hu) = heads_up(hand) else {
        return Vec::new();
    };
    let holes: [Option<&Vec<Card>>; 2] = std::array::from_fn(|side| {
        hand.holes
            .get(hu.seats[side])
            .and_then(|h| h.as_ref())
            .filter(|h| h.len() == cards.config.hole_cards)
    });
    let mut rng = Rng::new(seed);
    let mut f = Follower::new();
    let mut out = Vec::new();
    for e in &hu.history {
        let seat = match *e {
            Event::Fold { seat }
            | Event::Check { seat }
            | Event::Call { seat, .. }
            | Event::Bet { seat, .. }
            | Event::Raise { seat, .. } => Some(seat),
            _ => None,
        };
        let (node, street, before) = (f.node, f.street, f.steps.len());
        f.apply(tree, e, |s| s, &mut rng);
        let (Some(side), Some(node)) = (seat, node) else {
            continue;
        };
        if street == 0 || side > 1 {
            continue;
        }
        let Some(hole) = holes[side] else {
            continue;
        };
        let n = &tree.nodes[node as usize];
        if n.betting.street != street || n.betting.to_act != side {
            continue;
        }
        let Some(step) = f.steps.get(before).filter(|s| s.node == node) else {
            continue;
        };
        let board = &hand.board[..[0, 3, 4, 5][street].min(hand.board.len())];
        let probs = blueprint.probs(node, cards.hand_bucket(hole, board));
        let taken: Vec<(usize, f64)> = match step.mapped {
            Some(m) => m.to_vec(),
            None => vec![(step.action, 1.0)],
        };
        out.push(Scored {
            seat: hu.seats[side],
            street,
            frequency: taken.iter().map(|&(i, w)| w * probs[i]).sum(),
            top: probs.iter().copied().fold(0.0, f64::max),
            ev_loss: None,
            stderr: 0.0,
            bb_lost: 0.0,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ducy_play::Street;

    fn board(street: Street, cards: &str) -> Event {
        Event::Board {
            street,
            cards: cards
                .split_whitespace()
                .map(|c| c.parse().expect("card"))
                .collect(),
        }
    }

    /// Six seats, the button on 3: UTG (seat 0) opens, the button calls, the
    /// blinds fold; a flop is bet and called.
    fn six_handed() -> TableHand {
        TableHand {
            history: vec![
                Event::SmallBlind { seat: 4, amount: 1 },
                Event::BigBlind { seat: 5, amount: 2 },
                Event::Raise {
                    seat: 0,
                    to: 5,
                    all_in: false,
                },
                Event::Fold { seat: 1 },
                Event::Fold { seat: 2 },
                Event::Call {
                    seat: 3,
                    amount: 5,
                    all_in: false,
                },
                Event::Fold { seat: 4 },
                Event::Fold { seat: 5 },
                board(Street::Flop, "Ks 9d 4h"),
                Event::Bet {
                    seat: 0,
                    to: 4,
                    all_in: false,
                },
                Event::Call {
                    seat: 3,
                    amount: 4,
                    all_in: false,
                },
            ],
            button: 3,
            small_blind: 1,
            big_blind: 2,
            stacks: vec![200; 6],
            holes: vec![None; 6],
            board: Vec::new(),
        }
    }

    #[test]
    fn a_six_handed_pot_two_saw_the_flop_of_is_heads_up_with_the_blinds_dead() {
        let hu = heads_up(&six_handed()).expect("two saw the flop");
        // The button is in position.
        assert_eq!(hu.seats, [3, 0]);
        assert_eq!(hu.stacks, [200, 200]);
        // The blinds' 3 chips are antes, then the heads-up line: the button
        // limps, UTG raises, the button calls.
        assert_eq!(
            hu.history[..2],
            [
                Event::Ante { seat: 0, amount: 1 },
                Event::Ante { seat: 1, amount: 2 }
            ]
        );
        let after: Vec<_> = hu.history[2..].to_vec();
        assert!(matches!(after[2], Event::Call { seat: 0, .. }));
        assert!(matches!(after[3], Event::Raise { seat: 1, to: 5, .. }));
        assert!(matches!(after[4], Event::Call { seat: 0, .. }));
        assert!(matches!(after[6], Event::Bet { seat: 1, to: 4, .. }));
        assert!(matches!(
            after[7],
            Event::Call {
                seat: 0,
                amount: 4,
                ..
            }
        ));
    }

    #[test]
    fn three_at_the_flop_or_no_flop_is_not_heads_up() {
        let mut three = six_handed();
        // The big blind calls instead of folding.
        three.history[7] = Event::Call {
            seat: 5,
            amount: 3,
            all_in: false,
        };
        assert!(heads_up(&three).is_none());
        let mut preflop = six_handed();
        preflop.history.truncate(8);
        assert!(heads_up(&preflop).is_none());
    }

    #[test]
    fn a_heads_up_hand_keeps_its_seats_with_the_button_first() {
        let hand = TableHand {
            history: vec![
                Event::SmallBlind { seat: 1, amount: 1 },
                Event::BigBlind { seat: 0, amount: 2 },
                Event::Call {
                    seat: 1,
                    amount: 1,
                    all_in: false,
                },
                Event::Check { seat: 0 },
                board(Street::Flop, "Ks 9d 4h"),
                Event::Check { seat: 0 },
            ],
            button: 1,
            small_blind: 1,
            big_blind: 2,
            stacks: vec![100, 300],
            holes: vec![None, None],
            board: Vec::new(),
        };
        let hu = heads_up(&hand).expect("heads-up");
        assert_eq!(hu.seats, [1, 0]);
        assert_eq!(hu.stacks, [300, 100]);
        assert_eq!(hu.depth(2), 50.0);
        assert_eq!(hu.history[0], Event::SmallBlind { seat: 0, amount: 1 });
        assert_eq!(hu.history[5], Event::Check { seat: 1 });
    }
}
