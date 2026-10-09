//! A table where one person plays no-limit Hold'em or pot-limit Omaha (PLO4,
//! PLO5, PLO6) against ducy-play's personality bots.
//!
//! The page drives it one action at a time: [`BotTable::advance`] makes the
//! next bot act, and [`BotTable::act`] applies the person's action, so the UI
//! can show each decision as it happens. [`BotTable::state`] returns what the
//! person may see: their own cards, public chip counts, and other players'
//! cards only once they're shown down.
//!
//! [`MultiTable`] hosts the same kind of table for people on other devices:
//! the page passes it each player's messages and sends back what it returns.

use ducy_play::{
    Action, ChipRequest, Command, Departure, Outgoing, Personality, Refund, SeatedPlayer, Table,
    TableHost, TableRules, TableSeat, Variant,
};
use std::{
    cell::{Cell, RefCell},
    sync::Arc,
};

use ducy_gto::holdem::{
    abstraction::CardAbstraction,
    blueprint::Blueprint,
    bot::{GtoBot, RiverSolving, TurnSolving},
    hunl::{BettingTree, Hunl, HunlConfig},
    range::BucketCache,
    review::{HandRecord, HandReview, ReviewConfig, ReviewLog, Reviewer, SessionReview},
};
use serde::Serialize;
use wasm_bindgen::prelude::*;

fn err(e: impl std::fmt::Debug) -> JsError {
    JsError::new(&format!("{e:?}"))
}

#[derive(Serialize)]
struct PersonalityInfo {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    catchphrase: &'static str,
}

/// Every bot personality, for the page's seat picker.
#[wasm_bindgen(js_name = botPersonalities)]
pub fn bot_personalities() -> Result<JsValue, JsError> {
    let list: Vec<PersonalityInfo> = Personality::ALL
        .iter()
        .map(|p| PersonalityInfo {
            id: p.id(),
            name: p.name(),
            description: p.description(),
            catchphrase: p.catchphrase(),
        })
        .collect();
    serde_wasm_bindgen::to_value(&list).map_err(err)
}

/// What each of `seats` seats did in one finished hand, counted by the same
/// rules the bots use (ducy_play::stats::hand_counts): the page adds these
/// up for its player stats. `events` is the hand's event list as table
/// views give it. Returns one object per seat: {vpip, pfr, three_bet_chance,
/// three_bet, aggressive, calls, folds, faced_bets, folds_to_bets, saw_flop,
/// folded}.
#[wasm_bindgen(js_name = handCounts)]
pub fn hand_counts(events: JsValue, seats: usize) -> Result<JsValue, JsError> {
    let events: Vec<ducy_play::Event> = serde_wasm_bindgen::from_value(events).map_err(err)?;
    to_js(&ducy_play::stats::hand_counts(&events, seats))
}

#[derive(Serialize)]
struct BotReads {
    vpip: f64,
    pfr: f64,
    fold_to_bet: f64,
    aggression: f64,
}

/// How the bots read a player from summed counts: the same rates as the
/// page's, smoothed toward typical values so a few hands don't swing them
/// (ducy_play::stats::SeatStats). Returns {vpip, pfr, fold_to_bet, aggression}.
#[wasm_bindgen(js_name = botReads)]
#[allow(clippy::too_many_arguments)]
pub fn bot_reads(
    hands: u32,
    vpip_hands: u32,
    pfr_hands: u32,
    faced_bets: u32,
    folds_to_bets: u32,
    aggressive: u32,
    calls: u32,
) -> Result<JsValue, JsError> {
    let s = ducy_play::stats::SeatStats {
        hands,
        vpip_hands,
        pfr_hands,
        faced_bets,
        folds_to_bets,
        aggressive,
        calls,
    };
    to_js(&BotReads {
        vpip: s.vpip(),
        pfr: s.pfr(),
        fold_to_bet: s.fold_to_bet(),
        aggression: s.aggression(),
    })
}

#[derive(Serialize)]
struct PresetInfo {
    id: &'static str,
    label: &'static str,
    minutes: u32,
}

/// The tournament blind presets (ducy_play::structure): [{id, label, minutes}].
#[wasm_bindgen(js_name = blindPresets)]
pub fn blind_presets() -> Result<JsValue, JsError> {
    let list: Vec<PresetInfo> = ducy_play::structure::PRESETS
        .iter()
        .map(|p| PresetInfo {
            id: p.id,
            label: p.label,
            minutes: p.minutes,
        })
        .collect();
    to_js(&list)
}

/// A preset's blind structure for a starting stack: a list of
/// {sb, bb, ante, minutes} levels and {break: true, minutes} breaks.
#[wasm_bindgen(js_name = blindPreset)]
pub fn blind_preset(id: &str, stack: u64) -> Result<JsValue, JsError> {
    to_js(&ducy_play::structure::preset_structure(id, stack).map_err(|e| JsError::new(&e))?)
}

/// What's wrong with a structure as typed into a form, as messages to show
/// (none means it's playable).
#[wasm_bindgen(js_name = checkBlindStructure)]
pub fn check_blind_structure(steps: JsValue) -> Result<Vec<String>, JsError> {
    let steps: Vec<ducy_play::structure::StepInput> =
        serde_wasm_bindgen::from_value(steps).map_err(err)?;
    Ok(ducy_play::structure::check_structure(&steps))
}

/// About how long a tournament takes to find a winner: {minutes, hands,
/// level}. `steps` must be playable (see `checkBlindStructure`).
#[wasm_bindgen(js_name = estimateTournament)]
pub fn estimate_tournament(
    steps: JsValue,
    stack: u64,
    players: u32,
    table_size: u32,
    hands_per_hour: Option<f64>,
) -> Result<JsValue, JsError> {
    let steps: Vec<ducy_play::structure::Step> =
        serde_wasm_bindgen::from_value(steps).map_err(err)?;
    let o = ducy_play::structure::EstimateOptions {
        stack,
        players,
        table_size,
        hands_per_hour: hands_per_hour.unwrap_or(ducy_play::structure::HANDS_PER_HOUR),
        ..Default::default()
    };
    to_js(&ducy_play::structure::estimate(&steps, &o))
}

/// A finished hand replayed one event at a time (ducy_play::replay): frames
/// of {street, board, pot, seats: [{stack, bet, folded, all_in}], text,
/// event}, one before any action and one after each event. `events` is the
/// hand's event list as table views give it, `stacks` each seat's stack when
/// the hand started, `names` the seats' names for the text.
#[wasm_bindgen(js_name = replayFrames)]
pub fn replay_frames(
    events: JsValue,
    stacks: Vec<u64>,
    names: Vec<String>,
) -> Result<JsValue, JsError> {
    let events: Vec<ducy_play::Event> = serde_wasm_bindgen::from_value(events).map_err(err)?;
    to_js(&ducy_play::replay::replay_frames(&events, &stacks, &names))
}

pub(crate) fn rules_for(
    game: Option<&str>,
    small_blind: u64,
    big_blind: u64,
) -> Result<TableRules, JsError> {
    Ok(match game.unwrap_or("nlhe") {
        "nlhe" => TableRules::no_limit_holdem(small_blind, big_blind),
        g @ ("plo4" | "plo5" | "plo6") => TableRules::pot_limit_omaha(small_blind, big_blind)
            .with_variant(Variant::Omaha {
                hole_cards: g[3..].parse().unwrap_or(4),
            }),
        g => return Err(JsError::new(&format!("unknown game {g}"))),
    })
}

/// The GTO bot's data, once the page has loaded it with `loadGto`.
struct Gto {
    cards: Arc<CardAbstraction>,
    blueprint: Arc<Blueprint>,
    tree: Arc<BettingTree>,
}

/// Turn and river iterations per solve for the GTO bot in the page. Far
/// less turn work than natively (`TurnSolving::default`: 50 iterations of 8
/// rivers each): WebAssembly runs on one core, and a turn decision there
/// took about 3.7 s at 20 iterations of 8 rivers, too slow to play against.
/// 10 iterations of 4 rivers (below) take about a third of that, keeping
/// the opponent's river styles (Node, the `test-30m` model, CPU time, the
/// settings interleaved: 4.8 s against 1.6 s on a busy machine). A river
/// solve takes about 0.4 s.
const PAGE_TURN_ITERATIONS: usize = 10;
const PAGE_RIVER_ITERATIONS: usize = 200;

/// Turn solving's other settings in the page: river cards dealt per
/// iteration (`TurnSolving::new` deals 8), and whether the opponent picks
/// among river styles at each leaf (`TurnSolving::biases`, kept: the solve
/// is harder to exploit, and dropping them saved little at these settings).
const PAGE_TURN_RIVER_SAMPLES: usize = 4;
const PAGE_TURN_BIASES: bool = true;

thread_local! {
    static GTO: RefCell<Option<Gto>> = const { RefCell::new(None) };
    static SOLVING: Cell<(usize, usize)> =
        const { Cell::new((PAGE_TURN_ITERATIONS, PAGE_RIVER_ITERATIONS)) };
    static TURN_OPTIONS: Cell<(usize, bool)> =
        const { Cell::new((PAGE_TURN_RIVER_SAMPLES, PAGE_TURN_BIASES)) };
}

/// How hard GTO bots seated from now on solve the turn and river in real
/// time: iterations per solve, 0 to play the blueprint on that street.
#[wasm_bindgen(js_name = setGtoSolving)]
pub fn set_gto_solving(turn: usize, river: usize) {
    SOLVING.with(|s| s.set((turn, river)));
}

/// How GTO bots seated from now on solve the turn, besides the iterations:
/// `riverSamples` river cards dealt per iteration when valuing leaves (0
/// for all 48), and `biases` whether the opponent picks among river styles
/// at each leaf (sturdier, but several times the work at every leaf and in
/// the setup's best response) or both play the blueprint's river.
#[wasm_bindgen(js_name = setGtoTurnOptions)]
pub fn set_gto_turn_options(river_samples: usize, biases: bool) {
    TURN_OPTIONS.with(|s| s.set((river_samples, biases)));
}

/// The id that seats the GTO bot.
const GTO_ID: &str = "gto";

/// Loads the GTO bot: a heads-up no-limit Hold'em blueprint and the card
/// abstraction it was trained with (the compact form is enough). After
/// this, "gto" can be used as a bot id. It's trained for one opponent at
/// 100 big blinds; at bigger tables it falls back to a simple pot-odds rule.
#[wasm_bindgen(js_name = loadGto)]
pub fn load_gto(cards: &[u8], blueprint: &[u8]) -> Result<(), JsError> {
    let cards =
        CardAbstraction::load(cards).ok_or_else(|| JsError::new("not a card abstraction"))?;
    let config = HunlConfig::default();
    let game = Hunl::new(config, Some(&cards));
    let blueprint = Blueprint::load(blueprint, &game, &cards)
        .map_err(|e| JsError::new(&format!("blueprint doesn't match: {e:?}")))?;
    let tree = Arc::new(game.tree);
    GTO.with(|g| {
        *g.borrow_mut() = Some(Gto {
            cards: Arc::new(cards),
            blueprint: Arc::new(blueprint),
            tree,
        })
    });
    Ok(())
}

/// Whether `loadGto` has been called.
#[wasm_bindgen(js_name = gtoLoaded)]
pub fn gto_loaded() -> bool {
    GTO.with(|g| g.borrow().is_some())
}

fn gto_seat(seed: u64) -> Result<TableSeat, JsError> {
    GTO.with(|g| {
        let g = g.borrow();
        let g = g
            .as_ref()
            .ok_or_else(|| JsError::new("call loadGto first"))?;
        let (turn, river) = SOLVING.with(Cell::get);
        let (river_samples, biases) = TURN_OPTIONS.with(Cell::get);
        let mut turn = TurnSolving::new(turn);
        turn.river_samples = river_samples;
        if !biases {
            turn.biases.clear();
        }
        let bot = GtoBot::from_parts(g.cards.clone(), g.blueprint.clone(), g.tree.clone(), seed)
            .with_turn_solving(turn)
            .with_river_solving(RiverSolving::new(river));
        Ok(TableSeat::with_bot("GTO", GTO_ID, Box::new(bot)))
    })
}

/// "You" in seat 0, then one seat per bot id.
fn seats_for(bots: &[String], seed: u64) -> Result<Vec<TableSeat>, JsError> {
    if bots.is_empty() || bots.len() + 1 > ducy_play::MAX_PLAYERS {
        return Err(JsError::new("between 1 and 8 bots"));
    }
    let mut seats = vec![TableSeat::human("You", "you")];
    for (i, id) in bots.iter().enumerate() {
        if id == GTO_ID {
            seats.push(gto_seat(seed.wrapping_add(i as u64 + 1))?);
            continue;
        }
        let p =
            Personality::from_name(id).ok_or_else(|| JsError::new(&format!("unknown bot {id}")))?;
        seats.push(TableSeat::bot(
            p.id(),
            p.bot(Some(seed.wrapping_add(i as u64 + 1))),
        ));
    }
    Ok(seats)
}

/// The bot with id `id` (a personality, or "gto" once loaded), for a
/// restored table.
fn bot_for(id: &str, seed: u64) -> Result<Box<dyn ducy_play::Bot>, JsError> {
    if id == GTO_ID {
        return gto_seat(seed)?.bot.ok_or_else(|| JsError::new("no bot"));
    }
    let p = Personality::from_name(id).ok_or_else(|| JsError::new(&format!("unknown bot {id}")))?;
    Ok(Box::new(p.bot(Some(seed))))
}

fn action(kind: &str, amount: u64) -> Result<Action, JsError> {
    Ok(match kind {
        "fold" => Action::Fold,
        "check" => Action::Check,
        "call" => Action::Call,
        "bet" => Action::Bet(amount),
        "raise" => Action::Raise(amount),
        "allin" => Action::AllIn,
        _ => return Err(JsError::new(&format!("unknown action {kind}"))),
    })
}

/// A Hold'em or Omaha table: the person in seat 0 and up to eight bots.
///
/// Heads-up no-limit Hold'em against "gto", every finished hand is kept for
/// review: `reviewLastHand` grades the person's decisions in the last one,
/// and `reviewSession` all of them since the last `clearReviews`.
#[wasm_bindgen]
pub struct BotTable {
    table: Table,
    reviews: ReviewLog,
    /// The last hand number recorded for review.
    recorded: u64,
    /// Buckets for reviewing decisions as they're made, kept across calls.
    decision_cache: BucketCache,
}

#[derive(Serialize)]
struct SessionResult {
    hands: Vec<HandReview>,
    summary: SessionReview,
}

/// Turn and river samples for reviewing one decision as it's made: fewer
/// than a whole hand's review, so the feedback comes quickly.
const DECISION_FLOP_RUNOUTS: usize = 8;

/// Runs `f` with a reviewer for the loaded GTO bot.
fn with_reviewer<T>(f: impl FnOnce(&Reviewer) -> T) -> Result<T, JsError> {
    with_reviewer_config(ReviewConfig::default(), f)
}

fn with_reviewer_config<T>(
    config: ReviewConfig,
    f: impl FnOnce(&Reviewer) -> T,
) -> Result<T, JsError> {
    GTO.with(|g| {
        let g = g.borrow();
        let g = g
            .as_ref()
            .ok_or_else(|| JsError::new("call loadGto first"))?;
        let reviewer = Reviewer {
            tree: &g.tree,
            blueprint: &g.blueprint,
            cards: &g.cards,
            tree_big_blind: HunlConfig::default().big_blind,
            config,
        };
        Ok(f(&reviewer))
    })
}

#[wasm_bindgen]
impl BotTable {
    /// `bots` are personality ids (see `botPersonalities`), one per bot seat;
    /// the person sits in seat 0. Everyone starts with `buy_in` chips, and a
    /// player who goes broke is topped back up before the next hand. `game`
    /// is "nlhe" (no-limit Hold'em, the default) or "plo4", "plo5", "plo6"
    /// (pot-limit Omaha with that many hole cards).
    #[wasm_bindgen(constructor)]
    pub fn new(
        bots: Vec<String>,
        buy_in: u64,
        small_blind: u64,
        big_blind: u64,
        seed: u64,
        game: Option<String>,
    ) -> Result<BotTable, JsError> {
        let rules = rules_for(game.as_deref(), small_blind, big_blind)?;
        if small_blind == 0 || big_blind < small_blind || buy_in < big_blind {
            return Err(JsError::new("invalid blinds or buy-in"));
        }
        let mut table = Table::new(rules, seats_for(&bots, seed)?, buy_in, seed).map_err(err)?;
        // All-in with cards to come, the person may run it twice; the bots
        // go along with whatever they choose.
        table.set_run_it_twice(true);
        Ok(BotTable {
            table,
            reviews: ReviewLog::default(),
            recorded: 0,
            decision_cache: BucketCache::default(),
        })
    }

    /// Deals the next hand (moving the button and topping up broke players)
    /// and returns the state.
    #[wasm_bindgen(js_name = newHand)]
    pub fn new_hand(&mut self) -> Result<JsValue, JsError> {
        if self.table.in_hand() {
            return Err(JsError::new("the current hand isn't over"));
        }
        self.table.new_hand().map_err(err)?;
        self.state()
    }

    /// Lets the next bot act if it's a bot's turn; returns the state either way.
    pub fn advance(&mut self) -> Result<JsValue, JsError> {
        if self.table.hand().is_none() {
            return Err(JsError::new("no hand dealt"));
        }
        // Only bots left all-in: they run it once.
        if self.bots_choose_runs() {
            self.table.choose_runs(1).map_err(err)?;
            self.keep_for_review();
            return self.state();
        }
        self.table.advance().map_err(err)?;
        self.keep_for_review();
        self.state()
    }

    /// Applies the person's action: "fold", "check", "call", "bet", "raise"
    /// or "allin". `amount` is the street total for a bet or raise.
    pub fn act(&mut self, kind: &str, amount: u64) -> Result<JsValue, JsError> {
        if self.table.hand().is_none() {
            return Err(JsError::new("no hand dealt"));
        }
        if self.table.to_act() != Some(0) {
            return Err(JsError::new("it isn't your turn"));
        }
        self.table.act(0, action(kind, amount)?).map_err(err)?;
        self.keep_for_review();
        self.state()
    }

    /// Whether it's a bot's turn (the page calls `advance` while this is true).
    #[wasm_bindgen(js_name = botToAct)]
    pub fn bot_to_act(&self) -> bool {
        self.table.auto_to_act() || self.bots_choose_runs()
    }

    /// The person's answer when the hand asks whether to run it twice (the
    /// state's `run_choice`): the bots go along with it.
    #[wasm_bindgen(js_name = runTwice)]
    pub fn run_twice(&mut self, yes: bool) -> Result<JsValue, JsError> {
        match self.table.awaiting() {
            Some(ducy_play::Awaiting::RunChoice(seats)) if seats.contains(&0) => {}
            _ => return Err(JsError::new("there's nothing to choose")),
        }
        self.table
            .choose_runs(if yes { 2 } else { 1 })
            .map_err(err)?;
        self.keep_for_review();
        self.state()
    }

    /// What the person may see right now.
    pub fn state(&self) -> Result<JsValue, JsError> {
        if self.table.hand().is_none() {
            return Err(JsError::new("no hand dealt"));
        }
        to_js(&self.table.view(0))
    }

    /// Puts every seat back to the buy-in for the next hand, so no chips
    /// carry over (e.g. training against "gto", always at the same depth).
    /// Only between hands.
    #[wasm_bindgen(js_name = resetStacks)]
    pub fn reset_stacks(&mut self) -> Result<(), JsError> {
        for seat in 0..self.table.num_seats() {
            self.table.reset_stack(seat).map_err(err)?;
        }
        Ok(())
    }

    /// Hands kept for review (heads-up no-limit Hold'em against "gto").
    #[wasm_bindgen(js_name = reviewableHands)]
    pub fn reviewable_hands(&self) -> usize {
        self.reviews.len()
    }

    /// The review of the person's decisions in the last finished hand
    /// against "gto", or null if there is none. Each decision has the
    /// street, board, pot and price in big blinds, the action taken, the
    /// blueprint's actions with how often it plays each (and their values
    /// when valued), the big blinds lost, a grade (fine, inaccuracy,
    /// mistake, blunder, deviation or unknown) and a note.
    /// Null at tables without the GTO bot (review isn't available there).
    #[wasm_bindgen(js_name = reviewLastHand)]
    pub fn review_last_hand(&mut self) -> Result<JsValue, JsError> {
        if self.reviews.is_empty() {
            return Ok(JsValue::NULL);
        }
        let reviews = &mut self.reviews;
        let r = with_reviewer(|rv| reviews.last(rv).cloned())?;
        match r {
            Some(r) => to_js(&r),
            None => Ok(JsValue::NULL),
        }
    }

    /// Every kept hand's review, and totals for the session:
    /// `{hands, summary}`. Hands are reviewed once and remembered.
    #[wasm_bindgen(js_name = reviewSession)]
    pub fn review_session(&mut self) -> Result<JsValue, JsError> {
        let reviews = &mut self.reviews;
        let (hands, summary) = with_reviewer(|rv| reviews.all(rv))?;
        to_js(&SessionResult { hands, summary })
    }

    /// Forgets the hands kept for review, to start a new session.
    #[wasm_bindgen(js_name = clearReviews)]
    pub fn clear_reviews(&mut self) {
        self.reviews.clear();
    }

    /// The review of the person's most recent decision in the current hand
    /// (or the last hand, once it's over), for feedback straight after each
    /// action: the same fields as a decision in `reviewLastHand`. It uses
    /// only what the person could see then: their cards, the board so far
    /// and the bot's range, never the bot's cards or the cards to come.
    /// Uses fewer samples than `reviewLastHand`, so a flop decision's value
    /// is rougher (see its `stderr`). Null before the person's first
    /// decision, and at tables without the GTO bot.
    #[wasm_bindgen(js_name = reviewLastDecision)]
    pub fn review_last_decision(&mut self) -> Result<JsValue, JsError> {
        if !self.against_gto() {
            return Ok(JsValue::NULL);
        }
        // Every seat of a bot table is dealt in, so the person is player 0.
        let Some(rec) = self
            .table
            .hand()
            .and_then(|hand| HandRecord::in_progress(hand, 0))
        else {
            return Ok(JsValue::NULL);
        };
        let config = ReviewConfig {
            flop_runouts: DECISION_FLOP_RUNOUTS,
            ..ReviewConfig::default()
        };
        let cache = &mut self.decision_cache;
        match with_reviewer_config(config, |rv| rv.review_last_decision(&rec, cache))? {
            Some(r) => to_js(&r),
            None => Ok(JsValue::NULL),
        }
    }
}

impl BotTable {
    /// The hand asks whether to run it twice and the person isn't in it.
    fn bots_choose_runs(&self) -> bool {
        matches!(self.table.awaiting(), Some(ducy_play::Awaiting::RunChoice(seats)) if !seats.contains(&0))
    }

    /// Whether the person plays the GTO bot heads-up, where review works.
    fn against_gto(&self) -> bool {
        self.table.num_seats() == 2 && self.table.seat(1).id == GTO_ID
    }

    /// Keeps a just-finished hand for review when the person played it
    /// heads-up against the GTO bot.
    fn keep_for_review(&mut self) {
        let number = self.table.hand_number();
        let Some(hand) = self.table.hand() else {
            return;
        };
        if !hand.is_complete() || number == self.recorded {
            return;
        }
        self.recorded = number;
        if self.against_gto()
            && let Some(summary) = hand.summary(0)
        {
            self.reviews.record(&summary);
        }
    }
}

fn to_js(value: &impl Serialize) -> Result<JsValue, JsError> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(err)
}

#[derive(Serialize)]
struct Message<'a> {
    to: &'a str,
    data: &'a ducy_play::Update,
}

/// What a host call returns: the host's own view, whether a bot or away
/// player should act next, and the messages to send to each player.
#[derive(Serialize)]
struct HostResult<'a> {
    /// Goes up whenever anything changes, so the page can drop late results.
    seq: u64,
    state: ducy_play::TableView,
    #[serde(rename = "botToAct")]
    bot_to_act: bool,
    #[serde(rename = "turnMsLeft")]
    turn_ms_left: Option<u64>,
    #[serde(rename = "openSeats")]
    open_seats: usize,
    /// People who'd be dealt into the next hand, the host included.
    #[serde(rename = "playersIn")]
    players_in: usize,
    /// At a friends table: requests for chips waiting for the host, the
    /// buy-in limits, and the host's own chips approved for the next hand.
    #[serde(rename = "chipRequests")]
    chip_requests: Vec<ChipRequest>,
    #[serde(rename = "buyIn")]
    buy_in: Option<BuyIn>,
    #[serde(rename = "hostPendingChips")]
    host_pending_chips: u64,
    /// People at the table and their chips, and people who left (with a
    /// bank) and the chips they took, since the last call.
    players: Vec<SeatedPlayer>,
    departed: Vec<Departure>,
    /// Approved top-ups that didn't fit under the maximum at the deal, since
    /// the last call: the app returns them to the player (ducy-play Refund).
    refunds: Vec<Refund>,
    out: Vec<Message<'a>>,
    /// Only from `newHand`: the fair-deal seed the hand was dealt from (hex),
    /// or null for a secure random deal.
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<Option<String>>,
    /// What a hidden hand (`newHandHidden`) waits for, or null:
    /// {kind: 'board', cards: how many} or {kind: 'reveals', seats}.
    awaiting: Option<AwaitingOut>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum AwaitingOut {
    Board {
        cards: usize,
    },
    Reveals {
        seats: Vec<usize>,
    },
    /// Everyone left is all-in: these seats choose whether to run it twice.
    RunChoice {
        seats: Vec<usize>,
    },
}

#[derive(Serialize)]
struct BuyIn {
    min: u64,
    max: u64,
}

/// A table hosted for people on other devices: the host in seat 0, and the
/// other seats either bots that people may take over (`new`) or empty seats
/// for friends, with real chips the host hands out (`friends`). Every call
/// takes the time in milliseconds (e.g. `Date.now()`) and returns `{seq,
/// state, botToAct, turnMsLeft, openSeats, playersIn, chipRequests, buyIn,
/// hostPendingChips, out}`, where `out` lists `{to, data}` messages for the
/// page to send to each player.
#[wasm_bindgen]
pub struct MultiTable {
    host: TableHost,
}

#[wasm_bindgen]
impl MultiTable {
    /// Like [`BotTable::new`], with `open` saying which bot seats people may
    /// take (one flag per bot) and `host_name` the host's shown name.
    /// `turn_ms` is how long a person has to act (0 for no limit).
    #[wasm_bindgen(constructor)]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        bots: Vec<String>,
        open: Vec<u8>,
        host_name: String,
        buy_in: u64,
        small_blind: u64,
        big_blind: u64,
        seed: u64,
        game: Option<String>,
        turn_ms: u64,
    ) -> Result<MultiTable, JsError> {
        let rules = rules_for(game.as_deref(), small_blind, big_blind)?;
        if small_blind == 0 || big_blind < small_blind || buy_in < big_blind {
            return Err(JsError::new("invalid blinds or buy-in"));
        }
        if open.len() != bots.len() {
            return Err(JsError::new("one open flag per bot"));
        }
        let mut seats = seats_for(&bots, seed)?;
        let name: String = host_name.trim().chars().take(ducy_play::MAX_NAME).collect();
        if !name.is_empty() {
            seats[0].name = name;
        }
        let mut table = Table::new(rules, seats, buy_in, seed)
            .map_err(err)?
            .with_secure_deals();
        table.set_run_it_twice(true);
        let open = std::iter::once(false)
            .chain(open.iter().map(|&o| o != 0))
            .collect();
        let host = TableHost::new(table, open, turn_ms).map_err(err)?;
        Ok(MultiTable { host })
    }

    /// A table for friends: the host in seat 0 and `seats - 1` empty seats
    /// (2 to 10 in all) that people take as they join. No bots: a hand is
    /// dealt to whoever is seated with chips, once there are two. Chips are
    /// real: the host starts with `host_chips`, everyone else asks the host
    /// for chips, and a stack after a request must be `min_buy_in` to
    /// `max_buy_in` chips.
    #[allow(clippy::too_many_arguments)]
    pub fn friends(
        seats: usize,
        host_name: String,
        host_chips: u64,
        min_buy_in: u64,
        max_buy_in: u64,
        small_blind: u64,
        big_blind: u64,
        seed: u64,
        game: Option<String>,
        turn_ms: u64,
    ) -> Result<MultiTable, JsError> {
        let rules = rules_for(game.as_deref(), small_blind, big_blind)?;
        if small_blind == 0 || big_blind < small_blind || min_buy_in < big_blind {
            return Err(JsError::new("invalid blinds or buy-in"));
        }
        if !(2..=ducy_play::MAX_PLAYERS).contains(&seats) {
            return Err(JsError::new("a table seats 2 to 10"));
        }
        if host_chips < min_buy_in || host_chips > max_buy_in {
            return Err(JsError::new(
                "the host's chips must be within the buy-in limits",
            ));
        }
        let name: String = host_name.trim().chars().take(ducy_play::MAX_NAME).collect();
        let mut all = vec![TableSeat::human(
            if name.is_empty() { "Host" } else { &name },
            "you",
        )];
        all.extend((1..seats).map(|_| TableSeat::empty()));
        let mut table = Table::new(rules, all, host_chips, seed)
            .map_err(err)?
            .with_secure_deals();
        table.set_run_it_twice(true);
        let open = (0..seats).map(|s| s > 0).collect();
        let host = TableHost::new(table, open, turn_ms)
            .and_then(|h| h.with_bank(min_buy_in, max_buy_in))
            .map_err(err)?;
        Ok(MultiTable { host })
    }

    /// A club table: `seats` empty seats (2 to 10) for people, and no seat
    /// for the host, who only watches (the host's view shows no cards before
    /// they're shown down). Chips are real: people ask for them within
    /// `min_buy_in` to `max_buy_in`, the page approves or denies each request
    /// against the player's club balance, and `departed` in each result says
    /// what everyone who leaves takes with them.
    #[allow(clippy::too_many_arguments)]
    pub fn club(
        seats: usize,
        min_buy_in: u64,
        max_buy_in: u64,
        small_blind: u64,
        big_blind: u64,
        seed: u64,
        game: Option<String>,
        turn_ms: u64,
    ) -> Result<MultiTable, JsError> {
        let rules = rules_for(game.as_deref(), small_blind, big_blind)?;
        if small_blind == 0 || big_blind < small_blind || min_buy_in < big_blind {
            return Err(JsError::new("invalid blinds or buy-in"));
        }
        if !(2..=ducy_play::MAX_PLAYERS).contains(&seats) {
            return Err(JsError::new("a table seats 2 to 10"));
        }
        let all = (0..seats).map(|_| TableSeat::empty()).collect();
        let mut table = Table::new(rules, all, min_buy_in, seed)
            .map_err(err)?
            .with_secure_deals();
        table.set_run_it_twice(true);
        let host = TableHost::without_host(table, turn_ms, min_buy_in, max_buy_in).map_err(err)?;
        Ok(MultiTable { host })
    }

    fn result(&mut self, out: &[Outgoing], now: u64) -> Result<JsValue, JsError> {
        self.result_with_seed(out, now, None)
    }

    fn result_with_seed(
        &mut self,
        out: &[Outgoing],
        now: u64,
        seed: Option<Option<String>>,
    ) -> Result<JsValue, JsError> {
        let departed = self.host.take_departures();
        let refunds = self.host.take_refunds();
        to_js(&HostResult {
            players: self.host.seated(),
            departed,
            refunds,
            seq: self.host.seq(),
            state: self.host.host_view(),
            bot_to_act: self.host.auto_to_act(),
            turn_ms_left: self.host.turn_ms_left(now),
            open_seats: self.host.open_seats(),
            players_in: self.host.players_in(),
            chip_requests: self.host.chip_requests(),
            buy_in: self
                .host
                .buy_in_limits()
                .map(|(min, max)| BuyIn { min, max }),
            host_pending_chips: self.host.host_pending_chips(),
            out: out
                .iter()
                .map(|o| Message {
                    to: &o.to,
                    data: &o.update,
                })
                .collect(),
            seed,
            awaiting: self.host.awaiting().map(|a| match a {
                ducy_play::Awaiting::Board(street) => AwaitingOut::Board {
                    cards: if street == ducy_play::Street::Flop {
                        3
                    } else {
                        1
                    },
                },
                ducy_play::Awaiting::Reveals(seats) => AwaitingOut::Reveals { seats },
                ducy_play::Awaiting::RunChoice(seats) => AwaitingOut::RunChoice { seats },
            }),
        })
    }

    /// The host's view, with no messages.
    pub fn state(&mut self, now: f64) -> Result<JsValue, JsError> {
        self.result(&[], now as u64)
    }

    /// A message from player `client` (a JSON command object).
    pub fn handle(&mut self, client: &str, command: JsValue, now: f64) -> Result<JsValue, JsError> {
        let now = now as u64;
        let out = match serde_wasm_bindgen::from_value::<Command>(command) {
            Ok(c) => self.host.handle(client, c, now),
            Err(_) => vec![Outgoing {
                to: client.to_string(),
                update: ducy_play::Update::Rejected {
                    reason: "bad message".to_string(),
                },
            }],
        };
        self.result(&out, now)
    }

    /// Player `client`'s connection dropped.
    pub fn disconnected(&mut self, client: &str, now: f64) -> Result<JsValue, JsError> {
        let out = self.host.disconnected(client, now as u64);
        self.result(&out, now as u64)
    }

    /// Deals the next hand. With `seed` (64 hex digits, from `fairSeed`), it's
    /// dealt from that seed, card for card as `fairDeal` gives it, so everyone
    /// can check it; without, from secure randomness as before. The result's
    /// `seed` says which: the seed, or null.
    #[wasm_bindgen(js_name = newHand)]
    pub fn new_hand(&mut self, now: f64, seed: Option<String>) -> Result<JsValue, JsError> {
        let out = match &seed {
            Some(s) => {
                let bytes = crate::fair::bytes32(s)?;
                self.host.new_hand_from_seed(now as u64, &bytes)
            }
            None => self.host.new_hand(now as u64),
        }
        .map_err(err)?;
        self.result_with_seed(&out, now as u64, Some(seed.map(|s| s.to_lowercase())))
    }

    /// Deals the next hand from a trustless shuffle's deck (ducy-shuffle),
    /// made for `players` (client ids, seat order): only they're dealt in.
    /// The host never learns a hole card. The hand then waits (the result's
    /// `awaiting`) for the board, street by street (`dealBoard`), and at
    /// showdown for hands (`reveal`, `forfeit`); `tick` forfeits anyone out
    /// of time to show. Not at a table with bots or a host seat.
    #[wasm_bindgen(js_name = newHandHidden)]
    pub fn new_hand_hidden(&mut self, now: f64, players: Vec<String>) -> Result<JsValue, JsError> {
        let out = self
            .host
            .new_hand_hidden(now as u64, &players)
            .map_err(err)?;
        self.result(&out, now as u64)
    }

    /// The board cards a hidden hand waits for ("Ah", …), unlocked by the host.
    #[wasm_bindgen(js_name = dealBoard)]
    pub fn deal_board(&mut self, cards: Vec<String>, now: f64) -> Result<JsValue, JsError> {
        let cards = cards
            .iter()
            .map(|c| ducy::deck::Card::parse(c).map_err(err))
            .collect::<Result<Vec<_>, _>>()?;
        let out = self.host.deal_board(&cards, now as u64).map_err(err)?;
        self.result(&out, now as u64)
    }

    /// At showdown, `seat` showed these cards, checked against the shuffle
    /// (`shuffleOpenWith` with their published secret).
    pub fn reveal(
        &mut self,
        seat: usize,
        cards: Vec<String>,
        now: f64,
    ) -> Result<JsValue, JsError> {
        let mut deck = ducy::deck::Deck::empty();
        for c in &cards {
            deck |= ducy::deck::Card::parse(c).map_err(err)?;
        }
        let out = self.host.reveal(seat, deck, now as u64).map_err(err)?;
        self.result(&out, now as u64)
    }

    /// At showdown, `seat` doesn't show, or its cards didn't check out: it
    /// can't win.
    pub fn forfeit(&mut self, seat: usize, now: f64) -> Result<JsValue, JsError> {
        let out = self.host.forfeit(seat, now as u64).map_err(err)?;
        self.result(&out, now as u64)
    }

    /// The host's answer to running it twice (players send
    /// {type: 'run_twice', yes}). Twice only if every person still in says yes.
    #[wasm_bindgen(js_name = hostRunTwice)]
    pub fn host_run_twice(&mut self, yes: bool, now: f64) -> Result<JsValue, JsError> {
        let out = self.host.host_run_twice(yes, now as u64).map_err(err)?;
        self.result(&out, now as u64)
    }

    /// Lets a bot (or an away player) act.
    pub fn advance(&mut self, now: f64) -> Result<JsValue, JsError> {
        let out = self.host.advance(now as u64).map_err(err)?;
        self.result(&out, now as u64)
    }

    /// The host's own action.
    pub fn act(&mut self, kind: &str, amount: u64, now: f64) -> Result<JsValue, JsError> {
        let out = self.host.host_act(kind, amount, now as u64).map_err(err)?;
        self.result(&out, now as u64)
    }

    /// Runs the turn clock; call it every half second or so.
    pub fn tick(&mut self, now: f64) -> Result<JsValue, JsError> {
        let out = self.host.tick(now as u64);
        self.result(&out, now as u64)
    }

    /// How long a person may sit out before their seat is given up (and,
    /// with a bank, their chips leave with them), in milliseconds; 0 for no
    /// limit. Checked on each `tick`.
    #[wasm_bindgen(js_name = setSitOutLimit)]
    pub fn set_sit_out_limit(&mut self, ms: f64) {
        self.host.set_sit_out_limit(ms as u64);
    }

    /// Approves the chips the player in `seat` asked for.
    #[wasm_bindgen(js_name = approveChips)]
    pub fn approve_chips(&mut self, seat: usize, now: f64) -> Result<JsValue, JsError> {
        let out = self.host.approve_chips(seat, now as u64);
        self.result(&out, now as u64)
    }

    /// Turns down the chips the player in `seat` asked for.
    #[wasm_bindgen(js_name = denyChips)]
    pub fn deny_chips(&mut self, seat: usize, now: f64) -> Result<JsValue, JsError> {
        let out = self.host.deny_chips(seat, now as u64);
        self.result(&out, now as u64)
    }

    /// The host adds `amount` chips to their own stack, within the buy-in
    /// limits (before the next hand if they're in this one).
    #[wasm_bindgen(js_name = hostChips)]
    pub fn host_chips(&mut self, amount: u64, now: f64) -> Result<JsValue, JsError> {
        let out = self.host.host_chips(amount, now as u64).map_err(err)?;
        self.result(&out, now as u64)
    }

    /// Removes the person in `seat`: they fold out of this hand and the
    /// seat is free from the next.
    pub fn remove(&mut self, seat: usize, now: f64) -> Result<JsValue, JsError> {
        let out = self.host.remove(seat, now as u64);
        self.result(&out, now as u64)
    }

    /// The whole table as JSON, the hand in progress included, for
    /// `restore` after the page closes or the browser restarts. It holds
    /// every card of the hand in progress: keep it where only the host can
    /// read it.
    pub fn save(&self, now: f64) -> Result<String, JsError> {
        serde_json::to_string(&self.host.snapshot(now as u64)).map_err(err)
    }

    /// Loads a table saved with `save`, paused: its clocks stand still
    /// until `resume`. No one is connected; people get their seats back by
    /// joining again, and aren't folded for meanwhile. Fails for a snapshot
    /// from another version, or one that doesn't play out to what was saved.
    pub fn restore(json: &str, now: f64) -> Result<MultiTable, JsError> {
        let s: ducy_play::HostSnapshot = serde_json::from_str(json).map_err(err)?;
        let mut n = 0;
        let host = TableHost::restore(&s, now as u64, |id| {
            n += 1;
            bot_for(id, (now as u64).wrapping_add(n)).ok()
        })
        .map_err(err)?;
        Ok(MultiTable { host })
    }

    /// Stops the clocks (no turn runs out, no sit-out time counts), e.g.
    /// while no one can be reached.
    pub fn pause(&mut self, now: f64) {
        self.host.pause(now as u64);
    }

    /// Starts the clocks again where they stopped; returns the updates
    /// carrying the time left.
    pub fn resume(&mut self, now: f64) -> Result<JsValue, JsError> {
        let out = self.host.resume(now as u64);
        self.result(&out, now as u64)
    }

    /// Whether the clocks are stopped.
    #[wasm_bindgen(js_name = isPaused)]
    pub fn is_paused(&self) -> bool {
        self.host.is_paused()
    }
}
