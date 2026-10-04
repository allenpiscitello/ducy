//! Runs bots against each other for many hands and reports how each one did.
//!
//! Every hand starts from the same stack (100 big blinds by default), players
//! are randomly reseated across tables every few hands, and each player's
//! wins and losses are tracked as big blinds per 100 hands.
//!
//! ```text
//! cargo run --release -p ducy-play --bin ducy-arena -- --hands 20000
//! cargo run --release -p ducy-play --bin ducy-arena -- \
//!     --players doug_poker,milk_king,nik_airbag,old_man_coffee \
//!     --bot "mine=python3 ducy-play/examples/bots/simple_bot.py" \
//!     --hands 5000 --table-size 6 --finals 6 --csv results.csv
//! ```
//!
//! Run with `--help` for every option.

use std::{process::ExitCode, time::Instant};

use ducy_play::{
    ArenaConfig, ArenaResult, Bot, Personality, PersonalityBot, ProcessBot, TableRules, Variant,
    run_arena,
};

const HELP: &str = "\
ducy-arena: play bots against each other and report win rates

USAGE:
    ducy-arena [OPTIONS]

PLAYERS:
    --players LIST       Comma-separated personality ids, or \"all\" (default: all)
    --bot NAME=COMMAND   Add an external bot program speaking the ducy-play JSON
                         protocol, e.g. --bot \"mine=python3 my_bot.py\".
                         Repeatable. The command is split on spaces.
    --list               List the personalities and exit

SESSION:
    --hands N            Hands per player (default: 10000)
    --table-size N       Seats per table, 2-10 (default: 6)
    --reseat N           Hands at a table before reseating everyone (default: 9)
    --stack BB           Starting stack every hand, in big blinds (default: 100)
    --blinds SB/BB       Blinds in chips (default: 1/2)
    --game GAME          nlhe or plo (default: nlhe)
    --seed N             Seed for seating and cards (default: 1)
    --samples N          Monte Carlo deals per decision for personalities
                         (default: their own, 150; lower is faster)

AFTER THE SESSION:
    --finals N           Then play the top N players against each other only
    --final-hands N      Hands per player in the finals (default: --hands)
    --csv PATH           Write results as CSV (finals go to PATH with
                         .finals before the extension)
    -h, --help           Show this help
";

struct Options {
    players: Vec<Personality>,
    external: Vec<(String, String)>,
    hands: usize,
    table_size: usize,
    reseat: usize,
    stack: u64,
    blinds: (u64, u64),
    game: Variant,
    seed: u64,
    samples: Option<usize>,
    finals: usize,
    final_hands: Option<usize>,
    csv: Option<String>,
}

fn parse_args() -> Result<Option<Options>, String> {
    let mut o = Options {
        players: Personality::ALL.to_vec(),
        external: Vec::new(),
        hands: 10_000,
        table_size: 6,
        reseat: 9,
        stack: 100,
        blinds: (1, 2),
        game: Variant::Holdem,
        seed: 1,
        samples: None,
        finals: 0,
        final_hands: None,
        csv: None,
    };
    let mut args = std::env::args().skip(1);
    let number = |name: &str, value: Option<String>| -> Result<u64, String> {
        value
            .ok_or_else(|| format!("{name} needs a value"))?
            .parse()
            .map_err(|_| format!("{name} needs a whole number"))
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                return Ok(None);
            }
            "--list" => {
                for p in Personality::ALL {
                    println!("{:18} {:20} {}", p.id(), p.name(), p.description());
                }
                return Ok(None);
            }
            "--players" => {
                let list = args.next().ok_or("--players needs a value")?;
                o.players = if list == "all" {
                    Personality::ALL.to_vec()
                } else if list.is_empty() || list == "none" {
                    Vec::new()
                } else {
                    list.split(',')
                        .map(|id| {
                            Personality::from_name(id.trim())
                                .ok_or_else(|| format!("unknown personality \"{id}\" (see --list)"))
                        })
                        .collect::<Result<_, _>>()?
                };
            }
            "--bot" => {
                let spec = args.next().ok_or("--bot needs NAME=COMMAND")?;
                let (name, command) = spec.split_once('=').ok_or("--bot needs NAME=COMMAND")?;
                o.external.push((name.to_string(), command.to_string()));
            }
            "--hands" => o.hands = number("--hands", args.next())? as usize,
            "--table-size" => o.table_size = number("--table-size", args.next())? as usize,
            "--reseat" => o.reseat = number("--reseat", args.next())? as usize,
            "--stack" => o.stack = number("--stack", args.next())?,
            "--seed" => o.seed = number("--seed", args.next())?,
            "--samples" => o.samples = Some(number("--samples", args.next())? as usize),
            "--finals" => o.finals = number("--finals", args.next())? as usize,
            "--final-hands" => o.final_hands = Some(number("--final-hands", args.next())? as usize),
            "--csv" => o.csv = Some(args.next().ok_or("--csv needs a path")?),
            "--blinds" => {
                let v = args.next().ok_or("--blinds needs SB/BB")?;
                let (sb, bb) = v.split_once('/').ok_or("--blinds needs SB/BB, e.g. 1/2")?;
                o.blinds = (
                    sb.parse().map_err(|_| "--blinds needs whole numbers")?,
                    bb.parse().map_err(|_| "--blinds needs whole numbers")?,
                );
            }
            "--game" => {
                o.game = match args.next().as_deref() {
                    Some("nlhe" | "holdem") => Variant::Holdem,
                    Some("plo") => Variant::Omaha { hole_cards: 4 },
                    _ => return Err("--game must be nlhe or plo".into()),
                };
            }
            other => return Err(format!("unknown option {other} (see --help)")),
        }
    }
    Ok(Some(o))
}

/// A player's name and bot.
type NamedBot = (String, Box<dyn Bot>);

/// Creates the bots for `entries` (personality or external command).
fn make_bots(
    entries: &[Entry],
    samples: Option<usize>,
    seed: u64,
) -> Result<Vec<NamedBot>, String> {
    entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let bot: Box<dyn Bot> = match entry {
                Entry::Personality(p) => {
                    let mut style = p.style();
                    if let Some(samples) = samples {
                        style.samples = samples;
                    }
                    Box::new(PersonalityBot::new(
                        p.name(),
                        style,
                        Some(seed ^ (i as u64 + 1)),
                    ))
                }
                Entry::External(name, command) => {
                    let mut parts = command.split_whitespace();
                    let program = parts.next().ok_or(format!("empty command for {name}"))?;
                    let rest: Vec<&str> = parts.collect();
                    Box::new(
                        ProcessBot::spawn(program, &rest)
                            .map_err(|e| format!("couldn't start {name}: {e}"))?,
                    )
                }
            };
            Ok((entry.name(), bot))
        })
        .collect()
}

#[derive(Clone)]
enum Entry {
    Personality(Personality),
    External(String, String),
}

impl Entry {
    fn name(&self) -> String {
        match self {
            Self::Personality(p) => p.name().to_string(),
            Self::External(name, _) => name.clone(),
        }
    }
}

fn percent(count: u32, total: u32) -> f64 {
    if total == 0 {
        0.0
    } else {
        100.0 * count as f64 / total as f64
    }
}

fn print_results(title: &str, result: &ArenaResult, seconds: f64) {
    println!(
        "\n{title}: {} hands dealt in {seconds:.0}s",
        result.hands_dealt
    );
    println!(
        "  {:>2}  {:20} {:>8} {:>10} {:>9} {:>8} {:>5} {:>5} {:>5} {:>6} {:>5}",
        "#", "player", "hands", "net bb", "bb/100", "± 95%", "VPIP", "PFR", "AF", "F2Bet", "fb"
    );
    for (rank, p) in result.players.iter().enumerate() {
        let s = &p.stats;
        let af = if s.calls == 0 {
            s.aggressive as f64
        } else {
            s.aggressive as f64 / s.calls as f64
        };
        println!(
            "  {:>2}  {:20} {:>8} {:>+10.0} {:>+9.1} {:>8.1} {:>4.0}% {:>4.0}% {:>5.2} {:>5.0}% {:>5}",
            rank + 1,
            p.name,
            p.hands,
            p.net_bb,
            p.bb_per_100,
            2.0 * p.std_error,
            percent(s.vpip_hands, s.hands),
            percent(s.pfr_hands, s.hands),
            af,
            percent(s.folds_to_bets, s.faced_bets),
            p.fallbacks,
        );
    }
    println!(
        "  (± 95% is about two standard errors; VPIP/PFR = hands played/raised preflop, AF = postflop bets+raises per call, F2Bet = folds to postflop bets, fb = fallbacks)"
    );
}

fn write_csv(path: &str, result: &ArenaResult) -> std::io::Result<()> {
    let mut out = String::from(
        "rank,player,hands,net_chips,net_bb,bb_per_100,std_error,vpip,pfr,aggression,fold_to_bet,fallbacks\n",
    );
    for (rank, p) in result.players.iter().enumerate() {
        let s = &p.stats;
        out.push_str(&format!(
            "{},{},{},{},{:.2},{:.3},{:.3},{:.4},{:.4},{:.4},{:.4},{}\n",
            rank + 1,
            p.name.replace(',', " "),
            p.hands,
            p.net_chips,
            p.net_bb,
            p.bb_per_100,
            p.std_error,
            percent(s.vpip_hands, s.hands) / 100.0,
            percent(s.pfr_hands, s.hands) / 100.0,
            s.aggressive as f64 / s.calls.max(1) as f64,
            percent(s.folds_to_bets, s.faced_bets) / 100.0,
            p.fallbacks,
        ));
    }
    std::fs::write(path, out)
}

fn session(
    title: &str,
    config: &ArenaConfig,
    entries: &[Entry],
    o: &Options,
) -> Result<ArenaResult, String> {
    let mut bots = make_bots(entries, o.samples, config.seed)?;
    let started = Instant::now();
    let mut next_report = 0.1;
    let result = run_arena(config, &mut bots, |p| {
        let done = p.hands_per_player as f64 / p.target.max(1) as f64;
        if done >= next_report {
            eprintln!(
                "{title}: {:>3.0}% ({} hands/player, {} dealt, {:.0}s)",
                done.min(1.0) * 100.0,
                p.hands_per_player,
                p.hands_dealt,
                started.elapsed().as_secs_f64()
            );
            while next_report <= done {
                next_report += 0.1;
            }
        }
    })
    .map_err(|e| format!("{title} failed: {e}"))?;
    print_results(title, &result, started.elapsed().as_secs_f64());
    Ok(result)
}

fn run() -> Result<(), String> {
    let Some(o) = parse_args()? else {
        return Ok(());
    };
    let mut entries: Vec<Entry> = o.players.iter().map(|&p| Entry::Personality(p)).collect();
    entries.extend(
        o.external
            .iter()
            .map(|(name, command)| Entry::External(name.clone(), command.clone())),
    );
    if entries.len() < 2 {
        return Err("need at least 2 players".into());
    }

    let rules = match o.game {
        Variant::Holdem => TableRules::no_limit_holdem(o.blinds.0, o.blinds.1),
        Variant::Omaha { .. } => TableRules::pot_limit_omaha(o.blinds.0, o.blinds.1),
    };
    let config = ArenaConfig {
        rules,
        table_size: o.table_size,
        hands_per_player: o.hands,
        reseat_every: o.reseat,
        starting_bb: o.stack,
        seed: o.seed,
    };
    println!(
        "{} players, {}-handed tables, reseating every {} hands, {} bb stacks reset every hand, {} hands per player",
        entries.len(),
        o.table_size,
        o.reseat,
        o.stack,
        o.hands
    );
    let result = session("Session", &config, &entries, &o)?;
    if let Some(path) = &o.csv {
        write_csv(path, &result).map_err(|e| format!("couldn't write {path}: {e}"))?;
        println!("  wrote {path}");
    }

    if o.finals >= 2 {
        let top: Vec<Entry> = result
            .players
            .iter()
            .take(o.finals)
            .filter_map(|p| entries.iter().find(|e| e.name() == p.name).cloned())
            .collect();
        let finals = ArenaConfig {
            // Everyone in the finals sits at one table.
            table_size: top.len().clamp(2, 10),
            hands_per_player: o.final_hands.unwrap_or(o.hands),
            seed: o.seed.wrapping_add(1),
            ..config
        };
        let result = session("Finals", &finals, &top, &o)?;
        if let Some(path) = &o.csv {
            let path = match path.rsplit_once('.') {
                Some((stem, ext)) => format!("{stem}.finals.{ext}"),
                None => format!("{path}.finals"),
            };
            write_csv(&path, &result).map_err(|e| format!("couldn't write {path}: {e}"))?;
            println!("  wrote {path}");
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
