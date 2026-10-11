//! Opponents' ranges from their actions, and equity against them (#205).

use ducy::deck::{Card, Deck};
use ducy_play::ranges::{BET_SHARE, made_score, preflop_top, range_equity, reads};
use ducy_play::stats::OpponentModel;
use ducy_play::strength::observation_equity;
use ducy_play::{Action, Deal, Hand, Street, TableRules, Variant};
use rand::{SeedableRng, rngs::StdRng};

fn cards(s: &str) -> Vec<Card> {
    s.split(' ').map(|c| Card::parse(c).unwrap()).collect()
}

/// Heads-up No-limit Hold'em with known cards; seat 0 is the button.
fn hand(holes: &[&str], board: &str) -> Hand {
    let holes = holes.iter().map(|h| Deck::parse(h).unwrap()).collect();
    let deal = Deal::new(Variant::Holdem, holes, cards(board).try_into().unwrap()).unwrap();
    Hand::new(TableRules::no_limit_holdem(1, 2), &[1000, 1000], 0, deal).unwrap()
}

#[test]
fn starting_hands_rank_sensibly() {
    let top = |v, h: &str| preflop_top(v, &cards(h));
    assert!(top(Variant::Holdem, "As Ah") < 0.01);
    // By equity against a random hand, small pairs rank above it.
    assert!(top(Variant::Holdem, "Ks Qs") < 0.15);
    assert!(top(Variant::Holdem, "7c 2d") > 0.9);
    let plo = Variant::Omaha { hole_cards: 4 };
    assert!(top(plo, "As Ah Ks Kh") < 0.05);
    assert!(top(plo, "2c 7d 3h 8s") > 0.6);
}

#[test]
fn omaha_scores_use_exactly_two_hole_cards() {
    let board = cards("2h 5h 9h Kh 3c");
    let plo = Variant::Omaha { hole_cards: 4 };
    // One heart in hand: no flush in Omaha (it would be in Hold'em).
    let one = cards("Ah Ac Ad 7s");
    let two = cards("Ah Qh 7s 7d");
    assert!(made_score(plo, &two, &board) > made_score(plo, &one, &board));
    let holdem_one = cards("Ah Ac");
    assert!(made_score(Variant::Holdem, &holdem_one, &board) > made_score(plo, &one, &board));
}

#[test]
fn raises_and_bets_narrow_the_read() {
    // Button raises, big blind calls; on the flop the button bets.
    let mut h = hand(&["Kd Kc", "9s 8s"], "Ks 7h 2d Tc 4s");
    h.act(Action::Raise(6)).unwrap();
    h.act(Action::Call).unwrap();
    h.act(Action::Check).unwrap();
    h.act(Action::Bet(8)).unwrap();
    let obs = h.observation(1).unwrap();
    assert_eq!(obs.street, Street::Flop);
    let r = reads(&obs, &OpponentModel::new());
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].seat, 0);
    assert!(
        r[0].preflop < 0.2,
        "a raise reads as the top of a range: {}",
        r[0].preflop
    );
    assert_eq!(r[0].streets, vec![(3, BET_SHARE)]);
}

#[test]
fn equity_against_a_strong_line_is_lower_than_against_random_hands() {
    // Middle pair facing a raise preflop and a bet on the flop.
    let mut h = hand(&["Ad Kd", "8s 7c"], "8h Qc 2d 5s Jh");
    h.act(Action::Raise(6)).unwrap();
    h.act(Action::Call).unwrap();
    h.act(Action::Check).unwrap();
    h.act(Action::Bet(10)).unwrap();
    let obs = h.observation(1).unwrap();
    let mut rng = StdRng::seed_from_u64(3);
    let random = observation_equity(&obs, 2000, &mut rng);
    let ranged = range_equity(&obs, &OpponentModel::new(), 2000, &mut rng);
    assert!(
        ranged < random - 0.05,
        "against the range {ranged:.2}, against random hands {random:.2}"
    );
}

#[test]
fn checked_to_on_a_limped_pot_reads_wide() {
    // Button limps, big blind checks: the big blind's range says nothing.
    let mut h = hand(&["Ad Kd", "8s 7c"], "8h Qc 2d 5s Jh");
    h.act(Action::Call).unwrap();
    h.act(Action::Check).unwrap();
    let obs = h.observation(1).unwrap();
    let r = reads(&obs, &OpponentModel::new());
    assert_eq!(r[0].seat, 0);
    assert!(
        r[0].preflop > 0.2,
        "a limp reads as its playing range: {}",
        r[0].preflop
    );
    assert!(r[0].streets.is_empty());
}
