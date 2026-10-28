//! The club money model's cases, from the design doc's example club: Priya
//! under the club with 5,000 credit, Marco under Priya with 2,000, Ann under
//! Marco with 500, and Ben the club's own with 1,000 chips and no credit.

use ducy_play::club_money::{CLUB, EntryKind, FULL_SHARE, Money, MoneyError, Terms};

const DAY: u64 = 86_400_000;

fn bal(m: &Money, member: &str, backer: &str) -> i64 {
    m.link(member, backer).unwrap().balance
}

fn example() -> Money {
    let mut m = Money::new(0);
    m.add_link("priya", CLUB).unwrap();
    m.add_link("marco", "priya").unwrap();
    m.add_link("ann", "marco").unwrap();
    m.add_link("ben", CLUB).unwrap();
    m.set_credit("priya", CLUB, 5000).unwrap();
    m.set_credit("marco", "priya", 2000).unwrap();
    m.set_credit("ann", "marco", 500).unwrap();
    m.give(CLUB, "ben", 1000, "", 0).unwrap();
    m
}

#[test]
fn a_buy_in_moves_every_link_on_the_path() {
    let mut m = example();
    assert_eq!(m.path("ann").unwrap(), ["ann", "marco", "priya", CLUB]);
    assert_eq!(m.room("ann", 0).unwrap(), 500);
    let out = m.buy_in("ann", "t1", 500, 1).unwrap();
    assert_eq!(out.len(), 3);
    assert!(
        out.iter()
            .all(|e| e.amount == -500 && e.tx == out[0].tx && e.kind == EntryKind::BuyIn)
    );
    assert_eq!(
        [
            bal(&m, "ann", "marco"),
            bal(&m, "marco", "priya"),
            bal(&m, "priya", CLUB)
        ],
        [-500; 3]
    );

    // She stands up with 800: the club owes Priya 300, Priya Marco, Marco Ann.
    m.set_stack("ann", 800).unwrap();
    m.cash_out("ann", 0, 2).unwrap();
    assert_eq!(
        [
            bal(&m, "ann", "marco"),
            bal(&m, "marco", "priya"),
            bal(&m, "priya", CLUB)
        ],
        [300; 3]
    );
    // Winnings can be played again.
    assert_eq!(m.room("ann", 3).unwrap(), 800);
    assert!(m.audit().is_empty());
}

#[test]
fn standing_up_with_nothing_is_still_a_cash_out() {
    let mut m = example();
    m.buy_in("ann", "t1", 500, 1).unwrap();
    m.set_stack("ann", 0).unwrap();
    let out = m.cash_out("ann", 0, 2).unwrap();
    assert_eq!(out.len(), 3);
    assert!(out.iter().all(|e| e.kind == EntryKind::CashOut
        && e.amount == 0
        && e.balance == -500
        && e.about.as_deref() == Some("t1")));
    assert!(m.session("ann").is_none());
    assert!(m.audit().is_empty());
}

#[test]
fn room_is_the_least_on_the_path_and_refusals_move_nothing() {
    let mut m = example();
    assert_eq!(m.room("ben", 0).unwrap(), 1000);
    assert!(matches!(
        m.buy_in("ben", "t1", 1001, 0),
        Err(MoneyError::Room { room: 1000, .. })
    ));
    assert!(
        matches!(m.buy_in("ann", "t1", 501, 0), Err(MoneyError::Room { ref member, room: 500, .. }) if member == "ann")
    );

    // Priya's other players have used 4,800 of her credit: Ann's buy-in stops
    // at Priya's link, and none of the three moves.
    m.add_link("raj", "priya").unwrap();
    m.set_credit("raj", "priya", 5000).unwrap();
    m.buy_in("raj", "t2", 4800, 0).unwrap();
    assert_eq!(m.room("ann", 0).unwrap(), 200);
    let before = m.ledger().len();
    assert!(
        matches!(m.buy_in("ann", "t1", 500, 0), Err(MoneyError::Room { ref member, room: 200, .. }) if member == "priya")
    );
    assert_eq!(m.ledger().len(), before);
    assert_eq!(bal(&m, "ann", "marco"), 0);
    assert!(m.session("ann").is_none());
}

#[test]
fn links_only_lead_up_toward_the_club() {
    let mut m = example();
    assert_eq!(m.add_link("priya", "ann").unwrap_err(), MoneyError::Loop);
    assert_eq!(m.add_link("marco", "ann").unwrap_err(), MoneyError::Loop);
    assert_eq!(m.add_link("ann", "ann").unwrap_err(), MoneyError::BadId);
    assert_eq!(m.add_link(CLUB, "ann").unwrap_err(), MoneyError::BadId);
    assert_eq!(
        m.add_link("ann", "marco").unwrap_err(),
        MoneyError::LinkExists
    );
    // A second backer is fine.
    m.add_link("ann", CLUB).unwrap();
}

#[test]
fn several_backers_one_session_through_one() {
    let mut m = example();
    m.add_link("ann", CLUB).unwrap();
    m.give(CLUB, "ann", 300, "", 0).unwrap();
    // Her first backer stays the one she plays through.
    assert_eq!(m.path("ann").unwrap()[1], "marco");
    m.buy_in("ann", "t1", 200, 0).unwrap();
    assert_eq!(m.set_through("ann", CLUB).unwrap_err(), MoneyError::Seated);
    m.cash_out("ann", 0, 1).unwrap();
    m.set_through("ann", CLUB).unwrap();
    assert_eq!(m.path("ann").unwrap(), ["ann", CLUB]);
    assert_eq!(m.room("ann", 1).unwrap(), 300);
    assert!(m.set_through("ann", "ben").is_err());
}

#[test]
fn a_session_cashes_out_up_the_path_it_came_down() {
    let mut m = example();
    m.add_link("marco", CLUB).unwrap();
    m.set_credit("marco", CLUB, 2000).unwrap();
    m.buy_in("ann", "t1", 500, 0).unwrap();
    // Marco switches his own backer while Ann is seated.
    m.set_through("marco", CLUB).unwrap();
    m.cash_out("ann", 0, 1).unwrap();
    assert_eq!(bal(&m, "marco", "priya"), 0);
    assert_eq!(bal(&m, "marco", CLUB), 0);
}

#[test]
fn top_ups_and_refunds() {
    let mut m = example();
    m.buy_in("ann", "t1", 300, 0).unwrap();
    assert!(matches!(
        m.top_up("ann", 300, 1),
        Err(MoneyError::Room { room: 200, .. })
    ));
    m.top_up("ann", 200, 1).unwrap();
    assert_eq!(m.session("ann").unwrap().stack, 500);
    m.refund_top_up("ann", 50, 2).unwrap();
    assert_eq!(m.session("ann").unwrap().put, 450);
    assert_eq!(bal(&m, "priya", CLUB), -450);
    assert_eq!(m.top_up("ben", 10, 3).unwrap_err(), MoneyError::NotSeated);
    assert!(m.audit().is_empty());
}

#[test]
fn giving_and_taking_back_settle_one_link() {
    let mut m = example();
    m.buy_in("ann", "t1", 300, 0).unwrap();
    m.set_stack("ann", 0).unwrap();
    m.cash_out("ann", 0, 1).unwrap();
    // Ann owes Marco 300 and pays him cash.
    let e = m.give("marco", "ann", 300, "  Cash, Tuesday ", 2).unwrap();
    assert_eq!(
        (e.kind, e.balance, e.note.as_deref()),
        (EntryKind::Give, 0, Some("Cash, Tuesday"))
    );
    // Marco still owes Priya: giving moves one link.
    assert_eq!(bal(&m, "marco", "priya"), -300);

    m.give("marco", "ann", 100, "", 3).unwrap();
    assert_eq!(
        m.take_back("marco", "ann", 101, "", 4).unwrap_err(),
        MoneyError::TakeBack { owed: 100 }
    );
    m.take_back("marco", "ann", 100, "Transfer 4411", 4)
        .unwrap();
    assert_eq!(bal(&m, "ann", "marco"), 0);
    assert!(m.give("ben", "ann", 1, "", 5).is_err());
    assert_eq!(
        m.give(CLUB, "ben", 0, "", 5).unwrap_err(),
        MoneyError::Amount
    );
    // The buy-in, the cash-out at 0, two gives and the take-back.
    assert_eq!(m.entries("ann", "marco", 0).len(), 5);
    assert!(m.audit().is_empty());
}

#[test]
fn overdue_stops_new_play_until_paid_back() {
    let mut m = example();
    m.set_terms("ann", "marco", Some(Terms::Within { ms: 7 * DAY }))
        .unwrap();
    m.buy_in("ann", "t1", 500, 0).unwrap();
    // Seated, the clock runs but she plays on.
    m.set_stack("ann", 200).unwrap();
    m.cash_out("ann", 0, DAY).unwrap();
    assert_eq!(bal(&m, "ann", "marco"), -300);
    assert_eq!(m.room("ann", 7 * DAY - 1).unwrap(), 200);
    assert_eq!(m.room("ann", 7 * DAY).unwrap(), 0);
    assert!(
        matches!(m.buy_in("ann", "t1", 100, 7 * DAY), Err(MoneyError::Overdue { since, .. }) if since == 7 * DAY)
    );

    // Paying part of it isn't enough; paying it all is.
    m.give("marco", "ann", 100, "Cash", 8 * DAY).unwrap();
    assert!(m.buy_in("ann", "t1", 100, 8 * DAY).is_err());
    m.give("marco", "ann", 200, "Cash", 8 * DAY).unwrap();
    m.buy_in("ann", "t1", 100, 8 * DAY).unwrap();
    // Below zero again: a new clock.
    assert_eq!(m.link("ann", "marco").unwrap().below_since, Some(8 * DAY));
}

#[test]
fn a_seated_player_finishes_but_cant_top_up_once_overdue() {
    let mut m = example();
    m.set_terms("ann", "marco", Some(Terms::Within { ms: DAY }))
        .unwrap();
    m.buy_in("ann", "t1", 200, 0).unwrap();
    assert!(matches!(
        m.top_up("ann", 100, DAY),
        Err(MoneyError::Overdue { .. })
    ));
    m.cash_out("ann", 0, DAY + 1).unwrap();
}

#[test]
fn settle_day_is_the_end_of_the_next_one() {
    // Day 0, 1 January 1970, was a Thursday: the next Sunday ends at day 4.
    let mut m = example();
    m.set_terms("ann", "marco", Some(Terms::Weekly { day: 0 }))
        .unwrap();
    m.buy_in("ann", "t1", 100, DAY / 2).unwrap();
    let l = m.link("ann", "marco").unwrap().clone();
    assert_eq!(m.overdue(&l, 4 * DAY - 1), None);
    assert_eq!(m.overdue(&l, 4 * DAY), Some(4 * DAY));

    // Five hours behind UTC, Sunday ends five hours later.
    m.utc_offset_min = -300;
    assert_eq!(m.overdue(&l, 4 * DAY + 5 * 3_600_000 - 1), None);
    assert!(m.overdue(&l, 4 * DAY + 5 * 3_600_000).is_some());

    // Going below zero on the settle day itself: due at its end.
    m.utc_offset_min = 0;
    m.set_terms("ann", "marco", Some(Terms::Weekly { day: 4 }))
        .unwrap();
    let l = m.link("ann", "marco").unwrap().clone();
    assert_eq!(m.overdue(&l, DAY), Some(DAY));
    assert_eq!(
        m.set_terms("ann", "marco", Some(Terms::Weekly { day: 7 }))
            .unwrap_err(),
        MoneyError::BadId
    );
}

#[test]
fn rake_shares_go_up_the_path_as_the_player_stands_up() {
    // Club gives Priya 40%, Priya gives Marco 25%: of Ann's 10 rake, Marco
    // gets 2.5, Priya 1.5 and the club keeps 6.
    let mut m = example();
    m.set_share("priya", CLUB, Some(4000)).unwrap();
    m.set_share("marco", "priya", Some(2500)).unwrap();
    m.buy_in("ann", "t1", 500, 0).unwrap();
    let out = m.cash_out("ann", 1000, 1).unwrap();
    let shares: Vec<_> = out
        .iter()
        .filter(|e| e.kind == EntryKind::RakeShare)
        .map(|e| (e.member.as_str(), e.amount))
        .collect();
    assert_eq!(shares, [("marco", 250), ("priya", 400)]);
    assert_eq!(bal(&m, "marco", "priya"), 250);
    assert_eq!(bal(&m, "priya", CLUB), 400);
    assert_eq!(bal(&m, "ann", "marco"), 0);

    // Rakeback: Marco gives Ann 10% out of his share.
    m.set_share("ann", "marco", Some(1000)).unwrap();
    m.buy_in("ann", "t1", 500, 2).unwrap();
    m.cash_out("ann", 1000, 3).unwrap();
    assert_eq!(bal(&m, "ann", "marco"), 100);
    assert_eq!(bal(&m, "marco", "priya"), 500);
    assert!(m.audit().is_empty());
}

#[test]
fn shares_never_exceed_the_backers_own() {
    let mut m = example();
    m.set_default_share(CLUB, Some(1000)).unwrap();
    m.set_share("marco", "priya", Some(5000)).unwrap();
    // Priya has the club's default 10%, so Marco can't get more.
    let path = m.path("ann").unwrap();
    assert_eq!(m.shares(&path), [0, 1000, 1000]);
    assert_eq!(
        m.set_share("marco", "priya", Some(FULL_SHARE + 1))
            .unwrap_err(),
        MoneyError::Share
    );
}

#[test]
fn tournament_buy_ins_and_prizes_travel_the_path() {
    let mut m = example();
    m.tourney_buy_in("ann", "sunday", 200, 0).unwrap();
    m.tourney_buy_in("ann", "sunday", 200, 1).unwrap();
    assert!(matches!(
        m.tourney_buy_in("ann", "sunday", 200, 2),
        Err(MoneyError::Room { room: 100, .. })
    ));
    m.pay("ann", EntryKind::Prize, "sunday", 2000, 3).unwrap();
    assert_eq!(
        [
            bal(&m, "ann", "marco"),
            bal(&m, "marco", "priya"),
            bal(&m, "priya", CLUB)
        ],
        [1600; 3]
    );
    m.pay("ben", EntryKind::Prize, "high-hand", 50, 4).unwrap();
    assert_eq!(bal(&m, "ben", CLUB), 1050);
    assert_eq!(
        m.pay("ben", EntryKind::Give, "x", 50, 5).unwrap_err(),
        MoneyError::Kind
    );
    assert!(m.audit().is_empty());
}

#[test]
fn an_agency_serves_through_whoever_is_on_shift() {
    let mut m = example();
    m.add_link("north", CLUB).unwrap();
    m.set_credit("north", CLUB, 10_000).unwrap();
    m.make_agency("north").unwrap();
    m.add_to_agency("north", "zoe").unwrap();
    m.add_to_agency("north", "yan").unwrap();
    m.set_credit("zoe", "north", 3000).unwrap();
    m.set_credit("yan", "north", 3000).unwrap();
    m.add_link("kim", "north").unwrap();
    assert!(m.buy_in("north", "t1", 10, 0).is_err());

    // No one on shift: the agency's own link serves.
    assert_eq!(m.path("kim").unwrap(), ["kim", "north", CLUB]);
    // Zoe on shift: Kim plays through her, and the agency's people don't.
    m.set_on_shift("north", Some("zoe")).unwrap();
    assert_eq!(m.path("kim").unwrap(), ["kim", "zoe", "north", CLUB]);
    assert_eq!(m.path("yan").unwrap(), ["yan", "north", CLUB]);
    // Zoe's first time serving Kim: a new link with no credit, until she
    // answers his request with chips of her own.
    assert_eq!(m.room("kim", 0).unwrap(), 0);
    assert!(m.give("yan", "kim", 100, "", 0).is_err());
    m.give("zoe", "kim", 400, "", 0).unwrap();
    assert_eq!(m.room("kim", 0).unwrap(), 400);
    m.buy_in("kim", "t1", 400, 0).unwrap();
    assert_eq!(
        [
            bal(&m, "kim", "zoe"),
            bal(&m, "zoe", "north"),
            bal(&m, "north", CLUB)
        ],
        [0, -400, -400]
    );

    assert_eq!(
        m.set_on_shift("north", Some("ann")).unwrap_err(),
        MoneyError::NotInAgency
    );
    assert!(m.audit().is_empty());
}

#[test]
fn a_shift_change_hands_the_session_over() {
    let mut m = example();
    m.add_link("north", CLUB).unwrap();
    m.set_credit("north", CLUB, 10_000).unwrap();
    m.make_agency("north").unwrap();
    m.add_to_agency("north", "zoe").unwrap();
    m.add_to_agency("north", "yan").unwrap();
    m.set_credit("zoe", "north", 3000).unwrap();
    m.add_link("kim", "zoe").unwrap();
    m.set_credit("kim", "zoe", 1000).unwrap();
    m.add_link("kim", "north").unwrap();
    m.set_through("kim", "north").unwrap();
    m.set_on_shift("north", Some("zoe")).unwrap();

    m.buy_in("kim", "t1", 1000, 0).unwrap();
    assert_eq!(bal(&m, "kim", "zoe"), -1000);

    // Yan's shift: he has no credit with Kim, but a handover is never refused.
    m.set_on_shift("north", Some("yan")).unwrap();
    m.set_stack("kim", 1500).unwrap();
    let out = m.handover("kim", 0, 1).unwrap();
    assert!(!out.is_empty());
    assert_eq!(bal(&m, "kim", "zoe"), 500);
    assert_eq!(bal(&m, "kim", "yan"), -1500);
    assert_eq!(
        m.session("kim").unwrap().path,
        ["kim", "yan", "north", CLUB]
    );
    assert!(m.handover("kim", 0, 2).unwrap().is_empty());

    m.set_stack("kim", 1200).unwrap();
    m.cash_out("kim", 0, 3).unwrap();
    assert_eq!(bal(&m, "kim", "yan"), -300);
    assert_eq!(bal(&m, "zoe", "north"), 500);
    assert_eq!(bal(&m, "yan", "north"), -300);
    assert_eq!(bal(&m, "north", CLUB), 200);
    assert!(m.audit().is_empty());
}

#[test]
fn a_voided_prize_goes_back_up_the_path() {
    let mut m = example();
    m.pay("ann", EntryKind::Prize, "high-hand", 300, 0).unwrap();
    let out = m.void("ann", "high-hand", 300, 1).unwrap();
    assert!(
        out.iter()
            .all(|e| e.kind == EntryKind::Void && e.amount == -300)
    );
    assert_eq!(
        [
            bal(&m, "ann", "marco"),
            bal(&m, "marco", "priya"),
            bal(&m, "priya", CLUB)
        ],
        [0; 3]
    );
    // Not once she's played it away: her link no longer has it.
    m.pay("ann", EntryKind::Prize, "high-hand", 300, 2).unwrap();
    m.buy_in("ann", "t1", 200, 3).unwrap();
    assert_eq!(
        m.void("ann", "high-hand", 300, 4).unwrap_err(),
        MoneyError::TakeBack { owed: 100 }
    );
    assert!(m.audit().is_empty());
}

#[test]
fn a_club_moves_to_links_with_its_balances_and_seats() {
    let mut m = example();
    // Ann owed Marco 200 and is seated with 350 she brought 300 of.
    assert_eq!(
        m.open_balance("ann", "marco", -200, 0)
            .unwrap()
            .unwrap()
            .balance,
        -200
    );
    assert_eq!(m.open_balance("marco", "priya", 0, 0).unwrap(), None);
    m.open_session("ann", "t1", 350, 300).unwrap();
    assert_eq!(
        m.session("ann").unwrap().path,
        ["ann", "marco", "priya", CLUB]
    );
    assert_eq!(
        m.open_session("ann", "t1", 1, 1).unwrap_err(),
        MoneyError::Seated
    );
    // Once a link has entries, its balance isn't set again.
    assert_eq!(
        m.open_balance("ann", "marco", 5, 1).unwrap_err(),
        MoneyError::LinkExists
    );
    assert!(
        m.open_balance("ann", "priya", 5, 1).is_err(),
        "no such link"
    );
    m.cash_out("ann", 0, 2).unwrap();
    assert_eq!(bal(&m, "ann", "marco"), 150);
    assert_eq!(bal(&m, "marco", "priya"), 350);
    assert!(m.audit().is_empty());
}

#[cfg(feature = "serde")]
#[test]
fn saves_and_restores() {
    let mut m = example();
    m.set_terms("ann", "marco", Some(Terms::Weekly { day: 0 }))
        .unwrap();
    m.buy_in("ann", "t1", 100, 0).unwrap();
    let json = serde_json::to_string(&m).unwrap();
    let back: Money = serde_json::from_str(&json).unwrap();
    assert_eq!(back, m);
    assert!(json.contains("\"kind\":\"buy_in\""));
}
