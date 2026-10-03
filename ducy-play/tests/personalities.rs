use ducy::deck::Deck;
use ducy_play::bots::CallingStation;
use ducy_play::stats::OpponentModel;
use ducy_play::strength::preflop_percentile;
use ducy_play::{
    Action, Bot, Deal, Hand, MatchConfig, Observation, Personality, PersonalityBot, Street,
    TableRules, Variant, play_hand, run_match,
};
use rand::{SeedableRng, rngs::StdRng};

/// A personality with fewer Monte Carlo samples, to keep tests quick.
fn quick(p: Personality, seed: u64) -> PersonalityBot {
    let mut style = p.style();
    style.samples = 40;
    PersonalityBot::new(p.name(), style, Some(seed))
}

/// Folds to any bet after the flop; otherwise checks or calls.
struct Folder;

impl Bot for Folder {
    fn act(&mut self, obs: &Observation) -> Option<Action> {
        Some(match obs.legal.call {
            Some(_) if obs.street != Street::Preflop => Action::Fold,
            Some(_) => Action::Call,
            None => Action::Check,
        })
    }
}

#[test]
fn names_and_lookup() {
    for p in Personality::ALL {
        assert_eq!(Personality::from_name(p.id()), Some(p));
        assert_eq!(Personality::from_name(&p.name().to_uppercase()), Some(p));
        assert!(!p.description().is_empty());
        assert_eq!(p.bot(None).name(), p.name());
    }
    assert_eq!(
        Personality::from_name("Doug Poker"),
        Some(Personality::DougPoker)
    );
    assert_eq!(Personality::from_name("nobody"), None);
}

#[test]
fn preflop_percentiles_rank_hands() {
    let mut rng = StdRng::seed_from_u64(1);
    let mut pct = |variant, hand: &str| {
        preflop_percentile(variant, Deck::parse(hand).unwrap(), 400, &mut rng)
    };
    assert!(pct(Variant::Holdem, "As Ah") > 0.95);
    assert!(pct(Variant::Holdem, "Ks Qs") > 0.8);
    assert!(pct(Variant::Holdem, "7c 2d") < 0.15);
    let plo = Variant::Omaha { hole_cards: 4 };
    assert!(pct(plo, "As Ah Ks Kh") > 0.9);
    assert!(pct(plo, "2c 7d 3h 8s") < 0.3);
}

#[test]
fn personalities_play_legal_full_matches() {
    for rules in [
        TableRules::no_limit_holdem(1, 2),
        TableRules::pot_limit_omaha(1, 2).with_ante(1),
    ] {
        let config = MatchConfig::new(rules, 15, 4).duplicate();
        let mut bots: Vec<Box<dyn Bot>> = Personality::ALL
            .iter()
            .enumerate()
            .map(|(i, &p)| Box::new(quick(p, i as u64)) as Box<dyn Bot>)
            .collect();
        let result = run_match(&config, &mut bots).unwrap();
        assert_eq!(result.hands, 60);
        assert_eq!(result.net.iter().sum::<i64>(), 0);
        assert_eq!(result.fallbacks, vec![0; 4]);
    }
}

#[test]
fn personalities_play_their_styles() {
    let rules = TableRules::no_limit_holdem(1, 2);
    let mut bots: Vec<PersonalityBot> = Personality::ALL
        .iter()
        .enumerate()
        .map(|(i, &p)| quick(p, 10 + i as u64))
        .collect();
    let n = bots.len();
    let mut model = OpponentModel::new();
    for h in 0..400 {
        let deal = Deal::random(rules.variant, n, Some(h)).unwrap();
        let mut hand = Hand::new(rules, &vec![200; n], h as usize % n, deal).unwrap();
        let mut seated: Vec<&mut dyn Bot> = bots.iter_mut().map(|b| b as &mut dyn Bot).collect();
        play_hand(&mut hand, &mut seated).unwrap();
        model.record(hand.events(), n);
    }
    let rate = |count: u32, total: u32| count as f64 / total.max(1) as f64;
    let [doug, coffee, cheating, milk] = [0, 1, 2, 3].map(|s| model.seat(s));
    let vpip = |s: &ducy_play::stats::SeatStats| rate(s.vpip_hands, s.hands);
    let pfr = |s: &ducy_play::stats::SeatStats| rate(s.pfr_hands, s.hands);

    // Hands played: nit < balanced < loose-aggressive, and Milk King plays most.
    assert!(vpip(&coffee) < 0.15, "coffee vpip {}", vpip(&coffee));
    assert!(vpip(&coffee) < vpip(&doug) && vpip(&doug) < vpip(&cheating));
    assert!(vpip(&milk) > 0.5, "milk vpip {}", vpip(&milk));
    // Raising: Mister Cheating most, Milk King almost never.
    assert!(pfr(&cheating) > pfr(&doug) && pfr(&doug) > pfr(&milk));
    assert!(pfr(&milk) < 0.06, "milk pfr {}", pfr(&milk));
    // Milk King is far more passive and calls far more than anyone.
    assert!(milk.aggressive * 3 < milk.calls);
    assert!(
        rate(milk.folds_to_bets, milk.faced_bets) < rate(coffee.folds_to_bets, coffee.faced_bets)
    );
}

/// Plays `hands` heads-up hands between Mister Cheating (seat 0) and `villain`.
fn train(cheater: &mut PersonalityBot, villain: &mut dyn Bot, hands: u64) {
    let rules = TableRules::no_limit_holdem(1, 2);
    for h in 0..hands {
        let deal = Deal::random(rules.variant, 2, Some(1000 + h)).unwrap();
        let mut hand = Hand::new(rules, &[200, 200], h as usize % 2, deal).unwrap();
        play_hand(&mut hand, &mut [cheater as &mut dyn Bot, villain]).unwrap();
    }
}

#[test]
fn mister_cheating_exploits_opponents() {
    let rules = TableRules::no_limit_holdem(1, 2);
    let spot = || {
        let deal = Deal::random(rules.variant, 2, Some(7)).unwrap();
        Hand::new(rules, &[200, 200], 0, deal).unwrap()
    };
    let base = Personality::MisterCheating.style();

    // Before it has seen anything it plays its normal style.
    let fresh = quick(Personality::MisterCheating, 1);
    let obs = spot().observation(0).unwrap();
    assert_eq!(fresh.style_for(&obs).bluff, base.bluff);

    // Against someone who folds to every bet it bluffs more.
    let mut cheater = quick(Personality::MisterCheating, 2);
    train(&mut cheater, &mut Folder, 60);
    let s = cheater.model().seat(1);
    assert!(s.fold_to_bet() > 0.6, "{s:?}");
    let style = cheater.style_for(&spot().observation(0).unwrap());
    assert!(style.bluff > base.bluff, "{style:?}");

    // Against a calling station it stops bluffing and bets bigger.
    let mut cheater = quick(Personality::MisterCheating, 3);
    train(&mut cheater, &mut CallingStation, 60);
    let style = cheater.style_for(&spot().observation(0).unwrap());
    assert_eq!(style.bluff, 0.0);
    assert!(style.bet_size > base.bet_size);

    // Non-exploiting personalities never change their style.
    let mut doug = quick(Personality::DougPoker, 4);
    let mut station = CallingStation;
    for h in 0..20 {
        let deal = Deal::random(rules.variant, 2, Some(h)).unwrap();
        let mut hand = Hand::new(rules, &[200, 200], 0, deal).unwrap();
        play_hand(&mut hand, &mut [&mut doug as &mut dyn Bot, &mut station]).unwrap();
    }
    let doug_style = doug.style_for(&spot().observation(0).unwrap());
    assert_eq!(doug_style.bluff, Personality::DougPoker.style().bluff);
}
