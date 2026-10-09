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
        // Tables of 4, so every personality plays; the last is filled out
        // from the start of the list, so no one sits alone.
        let all = Personality::ALL;
        let tables: Vec<Vec<Personality>> = (0..all.len())
            .step_by(4)
            .map(|s| (s..s + 4).map(|i| all[i % all.len()]).collect())
            .collect();
        for (t, table) in tables.iter().enumerate() {
            let config = MatchConfig::new(rules, 6, t as u64).duplicate();
            let mut bots: Vec<Box<dyn Bot>> = table
                .iter()
                .enumerate()
                .map(|(i, &p)| Box::new(quick(p, i as u64)) as Box<dyn Bot>)
                .collect();
            let result = run_match(&config, &mut bots).unwrap();
            assert_eq!(result.hands, 6 * table.len());
            assert_eq!(result.net.iter().sum::<i64>(), 0);
            assert_eq!(result.fallbacks, vec![0; table.len()], "{table:?}");
        }
    }
}

#[test]
fn personalities_play_their_styles() {
    let rules = TableRules::no_limit_holdem(1, 2);
    // Full 9-handed table, where the style traits are baselines: the four
    // under test in seats 0-3, Michael Miserable filling the rest.
    let mut bots: Vec<PersonalityBot> = [
        Personality::DougPoker,
        Personality::OldManCoffee,
        Personality::MisterCheating,
        Personality::MilkKing,
    ]
    .into_iter()
    .chain([Personality::MichaelMiserable; 5])
    .enumerate()
    .map(|(i, p)| quick(p, 10 + i as u64))
    .collect();
    let n = bots.len();
    let mut model = OpponentModel::new();
    for h in 0..300 {
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
    assert!(
        vpip(&milk) > 0.4 && vpip(&milk) > vpip(&doug),
        "milk vpip {}",
        vpip(&milk)
    );
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

// --- Individual traits ---

fn exact(variant: Variant, rules: TableRules, holes: &[&str], board: &str) -> Hand {
    let holes = holes.iter().map(|h| Deck::parse(h).unwrap()).collect();
    let board: Vec<ducy::deck::Card> = board
        .split(' ')
        .map(|c| ducy::deck::Card::parse(c).unwrap())
        .collect();
    let deal = Deal::new(variant, holes, board.try_into().unwrap()).unwrap();
    Hand::new(rules, &[1000, 1000], 0, deal).unwrap()
}

fn nlhe(holes: &[&str], board: &str) -> Hand {
    exact(
        Variant::Holdem,
        TableRules::no_limit_holdem(1, 2),
        holes,
        board,
    )
}

/// A bot with a custom style built from a personality, for testing a trait.
fn with(p: Personality, f: impl FnOnce(&mut ducy_play::Style)) -> PersonalityBot {
    let mut style = p.style();
    style.samples = 300;
    f(&mut style);
    PersonalityBot::new(p.name(), style, Some(9))
}

fn summary_with_net(seat: usize, net: i64) -> ducy_play::HandSummary {
    let rules = TableRules::no_limit_holdem(1, 2);
    ducy_play::HandSummary {
        seat,
        rules,
        result: ducy_play::HandResult {
            pots: Vec::new(),
            showdown: false,
            payouts: vec![0, 0],
            final_stacks: vec![0, 0],
            net: if seat == 0 {
                vec![net, -net]
            } else {
                vec![-net, net]
            },
        },
        shown: vec![None, None],
        board: Vec::new(),
        history: Vec::new(),
    }
}

#[test]
fn tilt_and_heater_loosen_play_then_wear_off() {
    let obs = nlhe(&["7c 2d", "8s 3h"], "2c 7d 9h Jc 3s")
        .observation(0)
        .unwrap();

    let mut phil = Personality::PhilBigmouth.bot(Some(1));
    let calm = phil.style_for(&obs);
    phil.hand_over(&summary_with_net(0, -100)); // loses 50 big blinds
    assert!(phil.mood() > 0.8, "{}", phil.mood());
    let tilted = phil.style_for(&obs);
    assert!(tilted.vpip > calm.vpip && tilted.bluff > calm.bluff);
    assert!(tilted.call_factor < calm.call_factor);
    for _ in 0..30 {
        phil.hand_over(&summary_with_net(0, 0));
    }
    assert!(phil.mood() < 0.05);

    // Winning doesn't tilt Phil, but it fires up Chris Moneybags.
    let mut phil = Personality::PhilBigmouth.bot(Some(1));
    phil.hand_over(&summary_with_net(0, 100));
    assert_eq!(phil.mood(), 0.0);
    let mut chris = Personality::ChrisMoneybags.bot(Some(2));
    chris.hand_over(&summary_with_net(0, 100));
    assert!(chris.mood() > 0.8);
    assert!(chris.style_for(&obs).bluff > Personality::ChrisMoneybags.style().bluff);
}

#[test]
fn favorite_and_pretty_hands_get_played() {
    // Seat 0 (button, small blind) acts first heads-up.
    let board = "2c 7d 9h Jc 3s";
    let ten_deuce = nlhe(&["Tc 2d", "8s 3h"], "As 7c 9d Jh 3s")
        .observation(0)
        .unwrap();
    // A test-only style: the balanced default plus "always raise T2".
    let style = ducy_play::Style {
        always_play: "T2",
        samples: 300,
        ..ducy_play::Style::default()
    };
    let mut ten_deuce_fan = PersonalityBot::new("Ten-deuce tester", style, Some(9));
    assert!(matches!(
        ten_deuce_fan.act(&ten_deuce),
        Some(Action::Raise(_))
    ));
    let mut coffee = with(Personality::OldManCoffee, |_| {});
    assert_eq!(coffee.act(&ten_deuce), Some(Action::Fold));

    // In Omaha the same "T2" means any hand holding a ten and a deuce.
    let plo = TableRules::pot_limit_omaha(1, 2);
    let omaha = exact(
        plo.variant,
        plo,
        &["Tc 2d 7h 4s", "8s 3h 5c 6d"],
        "As Kc 9d Jh Qs",
    )
    .observation(0)
    .unwrap();
    assert!(matches!(ten_deuce_fan.act(&omaha), Some(Action::Raise(_))));

    let mut linda = with(Personality::LadyLuckLinda, |_| {});
    let suited_junk = nlhe(&["7s 2s", "8d 3h"], board).observation(0).unwrap();
    assert_eq!(linda.act(&suited_junk), Some(Action::Call));
    let weak_ace = nlhe(&["Ah 3d", "8d 4h"], board).observation(0).unwrap();
    assert_eq!(linda.act(&weak_ace), Some(Action::Call));
    let offsuit_junk = nlhe(&["7c 2d", "8d 3h"], board).observation(0).unwrap();
    assert_eq!(linda.act(&offsuit_junk), Some(Action::Fold));
}

#[test]
fn push_fold_style_only_shoves_or_folds_preflop() {
    let rules = TableRules::no_limit_holdem(1, 2);
    let mut style = Personality::DougPoker.style();
    style.samples = 40;
    style.push_fold_bb = f64::INFINITY;
    style.vpip = 0.14;
    let mut steve = PersonalityBot::new("Shover", style, Some(1));
    let mut doug = quick(Personality::DougPoker, 2);
    let mut shoves = 0;
    for h in 0..60 {
        let deal = Deal::random(rules.variant, 2, Some(h)).unwrap();
        let mut hand = Hand::new(rules, &[200, 200], h as usize % 2, deal).unwrap();
        play_hand(&mut hand, &mut [&mut steve as &mut dyn Bot, &mut doug]).unwrap();
        for event in hand.events() {
            match *event {
                ducy_play::Event::Board { .. } => break,
                ducy_play::Event::Raise {
                    seat: 0, all_in, ..
                }
                | ducy_play::Event::Bet {
                    seat: 0, all_in, ..
                } => {
                    assert!(all_in, "{:?}", hand.events());
                    shoves += 1;
                }
                _ => {}
            }
        }
    }
    assert!(shoves > 3, "{shoves}");
}

/// Plays both seats to the flop with a call and a check.
fn to_flop(hand: &mut Hand) {
    hand.act(Action::Call).unwrap();
    hand.act(Action::Check).unwrap();
    assert_eq!(hand.street(), Street::Flop);
}

#[test]
fn trappers_check_monsters_then_raise() {
    // Seat 1 (big blind) flops quad aces and acts first after the flop.
    let mut hand = nlhe(&["Kc Qd", "As Ah"], "Ad Ac 7h 2s 3d");
    to_flop(&mut hand);
    let mut trapper = with(Personality::DougPoker, |s| s.trap = 1.0);
    let first = hand.observation(1).unwrap();
    assert_eq!(trapper.act(&first), Some(Action::Check));
    hand.act(Action::Check).unwrap();
    hand.act(Action::Bet(4)).unwrap();
    let facing = hand.observation(1).unwrap();
    assert!(matches!(trapper.act(&facing), Some(Action::Raise(_))));

    // Without trapping, an aggressive style just bets it.
    let mut bettor = with(Personality::DougPoker, |s| s.aggression = 1.0);
    assert!(matches!(bettor.act(&first), Some(Action::Bet(_))));
}

#[test]
fn gus_bluffsen_bets_air_and_checks_monsters() {
    let mut daddy = with(Personality::GusBluffsen, |s| {
        s.bluff = 0.0;
        s.aggression = 1.0;
    });
    let mut monster = nlhe(&["Kc Qd", "As Ah"], "Ad Ac 7h 2s 3d");
    to_flop(&mut monster);
    assert_eq!(
        daddy.act(&monster.observation(1).unwrap()),
        Some(Action::Check)
    );

    let mut air = nlhe(&["As Ah", "7c 2d"], "Kd Qc Jh 9s 8d");
    to_flop(&mut air);
    assert!(matches!(
        daddy.act(&air.observation(1).unwrap()),
        Some(Action::Bet(_))
    ));
}

#[test]
fn uncle_gary_will_not_fold_a_pair() {
    // River: seat 1 holds bottom pair and faces a 5x pot overbet.
    let mut hand = nlhe(&["Ac Ad", "3c 2d"], "Kd Qd Jc 9h 3s");
    to_flop(&mut hand);
    for _ in 0..3 {
        hand.act(Action::Check).unwrap();
        if hand.street() == Street::River {
            break;
        }
        hand.act(Action::Check).unwrap();
    }
    assert_eq!(hand.street(), Street::River);
    hand.act(Action::Bet(20)).unwrap();
    let obs = hand.observation(1).unwrap();

    let mut gary = with(Personality::UncleGary, |s| s.caution = 0.0);
    assert_eq!(gary.act(&obs), Some(Action::Call));
    let mut not_gary = with(Personality::UncleGary, |s| {
        s.caution = 0.0;
        s.pair_call_factor = 1.0;
        s.call_factor = 1.0;
    });
    assert_eq!(not_gary.act(&obs), Some(Action::Fold));
}

#[test]
fn a_bot_priced_into_a_pot_calls_instead_of_folding() {
    // Seat 0 has put 800 of its 1,000 chips in; seat 1 shoves. Calling 200
    // to win 2,000 needs 10% equity, and even 72o has more than that against
    // the best hands: the tightest bot calls rather than fold.
    let mut hand = nlhe(&["7c 2d", "As Ah"], "2c 7d 9h Jc 3s");
    hand.act(Action::Raise(6)).unwrap(); // seat 0 opens
    hand.act(Action::Raise(18)).unwrap(); // seat 1 re-raises
    hand.act(Action::Raise(800)).unwrap(); // seat 0 puts most of its stack in
    hand.act(Action::AllIn).unwrap(); // seat 1 shoves
    let committed = hand.observation(0).unwrap();
    for p in [
        Personality::OldManCoffee,
        Personality::BradOwned,
        Personality::DougPoker,
    ] {
        let mut bot = with(p, |_| {});
        assert_eq!(bot.act(&committed), Some(Action::Call), "{}", p.name());
    }

    // At a poor price the same hand still folds: a 6-chip open facing a
    // shove for 1,000 needs about half the pot.
    let mut hand = nlhe(&["7c 2d", "As Ah"], "2c 7d 9h Jc 3s");
    hand.act(Action::Raise(6)).unwrap();
    hand.act(Action::AllIn).unwrap();
    let mut coffee = with(Personality::OldManCoffee, |_| {});
    assert_eq!(
        coffee.act(&hand.observation(0).unwrap()),
        Some(Action::Fold)
    );
}

/// Seat 0 with `hole` after a 100-chip raise and call, checked to the river,
/// where seat 1 bets `bet` into the 200 pot.
fn river_bet(hole: &str, bet: u64) -> ducy_play::Observation {
    let mut hand = nlhe(&[hole, "As Ah"], "2c 7d 9h Jc 3s");
    hand.act(Action::Raise(100)).unwrap();
    hand.act(Action::Call).unwrap();
    for _ in 0..2 {
        hand.act(Action::Check).unwrap();
        hand.act(Action::Check).unwrap();
    }
    hand.act(Action::Bet(bet)).unwrap();
    hand.observation(0).unwrap()
}

#[test]
fn some_prices_are_too_good_to_fold() {
    // Tom Collins is the most cautious caller (call factor 1.3, caution
    // 0.45). 8-high has about 7% on this river, Q-high about 18%.
    let mut tom = with(Personality::TomCollins, |_| {});

    // 20 into 200 (8% pot odds): every bot calls, even with air.
    assert_eq!(tom.act(&river_bet("8h 6d", 20)), Some(Action::Call));
    // 40 into 200 (14%): priced in, so caution can't ask for more than the
    // odds; Q-high's 18% calls (before, it wanted 28%).
    assert_eq!(tom.act(&river_bet("Qh 8d", 40)), Some(Action::Call));
    // A pot-sized bet (33%): it still folds.
    assert_eq!(tom.act(&river_bet("Qh 8d", 200)), Some(Action::Fold));
    assert_eq!(tom.act(&river_bet("8h 6d", 200)), Some(Action::Fold));
}

#[test]
fn tiny_four_bet_range_folds_to_four_bets() {
    let spot = |holes: &[&str]| {
        let mut hand = nlhe(holes, "2c 7d 9h Jc 3s");
        hand.act(Action::Raise(6)).unwrap(); // seat 0 opens
        hand.act(Action::Raise(18)).unwrap(); // seat 1 re-raises
        hand.observation(0).unwrap()
    };
    let mut wizard = with(Personality::DougPoker, |s| s.four_bet = 0.015);
    assert_eq!(wizard.act(&spot(&["Ts 9s", "Kd Kh"])), Some(Action::Fold));
    assert!(matches!(
        wizard.act(&spot(&["As Ah", "Kd Kh"])),
        Some(Action::Raise(_))
    ));
}

#[test]
fn lodge_regulars_play_their_styles() {
    let rules = TableRules::no_limit_holdem(1, 2);
    let lineup = [
        Personality::BradOwned,
        Personality::NikAirbag,
        Personality::Bungleman,
        Personality::MilkKing,
    ];
    let mut bots: Vec<PersonalityBot> = lineup
        .iter()
        .enumerate()
        .map(|(i, &p)| quick(p, 20 + i as u64))
        .collect();
    let n = bots.len();
    let mut model = OpponentModel::new();
    for h in 0..400 {
        let deal = Deal::random(rules.variant, n, Some(500 + h)).unwrap();
        let mut hand = Hand::new(rules, &vec![200; n], h as usize % n, deal).unwrap();
        let mut seated: Vec<&mut dyn Bot> = bots.iter_mut().map(|b| b as &mut dyn Bot).collect();
        play_hand(&mut hand, &mut seated).unwrap();
        model.record(hand.events(), n);
    }
    let rate = |count: u32, total: u32| count as f64 / total.max(1) as f64;
    let [brad, nik, bungle, milk] = [0, 1, 2, 3].map(|s| model.seat(s));
    let pfr = |s: &ducy_play::stats::SeatStats| rate(s.pfr_hands, s.hands);
    let vpip = |s: &ducy_play::stats::SeatStats| rate(s.vpip_hands, s.hands);

    // Nik raises the most, Bungleman a lot, Brad rarely.
    assert!(
        pfr(&nik) > pfr(&bungle) && pfr(&bungle) > pfr(&brad),
        "{nik:?} {bungle:?} {brad:?}"
    );
    assert!(vpip(&nik) > 0.5, "nik vpip {}", vpip(&nik));
    // Brad sees plenty of flops but folds when bet into far more than the whale.
    assert!(vpip(&brad) > 0.2, "brad vpip {}", vpip(&brad));
    assert!(
        rate(brad.folds_to_bets, brad.faced_bets) > rate(milk.folds_to_bets, milk.faced_bets) + 0.2,
        "{brad:?} {milk:?}"
    );
}

#[test]
fn andrew_favorable_opens_up_in_omaha() {
    // The same table in Hold'em and in pot-limit Omaha: Andrew plays far more
    // hands in Omaha, where Doug Poker (no Omaha adjustment) plays about the
    // same share of hands in both.
    let vpips = |rules: TableRules| {
        let lineup = [
            Personality::AndrewFavorable,
            Personality::DougPoker,
            Personality::MichaelMiserable,
            Personality::MilkKing,
        ];
        let mut bots: Vec<PersonalityBot> = lineup
            .iter()
            .enumerate()
            .map(|(i, &p)| quick(p, 80 + i as u64))
            .collect();
        let n = bots.len();
        let mut model = OpponentModel::new();
        for h in 0..400 {
            let deal = Deal::random(rules.variant, n, Some(1700 + h)).unwrap();
            let mut hand = Hand::new(rules, &vec![200; n], h as usize % n, deal).unwrap();
            let mut seated: Vec<&mut dyn Bot> =
                bots.iter_mut().map(|b| b as &mut dyn Bot).collect();
            play_hand(&mut hand, &mut seated).unwrap();
            model.record(hand.events(), n);
        }
        let v = |s: usize| {
            let s = model.seat(s);
            s.vpip_hands as f64 / s.hands.max(1) as f64
        };
        (v(0), v(1))
    };
    let (andrew_hu, doug_hu) = vpips(TableRules::no_limit_holdem(1, 2));
    let (andrew_plo, doug_plo) = vpips(TableRules::pot_limit_omaha(1, 2));
    let (andrew, doug) = (andrew_plo / andrew_hu, doug_plo / doug_hu);
    assert!(
        andrew > 1.25,
        "Andrew in Omaha: {andrew_plo} vs {andrew_hu} in Hold'em"
    );
    assert!(
        andrew > doug + 0.2,
        "Andrew opens up ({andrew}), Doug doesn't ({doug})"
    );
    assert_eq!(Personality::AndrewFavorable.catchphrase(), "Favorable.");
}

#[test]
fn the_mathematician_plays_tight_raises_what_he_plays_and_never_tilts() {
    // Against three loose players: few hands, most of them raised (raise or
    // fold when first in), and he doesn't come apart after losing pots.
    let rules = TableRules::no_limit_holdem(1, 2);
    let lineup = [
        Personality::TheMathematician,
        Personality::MilkKing,
        Personality::NikAirbag,
        Personality::UncleGary,
    ];
    let mut bots: Vec<PersonalityBot> = lineup
        .iter()
        .enumerate()
        .map(|(i, &p)| quick(p, 60 + i as u64))
        .collect();
    let n = bots.len();
    let mut model = OpponentModel::new();
    for h in 0..400 {
        let deal = Deal::random(rules.variant, n, Some(1300 + h)).unwrap();
        let mut hand = Hand::new(rules, &vec![200; n], h as usize % n, deal).unwrap();
        let mut seated: Vec<&mut dyn Bot> = bots.iter_mut().map(|b| b as &mut dyn Bot).collect();
        play_hand(&mut hand, &mut seated).unwrap();
        model.record(hand.events(), n);
    }
    let rate = |count: u32, total: u32| count as f64 / total.max(1) as f64;
    let math = model.seat(0);
    let vpip = rate(math.vpip_hands, math.hands);
    let pfr = rate(math.pfr_hands, math.hands);
    assert!(vpip < 0.3, "the mathematician vpip {vpip}");
    assert!(
        pfr > 0.6 * vpip,
        "raises most of what he plays: vpip {vpip}, pfr {pfr}"
    );
    let style = Personality::TheMathematician.style();
    assert_eq!((style.tilt, style.heater), (0.0, 0.0), "no moods");
    assert_eq!(
        Personality::from_name("The Mathematician"),
        Some(Personality::TheMathematician)
    );
}

#[test]
fn tom_collins_pushes_hard_and_gets_out_of_the_way() {
    // The table captain against the maniac and two calling stations: he
    // raises a lot and bets far more than he calls, but folds to bets far
    // more often than the maniac does.
    let rules = TableRules::no_limit_holdem(1, 2);
    let lineup = [
        Personality::TomCollins,
        Personality::NikAirbag,
        Personality::MilkKing,
        Personality::UncleGary,
    ];
    let mut bots: Vec<PersonalityBot> = lineup
        .iter()
        .enumerate()
        .map(|(i, &p)| quick(p, 40 + i as u64))
        .collect();
    let n = bots.len();
    let mut model = OpponentModel::new();
    for h in 0..400 {
        let deal = Deal::random(rules.variant, n, Some(900 + h)).unwrap();
        let mut hand = Hand::new(rules, &vec![200; n], h as usize % n, deal).unwrap();
        let mut seated: Vec<&mut dyn Bot> = bots.iter_mut().map(|b| b as &mut dyn Bot).collect();
        play_hand(&mut hand, &mut seated).unwrap();
        model.record(hand.events(), n);
    }
    let rate = |count: u32, total: u32| count as f64 / total.max(1) as f64;
    let [tom, nik] = [0, 1].map(|s| model.seat(s));
    let pfr = rate(tom.pfr_hands, tom.hands);
    assert!(pfr > 0.25, "tom pfr {pfr}");
    // Aggressive: bets and raises far more than he calls.
    assert!(rate(tom.aggressive, tom.calls) > 3.0, "{tom:?}");
    // Avoids trouble: folds to bets far more than the maniac.
    let (tom_folds, nik_folds) = (
        rate(tom.folds_to_bets, tom.faced_bets),
        rate(nik.folds_to_bets, nik.faced_bets),
    );
    assert!(
        tom_folds > nik_folds + 0.2,
        "tom folds {tom_folds}, nik {nik_folds}"
    );
    assert_eq!(
        Personality::from_name("tom_collins"),
        Some(Personality::TomCollins)
    );
}

#[test]
fn brad_always_plays_jacks() {
    let jacks = nlhe(&["Jc Jd", "8s 3h"], "2c 7d 9h Qc 3s")
        .observation(0)
        .unwrap();
    let mut brad = with(Personality::BradOwned, |_| {});
    assert!(matches!(brad.act(&jacks), Some(Action::Raise(_))));
}

// --- Table size and position ---

#[test]
fn ranges_scale_with_table_size_and_position() {
    use ducy_play::{position_strength, scale_for_table};
    // 9-handed is the baseline.
    assert!((scale_for_table(0.2, 9) - 0.2).abs() < 1e-12);
    assert!((scale_for_table(0.2, 6) - 0.284).abs() < 0.001);
    assert!((scale_for_table(0.2, 2) - 0.634).abs() < 0.001);
    assert!(scale_for_table(0.2, 10) < 0.2);
    assert_eq!(scale_for_table(0.9, 2), 0.95);
    assert_eq!(scale_for_table(0.0, 6), 0.0);

    // 6-handed, button at seat 0: small blind 1, big blind 2, then 3, 4, 5.
    assert_eq!(position_strength(0, 0, 6), 1.0);
    assert_eq!(position_strength(1, 0, 6), 0.0);
    assert_eq!(position_strength(2, 0, 6), 0.5);
    assert!(position_strength(3, 0, 6) < position_strength(5, 0, 6));
    // Heads-up the button is the small blind and acts last after the flop.
    assert_eq!(position_strength(0, 0, 2), 1.0);
    assert_eq!(position_strength(1, 0, 2), 0.5);
}

/// For `hands` hands of `n` copies of Doug Poker: overall VPIP, and per
/// position (0 = small blind ... n - 1 = button) how often it raised first in
/// when everyone before it had folded.
fn doug_table(n: usize, hands: u64) -> (f64, Vec<f64>) {
    let rules = TableRules::no_limit_holdem(1, 2);
    // Enough samples that hand rankings are steady near the range cutoffs.
    let mut style = Personality::DougPoker.style();
    style.samples = 200;
    let mut bots: Vec<PersonalityBot> = (0..n)
        .map(|i| PersonalityBot::new("Doug", style, Some(50 + i as u64)))
        .collect();
    let mut played = 0u32;
    // (raised first in, had the chance) per position.
    let mut first_in = vec![(0u32, 0u32); n];
    for h in 0..hands {
        let deal = Deal::random(rules.variant, n, Some(h)).unwrap();
        let button = h as usize % n;
        let mut hand = Hand::new(rules, &vec![200; n], button, deal).unwrap();
        let mut seated: Vec<&mut dyn Bot> = bots.iter_mut().map(|b| b as &mut dyn Bot).collect();
        play_hand(&mut hand, &mut seated).unwrap();
        let mut voluntary = vec![false; n];
        let mut opened = false;
        let mut acted = vec![false; n];
        for event in hand.events() {
            use ducy_play::Event::*;
            let (seat, raise) = match *event {
                Board { .. } => break,
                Call { seat, .. } => (seat, false),
                Raise { seat, .. } => (seat, true),
                Fold { seat } | Check { seat } => (seat, false),
                _ => continue,
            };
            if !acted[seat] && !opened {
                // Everyone before folded: a raise-first-in chance.
                let position = (seat + n - button - 1) % n;
                first_in[position].1 += 1;
                first_in[position].0 += u32::from(raise);
            }
            acted[seat] = true;
            if matches!(*event, Call { .. } | Raise { .. }) {
                voluntary[seat] = true;
                opened = true;
            }
        }
        played += voluntary.iter().filter(|&&v| v).count() as u32;
    }
    let rates = first_in
        .iter()
        .map(|&(r, t)| r as f64 / t.max(1) as f64)
        .collect();
    (played as f64 / (hands * n as u64) as f64, rates)
}

#[test]
fn bots_loosen_at_short_tables() {
    let (nine, _) = doug_table(9, 150);
    let (six, _) = doug_table(6, 150);
    let (two, _) = doug_table(2, 300);
    assert!(nine < six && six < two, "9: {nine}, 6: {six}, 2: {two}");
    // A solid regular plays roughly 15% of hands at a full table.
    assert!((0.1..0.25).contains(&nine), "{nine}");
}

#[test]
fn bots_play_wider_in_position() {
    // Raise-first-in rate by position, 6-handed: [SB, BB, UTG, HJ, CO, BTN].
    let (_, rfi) = doug_table(6, 600);
    let (small_blind, early, cutoff, button) = (rfi[0], rfi[2], rfi[4], rfi[5]);
    assert!(button > early * 1.2, "{rfi:?}");
    assert!(cutoff > early, "{rfi:?}");
    assert!(button > small_blind * 1.5, "{rfi:?}");
}
