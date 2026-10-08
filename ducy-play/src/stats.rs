//! Tracking how opponents play, from hand histories.

use crate::hand::{Event, Street};

/// Counts for one seat across the hands seen so far.
///
/// Observations identify seats, not players, so stats follow whoever sits in
/// a seat. That matches a fixed table; in a duplicate match, where bots
/// rotate seats, they blend the bots that sat there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SeatStats {
    /// Hands dealt to the seat.
    pub hands: u32,
    /// Hands where the seat voluntarily put chips in preflop (called or
    /// raised; posting a blind doesn't count).
    pub vpip_hands: u32,
    /// Hands where the seat raised preflop.
    pub pfr_hands: u32,
    /// Times the seat faced a bet or raise after the flop.
    pub faced_bets: u32,
    /// Times it folded to one.
    pub folds_to_bets: u32,
    /// Bets and raises after the flop.
    pub aggressive: u32,
    /// Calls after the flop.
    pub calls: u32,
}

impl SeatStats {
    /// Adds one hand's counts.
    pub fn add(&mut self, c: &HandCounts) {
        self.hands += 1;
        self.vpip_hands += u32::from(c.vpip);
        self.pfr_hands += u32::from(c.pfr);
        self.faced_bets += c.faced_bets;
        self.folds_to_bets += c.folds_to_bets;
        self.aggressive += c.aggressive;
        self.calls += c.calls;
    }

    /// Share of hands played voluntarily, with a light prior of 25% so a few
    /// hands don't swing it to an extreme.
    pub fn vpip(&self) -> f64 {
        smoothed(self.vpip_hands, self.hands, 0.25)
    }

    /// Share of hands raised preflop (prior 15%).
    pub fn pfr(&self) -> f64 {
        smoothed(self.pfr_hands, self.hands, 0.15)
    }

    /// How often it folds when bet into after the flop (prior 40%).
    pub fn fold_to_bet(&self) -> f64 {
        smoothed(self.folds_to_bets, self.faced_bets, 0.4)
    }

    /// Bets and raises per call after the flop (prior 1).
    pub fn aggression(&self) -> f64 {
        (self.aggressive as f64 + 2.0) / (self.calls as f64 + 2.0)
    }
}

/// `count / total`, pulled toward `prior` as if 4 hands at the prior had
/// already been seen.
fn smoothed(count: u32, total: u32, prior: f64) -> f64 {
    (count as f64 + 4.0 * prior) / (total as f64 + 4.0)
}

/// What one seat did in one finished hand. [`OpponentModel`] adds these up
/// for the bots, and ducy.cards counts its player stats from them too
/// (through ducy-wasm), so both read players the same way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct HandCounts {
    /// Called or raised preflop (posting a blind doesn't count).
    pub vpip: bool,
    /// Raised preflop.
    pub pfr: bool,
    /// Made a preflop decision facing exactly one raise (a chance to
    /// 3-bet), other than a call that put it all-in.
    pub three_bet_chance: bool,
    /// ... and re-raised.
    pub three_bet: bool,
    /// Bets and raises after the flop.
    pub aggressive: u32,
    /// Calls after the flop.
    pub calls: u32,
    /// Folds after the flop.
    pub folds: u32,
    /// Times it faced a bet or raise after the flop.
    pub faced_bets: u32,
    /// Times it folded to one.
    pub folds_to_bets: u32,
    /// Still in when the flop came.
    pub saw_flop: bool,
    /// Folded at some point.
    pub folded: bool,
}

/// Counts for each of `seats` seats from one finished hand's events.
pub fn hand_counts(history: &[Event], seats: usize) -> Vec<HandCounts> {
    let mut c = vec![HandCounts::default(); seats];
    let mut street = Street::Preflop;
    let mut street_bets = vec![0u64; seats];
    let mut current = 0u64;
    // Preflop bets and raises so far (blinds and posts aren't raises).
    let mut raises = 0u32;

    for event in history {
        let seat_ok = |seat: usize| seat < seats;
        match *event {
            Event::SmallBlind { seat, amount }
            | Event::BigBlind { seat, amount }
            | Event::Post {
                seat, live: amount, ..
            } if seat_ok(seat) => {
                street_bets[seat] += amount;
                current = current.max(street_bets[seat]);
            }
            Event::Board { street: next, .. } => {
                if street == Street::Preflop {
                    for s in c.iter_mut().filter(|s| !s.folded) {
                        s.saw_flop = true;
                    }
                }
                street = next;
                street_bets.iter_mut().for_each(|b| *b = 0);
                current = 0;
            }
            Event::Fold { seat } if seat_ok(seat) => {
                let facing = current > street_bets[seat];
                let s = &mut c[seat];
                if street == Street::Preflop {
                    preflop_decision(s, raises, false, false);
                } else {
                    s.folds += 1;
                    if facing {
                        s.faced_bets += 1;
                        s.folds_to_bets += 1;
                    }
                }
                s.folded = true;
            }
            Event::Call {
                seat,
                amount,
                all_in,
            } if seat_ok(seat) => {
                let s = &mut c[seat];
                if street == Street::Preflop {
                    preflop_decision(s, raises, false, all_in);
                    s.vpip = true;
                } else {
                    s.faced_bets += 1;
                    s.calls += 1;
                }
                street_bets[seat] += amount;
            }
            Event::Bet { seat, to, .. } | Event::Raise { seat, to, .. } if seat_ok(seat) => {
                let facing = current > street_bets[seat];
                let s = &mut c[seat];
                if street == Street::Preflop {
                    preflop_decision(s, raises, true, false);
                    s.vpip = true;
                    s.pfr = true;
                    raises += 1;
                } else {
                    s.aggressive += 1;
                    if facing {
                        s.faced_bets += 1;
                    }
                }
                street_bets[seat] = to;
                current = current.max(to);
            }
            _ => {}
        }
    }
    c
}

/// A preflop decision facing exactly one raise is a chance to 3-bet, the
/// first time, unless a call put the seat all-in (it couldn't have raised).
fn preflop_decision(s: &mut HandCounts, raises: u32, raised: bool, all_in_call: bool) {
    if raises == 1 && !s.three_bet_chance && !all_in_call {
        s.three_bet_chance = true;
        s.three_bet = raised;
    }
}

/// Stats for every seat, updated from each finished hand's history.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpponentModel {
    seats: Vec<SeatStats>,
}

impl OpponentModel {
    /// An empty model.
    pub fn new() -> Self {
        Self::default()
    }

    /// Stats for `seat` (all zeros if it hasn't been seen).
    pub fn seat(&self, seat: usize) -> SeatStats {
        self.seats.get(seat).copied().unwrap_or_default()
    }

    /// Adds one finished hand, given its full event history and seat count.
    pub fn record(&mut self, history: &[Event], seats: usize) {
        if self.seats.len() < seats {
            self.seats.resize(seats, SeatStats::default());
        }
        for (s, c) in self.seats.iter_mut().zip(hand_counts(history, seats)) {
            s.add(&c);
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_record() {
        use Event::*;
        // Seat 0 raises preflop, seat 1 calls; flop: seat 1 checks, seat 0
        // bets, seat 1 folds. Seat 2 folds preflop.
        let history = [
            SmallBlind { seat: 1, amount: 1 },
            BigBlind { seat: 2, amount: 2 },
            Raise {
                seat: 0,
                to: 6,
                all_in: false,
            },
            Call {
                seat: 1,
                amount: 5,
                all_in: false,
            },
            Fold { seat: 2 },
            Board {
                street: Street::Flop,
                cards: Vec::new(),
            },
            Check { seat: 1 },
            Bet {
                seat: 0,
                to: 8,
                all_in: false,
            },
            Fold { seat: 1 },
        ];
        let mut model = OpponentModel::new();
        model.record(&history, 3);
        model.record(&history, 3);

        let raiser = model.seat(0);
        assert_eq!(
            (raiser.hands, raiser.vpip_hands, raiser.pfr_hands),
            (2, 2, 2)
        );
        assert_eq!((raiser.aggressive, raiser.faced_bets), (2, 0));

        let caller = model.seat(1);
        assert_eq!((caller.vpip_hands, caller.pfr_hands), (2, 0));
        assert_eq!((caller.faced_bets, caller.folds_to_bets), (2, 2));
        // 2 folds out of 2, pulled toward the 40% prior.
        assert!((caller.fold_to_bet() - (2.0 + 1.6) / 6.0).abs() < 1e-12);

        // Folding the big blind preflop isn't voluntary play or a fold to a
        // postflop bet.
        let blind = model.seat(2);
        assert_eq!((blind.vpip_hands, blind.faced_bets), (0, 0));
        assert_eq!(model.seat(9), SeatStats::default());
    }

    #[test]
    fn test_hand_counts() {
        use Event::*;
        // Seat 3 posts a missed big blind live; seat 0 opens, seat 2 (big
        // blind) folds, seat 1 3-bets, seat 3 calls all-in for less, seat 0
        // calls. Flop: seat 1 bets, seat 0 folds.
        let history = [
            SmallBlind { seat: 1, amount: 1 },
            BigBlind { seat: 2, amount: 2 },
            Post {
                seat: 3,
                dead: 0,
                live: 2,
            },
            Raise {
                seat: 0,
                to: 6,
                all_in: false,
            },
            Fold { seat: 2 },
            Raise {
                seat: 1,
                to: 18,
                all_in: false,
            },
            Call {
                seat: 3,
                amount: 8,
                all_in: true,
            },
            Call {
                seat: 0,
                amount: 12,
                all_in: false,
            },
            Board {
                street: Street::Flop,
                cards: Vec::new(),
            },
            Bet {
                seat: 1,
                to: 20,
                all_in: false,
            },
            Fold { seat: 0 },
        ];
        let c = hand_counts(&history, 4);
        // The opener faced no raise before acting, so no 3-bet chance then;
        // facing the 3-bet it had already raised, so it isn't one either.
        assert_eq!(
            (c[0].vpip, c[0].pfr, c[0].three_bet_chance),
            (true, true, false)
        );
        assert_eq!((c[0].folds, c[0].faced_bets, c[0].folds_to_bets), (1, 1, 1));
        assert!(c[0].saw_flop && c[0].folded);
        assert_eq!(
            (c[1].three_bet_chance, c[1].three_bet, c[1].aggressive),
            (true, true, 1)
        );
        // The big blind folded facing the open: a chance, not taken.
        assert_eq!(
            (c[2].three_bet_chance, c[2].three_bet, c[2].vpip),
            (true, false, false)
        );
        assert!(!c[2].saw_flop);
        // Facing two raises isn't a 3-bet chance.
        assert_eq!((c[3].three_bet_chance, c[3].vpip), (false, true));
        // An all-in call facing one raise isn't one either: it couldn't raise.
        let short = [
            BigBlind { seat: 1, amount: 2 },
            Raise {
                seat: 0,
                to: 6,
                all_in: false,
            },
            Call {
                seat: 1,
                amount: 3,
                all_in: true,
            },
        ];
        assert!(!hand_counts(&short, 2)[1].three_bet_chance);
        // Seats past the count, or events for them, don't panic.
        assert_eq!(hand_counts(&history, 2).len(), 2);
    }
}
