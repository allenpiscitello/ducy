use ducy::deck::{Card, Deck};

use crate::{
    deal::Deal,
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
    deal: Deal,
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
        rules.validate()?;
        let n = stacks.len();
        if !(2..=MAX_PLAYERS).contains(&n) {
            return Err(PlayError::InvalidPlayerCount);
        }
        if button >= n || stacks.contains(&0) {
            return Err(PlayError::InvalidSetup);
        }
        if deal.hole_cards().len() != n
            || deal
                .hole_cards()
                .iter()
                .any(|h| h.num_cards() as usize != rules.variant.hole_cards())
        {
            return Err(PlayError::InvalidDeal);
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
            deal,
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
                return self.settle(true);
            };
            let shown = self.street.board_cards();
            self.street = next;
            let cards = self.deal.board()[shown..next.board_cards()].to_vec();
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
            if self.to_act.is_some() {
                return Ok(());
            }
        }
    }

    fn settle(&mut self, showdown: bool) -> Result<(), PlayError> {
        let n = self.seats.len();
        let contributed: Vec<u64> = self.seats.iter().map(|s| s.contributed).collect();
        let folded: Vec<bool> = self.seats.iter().map(|s| s.folded).collect();
        let mut payouts = vec![0u64; n];
        let mut pots = Vec::new();

        for (index, (amount, eligible)) in build_pots(&contributed, &folded).into_iter().enumerate()
        {
            let (winners, winning_hand) = if eligible.len() > 1 {
                let (winners, hand) = best_hands(
                    self.rules.variant,
                    self.deal.hole_cards(),
                    self.deal.board(),
                    &eligible,
                )?;
                (winners, Some(hand))
            } else {
                (eligible.clone(), None)
            };
            let awards = if winners.is_empty() {
                Vec::new()
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
        &self.deal.board()[..self.street.board_cards()]
    }

    /// The cards for this hand.
    pub fn deal(&self) -> &Deal {
        &self.deal
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
    pub fn snapshot(&self) -> HandSnapshot {
        HandSnapshot {
            rules: self.rules,
            stacks: self.seats.iter().map(|s| s.starting_stack).collect(),
            button: self.button,
            hole_cards: self
                .deal
                .hole_cards()
                .iter()
                .map(|d| d.iter(false).collect())
                .collect(),
            board: self.deal.board().to_vec(),
            events: self.events.clone(),
        }
    }

    /// Loads a saved hand by playing its actions again. Fails with
    /// [`PlayError::InvalidSnapshot`] unless that gives exactly the events
    /// that were saved.
    pub fn restore(s: &HandSnapshot) -> Result<Self, PlayError> {
        let bad = |_| PlayError::InvalidSnapshot;
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
        let deal = Deal::new(s.rules.variant, hole_cards, board).map_err(bad)?;
        let posts: Vec<Post> = s
            .events
            .iter()
            .filter_map(|e| match *e {
                Event::Post { seat, dead, live } => Some(Post {
                    player: seat,
                    dead,
                    live,
                }),
                _ => None,
            })
            .collect();
        let mut hand = Self::with_posts(s.rules, &s.stacks, s.button, deal, &posts).map_err(bad)?;
        for e in &s.events {
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
}
