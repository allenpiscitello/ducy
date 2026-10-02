use ducy::{
    deck::{Card, Deck},
    games::{
        GameEvaluation,
        flop_game::FlopGame,
        holdem::{HoldemGameEvaluation, HoldemGameState},
        omaha::{OmahaGameEvaluation, OmahaGameState},
    },
};

use crate::{error::PlayError, rules::Variant};

/// One pot (main or side) and who won it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pot {
    /// Chips in the pot.
    pub amount: u64,
    /// Seats that can win it: every player still in the hand who put in at
    /// least this pot's level. A pot with a single eligible seat is an
    /// uncalled bet being returned, or a pot won without a showdown.
    pub eligible: Vec<usize>,
    /// `(seat, chips)` for each winner. Split pots divide evenly; leftover
    /// chips go one at a time to winners starting left of the button.
    pub awards: Vec<(usize, u64)>,
    /// The winning hand, e.g. "Flush, Ace high", when the pot went to showdown.
    pub winning_hand: Option<String>,
}

/// Splits everyone's total contribution into a main pot and side pots.
///
/// Each level is set by the smallest remaining contribution among players
/// still in the hand. Folded players' chips go into the pots they reached and
/// are never eligible to win.
pub(crate) fn build_pots(contributed: &[u64], folded: &[bool]) -> Vec<(u64, Vec<usize>)> {
    let mut remaining = contributed.to_vec();
    let mut pots: Vec<(u64, Vec<usize>)> = Vec::new();
    loop {
        let level = remaining
            .iter()
            .zip(folded)
            .filter(|&(&r, &f)| !f && r > 0)
            .map(|(&r, _)| r)
            .min();
        let Some(level) = level else { break };
        let eligible: Vec<usize> = (0..remaining.len())
            .filter(|&i| !folded[i] && remaining[i] >= level)
            .collect();
        let mut amount = 0;
        for r in remaining.iter_mut() {
            let take = (*r).min(level);
            *r -= take;
            amount += take;
        }
        pots.push((amount, eligible));
    }
    // Chips a folded player put in beyond every live player's contribution.
    let leftover: u64 = remaining.iter().sum();
    if leftover > 0 {
        match pots.last_mut() {
            Some(last) => last.0 += leftover,
            None => pots.push((leftover, Vec::new())),
        }
    }
    pots
}

/// The best hands among `seats` on a complete board, as seat numbers plus a
/// description of the winning hand.
pub(crate) fn best_hands(
    variant: Variant,
    hole_cards: &[Deck],
    board: &[Card; 5],
    seats: &[usize],
) -> Result<(Vec<usize>, String), PlayError> {
    let mut flop = Deck::empty();
    for &card in &board[..3] {
        flop |= card;
    }
    let (winners, hand): (Vec<usize>, String) = match variant {
        Variant::Holdem => {
            let mut state = HoldemGameState::new();
            setup(&mut state, hole_cards, seats, flop, board)?;
            let winners = HoldemGameEvaluation {}.evaluate_winners(&state);
            let hand = describe(winners.first().map(|w| w.winning_hand()));
            (winners.iter().map(|w| w.player_index()).collect(), hand)
        }
        Variant::Omaha { hole_cards: n } => {
            let mut state = OmahaGameState::new(n);
            setup(&mut state, hole_cards, seats, flop, board)?;
            let winners = OmahaGameEvaluation {}.evaluate_winners(&state);
            let hand = describe(winners.first().map(|w| w.winning_hand()));
            (winners.iter().map(|w| w.player_index()).collect(), hand)
        }
    };
    Ok((winners.into_iter().map(|i| seats[i]).collect(), hand))
}

fn describe(hand: Option<&impl std::fmt::Display>) -> String {
    hand.map(|h| h.to_string()).unwrap_or_default()
}

fn setup(
    state: &mut impl FlopGame,
    hole_cards: &[Deck],
    seats: &[usize],
    flop: Deck,
    board: &[Card; 5],
) -> Result<(), PlayError> {
    let invalid = |_| PlayError::InvalidDeal;
    for &seat in seats {
        state.add_player(hole_cards[seat]).map_err(invalid)?;
    }
    state.set_flop(flop).map_err(invalid)?;
    state.set_turn(board[3]).map_err(invalid)?;
    state.set_river(board[4]).map_err(invalid)
}

/// Splits `amount` evenly among `winners`, giving leftover chips one at a
/// time starting with the first winner left of the button.
pub(crate) fn split(
    amount: u64,
    winners: &[usize],
    button: usize,
    seats: usize,
) -> Vec<(usize, u64)> {
    let mut order = winners.to_vec();
    order.sort_by_key(|&s| (s + seats - button - 1) % seats);
    let share = amount / order.len() as u64;
    let extra = amount % order.len() as u64;
    order
        .iter()
        .enumerate()
        .map(|(i, &s)| (s, share + u64::from((i as u64) < extra)))
        .collect()
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_build_pots() {
        // Two live players, matched.
        assert_eq!(
            build_pots(&[10, 10], &[false, false]),
            vec![(20, vec![0, 1])]
        );
        // A short all-in makes a side pot; the deep stack's extra comes back.
        assert_eq!(
            build_pots(&[20, 50, 100], &[false, false, false]),
            vec![(60, vec![0, 1, 2]), (60, vec![1, 2]), (50, vec![2])]
        );
        // A folded player's chips count toward the pots they reached.
        assert_eq!(
            build_pots(&[30, 20, 50], &[true, false, false]),
            vec![(60, vec![1, 2]), (40, vec![2])]
        );
        // Folded chips beyond every live player's level join the last pot.
        assert_eq!(
            build_pots(&[40, 10, 10], &[true, false, false]),
            vec![(60, vec![1, 2])]
        );
    }

    #[test]
    fn test_split() {
        assert_eq!(split(10, &[1, 2], 0, 4), vec![(1, 5), (2, 5)]);
        // Leftover chips go to the winners closest to the left of the button.
        assert_eq!(split(11, &[0, 2], 1, 4), vec![(2, 6), (0, 5)]);
        assert_eq!(split(5, &[3, 0, 1], 3, 4), vec![(0, 2), (1, 2), (3, 1)]);
    }
}
