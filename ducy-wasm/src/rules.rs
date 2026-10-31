//! A club's own rules (ducy-play's `club_rules`, ducy-web #455), for
//! ducy-web's club host: members and names, help, the lobby, the club info,
//! tables and reserved seats.
//!
//! The club stays ducy-web's plain object. Each call takes the rules' part
//! of it as JSON and an op ({op, …}), and returns `{ok: true, value, club}`,
//! where `club` is that part after the op when the op changes it, or
//! `{ok: false, message}` with nothing changed. Times are milliseconds
//! (e.g. `Date.now()`).

use std::collections::BTreeMap;

use ducy_play::club_rules::{self, Club, HOLD_MS, Refused, RuleError};
use rand::rngs::StdRng;
use serde::Deserialize;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

use crate::util::{err, to_js};

/// An op, as ducy-web sends it.
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Op {
    New {
        name: String,
        code: String,
        host_id: String,
    },
    Receive {
        from: String,
        body: Value,
    },
    Approve {
        id: String,
    },
    Decline {
        id: String,
    },
    Removable {
        id: String,
    },
    Remove {
        id: String,
    },
    RevokeInvite,
    AskHelp {
        from: String,
        text: String,
        #[serde(default)]
        table: String,
    },
    AnswerHelp {
        by: String,
        id: String,
        #[serde(default)]
        reply: String,
        #[serde(default = "yes")]
        handled: bool,
    },
    StaffOnline {
        online: Vec<String>,
        #[serde(default)]
        host_here: bool,
    },
    HelpBody {
        id: String,
        staff: Vec<Value>,
    },
    SayInLobby {
        from: String,
        text: String,
    },
    OnlineList {
        online: Vec<String>,
        #[serde(default)]
        host_here: bool,
    },
    LobbyBody {
        online: Vec<String>,
        #[serde(default)]
        host_here: bool,
    },
    SetInfo {
        info: Value,
    },
    NewTable {
        spec: Value,
        most: Option<u64>,
    },
    DefaultTableName {
        game: String,
        sb: u64,
        bb: u64,
    },
    TablesBody {
        #[serde(default)]
        seated: BTreeMap<String, u32>,
    },
    CanJoin {
        id: String,
        table: String,
        at: Option<String>,
    },
    NoteStacks {
        players: Vec<String>,
        hand: u64,
    },
    HandsToStay {
        id: String,
        table: String,
        completed: u64,
    },
    CloseClosing {
        keep: Vec<String>,
    },
    Reservations {
        table: String,
    },
    HeldSeats {
        table: String,
        #[serde(default)]
        except: String,
    },
    ReserveSeat {
        id: String,
        table: String,
        seated: usize,
        from: Option<String>,
        by: Option<String>,
        hold_ms: Option<u64>,
    },
    DropReservation {
        table: String,
        id: String,
    },
    HoldFrom {
        table: String,
        id: String,
        hold_ms: Option<u64>,
    },
    PruneReservations,
    NextLapse,
    PruneGroups,
    MovesUp {
        id: String,
        #[serde(default)]
        seated: BTreeMap<String, Vec<String>>,
        #[serde(default)]
        held: BTreeMap<String, usize>,
    },
    Promote {
        id: String,
        #[serde(default)]
        taken: BTreeMap<String, usize>,
    },
    WantsTable {
        id: String,
        #[serde(default)]
        taken: BTreeMap<String, usize>,
    },
    AskAbout {
        id: String,
    },
    ExtendAbout {
        id: String,
    },
    JoinList {
        id: String,
        key: String,
    },
    LeaveList {
        id: String,
        key: String,
    },
    BackOnList {
        id: String,
        key: String,
    },
    SatDown {
        id: String,
        table: String,
    },
    OfferSeats {
        #[serde(default)]
        taken: BTreeMap<String, usize>,
    },
    LapseLists,
    NextListDue,
    ListsFor {
        id: String,
    },
    GroupAnswer {
        id: String,
        who: String,
        yes: bool,
    },
    FinishAsk {
        id: String,
        most: Option<u64>,
    },
    AddService {
        id: String,
        alg: String,
        key: String,
        o: Value,
    },
    UpdateService {
        id: String,
        o: Value,
    },
    RevokeService {
        id: String,
    },
    ServicesView,
    CheckService {
        from: String,
        alg: String,
        key: String,
        seq: u64,
        body: Value,
        sig: Option<String>,
    },
    PrepareService {
        id: String,
        body: Value,
        member: Option<String>,
    },
    ServiceDone {
        id: String,
        #[serde(rename = "do")]
        what: String,
        amount: u64,
        key: Option<String>,
        value: Value,
    },
}

fn yes() -> bool {
    true
}

fn rng() -> StdRng {
    rand::make_rng()
}

fn value(v: impl serde::Serialize) -> Result<Value, RuleError> {
    serde_json::to_value(v).map_err(|e| RuleError(e.to_string()))
}

/// A service's request answered, or refused (`{refused: {status, reason}}`,
/// with the refusal counted, so the club still changes).
fn answer<T: serde::Serialize>(r: Result<T, Refused>) -> Result<Value, RuleError> {
    match r {
        Ok(v) => value(v),
        Err(refused) => Ok(json!({"refused": refused})),
    }
}

/// Runs `op` on the club whose rules' part is `club` (JSON; ignored for
/// `new`). Throws only for JSON or an op it can't read.
#[wasm_bindgen(js_name = clubRules)]
pub fn club_rules(club: &str, op: JsValue, now: f64) -> Result<JsValue, JsError> {
    let op: Op = serde_wasm_bindgen::from_value(op).map_err(err)?;
    let now = now as u64;
    if let Op::New {
        name,
        code,
        host_id,
    } = &op
    {
        let r = Club::new(name, code, host_id, now, &mut rng());
        return to_js(&match r {
            Ok(c) => json!({"ok": true, "value": null, "club": c}),
            Err(e) => json!({"ok": false, "message": e.0}),
        });
    }
    let mut c: Club = serde_json::from_str(club).map_err(err)?;
    let mut changes = true;
    let r: Result<Value, RuleError> = match op {
        Op::New { .. } => unreachable!("handled above"),
        Op::Receive { from, body } => value(c.receive(&from, &body, now)),
        Op::Approve { id } => value(c.approve(&id, now)),
        Op::Decline { id } => value(c.decline(&id, now)),
        Op::Removable { id } => {
            changes = false;
            value(c.removable(&id))
        }
        Op::Remove { id } => value(c.remove(&id, now)),
        Op::RevokeInvite => {
            c.revoke_invite(&mut rng());
            Ok(Value::Null)
        }
        Op::AskHelp { from, text, table } => c
            .ask_help(&from, &text, &table, now, &mut rng())
            .and_then(value),
        Op::AnswerHelp {
            by,
            id,
            reply,
            handled,
        } => c
            .answer_help(&by, &id, &reply, handled, now)
            .and_then(value),
        Op::StaffOnline { online, host_here } => {
            changes = false;
            value(c.staff_online(&online, host_here))
        }
        Op::HelpBody { id, staff } => {
            changes = false;
            Ok(c.help_body(&id, &staff))
        }
        Op::SayInLobby { from, text } => c
            .say_in_lobby(&from, &text, now, &mut rng())
            .and_then(value),
        Op::OnlineList { online, host_here } => {
            changes = false;
            value(c.online_list(&online, host_here))
        }
        Op::LobbyBody { online, host_here } => {
            changes = false;
            Ok(c.lobby_body(&online, host_here, now))
        }
        Op::SetInfo { info } => value(c.set_info(&info, now)),
        Op::NewTable { spec, most } => c.new_table(&spec, now, most, &mut rng()).and_then(value),
        Op::DefaultTableName { game, sb, bb } => {
            changes = false;
            value(c.default_table_name(&game, sb, bb))
        }
        Op::TablesBody { seated } => {
            changes = false;
            Ok(c.tables_body(&seated))
        }
        Op::CanJoin { id, table, at } => {
            changes = false;
            value(c.can_join(&id, &table, at.as_deref()))
        }
        Op::NoteStacks { players, hand } => {
            c.note_stacks(&players, hand);
            Ok(Value::Null)
        }
        Op::HandsToStay {
            id,
            table,
            completed,
        } => {
            changes = false;
            value(c.hands_to_stay(&id, &table, completed))
        }
        Op::CloseClosing { keep } => {
            c.close_closing(&keep);
            Ok(Value::Null)
        }
        Op::Reservations { table } => {
            changes = false;
            value(c.reservations(&table, now))
        }
        Op::HeldSeats { table, except } => {
            changes = false;
            value(c.held_seats(&table, now, &except))
        }
        Op::ReserveSeat {
            id,
            table,
            seated,
            from,
            by,
            hold_ms,
        } => c
            .reserve_seat(
                &id,
                &table,
                seated,
                from.as_deref(),
                by.as_deref(),
                now,
                hold_ms.unwrap_or(HOLD_MS),
            )
            .and_then(value),
        Op::DropReservation { table, id } => value(c.drop_reservation(&table, &id)),
        Op::HoldFrom { table, id, hold_ms } => {
            value(c.hold_from(&table, &id, now, hold_ms.unwrap_or(HOLD_MS)))
        }
        Op::PruneReservations => {
            let gone = c.prune_reservations(now);
            let gone: Vec<Value> = gone
                .into_iter()
                .map(|(table, e)| {
                    let mut v = serde_json::to_value(e).unwrap_or_default();
                    v["table"] = table.into();
                    v
                })
                .collect();
            Ok(Value::Array(gone))
        }
        Op::NextLapse => {
            changes = false;
            value(c.next_lapse())
        }
        Op::PruneGroups => {
            c.prune_groups();
            Ok(Value::Null)
        }
        Op::MovesUp { id, seated, held } => {
            changes = false;
            value(c.moves_up(&id, &seated, &held))
        }
        Op::Promote { id, taken } => value(c.promote(&id, &taken)),
        Op::WantsTable { id, taken } => {
            changes = false;
            value(c.wants_table(&id, &taken))
        }
        Op::AskAbout { id } => c.ask_about(&id, now).and_then(value),
        Op::ExtendAbout { id } => value(c.extend_about(&id, now)),
        Op::JoinList { id, key } => c.join_list(&id, &key, now).and_then(value),
        Op::LeaveList { id, key } => value(c.leave_list(&id, &key)),
        Op::BackOnList { id, key } => c.back_on_list(&id, &key, now).and_then(value),
        Op::SatDown { id, table } => value(c.sat_down(&id, &table)),
        Op::OfferSeats { taken } => value(c.offer_seats(&taken, now)),
        Op::LapseLists => value(c.lapse_lists(now)),
        Op::NextListDue => {
            changes = false;
            value(c.next_list_due())
        }
        Op::ListsFor { id } => {
            changes = false;
            Ok(c.lists_for(&id))
        }
        Op::GroupAnswer { id, who, yes } => value(c.group_answer(&id, &who, yes, now)),
        Op::FinishAsk { id, most } => value(c.finish_ask(&id, now, most, &mut rng())),
        Op::AddService { id, alg, key, o } => {
            c.add_service(&id, &alg, &key, &o, now).and_then(value)
        }
        Op::UpdateService { id, o } => c.update_service(&id, &o).and_then(value),
        Op::RevokeService { id } => c.revoke_service(&id, now).and_then(value),
        Op::ServicesView => {
            changes = false;
            value(c.services_view(now))
        }
        Op::CheckService {
            from,
            alg,
            key,
            seq,
            body,
            sig,
        } => answer(c.check_service_request(&from, &alg, &key, seq, &body, sig.as_deref(), now)),
        Op::PrepareService { id, body, member } => {
            answer(c.prepare_service_request(&id, &body, member.as_deref(), now))
        }
        Op::ServiceDone {
            id,
            what,
            amount,
            key,
            value: v,
        } => {
            c.service_done(&id, &what, amount, key.as_deref(), &v, now);
            Ok(Value::Null)
        }
    };
    to_js(&match r {
        Ok(v) if changes => json!({"ok": true, "value": v, "club": c}),
        Ok(v) => json!({"ok": true, "value": v}),
        Err(e) => json!({"ok": false, "message": e.0}),
    })
}

/// What a name reads as (#450): names with the same key are the same name.
#[wasm_bindgen(js_name = nameKey)]
pub fn name_key(name: &str) -> String {
    club_rules::name_key(name)
}

/// The club info as given, checked, against the info it replaces (JSON, or
/// "null").
#[wasm_bindgen(js_name = cleanClubInfo)]
pub fn clean_club_info(input: &str, prev: &str, now: f64) -> Result<JsValue, JsError> {
    let input: Value = serde_json::from_str(input).map_err(err)?;
    let prev: Option<club_rules::Info> = serde_json::from_str(prev).unwrap_or(None);
    to_js(&club_rules::clean_info(&input, prev.as_ref(), now as u64))
}
