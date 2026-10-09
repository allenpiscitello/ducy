use ducy::deck::{Card, Deck};

use crate::{
    deal::{Deal, Dealer, HiddenDeal},
    error::PlayError,
    rules::{BettingStructure, TableRules},
    showdown::{Pot, best_hands, build_pots, split},
    snapshot::HandSnapshot,
};

/// Most players at one table.
pub const MAX_PLAYERS: usize = 10;

/// A betting round.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum Street {
    /// Hole cards dealt, blinds posted.
    Preflop,
    /// Three board cards.
    Flop,
    /// Fourth board card.
    Turn,
    /// Fifth board card.
    River,
}

impl Street {
    fn next(self) -> Option<Self> {
        match self {
            Self::Preflop => Some(Self::Flop),
            Self::Flop => Some(Self::Turn),
            Self::Turn => Some(Self::River),
            Self::River => None,
        }
    }

    /// Board cards visible on this street.
    pub fn board_cards(self) -> usize {
        match self {
            Self::Preflop => 0,
            Self::Flop => 3,
            Self::Turn => 4,
            Self::River => 5,
        }
    }
}

/// A player decision. Bet and raise amounts are the player's **total** for
/// the street ("raise to"), not the amount added.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Give up the hand. Only allowed when facing a bet.
    Fold,
    /// Pass when there is nothing to call.
    Check,
    /// Match the current bet, or go all-in for less.
    Call,
    /// Open the betting on a street, to this street total.
    Bet(u64),
    /// Raise the current bet to this street total.
    Raise(u64),
    /// Put every remaining chip in: a call, bet or raise depending on the
    /// spot. Only allowed where that call, bet or raise is legal.
    AllIn,
}

/// The smallest and largest legal street totals for a bet or raise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RaiseRange {
    /// Smallest legal total. Below a full raise only when it puts the player
    /// all-in.
    pub min_to: u64,
    /// Largest legal total: the player's stack, or the pot limit.
    pub max_to: u64,
}

/// What the player to act may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LegalActions {
    /// The seat to act.
    pub seat: usize,
    /// Folding is allowed (the player faces a bet).
    pub can_fold: bool,
    /// Checking is allowed (nothing to call).
    pub can_check: bool,
    /// Chips a call adds, if there is something to call. Less than the bet
    /// when the call puts the player all-in.
    pub call: Option<u64>,
    /// Bet range when no one has bet this street.
    pub bet: Option<RaiseRange>,
    /// Raise range when facing a bet. `None` when the player can't raise:
    /// they don't have enough chips, everyone else is all-in, or an all-in
    /// smaller than a full raise didn't reopen the betting for them.
    pub raise: Option<RaiseRange>,
}

/// Something that happened in the hand, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "type", rename_all = "snake_case"))]
pub enum Event {
    /// A seat posted an ante.
    Ante {
        /// The seat, as an index into the hand's stacks.
        seat: usize,
        /// Chips posted.
        amount: u64,
    },
    /// A seat posted the small blind (less if all-in).
    SmallBlind {
        /// The seat, as an index into the hand's stacks.
        seat: usize,
        /// Chips posted.
        amount: u64,
    },
    /// A seat posted the big blind (less if all-in).
    BigBlind {
        /// The seat, as an index into the hand's stacks.
        seat: usize,
        /// Chips posted.
        amount: u64,
    },
    /// A seat coming back after missing blinds posted them: `dead` goes
    /// into the pot without counting toward their bet, `live` counts as
    /// their bet (see [`Post`]).
    Post {
        /// The seat, as an index into the hand's stacks.
        seat: usize,
        /// Chips into the pot that don't count toward their bet.
        dead: u64,
        /// Chips that count as their bet for the street.
        live: u64,
    },
    /// A seat folded.
    Fold {
        /// The seat, as an index into the hand's stacks.
        seat: usize,
    },
    /// A seat checked.
    Check {
        /// The seat, as an index into the hand's stacks.
        seat: usize,
    },
    /// A seat called, adding `amount`.
    Call {
        /// The seat, as an index into the hand's stacks.
        seat: usize,
        /// Chips added to call.
        amount: u64,
        /// Whether that was all their chips.
        all_in: bool,
    },
    /// A seat bet to `to` for the street.
    Bet {
        /// The seat, as an index into the hand's stacks.
        seat: usize,
        /// Their total bet for the street.
        to: u64,
        /// Whether that was all their chips.
        all_in: bool,
    },
    /// A seat raised to `to` for the street.
    Raise {
        /// The seat, as an index into the hand's stacks.
        seat: usize,
        /// Their total bet for the street.
        to: u64,
        /// Whether that was all their chips.
        all_in: bool,
    },
    /// Board cards dealt for a new street.
    Board {
        /// The street dealt.
        street: Street,
        /// The new cards (3 on the flop, then 1).
        cards: Vec<Card>,
    },
    /// At showdown, a seat whose hole cards the engine didn't know showed
    /// them (see [`Dealer`]).
    Reveal {
        /// The seat, as an index into the hand's stacks.
        seat: usize,
        /// Its hole cards.
        cards: Vec<Card>,
    },
    /// At showdown, a seat didn't show its hole cards: it can't win.
    Forfeit {
        /// The seat, as an index into the hand's stacks.
        seat: usize,
    },
    /// The players all-in chose how many times to run the rest of the
    /// board (see [`Hand::choose_runs`]): 1, or 2 to run it twice.
    Runs {
        /// 1 or 2.
        count: u8,
    },
    /// Running it twice: the second run's new cards, dealt after the first
    /// run's board. Each pot is split between the two boards.
    SecondBoard {
        /// The cards the second board doesn't share with the first.
        cards: Vec<Card>,
    },
    /// A seat won chips from pot `pot` (0 is the main pot).
    Award {
        /// The seat, as an index into the hand's stacks.
        seat: usize,
        /// Which pot: 0 is the main pot, then the side pots in order.
        pot: usize,
        /// Chips won.
        amount: u64,
    },
}

/// The outcome of a finished hand.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct HandResult {
    /// Main pot first, then side pots.
    pub pots: Vec<Pot>,
    /// Whether hands were compared. False when everyone else folded.
    pub showdown: bool,
    /// Chips each seat collected from the pots.
    pub payouts: Vec<u64>,
    /// Each seat's stack after the hand.
    pub final_stacks: Vec<u64>,
    /// Each seat's profit or loss.
    pub net: Vec<i64>,
}

#[derive(Clone, Debug)]
struct Seat {
    starting_stack: u64,
    stack: u64,
    street_bet: u64,
    contributed: u64,
    folded: bool,
    acted: bool,
    can_raise: bool,
}

impl Seat {
    fn all_in(&self) -> bool {
        self.stack == 0
    }

    fn can_act(&self) -> bool {
        !self.folded && !self.all_in()
    }
}

/// What a hand is waiting for from outside before it can go on, with a
/// [`Dealer`] that doesn't give the engine every card (see [`Hand::awaiting`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Awaiting {
    /// The board cards for this street: [`Hand::deal_board`].
    Board(Street),
    /// These seats' hole cards at showdown: [`Hand::reveal`] or
    /// [`Hand::forfeit`] each.
    Reveals(Vec<usize>),
    /// No more betting is possible and board cards are still to come: these
    /// seats (everyone still in) choose whether to run it twice
    /// ([`Hand::choose_runs`]). Only when offered ([`Hand::offer_run_twice`]).
    RunChoice(Vec<usize>),
}

/// Blinds a player posts on coming back after missing them (see
/// [`Hand::with_posts`]): `dead` goes straight into the pot (a missed small
/// blind), `live` counts as their bet for the street (a missed big blind,
/// so they may check if no one raises). Either may be 0, and both are
/// capped at the player's stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Post {
    /// The player, as an index into the hand's stacks.
    pub player: usize,
    /// Chips into the pot that don't count toward their bet.
    pub dead: u64,
    /// Chips that count as their bet for the street.
    pub live: u64,
}

/// One hand of poker from the blinds to the payout.
///
/// Create it with [`Hand::new`], then repeatedly ask [`Hand::to_act`] /
/// [`Hand::legal_actions`] and call [`Hand::act`] until
/// [`Hand::result`] returns the outcome. Board cards are revealed as streets
/// are reached, and when no more betting is possible (everyone left is
/// all-in) the rest of the board is dealt and the hand goes to showdown.
///
/// Rules followed:
/// - Antes, then blinds: the small blind is the seat after the button and the
///   big blind the next seat; heads-up, the button posts the small blind.
/// - Preflop the seat after the big blind acts first (heads-up, the button);
///   on later streets the first live seat after the button.
/// - The minimum bet is the big blind and a raise must be at least the size
///   of the previous bet or raise on the street, unless it puts the player
///   all-in. An all-in that is smaller than a full raise does not let players
///   who already acted raise again.
/// - Pot-limit raises are capped at the pot after calling.
/// - Side pots are built from each player's total contribution; uncalled
///   chips go back to their owner. Split pots give leftover chips to the
///   winners closest to the left of the button.
#[derive(Clone, Debug)]
pub struct Hand {
    rules: TableRules,
    /// The whole deal, when every card was known from the start.
    deal: Option<Deal>,
    /// Each seat's hole cards as far as the engine knows them: `None` until
    /// a hidden hand is shown at showdown.
    hole: Vec<Option<Deck>>,
    /// Seats that didn't show at showdown.
    forfeited: Vec<bool>,
    /// Dealt by a dealer that hid cards from the engine.
    hidden: bool,
    /// The board cards known so far, in order (all five up front with a
    /// [`Deal`]).
    board: Vec<Card>,
    awaiting: Option<Awaiting>,
    /// Running it twice may be offered when everyone left is all-in.
    run_offer: bool,
    /// How many times the board is run: 0 until chosen.
    runs: u8,
    /// Board cards both runs share (those dealt before the choice).
    run_from: usize,
    /// The second run's whole board, when running it twice.
    second_board: Vec<Card>,
    button: usize,
    seats: Vec<Seat>,
    street: Street,
    current_bet: u64,
    last_raise: u64,
    to_act: Option<usize>,
    events: Vec<Event>,
    result: Option<HandResult>,
}

impl Hand {
    /// Starts a hand: validates the setup, posts antes and blinds, and finds
    /// the first player to act. `stacks[i]` is seat `i`'s stack and the deal
    /// must have hole cards for every seat.
    pub fn new(
        rules: TableRules,
        stacks: &[u64],
        button: usize,
        deal: Deal,
    ) -> Result<Self, PlayError> {
        Self::with_posts(rules, stacks, button, deal, &[])
    }

    /// Like [`Hand::new`], with extra blinds posted after the regular ones:
    /// players coming back after sitting out who pay the blinds they missed.
    pub fn with_posts(
        rules: TableRules,
        stacks: &[u64],
        button: usize,
        deal: Deal,
        posts: &[Post],
    ) -> Result<Self, PlayError> {
        let mut hand = Self::with_dealer(rules, stacks, button, &deal, posts)?;
        hand.deal = Some(deal);
        Ok(hand)
    }

    /// Like [`Hand::with_posts`], with the cards from any [`Dealer`]. With a
    /// dealer that hides cards from the engine, the hand stops to wait for
    /// them when they're needed: see [`Hand::awaiting`].
    pub fn with_dealer(
        rules: TableRules,
        stacks: &[u64],
        button: usize,
        dealer: &dyn Dealer,
        posts: &[Post],
    ) -> Result<Self, PlayError> {
        rules.validate()?;
        let n = stacks.len();
        if !(2..=MAX_PLAYERS).contains(&n) {
            return Err(PlayError::InvalidPlayerCount);
        }
        if button >= n || stacks.contains(&0) {
            return Err(PlayError::InvalidSetup);
        }
        if dealer.players() != n {
            return Err(PlayError::InvalidDeal);
        }
        let hole: Vec<Option<Deck>> = (0..n).map(|i| dealer.hole_cards(i)).collect();
        let board = dealer.board().map(|b| b.to_vec()).unwrap_or_default();
        let hidden = board.len() < 5 || hole.iter().any(Option::is_none);
        // The cards the engine knows: the right number each, none twice.
        let mut seen = Deck::empty();
        for h in hole.iter().flatten() {
            if h.num_cards() as usize != rules.variant.hole_cards()
                || u64::from(seen) & u64::from(*h) != 0
            {
                return Err(PlayError::InvalidDeal);
            }
            seen |= *h;
        }
        for &c in &board {
            if seen.has_card(&c) {
                return Err(PlayError::InvalidDeal);
            }
            seen |= c;
        }

        let seats = stacks
            .iter()
            .map(|&stack| Seat {
                starting_stack: stack,
                stack,
                street_bet: 0,
                contributed: 0,
                folded: false,
                acted: false,
                can_raise: true,
            })
            .collect();
        let mut hand = Self {
            rules,
            deal: None,
            hole,
            forfeited: vec![false; n],
            hidden,
            board,
            awaiting: None,
            run_offer: false,
            runs: 0,
            run_from: 0,
            second_board: Vec::new(),
            button,
            seats,
            street: Street::Preflop,
            current_bet: rules.big_blind,
            last_raise: rules.big_blind,
            to_act: None,
            events: Vec::new(),
            result: None,
        };
        hand.post_forced_bets();
        for p in posts {
            if p.player >= n || p.live > rules.big_blind {
                return Err(PlayError::InvalidSetup);
            }
            let s = &mut hand.seats[p.player];
            let dead = p.dead.min(s.stack);
            s.stack -= dead;
            s.contributed += dead;
            let live = hand.put_in(p.player, p.live);
            hand.events.push(Event::Post {
                seat: p.player,
                dead,
                live,
            });
        }

        let first = if n == 2 {
            button
        } else {
            (hand.big_blind_seat() + 1) % n
        };
        hand.to_act = hand.find_to_act(first);
        if hand.to_act.is_none() {
            hand.finish_street()?;
        }
        Ok(hand)
    }

    fn small_blind_seat(&self) -> usize {
        if self.seats.len() == 2 {
            self.button
        } else {
            (self.button + 1) % self.seats.len()
        }
    }

    fn big_blind_seat(&self) -> usize {
        (self.small_blind_seat() + 1) % self.seats.len()
    }

    fn post_forced_bets(&mut self) {
        if self.rules.ante > 0 {
            for seat in 0..self.seats.len() {
                let s = &mut self.seats[seat];
                let amount = s.stack.min(self.rules.ante);
                s.stack -= amount;
                s.contributed += amount;
                self.events.push(Event::Ante { seat, amount });
            }
        }
        let (sb, bb) = (self.small_blind_seat(), self.big_blind_seat());
        let amount = self.put_in(sb, self.rules.small_blind);
        self.events.push(Event::SmallBlind { seat: sb, amount });
        let amount = self.put_in(bb, self.rules.big_blind);
        self.events.push(Event::BigBlind { seat: bb, amount });
    }

    /// Moves up to `amount` from the seat's stack into the pot, returning the
    /// chips actually added.
    fn put_in(&mut self, seat: usize, amount: u64) -> u64 {
        let s = &mut self.seats[seat];
        let amount = amount.min(s.stack);
        s.stack -= amount;
        s.street_bet += amount;
        s.contributed += amount;
        amount
    }

    /// Whether `seat` still has to act on this street.
    fn needs_action(&self, seat: usize) -> bool {
        let s = &self.seats[seat];
        if !s.can_act() {
            return false;
        }
        if s.street_bet < self.current_bet {
            return true;
        }
        let others_can_act = self
            .seats
            .iter()
            .enumerate()
            .any(|(i, o)| i != seat && o.can_act());
        !s.acted && others_can_act
    }

    /// The first seat at or after `start` that still has to act.
    fn find_to_act(&self, start: usize) -> Option<usize> {
        let n = self.seats.len();
        (0..n)
            .map(|i| (start + i) % n)
            .find(|&s| self.needs_action(s))
    }

    /// Seat whose turn it is, or `None` once the hand is complete.
    pub fn to_act(&self) -> Option<usize> {
        self.to_act
    }

    /// What the player to act may do, or `None` once the hand is complete.
    pub fn legal_actions(&self) -> Option<LegalActions> {
        let seat = self.to_act?;
        let s = &self.seats[seat];
        let to_call = self.current_bet.saturating_sub(s.street_bet);
        let all_in_to = s.street_bet + s.stack;
        let others_can_act = self
            .seats
            .iter()
            .enumerate()
            .any(|(i, o)| i != seat && o.can_act());

        let aggression = if s.can_raise && s.stack > to_call && others_can_act {
            let max_to = match self.rules.structure {
                BettingStructure::NoLimit => all_in_to,
                BettingStructure::PotLimit => {
                    let pot: u64 = self.seats.iter().map(|o| o.contributed).sum();
                    all_in_to.min(self.current_bet + pot + to_call)
                }
            };
            let full = if self.current_bet == 0 {
                self.rules.big_blind
            } else {
                self.current_bet + self.last_raise
            };
            Some(RaiseRange {
                min_to: full.min(max_to),
                max_to,
            })
        } else {
            None
        };
        let (bet, raise) = if self.current_bet == 0 {
            (aggression, None)
        } else {
            (None, aggression)
        };

        Some(LegalActions {
            seat,
            can_fold: to_call > 0,
            can_check: to_call == 0,
            call: (to_call > 0).then(|| to_call.min(s.stack)),
            bet,
            raise,
        })
    }

    /// Plays `action` for the seat to act, then advances the hand: to the
    /// next player, the next street, or the showdown.
    pub fn act(&mut self, action: Action) -> Result<(), PlayError> {
        let legal = self.legal_actions().ok_or(PlayError::HandComplete)?;
        let seat = legal.seat;
        let all_in_to = self.seats[seat].street_bet + self.seats[seat].stack;

        let action = match action {
            Action::AllIn => match (legal.bet.or(legal.raise), legal.call) {
                (Some(range), _) if range.max_to == all_in_to => {
                    if legal.bet.is_some() {
                        Action::Bet(all_in_to)
                    } else {
                        Action::Raise(all_in_to)
                    }
                }
                (_, Some(call)) if call == self.seats[seat].stack => Action::Call,
                _ => return Err(PlayError::IllegalAction),
            },
            other => other,
        };

        match action {
            Action::Fold if legal.can_fold => {
                self.seats[seat].folded = true;
                self.events.push(Event::Fold { seat });
            }
            Action::Check if legal.can_check => {
                self.events.push(Event::Check { seat });
            }
            Action::Call => {
                let amount = legal.call.ok_or(PlayError::IllegalAction)?;
                self.put_in(seat, amount);
                let all_in = self.seats[seat].all_in();
                self.events.push(Event::Call {
                    seat,
                    amount,
                    all_in,
                });
            }
            Action::Bet(to) => {
                let range = legal.bet.ok_or(PlayError::IllegalAction)?;
                self.raise_to(seat, to, range)?;
                let all_in = self.seats[seat].all_in();
                self.events.push(Event::Bet { seat, to, all_in });
            }
            Action::Raise(to) => {
                let range = legal.raise.ok_or(PlayError::IllegalAction)?;
                self.raise_to(seat, to, range)?;
                let all_in = self.seats[seat].all_in();
                self.events.push(Event::Raise { seat, to, all_in });
            }
            _ => return Err(PlayError::IllegalAction),
        }
        self.seats[seat].acted = true;
        self.advance(seat)
    }

    fn raise_to(&mut self, seat: usize, to: u64, range: RaiseRange) -> Result<(), PlayError> {
        if to < range.min_to || to > range.max_to {
            return Err(PlayError::IllegalAction);
        }
        let increase = to - self.current_bet;
        // A bet counts as a raise from zero; last_raise starts at the big blind.
        let full = increase >= self.last_raise;
        self.put_in(seat, to - self.seats[seat].street_bet);
        self.current_bet = to;
        if full {
            self.last_raise = increase;
        }
        for (i, other) in self.seats.iter_mut().enumerate() {
            if i == seat || !other.can_act() {
                continue;
            }
            if full {
                other.can_raise = true;
            } else if other.acted {
                // An incomplete raise doesn't reopen the betting for players
                // who already acted.
                other.can_raise = false;
            }
            other.acted = false;
        }
        Ok(())
    }

    fn advance(&mut self, last: usize) -> Result<(), PlayError> {
        if self.seats.iter().filter(|s| !s.folded).count() == 1 {
            self.to_act = None;
            return self.settle(false);
        }
        self.to_act = self.find_to_act((last + 1) % self.seats.len());
        if self.to_act.is_none() {
            self.finish_street()?;
        }
        Ok(())
    }

    /// Deals the following streets until someone has to act, or settles the
    /// hand after the river.
    fn finish_street(&mut self) -> Result<(), PlayError> {
        loop {
            let Some(next) = self.street.next() else {
                return self.showdown();
            };
            // Everyone left is all-in with cards to come: they may choose to
            // run it twice first.
            if self.run_offer && self.runs == 0 && self.deal.is_some() && self.runout() {
                self.to_act = None;
                let players = (0..self.seats.len())
                    .filter(|&s| !self.seats[s].folded)
                    .collect();
                self.awaiting = Some(Awaiting::RunChoice(players));
                return Ok(());
            }
            // A board dealt street by street: wait for these cards.
            if self.board.len() < next.board_cards() {
                self.to_act = None;
                self.awaiting = Some(Awaiting::Board(next));
                return Ok(());
            }
            self.start_street(next);
            if self.to_act.is_some() {
                return Ok(());
            }
        }
    }

    fn start_street(&mut self, next: Street) {
        let shown = self.street.board_cards();
        self.street = next;
        let cards = self.board[shown..next.board_cards()].to_vec();
        self.events.push(Event::Board {
            street: next,
            cards,
        });
        self.current_bet = 0;
        self.last_raise = self.rules.big_blind;
        for s in &mut self.seats {
            s.street_bet = 0;
            s.acted = false;
            s.can_raise = true;
        }
        self.to_act = self.find_to_act((self.button + 1) % self.seats.len());
    }

    /// No more betting: two or more players still in, and at most one of
    /// them with chips left to bet.
    fn runout(&self) -> bool {
        let live = self.seats.iter().filter(|s| !s.folded).count();
        let can_bet = self.seats.iter().filter(|s| s.can_act()).count();
        live >= 2 && can_bet <= 1
    }

    /// Lets the players run it twice if everyone left goes all-in with board
    /// cards to come: the hand then waits for [`Hand::choose_runs`]
    /// ([`Awaiting::RunChoice`]). Only with a [`Deal`], which has the cards
    /// for a second board.
    pub fn offer_run_twice(&mut self, on: bool) {
        self.run_offer = on;
    }

    /// How many times the board is run (1 or 2), once chosen; 0 before.
    pub fn runs(&self) -> u8 {
        self.runs
    }

    /// The second run's board, when running it twice, as far as it's been
    /// dealt (all five cards once the hand reaches showdown).
    pub fn second_board(&self) -> &[Card] {
        let shown = self
            .events
            .iter()
            .any(|e| matches!(e, Event::SecondBoard { .. }));
        if shown { &self.second_board } else { &[] }
    }

    /// The players' choice ([`Awaiting::RunChoice`]): run the rest of the
    /// board once, or twice (`runs` 2), splitting each pot between the two
    /// boards. The second board shares the cards already dealt and takes the
    /// rest from the deck after the first.
    pub fn choose_runs(&mut self, runs: u8) -> Result<(), PlayError> {
        if !matches!(self.awaiting, Some(Awaiting::RunChoice(_))) || !(1..=2).contains(&runs) {
            return Err(PlayError::IllegalAction);
        }
        self.awaiting = None;
        self.runs = runs;
        self.run_from = self.street.board_cards();
        if runs == 2 {
            let deal = self.deal.as_ref().ok_or(PlayError::IllegalAction)?;
            let need = 5 - self.run_from;
            if deal.rest().len() < need {
                return Err(PlayError::NotEnoughCards);
            }
            self.second_board = self.board[..self.run_from].to_vec();
            self.second_board.extend_from_slice(&deal.rest()[..need]);
        }
        self.events.push(Event::Runs { count: runs });
        self.finish_street()
    }

    /// The seats still contesting the pot at showdown: in the hand and not
    /// forfeited.
    fn contesting(&self) -> Vec<usize> {
        (0..self.seats.len())
            .filter(|&s| !self.seats[s].folded && !self.forfeited[s])
            .collect()
    }

    /// Goes to showdown, first waiting for anyone whose hole cards the
    /// engine doesn't know to show them. Only shown hands can win; if
    /// everyone else has forfeited, the last player in wins unseen.
    fn showdown(&mut self) -> Result<(), PlayError> {
        let contesting = self.contesting();
        let unseen: Vec<usize> = contesting
            .iter()
            .copied()
            .filter(|&s| self.hole[s].is_none())
            .collect();
        if contesting.len() > 1 && !unseen.is_empty() {
            self.to_act = None;
            self.awaiting = Some(Awaiting::Reveals(unseen));
            return Ok(());
        }
        self.awaiting = None;
        if self.runs == 2
            && !self
                .events
                .iter()
                .any(|e| matches!(e, Event::SecondBoard { .. }))
        {
            self.events.push(Event::SecondBoard {
                cards: self.second_board[self.run_from..].to_vec(),
            });
        }
        self.settle(true)
    }

    /// What the hand is waiting for from outside, if anything: board cards
    /// it doesn't know, or hidden hands at showdown. Nobody acts meanwhile.
    pub fn awaiting(&self) -> Option<&Awaiting> {
        self.awaiting.as_ref()
    }

    /// Deals the cards for the street the hand is waiting for
    /// ([`Awaiting::Board`]): one card for the turn or river, three for the
    /// flop. The dealer vouches for them; the engine only checks it doesn't
    /// already know them.
    pub fn deal_board(&mut self, cards: &[Card]) -> Result<(), PlayError> {
        let Some(Awaiting::Board(street)) = self.awaiting else {
            return Err(PlayError::IllegalAction);
        };
        if self.board.len() + cards.len() != street.board_cards() || !self.cards_unseen(cards) {
            return Err(PlayError::InvalidDeal);
        }
        self.board.extend_from_slice(cards);
        self.awaiting = None;
        self.start_street(street);
        if self.to_act.is_none() {
            self.finish_street()?;
        }
        Ok(())
    }

    /// Whether none of `cards` is one the engine already knows, and none is
    /// there twice.
    fn cards_unseen(&self, cards: &[Card]) -> bool {
        let mut seen = Deck::empty();
        for h in self.hole.iter().flatten() {
            seen |= *h;
        }
        for &c in self.board.iter().chain(cards) {
            if seen.has_card(&c) {
                return false;
            }
            seen |= c;
        }
        true
    }

    /// At showdown, `seat` shows the hole cards the engine didn't know. The
    /// dealer checks they're really the seat's cards (e.g. against the
    /// trustless shuffle); the engine checks the count and that it doesn't
    /// already know them.
    pub fn reveal(&mut self, seat: usize, cards: Deck) -> Result<(), PlayError> {
        if !self.awaiting_reveal(seat) {
            return Err(PlayError::IllegalAction);
        }
        let list: Vec<Card> = cards.iter(false).collect();
        if list.len() != self.rules.variant.hole_cards() || !self.cards_unseen(&list) {
            return Err(PlayError::InvalidDeal);
        }
        self.hole[seat] = Some(cards);
        self.events.push(Event::Reveal { seat, cards: list });
        self.showdown()
    }

    /// At showdown, `seat` doesn't show (they declined, left, or ran out of
    /// time): they can't win any pot.
    pub fn forfeit(&mut self, seat: usize) -> Result<(), PlayError> {
        if !self.awaiting_reveal(seat) {
            return Err(PlayError::IllegalAction);
        }
        self.forfeited[seat] = true;
        self.events.push(Event::Forfeit { seat });
        self.showdown()
    }

    fn awaiting_reveal(&self, seat: usize) -> bool {
        matches!(&self.awaiting, Some(Awaiting::Reveals(seats)) if seats.contains(&seat))
            && self.hole[seat].is_none()
    }

    fn settle(&mut self, showdown: bool) -> Result<(), PlayError> {
        let n = self.seats.len();
        let contributed: Vec<u64> = self.seats.iter().map(|s| s.contributed).collect();
        // A forfeited hand can't win, but its chips stay in the pots it
        // reached, like a fold's. If every hand left was forfeited, they
        // share the pots as if none had been.
        let any_contesting = !self.contesting().is_empty();
        let out: Vec<bool> = (0..n)
            .map(|s| self.seats[s].folded || (any_contesting && self.forfeited[s]))
            .collect();
        let mut payouts = vec![0u64; n];
        let mut pots = Vec::new();
        let hole: Vec<Deck> = self
            .hole
            .iter()
            .map(|h| h.unwrap_or_else(Deck::empty))
            .collect();

        for (index, (amount, eligible)) in build_pots(&contributed, &out).into_iter().enumerate() {
            let shown = eligible.iter().all(|&s| self.hole[s].is_some());
            let contested = eligible.len() > 1 && shown && self.board.len() == 5;
            let (winners, winning_hand) = if contested {
                let board: [Card; 5] = self.board.clone().try_into().expect("five cards");
                let (winners, hand) = best_hands(self.rules.variant, &hole, &board, &eligible)?;
                (winners, Some(hand))
            } else {
                (eligible.clone(), None)
            };
            let awards = if winners.is_empty() {
                Vec::new()
            } else if contested && showdown && self.runs == 2 {
                // Run twice: half the pot on each board, the odd chip to the first.
                let second: [Card; 5] = self.second_board.clone().try_into().expect("five cards");
                let (winners2, _) = best_hands(self.rules.variant, &hole, &second, &eligible)?;
                let half = amount / 2;
                let mut awards = split(amount - half, &winners, self.button, n);
                awards.extend(split(half, &winners2, self.button, n));
                awards
            } else {
                split(amount, &winners, self.button, n)
            };
            for award in &awards {
                payouts[award.seat] += award.amount;
                self.events.push(Event::Award {
                    seat: award.seat,
                    pot: index,
                    amount: award.amount,
                });
            }
            pots.push(Pot {
                amount,
                eligible,
                awards,
                winning_hand,
            });
        }

        let final_stacks: Vec<u64> = self
            .seats
            .iter()
            .zip(&payouts)
            .map(|(s, p)| s.stack + p)
            .collect();
        let net = self
            .seats
            .iter()
            .zip(&final_stacks)
            .map(|(s, &f)| f as i64 - s.starting_stack as i64)
            .collect();
        self.to_act = None;
        self.result = Some(HandResult {
            pots,
            showdown,
            payouts,
            final_stacks,
            net,
        });
        Ok(())
    }

    /// The outcome once the hand is complete.
    pub fn result(&self) -> Option<&HandResult> {
        self.result.as_ref()
    }

    /// Whether the hand has finished.
    pub fn is_complete(&self) -> bool {
        self.result.is_some()
    }

    /// The current street.
    pub fn street(&self) -> Street {
        self.street
    }

    /// Board cards revealed so far.
    pub fn board(&self) -> &[Card] {
        &self.board[..self.street.board_cards()]
    }

    /// The cards for this hand, when every one was known from the start
    /// (a [`Deal`]); `None` with a dealer that hides them.
    pub fn deal(&self) -> Option<&Deal> {
        self.deal.as_ref()
    }

    /// `seat`'s hole cards, if the engine knows them: always with a
    /// [`Deal`]; with a hiding dealer, once shown at showdown.
    pub fn hole_cards(&self, seat: usize) -> Option<Deck> {
        self.hole.get(seat).copied().flatten()
    }

    /// Whether `seat` forfeited at showdown by not showing.
    pub fn has_forfeited(&self, seat: usize) -> bool {
        self.forfeited[seat]
    }

    /// The rules for this hand.
    pub fn rules(&self) -> &TableRules {
        &self.rules
    }

    /// The button seat.
    pub fn button(&self) -> usize {
        self.button
    }

    /// Number of seats.
    pub fn num_seats(&self) -> usize {
        self.seats.len()
    }

    /// Chips a seat has left behind (not yet in the pot).
    pub fn stack(&self, seat: usize) -> u64 {
        self.seats[seat].stack
    }

    /// Chips a seat has put in on the current street.
    pub fn street_bet(&self, seat: usize) -> u64 {
        self.seats[seat].street_bet
    }

    /// Chips a seat has put in during the whole hand, antes included.
    pub fn contributed(&self, seat: usize) -> u64 {
        self.seats[seat].contributed
    }

    /// Whether a seat has folded.
    pub fn has_folded(&self, seat: usize) -> bool {
        self.seats[seat].folded
    }

    /// Whether a seat is all-in.
    pub fn is_all_in(&self, seat: usize) -> bool {
        self.seats[seat].all_in()
    }

    /// Total chips in the middle, including bets on the current street.
    pub fn pot(&self) -> u64 {
        self.seats.iter().map(|s| s.contributed).sum()
    }

    /// The highest bet on the current street.
    pub fn current_bet(&self) -> u64 {
        self.current_bet
    }

    /// Everything that has happened, in order.
    pub fn events(&self) -> &[Event] {
        &self.events
    }
}

impl Hand {
    /// The hand as saved data: how it started and what has happened since.
    /// A hand with hidden cards saves only what the engine knows: its
    /// events carry the board dealt so far and the hands shown.
    pub fn snapshot(&self) -> HandSnapshot {
        HandSnapshot {
            rules: self.rules,
            stacks: self.seats.iter().map(|s| s.starting_stack).collect(),
            button: self.button,
            hole_cards: self
                .hole
                .iter()
                .map(|d| d.map(|d| d.iter(false).collect()).unwrap_or_default())
                .collect(),
            board: self.board.clone(),
            events: self.events.clone(),
            hidden: self.hidden,
            run_offer: self.run_offer,
            rest: self
                .deal
                .as_ref()
                .map(|d| d.rest().to_vec())
                .unwrap_or_default(),
        }
    }

    /// Loads a saved hand by playing its actions again. Fails with
    /// [`PlayError::InvalidSnapshot`] unless that gives exactly the events
    /// that were saved.
    pub fn restore(s: &HandSnapshot) -> Result<Self, PlayError> {
        let bad = |_| PlayError::InvalidSnapshot;
        if s.hidden {
            return Self::restore_hidden(s);
        }
        let hole_cards = s
            .hole_cards
            .iter()
            .map(|cards| {
                let mut d = Deck::empty();
                for &c in cards {
                    d |= c;
                }
                d
            })
            .collect();
        let board: [Card; 5] = s
            .board
            .clone()
            .try_into()
            .map_err(|_| PlayError::InvalidSnapshot)?;
        let mut deal = Deal::new(s.rules.variant, hole_cards, board).map_err(bad)?;
        if !s.rest.is_empty() {
            deal = deal.with_rest(s.rest.clone()).map_err(bad)?;
        }
        let posts = posts_of(s);
        let mut hand = Self::with_posts(s.rules, &s.stacks, s.button, deal, &posts).map_err(bad)?;
        hand.offer_run_twice(s.run_offer);
        for e in &s.events {
            if let Event::Runs { count } = *e {
                hand.choose_runs(count).map_err(bad)?;
                continue;
            }
            let action = match *e {
                Event::Fold { .. } => Action::Fold,
                Event::Check { .. } => Action::Check,
                Event::Call { .. } => Action::Call,
                Event::Bet { to, .. } => Action::Bet(to),
                Event::Raise { to, .. } => Action::Raise(to),
                _ => continue,
            };
            hand.act(action).map_err(bad)?;
        }
        if hand.events != s.events {
            return Err(PlayError::InvalidSnapshot);
        }
        Ok(hand)
    }

    /// A hidden hand, played again from its events: the actions, and the
    /// board cards and shown hands as they came in.
    fn restore_hidden(s: &HandSnapshot) -> Result<Self, PlayError> {
        let bad = |_| PlayError::InvalidSnapshot;
        let dealer = HiddenDeal {
            players: s.stacks.len(),
        };
        let mut hand =
            Self::with_dealer(s.rules, &s.stacks, s.button, &dealer, &posts_of(s)).map_err(bad)?;
        for e in &s.events {
            match e {
                Event::Fold { .. } => hand.act(Action::Fold),
                Event::Check { .. } => hand.act(Action::Check),
                Event::Call { .. } => hand.act(Action::Call),
                Event::Bet { to, .. } => hand.act(Action::Bet(*to)),
                Event::Raise { to, .. } => hand.act(Action::Raise(*to)),
                // Dealt by the hand itself once the board's cards are in.
                Event::Board { cards, .. } if hand.awaiting.is_some() => hand.deal_board(cards),
                Event::Reveal { seat, cards } => {
                    let mut d = Deck::empty();
                    for &c in cards {
                        d |= c;
                    }
                    hand.reveal(*seat, d)
                }
                Event::Forfeit { seat } => hand.forfeit(*seat),
                _ => continue,
            }
            .map_err(bad)?;
        }
        if hand.events != s.events {
            return Err(PlayError::InvalidSnapshot);
        }
        Ok(hand)
    }
}

/// The extra blinds a saved hand started with.
fn posts_of(s: &HandSnapshot) -> Vec<Post> {
    s.events
        .iter()
        .filter_map(|e| match *e {
            Event::Post { seat, dead, live } => Some(Post {
                player: seat,
                dead,
                live,
            }),
            _ => None,
        })
        .collect()
}
