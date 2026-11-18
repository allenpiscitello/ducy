//! Running it twice pot by pot: a pot runs twice only when everyone who can
//! win it agrees. The short stack can run the main pot once while the two
//! big stacks run their side pot twice.

use ducy::deck::{Card, Deck};
use ducy_play::{
    Action, Awaiting, Deal, Event, Hand, HandResult, HandSnapshot, Rake, TableRules, Variant,
    replay::replay_frames,
};

const BOARD: &str = "2c 7d 9h Jc 3s";
/// The second board, run from preflop: a queen for Cy.
const REST: &str = "Qd 4c 5d 8s Tc";

fn cards(s: &str) -> Vec<Card> {
    s.split_whitespace()
        .map(|c| Card::parse(c).unwrap())
        .collect()
}

fn deal(holes: &[&str]) -> Deal {
    let hole_cards = holes.iter().map(|h| Deck::parse(h).unwrap()).collect();
    Deal::new(
        Variant::Holdem,
        hole_cards,
        cards(BOARD).try_into().unwrap(),
    )
    .unwrap()
    .with_rest(cards(REST))
    .unwrap()
}

/// Ann (aces, 50) on the button shoves, Bob (kings, 100) shoves over her,
/// Cy (queens, 100) calls. Main pot 150 (all three), side pot 100 (Bob and
/// Cy). Ann wins the first board, Cy's set the second; Bob beats Cy on the
/// first.
fn three_way(rules: TableRules) -> Hand {
    let d = deal(&["As Ah", "Ks Kh", "Qs Qh"]);
    let mut hand = Hand::new(rules, &[50, 100, 100], 0, d).unwrap();
    hand.offer_run_twice(true);
    hand.act(Action::AllIn).unwrap();
    hand.act(Action::AllIn).unwrap();
    hand.act(Action::Call).unwrap();
    assert_eq!(hand.awaiting(), Some(&Awaiting::RunChoice(vec![0, 1, 2])));
    assert_eq!(hand.run_pots(), vec![vec![0, 1, 2], vec![1, 2]]);
    hand
}

fn rules() -> TableRules {
    TableRules::no_limit_holdem(1, 2)
}

fn pot_runs(r: &HandResult) -> Vec<u8> {
    r.pots.iter().map(|p| p.runs).collect()
}

fn awards(r: &HandResult, pot: usize) -> Vec<(usize, u64)> {
    r.pots[pot]
        .awards
        .iter()
        .map(|a| (a.seat, a.amount))
        .collect()
}

#[test]
fn the_side_pot_runs_twice_and_the_main_pot_once() {
    let mut hand = three_way(rules());
    hand.choose_pot_runs(&[1, 2]).unwrap();
    let r = hand.result().unwrap().clone();
    assert_eq!(pot_runs(&r), vec![1, 2]);
    // Ann's main pot on the first board alone.
    assert_eq!(awards(&r, 0), vec![(0, 150)]);
    // The side pot: Bob on the first board, Cy on the second.
    assert_eq!(awards(&r, 1), vec![(1, 50), (2, 50)]);
    assert_eq!(r.payouts, vec![150, 50, 50]);
    assert_eq!(hand.runs(), 2);
    assert_eq!(hand.pot_runs(), &[1, 2]);
    assert_eq!(hand.second_board(), cards(REST));
    assert!(hand.events().contains(&Event::Runs {
        count: 2,
        pots: vec![1, 2]
    }));
    assert!(
        hand.events()
            .contains(&Event::SecondBoard { cards: cards(REST) })
    );
}

#[test]
fn all_yes_splits_every_pot() {
    let mut hand = three_way(rules());
    hand.choose_runs(2).unwrap();
    let r = hand.result().unwrap();
    assert_eq!(pot_runs(r), vec![2, 2]);
    assert_eq!(awards(r, 0), vec![(0, 75), (2, 75)]);
    assert_eq!(r.payouts, vec![75, 50, 125]);
    assert!(hand.events().contains(&Event::Runs {
        count: 2,
        pots: vec![2, 2]
    }));
}

#[test]
fn all_no_is_the_usual_hand_with_one_board() {
    let mut hand = three_way(rules());
    hand.choose_pot_runs(&[1, 1]).unwrap();
    let r = hand.result().unwrap();
    assert_eq!(pot_runs(r), vec![1, 1]);
    assert_eq!(r.payouts, vec![150, 100, 0]);
    assert_eq!(hand.runs(), 1);
    assert!(hand.second_board().is_empty());
    assert!(
        !hand
            .events()
            .iter()
            .any(|e| matches!(e, Event::SecondBoard { .. }))
    );
    assert!(hand.events().contains(&Event::Runs {
        count: 1,
        pots: vec![1, 1]
    }));
}

#[test]
fn a_pot_one_player_can_win_runs_once() {
    // Cy (150) shoves on the button and Ann (50) and Bob (100) call: the 50
    // Bob can't match goes back to Cy, run once.
    let d = deal(&["Qs Qh", "As Ah", "Ks Kh"]);
    let mut hand = Hand::new(rules(), &[150, 50, 100], 0, d).unwrap();
    hand.offer_run_twice(true);
    hand.act(Action::AllIn).unwrap();
    hand.act(Action::Call).unwrap();
    hand.act(Action::Call).unwrap();
    assert_eq!(hand.run_pots(), vec![vec![0, 1, 2], vec![0, 2], vec![0]]);
    hand.choose_runs(2).unwrap();
    assert_eq!(hand.pot_runs(), &[2, 2, 1]);
    let r = hand.result().unwrap();
    assert_eq!(pot_runs(r), vec![2, 2, 1]);
    assert_eq!(awards(r, 2), vec![(0, 50)]);
}

#[test]
fn wrong_choices_are_refused() {
    let mut hand = three_way(rules());
    assert!(hand.choose_pot_runs(&[2]).is_err(), "one per pot");
    assert!(hand.choose_pot_runs(&[1, 3]).is_err());
    assert!(hand.awaiting().is_some(), "still waiting");
    hand.choose_pot_runs(&[1, 2]).unwrap();
    assert!(hand.choose_pot_runs(&[1, 2]).is_err(), "chosen already");
}

#[test]
fn the_odd_chip_of_a_side_pot_goes_to_the_first_board() {
    // Four players, Dee on 3 raises to 5 and folds to Bob's shove: her 5 make
    // the main pot 16 (Ann's 4 from each) and the side pot an odd 193.
    let d = deal(&["As Ah", "Ks Kh", "Qs Qh", "8h 8d"]);
    let mut hand = Hand::new(rules(), &[4, 100, 100, 100], 0, d).unwrap();
    hand.offer_run_twice(true);
    hand.act(Action::Raise(5)).unwrap(); // Dee
    hand.act(Action::AllIn).unwrap(); // Ann, 4
    hand.act(Action::AllIn).unwrap(); // Bob, 100
    hand.act(Action::Call).unwrap(); // Cy
    hand.act(Action::Fold).unwrap(); // Dee
    assert_eq!(hand.run_pots(), vec![vec![0, 1, 2], vec![1, 2]]);
    hand.choose_pot_runs(&[1, 2]).unwrap();
    let r = hand.result().unwrap();
    assert_eq!(r.pots[0].amount, 16);
    assert_eq!(r.pots[1].amount, 193);
    assert_eq!(awards(r, 0), vec![(0, 16)]);
    assert_eq!(awards(r, 1), vec![(1, 97), (2, 96)]);
    assert_eq!(r.payouts, vec![16, 97, 96, 0]);
}

#[test]
fn four_way_with_two_side_pots() {
    // Ann 30, Bob 60, Cy and Dee 100: main pot (all four), side pot 1 (Bob,
    // Cy, Dee), side pot 2 (Cy, Dee). Ann and Bob say no: only side pot 2
    // runs twice.
    let d = deal(&["As Ah", "Ks Kh", "Qs Qh", "Ts Th"]);
    let mut hand = Hand::new(rules(), &[30, 60, 100, 100], 0, d).unwrap();
    hand.offer_run_twice(true);
    hand.act(Action::AllIn).unwrap(); // Dee, under the gun
    hand.act(Action::AllIn).unwrap(); // Ann
    hand.act(Action::AllIn).unwrap(); // Bob
    hand.act(Action::Call).unwrap(); // Cy
    assert_eq!(
        hand.run_pots(),
        vec![vec![0, 1, 2, 3], vec![1, 2, 3], vec![2, 3]]
    );
    hand.choose_pot_runs(&[1, 1, 2]).unwrap();
    let r = hand.result().unwrap().clone();
    assert_eq!(pot_runs(&r), vec![1, 1, 2]);
    assert_eq!(awards(&r, 0), vec![(0, 120)]);
    assert_eq!(awards(&r, 1), vec![(1, 90)]);
    // Cy's queens beat Dee's tens on both boards.
    assert_eq!(awards(&r, 2), vec![(2, 40), (2, 40)]);
    assert_eq!(r.payouts, vec![120, 90, 80, 0]);
    let text: Vec<String> = replay_frames(hand.events(), &[30, 60, 100, 100], &[])
        .into_iter()
        .map(|f| f.text)
        .collect();
    assert!(
        text.contains(&"Side pot 2 runs twice, the rest once".to_string()),
        "{text:?}"
    );
}

#[test]
fn rake_comes_out_of_each_pot_whether_it_runs_once_or_twice() {
    let rake = Rake {
        percent: 1000,
        cap: 0,
        no_flop: true,
        time: 0,
    };
    let mut hand = three_way(rules().with_rake(rake));
    hand.choose_pot_runs(&[1, 2]).unwrap();
    let r = hand.result().unwrap();
    // 10% of 250: 15 from the main pot, 10 from the side pot.
    assert_eq!(r.house(), 25);
    assert_eq!(awards(r, 0), vec![(0, 135)]);
    assert_eq!(awards(r, 1), vec![(1, 45), (2, 45)]);
    assert_eq!(r.final_stacks.iter().sum::<u64>() + r.house(), 250);
}

#[test]
fn a_mixed_choice_restores_from_a_snapshot() {
    let mut hand = three_way(rules());
    hand.choose_pot_runs(&[1, 2]).unwrap();
    let json = serde_json::to_string(&hand.snapshot()).unwrap();
    let back = Hand::restore(&serde_json::from_str::<HandSnapshot>(&json).unwrap()).unwrap();
    assert_eq!(back.result(), hand.result());
    assert_eq!(back.pot_runs(), &[1, 2]);
    assert_eq!(back.second_board(), hand.second_board());
}

#[test]
fn the_replay_says_which_pot_runs_twice() {
    let mut hand = three_way(rules());
    hand.choose_pot_runs(&[1, 2]).unwrap();
    let text: Vec<String> = replay_frames(hand.events(), &[50, 100, 100], &[])
        .into_iter()
        .map(|f| f.text)
        .collect();
    assert!(
        text.contains(&"The side pot runs twice, the main pot once".to_string()),
        "{text:?}"
    );
}
