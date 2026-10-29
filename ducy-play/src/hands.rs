//! What a table shows about the hand in play, from what a viewer sees of it
//! (a [`TableView`]'s seats and events), for pages that draw the table:
//!
//! - the pots as they form ([`shown_pots`]): the main pot, the side pots,
//!   and an all-in nobody could call, which isn't in any pot but goes back
//!   to whoever bet it. The hand itself only splits the pots at the end.
//! - a player's hand in words ([`hand_words`]), like "Straight, 8 high".
//! - who posts the next big blind ([`next_big_blind`]), for a player who
//!   leaves before theirs.

use ducy::{
    deck::{Card, Deck},
    games::{
        GameEvaluation,
        flop_game::FlopGame,
        holdem::{HoldemGameEvaluation, HoldemGameState},
        omaha::{OmahaGameEvaluation, OmahaGameState},
    },
};

use crate::{
    hand::Event,
    showdown::build_pots,
    table::{SeatState, TableView, next_positions},
};

/// A seat as these rules read it: its chips and whether it's in the hand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct SeatChips {
    /// Chips behind.
    pub stack: u64,
    /// Chips bet on this street.
    pub street_bet: u64,
    /// Folded this hand.
    pub folded: bool,
    /// All in this hand.
    pub all_in: bool,
    /// No one sits here.
    pub empty: bool,
    /// Sitting out this hand.
    pub sitting_out: bool,
}

impl From<&SeatState> for SeatChips {
    fn from(s: &SeatState) -> Self {
        SeatChips {
            stack: s.stack,
            street_bet: s.street_bet,
            folded: s.folded,
            all_in: s.all_in,
            empty: s.empty,
            sitting_out: s.sitting_out,
        }
    }
}

/// A pot as it stands.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ShownPot {
    /// Chips in it.
    pub amount: u64,
    /// The seats that can win it.
    pub eligible: Vec<usize>,
}

/// The top of a bet only its bettor can win: it goes back to them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Uncalled {
    /// The bettor.
    pub seat: usize,
    /// Chips going back.
    pub amount: u64,
}

/// The pots split out, with what's uncalled.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Pots {
    /// The main pot first, then the side pots.
    pub pots: Vec<ShownPot>,
    /// What nobody could call, if anything.
    pub uncalled: Option<Uncalled>,
}

/// What each of `n` seats has put in so far this hand, from its events
/// (antes, blinds, missed blinds, calls, bets and raises), and what each has
/// bet on the street now.
pub fn contributions(events: &[Event], n: usize) -> (Vec<u64>, Vec<u64>) {
    let mut put = vec![0u64; n];
    let mut street = vec![0u64; n];
    for e in events {
        let (seat, to_pot, to_bet) = match *e {
            Event::Ante { seat, amount } => (seat, amount, 0),
            Event::Post { seat, dead, live } => (seat, dead, live),
            Event::SmallBlind { seat, amount }
            | Event::BigBlind { seat, amount }
            | Event::Call { seat, amount, .. } => (seat, 0, amount),
            Event::Bet { seat, to, .. } | Event::Raise { seat, to, .. } => (
                seat,
                0,
                to.saturating_sub(street.get(seat).copied().unwrap_or(0)),
            ),
            Event::Board { .. } => {
                street.fill(0);
                continue;
            }
            _ => continue,
        };
        if seat < n {
            street[seat] += to_bet;
            put[seat] += to_pot + to_bet;
        }
    }
    (put, street)
}

/// The pots from what each seat has put in (`put`) and who has folded, the
/// way the hand splits them at the end: a pot for each all-in level of the
/// players still in, folded players' chips in the pots they reached. The
/// top pot is uncalled when only one player can win it and others are
/// still in.
pub fn split_pots(put: &[u64], folded: &[bool]) -> Pots {
    let mut pots: Vec<ShownPot> = build_pots(put, folded)
        .into_iter()
        .filter(|(amount, _)| *amount > 0)
        .map(|(amount, eligible)| ShownPot { amount, eligible })
        .collect();
    let contested = pots.iter().any(|p| p.eligible.len() > 1);
    let uncalled = match pots.last() {
        Some(top) if contested && top.eligible.len() == 1 => {
            let top = pots.pop().expect("a top pot");
            Some(Uncalled {
                seat: top.eligible[0],
                amount: top.amount,
            })
        }
        _ => None,
    };
    Pots { pots, uncalled }
}

/// Nobody can bet any more this hand: everyone still in is all-in but at
/// most one, who has matched every bet.
pub fn betting_closed(seats: &[SeatChips]) -> bool {
    let live: Vec<&SeatChips> = seats
        .iter()
        .filter(|s| !s.folded && !s.empty && !s.sitting_out)
        .collect();
    let free: Vec<&&SeatChips> = live.iter().filter(|s| !s.all_in).collect();
    if live.len() < 2 || free.len() > 1 {
        return false;
    }
    free.first()
        .is_none_or(|f| live.iter().all(|s| s.street_bet <= f.street_bet))
}

/// The pots to show at the table now, or `None` when there's just the one
/// pot and nothing uncalled (shown as usual). While betting goes on, this
/// street's bets stay in front of the players and only chips already
/// gathered are split; once nobody can bet any more, every chip is.
pub fn shown_pots(seats: &[SeatChips], events: &[Event]) -> Option<Pots> {
    let (put, _) = contributions(events, seats.len());
    let gathered: Vec<u64> = if betting_closed(seats) {
        put
    } else {
        put.iter()
            .zip(seats)
            .map(|(p, s)| p.saturating_sub(s.street_bet))
            .collect()
    };
    let folded: Vec<bool> = seats.iter().map(|s| s.folded).collect();
    let pots = split_pots(&gathered, &folded);
    (pots.pots.len() > 1 || pots.uncalled.is_some()).then_some(pots)
}

/// The seat that posts the big blind in the next hand, or `None` if fewer
/// than two seats would be dealt in, from the seats as a viewer sees them
/// and the current `button` (the rule [`crate::Table::next_big_blind`]
/// follows). `refill`: seats with no chips are dealt in anyway (a table
/// that tops its bots up between hands); otherwise only seats with chips.
pub fn next_big_blind(seats: &[SeatChips], button: usize, refill: bool) -> Option<usize> {
    let plays = |s: usize| {
        let x = &seats[s];
        !x.empty && !x.sitting_out && (refill || x.stack > 0)
    };
    next_positions(seats.len(), button, plays).map(|(_, _, bb)| bb)
}

impl TableView {
    fn seat_chips(&self) -> Vec<SeatChips> {
        self.seats.iter().map(SeatChips::from).collect()
    }

    /// [`shown_pots`] for this view, while the hand goes on.
    pub fn shown_pots(&self) -> Option<Pots> {
        if self.complete {
            return None;
        }
        shown_pots(&self.seat_chips(), &self.events)
    }

    /// The seat that posts the big blind in the next hand, as
    /// [`next_big_blind`] finds it.
    pub fn next_big_blind(&self, refill: bool) -> Option<usize> {
        next_big_blind(&self.seat_chips(), self.button, refill)
    }
}

/// What `hole` makes with `board` (3 to 5 cards so far), in words, like
/// "Straight, 8 high": the best five cards as ducy's evaluator finds them
/// (Hold'em: any five; Omaha: exactly two of the hole cards and three of the
/// board). `None` before the flop, or for cards it can't read.
pub fn hand_words(hole: &[String], board: &[String]) -> Option<String> {
    if hole.is_empty() || board.len() < 3 || board.len() > 5 {
        return None;
    }
    let n = hole.len();
    let hole = Deck::parse(&hole.join(" ")).ok()?;
    let board: Vec<Card> = board
        .iter()
        .map(|c| Card::parse(c).ok())
        .collect::<Option<_>>()?;
    let name = match n {
        2 => {
            let mut state = HoldemGameState::new();
            deal(&mut state, hole, &board)?;
            let winners = HoldemGameEvaluation {}.evaluate_winners(&state);
            winners.first()?.winning_hand().to_string()
        }
        n => {
            let mut state = OmahaGameState::new(n as u32);
            deal(&mut state, hole, &board)?;
            let winners = OmahaGameEvaluation {}.evaluate_winners(&state);
            winners.first()?.winning_hand().to_string()
        }
    };
    name_words(&name)
}

fn deal(state: &mut impl FlopGame, hole: Deck, board: &[Card]) -> Option<()> {
    state.add_player(hole).ok()?;
    let mut flop = Deck::empty();
    for &c in &board[..3] {
        flop |= c;
    }
    state.set_flop(flop).ok()?;
    if let Some(&turn) = board.get(3) {
        state.set_turn(turn).ok()?;
    }
    if let Some(&river) = board.get(4) {
        state.set_river(river).ok()?;
    }
    Some(())
}

/// One rank in words: faces spelled out ("ace"), numbers as digits ("8").
fn one(rank: &str) -> String {
    match rank {
        "A" => "ace",
        "K" => "king",
        "Q" => "queen",
        "J" => "jack",
        "T" => "10",
        r => r,
    }
    .to_string()
}

/// Several of a rank: "aces", "8s", "10s".
fn many(rank: &str) -> String {
    format!("{}s", one(rank))
}

fn cap(s: String) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// ducy's name for a hand ("Straight 8 high", "Pair of 8, 9 7 2", "Full
/// House 9 full of 2", as a pot's `winning_hand` gives it) in words, like
/// "Straight, 8 high", "Pair of 8s", "Full house, 9s full of 2s"; `None` if
/// it isn't one.
pub fn name_words(name: &str) -> Option<String> {
    let w: Vec<&str> = name.split([' ', ',']).filter(|s| !s.is_empty()).collect();
    let words = match w.as_slice() {
        ["High", "Card", r, ..] => format!("{} high", cap(one(r))),
        ["Pair", "of", r, ..] => format!("Pair of {}", many(r)),
        ["Two", "Pair", a, "over", b, ..] => format!("Two pair, {} and {}", many(a), many(b)),
        ["Three", "of", "a", "Kind", r, ..] => format!("Three of a kind, {}", many(r)),
        ["Straight", "Flush", "A", "high", ..] => "Royal flush".into(),
        ["Straight", "Flush", r, "high", ..] => format!("Straight flush, {} high", one(r)),
        ["Straight", r, "high", ..] => format!("Straight, {} high", one(r)),
        ["Flush", r, ..] => format!("Flush, {} high", one(r)),
        ["Full", "House", t, "full", "of", p, ..] => {
            format!("Full house, {} full of {}", many(t), many(p))
        }
        ["Four", "of", "a", "Kind", r, ..] => format!("Four of a kind, {}", many(r)),
        _ => return None,
    };
    Some(words)
}

/// What a hand's record is about: where it was played, and whose view it
/// was (`who`: "host", or the player's id).
#[cfg(feature = "serde")]
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RecordOf {
    /// The club's code.
    pub club: String,
    /// The table's id.
    pub table: String,
    /// The table's name.
    pub table_name: String,
    /// The game, e.g. "nlhe".
    pub game: String,
    /// Whose view it was.
    pub who: String,
}

/// One seat in a hand's record.
#[cfg(feature = "serde")]
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RecordSeat {
    /// The seat's number in the view.
    pub i: usize,
    /// The name shown.
    pub name: String,
    /// Its cards, if the viewer saw them.
    pub cards: Option<Vec<String>>,
    /// Chips behind at the end.
    pub stack_end: u64,
    /// Net result.
    pub net: i64,
    /// Chips won.
    pub won: u64,
    /// Folded.
    pub folded: bool,
    /// Dealt in.
    pub in_hand: bool,
}

/// What's kept of a finished hand, as its viewer saw it: a host's view of
/// a club table shows no cards that weren't shown down, and a player's shows
/// their own, so a record never holds anyone's unshown cards. Seat numbers
/// are the viewer's (they're seat 0). Read leniently: a record kept by an
/// older page reads with what it has.
#[cfg(feature = "serde")]
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HandRecord {
    /// `club:table:hand:at`.
    pub id: String,
    /// The club's code.
    pub club: String,
    /// The table's id.
    pub table: String,
    /// The table's name.
    pub table_name: String,
    /// The game.
    pub game: String,
    /// Whose view it was.
    pub who: String,
    /// When it was kept, in milliseconds.
    pub at: u64,
    /// The hand's number at its table.
    pub hand: u64,
    /// The small blind.
    pub sb: u64,
    /// The big blind.
    pub bb: u64,
    /// The button's seat.
    pub button: usize,
    /// Hole cards per player.
    pub hole_cards: usize,
    /// The board.
    pub board: Vec<String>,
    /// Run twice, or a bomb pot's second board.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub board2: Vec<String>,
    /// It went to a showdown.
    pub showdown: bool,
    /// Chips in the pots.
    pub pot: u64,
    /// Every seat.
    pub seats: Vec<RecordSeat>,
    /// What happened, in order.
    pub events: Vec<Event>,
}

#[cfg(feature = "serde")]
impl HandRecord {
    /// The record of the finished hand in `view`.
    pub fn new(view: &TableView, of: RecordOf, now: u64) -> HandRecord {
        let seats: Vec<RecordSeat> = view
            .seats
            .iter()
            .enumerate()
            .map(|(i, s)| RecordSeat {
                i,
                name: s.name.clone(),
                cards: s.cards.clone(),
                stack_end: s.stack,
                net: s.net,
                won: s.won,
                folded: s.folded,
                in_hand: !s.empty && !s.sitting_out,
            })
            .collect();
        let pot = match view.pots.iter().map(|p| p.amount).sum() {
            0 => seats.iter().map(|s| s.won).sum(),
            p => p,
        };
        HandRecord {
            id: format!("{}:{}:{}:{now}", of.club, of.table, view.hand_number),
            club: of.club,
            table: of.table,
            table_name: of.table_name,
            game: of.game,
            who: of.who,
            at: now,
            hand: view.hand_number,
            sb: view.small_blind,
            bb: view.big_blind,
            button: view.button,
            hole_cards: view.hole_cards,
            board: view.board.clone(),
            board2: view.second_board.clone(),
            showdown: view.showdown,
            pot,
            seats,
            events: view.events.clone(),
        }
    }

    /// The hand replayed ([`crate::replay::replay_frames`]), each seat
    /// starting from its final stack less its net result.
    pub fn frames(&self) -> Vec<crate::replay::Frame> {
        let start: Vec<u64> = self
            .seats
            .iter()
            .map(|s| match s.in_hand {
                true => s.stack_end.saturating_add_signed(-s.net),
                false => s.stack_end,
            })
            .collect();
        let names: Vec<String> = self.seats.iter().map(|s| s.name.clone()).collect();
        crate::replay::replay_frames(&self.events, &start, &names)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::hand::Street;

    fn seat(street_bet: u64, all_in: bool) -> SeatChips {
        SeatChips {
            street_bet,
            all_in,
            ..Default::default()
        }
    }

    fn pot(amount: u64, eligible: &[usize]) -> ShownPot {
        ShownPot {
            amount,
            eligible: eligible.to_vec(),
        }
    }

    fn board() -> Event {
        Event::Board {
            street: Street::Flop,
            cards: vec![],
        }
    }

    // You (seat 0) shove $500 at 1/2; a $200 stack calls all-in.
    fn shove() -> Vec<Event> {
        vec![
            Event::SmallBlind {
                seat: 1,
                amount: 100,
            },
            Event::BigBlind {
                seat: 0,
                amount: 200,
            },
            Event::Raise {
                seat: 1,
                to: 600,
                all_in: false,
            },
            Event::Raise {
                seat: 0,
                to: 50000,
                all_in: true,
            },
            Event::Call {
                seat: 1,
                amount: 19400,
                all_in: true,
            },
        ]
    }

    #[test]
    fn a_shove_a_shorter_stack_calls_the_rest_goes_back() {
        let events = shove();
        assert_eq!(contributions(&events, 2).0, vec![50000, 20000]);
        let seats = [seat(50000, true), seat(20000, true)];
        assert!(betting_closed(&seats));
        assert_eq!(
            shown_pots(&seats, &events),
            Some(Pots {
                pots: vec![pot(40000, &[0, 1])],
                uncalled: Some(Uncalled {
                    seat: 0,
                    amount: 30000
                }),
            })
        );
    }

    #[test]
    fn side_pots_for_each_all_in_level() {
        // All-in for 20 and 50, two put in 100, one folded after 30.
        let r = split_pots(&[20, 50, 100, 100, 30], &[false, false, false, false, true]);
        assert_eq!(
            r.pots,
            vec![
                pot(100, &[0, 1, 2, 3]),
                pot(100, &[1, 2, 3]),
                pot(100, &[2, 3])
            ]
        );
        assert_eq!(r.uncalled, None);
        // The biggest stack's extra that nobody matched isn't a pot.
        let r = split_pots(&[20, 50, 150], &[false, false, false]);
        assert_eq!(r.pots, vec![pot(60, &[0, 1, 2]), pot(60, &[1, 2])]);
        assert_eq!(
            r.uncalled,
            Some(Uncalled {
                seat: 2,
                amount: 100
            })
        );
        // Everyone else folded: that's just their pot.
        let r = split_pots(&[10, 30], &[true, false]);
        assert_eq!(r.pots, vec![pot(40, &[1])]);
        assert_eq!(r.uncalled, None);
    }

    #[test]
    fn one_pot_and_nothing_uncalled_is_shown_as_usual() {
        let events = vec![
            Event::SmallBlind {
                seat: 0,
                amount: 100,
            },
            Event::BigBlind {
                seat: 1,
                amount: 200,
            },
            Event::Call {
                seat: 0,
                amount: 100,
                all_in: false,
            },
            board(),
        ];
        assert_eq!(shown_pots(&[seat(0, false), seat(0, false)], &events), None);
    }

    #[test]
    fn a_bet_not_yet_answered_stays_in_front_of_the_bettor() {
        // Seat 2 went all-in for $50 preflop; seat 0 bets $100 on the flop
        // and seat 1 hasn't acted.
        let events = vec![
            Event::Call {
                seat: 0,
                amount: 5000,
                all_in: false,
            },
            Event::Call {
                seat: 1,
                amount: 5000,
                all_in: false,
            },
            Event::Call {
                seat: 2,
                amount: 5000,
                all_in: true,
            },
            board(),
            Event::Bet {
                seat: 0,
                to: 10000,
                all_in: false,
            },
        ];
        let seats = [seat(10000, false), seat(0, false), seat(0, true)];
        assert!(!betting_closed(&seats));
        assert_eq!(shown_pots(&seats, &events), None);
    }

    #[test]
    fn missed_blinds_and_antes_count() {
        let events = vec![
            Event::Ante {
                seat: 0,
                amount: 10,
            },
            Event::Ante {
                seat: 1,
                amount: 10,
            },
            Event::Post {
                seat: 1,
                dead: 100,
                live: 200,
            },
            Event::Raise {
                seat: 1,
                to: 600,
                all_in: false,
            },
        ];
        let (put, street) = contributions(&events, 2);
        assert_eq!(put, vec![10, 710]);
        assert_eq!(street, vec![0, 600]);
    }

    fn view(seats: &[SeatChips], events: Vec<Event>, pot: u64, complete: bool) -> TableView {
        TableView {
            hand_number: 1,
            street: Some(Street::Preflop),
            board: vec![],
            pot,
            current_bet: 0,
            button: 0,
            hero: 0,
            seat: 0,
            small_blind: 100,
            big_blind: 200,
            hole_cards: 2,
            pot_limit: false,
            ante: 0,
            to_act: None,
            legal: None,
            seats: seats
                .iter()
                .map(|s| SeatState {
                    name: String::new(),
                    id: String::new(),
                    human: true,
                    away: false,
                    empty: s.empty,
                    sitting_out: s.sitting_out,
                    stack: s.stack,
                    street_bet: s.street_bet,
                    folded: s.folded,
                    all_in: s.all_in,
                    cards: None,
                    won: 0,
                    net: 0,
                })
                .collect(),
            events,
            complete,
            showdown: false,
            pots: vec![],
            run_choice: false,
            runs: 0,
            second_board: vec![],
            bomb_pot: false,
        }
    }

    #[test]
    fn a_views_pots_while_the_hand_goes_on() {
        let seats = [seat(50000, true), seat(20000, true)];
        let v = view(&seats, shove(), 70000, false);
        assert_eq!(v.shown_pots(), shown_pots(&seats, &shove()));
        // Once it's over, the hand has paid the pots.
        assert_eq!(view(&seats, shove(), 70000, true).shown_pots(), None);
    }

    fn chips(stack: u64) -> SeatChips {
        SeatChips {
            stack,
            ..Default::default()
        }
    }

    #[test]
    fn the_next_big_blind() {
        let six = [chips(100); 6];
        // Six-handed: the button moves one seat, the big blind two after it.
        assert_eq!(next_big_blind(&six, 3, false), Some(0));
        assert_eq!(next_big_blind(&six, 4, false), Some(1));
        assert_eq!(next_big_blind(&six, 0, false), Some(3));
        // Heads-up the button posts the small blind, the other the big.
        assert_eq!(next_big_blind(&six[..2], 0, false), Some(0));
        assert_eq!(next_big_blind(&six[..2], 1, false), Some(1));
        // Empty seats and players sitting out are skipped.
        let gaps = [
            chips(100),
            SeatChips {
                empty: true,
                ..chips(0)
            },
            chips(100),
            SeatChips {
                sitting_out: true,
                ..chips(100)
            },
            chips(100),
        ];
        assert_eq!(next_big_blind(&gaps, 0, false), Some(0));
        assert_eq!(next_big_blind(&gaps, 4, false), Some(4));
        // A seat with no chips plays only where bots are topped up.
        let broke = [chips(100), chips(0), chips(100), chips(100)];
        assert_eq!(next_big_blind(&broke, 1, false), Some(0));
        assert_eq!(next_big_blind(&broke, 3, false), Some(3));
        assert_eq!(next_big_blind(&broke, 3, true), Some(2));
        // Fewer than two players: none.
        assert_eq!(
            next_big_blind(
                &[
                    chips(100),
                    SeatChips {
                        empty: true,
                        ..chips(0)
                    }
                ],
                0,
                false
            ),
            None
        );
    }

    fn cards(s: &str) -> Vec<String> {
        s.as_bytes()
            .chunks(2)
            .map(|c| String::from_utf8_lossy(c).into_owned())
            .collect()
    }

    fn words(hole: &str, board: &str) -> Option<String> {
        hand_words(&cards(hole), &cards(board))
    }

    #[test]
    fn holdem_hands_in_words() {
        let cases = [
            ("AsKd", "2c7h9s", "Ace high"),
            ("8s8d", "2c7h9s", "Pair of 8s"),
            ("AsAd", "2c7h9s", "Pair of aces"),
            ("KsKd", "7c7h2s", "Two pair, kings and 7s"),
            ("TsTd", "Tc7h2s", "Three of a kind, 10s"),
            ("8s7d", "6c5h4s", "Straight, 8 high"),
            ("Ah2d", "3c4h5s", "Straight, 5 high"),
            ("AsKs", "2s7s9s", "Flush, ace high"),
            ("9h9d", "9c2h2s", "Full house, 9s full of 2s"),
            ("QhQd", "Qc2h7sQs", "Four of a kind, queens"),
            ("9h8h", "7h6h5h", "Straight flush, 9 high"),
            ("AhKh", "QhJhTh", "Royal flush"),
        ];
        for (hole, board, want) in cases {
            assert_eq!(
                words(hole, board).as_deref(),
                Some(want),
                "{hole} on {board}"
            );
        }
    }

    #[test]
    fn the_turn_and_river_count_and_before_the_flop_nothing() {
        assert_eq!(words("AsKd", "2c7h9sAh").as_deref(), Some("Pair of aces"));
        assert_eq!(
            words("AsKd", "2c7h9sAhKs").as_deref(),
            Some("Two pair, aces and kings")
        );
        assert_eq!(words("AsKd", ""), None);
        assert_eq!(words("AsKd", "2c7h"), None);
        assert_eq!(hand_words(&["".into(), "".into()], &cards("2c7h9s")), None);
    }

    #[test]
    fn omaha_uses_exactly_two_hole_cards() {
        assert_eq!(words("AsKsQd2c", "JsTs3h").as_deref(), Some("Ace high"));
        assert_eq!(
            words("AsKsQd2c", "JsTs3s").as_deref(),
            Some("Flush, ace high")
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn a_record_keeps_what_the_viewer_saw() {
        // Ann (seat 0) raises, Bob calls, Ann folds the flop to a bet.
        let events = vec![
            Event::SmallBlind { seat: 0, amount: 1 },
            Event::BigBlind { seat: 1, amount: 2 },
            Event::Raise {
                seat: 0,
                to: 6,
                all_in: false,
            },
            Event::Call {
                seat: 1,
                amount: 4,
                all_in: false,
            },
            board(),
            Event::Bet {
                seat: 1,
                to: 8,
                all_in: false,
            },
            Event::Fold { seat: 0 },
            Event::Award {
                seat: 1,
                pot: 0,
                amount: 20,
            },
        ];
        let mut v = view(
            &[
                chips(194),
                chips(214),
                SeatChips {
                    empty: true,
                    ..chips(0)
                },
            ],
            events,
            0,
            true,
        );
        v.hand_number = 7;
        v.seats[0].name = "Ann".into();
        v.seats[0].net = -6;
        v.seats[0].folded = true;
        v.seats[0].cards = Some(vec!["As".into(), "Kd".into()]);
        v.seats[1].name = "Bob".into();
        v.seats[1].net = 6;
        v.seats[1].won = 20;
        let of = RecordOf {
            club: "ABCD-EFGH".into(),
            table: "t1".into(),
            table_name: "Main".into(),
            game: "nlhe".into(),
            who: "ann".into(),
        };
        let r = HandRecord::new(&v, of, 1000);
        assert_eq!(r.id, "ABCD-EFGH:t1:7:1000");
        assert_eq!(r.pot, 20, "the winnings when the view has no pots");
        assert_eq!(r.seats[0].cards, Some(vec!["As".into(), "Kd".into()]));
        assert_eq!(r.seats[1].cards, None);
        assert!(!r.seats[2].in_hand);
        let f = r.frames();
        assert_eq!(f.len(), r.events.len() + 1);
        assert_eq!(
            f[0].seats[..2].iter().map(|s| s.stack).collect::<Vec<_>>(),
            vec![200, 208],
            "start = end - net"
        );
        assert_eq!(
            f.last().unwrap().seats[..2]
                .iter()
                .map(|s| s.stack)
                .collect::<Vec<_>>(),
            vec![194, 214]
        );
        // Kept as JSON and read back the same; no second board, no board2.
        let json = serde_json::to_value(&r).unwrap();
        assert!(json.get("board2").is_none());
        assert_eq!(json["seats"][0]["stackEnd"], 194);
        assert_eq!(json["tableName"], "Main");
        assert_eq!(serde_json::from_value::<HandRecord>(json).unwrap(), r);
    }

    #[test]
    fn names_map_one_for_one() {
        assert_eq!(
            name_words("Pair of 8, 9 7 2").as_deref(),
            Some("Pair of 8s")
        );
        assert_eq!(
            name_words("Full House K full of T").as_deref(),
            Some("Full house, kings full of 10s")
        );
        assert_eq!(
            name_words("Two Pair Q over J, A").as_deref(),
            Some("Two pair, queens and jacks")
        );
        assert_eq!(name_words("nonsense"), None);
    }
}
