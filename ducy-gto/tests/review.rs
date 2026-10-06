use std::sync::{Arc, Mutex};

use ducy_gto::{
    Profile, Rng,
    holdem::{
        abstraction::CardAbstraction,
        blueprint::Blueprint,
        bot::GtoBot,
        cards::{Card, NUM_HOLES, bit, hole_cards, hole_index, mask, parse, score},
        hunl::{Hunl, HunlAction, HunlConfig},
        range::{BucketCache, board_for},
        review::{
            Evaluator, Grade, HandRecord, ReviewConfig, ReviewLog, Reviewer, SessionReview, replay,
        },
    },
};
use ducy_play::{
    Bot, Event, HandSummary, MatchConfig, Observation, Street, TableRules, bots::RandomBot,
    run_match,
};

const BUCKETS: usize = 8;

/// A simple strategy by bucket (the quick abstraction buckets by hand
/// strength): preflop, check or call everything; after it, shove the top
/// quarter, call with the top half, and check or fold the rest.
fn threshold_blueprint(game: &Hunl, cards: &CardAbstraction) -> Blueprint {
    let mut p = Profile::new();
    for (id, n) in game.tree.nodes.iter().enumerate() {
        if n.actions.is_empty() {
            continue;
        }
        let street = n.betting.street;
        for b in 0..cards.num_buckets([0, 3, 4, 5][street]) {
            let pos = |pred: &dyn Fn(&HunlAction) -> bool| n.actions.iter().position(pred);
            let passive = pos(&|a| matches!(a, HunlAction::Check | HunlAction::Call)).unwrap();
            let pick = if street == 0 {
                passive
            } else if b >= 3 * BUCKETS / 4 && n.actions.len() > passive + 1 {
                n.actions.len() - 1
            } else if b >= BUCKETS / 2 {
                passive
            } else {
                pos(&|a| *a == HunlAction::Fold).unwrap_or(passive)
            };
            let mut probs = vec![0.0; n.actions.len()];
            probs[pick] = 1.0;
            p.set((id as u64) << 16 | b as u64, probs);
        }
    }
    Blueprint::from_profile(game, cards, &p)
}

/// A mixed strategy: random probabilities for every node and bucket.
fn random_blueprint(game: &Hunl, cards: &CardAbstraction, seed: u64) -> Blueprint {
    let mut rng = Rng::new(seed);
    let mut p = Profile::new();
    for (id, n) in game.tree.nodes.iter().enumerate() {
        if n.actions.is_empty() {
            continue;
        }
        for b in 0..cards.num_buckets([0, 3, 4, 5][n.betting.street]) {
            let w: Vec<f64> = (0..n.actions.len())
                .map(|_| rng.next_f64() + 0.05)
                .collect();
            let t: f64 = w.iter().sum();
            p.set(
                (id as u64) << 16 | b as u64,
                w.iter().map(|x| x / t).collect(),
            );
        }
    }
    Blueprint::from_profile(game, cards, &p)
}

fn setup() -> (CardAbstraction, HunlConfig) {
    (CardAbstraction::quick(BUCKETS), HunlConfig::default())
}

fn c(s: &str) -> ducy::deck::Card {
    ducy::deck::Card::parse(s).unwrap()
}

fn parse_cards(s: &str) -> Vec<Card> {
    parse(s).unwrap()
}

/// A hand at blinds 1/2, 100 big blinds: the opponent on the button (seat 0)
/// limps, everyone checks to the river, the big blind (seat 1, reviewed)
/// checks, the button shoves, and the big blind answers with `last`.
fn river_shove(hole: &str, board: &str, last: Event) -> HandRecord {
    let b: Vec<&str> = board.split(' ').collect();
    let history = vec![
        Event::SmallBlind { seat: 0, amount: 1 },
        Event::BigBlind { seat: 1, amount: 2 },
        Event::Call {
            seat: 0,
            amount: 1,
            all_in: false,
        },
        Event::Check { seat: 1 },
        Event::Board {
            street: Street::Flop,
            cards: vec![c(b[0]), c(b[1]), c(b[2])],
        },
        Event::Check { seat: 1 },
        Event::Check { seat: 0 },
        Event::Board {
            street: Street::Turn,
            cards: vec![c(b[3])],
        },
        Event::Check { seat: 1 },
        Event::Check { seat: 0 },
        Event::Board {
            street: Street::River,
            cards: vec![c(b[4])],
        },
        Event::Check { seat: 1 },
        Event::Bet {
            seat: 0,
            to: 198,
            all_in: true,
        },
        last,
    ];
    HandRecord {
        seat: 1,
        button: 0,
        hole: parse_cards(hole).try_into().unwrap(),
        board: parse_cards(board),
        history,
        big_blind: 2,
        net: 0,
    }
}

fn reviewer<'a>(game: &'a Hunl, cards: &'a CardAbstraction, bp: &'a Blueprint) -> Reviewer<'a> {
    Reviewer {
        tree: &game.tree,
        blueprint: bp,
        cards,
        tree_big_blind: game.config.big_blind,
        config: ReviewConfig::default(),
    }
}

#[test]
fn folding_the_nuts_and_calling_with_air_are_blunders() {
    let (cards, config) = setup();
    let game = Hunl::new(config, Some(&cards));
    let bp = threshold_blueprint(&game, &cards);
    let r = reviewer(&game, &cards, &bp);
    let board = "Ts Js Qs 4d 2c";
    // The royal flush folds to a shove: the whole pot of 101 bb is lost.
    let rev = r.review(
        &river_shove("As Ks", board, Event::Fold { seat: 1 }),
        &mut BucketCache::default(),
    );
    let d = rev.decisions.last().unwrap();
    assert_eq!(d.street, "river");
    assert_eq!(d.grade, Grade::Blunder, "{d:?}");
    assert!((d.bb_lost - 101.0).abs() < 1e-6, "{d:?}");
    assert_eq!(d.best.as_deref(), Some("call"));
    // Seven-three, beaten by every hand that shoves, calls off 99 bb.
    let rev = r.review(
        &river_shove(
            "7h 3d",
            board,
            Event::Call {
                seat: 1,
                amount: 198,
                all_in: true,
            },
        ),
        &mut BucketCache::default(),
    );
    let d = rev.decisions.last().unwrap();
    assert_eq!(d.grade, Grade::Blunder, "{d:?}");
    assert!((d.bb_lost - 99.0).abs() < 1e-6, "{d:?}");
    // Its earlier checks were the blueprint's play.
    assert!(
        rev.decisions[..rev.decisions.len() - 1]
            .iter()
            .all(|d| d.grade == Grade::Fine && d.bb_lost == 0.0)
    );
    assert!((rev.bb_lost - 99.0).abs() < 1e-6);
}

#[test]
fn checking_the_nuts_gives_up_value() {
    let (cards, config) = setup();
    let game = Hunl::new(config, Some(&cards));
    let bp = threshold_blueprint(&game, &cards);
    let r = reviewer(&game, &cards, &bp);
    // The big blind checks the royal flush on the river instead of shoving,
    // which the medium hands would have called.
    let rec = river_shove(
        "As Ks",
        "Ts Js Qs 4d 2c",
        Event::Call {
            seat: 1,
            amount: 198,
            all_in: true,
        },
    );
    let rev = r.review(&rec, &mut BucketCache::default());
    let check = &rev.decisions[rev.decisions.len() - 2];
    assert_eq!(check.action, "check");
    assert_eq!(check.main, "all-in");
    assert_eq!(check.frequency, 0.0);
    assert!(check.bb_lost > 0.0, "{check:?}");
    assert_ne!(check.grade, Grade::Fine);
    // Calling the shove with the nuts afterwards is the blueprint's play.
    assert_eq!(rev.decisions.last().unwrap().grade, Grade::Fine);
}

#[test]
fn reviews_are_reproducible() {
    let (cards, config) = setup();
    let game = Hunl::new(config, Some(&cards));
    let bp = random_blueprint(&game, &cards, 3);
    let mut r = reviewer(&game, &cards, &bp);
    // A mixed strategy prunes nothing: a small sample keeps it quick.
    r.config.flop_runouts = 3;
    let rec = river_shove("9h 9d", "Ts Js 3h 4d 2c", Event::Fold { seat: 1 });
    let a = r.review(&rec, &mut BucketCache::default());
    let b = r.review(&rec, &mut BucketCache::default());
    assert_eq!(a, b);
}

thread_local! {
    static BUCKETS_SEEN: std::cell::RefCell<std::collections::HashMap<(usize, Vec<Card>), u16>> =
        Default::default();
}

/// `cards.bucket`, remembered (brute force asks for the same ones often).
fn bucket(cards: &CardAbstraction, hole: [Card; 2], board: &[Card]) -> u16 {
    let key = (hole_index(hole[0], hole[1]), board.to_vec());
    BUCKETS_SEEN.with(|m| {
        *m.borrow_mut()
            .entry(key)
            .or_insert_with(|| cards.bucket(hole, board))
    })
}

/// Every opponent hand's value one at a time, walking the tree and every
/// river card: what the evaluator does with vectors.
#[allow(clippy::too_many_arguments)]
fn brute(
    game: &Hunl,
    cards: &CardAbstraction,
    bp: &Blueprint,
    node: u32,
    side: usize,
    hero: [Card; 2],
    opp: [Card; 2],
    board: &[Card],
) -> f64 {
    let n = &game.tree.nodes[node as usize];
    let b = &n.betting;
    let used = mask(board) | mask(&hero) | mask(&opp);
    let need = if n.actions.is_empty() {
        5
    } else {
        [0, 3, 4, 5][b.street]
    };
    if n.actions.is_empty()
        && let Some(f) = b.folded
    {
        return if f == side {
            -(b.contributed[side] as f64)
        } else {
            b.contributed[1 - side] as f64
        };
    }
    if board.len() < need {
        let (mut v, mut k) = (0.0, 0.0);
        for x in 0..52u8 {
            if used & bit(x) == 0 {
                let mut nb = board.to_vec();
                nb.push(x);
                v += brute(game, cards, bp, node, side, hero, opp, &nb);
                k += 1.0;
            }
        }
        return v / k;
    }
    if n.actions.is_empty() {
        let bm = mask(board);
        let me = score(bm | mask(&hero));
        let them = score(bm | mask(&opp));
        return (me.cmp(&them) as i32) as f64 * b.contributed[0].min(b.contributed[1]) as f64;
    }
    let who = if b.to_act == side { hero } else { opp };
    let probs = bp.probs(node, bucket(cards, who, board));
    n.children
        .iter()
        .zip(&probs)
        .map(|(&ch, &p)| p * brute(game, cards, bp, ch, side, hero, opp, board))
        .sum()
}

#[test]
fn turn_and_river_values_match_brute_force() {
    let (cards, config) = setup();
    let game = Hunl::new(config, Some(&cards));
    let bp = random_blueprint(&game, &cards, 5);
    let rec = river_shove("9h 9d", "Ts Js 3h 4d 2c", Event::Fold { seat: 1 });
    let decisions = replay(
        &rec,
        &game.tree,
        &cards,
        &bp,
        &mut BucketCache::default(),
        1,
    );
    let eval = Evaluator::new(&game.tree, &bp, &cards, 2);
    let mut rng = Rng::new(1);
    // The big blind's first turn and river decisions.
    for street in [2, 3] {
        let d = decisions
            .iter()
            .find(|d| d.player == 1 && d.street == street)
            .unwrap();
        let node = d.node.unwrap();
        let board = board_for(street, &rec.board);
        // A small opponent range keeps brute force quick.
        let mut range = vec![0.0; NUM_HOLES];
        for (h, w) in [
            ("Ah Kh", 1.0),
            ("5c 6c", 0.5),
            ("Td 8d", 2.0),
            ("Qc Qd", 0.25),
        ] {
            let x = parse_cards(h);
            range[hole_index(x[0], x[1])] = w;
        }
        let v = eval
            .values(
                node,
                1,
                rec.hole,
                board,
                &range,
                &ReviewConfig::default(),
                &mut rng,
            )
            .unwrap();
        let n = &game.tree.nodes[node as usize];
        for (a, &child) in n.children.iter().enumerate() {
            let (mut num, mut den) = (0.0, 0.0);
            for (h, &w) in range.iter().enumerate() {
                if w > 0.0 {
                    let (x, y) = hole_cards(h);
                    num += w * brute(&game, &cards, &bp, child, 1, rec.hole, [x, y], board);
                    den += w;
                }
            }
            let want = (num / den + n.betting.contributed[1] as f64) / 2.0;
            assert!(
                (v.ev[a] - want).abs() < 1e-6,
                "street {street} action {a}: {} vs {want}",
                v.ev[a]
            );
            assert_eq!(v.stderr[a], 0.0);
        }
    }
}

#[test]
fn ranges_match_brute_force() {
    let (cards, config) = setup();
    let game = Hunl::new(config, Some(&cards));
    let bp = random_blueprint(&game, &cards, 7);
    let rec = river_shove("9h 9d", "Ts Js 3h 4d 2c", Event::Fold { seat: 1 });
    let decisions = replay(
        &rec,
        &game.tree,
        &cards,
        &bp,
        &mut BucketCache::default(),
        1,
    );
    // The button's range before the big blind's last decision: every one of
    // its actions' probabilities for each hand's bucket, multiplied.
    let last = decisions.last().unwrap();
    let mut probs = vec![1.0; NUM_HOLES];
    for d in decisions.iter().filter(|d| d.player == 0) {
        let step = d.step.as_ref().unwrap();
        let board = board_for(d.street, &rec.board);
        for (h, p) in probs.iter_mut().enumerate() {
            let (a, b) = hole_cards(h);
            if mask(&rec.board) & (bit(a) | bit(b)) != 0 {
                *p = 0.0;
            } else {
                *p *= bp.probs(step.node, cards.bucket([a, b], board))[step.action];
            }
        }
    }
    for (h, (got, want)) in last.ranges[0].iter().zip(&probs).enumerate() {
        assert!((got - want).abs() < 1e-12, "hand {h}");
    }
}

/// A bot that records the pot at each of its decisions, and each hand's
/// summary for its seat.
struct Recorder {
    bot: Box<dyn Bot>,
    current: Vec<u64>,
    pots: Arc<Mutex<Vec<Vec<u64>>>>,
    hands: Arc<Mutex<Vec<HandSummary>>>,
}

impl Recorder {
    fn new(
        bot: Box<dyn Bot>,
        pots: &Arc<Mutex<Vec<Vec<u64>>>>,
        hands: &Arc<Mutex<Vec<HandSummary>>>,
    ) -> Self {
        Self {
            bot,
            current: Vec::new(),
            pots: pots.clone(),
            hands: hands.clone(),
        }
    }
}

impl Bot for Recorder {
    fn act(&mut self, obs: &Observation) -> Option<ducy_play::Action> {
        self.current.push(obs.pot);
        self.bot.act(obs)
    }

    fn hand_over(&mut self, s: &HandSummary) {
        self.pots
            .lock()
            .unwrap()
            .push(std::mem::take(&mut self.current));
        self.hands.lock().unwrap().push(s.clone());
        self.bot.hand_over(s);
    }
}

#[test]
fn replay_reproduces_the_real_pot_and_self_review_loses_nothing() {
    let cards = Arc::new(CardAbstraction::quick(BUCKETS));
    let config = HunlConfig::default();
    let game = Hunl::new(config.clone(), Some(&cards));
    let bp = threshold_blueprint(&game, &cards);
    let bytes = bp.save();
    let pots = Arc::new(Mutex::new(Vec::new()));
    let hands = Arc::new(Mutex::new(Vec::new()));
    let me = Recorder::new(
        Box::new(GtoBot::new(config.clone(), cards.clone(), &bytes, 1).unwrap()),
        &pots,
        &hands,
    );
    // Against a random bettor (every size, so many mapped bets) and itself.
    let others: Vec<Box<dyn Bot>> = vec![
        Box::new(RandomBot::new(Some(2))),
        Box::new(GtoBot::new(config.clone(), cards.clone(), &bytes, 3).unwrap()),
    ];
    let mut me = Some(me);
    let mut all_reviews = Vec::new();
    for other in others {
        let mut bots: Vec<Box<dyn Bot>> = vec![Box::new(me.take().unwrap()), other];
        run_match(
            &MatchConfig::new(TableRules::no_limit_holdem(1, 2), 30, 9),
            &mut bots,
        )
        .unwrap();
        let r = reviewer(&game, &cards, &bp);
        let hs = std::mem::take(&mut *hands.lock().unwrap());
        let ps = std::mem::take(&mut *pots.lock().unwrap());
        let mut cache = BucketCache::default();
        for (s, pots) in hs.iter().zip(&ps) {
            let rec = HandRecord::from_summary(s).unwrap();
            let decisions = replay(&rec, &game.tree, &cards, &bp, &mut cache, 1);
            let mine: Vec<u64> = decisions
                .iter()
                .filter(|d| d.player == rec.side())
                .map(|d| d.real.pot())
                .collect();
            assert_eq!(&mine, pots, "pots before each decision");
            all_reviews.push(r.review(&rec, &mut cache));
        }
        // A fresh recorder for the next opponent.
        me = Some(Recorder::new(
            Box::new(GtoBot::new(config.clone(), cards.clone(), &bytes, 1).unwrap()),
            &pots,
            &hands,
        ));
    }
    // The bot plays the blueprint, so nothing is charged, wherever the hand
    // could be followed.
    let s = SessionReview::of(&all_reviews);
    assert_eq!(s.hands, 60);
    assert!(s.decisions > 60);
    assert_eq!(s.bb_lost, 0.0, "{s:?}");
    assert_eq!(
        s.inaccuracies + s.mistakes + s.blunders + s.deviations,
        0,
        "{s:?}"
    );
    assert!(s.fine * 10 >= s.decisions * 9, "{s:?}");
}

#[test]
fn flop_values_converge_to_brute_force() {
    let (cards, config) = setup();
    let game = Hunl::new(config, Some(&cards));
    let bp = threshold_blueprint(&game, &cards);
    let rec = river_shove("9h 9d", "Ts Js 3h 4d 2c", Event::Fold { seat: 1 });
    let decisions = replay(
        &rec,
        &game.tree,
        &cards,
        &bp,
        &mut BucketCache::default(),
        1,
    );
    let eval = Evaluator::new(&game.tree, &bp, &cards, 2);
    let d = decisions
        .iter()
        .find(|d| d.player == 1 && d.street == 1)
        .unwrap();
    let node = d.node.unwrap();
    let board = board_for(1, &rec.board);
    let mut range = vec![0.0; NUM_HOLES];
    for (h, w) in [("Ah Kh", 1.0), ("5c 6c", 0.5), ("Qc Qd", 0.25)] {
        let x = parse_cards(h);
        range[hole_index(x[0], x[1])] = w;
    }
    let n = &game.tree.nodes[node as usize];
    let want: Vec<f64> = n
        .children
        .iter()
        .map(|&child| {
            let (mut num, mut den) = (0.0, 0.0);
            for (h, &w) in range.iter().enumerate() {
                if w > 0.0 {
                    let (x, y) = hole_cards(h);
                    num += w * brute(&game, &cards, &bp, child, 1, rec.hole, [x, y], board);
                    den += w;
                }
            }
            (num / den + n.betting.contributed[1] as f64) / 2.0
        })
        .collect();
    let mut rng = Rng::new(3);
    // Every turn and river once: exact.
    let all = ReviewConfig {
        flop_runouts: 47 * 46,
        ..ReviewConfig::default()
    };
    let v = eval
        .values(node, 1, rec.hole, board, &range, &all, &mut rng)
        .unwrap();
    for (a, &want) in want.iter().enumerate() {
        assert!(
            (v.ev[a] - want).abs() < 1e-6,
            "{a}: {} vs {}",
            v.ev[a],
            want
        );
    }
    // The default sample: within a few standard errors.
    let v = eval
        .values(
            node,
            1,
            rec.hole,
            board,
            &range,
            &ReviewConfig::default(),
            &mut rng,
        )
        .unwrap();
    assert_eq!(v.runouts, 32);
    for (a, &want) in want.iter().enumerate() {
        assert!(
            (v.ev[a] - want).abs() <= 4.0 * v.stderr[a] + 1e-9,
            "{a}: {} ± {} vs {}",
            v.ev[a],
            v.stderr[a],
            want
        );
    }
}

#[test]
fn review_log_keeps_hands_and_reviews_each_once() {
    let cards = Arc::new(CardAbstraction::quick(BUCKETS));
    let config = HunlConfig::default();
    let game = Hunl::new(config.clone(), Some(&cards));
    let bp = threshold_blueprint(&game, &cards);
    let bytes = bp.save();
    let pots = Arc::new(Mutex::new(Vec::new()));
    let hands = Arc::new(Mutex::new(Vec::new()));
    let mut bots: Vec<Box<dyn Bot>> = vec![
        Box::new(GtoBot::new(config.clone(), cards.clone(), &bytes, 4).unwrap()),
        Box::new(Recorder::new(
            Box::new(RandomBot::new(Some(5))),
            &pots,
            &hands,
        )),
    ];
    run_match(
        &MatchConfig::new(TableRules::no_limit_holdem(1, 2), 12, 3),
        &mut bots,
    )
    .unwrap();
    let r = reviewer(&game, &cards, &bp);
    let mut log = ReviewLog::default();
    for s in hands.lock().unwrap().iter() {
        assert!(log.record(s));
    }
    assert_eq!(log.len(), 12);
    let last = log.last(&r).unwrap().clone();
    let (all, session) = log.all(&r);
    assert_eq!(all.len(), 12);
    assert_eq!(all[11], last);
    assert_eq!(session.hands, 12);
    assert_eq!(
        session.decisions,
        all.iter().map(|h| h.decisions.len()).sum::<usize>()
    );
    let total: f64 = all.iter().map(|h| h.bb_lost).sum();
    assert!((session.bb_lost - total).abs() < 1e-9);
    // A random bettor makes plenty of plays the blueprint doesn't.
    assert!(session.fine < session.decisions, "{session:?}");
    log.clear();
    assert!(log.is_empty() && log.last(&r).is_none());
    // Only heads-up no-limit Hold'em is kept.
    let mut s = hands.lock().unwrap()[0].clone();
    s.rules = TableRules::pot_limit_omaha(1, 2);
    assert!(!log.record(&s));
}
