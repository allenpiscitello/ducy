//! The house's rake and time (`Rake`): taken from the pots before they're
//! paid, and from the stacks once they are, with every chip accounted for.

use ducy::deck::{Card, Deck};
use ducy_play::{
    Action, Awaiting, Deal, Event, Hand, HandResult, PlayError, Rake, Table, TableRules, TableSeat,
    Variant, hands::HandRecord,
};

const BOARD: &str = "2c 7d 9h Jc 3s";

fn deal(holes: &[&str], board: &str) -> Deal {
    let hole_cards = holes.iter().map(|h| Deck::parse(h).unwrap()).collect();
    let board: Vec<Card> = board.split(' ').map(|c| Card::parse(c).unwrap()).collect();
    Deal::new(Variant::Holdem, hole_cards, board.try_into().unwrap()).unwrap()
}

fn rake(percent: u32, cap: u64, no_flop: bool) -> Rake {
    Rake {
        percent,
        cap,
        no_flop,
        time: 0,
    }
}

fn check_down(hand: &mut Hand) {
    while let Some(legal) = hand.legal_actions() {
        hand.act(if legal.can_check {
            Action::Check
        } else {
            Action::Call
        })
        .unwrap();
    }
}

/// Every chip is somewhere: in the stacks or the house's, and each pot's
/// awards are the pot less its rake.
fn adds_up(r: &HandResult, start: &[u64], events: &[Event]) {
    let house = r.house();
    assert_eq!(
        r.final_stacks.iter().sum::<u64>() + house,
        start.iter().sum::<u64>()
    );
    for p in &r.pots {
        assert_eq!(
            p.awards.iter().map(|a| a.amount).sum::<u64>(),
            p.amount - p.rake
        );
    }
    let taken: u64 = events
        .iter()
        .map(|e| match *e {
            Event::Rake { amount, .. } | Event::Time { amount, .. } => amount,
            _ => 0,
        })
        .sum();
    assert_eq!(taken, house);
    assert_eq!(
        r.pots.iter().map(|p| p.rake).sum::<u64>(),
        events
            .iter()
            .map(|e| match *e {
                Event::Rake { amount, .. } => amount,
                _ => 0,
            })
            .sum::<u64>()
    );
    for (i, (&end, &net)) in r.final_stacks.iter().zip(&r.net).enumerate() {
        assert_eq!(end as i64 - start[i] as i64, net);
    }
}

#[test]
fn each_player_pays_their_share_of_what_they_put_in() {
    // 5%: Ann puts in 40 (all-in), Bob and Cy 60 each. The rake is 8: Ann's
    // share 2, Bob's and Cy's 3, whoever wins.
    let rules = TableRules::no_limit_holdem(1, 2).with_rake(rake(500, 0, true));
    let start = [40, 100, 100];
    let d = deal(&["As Ah", "Ks Kh", "Qs Qh"], BOARD);
    let mut hand = Hand::new(rules, &start, 2, d).unwrap();
    hand.act(Action::Raise(60)).unwrap(); // Cy, on the button
    hand.act(Action::AllIn).unwrap(); // Ann, 40 in all
    hand.act(Action::Call).unwrap(); // Bob
    check_down(&mut hand);
    let r = hand.result().unwrap().clone();
    assert_eq!(r.rake, vec![2, 3, 3]);
    assert_eq!(r.house(), 8);
    // Each pot gives by its size: 6 from the main pot of 120, 2 from the
    // side pot of 40.
    let pots: Vec<(u64, u64)> = r.pots.iter().map(|p| (p.amount, p.rake)).collect();
    assert_eq!(pots, vec![(120, 6), (40, 2)]);
    // Aces win the main pot, kings the side pot.
    assert_eq!(r.final_stacks, vec![114, 78, 40]);
    assert!(hand.events().contains(&Event::Rake { pot: 0, amount: 6 }));
    adds_up(&r, &start, hand.events());
}

#[test]
fn whole_chips_rounded_down_and_capped() {
    // $1/$2 in hundredths, $1 chips: 5% of a $40 pot is $2.
    let rules = TableRules::no_limit_holdem(100, 200)
        .with_chip(100)
        .with_rake(rake(500, 300, false));
    let start = [10_000, 10_000];
    let mut hand = Hand::new(rules, &start, 0, deal(&["As Ah", "Kd Kh"], BOARD)).unwrap();
    hand.act(Action::Raise(2000)).unwrap();
    hand.act(Action::Call).unwrap();
    check_down(&mut hand);
    let r = hand.result().unwrap().clone();
    assert_eq!((r.house(), r.rake.clone()), (200, vec![100, 100]));
    adds_up(&r, &start, hand.events());

    // A $100 pot: 5% is $5, capped at $3.
    let mut hand = Hand::new(rules, &start, 0, deal(&["As Ah", "Kd Kh"], BOARD)).unwrap();
    hand.act(Action::Raise(5000)).unwrap();
    hand.act(Action::Call).unwrap();
    check_down(&mut hand);
    let r = hand.result().unwrap().clone();
    assert_eq!(r.house(), 300);
    assert_eq!(r.final_stacks, vec![14_700, 5000]);
    adds_up(&r, &start, hand.events());

    // An $8 pot: 5% is 40 hundredths, less than a chip: nothing.
    let mut hand = Hand::new(rules, &start, 0, deal(&["As Ah", "Kd Kh"], BOARD)).unwrap();
    hand.act(Action::Raise(400)).unwrap();
    hand.act(Action::Call).unwrap();
    check_down(&mut hand);
    let r = hand.result().unwrap();
    assert!(r.rake.is_empty());
    assert!(
        !hand
            .events()
            .iter()
            .any(|e| matches!(e, Event::Rake { .. }))
    );
}

#[test]
fn no_flop_no_drop_and_uncalled_chips_are_not_raked() {
    let three = |r: Rake| {
        let rules = TableRules::no_limit_holdem(1, 2).with_rake(r);
        let d = deal(&["As Ah", "Kd Kh", "Qc Qd"], BOARD);
        let mut hand = Hand::new(rules, &[100, 100, 100], 0, d).unwrap();
        hand.act(Action::Raise(10)).unwrap(); // the button
        hand.act(Action::Fold).unwrap();
        hand.act(Action::Fold).unwrap();
        hand
    };
    // No flop, no drop.
    let hand = three(rake(2000, 0, true));
    let r = hand.result().unwrap();
    assert!(r.rake.is_empty());
    assert_eq!(r.final_stacks, vec![103, 99, 98]);
    // Raked before the flop too: 20% of the 5 called (the blinds and 2 of
    // the raise; the other 8 went back uncalled).
    let hand = three(rake(2000, 0, false));
    let r = hand.result().unwrap().clone();
    assert_eq!(r.house(), 1);
    assert_eq!(r.final_stacks, vec![102, 99, 98]);
    adds_up(&r, &[100, 100, 100], hand.events());
}

#[test]
fn running_it_twice_splits_what_is_left() {
    let rules = TableRules::no_limit_holdem(1, 2).with_rake(rake(1000, 0, true));
    let rest: Vec<Card> = ["Ad", "Kc", "8s", "8h", "4d"]
        .iter()
        .map(|c| Card::parse(c).unwrap())
        .collect();
    let d = deal(&["As Ah", "Kd Kh"], BOARD).with_rest(rest).unwrap();
    let start = [100, 100];
    let mut hand = Hand::new(rules, &start, 0, d).unwrap();
    hand.offer_run_twice(true);
    hand.act(Action::AllIn).unwrap();
    hand.act(Action::Call).unwrap();
    assert!(matches!(hand.awaiting(), Some(Awaiting::RunChoice(_))));
    hand.choose_runs(2).unwrap();
    let r = hand.result().unwrap().clone();
    // 10% of 200: the 180 left goes 90 to each board.
    assert_eq!(r.house(), 20);
    assert_eq!(r.pots[0].awards.iter().map(|a| a.amount).sum::<u64>(), 180);
    adds_up(&r, &start, hand.events());
}

#[test]
fn a_double_board_bomb_pot_is_raked_once() {
    let rules = TableRules::pot_limit_omaha(1, 2)
        .bomb_pot(10, true)
        .with_rake(rake(500, 0, true));
    let start = [200, 200, 200];
    let d = Deal::random(rules.variant, 3, Some(3)).unwrap();
    let mut hand = Hand::new(rules, &start, 0, d).unwrap();
    check_down(&mut hand);
    let r = hand.result().unwrap().clone();
    // A bomb pot starts on the flop: 5% of 30.
    assert_eq!(r.house(), 1);
    adds_up(&r, &start, hand.events());
}

#[test]
fn time_is_paid_from_each_stack_once_the_hand_is_over() {
    let time = Rake {
        time: 3,
        ..Rake::default()
    };
    let rules = TableRules::no_limit_holdem(1, 2).with_rake(time);
    let start = [50, 100];
    let mut hand = Hand::new(rules, &start, 0, deal(&["Kd Kh", "As Ah"], BOARD)).unwrap();
    hand.act(Action::AllIn).unwrap();
    hand.act(Action::Call).unwrap();
    let r = hand.result().unwrap().clone();
    // The loser has nothing left to pay with.
    assert_eq!(r.final_stacks, vec![0, 147]);
    assert_eq!(r.rake, vec![0, 3]);
    assert_eq!(
        hand.events().last(),
        Some(&Event::Time { seat: 1, amount: 3 })
    );
    adds_up(&r, &start, hand.events());
}

#[test]
fn a_rake_over_the_whole_pot_is_refused() {
    let rules = TableRules::no_limit_holdem(1, 2).with_rake(rake(10_001, 0, false));
    let d = Deal::random(rules.variant, 2, Some(1)).unwrap();
    assert_eq!(
        Hand::new(rules, &[100, 100], 0, d).err(),
        Some(PlayError::InvalidSetup)
    );
}

#[test]
fn a_raked_hand_saves_and_restores() {
    let rules = TableRules::no_limit_holdem(1, 2).with_rake(Rake {
        percent: 500,
        cap: 4,
        no_flop: true,
        time: 1,
    });
    let mut hand = Hand::new(rules, &[100, 100], 0, deal(&["As Ah", "Kd Kh"], BOARD)).unwrap();
    hand.act(Action::Raise(40)).unwrap();
    hand.act(Action::Call).unwrap();
    check_down(&mut hand);
    let saved = hand.snapshot();
    let back = Hand::restore(&saved).unwrap();
    assert_eq!(back.result(), hand.result());
    assert_eq!(back.events(), hand.events());
    let json = serde_json::to_string(&saved).unwrap();
    assert!(json.contains("\"rake\""));
    // Without one, the rules save as before.
    let plain = serde_json::to_string(&TableRules::no_limit_holdem(1, 2)).unwrap();
    assert!(!plain.contains("rake"));
}

#[test]
fn a_table_shows_the_rake() {
    let rules = TableRules::no_limit_holdem(1, 2).with_rake(rake(1000, 0, false));
    let seats = vec![
        TableSeat::human("Ann", "ann"),
        TableSeat::human("Bob", "bob"),
    ];
    let mut t = Table::new(rules, seats, 100, 5).unwrap();
    let play_out = |t: &mut Table| {
        while let (Some(seat), Some(legal)) = (t.to_act(), t.hand().and_then(|h| h.legal_actions()))
        {
            let a = if legal.can_check {
                Action::Check
            } else {
                Action::Call
            };
            t.act(seat, a).unwrap();
        }
    };
    t.new_hand().unwrap();
    play_out(&mut t);
    let v = t.view(0);
    assert!(v.complete);
    assert_eq!(v.rake, 0, "4 × 10% is less than a chip");
    // A bigger pot.
    t.new_hand().unwrap();
    let seat = t.to_act().unwrap();
    t.act(seat, Action::Raise(20)).unwrap();
    let seat = t.to_act().unwrap();
    t.act(seat, Action::Call).unwrap();
    play_out(&mut t);
    let v = t.view(1);
    assert_eq!(v.rake, 4);
    assert_eq!(v.seats.iter().map(|s| s.rake).sum::<u64>(), 4);
    assert_eq!(v.pots[0].rake, 4);
    assert_eq!(v.seats.iter().map(|s| s.stack).sum::<u64>(), 196);

    // Its record and replay show it too.
    let rec = HandRecord::new(&v, Default::default(), 9);
    assert_eq!(rec.rake, 4);
    assert_eq!(rec.seats.iter().map(|s| s.rake).sum::<u64>(), 4);
    let frames = rec.frames();
    let rake_frame = frames.iter().find(|f| f.text == "Rake 4").unwrap();
    assert_eq!(rake_frame.pot, 36);
    let last = frames.last().unwrap();
    assert_eq!(last.seats.iter().map(|s| s.stack).sum::<u64>(), 196);
    assert_eq!(last.pot, 0);
}

/// Random play with every kind of rake: every chip is still somewhere.
#[test]
fn random_play_with_rake_conserves_chips() {
    let mut state = 11u64;
    let mut next = move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    for game in 0..400u64 {
        let base = match game % 3 {
            0 => TableRules::no_limit_holdem(1, 2),
            1 => TableRules::pot_limit_omaha(5, 10).with_chip(5),
            _ => TableRules::pot_limit_omaha(1, 2).bomb_pot(2, true),
        };
        let r = Rake {
            percent: (next() % 1001) as u32,
            cap: next() % 3 * 7,
            no_flop: next() % 2 == 0,
            time: next() % 3,
        };
        let rules = base.with_rake(r);
        let players = 2 + (next() % 5) as usize;
        let start: Vec<u64> = (0..players).map(|_| 3 + next() % 300).collect();
        let deal = Deal::random(rules.variant, players, Some(game)).unwrap();
        let mut hand = Hand::new(rules, &start, game as usize % players, deal).unwrap();
        hand.offer_run_twice(next() % 2 == 0);
        let mut steps = 0;
        while !hand.is_complete() {
            steps += 1;
            assert!(steps < 200, "hand did not finish");
            if let Some(Awaiting::RunChoice(_)) = hand.awaiting() {
                hand.choose_runs(1 + (next() % 2) as u8).unwrap();
                continue;
            }
            let legal = hand.legal_actions().unwrap();
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
                _ if legal.can_check => Action::Check,
                _ => Action::Call,
            };
            hand.act(action).unwrap();
        }
        let res = hand.result().unwrap().clone();
        adds_up(&res, &start, hand.events());
        let pots: u64 = res.pots.iter().map(|p| p.rake).sum();
        assert!(r.cap == 0 || pots <= r.cap);
    }
}
