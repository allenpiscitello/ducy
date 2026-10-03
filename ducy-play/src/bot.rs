use ducy::deck::{Card, Deck};

use crate::{
    error::PlayError,
    hand::{Action, Event, Hand, HandResult, LegalActions, Street},
    rules::TableRules,
};

/// What a bot may see about one seat: public chip counts, never hole cards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SeatView {
    /// Chips left behind.
    pub stack: u64,
    /// Chips put in on this street.
    pub street_bet: u64,
    /// Chips put in during the whole hand.
    pub contributed: u64,
    /// Whether the seat folded.
    pub folded: bool,
    /// Whether the seat is all-in.
    pub all_in: bool,
}

/// Everything the player to act is allowed to know when deciding: their own
/// hole cards plus public information. Other players' hole cards are never
/// included.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Observation {
    /// The acting seat.
    pub seat: usize,
    /// The acting seat's hole cards.
    pub hole_cards: Deck,
    /// Game, betting structure, blinds and ante.
    pub rules: TableRules,
    /// Current street.
    pub street: Street,
    /// Board cards revealed so far.
    pub board: Vec<Card>,
    /// The button seat.
    pub button: usize,
    /// Total chips in the middle, including bets on this street.
    pub pot: u64,
    /// The highest bet on this street.
    pub current_bet: u64,
    /// Every seat's public chip state, by seat number.
    pub seats: Vec<SeatView>,
    /// What the acting seat may do.
    pub legal: LegalActions,
    /// Everything that has happened so far.
    pub history: Vec<Event>,
}

/// What a bot learns when a hand ends.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct HandSummary {
    /// The seat this bot played.
    pub seat: usize,
    /// Game, betting structure, blinds and ante.
    pub rules: TableRules,
    /// Pots, payouts and net results.
    pub result: HandResult,
    /// Hole cards revealed at showdown, by seat (`None` for seats that folded
    /// or when there was no showdown, except the bot's own seat).
    pub shown: Vec<Option<Deck>>,
    /// The board as far as it was dealt.
    pub board: Vec<Card>,
    /// The full action history.
    pub history: Vec<Event>,
}

/// A poker player. Implement this to plug a strategy into
/// [`play_hand`] or [`crate::run_match`]; to write a bot in another
/// language, run it as a separate program with [`crate::ProcessBot`].
pub trait Bot {
    /// Chooses an action for `obs.seat`. Returning `None`, or an action that
    /// isn't legal, makes the bot check if it can and fold otherwise.
    fn act(&mut self, obs: &Observation) -> Option<Action>;

    /// Called on every bot when a hand ends. Does nothing by default.
    fn hand_over(&mut self, _summary: &HandSummary) {}
}

/// The default when a bot doesn't give a legal action: check, else fold.
pub fn fallback_action(legal: &LegalActions) -> Action {
    if legal.can_check {
        Action::Check
    } else {
        Action::Fold
    }
}

impl Hand {
    /// What `seat` may see right now, if it is that seat's turn.
    pub fn observation(&self, seat: usize) -> Option<Observation> {
        let legal = self.legal_actions().filter(|l| l.seat == seat)?;
        Some(Observation {
            seat,
            hole_cards: self.deal().hole_cards()[seat],
            rules: *self.rules(),
            street: self.street(),
            board: self.board().to_vec(),
            button: self.button(),
            pot: self.pot(),
            current_bet: self.current_bet(),
            seats: self.seat_views(),
            legal,
            history: self.events().to_vec(),
        })
    }

    /// Every seat's public chip state.
    pub fn seat_views(&self) -> Vec<SeatView> {
        (0..self.num_seats())
            .map(|s| SeatView {
                stack: self.stack(s),
                street_bet: self.street_bet(s),
                contributed: self.contributed(s),
                folded: self.has_folded(s),
                all_in: self.is_all_in(s),
            })
            .collect()
    }

    /// The end-of-hand summary for `seat`, once the hand is complete.
    pub fn summary(&self, seat: usize) -> Option<HandSummary> {
        let result = self.result()?.clone();
        let shown = (0..self.num_seats())
            .map(|s| {
                let revealed = s == seat || (result.showdown && !self.has_folded(s));
                revealed.then(|| self.deal().hole_cards()[s])
            })
            .collect();
        Some(HandSummary {
            seat,
            rules: *self.rules(),
            result,
            shown,
            board: self.board().to_vec(),
            history: self.events().to_vec(),
        })
    }
}

/// The outcome of [`play_hand`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandOutcome {
    /// Pots, payouts and net results.
    pub result: HandResult,
    /// For each seat, how many times its bot gave no action or an illegal
    /// one and the fallback was used instead.
    pub fallbacks: Vec<u32>,
}

/// Plays `hand` to the end, asking `bots[seat]` for each decision, then tells
/// every bot how the hand went.
///
/// A bot that returns `None` or an illegal action checks if it can and folds
/// otherwise, so one broken bot can't stall the table.
pub fn play_hand(hand: &mut Hand, bots: &mut [&mut dyn Bot]) -> Result<HandOutcome, PlayError> {
    if bots.len() != hand.num_seats() {
        return Err(PlayError::InvalidPlayerCount);
    }
    let mut fallbacks = vec![0u32; bots.len()];
    while let Some(seat) = hand.to_act() {
        let obs = hand.observation(seat).ok_or(PlayError::HandComplete)?;
        let played = match bots[seat].act(&obs) {
            Some(action) => hand.act(action).is_ok(),
            None => false,
        };
        if !played {
            fallbacks[seat] += 1;
            hand.act(fallback_action(&obs.legal))?;
        }
    }
    for (seat, bot) in bots.iter_mut().enumerate() {
        if let Some(summary) = hand.summary(seat) {
            bot.hand_over(&summary);
        }
    }
    let result = hand.result().ok_or(PlayError::HandComplete)?.clone();
    Ok(HandOutcome { result, fallbacks })
}
