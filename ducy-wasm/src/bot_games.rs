//! The games against the bots (ducy-bot-games, ducy-web #465): the heads-up
//! ladder, the Regulars' table, hand reviews and the bots' table talk.
//!
//! `botGames(op)` takes an op (`{op, …}`) and returns its value. Random draws
//! come in from the page (`r`, or `draws`: numbers in `0..1`).

use ducy_bot_games::chat;
use ducy_bot_games::ladder::{self, Progress};
use ducy_bot_games::regulars::{self, Table};
use ducy_bot_games::review::{self, GameReview, Lesson, Totals};
use serde::Deserialize;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

use crate::util::{err, to_js};

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Op {
    LadderRules,
    RungsFor {
        ids: Vec<String>,
        #[serde(default)]
        gto: bool,
    },
    RungShown {
        p: Progress,
        rungs: Vec<String>,
        i: i64,
    },
    StartGame {
        p: Progress,
        rungs: Vec<String>,
    },
    KeepGame {
        p: Progress,
        mine: f64,
        hands: u32,
    },
    StakesOffer {
        p: Progress,
    },
    AnswerStakes {
        p: Progress,
        yes: bool,
    },
    ChipsNow {
        p: Progress,
    },
    DropTo {
        level: i64,
        chips: i64,
    },
    FinishGame {
        p: Progress,
        rungs: Vec<String>,
        won: bool,
    },
    StartBank {
        bots: usize,
        buy_in: i64,
        #[serde(default)]
        buy_ins: Option<Vec<i64>>,
    },
    PickRegulars {
        ids: Vec<String>,
        n: usize,
        draws: Vec<f64>,
    },
    BetweenHands {
        #[serde(flatten)]
        table: Table,
        draws: Vec<f64>,
    },
    AddReview {
        #[serde(default)]
        totals: Totals,
        review: Value,
    },
    AddTotals {
        #[serde(default)]
        a: Totals,
        #[serde(default)]
        b: Totals,
    },
    MixCost {
        d: Value,
    },
    IsVariant {
        d: Value,
    },
    WorstIndex {
        review: Value,
    },
    Per100 {
        #[serde(default)]
        t: Totals,
    },
    AddHand {
        #[serde(default)]
        g: Option<GameReview>,
        review: Value,
        hand: u32,
    },
    LessonsByHand {
        #[serde(default)]
        lessons: Option<Vec<Lesson>>,
    },
    LessonsShare {
        #[serde(default)]
        g: Option<GameReview>,
    },
    PersonaOf {
        id: String,
    },
    Speaks {
        id: String,
        moment: String,
        #[serde(default)]
        heads_up: bool,
        #[serde(default)]
        ladder: bool,
        r: f64,
    },
    LinesFor {
        id: String,
        moment: String,
        #[serde(default)]
        how: Option<String>,
    },
    LineFor {
        id: String,
        moment: String,
        #[serde(default)]
        how: Option<String>,
        #[serde(default)]
        mine: Vec<String>,
        #[serde(default)]
        table: Vec<String>,
        r: f64,
    },
    ReplyMoment {
        text: String,
    },
    CallShape {
        #[serde(default)]
        events: Option<Vec<Value>>,
        seat: i64,
    },
    HandMoments {
        seats: Vec<Value>,
        big_blind: f64,
        #[serde(default)]
        events: Option<Vec<Value>>,
    },
}

/// The page's draws, in order (again from the start if they run out).
fn draws(d: Vec<f64>) -> impl FnMut() -> f64 {
    let mut i = 0;
    move || {
        let x = if d.is_empty() { 0.0 } else { d[i % d.len()] };
        i += 1;
        x
    }
}

/// Runs a bot-games op.
#[wasm_bindgen(js_name = botGames)]
pub fn bot_games(op: JsValue) -> Result<JsValue, JsError> {
    let op: Op = serde_wasm_bindgen::from_value(op).map_err(err)?;
    let v = match op {
        Op::LadderRules => json!({
            "order": ladder::ORDER, "champion": ladder::CHAMPION, "startBb": ladder::START_BB,
            "winsToTake": ladder::WINS_TO_TAKE, "blinds": ladder::BLINDS, "stakesEvery": ladder::STAKES_EVERY,
            "bankrollBuyIns": regulars::BANKROLL_BUY_INS, "leaveChance": regulars::LEAVE_CHANCE,
            "regularsSeats": regulars::REGULARS_SEATS, "lessonsKept": review::LESSONS_KEPT,
        }),
        Op::RungsFor { ids, gto } => json!(ladder::rungs_for(&ids, gto)),
        Op::RungShown { p, rungs, i } => json!(ladder::rung_shown(&p, &rungs, i)),
        Op::StartGame { p, rungs } => json!(ladder::start_game(&p, &rungs)),
        Op::KeepGame { p, mine, hands } => json!(ladder::keep_game(&p, mine, hands)),
        Op::StakesOffer { p } => json!(ladder::stakes_offer(&p)),
        Op::AnswerStakes { p, yes } => json!(ladder::answer_stakes(&p, yes)),
        Op::ChipsNow { p } => json!(ladder::chips_now(&p)),
        Op::DropTo { level, chips } => json!(ladder::drop_to(level, chips)),
        Op::FinishGame { p, rungs, won } => json!(ladder::finish_game(&p, &rungs, won)),
        Op::StartBank {
            bots,
            buy_in,
            buy_ins,
        } => json!(regulars::start_bank(bots, buy_in, buy_ins.as_deref())),
        Op::PickRegulars { ids, n, draws: d } => {
            json!(regulars::pick_regulars(&ids, n, &mut draws(d)))
        }
        Op::BetweenHands { table, draws: d } => {
            json!(regulars::between_hands(&table, &mut draws(d)))
        }
        Op::AddReview { mut totals, review } => {
            totals.add_review(&review);
            json!(totals)
        }
        Op::AddTotals { mut a, b } => {
            a.add(&b);
            json!(a)
        }
        Op::MixCost { d } => json!(review::mix_cost(&d)),
        Op::IsVariant { d } => json!(review::is_variant(&d)),
        Op::WorstIndex { review } => json!(review::worst_index(&review).map_or(-1, |i| i as i64)),
        Op::Per100 { t } => json!(t.per100()),
        Op::AddHand { g, review, hand } => json!(review::add_hand(g, &review, hand)),
        Op::LessonsByHand { lessons } => {
            json!(review::lessons_by_hand(&lessons.unwrap_or_default()))
        }
        Op::LessonsShare { g } => json!(review::lessons_share(g.as_ref())),
        Op::PersonaOf { id } => json!(chat::persona_of(&id)),
        Op::Speaks {
            id,
            moment,
            heads_up,
            ladder,
            r,
        } => json!(chat::speaks(&id, &moment, heads_up, ladder, r)),
        Op::LinesFor { id, moment, how } => json!(chat::lines_for(&id, &moment, how.as_deref())),
        Op::LineFor {
            id,
            moment,
            how,
            mine,
            table,
            r,
        } => json!(chat::line_for(
            &id,
            &moment,
            how.as_deref(),
            &mine,
            &table,
            r
        )),
        Op::ReplyMoment { text } => json!(chat::reply_moment(&text)),
        Op::CallShape { events, seat } => {
            json!(chat::call_shape(events.as_deref().unwrap_or(&[]), seat))
        }
        Op::HandMoments {
            seats,
            big_blind,
            events,
        } => json!(chat::hand_moments(&seats, big_blind, events.as_deref())),
    };
    to_js(&v)
}
