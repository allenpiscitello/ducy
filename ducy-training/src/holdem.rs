//! Hold'em starting hands as the range trainer sees them (ducy-web #464):
//! the kind of hand each is, so practice can spot patterns in mistakes ("you
//! play too many weak offsuit hands") and deal more of the kinds a player
//! gets wrong. Each hand gets exactly one kind.

/// Each kind and its plural description, written to complete "you play too
/// many ___".
pub const HOLDEM_TAG_LABELS: [(&str, &str); 12] = [
    ("pairs_big", "big pairs (JJ+)"),
    ("pairs_mid", "middle pairs (77–TT)"),
    ("pairs_small", "small pairs (22–66)"),
    ("ace_suited", "suited aces with a weak kicker (A2s–A9s)"),
    ("ace_offsuit", "offsuit aces with a weak kicker (A2o–A9o)"),
    ("broadway_suited", "suited broadway hands (KQs, AJs, QTs…)"),
    (
        "broadway_offsuit",
        "offsuit broadway hands (KQo, AJo, QTo…)",
    ),
    ("connector_suited", "suited connectors (T9s, 76s, 54s…)"),
    ("gapper_suited", "suited gappers (J9s, 86s, 64s…)"),
    (
        "connector_offsuit",
        "offsuit connectors and gappers (T9o, 86o…)",
    ),
    ("weak_suited", "weak suited hands (K6s, Q4s, J3s…)"),
    ("weak_offsuit", "weak offsuit hands (K7o, Q5o, J8o…)"),
];

/// A rank's value: 2 to 14 (an ace), or None.
fn value(c: &str) -> Option<u8> {
    Some(match c.chars().next()? {
        '2'..='9' => c.as_bytes()[0] - b'0',
        'T' => 10,
        'J' => 11,
        'Q' => 12,
        'K' => 13,
        'A' => 14,
        _ => return None,
    })
}

/// The kind of a two-card hand like `["Kh", "7d"]`: "pairs_big",
/// "pairs_mid", "pairs_small", "broadway_suited", "broadway_offsuit",
/// "ace_suited", "ace_offsuit", "connector_suited", "connector_offsuit",
/// "gapper_suited", "weak_suited" or "weak_offsuit". None if they aren't two
/// cards.
pub fn holdem_tag(cards: &[&str]) -> Option<&'static str> {
    let [x, y] = cards else { return None };
    let (mut a, mut b) = (value(x)?, value(y)?);
    if b > a {
        std::mem::swap(&mut a, &mut b);
    }
    let suited = x.chars().nth(1) == y.chars().nth(1);
    let pick = |s: &'static str, o: &'static str| if suited { s } else { o };
    Some(if a == b {
        match a {
            11.. => "pairs_big",
            7.. => "pairs_mid",
            _ => "pairs_small",
        }
    } else if b >= 10 {
        pick("broadway_suited", "broadway_offsuit")
    } else if a == 14 {
        // Wheel cards connect with the ace, but A-x is covered here.
        pick("ace_suited", "ace_offsuit")
    } else {
        match a - b - 1 {
            0 => pick("connector_suited", "connector_offsuit"),
            1 => pick("gapper_suited", "connector_offsuit"),
            2 => pick("gapper_suited", "weak_offsuit"),
            _ => pick("weak_suited", "weak_offsuit"),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_hand_one_kind() {
        let tag = |a: &str, b: &str| holdem_tag(&[a, b]).unwrap();
        assert_eq!(
            (tag("Jh", "Jd"), tag("7c", "7s"), tag("2c", "2d")),
            ("pairs_big", "pairs_mid", "pairs_small")
        );
        assert_eq!(
            (tag("Kh", "Qh"), tag("Ah", "Td")),
            ("broadway_suited", "broadway_offsuit")
        );
        assert_eq!(
            (tag("Ah", "5h"), tag("9d", "Ac")),
            ("ace_suited", "ace_offsuit")
        );
        assert_eq!(
            (tag("Th", "9h"), tag("Th", "9d")),
            ("connector_suited", "connector_offsuit")
        );
        assert_eq!(
            (tag("Jh", "9h"), tag("Jh", "9d"), tag("8h", "5d")),
            ("gapper_suited", "connector_offsuit", "weak_offsuit")
        );
        assert_eq!(
            (tag("Kh", "6h"), tag("Kh", "7d")),
            ("weak_suited", "weak_offsuit")
        );
        assert_eq!(holdem_tag(&["Kh"]), None);
    }
}
