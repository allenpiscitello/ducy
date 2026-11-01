//! Bankroll Quest (ducy-quest, ducy-web #466): the seeded world of card
//! rooms, and the quest itself.
//!
//! `quest(op)` takes an op (`{op, …}`) and returns its value. A quest goes in
//! and comes back as the page's save (`q`); rooms are their ids, or for buy-in
//! limits a room's `{minBuyIn, maxBuyIn}`; real time `now` is milliseconds.
//! An op that may leave the quest as it was returns `null` when it does.

use ducy_quest::lineup::{self, Lineup};
use ducy_quest::quest::{self as core, ListGame, Quest, RunRecord, Session, TableChoice};
use ducy_quest::reads::{self, Rates, SatWith};
use ducy_quest::scenarios::{self, Message, Played};
use ducy_quest::world::{self, Limits};
use ducy_quest::{backer, people, stats, study, waitlist};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

use crate::util::{err, to_js};

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Op {
    // ---- The world ----
    Roll {
        seed: u64,
        keys: String,
    },
    Activity {
        day: i64,
        hour: i64,
    },
    TablesRunning {
        seed: u64,
        room: String,
        stakes: String,
        #[serde(default)]
        game: Option<String>,
        minutes: f64,
    },
    Lobby {
        seed: u64,
        minutes: f64,
    },
    CheapestBuyIn {
        #[serde(default = "forty")]
        min_bb: f64,
    },
    ListsNow {
        seed: u64,
        minutes: f64,
        #[serde(default)]
        home: Option<String>,
    },
    WaitingAt {
        seed: u64,
        room: String,
        game: String,
        stakes: String,
        minutes: f64,
    },
    SeatGap {
        tables: f64,
        #[serde(default)]
        home: bool,
        stakes: String,
    },
    Roster {
        seed: u64,
        room: String,
    },
    IsReg {
        seed: u64,
        rooms: Vec<String>,
        id: String,
    },
    Tourist {
        seed: u64,
        room: String,
        day: i64,
        slot: i64,
        minutes: f64,
        #[serde(default)]
        family: Option<String>,
    },
    LineupFor {
        seed: u64,
        table: String,
        stakes: String,
        minutes: f64,
        #[serde(default)]
        have: Option<Vec<String>>,
        #[serde(default)]
        generic: bool,
    },
    RoomLineups {
        seed: u64,
        room: String,
        minutes: f64,
        #[serde(default)]
        have: Option<Vec<String>>,
    },
    BuyInFor {
        seed: u64,
        table: String,
        id: String,
        blinds: [i64; 2],
        #[serde(default)]
        room: Limits,
        minutes: f64,
    },
    LineupStacks {
        seed: u64,
        table: String,
        ids: Vec<String>,
        blinds: [i64; 2],
        #[serde(default)]
        room: Limits,
        minutes: f64,
    },
    SeatedHours {
        u: f64,
        v: f64,
    },
    ReplacementPool {
        stakes: String,
        #[serde(default)]
        have: Option<Vec<String>>,
        #[serde(default)]
        seed: u64,
        #[serde(default)]
        table: Option<String>,
        #[serde(default)]
        minutes: f64,
        #[serde(default)]
        generic: bool,
    },
    // ---- The quest ----
    Rules,
    NewQuest {
        now: f64,
        seed: u64,
    },
    GameMinutes {
        q: Quest,
        now: f64,
    },
    SyncClock {
        q: Quest,
        now: f64,
    },
    AwayUntil {
        q: Quest,
    },
    Resume {
        q: Quest,
        now: f64,
    },
    SpendMinutes {
        q: Quest,
        minutes: f64,
        now: f64,
    },
    ClockText {
        minutes: f64,
    },
    BuyInRange {
        stakes: [f64; 2],
        #[serde(default)]
        room: Limits,
    },
    CanSit {
        q: Quest,
        stakes: [f64; 2],
        #[serde(default)]
        room: Limits,
    },
    SitDown {
        q: Quest,
        table: TableChoice,
        amount: f64,
        #[serde(default)]
        room: Limits,
        now: f64,
    },
    SetStack {
        q: Quest,
        stack: f64,
    },
    RecordHand {
        q: Quest,
        stack: f64,
    },
    TopUp {
        q: Quest,
        to: f64,
        #[serde(default)]
        room: Limits,
    },
    TopUpChoices {
        q: Quest,
        stack: f64,
        #[serde(default)]
        room: Limits,
    },
    Leave {
        q: Quest,
        now: f64,
    },
    TableMinutes {
        q: Quest,
        now: f64,
    },
    Exhausted {
        q: Quest,
        now: f64,
    },
    Sleep {
        q: Quest,
        place: String,
        #[serde(default)]
        cost: f64,
        now: f64,
    },
    Wake {
        q: Quest,
        now: f64,
    },
    MoveHome {
        q: Quest,
        #[serde(default)]
        city: Option<String>,
        cost: f64,
    },
    CheckMilestones {
        q: Quest,
    },
    Status {
        q: Quest,
        cheapest: f64,
    },
    StartIn {
        q: Quest,
        region: String,
        room: String,
        #[serde(default)]
        city: Option<String>,
    },
    Travel {
        q: Quest,
        room: String,
        minutes: f64,
        now: f64,
        #[serde(default)]
        cost: f64,
        #[serde(default)]
        then: Option<String>,
    },
    Arrive {
        q: Quest,
        now: f64,
    },
    JoinList {
        q: Quest,
        game: ListGame,
        ahead: f64,
        gap: f64,
        now: f64,
    },
    LeaveList {
        q: Quest,
        key: String,
    },
    ListStatus {
        q: Quest,
        now: f64,
    },
    DropMissed {
        q: Quest,
        now: f64,
    },
    SeeCalls {
        q: Quest,
        now: f64,
    },
    UnseenCalls {
        q: Quest,
        now: f64,
    },
    RecordStudy {
        q: Quest,
        right: bool,
    },
    RunRecord {
        q: Quest,
        now: f64,
        result: String,
    },
    NewRun {
        q: Quest,
        now: f64,
        result: String,
    },
    // ---- Study and stats ----
    Question {
        seed: u64,
        n: u64,
    },
    ByStakes {
        history: Vec<Session>,
    },
    BiggestPots {
        history: Vec<Session>,
    },
    WithRegulars {
        history: Vec<Session>,
    },
    BankrollSeries {
        history: Vec<Session>,
    },
    HallOrder {
        runs: Vec<RunRecord>,
    },
    DurationText {
        minutes: f64,
    },
    // ---- Reads ----
    TagPlayer {
        q: Quest,
        id: String,
        tag: String,
        #[serde(default)]
        hands: f64,
    },
    SuggestFamily {
        #[serde(default)]
        r: Option<Rates>,
        #[serde(default = "fifteen")]
        min_hands: f64,
    },
    ReviewReads {
        q: Quest,
        people: Vec<SatWith>,
    },
    AccuracyText {
        #[serde(default)]
        s: Option<reads::Tally>,
    },
    ThinkMs {
        family: String,
        a: f64,
        b: f64,
    },
    // ---- Scenarios ----
    Post {
        q: Quest,
        msg: Message,
    },
    MarkRead {
        q: Quest,
    },
    OpenOffers {
        q: Quest,
        minutes: f64,
    },
    StakingTotals {
        q: Quest,
    },
    SkillOf {
        id: String,
    },
    HonestyOf {
        id: String,
        #[serde(default)]
        reg: bool,
    },
    MaybeAskStake {
        q: Quest,
        people: Vec<Played>,
        stakes: [f64; 2],
        minutes: f64,
        #[serde(default)]
        regulars: Vec<String>,
        #[serde(default)]
        n: f64,
    },
    AnswerStake {
        q: Quest,
        msg_id: String,
        answer: String,
        minutes: f64,
    },
    StakeOutcome {
        seed: u64,
        stake: scenarios::Stake,
    },
    ResolveStakes {
        q: Quest,
        minutes: f64,
    },
    OwedBy {
        q: Quest,
        id: String,
    },
    Confront {
        q: Quest,
        id: String,
        minutes: f64,
    },
    // ---- Backers ----
    PassMark {
        q: Quest,
        mastered: f64,
    },
    CutFor {
        pct: f64,
        mark: f64,
        #[serde(default)]
        offer_cut: Option<f64>,
    },
    StakesUp {
        stakes: [f64; 2],
    },
    MaybeBackerOffer {
        q: Quest,
        stakes: [f64; 2],
        minutes: f64,
        #[serde(default)]
        n: f64,
    },
    FinishTest {
        q: Quest,
        msg_id: String,
        score: backer::Score,
        mastered: f64,
        minutes: f64,
    },
    OwedToBacker {
        q: Quest,
    },
    SettleBacking {
        q: Quest,
        minutes: f64,
    },
    AnswerSettle {
        q: Quest,
        msg_id: String,
        answer: String,
        minutes: f64,
    },
}

fn fifteen() -> f64 {
    15.0
}
fn forty() -> f64 {
    40.0
}

// A value for the page: serialized straight (an unseen call's `expires` is
// Infinity, which JSON would lose).
fn out(v: &impl Serialize) -> Result<JsValue, JsError> {
    to_js(v)
}

// A quest after an op that may leave it as it was: `null` then, so the page
// keeps the quest it passed in (it checks `next === q`).
fn kept(before: &Quest, after: Quest) -> Result<JsValue, JsError> {
    if *before == after {
        Ok(JsValue::NULL)
    } else {
        out(&after)
    }
}

fn lineups(l: Vec<(String, Lineup)>) -> Value {
    json!(
        l.into_iter()
            .map(|(id, t)| json!([id, t]))
            .collect::<Vec<_>>()
    )
}

/// Runs a Bankroll Quest op.
#[wasm_bindgen(js_name = quest)]
pub fn quest(op: JsValue) -> Result<JsValue, JsError> {
    let op: Op = serde_wasm_bindgen::from_value(op).map_err(err)?;
    match op {
        Op::Roll { seed, keys } => out(&ducy_quest::dice::roll_str(seed, &keys)),
        Op::Activity { day, hour } => out(&world::activity(day, hour)),
        Op::TablesRunning {
            seed,
            room,
            stakes,
            game,
            minutes,
        } => {
            let game = game.unwrap_or_else(|| "nlhe".into());
            let r =
                world::room_by_id(&room).ok_or_else(|| JsError::new(&format!("no room {room}")))?;
            let g = r
                .games
                .iter()
                .find(|g| g.stakes == stakes && g.game == game)
                .ok_or_else(|| JsError::new(&format!("no {game} {stakes} at {room}")))?;
            out(&world::tables_running(seed, r, g, minutes))
        }
        Op::Lobby { seed, minutes } => out(&world::lobby(seed, minutes)),
        Op::CheapestBuyIn { min_bb } => out(&world::cheapest_buy_in(min_bb)),
        Op::ListsNow {
            seed,
            minutes,
            home,
        } => out(&waitlist::lists_now(seed, minutes, home.as_deref())),
        Op::WaitingAt {
            seed,
            room,
            game,
            stakes,
            minutes,
        } => out(&waitlist::waiting_at(seed, &room, &game, &stakes, minutes)),
        Op::SeatGap {
            tables,
            home,
            stakes,
        } => out(&waitlist::seat_gap(tables, home, &stakes)),
        Op::Roster { seed, room } => out(&people::roster(seed, &room)),
        Op::IsReg { seed, rooms, id } => out(&people::is_reg(seed, &rooms, &id)),
        Op::Tourist {
            seed,
            room,
            day,
            slot,
            minutes,
            family,
        } => out(&people::tourist(
            seed,
            &room,
            day,
            slot,
            minutes,
            family.as_deref(),
        )),
        Op::LineupFor {
            seed,
            table,
            stakes,
            minutes,
            have,
            generic,
        } => out(&lineup::lineup_for(
            seed,
            &table,
            &stakes,
            minutes,
            have.as_deref(),
            generic,
        )),
        Op::RoomLineups {
            seed,
            room,
            minutes,
            have,
        } => out(&lineups(lineup::room_lineups(
            seed,
            &room,
            minutes,
            have.as_deref(),
        ))),
        Op::BuyInFor {
            seed,
            table,
            id,
            blinds,
            room,
            minutes,
        } => out(&lineup::buy_in_for(
            seed, &table, &id, blinds, &room, minutes,
        )),
        Op::LineupStacks {
            seed,
            table,
            ids,
            blinds,
            room,
            minutes,
        } => out(&lineup::lineup_stacks(
            seed, &table, &ids, blinds, &room, minutes,
        )),
        Op::SeatedHours { u, v } => out(&lineup::seated_hours(u, v)),
        Op::ReplacementPool {
            stakes,
            have,
            seed,
            table,
            minutes,
            generic,
        } => out(&lineup::replacement_pool(
            &stakes,
            have.as_deref(),
            seed,
            table.as_deref(),
            minutes,
            generic,
        )),
        Op::Rules => out(&json!({
            "startBankroll": core::START_BANKROLL, "goal": core::GOAL, "questSpeed": core::QUEST_SPEED,
            "awayCapDays": core::AWAY_CAP_DAYS, "minBuyInBb": core::MIN_BUY_IN_BB, "maxBuyInBb": core::MAX_BUY_IN_BB,
            "tableIdleMs": core::TABLE_IDLE_MS, "awayMs": core::AWAY_MS, "maxTableMinutes": core::MAX_TABLE_MINUTES,
            "tiredMinutes": core::TIRED_MINUTES, "sleepMinutes": core::SLEEP_MINUTES, "callMinutes": core::CALL_MINUTES,
            "milestones": core::MILESTONES, "tags": reads::TAGS, "readHands": reads::READ_HANDS, "never": backer::NEVER,
            "cheapest": world::cheapest_buy_in(core::MIN_BUY_IN_BB),
            "rooms": world::world().rooms, "stakes": world::world().stakes,
        })),
        Op::NewQuest { now, seed } => out(&core::new_quest(now, seed)),
        Op::GameMinutes { q, now } => out(&core::game_minutes(&q, now)),
        Op::SyncClock { q, now } => out(&core::sync_clock(&q, now)),
        Op::AwayUntil { q } => out(&core::away_until(&q)),
        Op::Resume { q, now } => out(&core::resume(&q, now)),
        Op::SpendMinutes { q, minutes, now } => out(&core::spend_minutes(&q, minutes, now)),
        Op::ClockText { minutes } => out(&core::clock_text(minutes)),
        Op::BuyInRange { stakes, room } => {
            let (min, max) = core::buy_in_range(stakes, &room);
            out(&json!({"min": min, "max": max}))
        }
        Op::CanSit { q, stakes, room } => out(&core::can_sit(&q, stakes, &room)),
        Op::SitDown {
            q,
            table,
            amount,
            room,
            now,
        } => out(&core::sit_down(&q, &table, amount, &room, now)),
        Op::SetStack { q, stack } => kept(&q, core::set_stack(&q, stack)),
        Op::RecordHand { q, stack } => kept(&q, core::record_hand(&q, stack)),
        Op::TopUp { q, to, room } => kept(&q, core::top_up(&q, to, &room)),
        Op::TopUpChoices { q, stack, room } => out(&core::top_up_choices(&q, stack, &room)),
        Op::Leave { q, now } => kept(&q, core::leave(&q, now)),
        Op::TableMinutes { q, now } => out(&core::table_minutes(&q, now)),
        Op::Exhausted { q, now } => out(&core::exhausted(&q, now)),
        Op::Sleep {
            q,
            place,
            cost,
            now,
        } => kept(&q, core::sleep(&q, &place, cost, now)),
        Op::Wake { q, now } => kept(&q, core::wake(&q, now)),
        Op::MoveHome { q, city, cost } => {
            kept(&q, core::move_home(&q, city.as_deref().unwrap_or(""), cost))
        }
        Op::CheckMilestones { q } => {
            let (q, reached) = core::check_milestones(&q);
            out(&json!({"q": q, "reached": reached}))
        }
        Op::Status { q, cheapest } => out(&core::status(&q, cheapest)),
        Op::StartIn {
            q,
            region,
            room,
            city,
        } => kept(&q, core::start_in(&q, &region, &room, city.as_deref())),
        Op::Travel {
            q,
            room,
            minutes,
            now,
            cost,
            then,
        } => kept(
            &q,
            core::travel(&q, &room, minutes, now, cost, then.as_deref()),
        ),
        Op::Arrive { q, now } => kept(&q, core::arrive(&q, now)),
        Op::JoinList {
            q,
            game,
            ahead,
            gap,
            now,
        } => kept(&q, core::join_list(&q, &game, ahead, gap, now)),
        Op::LeaveList { q, key } => kept(&q, core::leave_list(&q, &key)),
        Op::ListStatus { q, now } => out(&core::list_status(&q, now)),
        Op::DropMissed { q, now } => kept(&q, core::drop_missed(&q, now)),
        Op::SeeCalls { q, now } => kept(&q, core::see_calls(&q, now)),
        Op::UnseenCalls { q, now } => out(&core::unseen_calls(&q, now)),
        Op::RecordStudy { q, right } => out(&core::record_study(&q, right)),
        Op::RunRecord { q, now, result } => out(&core::run_record(&q, now, &result)),
        Op::NewRun { q, now, result } => out(&core::new_run(&q, now, &result)),
        Op::Question { seed, n } => out(&study::question(seed, n)),
        Op::ByStakes { history } => out(&stats::by_stakes(&history)),
        Op::BiggestPots { history } => {
            let (won, lost) = stats::biggest_pots(&history);
            out(&json!({"won": won, "lost": lost}))
        }
        Op::WithRegulars { history } => out(&stats::with_regulars(&history)),
        Op::BankrollSeries { history } => out(&stats::bankroll_series(&history)),
        Op::HallOrder { runs } => out(&stats::hall_order(&runs)),
        Op::DurationText { minutes } => out(&stats::duration_text(minutes)),
        Op::TagPlayer { q, id, tag, hands } => kept(&q, reads::tag_player(&q, &id, &tag, hands)),
        Op::SuggestFamily { r, min_hands } => out(&reads::suggest_family(r.as_ref(), min_hands)),
        Op::ReviewReads { q, people } => {
            let (rows, q) = reads::review_reads(&q, &people);
            out(&json!({"rows": rows, "q": q}))
        }
        Op::AccuracyText { s } => out(&reads::accuracy_text(s.as_ref())),
        Op::ThinkMs { family, a, b } => out(&reads::think_ms(&family, a, b)),
        Op::Post { q, msg } => out(&scenarios::post(&q, msg)),
        Op::MarkRead { q } => kept(&q, scenarios::mark_read(&q)),
        Op::OpenOffers { q, minutes } => out(&scenarios::open_offers(&q, minutes)),
        Op::StakingTotals { q } => out(&scenarios::staking_totals(&q)),
        Op::SkillOf { id } => out(&scenarios::skill_of(&id)),
        Op::HonestyOf { id, reg } => out(&scenarios::honesty_of(&id, reg)),
        Op::MaybeAskStake {
            q,
            people,
            stakes,
            minutes,
            regulars,
            n,
        } => kept(
            &q,
            scenarios::maybe_ask_stake(
                &q,
                &people,
                stakes,
                minutes,
                &|id| regulars.iter().any(|r| r == id),
                n,
            ),
        ),
        Op::AnswerStake {
            q,
            msg_id,
            answer,
            minutes,
        } => kept(&q, scenarios::answer_stake(&q, &msg_id, &answer, minutes)),
        Op::StakeOutcome { seed, stake } => out(&scenarios::stake_outcome(seed, &stake)),
        Op::ResolveStakes { q, minutes } => kept(&q, scenarios::resolve_stakes(&q, minutes)),
        Op::OwedBy { q, id } => out(&scenarios::owed_by(&q, &id)),
        Op::Confront { q, id, minutes } => kept(&q, scenarios::confront(&q, &id, minutes)),
        Op::PassMark { q, mastered } => out(&backer::pass_mark(&q, mastered)),
        Op::CutFor {
            pct,
            mark,
            offer_cut,
        } => out(&backer::cut_for(
            pct,
            mark,
            offer_cut.unwrap_or(backer::CUT_AT_PASS),
        )),
        Op::StakesUp { stakes } => out(&backer::stakes_up(stakes)),
        Op::MaybeBackerOffer {
            q,
            stakes,
            minutes,
            n,
        } => kept(&q, backer::maybe_backer_offer(&q, stakes, minutes, n)),
        Op::FinishTest {
            q,
            msg_id,
            score,
            mastered,
            minutes,
        } => kept(
            &q,
            backer::finish_test(&q, &msg_id, score, mastered, minutes),
        ),
        Op::OwedToBacker { q } => {
            let (net, owed) = backer::owed_to_backer(&q);
            out(&json!({"net": net, "owed": owed}))
        }
        Op::SettleBacking { q, minutes } => kept(&q, backer::settle_backing(&q, minutes)),
        Op::AnswerSettle {
            q,
            msg_id,
            answer,
            minutes,
        } => kept(&q, backer::answer_settle(&q, &msg_id, &answer, minutes)),
    }
}
