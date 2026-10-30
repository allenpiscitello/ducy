//! A club's services (ducy-web #456): outside servers the owner of an
//! always-on club lets give members chips, within limits. The rules
//! ([`crate::club_rules`]) keep each service and check its requests; here a
//! checked request is carried out in one step (the chips moved through the
//! book, the log, the budget and the once-only key), and the owner's
//! changes are logged.
//!
//! The caller checks a request's signature and finds the member first.

use serde::Serialize;
use serde_json::{Value, json};

use crate::club_host::ClubState;
use crate::club_rules::{Prepared, Refused, RuleError, ServiceStatus, Status, chip_text};

/// What a service's request comes to.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ServiceAnswer {
    /// The answer to send back.
    pub value: Value,
    /// The member whose balance changed, to tell them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member: Option<String>,
    /// The members the caller lists, for "members" ({id, name, balance}):
    /// it adds the IDs people see.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub members: Option<Vec<Value>>,
}

fn chips(n: i64) -> f64 {
    n as f64 / 100.0
}

impl ClubState {
    fn owner(&self) -> String {
        self.book
            .club
            .owner_id
            .clone()
            .unwrap_or_else(|| self.book.club.host_id.clone())
    }

    /// The owner adds a service by its key: `o` its name and limits.
    pub fn add_service(
        &mut self,
        id: &str,
        alg: &str,
        key: &str,
        o: &Value,
        now: u64,
    ) -> Result<Value, RuleError> {
        let s = self.book.club.add_service(id, alg, key, o, now)?;
        let (name, out) = (
            s.name.clone(),
            serde_json::to_value(s).unwrap_or(Value::Null),
        );
        let by = self.owner();
        self.book
            .log_action(&by, &format!("Added the service {name}"), now, None);
        Ok(out)
    }

    /// The owner changes a service's limits; `addBudget` tops its budget up.
    pub fn update_service(&mut self, id: &str, o: &Value, now: u64) -> Result<Value, RuleError> {
        let s = self.book.club.update_service(id, o)?;
        let (name, out) = (
            s.name.clone(),
            serde_json::to_value(s).unwrap_or(Value::Null),
        );
        let add = o
            .get("addBudget")
            .and_then(|v| v.as_f64().or_else(|| v.as_str()?.trim().parse().ok()))
            .unwrap_or(0.0);
        let line = if add > 0.0 {
            format!("Added {} to {name}’s budget", chip_text(add as u64))
        } else {
            format!("Changed {name}’s limits")
        };
        let by = self.owner();
        self.book.log_action(&by, &line, now, None);
        Ok(out)
    }

    /// Revoking: the key is forgotten at once; the record stays.
    pub fn revoke_service(&mut self, id: &str, now: u64) -> Result<Value, RuleError> {
        let s = self.book.club.revoke_service(id, now)?;
        let (name, out) = (
            s.name.clone(),
            serde_json::to_value(s).unwrap_or(Value::Null),
        );
        let by = self.owner();
        self.book
            .log_action(&by, &format!("Revoked the service {name}"), now, None);
        Ok(out)
    }

    /// Carries out service `id`'s checked request `body` for `member` (its
    /// id, found by the caller): its status, the members, or chips given or
    /// taken back, logged, with its budget and once-only key. Refused with
    /// the HTTP status to answer with.
    pub fn service_request(
        &mut self,
        id: &str,
        body: &Value,
        member: Option<&str>,
        now: u64,
    ) -> Result<ServiceAnswer, Refused> {
        // Checked already, but never carried out for a service revoked since.
        let active = |s: &crate::club_rules::Service| s.status == ServiceStatus::Active;
        if !self.book.club.services.get(id).is_some_and(active) {
            return Err(Refused {
                status: 401,
                reason: "This club has no such service.".into(),
            });
        }
        let p = self
            .book
            .club
            .prepare_service_request(id, body, member, now)?;
        let (op, member, amount, note, said, key) = match p {
            Prepared::Value(value) => {
                return Ok(ServiceAnswer {
                    value,
                    ..ServiceAnswer::default()
                });
            }
            Prepared::Members => {
                let list = self
                    .book
                    .club
                    .members
                    .values()
                    .filter(|m| m.status == Status::Member)
                    .map(|m| json!({"id": m.id, "name": m.name, "balance": chips(self.book.balance(&m.id))}))
                    .collect();
                return Ok(ServiceAnswer {
                    members: Some(list),
                    ..ServiceAnswer::default()
                });
            }
            Prepared::Move {
                op,
                member,
                amount,
                note,
                said,
                key,
            } => (op, member, amount, note, said, key),
        };
        let refusal = |status: u16, e: RuleError| Refused {
            status,
            reason: e.0,
        };
        let name = self
            .book
            .club
            .services
            .get(id)
            .map_or(String::new(), |s| s.name.clone());
        let who = self
            .book
            .club
            .members
            .get(&member)
            .map_or(member.clone(), |m| m.name.clone());
        let said_part = if said.is_empty() {
            String::new()
        } else {
            format!(": {said}")
        };
        let (entry, line) = if op == "give" {
            let e = self
                .book
                .issue(&member, Some(amount as i64), &note, now)
                .map_err(|e| refusal(409, e))?;
            (
                e,
                format!(
                    "{name} (service) gave {who} {}{said_part}",
                    chip_text(amount)
                ),
            )
        } else {
            let e = self
                .book
                .reclaim(&member, Some(amount as i64), &note, now)
                .map_err(|e| refusal(409, e))?;
            (
                e,
                format!(
                    "{name} (service) took back {} from {who}{said_part}",
                    chip_text(amount)
                ),
            )
        };
        self.book.log_action(id, &line, now, Some(amount as i64));
        let budget = self.book.club.services.get(id).map_or(0, |s| s.budget);
        let budget = if op == "give" {
            budget.saturating_sub(amount)
        } else {
            budget
        };
        let value = json!({
            "op": op, "member": member, "amount": chips(amount as i64),
            "balance": chips(self.book.balance(&member)), "entry": entry.n, "budget": chips(budget as i64),
        });
        self.book
            .club
            .service_done(id, &op, amount, key.as_deref(), &value, now);
        Ok(ServiceAnswer {
            value,
            member: Some(member),
            members: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000_000;

    fn club() -> ClubState {
        let mut c: ClubState = serde_json::from_value(json!({
            "code": "ABCD-EFGH", "hostId": "ROOM", "ownerId": "OWNER",
            "members": {"ann": {"id": "ann", "name": "Ann", "status": "member"}},
        }))
        .unwrap();
        c.book.ensure_links(0).unwrap();
        c.book.sync();
        c
    }

    fn logged(c: &ClubState) -> Vec<String> {
        c.book
            .log
            .iter()
            .map(|l| l["text"].as_str().unwrap_or("").to_string())
            .collect()
    }

    #[test]
    fn a_service_gives_takes_and_is_logged_once() {
        let mut c = club();
        let o = json!({"name": "Rewards", "perms": {"give": true, "take": true, "read": true}, "budget": 50_000});
        c.add_service("SVC", "ed25519", "KEY", &o, NOW).unwrap();
        c.update_service("SVC", &json!({"addBudget": 10_000}), NOW)
            .unwrap();
        let give = json!({"op": "give", "args": {"member": "ann", "amount": 25, "note": "league", "key": "k1"}});
        let a = c.service_request("SVC", &give, Some("ann"), NOW).unwrap();
        assert_eq!(a.member.as_deref(), Some("ann"));
        assert_eq!(a.value["balance"], 25.0);
        assert_eq!(a.value["budget"], 575.0);
        assert_eq!(c.book.balance("ann"), 2500);
        // The same key again: the first answer, no second payment.
        let again = c
            .service_request("SVC", &give, Some("ann"), NOW + 1)
            .unwrap();
        assert_eq!(again.value["repeat"], true);
        assert_eq!(c.book.balance("ann"), 2500);
        // Taking back more than she has: refused, nothing moved or logged.
        let take = json!({"op": "take", "args": {"member": "ann", "amount": 30}});
        let r = c
            .service_request("SVC", &take, Some("ann"), NOW)
            .unwrap_err();
        assert_eq!(r.status, 409);
        let take = json!({"op": "take", "args": {"member": "ann", "amount": 5}});
        c.service_request("SVC", &take, Some("ann"), NOW).unwrap();
        assert_eq!(c.book.balance("ann"), 2000);
        let members = c
            .service_request("SVC", &json!({"op": "members"}), None, NOW)
            .unwrap()
            .members
            .unwrap();
        assert_eq!(
            members,
            vec![json!({"id": "ann", "name": "Ann", "balance": 20.0})]
        );
        c.revoke_service("SVC", NOW).unwrap();
        assert_eq!(
            logged(&c),
            [
                "Added the service Rewards",
                "Added 100 to Rewards’s budget",
                "Rewards (service) gave Ann 25: league",
                "Rewards (service) took back 5 from Ann",
                "Revoked the service Rewards",
            ]
        );
        assert!(c.service_request("SVC", &give, Some("ann"), NOW).is_err());
    }
}
