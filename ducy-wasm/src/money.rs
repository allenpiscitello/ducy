//! A club's money (ducy-play's `club_money`, ducy-web #454), for ducy-web's
//! club host: links, paths, credit and the ledger, as plain JS objects.
//! Every move is one `run` call with an op ({op, …}); every call that can
//! depend on the time takes it in milliseconds (e.g. `Date.now()`).

use ducy_play::club_money::{Entry, EntryKind, Money, MoneyError, Terms};
use serde::Deserialize;
use serde_json::json;
use wasm_bindgen::prelude::*;

use crate::util::{err, to_js};

/// A move, as ducy-web sends it.
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Op {
    AddLink {
        member: String,
        backer: String,
    },
    SetCredit {
        member: String,
        backer: String,
        credit: u64,
    },
    SetTerms {
        member: String,
        backer: String,
        terms: Option<Terms>,
    },
    SetShare {
        member: String,
        backer: String,
        share: Option<u32>,
    },
    SetDefaultShare {
        backer: String,
        share: Option<u32>,
    },
    SetThrough {
        member: String,
        backer: String,
    },
    MakeAgency {
        id: String,
    },
    AddToAgency {
        agency: String,
        person: String,
    },
    SetOnShift {
        agency: String,
        person: Option<String>,
    },
    Give {
        backer: String,
        member: String,
        amount: u64,
        #[serde(default)]
        note: String,
    },
    TakeBack {
        backer: String,
        member: String,
        amount: u64,
        #[serde(default)]
        note: String,
    },
    BuyIn {
        member: String,
        table: String,
        amount: u64,
    },
    TopUp {
        member: String,
        amount: u64,
    },
    RefundTopUp {
        member: String,
        amount: u64,
    },
    SetStack {
        member: String,
        stack: u64,
    },
    CashOut {
        member: String,
        #[serde(default)]
        contributed: u64,
    },
    Handover {
        member: String,
        #[serde(default)]
        contributed: u64,
    },
    TourneyBuyIn {
        member: String,
        tourney: String,
        amount: u64,
    },
    Pay {
        member: String,
        kind: EntryKind,
        about: String,
        amount: u64,
    },
    Void {
        member: String,
        about: String,
        amount: u64,
    },
    OpenBalance {
        member: String,
        backer: String,
        balance: i64,
    },
    OpenSession {
        member: String,
        table: String,
        stack: u64,
        put: u64,
    },
}

/// A club's links, sessions and ledger.
#[wasm_bindgen]
pub struct ClubMoney {
    money: Money,
}

fn none<T>(r: Result<T, MoneyError>) -> Result<Vec<Entry>, MoneyError> {
    r.map(|_| Vec::new())
}

#[wasm_bindgen]
impl ClubMoney {
    /// No links yet; settle days in the club's time, `utcOffsetMin` from UTC.
    #[wasm_bindgen(constructor)]
    pub fn new(utc_offset_min: i32) -> ClubMoney {
        ClubMoney {
            money: Money::new(utc_offset_min),
        }
    }

    /// The money saved with `save`.
    pub fn restore(json: &str) -> Result<ClubMoney, JsError> {
        Ok(ClubMoney {
            money: serde_json::from_str(json).map_err(err)?,
        })
    }

    /// Everything, as JSON.
    pub fn save(&self) -> Result<String, JsError> {
        serde_json::to_string(&self.money).map_err(err)
    }

    /// Everything, as a plain object (what `save` writes, parsed), for a
    /// club object to keep.
    pub fn data(&self) -> Result<JsValue, JsError> {
        to_js(&self.money)
    }

    /// The money from `data`.
    #[wasm_bindgen(js_name = fromData)]
    pub fn from_data(data: JsValue) -> Result<ClubMoney, JsError> {
        Ok(ClubMoney {
            money: serde_wasm_bindgen::from_value(data).map_err(err)?,
        })
    }

    /// Makes a move: `{ok: true, entries}` with the ledger entries it wrote,
    /// or `{ok: false, error: {code, …}, message}` with nothing changed.
    /// Throws only for an op it can't read.
    pub fn run(&mut self, op: JsValue, now: f64) -> Result<JsValue, JsError> {
        let op: Op = serde_wasm_bindgen::from_value(op).map_err(err)?;
        let now = now as u64;
        let m = &mut self.money;
        let r = match op {
            Op::AddLink { member, backer } => none(m.add_link(&member, &backer)),
            Op::SetCredit {
                member,
                backer,
                credit,
            } => none(m.set_credit(&member, &backer, credit)),
            Op::SetTerms {
                member,
                backer,
                terms,
            } => none(m.set_terms(&member, &backer, terms)),
            Op::SetShare {
                member,
                backer,
                share,
            } => none(m.set_share(&member, &backer, share)),
            Op::SetDefaultShare { backer, share } => none(m.set_default_share(&backer, share)),
            Op::SetThrough { member, backer } => none(m.set_through(&member, &backer)),
            Op::MakeAgency { id } => none(m.make_agency(&id)),
            Op::AddToAgency { agency, person } => none(m.add_to_agency(&agency, &person)),
            Op::SetOnShift { agency, person } => none(m.set_on_shift(&agency, person.as_deref())),
            Op::Give {
                backer,
                member,
                amount,
                note,
            } => m
                .give(&backer, &member, amount, &note, now)
                .map(|e| vec![e]),
            Op::TakeBack {
                backer,
                member,
                amount,
                note,
            } => m
                .take_back(&backer, &member, amount, &note, now)
                .map(|e| vec![e]),
            Op::BuyIn {
                member,
                table,
                amount,
            } => m.buy_in(&member, &table, amount, now),
            Op::TopUp { member, amount } => m.top_up(&member, amount, now),
            Op::RefundTopUp { member, amount } => m.refund_top_up(&member, amount, now),
            Op::SetStack { member, stack } => none(m.set_stack(&member, stack)),
            Op::CashOut {
                member,
                contributed,
            } => m.cash_out(&member, contributed, now),
            Op::Handover {
                member,
                contributed,
            } => m.handover(&member, contributed, now),
            Op::TourneyBuyIn {
                member,
                tourney,
                amount,
            } => m.tourney_buy_in(&member, &tourney, amount, now),
            Op::Pay {
                member,
                kind,
                about,
                amount,
            } => m.pay(&member, kind, &about, amount, now),
            Op::Void {
                member,
                about,
                amount,
            } => m.void(&member, &about, amount, now),
            Op::OpenBalance {
                member,
                backer,
                balance,
            } => m
                .open_balance(&member, &backer, balance, now)
                .map(|e| e.into_iter().collect()),
            Op::OpenSession {
                member,
                table,
                stack,
                put,
            } => none(m.open_session(&member, &table, stack, put)),
        };
        to_js(&match r {
            Ok(entries) => json!({"ok": true, "entries": entries}),
            Err(e) => json!({"ok": false, "error": e, "message": e.to_string()}),
        })
    }

    /// The path `member` plays through now, from them up to `club`; throws
    /// if they have none.
    pub fn path(&self, member: &str) -> Result<Vec<String>, JsError> {
        self.money
            .path(member)
            .map_err(|e| JsError::new(&e.to_string()))
    }

    /// How much `member` can bring to a table now (0 when a link on their
    /// path is overdue).
    pub fn room(&self, member: &str, now: f64) -> Result<f64, JsError> {
        self.money
            .room(member, now as u64)
            .map(|r| r as f64)
            .map_err(|e| JsError::new(&e.to_string()))
    }

    /// One link, or null.
    pub fn link(&self, member: &str, backer: &str) -> Result<JsValue, JsError> {
        to_js(&self.money.link(member, backer))
    }

    /// Every link.
    pub fn links(&self) -> Result<JsValue, JsError> {
        to_js(&self.money.links())
    }

    /// `member`'s links up to their backers.
    #[wasm_bindgen(js_name = backersOf)]
    pub fn backers_of(&self, member: &str) -> Result<JsValue, JsError> {
        to_js(&self.money.backers_of(member).collect::<Vec<_>>())
    }

    /// The links of everyone `backer` backs.
    #[wasm_bindgen(js_name = membersOf)]
    pub fn members_of(&self, backer: &str) -> Result<JsValue, JsError> {
        to_js(&self.money.members_of(backer).collect::<Vec<_>>())
    }

    /// A member's or agent's settings ({through, defaultShare, agency}), or null.
    pub fn account(&self, id: &str) -> Result<JsValue, JsError> {
        to_js(&self.money.account(id))
    }

    /// A seated player's session ({about, path, stack, put}), or null.
    pub fn session(&self, member: &str) -> Result<JsValue, JsError> {
        to_js(&self.money.session(member))
    }

    /// When the link became overdue, or null if it isn't (or doesn't exist).
    pub fn overdue(&self, member: &str, backer: &str, now: f64) -> Option<f64> {
        let l = self.money.link(member, backer)?;
        self.money.overdue(l, now as u64).map(|t| t as f64)
    }

    /// The entries of one link after entry `since`.
    pub fn entries(&self, member: &str, backer: &str, since: f64) -> Result<JsValue, JsError> {
        to_js(&self.money.entries(member, backer, since as u64))
    }

    /// The whole ledger, after entry `since`.
    pub fn ledger(&self, since: f64) -> Result<JsValue, JsError> {
        let since = since as u64;
        to_js(
            &self
                .money
                .ledger()
                .iter()
                .filter(|e| e.n > since)
                .collect::<Vec<_>>(),
        )
    }

    /// What doesn't add up; empty when the books are right.
    pub fn audit(&self) -> Vec<String> {
        self.money.audit()
    }
}
