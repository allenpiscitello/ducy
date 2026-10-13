//! Plays GtoBot against Slumbot, the 2018 computer poker competition's
//! heads-up no-limit winner, through its public web API, and reports
//! the win rate in big blinds per 100 hands.
//!
//!     cargo run --release -p ducy-gto --example slumbot -- \
//!         --cards cards.bin --blueprint blueprint.bin --hands 1000 --log hands.jsonl
//!
//! Slumbot plays blinds 50/100 with 20,000 chip stacks (200 big blinds) and
//! deals the cards. Each hand runs in ducy-play's engine as a hidden deal:
//! Slumbot's action string (`b200c/kk/kb400`: `k` check, `c` call, `f` fold,
//! `bN` a bet or raise to N on the street) and its board are replayed into
//! the engine, GtoBot decides from the engine's observation with its own hole
//! cards added, and its action goes back as the same notation.
//!
//! Slumbot also reports a baseline for each hand: what its own strategy
//! would have won with our cards. Our result minus that cancels much of the
//! luck, so `vs baseline` needs far fewer hands than the raw rate.
//!
//! `--solve` turns on turn and river solving. `--delay MS` waits between
//! hands. `--log FILE` appends every hand as a JSON line. The API is reached
//! with `curl`.
//!
//! The blueprint is trained for 100 big blinds, so at 200 its all-in and
//! deep-stack play are approximations; read the results with that in mind.

use std::{io::Write, process::Command, sync::Arc, thread, time::Duration};

use ducy::deck::{Card, Deck};
use ducy_gto::holdem::{abstraction::CardAbstraction, bot::GtoBot, hunl::HunlConfig};
use ducy_play::{Action, Awaiting, Bot, Hand, HiddenDeal, TableRules, fallback_action};
use serde_json::{Value, json};

const API: &str = "https://slumbot.com/api";
const SMALL_BLIND: u64 = 50;
const BIG_BLIND: u64 = 100;
const STACK: u64 = 20_000;
/// Our seat in the engine; Slumbot is seat 1.
const ME: usize = 0;

fn post(path: &str, body: &Value) -> Result<Value, String> {
    let out = Command::new("curl")
        .args([
            "-sS",
            "-m",
            "60",
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "-d",
        ])
        .arg(body.to_string())
        .arg(format!("{API}/{path}"))
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "curl: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let v: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("bad reply {:?}: {e}", String::from_utf8_lossy(&out.stdout)))?;
    match v.get("error_msg").and_then(Value::as_str) {
        Some(err) => Err(format!("Slumbot: {err}")),
        None => Ok(v),
    }
}

fn cards(v: &Value, key: &str) -> Vec<Card> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|c| c.as_str()?.parse().ok()).collect())
        .unwrap_or_default()
}

fn deck(cards: &[Card]) -> Deck {
    let mut d = Deck::empty();
    for &c in cards {
        d |= c;
    }
    d
}

/// The engine's action for Slumbot notation `tok`, by the seat to act.
fn engine_action(hand: &Hand, tok: &str) -> Result<Action, String> {
    let seat = hand.to_act().ok_or("no one to act")?;
    Ok(match tok {
        "k" => Action::Check,
        "c" => Action::Call,
        "f" => Action::Fold,
        _ => {
            let to: u64 = tok
                .strip_prefix('b')
                .and_then(|n| n.parse().ok())
                .ok_or_else(|| format!("unknown action {tok:?}"))?;
            if to >= hand.street_bet(seat) + hand.stack(seat) {
                Action::AllIn
            } else if hand.current_bet() == 0 {
                Action::Bet(to)
            } else {
                Action::Raise(to)
            }
        }
    })
}

/// Our action in Slumbot notation.
fn notation(hand: &Hand, action: Action) -> String {
    match action {
        Action::Fold => "f".into(),
        Action::Check => "k".into(),
        Action::Call => "c".into(),
        Action::Bet(to) | Action::Raise(to) => format!("b{to}"),
        Action::AllIn => format!("b{}", hand.street_bet(ME) + hand.stack(ME)),
    }
}

/// Deals board cards the hand is waiting for, from Slumbot's board.
fn deal_board(hand: &mut Hand, board: &[Card]) -> Result<(), String> {
    while let Some(Awaiting::Board(street)) = hand.awaiting().cloned() {
        let have = hand.board().len();
        let need = street.board_cards();
        if board.len() < need {
            return Ok(());
        }
        hand.deal_board(&board[have..need])
            .map_err(|e| format!("board: {e:?}"))?;
    }
    Ok(())
}

/// Replays Slumbot's action string from token `done` on. Returns the new
/// count of tokens applied.
fn replay(hand: &mut Hand, action: &str, board: &[Card], mut done: usize) -> Result<usize, String> {
    let tokens: Vec<String> = tokenize(action);
    for tok in &tokens[done..] {
        if tok == "/" {
            deal_board(hand, board)?;
        } else {
            let a = engine_action(hand, tok)?;
            hand.act(a).map_err(|e| format!("{tok}: {e:?}"))?;
        }
        done += 1;
    }
    deal_board(hand, board)?;
    Ok(done)
}

/// `b200c/kk/kb400` → b200, c, /, k, k, /, k, b400.
fn tokenize(action: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = action.chars().peekable();
    while let Some(c) = chars.next() {
        if c == 'b' {
            let mut s = String::from("b");
            while let Some(d) = chars.peek().filter(|d| d.is_ascii_digit()) {
                s.push(*d);
                chars.next();
            }
            out.push(s);
        } else {
            out.push(c.to_string());
        }
    }
    out
}

struct Stats {
    hands: u64,
    won: Vec<f64>,
    vs_base: Vec<f64>,
}

impl Stats {
    fn rate(xs: &[f64]) -> (f64, f64) {
        let n = xs.len() as f64;
        if n < 2.0 {
            return (0.0, 0.0);
        }
        let mean = xs.iter().sum::<f64>() / n;
        let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
        // Chips per hand → big blinds per 100 hands.
        let k = 100.0 / BIG_BLIND as f64;
        (mean * k, 1.96 * (var / n).sqrt() * k)
    }
}

fn play_hand(bot: &mut GtoBot, token: &mut Option<String>) -> Result<Value, String> {
    let body = match token {
        Some(t) => json!({ "token": t }),
        None => json!({}),
    };
    let mut r = post("new_hand", &body)?;
    if let Some(t) = r.get("token").and_then(Value::as_str) {
        *token = Some(t.to_string());
    }
    let hole = cards(&r, "hole_cards");
    // client_pos 0: we're the big blind, so Slumbot has the button.
    let button = if r["client_pos"].as_u64() == Some(0) {
        1
    } else {
        ME
    };
    let rules = TableRules::no_limit_holdem(SMALL_BLIND, BIG_BLIND);
    let mut hand = Hand::with_dealer(
        rules,
        &[STACK, STACK],
        button,
        &HiddenDeal { players: 2 },
        &[],
    )
    .map_err(|e| format!("hand: {e:?}"))?;
    let mut done = 0;
    loop {
        let action = r["action"].as_str().unwrap_or("").to_string();
        let board = cards(&r, "board");
        done = replay(&mut hand, &action, &board, done)?;
        if r.get("winnings").is_some() {
            return Ok(r);
        }
        let mut obs = hand
            .observation(ME)
            .ok_or_else(|| format!("not our turn after {action:?}"))?;
        obs.hole_cards = deck(&hole);
        // Anything the engine wouldn't accept becomes check, else fold.
        let a = bot
            .act(&obs)
            .filter(|a| hand.clone().act(*a).is_ok())
            .unwrap_or_else(|| fallback_action(&obs.legal));
        let incr = notation(&hand, a);
        let t = token.clone().ok_or("no token")?;
        r = post("act", &json!({ "token": t, "incr": incr }))?;
    }
}

fn main() {
    let mut cards_path = String::new();
    let mut blueprint_path = String::new();
    let mut hands = 100u64;
    let mut solve = false;
    let mut delay = 0u64;
    let mut log: Option<String> = None;
    let mut seed = 1u64;
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut v = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--cards" => cards_path = v(),
            "--blueprint" => blueprint_path = v(),
            "--hands" => hands = v().replace('_', "").parse().expect("--hands"),
            "--solve" => solve = true,
            "--delay" => delay = v().parse().expect("--delay"),
            "--log" => log = Some(v()),
            "--seed" => seed = v().parse().expect("--seed"),
            f => panic!("unknown option {f}"),
        }
    }
    let cards = Arc::new(
        CardAbstraction::load(&std::fs::read(&cards_path).expect("read --cards"))
            .expect("abstraction"),
    );
    let bytes = std::fs::read(&blueprint_path).expect("read --blueprint");
    let mut bot = GtoBot::new(HunlConfig::default(), cards, &bytes, seed).expect("blueprint");
    if solve {
        bot = bot.with_solving();
    }
    let mut out = log.map(|p| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .expect("open --log")
    });
    let mut token = None;
    let mut s = Stats {
        hands: 0,
        won: Vec::new(),
        vs_base: Vec::new(),
    };
    let mut errors = 0;
    while s.hands < hands {
        match play_hand(&mut bot, &mut token) {
            Ok(r) => {
                let won = r["winnings"].as_f64().unwrap_or(0.0);
                let base = r["baseline_winnings"].as_f64();
                s.hands += 1;
                s.won.push(won);
                if let Some(b) = base {
                    s.vs_base.push(won - b);
                }
                if let Some(f) = out.as_mut() {
                    writeln!(f, "{r}").expect("write --log");
                }
                if s.hands % 100 == 0 || s.hands == hands {
                    let (w, wc) = Stats::rate(&s.won);
                    let (b, bc) = Stats::rate(&s.vs_base);
                    println!(
                        "{:>7} hands: {w:+.1} ± {wc:.1} bb/100, vs baseline {b:+.1} ± {bc:.1} (off-tree {}, errors {errors})",
                        s.hands, bot.off_tree
                    );
                }
            }
            Err(e) => {
                errors += 1;
                eprintln!("hand failed: {e}");
                // A fresh session after an error, after a pause.
                token = None;
                thread::sleep(Duration::from_secs(5));
                if errors > 20 {
                    panic!("too many errors");
                }
            }
        }
        if delay > 0 {
            thread::sleep(Duration::from_millis(delay));
        }
    }
}
