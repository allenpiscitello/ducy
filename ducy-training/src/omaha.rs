//! Omaha starting hands as the range trainer describes them (ducy-web #464):
//! the kinds `ducy::games::omaha_analysis::hand_tags` gives, in words.

/// Each tag `hand_tags` can give, and a plural description that completes
/// "you play too many ___".
pub const OMAHA_TAG_LABELS: [(&str, &str); 13] = [
    ("aa", "aces (AAxx)"),
    ("big_pair", "kings or queens (KKxx, QQxx)"),
    ("small_pair", "hands built around a small pair"),
    ("double_paired", "double-paired hands"),
    ("dead_cards", "hands with trips or quads (dead cards)"),
    ("double_suited", "double-suited hands"),
    ("rainbow", "rainbow hands with no flush draw"),
    ("suited_ace", "hands with a suited ace"),
    (
        "weak_suited",
        "weak suited hands (suited, but no ace or king in the suit)",
    ),
    ("rundown", "rundowns (four ranks in a row)"),
    ("broadway", "broadway-heavy hands"),
    ("low", "hands full of low cards"),
    ("danglers", "hands with a dangling card"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use ducy::deck::Card;
    use ducy::games::omaha_analysis::hand_tags;

    #[test]
    fn every_tag_has_words() {
        let hands = [
            "As Ad Kh Qh",
            "Js Ts 9d 8d",
            "7c 6c 5h 4h",
            "Kd Kc 2s 7h",
            "Ah 2h 3c 3d",
            "9s 9h 6c 2d",
            "2c 3d 4h 5s 6c",
            "As Ah Ad 2c",
        ];
        for h in hands {
            let cards: Vec<Card> = h.split(' ').map(|c| Card::parse(c).unwrap()).collect();
            for t in hand_tags(&cards) {
                assert!(OMAHA_TAG_LABELS.iter().any(|(k, _)| *k == t), "{t}");
            }
        }
    }
}
