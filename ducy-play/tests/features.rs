//! Decision features (#214): every value in range in real games of Hold'em,
//! PLO4 and PLO6, and the card features right in hand-picked spots.

use ducy::deck::{Card, Deck};
use ducy_play::{
    Action, Bot, Deal, Hand, HandSummary, MatchConfig, Observation, TableRules, Variant,
    features::*,
    personality::{Personality, PersonalityBot},
    run_match,
    stats::OpponentModel,
};
use rand::{SeedableRng, rngs::StdRng};

/// Plays as a personality, and checks the features of every decision.
struct Checker {
    bot: PersonalityBot,
    model: OpponentModel,
    rng: StdRng,
    /// Decisions seen per street.
    streets: [u32; 4],
}

impl Bot for Checker {
    fn act(&mut self, obs: &Observation) -> Option<Action> {
        let style = *self.bot.style();
        let config = FeatureConfig { samples: 30 };
        let f = decision_features(obs, &self.model, &style, &config, &mut self.rng);
        for (i, x) in f.iter().enumerate() {
            assert!(
                x.is_finite() && (-1e-6..=1.0 + 1e-6).contains(x),
                "feature {i} is {x} in {obs:?}"
            );
        }
        let street = f[STREET..STREET + 4]
            .iter()
            .position(|&x| x == 1.0)
            .expect("one street");
        assert_eq!(f[STREET..STREET + 4].iter().sum::<f32>(), 1.0);
        self.streets[street] += 1;
        if street > 0 {
            assert!(f[STRENGTH] > 0.0 || f[CATEGORY] == 0.0);
        }
        self.bot.act(obs)
    }

    fn hand_over(&mut self, summary: &HandSummary) {
        self.model
            .record(&summary.history, summary.result.net.len());
        self.bot.hand_over(summary);
    }
}

/// Plays `deals` six-handed deals of `rules`.
fn check_game(rules: TableRules, deals: usize) {
    let mut bots: Vec<Box<dyn Bot>> = Personality::ALL[..6]
        .iter()
        .enumerate()
        .map(|(i, p)| {
            Box::new(Checker {
                bot: p.bot(Some(i as u64)),
                model: OpponentModel::new(),
                rng: StdRng::seed_from_u64(i as u64),
                streets: [0; 4],
            }) as Box<dyn Bot>
        })
        .collect();
    run_match(&MatchConfig::new(rules, deals, 7), &mut bots).expect("match");
}

#[test]
fn every_feature_is_in_range_in_holdem() {
    check_game(TableRules::no_limit_holdem(1, 2), 40);
}

// Omaha's exact strength scores every two-card holding with every board
// triple: slow in a debug build, so fewer deals.
#[test]
fn every_feature_is_in_range_in_omaha() {
    check_game(TableRules::pot_limit_omaha(1, 2), 8);
    check_game(
        TableRules::pot_limit_omaha(1, 2).with_variant(Variant::Omaha { hole_cards: 6 }),
        8,
    );
}

fn cards(s: &str) -> Vec<Card> {
    s.split(' ').map(|c| c.parse().unwrap()).collect()
}

fn deck(s: &str) -> Deck {
    let mut d = Deck::empty();
    for c in cards(s) {
        d |= c;
    }
    d
}

/// The features of the first decision on the flop, heads-up, for the player
/// holding `mine` against `theirs`, after a limp and a check.
fn on_flop(rules: TableRules, mine: &str, theirs: &str, board: &str) -> [f32; NUM_FEATURES] {
    let b = cards(board);
    let deal = Deal::new(
        rules.variant,
        vec![deck(mine), deck(theirs)],
        [b[0], b[1], b[2], b[3], b[4]],
    )
    .unwrap();
    // Seat 1 has the button: seat 0 acts first on the flop.
    let mut hand = Hand::new(rules, &[200, 200], 1, deal).unwrap();
    hand.act(Action::Call).unwrap();
    hand.act(Action::Check).unwrap();
    let obs = hand.observation(0).expect("seat 0 to act on the flop");
    decision_features(
        &obs,
        &OpponentModel::new(),
        &PersonalityBot::new("t", Default::default(), None)
            .style()
            .clone(),
        &FeatureConfig { samples: 0 },
        &mut StdRng::seed_from_u64(1),
    )
}

#[test]
fn holdem_card_features() {
    let he = TableRules::no_limit_holdem(1, 2);
    // A royal flush on the flop: the nuts.
    let f = on_flop(he, "Js Ts", "2c 3d", "As Ks Qs 4h 5h");
    assert_eq!((f[IS_NUTS], f[NUTNESS], f[CATEGORY]), (1.0, 1.0, 1.0));
    assert_eq!(f[STREET + 1], 1.0);
    assert_eq!(f[POSITION], 0.0, "first to act");
    assert_eq!(f[BOARD_FLUSH], 1.0);
    // The nut flush draw with two overcards.
    let f = on_flop(he, "Ah Kh", "2c 3d", "Qh 7h 2s 9c 4d");
    assert_eq!(
        (f[FLUSH_DRAW], f[NUT_FLUSH_DRAW], f[CATEGORY]),
        (1.0, 1.0, 0.0)
    );
    assert!(
        f[IMPROVE_OUTS] > 0.3,
        "flush cards and overcards improve it: {}",
        f[IMPROVE_OUTS]
    );
    assert_eq!(f[BOARD_TWO_TONE], 1.0);
    // A weaker flush draw, and an open-ended straight draw.
    let f = on_flop(he, "8h 6h", "2c 3d", "Qh 7h 5s 9c 4d");
    assert_eq!((f[FLUSH_DRAW], f[NUT_FLUSH_DRAW]), (1.0, 0.0));
    assert!(
        f[STRONG_OUTS] > 0.3,
        "flush and straight outs: {}",
        f[STRONG_OUTS]
    );
    // A set on a straighty board: strong, but J8 and 86 beat it.
    let f = on_flop(he, "7c 7d", "2c 3d", "7s 9h Tc Kc 4d");
    assert_eq!(f[CATEGORY], 3.0 / 8.0);
    assert!(f[STRENGTH] > 0.95 && f[IS_NUTS] == 0.0);
    assert_eq!((f[HOLE_PAIR], f[BOARD_STRAIGHT]), (1.0, 1.0));
}

#[test]
fn omaha_uses_exactly_two_hole_cards() {
    let plo = TableRules::pot_limit_omaha(1, 2);
    // One heart in the hand: no flush and no flush draw in Omaha, though
    // Hold'em's rules would see a made flush.
    let f = on_flop(plo, "Ah 2c 3d 4s", "5c 6d 7s 8c", "Kh Qh Jh 9d 9s");
    assert!(f[CATEGORY] < 5.0 / 8.0);
    assert_eq!(f[FLUSH_DRAW], 0.0);
    // Two hearts: the nut flush draw.
    let f = on_flop(plo, "Ah 2h 3d 4s", "5c 6d 7s 8c", "Kh Qh 7c 9d 9s");
    assert_eq!((f[FLUSH_DRAW], f[NUT_FLUSH_DRAW]), (1.0, 1.0));
    assert_eq!((f[OMAHA], f[POT_LIMIT], f[HOLE_CARDS]), (1.0, 1.0, 0.5));
}

#[test]
fn the_style_is_in_the_features() {
    let a = style_features(&Personality::NikAirbag.style());
    let b = style_features(&Personality::DougPoker.style());
    assert_ne!(a, b);
    assert_eq!(a.len(), STYLE.end - STYLE.start);
}
