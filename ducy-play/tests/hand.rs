use ducy::deck::{Card, Deck};
use ducy_play::{
    Action, BettingStructure, Deal, Event, Hand, PlayError, RaiseRange, Street, TableRules, Variant,
};

fn deal(variant: Variant, holes: &[&str], board: &str) -> Deal {
    let hole_cards = holes.iter().map(|h| Deck::parse(h).unwrap()).collect();
    let board: Vec<Card> = board.split(' ').map(|c| Card::parse(c).unwrap()).collect();
    Deal::new(variant, hole_cards, board.try_into().unwrap()).unwrap()
}

fn nlhe(stacks: &[u64], button: usize, holes: &[&str], board: &str) -> Hand {
    let rules = TableRules::no_limit_holdem(1, 2);
    Hand::new(rules, stacks, button, deal(rules.variant, holes, board)).unwrap()
}

/// Checks or calls until the hand ends.
fn check_down(hand: &mut Hand) {
    while let Some(legal) = hand.legal_actions() {
        let action = if legal.can_check {
            Action::Check
        } else {
            Action::Call
        };
        hand.act(action).unwrap();
    }
}

const BOARD: &str = "2c 7d 9h Jc 3s";

#[test]
fn heads_up_button_posts_small_blind_and_acts_first_preflop() {
    let mut hand = nlhe(&[100, 100], 0, &["As Ah", "Kd Kh"], BOARD);
    assert_eq!(hand.stack(0), 99);
    assert_eq!(hand.stack(1), 98);
    assert_eq!(hand.to_act(), Some(0));
    hand.act(Action::Call).unwrap();
    // The big blind has the option.
    assert_eq!(hand.to_act(), Some(1));
    hand.act(Action::Check).unwrap();
    // Postflop the big blind acts first.
    assert_eq!(hand.street(), Street::Flop);
    assert_eq!(hand.board().len(), 3);
    assert_eq!(hand.to_act(), Some(1));
}

#[test]
fn three_handed_order() {
    let mut hand = nlhe(&[100, 100, 100], 0, &["As Ah", "Kd Kh", "Qc Qd"], BOARD);
    // Button 0, small blind 1, big blind 2: the button acts first preflop.
    assert_eq!(hand.to_act(), Some(0));
    hand.act(Action::Call).unwrap();
    hand.act(Action::Call).unwrap();
    hand.act(Action::Check).unwrap();
    // The small blind acts first postflop.
    assert_eq!(hand.street(), Street::Flop);
    assert_eq!(hand.to_act(), Some(1));
}

#[test]
fn fold_preflop_wins_blinds_without_showdown() {
    let mut hand = nlhe(&[100, 100, 100], 0, &["As Ah", "Kd Kh", "Qc Qd"], BOARD);
    hand.act(Action::Fold).unwrap();
    hand.act(Action::Fold).unwrap();
    let r = hand.result().unwrap();
    assert!(!r.showdown);
    assert_eq!(r.final_stacks, vec![100, 99, 101]);
    assert_eq!(r.net, vec![0, -1, 1]);
    assert_eq!(hand.to_act(), None);
    assert_eq!(hand.act(Action::Check), Err(PlayError::HandComplete));
}

#[test]
fn showdown_best_hand_wins() {
    let mut hand = nlhe(&[100, 100], 0, &["As Ah", "Kd Kh"], BOARD);
    check_down(&mut hand);
    let r = hand.result().unwrap();
    assert!(r.showdown);
    assert_eq!(r.final_stacks, vec![102, 98]);
    assert!(r.pots[0].winning_hand.as_deref().unwrap().contains("Pair"));
    assert_eq!(hand.board().len(), 5);
}

#[test]
fn split_pot_odd_chip_goes_left_of_button() {
    // Both play the board straight; 3-handed with one folded small blind
    // leaves an odd pot of 5.
    let rules = TableRules::no_limit_holdem(1, 2);
    let d = deal(
        rules.variant,
        &["2s 3s", "4d 4h", "2h 3h"],
        "Tc Jd Qh Kc As",
    );
    let mut hand = Hand::new(rules, &[100, 100, 100], 0, d).unwrap();
    hand.act(Action::Call).unwrap(); // button
    hand.act(Action::Fold).unwrap(); // small blind
    hand.act(Action::Check).unwrap(); // big blind
    check_down(&mut hand);
    let r = hand.result().unwrap();
    assert_eq!(r.pots[0].amount, 5);
    // Seats 0 and 2 split. Going left from the button (seat 0), seat 2 comes
    // before seat 0, so it gets the odd chip.
    assert_eq!(r.payouts, vec![2, 0, 3]);
}

#[test]
fn side_pots_with_three_all_ins() {
    // Seat 0 short with the best hand, seat 1 medium second best, seat 2 deep.
    let mut hand = nlhe(&[20, 50, 100], 2, &["As Ah", "Ks Kh", "Qs Qh"], BOARD);
    // Button 2, small blind 0, big blind 1: button acts first.
    assert_eq!(hand.to_act(), Some(2));
    hand.act(Action::AllIn).unwrap(); // 100
    hand.act(Action::AllIn).unwrap(); // 20, a call for less
    hand.act(Action::AllIn).unwrap(); // 50, a call for less
    let r = hand.result().unwrap();
    assert!(r.showdown);
    let pots: Vec<(u64, Vec<usize>)> = r
        .pots
        .iter()
        .map(|p| (p.amount, p.eligible.clone()))
        .collect();
    assert_eq!(
        pots,
        vec![(60, vec![0, 1, 2]), (60, vec![1, 2]), (50, vec![2])]
    );
    // Aces win the main pot, kings the side pot, and seat 2 gets 50 back.
    assert_eq!(r.final_stacks, vec![60, 60, 50]);
    assert_eq!(r.final_stacks.iter().sum::<u64>(), 170);
}

#[test]
fn raise_then_fold_wins_the_pot() {
    let mut hand = nlhe(&[100, 100], 0, &["As Ah", "Kd Kh"], BOARD);
    hand.act(Action::Raise(10)).unwrap();
    hand.act(Action::Fold).unwrap();
    let r = hand.result().unwrap();
    assert_eq!(r.final_stacks, vec![102, 98]);
    // Everyone else folded, so the raiser takes the single pot, own chips included.
    assert_eq!(r.pots.len(), 1);
    assert_eq!(r.pots[0].amount, 12);
    assert_eq!(r.pots[0].eligible, vec![0]);
}

#[test]
fn no_limit_raise_sizes() {
    let mut hand = nlhe(&[100, 100, 100], 0, &["As Ah", "Kd Kh", "Qc Qd"], BOARD);
    let legal = hand.legal_actions().unwrap();
    assert_eq!(
        legal.raise,
        Some(RaiseRange {
            min_to: 4,
            max_to: 100
        })
    );
    assert_eq!(hand.act(Action::Raise(3)), Err(PlayError::IllegalAction));
    assert_eq!(hand.act(Action::Check), Err(PlayError::IllegalAction));
    assert_eq!(hand.act(Action::Bet(10)), Err(PlayError::IllegalAction));
    hand.act(Action::Raise(10)).unwrap();
    // Raised by 8, so the next raise is to at least 18.
    let legal = hand.legal_actions().unwrap();
    assert_eq!(legal.call, Some(9));
    assert_eq!(legal.raise.unwrap().min_to, 18);
    hand.act(Action::Raise(30)).unwrap();
    assert_eq!(hand.legal_actions().unwrap().raise.unwrap().min_to, 50);
}

#[test]
fn postflop_min_bet_is_big_blind() {
    let mut hand = nlhe(&[100, 100], 0, &["As Ah", "Kd Kh"], BOARD);
    hand.act(Action::Call).unwrap();
    hand.act(Action::Check).unwrap();
    let legal = hand.legal_actions().unwrap();
    assert!(legal.can_check && !legal.can_fold);
    assert_eq!(
        legal.bet,
        Some(RaiseRange {
            min_to: 2,
            max_to: 98
        })
    );
    assert_eq!(hand.act(Action::Fold), Err(PlayError::IllegalAction));
    assert_eq!(hand.act(Action::Bet(1)), Err(PlayError::IllegalAction));
    hand.act(Action::Bet(6)).unwrap();
    assert_eq!(hand.legal_actions().unwrap().raise.unwrap().min_to, 12);
}

#[test]
fn short_all_in_does_not_reopen_betting() {
    // Seat 2 (big blind) has 15 behind after posting.
    let mut hand = nlhe(&[100, 100, 17], 0, &["As Ah", "Kd Kh", "Qc Qd"], BOARD);
    hand.act(Action::Raise(10)).unwrap(); // button raises to 10
    hand.act(Action::Call).unwrap(); // small blind calls 10
    hand.act(Action::AllIn).unwrap(); // big blind all-in to 17: +7, less than a full raise of 8
    // The button already acted and faces an incomplete raise: call or fold only.
    let legal = hand.legal_actions().unwrap();
    assert_eq!(legal.seat, 0);
    assert_eq!(legal.call, Some(7));
    assert_eq!(legal.raise, None);
    assert_eq!(hand.act(Action::Raise(40)), Err(PlayError::IllegalAction));
    hand.act(Action::Call).unwrap();
    hand.act(Action::Call).unwrap();
    assert_eq!(hand.street(), Street::Flop);
}

#[test]
fn full_all_in_raise_reopens_betting() {
    let mut hand = nlhe(&[100, 100, 30], 0, &["As Ah", "Kd Kh", "Qc Qd"], BOARD);
    hand.act(Action::Raise(10)).unwrap();
    hand.act(Action::Call).unwrap();
    hand.act(Action::AllIn).unwrap(); // to 30: a full raise of 20
    let legal = hand.legal_actions().unwrap();
    assert_eq!(legal.raise.unwrap().min_to, 50);
}

#[test]
fn pot_limit_maximums() {
    let rules = TableRules::pot_limit_omaha(1, 2);
    let d = deal(
        rules.variant,
        &["As Ah Kd Kh", "Qs Qh Jd Jh", "Ts Th 9d 9h"],
        "2c 7d 9c Jc 3s",
    );
    let mut hand = Hand::new(rules, &[1000, 1000, 1000], 0, d).unwrap();
    // Pot 3, call 2: raise to 2 + 3 + 2 = 7.
    assert_eq!(
        hand.legal_actions().unwrap().raise,
        Some(RaiseRange {
            min_to: 4,
            max_to: 7
        })
    );
    assert_eq!(hand.act(Action::Raise(8)), Err(PlayError::IllegalAction));
    assert_eq!(hand.act(Action::AllIn), Err(PlayError::IllegalAction));
    hand.act(Action::Raise(7)).unwrap();
    // Small blind: pot 10, call 6: raise to 7 + 10 + 6 = 23.
    assert_eq!(hand.legal_actions().unwrap().raise.unwrap().max_to, 23);
    hand.act(Action::Call).unwrap();
    hand.act(Action::Call).unwrap();
    // Flop: pot 21, so a pot-sized bet is 21.
    assert_eq!(hand.street(), Street::Flop);
    assert_eq!(
        hand.legal_actions().unwrap().bet,
        Some(RaiseRange {
            min_to: 2,
            max_to: 21
        })
    );
}

#[test]
fn omaha_uses_exactly_two_hole_cards() {
    // Four hearts on board; seat 0 holds one heart so has no flush, seat 1 two.
    let rules = TableRules::pot_limit_omaha(1, 2);
    let d = deal(
        rules.variant,
        &["Ah Kc Kd Qs", "3h 2h 4c 5d"],
        "6h 9h Th Jh 8c",
    );
    let mut hand = Hand::new(rules, &[100, 100], 0, d).unwrap();
    check_down(&mut hand);
    let r = hand.result().unwrap();
    assert_eq!(r.final_stacks, vec![98, 102]);
}

#[test]
fn plo5_and_holdem_with_pot_limit() {
    let rules = TableRules::pot_limit_omaha(1, 2).with_variant(Variant::Omaha { hole_cards: 5 });
    let d = Deal::random(rules.variant, 6, Some(1)).unwrap();
    assert!(d.hole_cards().iter().all(|h| h.num_cards() == 5));
    let mut hand = Hand::new(rules, &[100; 6], 3, d).unwrap();
    check_down(&mut hand);
    assert_eq!(hand.result().unwrap().final_stacks.iter().sum::<u64>(), 600);

    let rules = TableRules::no_limit_holdem(1, 2).with_structure(BettingStructure::PotLimit);
    let d = Deal::random(rules.variant, 2, Some(2)).unwrap();
    let hand = Hand::new(rules, &[100, 100], 0, d).unwrap();
    assert_eq!(hand.legal_actions().unwrap().raise.unwrap().max_to, 6);
}

#[test]
fn antes_go_in_the_pot() {
    let rules = TableRules::no_limit_holdem(1, 2).with_ante(1);
    let d = Deal::random(rules.variant, 3, Some(3)).unwrap();
    let mut hand = Hand::new(rules, &[100, 100, 100], 0, d).unwrap();
    assert_eq!(hand.pot(), 6);
    // Antes don't count toward the call.
    assert_eq!(hand.legal_actions().unwrap().call, Some(2));
    hand.act(Action::Fold).unwrap();
    hand.act(Action::Fold).unwrap();
    assert_eq!(hand.result().unwrap().final_stacks, vec![99, 98, 103]);
}

#[test]
fn all_in_preflop_runs_out_the_board() {
    let mut hand = nlhe(&[50, 50], 0, &["As Ah", "Kd Kh"], BOARD);
    hand.act(Action::AllIn).unwrap();
    hand.act(Action::Call).unwrap();
    assert!(hand.is_complete());
    let streets: Vec<Street> = hand
        .events()
        .iter()
        .filter_map(|e| match e {
            Event::Board { street, .. } => Some(*street),
            _ => None,
        })
        .collect();
    assert_eq!(streets, vec![Street::Flop, Street::Turn, Street::River]);
    assert_eq!(hand.result().unwrap().final_stacks, vec![100, 0]);
}

#[test]
fn blind_all_in_skips_betting() {
    // The big blind can't cover the blind; the small blind just calls.
    let mut hand = nlhe(&[100, 1], 0, &["As Ah", "Kd Kh"], BOARD);
    assert!(hand.is_all_in(1));
    let legal = hand.legal_actions().unwrap();
    assert_eq!(legal.call, Some(1));
    assert_eq!(legal.raise, None);
    hand.act(Action::Call).unwrap();
    assert!(hand.is_complete());
}

#[test]
fn setup_errors() {
    let rules = TableRules::no_limit_holdem(1, 2);
    let d = || Deal::random(rules.variant, 2, Some(4)).unwrap();
    assert_eq!(
        Hand::new(rules, &[100], 0, d()).err(),
        Some(PlayError::InvalidPlayerCount)
    );
    assert_eq!(
        Hand::new(rules, &[100, 0], 0, d()).err(),
        Some(PlayError::InvalidSetup)
    );
    assert_eq!(
        Hand::new(rules, &[100, 100], 2, d()).err(),
        Some(PlayError::InvalidSetup)
    );
    assert_eq!(
        Hand::new(rules, &[100, 100, 100], 0, d()).err(),
        Some(PlayError::InvalidDeal)
    );
    assert_eq!(
        Hand::new(TableRules::no_limit_holdem(3, 2), &[100, 100], 0, d()).err(),
        Some(PlayError::InvalidBlinds)
    );
    assert_eq!(
        Deal::random(Variant::Omaha { hole_cards: 6 }, 8, None).err(),
        Some(PlayError::NotEnoughCards)
    );
    let dup = Deal::new(
        Variant::Holdem,
        vec![Deck::parse("As Ah").unwrap(), Deck::parse("As Kh").unwrap()],
        ["2c", "3c", "4c", "5c", "6c"].map(|c| Card::parse(c).unwrap()),
    );
    assert_eq!(dup.err(), Some(PlayError::InvalidDeal));
}

#[test]
fn random_deals_are_seeded() {
    let a = Deal::random(Variant::Holdem, 6, Some(9)).unwrap();
    assert_eq!(a, Deal::random(Variant::Holdem, 6, Some(9)).unwrap());
    assert_ne!(a, Deal::random(Variant::Holdem, 6, Some(10)).unwrap());
}

/// Plays many hands with random legal actions and checks that chips are
/// conserved and every hand finishes.
#[test]
fn random_play_conserves_chips() {
    // SplitMix64, so the test needs no extra dependency.
    let mut state = 7u64;
    let mut next = move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    for game in 0..400u64 {
        let rules = match game % 3 {
            0 => TableRules::no_limit_holdem(1, 2),
            1 => TableRules::pot_limit_omaha(1, 2),
            _ => TableRules::pot_limit_omaha(5, 10)
                .with_variant(Variant::Omaha { hole_cards: 6 })
                .with_ante(1),
        };
        let players = 2 + (next() % 5) as usize;
        let stacks: Vec<u64> = (0..players).map(|_| 1 + next() % 300).collect();
        let deal = Deal::random(rules.variant, players, Some(game)).unwrap();
        let mut hand = Hand::new(rules, &stacks, game as usize % players, deal).unwrap();
        let mut steps = 0;
        while let Some(legal) = hand.legal_actions() {
            steps += 1;
            assert!(steps < 200, "hand did not finish");
            let range = legal.bet.or(legal.raise);
            let action = match next() % 6 {
                0 if legal.can_fold => Action::Fold,
                1 | 2 if range.is_some() => {
                    let r = range.unwrap();
                    let to = r.min_to + next() % (r.max_to - r.min_to + 1);
                    if legal.bet.is_some() {
                        Action::Bet(to)
                    } else {
                        Action::Raise(to)
                    }
                }
                3 => Action::AllIn,
                _ if legal.can_check => Action::Check,
                _ => Action::Call,
            };
            match hand.act(action) {
                Ok(()) => {}
                Err(PlayError::IllegalAction) if action == Action::AllIn => {
                    hand.act(if legal.can_check {
                        Action::Check
                    } else {
                        Action::Call
                    })
                    .unwrap();
                }
                Err(e) => panic!("{action:?} rejected: {e}"),
            }
        }
        let r = hand.result().unwrap();
        assert_eq!(
            r.final_stacks.iter().sum::<u64>(),
            stacks.iter().sum::<u64>()
        );
        assert_eq!(r.net.iter().sum::<i64>(), 0);
        assert_eq!(
            r.pots.iter().map(|p| p.amount).sum::<u64>(),
            r.payouts.iter().sum::<u64>()
        );
    }
}
