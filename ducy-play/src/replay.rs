//! Replaying a finished hand one event at a time: the board, the pot and
//! every seat's stack and bet after each event, from the hand's event list
//! and each seat's stack when it started. The chip accounting follows
//! [`Hand`](crate::Hand)'s, so a replay can't drift from it: every frame's
//! stacks, bets and pot add up to the starting stacks.
//!
//! ```
//! use ducy_play::{Event, replay::replay_frames};
//!
//! let events = [
//!     Event::SmallBlind { seat: 0, amount: 1 },
//!     Event::BigBlind { seat: 1, amount: 2 },
//!     Event::Fold { seat: 0 },
//!     Event::Award { seat: 1, pot: 0, amount: 3 },
//! ];
//! let frames = replay_frames(&events, &[100, 100], &["Ann".into(), "Bob".into()]);
//! assert_eq!(frames.len(), 5); // before any action, then one per event
//! assert_eq!(frames[3].text, "Ann folds");
//! assert_eq!(frames[4].seats[1].stack, 101);
//! ```

use crate::hand::{Event, Street};
use ducy::deck::Card;

/// One seat in a frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ReplaySeat {
    /// Chips behind.
    pub stack: u64,
    /// Chips bet on this street, not yet in the pot.
    pub bet: u64,
    /// Out of the hand.
    pub folded: bool,
    /// No chips left to bet.
    pub all_in: bool,
}

/// The hand after one event (or before any, for the first frame).
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Frame {
    /// The street being played.
    pub street: Street,
    /// The board cards dealt so far.
    pub board: Vec<Card>,
    /// Chips in the pot (bets go in when the street ends).
    pub pot: u64,
    /// Every seat, in seat order.
    pub seats: Vec<ReplaySeat>,
    /// What happened, e.g. "Ann raises to 6".
    pub text: String,
    /// The index of the event in the list (none for the first frame).
    pub event: Option<usize>,
}

/// The frames of a finished hand: one before any action, then one after
/// each event. `stacks` are the seats' stacks when the hand started and
/// `names` their names for the text ("Seat 3" when missing).
pub fn replay_frames(events: &[Event], stacks: &[u64], names: &[String]) -> Vec<Frame> {
    let mut seats: Vec<ReplaySeat> = stacks
        .iter()
        .map(|&stack| ReplaySeat {
            stack,
            ..Default::default()
        })
        .collect();
    let mut pot = 0u64;
    let mut street = Street::Preflop;
    let mut board: Vec<Card> = Vec::new();
    let mut awarding = false;
    let name = |seat: usize| {
        names
            .get(seat)
            .cloned()
            .unwrap_or_else(|| format!("Seat {seat}"))
    };
    let collect = |seats: &mut Vec<ReplaySeat>, pot: &mut u64| {
        for s in seats.iter_mut() {
            *pot += s.bet;
            s.bet = 0;
        }
    };
    let all_in = |s: bool| if s { " (all-in)" } else { "" };

    let mut frames = vec![Frame {
        street,
        board: Vec::new(),
        pot,
        seats: seats.clone(),
        text: "Cards are dealt".into(),
        event: None,
    }];
    for (i, e) in events.iter().enumerate() {
        let text = match *e {
            Event::Ante { seat, amount } => {
                if let Some(s) = seats.get_mut(seat) {
                    s.stack = s.stack.saturating_sub(amount);
                    pot += amount;
                }
                format!("{} antes {amount}", name(seat))
            }
            Event::Post { seat, dead, live } => {
                if let Some(s) = seats.get_mut(seat) {
                    s.stack = s.stack.saturating_sub(dead + live);
                    s.bet += live;
                    pot += dead;
                }
                let parts: Vec<String> = [(dead, "dead"), (live, "live")]
                    .iter()
                    .filter(|(n, _)| *n > 0)
                    .map(|(n, what)| format!("{n} {what}"))
                    .collect();
                format!("{} posts missed blinds: {}", name(seat), parts.join(" + "))
            }
            Event::SmallBlind { seat, amount }
            | Event::BigBlind { seat, amount }
            | Event::Call { seat, amount, .. } => {
                let shove = matches!(e, Event::Call { all_in: true, .. });
                if let Some(s) = seats.get_mut(seat) {
                    s.stack = s.stack.saturating_sub(amount);
                    s.bet += amount;
                    if shove || s.stack == 0 {
                        s.all_in = s.stack == 0;
                    }
                }
                match e {
                    Event::SmallBlind { .. } => {
                        format!("{} posts the small blind {amount}", name(seat))
                    }
                    Event::BigBlind { .. } => {
                        format!("{} posts the big blind {amount}", name(seat))
                    }
                    _ => format!("{} calls {amount}{}", name(seat), all_in(shove)),
                }
            }
            Event::Bet {
                seat,
                to,
                all_in: shove,
            }
            | Event::Raise {
                seat,
                to,
                all_in: shove,
            } => {
                if let Some(s) = seats.get_mut(seat) {
                    let add = to.saturating_sub(s.bet);
                    s.stack = s.stack.saturating_sub(add);
                    s.bet = to;
                    s.all_in = shove || s.stack == 0;
                }
                let verb = if matches!(e, Event::Bet { .. }) {
                    "bets"
                } else {
                    "raises to"
                };
                format!("{} {verb} {to}{}", name(seat), all_in(shove))
            }
            Event::Fold { seat } => {
                if let Some(s) = seats.get_mut(seat) {
                    s.folded = true;
                }
                format!("{} folds", name(seat))
            }
            Event::Check { seat } => format!("{} checks", name(seat)),
            Event::Reveal { seat, ref cards } => {
                let cards: Vec<String> = cards.iter().map(|c| c.to_string()).collect();
                format!("{} shows {}", name(seat), cards.join(" "))
            }
            Event::Forfeit { seat } => format!("{} doesn't show, and can't win", name(seat)),
            Event::Runs { count } => {
                if count == 2 {
                    "They run it twice".to_string()
                } else {
                    "They run it once".to_string()
                }
            }
            Event::SecondBoard { ref cards } => {
                let cards: Vec<String> = cards.iter().map(|c| c.to_string()).collect();
                format!("Second run: {}", cards.join(" "))
            }
            Event::Board {
                street: next,
                ref cards,
            } => {
                collect(&mut seats, &mut pot);
                street = next;
                board.extend(cards.iter().copied());
                let label = match next {
                    Street::Preflop => "Preflop",
                    Street::Flop => "Flop",
                    Street::Turn => "Turn",
                    Street::River => "River",
                };
                let cards: Vec<String> = cards.iter().map(|c| c.to_string()).collect();
                format!("{label}: {}", cards.join(" "))
            }
            Event::Award {
                seat,
                pot: which,
                amount,
            } => {
                if !awarding {
                    collect(&mut seats, &mut pot);
                    awarding = true;
                }
                if let Some(s) = seats.get_mut(seat) {
                    s.stack += amount;
                    pot = pot.saturating_sub(amount);
                }
                let side = if which > 0 { " (side pot)" } else { "" };
                format!("{} wins {amount}{side}", name(seat))
            }
        };
        frames.push(Frame {
            street,
            board: board.clone(),
            pot,
            seats: seats.clone(),
            text,
            event: Some(i),
        });
    }
    frames
}
