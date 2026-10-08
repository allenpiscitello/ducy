//! Hand replays (ducy_play::replay, #157): the same frames as ducy.cards'
//! JavaScript replayer made of real hands (replay-fixture.json: side pots,
//! a split pot, a missed-blind post), and every frame's chips add up.
#![cfg(feature = "serde")]

use ducy_play::Event;
use ducy_play::replay::replay_frames;
use serde_json::{Value, json};

#[test]
fn frames_match_the_page_and_chips_add_up() {
    let f: Value = serde_json::from_str(include_str!("replay-fixture.json")).unwrap();
    let hands = f["hands"].as_array().unwrap();
    assert!(hands.len() > 50);
    for (h, hand) in hands.iter().enumerate() {
        let events: Vec<Event> = serde_json::from_value(hand["events"].clone()).unwrap();
        let stacks: Vec<u64> = serde_json::from_value(hand["stacks"].clone()).unwrap();
        let names: Vec<String> = serde_json::from_value(hand["names"].clone()).unwrap();
        let frames = replay_frames(&events, &stacks, &names);
        let want = hand["frames"].as_array().unwrap();
        assert_eq!(frames.len(), want.len(), "hand {h}");
        let total: u64 = stacks.iter().sum();
        for (i, (got, want)) in frames.iter().zip(want).enumerate() {
            let got = json!({
                "street": got.street,
                "board": got.board,
                "pot": got.pot,
                "text": got.text,
                "seats": got.seats,
            });
            assert_eq!(&got, want, "hand {h} ({}), frame {i}", hand["label"]);
            let chips: u64 =
                frames[i].seats.iter().map(|s| s.stack + s.bet).sum::<u64>() + frames[i].pot;
            assert_eq!(chips, total, "hand {h}, frame {i}: chips add up");
        }
        // A finished hand ends with everything awarded.
        assert_eq!(frames.last().unwrap().pot, 0, "hand {h}");
    }
}
