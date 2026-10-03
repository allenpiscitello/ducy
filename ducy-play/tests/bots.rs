use std::time::Duration;

use ducy::deck::{Card, Deck};
use ducy_play::bots::{CallingStation, EquityBot, Raiser, RandomBot};
use ducy_play::{
    Action, Bot, Deal, Hand, HandSummary, MatchConfig, Observation, ProcessBot, TableRules,
    play_hand, run_match,
};

fn exact_hand() -> Hand {
    let rules = TableRules::no_limit_holdem(1, 2);
    let holes = ["As Ah", "Kd Kh", "Qc Qd"]
        .iter()
        .map(|h| Deck::parse(h).unwrap())
        .collect();
    let board = ["2c", "7d", "9h", "Jc", "3s"].map(|c| Card::parse(c).unwrap());
    let deal = Deal::new(rules.variant, holes, board).unwrap();
    Hand::new(rules, &[100, 100, 100], 0, deal).unwrap()
}

/// Records what it was shown, then checks or calls.
#[derive(Default)]
struct Recorder {
    observations: Vec<Observation>,
    summaries: Vec<HandSummary>,
}

impl Bot for Recorder {
    fn act(&mut self, obs: &Observation) -> Option<Action> {
        self.observations.push(obs.clone());
        CallingStation.act(obs)
    }

    fn hand_over(&mut self, summary: &HandSummary) {
        self.summaries.push(summary.clone());
    }
}

/// Never answers, or answers with something illegal.
struct Broken {
    illegal: bool,
}

impl Bot for Broken {
    fn act(&mut self, _: &Observation) -> Option<Action> {
        self.illegal.then_some(Action::Raise(1))
    }
}

#[test]
fn observations_only_show_own_cards() {
    let mut hand = exact_hand();
    let (mut a, mut b, mut c) = (
        Recorder::default(),
        Recorder::default(),
        Recorder::default(),
    );
    let outcome = play_hand(&mut hand, &mut [&mut a, &mut b, &mut c]).unwrap();
    assert_eq!(outcome.fallbacks, vec![0, 0, 0]);
    assert_eq!(outcome.result.final_stacks.iter().sum::<u64>(), 300);

    for (seat, bot) in [&a, &b, &c].into_iter().enumerate() {
        assert!(!bot.observations.is_empty());
        for obs in &bot.observations {
            assert_eq!(obs.seat, seat);
            assert_eq!(obs.hole_cards, hand.deal().hole_cards()[seat]);
            let json = serde_json::to_string(obs).unwrap();
            for (other, cards) in hand.deal().hole_cards().iter().enumerate() {
                if other != seat {
                    for card in cards.iter(false) {
                        assert!(!json.contains(&format!("\"{card}\"")), "{json}");
                    }
                }
            }
        }
        // Everyone checked down, so every hand is shown at the end.
        let summary = &bot.summaries[0];
        assert_eq!(summary.seat, seat);
        assert!(summary.shown.iter().all(Option::is_some));
        assert_eq!(summary.board.len(), 5);
    }
    // Only the acting seat gets an observation.
    let hand = exact_hand();
    assert!(hand.observation(0).is_some());
    assert!(hand.observation(1).is_none());
}

#[test]
fn folded_hands_stay_hidden() {
    let mut hand = exact_hand();
    let mut rec = Recorder::default();
    let (mut f1, mut f2) = (Broken { illegal: false }, Broken { illegal: false });
    // Seat 0 calls; seats 1 and 2 never answer. Seat 1 (small blind) faces a
    // bet and folds; seat 2 (big blind) can check, so it checks it down.
    play_hand(&mut hand, &mut [&mut rec, &mut f1, &mut f2]).unwrap();
    let summary = &rec.summaries[0];
    assert!(summary.result.showdown);
    assert_eq!(summary.shown[1], None);
    assert!(summary.shown[0].is_some() && summary.shown[2].is_some());
}

#[test]
fn bots_that_do_not_answer_check_or_fold() {
    let mut hand = exact_hand();
    let (mut silent, mut illegal, mut caller) = (
        Broken { illegal: false },
        Broken { illegal: true },
        CallingStation,
    );
    let outcome = play_hand(&mut hand, &mut [&mut silent, &mut illegal, &mut caller]).unwrap();
    // Seat 0 faces the big blind and folds; seat 1 folds the small blind.
    assert_eq!(outcome.fallbacks, vec![1, 1, 0]);
    assert!(!outcome.result.showdown);
    assert_eq!(outcome.result.final_stacks, vec![100, 99, 101]);
}

#[test]
fn duplicate_match_is_fair_between_identical_bots() {
    let config = MatchConfig::new(TableRules::no_limit_holdem(1, 2), 50, 3).duplicate();
    let mut bots: Vec<Box<dyn Bot>> = vec![Box::new(CallingStation), Box::new(CallingStation)];
    let result = run_match(&config, &mut bots).unwrap();
    assert_eq!(result.hands, 100);
    // Each bot played both seats of every deal with the same strategy.
    assert_eq!(result.net, vec![0, 0]);
    assert_eq!(result.fallbacks, vec![0, 0]);
}

#[test]
fn built_in_bots_play_full_matches() {
    for rules in [
        TableRules::no_limit_holdem(1, 2),
        TableRules::pot_limit_omaha(1, 2).with_ante(1),
    ] {
        let config = MatchConfig::new(rules, 20, 11).duplicate();
        let mut bots: Vec<Box<dyn Bot>> = vec![
            Box::new(CallingStation),
            Box::new(Raiser),
            Box::new(RandomBot::new(Some(5))),
            Box::new(EquityBot::new(30, Some(6))),
        ];
        let result = run_match(&config, &mut bots).unwrap();
        assert_eq!(result.hands, 80);
        assert_eq!(result.net.iter().sum::<i64>(), 0);
        // Built-in bots always give legal actions.
        assert_eq!(result.fallbacks, vec![0; 4]);
    }
}

#[test]
fn equity_bot_beats_a_calling_station() {
    let config = MatchConfig::new(TableRules::no_limit_holdem(1, 2), 150, 21).duplicate();
    let mut bots: Vec<Box<dyn Bot>> = vec![
        Box::new(EquityBot::new(60, Some(1))),
        Box::new(CallingStation),
    ];
    let result = run_match(&config, &mut bots).unwrap();
    assert!(result.net[0] > 0, "{result:?}");
    assert!(result.bb_per_100(0) > 0.0);
}

#[test]
fn equity_estimates() {
    let mut hand = exact_hand();
    // Preflop, aces against two random hands.
    let obs = hand.observation(0).unwrap();
    let eq = EquityBot::new(2_000, Some(4)).equity(&obs);
    assert!((0.6..0.8).contains(&eq), "{eq}");
    hand.act(Action::Fold).unwrap();
    // Kings against one random hand.
    let obs = hand.observation(1).unwrap();
    let eq = EquityBot::new(2_000, Some(4)).equity(&obs);
    assert!((0.75..0.9).contains(&eq), "{eq}");
}

fn has_python() -> bool {
    std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn external_python_bot_plays_a_match() {
    if !has_python() {
        eprintln!("python3 not found; skipping");
        return;
    }
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/bots/simple_bot.py");
    let config = MatchConfig::new(TableRules::pot_limit_omaha(1, 2), 30, 8).duplicate();
    let mut bots: Vec<Box<dyn Bot>> = vec![
        Box::new(ProcessBot::spawn("python3", &[script]).unwrap()),
        Box::new(CallingStation),
        Box::new(Raiser),
    ];
    let result = run_match(&config, &mut bots).unwrap();
    assert_eq!(result.hands, 90);
    assert_eq!(result.fallbacks[0], 0, "{result:?}");
    assert_eq!(result.net.iter().sum::<i64>(), 0);
}

#[test]
fn misbehaving_external_bots_fall_back() {
    if !has_python() {
        eprintln!("python3 not found; skipping");
        return;
    }
    let scripts = [
        // Unreadable replies.
        "import sys\nfor _ in sys.stdin: print('nonsense', flush=True)",
        // Too slow.
        "import sys, time\nfor _ in sys.stdin: time.sleep(2)",
        // Exits straight away.
        "pass",
        // Illegal action.
        "import sys, json\nfor l in sys.stdin:\n  if json.loads(l)['type'] == 'act': print(json.dumps({'action': 'raise', 'amount': 1}), flush=True)",
    ];
    for script in scripts {
        let bot = ProcessBot::spawn("python3", &["-c", script])
            .unwrap()
            .with_timeout(Duration::from_millis(200));
        let config = MatchConfig::new(TableRules::no_limit_holdem(1, 2), 3, 9);
        let mut bots: Vec<Box<dyn Bot>> = vec![Box::new(bot), Box::new(CallingStation)];
        let result = run_match(&config, &mut bots).unwrap();
        assert_eq!(result.hands, 3);
        assert!(result.fallbacks[0] > 0, "{script}: {result:?}");
        assert_eq!(result.net.iter().sum::<i64>(), 0);
    }
}

#[test]
fn observation_json_shape() {
    let hand = exact_hand();
    let obs = hand.observation(0).unwrap();
    let json: serde_json::Value = serde_json::to_value(&obs).unwrap();
    assert_eq!(json["seat"], 0);
    assert_eq!(json["street"], "preflop");
    assert_eq!(json["rules"]["variant"], "holdem");
    assert_eq!(json["rules"]["structure"], "no_limit");
    assert_eq!(json["legal"]["can_check"], false);
    assert_eq!(json["legal"]["call"], 2);
    assert_eq!(json["legal"]["raise"]["min_to"], 4);
    assert!(json["hole_cards"].as_str().unwrap().contains('A'));
    assert_eq!(json["history"][0]["type"], "small_blind");
    assert_eq!(json["seats"].as_array().unwrap().len(), 3);
}
