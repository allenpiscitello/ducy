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
        let mut street = Street::Preflop;
        let mut street_bets = vec![0u64; seats];
        let mut current = 0u64;
        let mut vpip = vec![false; seats];
        let mut pfr = vec![false; seats];

        for event in history {
            // Whether `seat` faced a bet larger than what it had put in.
            let facing = |seat: usize, street_bets: &[u64]| current > street_bets[seat];
            match *event {
                Event::SmallBlind { seat, amount }
                | Event::BigBlind { seat, amount }
                | Event::Post {
                    seat, live: amount, ..
                } => {
                    street_bets[seat] += amount;
                    current = current.max(street_bets[seat]);
                }
                Event::Board { street: next, .. } => {
                    street = next;
                    street_bets.iter_mut().for_each(|b| *b = 0);
                    current = 0;
                }
                Event::Fold { seat } => {
                    if street != Street::Preflop && facing(seat, &street_bets) {
                        let s = &mut self.seats[seat];
                        s.faced_bets += 1;
                        s.folds_to_bets += 1;
                    }
                }
                Event::Check { .. } => {}
                Event::Call { seat, amount, .. } => {
                    if street == Street::Preflop {
                        vpip[seat] = true;
                    } else {
                        let s = &mut self.seats[seat];
                        s.faced_bets += 1;
                        s.calls += 1;
                    }
                    street_bets[seat] += amount;
                }
                Event::Bet { seat, to, .. } | Event::Raise { seat, to, .. } => {
                    if street == Street::Preflop {
                        vpip[seat] = true;
                        pfr[seat] = true;
                    } else {
                        let was_facing = facing(seat, &street_bets);
                        let s = &mut self.seats[seat];
                        s.aggressive += 1;
                        if was_facing {
                            s.faced_bets += 1;
                        }
                    }
                    street_bets[seat] = to;
                    current = current.max(to);
                }
                Event::Ante { .. } | Event::Award { .. } => {}
            }
        }
        for (seat, s) in self.seats.iter_mut().enumerate().take(seats) {
            s.hands += 1;
            s.vpip_hands += u32::from(vpip[seat]);
            s.pfr_hands += u32::from(pfr[seat]);
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
}
