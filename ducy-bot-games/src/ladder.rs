//! The heads-up ladder (ducy-web #212), Train's Punch-Out: play the bots one
//! at a time, easiest first, up to the GTO bot at the top. Each rung is a
//! best-of-3 series of heads-up freezeouts: both players buy in for
//! [`START_BB`] big blinds, no rebuys. Every [`STAKES_EVERY`] hands the bot
//! offers to double the blinds.
//!
//! Your chips carry over from game to game: you start with two of the first
//! rung's buy-ins, each game won adds a buy-in and each game lost takes one,
//! and every rung up doubles the stakes. Win two games to move up a rung.
//! Lose the series, or be left unable to buy in for the next game, and you
//! drop to the highest rung below whose buy-in you can pay; when you can't
//! pay any, you start over at the bottom with the starting chips.
//!
//! [`Progress`] is the page's saved record (kept as its JSON, with whatever
//! else the page keeps in it); the page plays the games.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Each game's buy-in, in big blinds.
pub const START_BB: i64 = 200;
/// Games to win (or lose) a series: best of 3.
pub const WINS_TO_TAKE: u32 = 2;
/// The first rung's blinds, in chips (hundredths: 100/200 shows as 1/2).
/// Each rung up doubles them.
pub const BLINDS: [i64; 2] = [100, 200];
/// Hands between offers to double the blinds.
pub const STAKES_EVERY: u32 = 100;

/// The bots, easiest to hardest: by how much the GTO bot wins from each,
/// heads-up No-limit Hold'em at 200 BB, 40,000 duplicate hands each
/// (ducy-gto's gto_match --stack 200 against the gto-hunl-300m model). From
/// Jonny Slow up most are inside the others' intervals; they're in measured
/// order, except Doug Poker stays at the top, where he was chosen to be.
pub const ORDER: [&str; 20] = [
    "rampart",
    "nik_airbag",
    "milk_king",
    "bungleman",
    "jonny_slow",
    "bart_handsome",
    "chris_moneybags",
    "jen_silly",
    "brad_owned",
    "mister_cheating",
    "phil_bigmouth",
    "tom_collins",
    "danny_smallball",
    "andrew_favorable",
    "robin_jane_lewd",
    "michael_miserable",
    "the_mathematician",
    "tommy_sweeden",
    "gus_bluffsen",
    "doug_poker",
];
/// The bot at the top.
pub const CHAMPION: &str = "gto";

/// A rung's blinds.
pub fn blinds_at(level: i64) -> [i64; 2] {
    let k = level.clamp(0, 40) as u32;
    BLINDS.map(|b| b << k)
}

/// A rung's buy-in for each game.
pub fn buy_in_at(level: i64) -> i64 {
    START_BB * blinds_at(level)[1]
}

/// The chips you start the ladder with, and start over with: two first-rung
/// buy-ins.
pub fn start_chips() -> i64 {
    2 * buy_in_at(0)
}

/// The chips at the table in a game at this rung: both players' buy-ins.
pub fn table_chips_at(level: i64) -> i64 {
    2 * buy_in_at(level)
}

/// Your record against one bot: series won and lost.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// Series won.
    #[serde(default)]
    pub won: u32,
    /// Series lost.
    #[serde(default)]
    pub lost: u32,
}

/// The game under way, kept so it can be left and resumed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Game {
    /// Your chips at the table and theirs.
    pub stacks: [i64; 2],
    /// Hands played.
    #[serde(default)]
    pub hands: u32,
    /// The blinds now (a game kept before the stakes could rise has none: its
    /// rung's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blinds: Option<[i64; 2]>,
    /// The hand count at which the bot next offers to raise the stakes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offer_at: Option<u32>,
    /// What else the page keeps with it (its review).
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Your progress up the ladder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    /// The save's version: 2.
    #[serde(default = "two")]
    pub v: u32,
    /// Your rung.
    #[serde(default)]
    pub level: i64,
    /// The highest rung you've reached.
    #[serde(default)]
    pub best: i64,
    /// Games won in the series under way.
    #[serde(default)]
    pub wins: u32,
    /// Games lost in it.
    #[serde(default)]
    pub losses: u32,
    /// Your chips away from the table: a game under way has a buy-in of them
    /// at the table.
    #[serde(default = "start_chips")]
    pub chips: i64,
    /// A game is under way.
    #[serde(default)]
    pub playing: bool,
    /// Times you've beaten the top bot.
    #[serde(default)]
    pub titles: u32,
    /// Your record against each bot.
    #[serde(default)]
    pub record: BTreeMap<String, Record>,
    /// The game under way, or none.
    #[serde(default)]
    pub game: Option<Game>,
    /// What else the page keeps (the last game's review).
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn two() -> u32 {
    2
}

impl Default for Progress {
    fn default() -> Self {
        Progress {
            v: 2,
            level: 0,
            best: 0,
            wins: 0,
            losses: 0,
            chips: start_chips(),
            playing: false,
            titles: 0,
            record: BTreeMap::new(),
            game: None,
            extra: Map::new(),
        }
    }
}

/// The rungs, bottom to top: the ranked bots this engine has, then any it
/// has that aren't ranked yet, then the GTO bot when it's there.
pub fn rungs_for(ids: &[String], gto: bool) -> Vec<String> {
    let mut rungs: Vec<String> = ORDER
        .iter()
        .filter(|id| ids.iter().any(|x| x == *id))
        .map(|id| id.to_string())
        .collect();
    for id in ids {
        if !rungs.contains(id) && id != CHAMPION {
            rungs.push(id.clone());
        }
    }
    if gto {
        rungs.push(CHAMPION.to_string());
    }
    rungs
}

/// Your rung, kept on the ladder (it can get shorter: no GTO bot here).
pub fn level_of(p: &Progress, rungs: &[String]) -> i64 {
    p.level.min(rungs.len() as i64 - 1).max(0)
}

/// Your opponent.
pub fn opponent_of<'a>(p: &Progress, rungs: &'a [String]) -> Option<&'a str> {
    rungs.get(level_of(p, rungs) as usize).map(String::as_str)
}

/// A game was left and is waiting to be resumed.
pub fn paused_game(p: &Progress) -> Option<&Game> {
    if p.playing { p.game.as_ref() } else { None }
}

/// Whether rung `i`'s bot shows on the ladder. It goes by the rung, not the
/// bot, so when the bots are re-ranked a bot you've played that moves up
/// into a rung you haven't reached is hidden again, and whoever moves into a
/// rung you have reached shows: rungs below yours, and up to your best; your
/// own rung once you've been above it, its series is under way, or you've
/// played its bot before.
pub fn rung_shown(p: &Progress, rungs: &[String], i: i64) -> bool {
    let level = level_of(p, rungs);
    if i == level {
        let played = usize::try_from(i)
            .ok()
            .and_then(|i| rungs.get(i))
            .is_some_and(|id| p.record.contains_key(id));
        return i < p.best || played || p.wins + p.losses > 0 || paused_game(p).is_some();
    }
    i < level || i <= p.best
}

/// A game is starting, or resuming where it was left: a fresh one at your
/// rung's stakes, each player buying in.
pub fn start_game(p: &Progress, rungs: &[String]) -> Progress {
    let level = level_of(p, rungs);
    let game = match &p.game {
        Some(g) => Game {
            blinds: Some(g.blinds.unwrap_or(blinds_at(level))),
            offer_at: Some(g.offer_at.unwrap_or(STAKES_EVERY)),
            ..g.clone()
        },
        None => Game {
            stacks: [buy_in_at(level), buy_in_at(level)],
            hands: 0,
            blinds: Some(blinds_at(level)),
            offer_at: Some(STAKES_EVERY),
            extra: Map::new(),
        },
    };
    Progress {
        level,
        playing: true,
        game: Some(game),
        ..p.clone()
    }
}

/// The game so far, after each action: your chips behind, and theirs (all
/// the rest). Leaving in the middle of a hand folds it: what you'd put in the
/// pot is theirs. The game's blinds, next offer and review stay as they are.
pub fn keep_game(p: &Progress, mine: f64, hands: u32) -> Progress {
    let all = match &p.game {
        Some(g) => g.stacks[0] + g.stacks[1],
        None => table_chips_at(p.level),
    };
    let m = (mine.round() as i64).clamp(0, all.max(0));
    let g = p.game.clone().unwrap_or_default();
    Progress {
        game: Some(Game {
            stacks: [m, all - m],
            hands,
            ..g
        }),
        ..p.clone()
    }
}

/// The game's blinds now.
pub fn game_blinds(p: &Progress) -> [i64; 2] {
    p.game
        .as_ref()
        .and_then(|g| g.blinds)
        .unwrap_or(blinds_at(p.level))
}

/// The bot's offer to raise the stakes, once the game has reached its next
/// offer: the doubled blinds. Both players need more than the new big blind
/// behind, or there's nothing to raise.
pub fn stakes_offer(p: &Progress) -> Option<[i64; 2]> {
    let g = p.game.as_ref().filter(|_| p.playing)?;
    if g.hands < g.offer_at.unwrap_or(STAKES_EVERY) {
        return None;
    }
    let [sb, bb] = game_blinds(p);
    let next = [sb * 2, bb * 2];
    (g.stacks[0].min(g.stacks[1]) > next[1]).then_some(next)
}

/// Your answer to the offer: accepted, the blinds double from the next hand.
/// Either way the next offer is [`STAKES_EVERY`] hands on.
pub fn answer_stakes(p: &Progress, yes: bool) -> Progress {
    let offer = stakes_offer(p);
    let Some(g) = &p.game else { return p.clone() };
    let mut g = Game {
        offer_at: Some(g.hands + STAKES_EVERY),
        ..g.clone()
    };
    if let (true, Some(o)) = (yes, offer) {
        g.blinds = Some(o);
    }
    Progress {
        game: Some(g),
        ..p.clone()
    }
}

/// Your chips as they stand: with a game under way, the buy-in is what you
/// have at the table.
pub fn chips_now(p: &Progress) -> i64 {
    match &p.game {
        Some(g) => p.chips - (g.stacks[0] + g.stacks[1]) / 2 + g.stacks[0],
        None => p.chips,
    }
}

/// The rung you drop to with these chips after losing at `level`: the
/// highest one below whose buy-in you can pay, or -1 when you can't pay any.
pub fn drop_to(level: i64, chips: i64) -> i64 {
    (0..level)
        .rev()
        .find(|&j| buy_in_at(j) <= chips)
        .unwrap_or(-1)
}

/// What a finished game led to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    /// The series goes on.
    Game,
    /// Series won: on to the next rung.
    Up,
    /// Series won at the top.
    Champion,
    /// Series lost, or no chips left for the next game: down to the rung you
    /// can pay for.
    Down,
    /// Series lost on the bottom rung, with a buy-in left.
    Stay,
    /// No buy-in left for any rung: start over at the bottom.
    Bust,
}

/// A finished game's result.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Finish {
    /// The new progress.
    pub progress: Progress,
    /// What happened.
    pub outcome: Outcome,
    /// For a drop: the series ended because you couldn't buy in for the next
    /// game, not two games lost.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short: Option<bool>,
}

/// A game is over: `won` is whether you took all the chips, and with them a
/// buy-in from the bot (lost, your buy-in is gone).
pub fn finish_game(p: &Progress, rungs: &[String], won: bool) -> Finish {
    let level = level_of(p, rungs);
    let opp = rungs.get(level as usize).cloned().unwrap_or_default();
    let stake = match &p.game {
        Some(g) => (g.stacks[0] + g.stacks[1]) / 2,
        None => buy_in_at(level),
    };
    let mut q = Progress {
        level,
        playing: false,
        game: None,
        chips: p.chips + if won { stake } else { -stake },
        wins: p.wins + u32::from(won),
        losses: p.losses + u32::from(!won),
        ..p.clone()
    };
    let done = |progress, outcome| Finish {
        progress,
        outcome,
        short: None,
    };
    let broke = q.chips < buy_in_at(level);
    if q.wins < WINS_TO_TAKE && q.losses < WINS_TO_TAKE && !broke {
        return done(q, Outcome::Game);
    }
    let took = q.wins >= WINS_TO_TAKE;
    let r = q.record.entry(opp).or_default();
    if took {
        r.won += 1;
    } else {
        r.lost += 1;
    }
    q.wins = 0;
    q.losses = 0;
    if took {
        if level == rungs.len() as i64 - 1 {
            q.titles += 1;
            return done(q, Outcome::Champion);
        }
        q.level = level + 1;
        q.best = q.best.max(q.level);
        return done(q, Outcome::Up);
    }
    if level == 0 && !broke {
        return done(q, Outcome::Stay);
    }
    let short = Some(p.losses + 1 < WINS_TO_TAKE);
    let to = drop_to(level, q.chips);
    if to < 0 {
        q.level = 0;
        q.chips = start_chips();
        return Finish {
            progress: q,
            outcome: Outcome::Bust,
            short,
        };
    }
    q.level = to;
    Finish {
        progress: q,
        outcome: Outcome::Down,
        short,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const K: i64 = 100;

    fn rungs() -> Vec<String> {
        ["a", "b", "c", "d"].map(String::from).to_vec()
    }

    // Plays games: true is a game won.
    fn series(
        mut p: Progress,
        games: &[bool],
        rungs: &[String],
    ) -> (Progress, Outcome, Option<bool>) {
        let mut out = (Outcome::Game, None);
        for &won in games {
            let f = finish_game(&start_game(&p, rungs), rungs, won);
            p = f.progress;
            out = (f.outcome, f.short);
        }
        (p, out.0, out.1)
    }

    fn at(level: i64, best: i64, chips: i64) -> Progress {
        Progress {
            level,
            best,
            chips,
            ..Progress::default()
        }
    }

    #[test]
    fn the_ladder_runs_from_the_easiest_ranked_bot_to_the_gto_bot() {
        let order: Vec<String> = ORDER.map(String::from).to_vec();
        let mut rev = order.clone();
        rev.reverse();
        let mut with_gto = order.clone();
        with_gto.push(CHAMPION.into());
        assert_eq!(rungs_for(&rev, true), with_gto);
        assert_eq!(rungs_for(&order, false), order);
        let ids = [ORDER[1], "newbot", ORDER[0]].map(String::from);
        assert_eq!(
            rungs_for(&ids, true),
            [ORDER[0], ORDER[1], "newbot", CHAMPION].map(String::from)
        );
    }

    #[test]
    fn games_are_200_bb_deep_and_every_rung_up_doubles_the_stakes() {
        assert_eq!(blinds_at(0), [100, 200]);
        assert_eq!(blinds_at(1), [200, 400]);
        assert_eq!(blinds_at(3), [800, 1600]);
        assert_eq!(
            [buy_in_at(0), buy_in_at(1), buy_in_at(2)],
            [400 * K, 800 * K, 1600 * K]
        );
        assert_eq!(table_chips_at(1), 1600 * K);
        assert!(table_chips_at(ORDER.len() as i64 + 1) < 1 << 53);
    }

    #[test]
    fn you_start_with_800_and_a_sweep_doubles_it() {
        let r = rungs();
        let p = Progress::default();
        assert_eq!(p.chips, 800 * K);
        assert_eq!(opponent_of(&p, &r), Some("a"));
        assert_eq!(start_game(&p, &r).game.unwrap().stacks, [400 * K, 400 * K]);
        let f = finish_game(&start_game(&p, &r), &r, true);
        assert_eq!(f.outcome, Outcome::Game);
        let q = &f.progress;
        assert_eq!(
            (q.wins, q.losses, q.playing, q.chips),
            (1, 0, false, 1200 * K)
        );
        let f = finish_game(&start_game(&f.progress, &r), &r, true);
        assert_eq!(f.outcome, Outcome::Up);
        let p = f.progress;
        assert_eq!(
            (p.level, p.best, p.wins, p.losses, p.chips),
            (1, 1, 0, 0, 1600 * K)
        );
        assert_eq!(p.record["a"], Record { won: 1, lost: 0 });
        assert_eq!(opponent_of(&p, &r), Some("b"));
        assert_eq!(start_game(&p, &r).game.unwrap().stacks, [800 * K, 800 * K]);
    }

    #[test]
    fn a_sweep_leaves_room_to_lose_a_game_on_the_next_rung() {
        let r = rungs();
        let (p, _, _) = series(Progress::default(), &[true, true], &r);
        let (p, out, _) = series(p, &[false], &r);
        assert_eq!(out, Outcome::Game);
        assert_eq!((p.level, p.chips), (1, 800 * K));
        let (p, out, _) = series(p, &[true, true], &r);
        assert_eq!(out, Outcome::Up);
        assert_eq!((p.level, p.chips), (2, 2400 * K));
    }

    #[test]
    fn two_one_on_the_first_rung_you_must_win_the_first_game_of_the_next() {
        let r = rungs();
        let (p, out, _) = series(Progress::default(), &[true, false, true], &r);
        assert_eq!(out, Outcome::Up);
        assert_eq!((p.level, p.chips), (1, 1200 * K));
        let f = finish_game(&start_game(&p, &r), &r, false);
        assert_eq!(f.outcome, Outcome::Down);
        assert_eq!(f.short, Some(true), "lost for want of chips, not two games");
        let p = f.progress;
        assert_eq!((p.level, p.chips, p.wins, p.losses), (0, 400 * K, 0, 0));
        assert_eq!(p.record["b"], Record { won: 0, lost: 1 });
    }

    #[test]
    fn losing_drops_you_to_the_highest_rung_you_can_pay_for() {
        let r = rungs();
        let (p, _, _) = series(at(3, 3, 9600 * K), &[false], &r);
        let f = finish_game(&start_game(&p, &r), &r, false);
        assert_eq!(f.outcome, Outcome::Down);
        assert_eq!(f.short, Some(false));
        assert_eq!((f.progress.level, f.progress.chips), (2, 3200 * K));
        assert_eq!(drop_to(3, 900 * K), 1);
        assert_eq!(drop_to(3, 400 * K), 0);
        assert_eq!(drop_to(3, 399 * K), -1);
        let (p, out, _) = series(at(3, 0, 4100 * K), &[false], &r);
        assert_eq!(out, Outcome::Down);
        assert_eq!((p.level, p.chips), (1, 900 * K));
    }

    #[test]
    fn with_no_buy_in_left_for_any_rung_you_start_over_at_the_bottom() {
        let r = rungs();
        let (p, out, _) = series(Progress::default(), &[false, false], &r);
        assert_eq!(out, Outcome::Bust);
        assert_eq!((p.level, p.chips), (0, start_chips()));
        assert_eq!(p.record["a"], Record { won: 0, lost: 1 });
        let (p, out, _) = series(at(2, 2, 1600 * K), &[false], &r);
        assert_eq!(out, Outcome::Bust);
        assert_eq!((p.level, p.chips, p.best), (0, start_chips(), 2));
    }

    #[test]
    fn losing_a_series_on_the_bottom_rung_with_a_buy_in_left_keeps_you_there() {
        let (p, out, _) = series(Progress::default(), &[false, true, false], &rungs());
        assert_eq!(out, Outcome::Stay);
        assert_eq!((p.level, p.chips), (0, 400 * K));
    }

    #[test]
    fn beating_the_top_bot_makes_you_champion() {
        let r = rungs();
        let (p, out, _) = series(at(3, 3, 6400 * K), &[true, true], &r);
        assert_eq!(out, Outcome::Champion);
        assert_eq!((p.level, p.titles, p.chips), (3, 1, 12800 * K));
        let (p, out, _) = series(p, &[true, false, true], &r);
        assert_eq!((out, p.titles), (Outcome::Champion, 2));
    }

    #[test]
    fn a_game_left_in_the_middle_resumes_with_the_same_chips() {
        let r = rungs();
        let p = at(1, 0, 1600 * K);
        assert!(paused_game(&p).is_none());
        let p = start_game(&p, &r);
        let fresh = Game {
            stacks: [800 * K, 800 * K],
            hands: 0,
            blinds: Some(blinds_at(1)),
            offer_at: Some(STAKES_EVERY),
            extra: Map::new(),
        };
        assert_eq!(p.game, Some(fresh));
        let p = keep_game(&p, 106500.0, 12);
        assert_eq!(paused_game(&p).unwrap().stacks, [106500, 160000 - 106500]);
        assert_eq!(paused_game(&p).unwrap().hands, 12);
        assert_eq!(chips_now(&p), 1600 * K - 800 * K + 106500);
        assert_eq!(start_game(&p, &r).game, p.game);
        assert_eq!(p.losses, 0, "leaving isn't losing");
        let f = finish_game(&p, &r, true);
        assert!(f.progress.game.is_none());
        assert_eq!(f.progress.chips, 2400 * K);
    }

    #[test]
    fn a_shorter_ladder_keeps_you_on_it_at_its_top_rungs_stakes() {
        let r = rungs();
        let p = at(9, 0, 6400 * K);
        assert_eq!(opponent_of(&p, &r), Some("d"));
        assert_eq!(start_game(&p, &r).game.unwrap().stacks, [buy_in_at(3); 2]);
        let f = finish_game(&start_game(&p, &r), &r, false);
        assert_eq!(f.progress.losses, 1);
        assert_eq!((f.progress.level, f.progress.chips), (3, 3200 * K));
    }

    #[test]
    fn every_100_hands_the_bot_offers_to_double_the_blinds() {
        let r = rungs();
        let p = start_game(&Progress::default(), &r);
        assert_eq!(stakes_offer(&p), None);
        let p = keep_game(&p, 41000.0, STAKES_EVERY - 1);
        assert_eq!(stakes_offer(&p), None);
        let p = keep_game(&p, 41000.0, STAKES_EVERY);
        assert_eq!(stakes_offer(&p), Some([200, 400]));
        let p = answer_stakes(&p, true);
        assert_eq!(game_blinds(&p), [200, 400]);
        assert_eq!(stakes_offer(&p), None);
        let p = keep_game(&p, 30000.0, 2 * STAKES_EVERY);
        assert_eq!(stakes_offer(&p), Some([400, 800]));
        let p = answer_stakes(&p, false);
        assert_eq!(game_blinds(&p), [200, 400]);
        assert_eq!(
            stakes_offer(&keep_game(&p, 30000.0, 2 * STAKES_EVERY + 50)),
            None
        );
        assert_eq!(
            stakes_offer(&keep_game(&p, 30000.0, 3 * STAKES_EVERY)),
            Some([400, 800])
        );
        let paused = Progress {
            playing: false,
            ..p
        };
        assert_eq!(
            start_game(&paused, &r).game.unwrap().blinds,
            Some([200, 400])
        );
    }

    #[test]
    fn no_offer_when_someone_cant_cover_the_doubled_big_blind() {
        let p = keep_game(
            &start_game(&Progress::default(), &rungs()),
            350.0,
            STAKES_EVERY,
        );
        assert_eq!(stakes_offer(&p), None);
    }

    #[test]
    fn raising_the_blinds_on_a_higher_rung_keeps_its_buy_in_and_bankroll() {
        let r = rungs();
        let p = start_game(&at(2, 0, 6400 * K), &r);
        let p = keep_game(&p, (1700 * K) as f64, STAKES_EVERY);
        assert_eq!(stakes_offer(&p), Some([800, 1600]));
        let p = answer_stakes(&p, true);
        let p = keep_game(&p, (1800 * K) as f64, STAKES_EVERY + 1);
        assert_eq!(game_blinds(&p), [800, 1600]);
        let g = p.game.as_ref().unwrap();
        assert_eq!(g.offer_at, Some(2 * STAKES_EVERY));
        assert_eq!(g.stacks, [1800 * K, 1400 * K]);
        assert_eq!(chips_now(&p), 6600 * K);
        let saved: Progress = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(start_game(&saved, &r).game, p.game);
        assert_eq!(finish_game(&p, &r, true).progress.chips, 8000 * K);
        assert_eq!(finish_game(&p, &r, false).progress.chips, 4800 * K);
        let next = start_game(&finish_game(&p, &r, true).progress, &r);
        assert_eq!(game_blinds(&next), blinds_at(2));
        assert_eq!(next.game.unwrap().offer_at, Some(STAKES_EVERY));
    }

    #[test]
    fn a_game_kept_before_the_stakes_could_rise_goes_on_at_1_2() {
        let p: Progress = serde_json::from_value(json!({
            "v": 2, "level": 0, "chips": 80000, "playing": true,
            "game": {"stacks": [53000, 27000], "hands": 40},
        }))
        .unwrap();
        let p = start_game(&p, &rungs());
        assert_eq!(game_blinds(&p), BLINDS);
        assert_eq!(p.game.unwrap().offer_at, Some(STAKES_EVERY));
    }

    #[test]
    fn what_else_the_page_keeps_is_kept() {
        let r = rungs();
        let p: Progress = serde_json::from_value(json!({
            "v": 2, "level": 0, "best": 0, "wins": 0, "losses": 0, "chips": 80000, "playing": true,
            "titles": 0, "record": {},
            "game": {"stacks": [40000, 40000], "hands": 3, "review": {"totals": {"hands": 3}, "lessons": []}},
            "lastReview": {"at": 1, "opp": "a"},
        }))
        .unwrap();
        let q = answer_stakes(&keep_game(&start_game(&p, &r), 30000.0, 4), false);
        let v = serde_json::to_value(&q).unwrap();
        assert_eq!(v["game"]["review"]["totals"]["hands"], 3);
        assert_eq!(v["lastReview"]["opp"], "a");
        assert_eq!(v["game"]["offerAt"], 104);
        let f = serde_json::to_value(finish_game(&q, &r, true)).unwrap();
        assert_eq!(f["progress"]["lastReview"]["opp"], "a");
        assert_eq!(f["progress"]["game"], Value::Null);
        assert_eq!(f["outcome"], "game");
        assert!(f.get("short").is_none());
    }

    #[test]
    fn which_rungs_show_goes_by_the_rung() {
        let ids = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
        let rec = |names: &[(&str, u32, u32)]| {
            names
                .iter()
                .map(|&(n, won, lost)| (n.to_string(), Record { won, lost }))
                .collect::<BTreeMap<_, _>>()
        };
        let p = Progress {
            level: 2,
            best: 3,
            record: rec(&[("a", 1, 0), ("b", 1, 0), ("d", 0, 1), ("e", 0, 1)]),
            ..Progress::default()
        };
        let shown = |p: &Progress, r: &[String]| {
            (0..r.len() as i64)
                .map(|i| rung_shown(p, r, i))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            shown(&p, &ids("a b c d e f")),
            [true, true, true, true, false, false]
        );
        assert_eq!(
            shown(&p, &ids("a b c e f d")),
            [true, true, true, true, false, false]
        );
        assert!(rung_shown(&p, &ids("a z c d"), 1));
        let fresh = Progress {
            level: 1,
            best: 1,
            record: rec(&[("a", 1, 0)]),
            ..Progress::default()
        };
        assert!(!rung_shown(&fresh, &ids("a b c"), 1));
        assert!(rung_shown(
            &Progress {
                wins: 1,
                ..fresh.clone()
            },
            &ids("a b c"),
            1
        ));
        assert!(rung_shown(&fresh, &ids("b a c"), 1));
        assert!(!rung_shown(&fresh, &ids("a b c"), 2));
    }
}
