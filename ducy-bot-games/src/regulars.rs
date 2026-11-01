//! Play the Regulars (ducy-web #225): a 6-handed table of random Regulars
//! (the personality bots), run like a card room. Each Regular brings a
//! bankroll of [`BANKROLL_BUY_INS`] buy-ins. Between hands, one below half a
//! buy-in tops back up to a full buy-in from what's left of it; one with no
//! chips and nothing left leaves, and a new random Regular takes the seat
//! with a full bankroll. And after any hand, whatever their stack, a Regular
//! may just get up and go ([`LEAVE_CHANCE`]), as people do, so the table
//! keeps changing. The person rebuys or tops up when they choose (the page).

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

/// Buy-ins in each Regular's bankroll.
pub const BANKROLL_BUY_INS: i64 = 3;
/// The chance each Regular leaves after a hand, whatever their stack.
pub const LEAVE_CHANCE: f64 = 0.015;
/// Bot seats at the table: with you, 6-handed.
pub const REGULARS_SEATS: usize = 5;

/// What each bot seat has left after its first buy-in (seat 0 is you, with
/// `bots` seats after it). `buy_ins`, when given, is each seat's own buy-in.
pub fn start_bank(bots: usize, buy_in: i64, buy_ins: Option<&[i64]>) -> Vec<i64> {
    let mut bank = vec![0];
    for i in 0..bots {
        let b = buy_ins
            .and_then(|b| b.get(i + 1).copied())
            .unwrap_or(buy_in);
        bank.push((BANKROLL_BUY_INS - 1) * b);
    }
    bank
}

fn pick(n: usize, r: f64) -> usize {
    ((r * n as f64).floor() as usize).min(n - 1)
}

/// `n` Regulars picked at random from `ids`, all different when there are
/// enough. `rng` gives numbers in `0..1`.
pub fn pick_regulars(ids: &[String], n: usize, rng: &mut impl FnMut() -> f64) -> Vec<String> {
    let mut pool = ids.to_vec();
    let mut out = Vec::new();
    while out.len() < n && !pool.is_empty() {
        let i = pick(pool.len(), rng());
        out.push(pool.remove(i));
    }
    while out.len() < n && !ids.is_empty() {
        out.push(ids[pick(ids.len(), rng())].clone());
    }
    out
}

/// The table before the next hand.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Table {
    /// Every seat's chips (seat 0 yours).
    pub stacks: Vec<i64>,
    /// What each bot seat has left (from [`start_bank`]).
    pub bank: Vec<i64>,
    /// The Regular in each bot seat (seat i + 1).
    pub bots: Vec<String>,
    /// The table's buy-in.
    pub buy_in: i64,
    /// Every Regular who might sit down.
    pub all: Vec<String>,
    /// Each Regular's chance of leaving anyway.
    #[serde(default = "leave_chance")]
    pub leave_chance: f64,
    /// Players who buy in for different amounts (a Bankroll challenge
    /// table): each seat's own buy-in (seat 0 unused).
    #[serde(default)]
    pub buy_ins: Option<Vec<i64>>,
    /// And what a newcomer buys in for there.
    #[serde(default)]
    pub buy_in_of: Option<BTreeMap<String, i64>>,
    /// Each Regular's name: never two of the same name at a table.
    #[serde(default)]
    pub names: Option<BTreeMap<String, String>>,
}

fn leave_chance() -> f64 {
    LEAVE_CHANCE
}

/// A seat's stack to set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SetStack {
    /// The seat.
    pub seat: usize,
    /// Its chips.
    pub chips: i64,
}

/// A Regular leaving, and who takes the seat.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Swap {
    /// The seat.
    pub seat: usize,
    /// Who left.
    pub from: String,
    /// Who sits down.
    pub id: String,
    /// What they buy in for.
    pub chips: i64,
}

/// What changes before the next hand.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Between {
    /// The stacks to set.
    pub sets: Vec<SetStack>,
    /// The Regulars leaving and who replaces them.
    pub swaps: Vec<Swap>,
    /// The new bank.
    pub bank: Vec<i64>,
    /// The new seating.
    pub bots: Vec<String>,
    /// The new buy-ins by seat, at a table that has them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buy_ins: Option<Vec<i64>>,
}

fn put(v: &mut Vec<i64>, seat: usize, x: i64) {
    if v.len() <= seat {
        v.resize(seat + 1, 0);
    }
    v[seat] = x;
}

/// What changes before the next hand: top-ups from the bankrolls, and the
/// Regulars leaving (broke, or done for the day) with who takes their seats.
/// `rng` gives numbers in `0..1`.
pub fn between_hands(t: &Table, rng: &mut impl FnMut() -> f64) -> Between {
    let mut out = Between {
        sets: Vec::new(),
        swaps: Vec::new(),
        bank: t.bank.clone(),
        bots: t.bots.clone(),
        buy_ins: t.buy_ins.clone(),
    };
    for seat in 1..t.stacks.len() {
        let stack = t.stacks[seat];
        let target = out
            .buy_ins
            .as_ref()
            .and_then(|b| b.get(seat).copied())
            .unwrap_or(t.buy_in);
        // Done for the day, whatever's in front of them.
        if stack > 0 && t.leave_chance > 0.0 && rng() < t.leave_chance {
            replace(t, &mut out, seat, rng);
            continue;
        }
        if stack * 2 >= target {
            continue;
        }
        let left = out.bank.get(seat).copied().unwrap_or(0);
        if left > 0 {
            let add = (target - stack).min(left);
            put(&mut out.bank, seat, left - add);
            out.sets.push(SetStack {
                seat,
                chips: stack + add,
            });
            continue;
        }
        if stack > 0 {
            continue; // playing out the last of the bankroll
        }
        if replace(t, &mut out, seat, rng) {
            continue;
        }
        // Broke, and no one to take the seat: they buy in again.
        out.sets.push(SetStack {
            seat,
            chips: target,
        });
        put(&mut out.bank, seat, (BANKROLL_BUY_INS - 1) * target);
    }
    out
}

/// A new Regular takes `seat`. Never two of the same player at a table (or
/// two of the same name): with no one else to sit down, the seat keeps its
/// player (false).
fn replace(t: &Table, out: &mut Between, seat: usize, rng: &mut impl FnMut() -> f64) -> bool {
    let name = |id: &String| -> String {
        t.names
            .as_ref()
            .and_then(|n| n.get(id))
            .cloned()
            .unwrap_or_else(|| id.clone())
    };
    let here: HashSet<String> = out.bots.iter().map(name).collect();
    let free: Vec<String> = t
        .all
        .iter()
        .filter(|id| !out.bots.contains(id) && !here.contains(&name(id)))
        .cloned()
        .collect();
    if free.is_empty() {
        return false;
    }
    let id = pick_regulars(&free, 1, rng).remove(0);
    let chips = t
        .buy_in_of
        .as_ref()
        .and_then(|b| b.get(&id).copied())
        .unwrap_or(t.buy_in);
    if out.bots.len() < seat {
        out.bots.resize(seat, String::new());
    }
    let from = std::mem::replace(&mut out.bots[seat - 1], id.clone());
    out.swaps.push(Swap {
        seat,
        from,
        id,
        chips,
    });
    if let Some(b) = &mut out.buy_ins {
        put(b, seat, chips);
    }
    put(&mut out.bank, seat, (BANKROLL_BUY_INS - 1) * chips);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(s: &str) -> Vec<String> {
        s.split(' ').map(String::from).collect()
    }
    fn all() -> Vec<String> {
        ids("doug_poker milk_king jen_silly phil_bigmouth nik_airbag tom_collins rampart")
    }
    fn seq(xs: &[f64]) -> impl FnMut() -> f64 + '_ {
        let mut i = 0;
        move || {
            let x = xs[i % xs.len()];
            i += 1;
            x
        }
    }
    fn table(stacks: &[i64], bank: &[i64], bots: &str, buy_in: i64) -> Table {
        Table {
            stacks: stacks.to_vec(),
            bank: bank.to_vec(),
            bots: ids(bots),
            buy_in,
            all: all(),
            leave_chance: 0.0,
            ..Table::default()
        }
    }
    fn set(seat: usize, chips: i64) -> SetStack {
        SetStack { seat, chips }
    }

    #[test]
    fn each_regular_brings_three_buy_ins() {
        assert_eq!(start_bank(2, 100, None), [0, 200, 200]);
    }

    #[test]
    fn regulars_are_picked_at_random_all_different() {
        let p = pick_regulars(&all(), 5, &mut seq(&[0.5, 0.1, 0.9, 0.3, 0.7]));
        assert_eq!(p.len(), 5);
        assert_eq!(p.iter().collect::<HashSet<_>>().len(), 5);
        assert_eq!(pick_regulars(&ids("a b"), 3, &mut seq(&[0.0])).len(), 3);
    }

    #[test]
    fn a_regular_above_half_a_buy_in_is_left_alone() {
        let r = between_hands(
            &table(&[100, 50, 400], &[0, 200, 200], "a b", 100),
            &mut seq(&[0.0]),
        );
        assert!(r.sets.is_empty() && r.swaps.is_empty());
        assert_eq!(r.bank, [0, 200, 200]);
    }

    #[test]
    fn below_half_a_buy_in_a_regular_tops_up_from_the_bankroll() {
        let r = between_hands(
            &table(&[100, 30, 0], &[0, 200, 200], "a b", 100),
            &mut seq(&[0.0]),
        );
        assert_eq!(r.sets, [set(1, 100), set(2, 100)]);
        assert_eq!(r.bank, [0, 130, 100]);
    }

    #[test]
    fn with_buy_ins_by_seat_each_regular_tops_up_to_their_own() {
        let t = Table {
            buy_ins: Some(vec![0, 40, 400, 200]),
            ..table(&[100, 25, 150, 0], &[0, 80, 800, 0], "a b c", 100)
        };
        let r = between_hands(&t, &mut seq(&[0.0]));
        assert_eq!(r.sets, [set(2, 400)]);
        assert_eq!(r.swaps.len(), 1);
        assert_eq!(r.swaps[0].chips, 100);
        assert_eq!(r.buy_ins, Some(vec![0, 40, 400, 100]));
        let plain = between_hands(&table(&[100, 60], &[0, 200], "a", 100), &mut seq(&[0.0]));
        assert_eq!(plain.buy_ins, None);
    }

    #[test]
    fn the_last_of_a_bankroll_tops_up_only_what_is_left() {
        let r = between_hands(&table(&[100, 10], &[0, 40], "a", 100), &mut seq(&[0.0]));
        assert_eq!(r.sets, [set(1, 50)]);
        assert_eq!(r.bank, [0, 0]);
    }

    #[test]
    fn with_chips_left_and_nothing_behind_a_regular_plays_them_out() {
        let r = between_hands(&table(&[100, 20], &[0, 0], "a", 100), &mut seq(&[0.0]));
        assert!(r.sets.is_empty() && r.swaps.is_empty());
    }

    #[test]
    fn broke_with_nothing_behind_a_regular_leaves_and_a_new_one_sits_down() {
        let r = between_hands(
            &table(&[100, 0, 300], &[0, 0, 200], "doug_poker milk_king", 100),
            &mut seq(&[0.0]),
        );
        assert_eq!(r.swaps.len(), 1);
        let s = &r.swaps[0];
        assert_eq!((s.seat, s.from.as_str(), s.chips), (1, "doug_poker", 100));
        assert!(!["doug_poker", "milk_king"].contains(&s.id.as_str()));
        assert_eq!(r.bank[1], 200);
        assert_eq!(r.bots, [s.id.clone(), "milk_king".into()]);
    }

    #[test]
    fn your_own_stack_is_never_topped_up() {
        let r = between_hands(&table(&[0, 100], &[0, 200], "a", 100), &mut seq(&[0.0]));
        assert!(r.sets.is_empty() && r.swaps.is_empty());
    }

    #[test]
    fn after_any_hand_a_regular_may_get_up_and_go() {
        let t = Table {
            leave_chance: LEAVE_CHANCE,
            ..table(
                &[100, 900, 300],
                &[0, 200, 200],
                "doug_poker milk_king",
                100,
            )
        };
        let r = between_hands(&t, &mut seq(&[0.001, 0.0, 0.5]));
        assert_eq!(r.swaps.len(), 1);
        let s = &r.swaps[0];
        assert_eq!((s.seat, s.from.as_str(), s.chips), (1, "doug_poker", 100));
        assert_eq!(r.bank[1], 200);
        assert_eq!(r.bots, [s.id.clone(), "milk_king".into()]);
        let broke = Table {
            leave_chance: LEAVE_CHANCE,
            ..table(&[100, 0], &[0, 200], "a", 100)
        };
        let r = between_hands(&broke, &mut seq(&[0.0]));
        assert!(r.swaps.is_empty());
        assert_eq!(r.sets, [set(1, 100)]);
    }

    #[test]
    fn over_many_hands_about_leave_chance_of_regulars_leave() {
        let mut x: u64 = 7;
        let mut rng = move || {
            x = (x * 1103515245 + 12345) % 2147483648;
            x as f64 / 2147483648.0
        };
        let t = Table {
            leave_chance: LEAVE_CHANCE,
            ..table(
                &[100, 200, 200, 200, 200, 200],
                &[0, 200, 200, 200, 200, 200],
                "a b c d e",
                200,
            )
        };
        let hands = 20000;
        let left: usize = (0..hands)
            .map(|_| between_hands(&t, &mut rng).swaps.len())
            .sum();
        let rate = left as f64 / (hands * 5) as f64;
        assert!((rate - LEAVE_CHANCE).abs() < 0.003, "rate {rate}");
    }

    #[test]
    fn never_two_of_the_same_player_at_a_table() {
        let mut t = table(&[100, 0, 300], &[0, 0, 200], "a b", 100);
        t.all = ids("a b");
        let r = between_hands(&t, &mut seq(&[0.5]));
        assert!(r.swaps.is_empty());
        assert_eq!(r.sets, [set(1, 100)]);
        assert_eq!(r.bank[1], 200);
        let mut stays = table(&[100, 80, 300], &[0, 200, 200], "a b", 100);
        stays.all = ids("a b");
        stays.leave_chance = 1.0;
        let r = between_hands(&stays, &mut seq(&[0.0]));
        assert!(r.swaps.is_empty());
        assert_eq!(r.bots, ids("a b"));
        // Nobody with the same name, over many hands.
        let all = ids("a b c d a2 e");
        let names: BTreeMap<String, String> = all
            .iter()
            .map(|id| (id.clone(), id.trim_end_matches('2').to_string()))
            .collect();
        let mut bots = ids("a b c");
        let mut stacks = vec![100, 0, 0, 0];
        let mut x: u64 = 11;
        let mut rng = move || {
            x = (x * 1103515245 + 12345) % 2147483648;
            x as f64 / 2147483648.0
        };
        for h in 0..200 {
            let t = Table {
                stacks: stacks.clone(),
                bank: vec![0; 4],
                bots: bots.clone(),
                buy_in: 100,
                all: all.clone(),
                leave_chance: 0.3,
                names: Some(names.clone()),
                ..Table::default()
            };
            bots = between_hands(&t, &mut rng).bots;
            let distinct: HashSet<&String> = bots.iter().map(|b| &names[b]).collect();
            assert_eq!(distinct.len(), bots.len(), "{bots:?}");
            stacks = std::iter::once(100)
                .chain((0..bots.len()).map(|i| if (h + i) % 2 == 0 { 0 } else { 100 }))
                .collect();
        }
    }
}
