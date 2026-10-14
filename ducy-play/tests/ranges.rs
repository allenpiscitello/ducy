//! Opponents' ranges from their actions, and equity against them (#205).

use ducy::deck::{Card, Deck};
use ducy_play::ranges::{BET_SHARE, has_nuts, made_score, preflop_top, range_equity, reads};
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

#[test]
fn the_nuts_are_recognised() {
    let board = cards("Ah Kh Qh 4c 2d");
    // The royal flush is the nuts; trip aces aren't.
    assert!(has_nuts(Variant::Holdem, &cards("Jh Th"), &board));
    assert!(!has_nuts(Variant::Holdem, &cards("As Ad"), &board));
    // On a board with no flush or straight possible, top set is.
    let dry = cards("Ks 8d 3c 2h 7s");
    assert!(has_nuts(Variant::Holdem, &cards("Kd Kh"), &dry));
    assert!(!has_nuts(Variant::Holdem, &cards("8s 8c"), &dry));
    // Omaha: exactly two hole cards, so one heart with a four-heart board
    // isn't a flush, and the nut straight needs two connecting cards.
    let plo = Variant::Omaha { hole_cards: 4 };
    let four_hearts = cards("2h 5h 9h Kh 3c");
    assert!(has_nuts(plo, &cards("Ah Qh 7s 7d"), &four_hearts));
    assert!(!has_nuts(plo, &cards("Ah Ac Ad 7s"), &four_hearts));
    // Not before the flop.
    assert!(!has_nuts(Variant::Holdem, &cards("As Ah"), &[]));
}

#[test]
fn checked_to_last_on_the_river_with_the_nuts_it_bets() {
    use ducy_play::{Bot, Personality, PersonalityBot};
    // The button (seat 0, last to act after the flop) holds the nut flush;
    // the big blind checks every street. Checking back would end the hand.
    for p in [Personality::JonnySlow, Personality::MilkKing] {
        let mut h = hand(&["Ah Th", "8c 3d"], "Kh 7h 2h 9c 4s");
        h.act(Action::Call).unwrap();
        h.act(Action::Check).unwrap();
        for _ in 0..2 {
            h.act(Action::Check).unwrap();
            h.act(Action::Check).unwrap();
        }
        h.act(Action::Check).unwrap();
        let obs = h.observation(0).unwrap();
        assert_eq!(obs.street, Street::River);
        let mut style = p.style();
        style.samples = 40;
        let mut bot = PersonalityBot::new(p.name(), style, Some(1));
        assert!(
            matches!(bot.act(&obs), Some(Action::Bet(_))),
            "{} checked the nuts down",
            p.name()
        );
    }
}

#[test]
fn the_river_bets_strong_hands_and_air_and_checks_the_middle() {
    use ducy_play::{Bot, PersonalityBot, Style};
    // Checked to on the river, last to act, on Kh 7d 2c 9c 4s: a style that
    // bets every hand it would bet or bluff with. A set of kings bets for
    // value and six-high bluffs, but second pair has showdown value: betting
    // it gets called by better and folds out worse, so it checks. Without
    // the polarized river it bets second pair too.
    let bets = |hole: &str, polar: bool| {
        let mut h = hand(&[hole, "8c 3d"], "Kh 7d 2c 9c 4s");
        h.act(Action::Call).unwrap();
        h.act(Action::Check).unwrap();
        for _ in 0..2 {
            h.act(Action::Check).unwrap();
            h.act(Action::Check).unwrap();
        }
        h.act(Action::Check).unwrap();
        let obs = h.observation(0).unwrap();
        assert_eq!(obs.street, Street::River);
        let style = Style {
            aggression: 1.0,
            bluff: 1.0,
            ranges: false,
            samples: 300,
            polar_river: polar,
            ..Style::default()
        };
        (0..10)
            .filter(|&s| {
                let mut bot = PersonalityBot::new("river", style, Some(s));
                matches!(bot.act(&obs), Some(Action::Bet(_)))
            })
            .count()
    };
    assert_eq!(bets("Ks Kc", true), 10, "a set of kings checked");
    assert_eq!(bets("6h 3h", true), 10, "six-high didn't bluff");
    assert_eq!(bets("9h 8h", true), 0, "second pair bet");
    assert_eq!(
        bets("9h 8h", false),
        10,
        "second pair checked without the polarized river"
    );
}

#[test]
fn heads_up_with_the_nuts_it_raises_instead_of_calling() {
    use ducy_play::{Bot, Personality, PersonalityBot};
    // Jonny Slow, the most passive caller, holds the nut flush on the river
    // and faces a bet: he raises.
    for p in [
        Personality::JonnySlow,
        Personality::MilkKing,
        Personality::DougPoker,
    ] {
        let mut h = hand(&["8c 3d", "Ah Th"], "Kh 7h 2h 9c 4s");
        h.act(Action::Call).unwrap();
        h.act(Action::Check).unwrap();
        for _ in 0..2 {
            h.act(Action::Check).unwrap();
            h.act(Action::Check).unwrap();
        }
        h.act(Action::Check).unwrap();
        h.act(Action::Bet(4)).unwrap();
        let obs = h.observation(1).unwrap();
        assert_eq!(obs.street, Street::River);
        let mut style = p.style();
        style.samples = 40;
        let mut bot = PersonalityBot::new(p.name(), style, Some(1));
        assert!(
            matches!(bot.act(&obs), Some(Action::Raise(_))),
            "{} just called the nuts",
            p.name()
        );
    }
}
