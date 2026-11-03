//! Chat at a hosted table, as the host checks it: what someone typed made
//! into one short line, the name on it, and how many messages one person may
//! send at a time. The host checks every message and sends it on with the
//! sender's name; carrying it (encrypted or not) is the app's.

use std::collections::HashMap;

/// Characters in one message.
pub const CHAT_MAX: usize = 200;
/// A name on a chat line, in UTF-16 units (as a page counts them).
pub const NAME_MAX: usize = 20;
/// Messages one person can send...
pub const CHAT_BURST: usize = 5;
/// ...in this long.
pub const CHAT_WINDOW_MS: u64 = 10_000;

/// Whitespace as JavaScript's `\s` has it (which takes in U+FEFF but not
/// U+0085), so a line reads the same whichever side cleaned it.
fn js_space(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

fn blank(c: char) -> bool {
    js_space(c) || c <= '\u{1f}' || c == '\u{7f}'
}

/// What someone typed, ready to send: control characters and runs of
/// spaces become one space, and it's cut to [`CHAT_MAX`] characters. Empty
/// if nothing's left.
pub fn clean(text: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in text.chars() {
        if blank(c) {
            gap = true;
        } else {
            if gap && !out.is_empty() {
                out.push(' ');
            }
            gap = false;
            out.push(c);
        }
    }
    let cut: String = out.chars().take(CHAT_MAX).collect();
    cut.trim_end_matches(js_space).to_string()
}

/// A name for a chat line: what the player calls themselves, cleaned and
/// cut to [`NAME_MAX`] (never through an emoji), or "A player".
pub fn name(text: &str) -> String {
    let mut units = 0;
    let cut: String = clean(text)
        .chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= NAME_MAX
        })
        .collect();
    if cut.is_empty() {
        "A player".to_string()
    } else {
        cut
    }
}

/// The host's flood control: at most `burst` messages from one person in
/// any `window_ms`.
#[derive(Clone, Debug)]
pub struct Limiter {
    burst: usize,
    window_ms: u64,
    sent: HashMap<String, Vec<u64>>,
}

impl Default for Limiter {
    fn default() -> Self {
        Self::new(CHAT_BURST, CHAT_WINDOW_MS)
    }
}

impl Limiter {
    /// At most `burst` messages in any `window_ms`.
    pub fn new(burst: usize, window_ms: u64) -> Self {
        Self {
            burst,
            window_ms,
            sent: HashMap::new(),
        }
    }

    /// Whether `who` may send another message at `now`; if so, it counts.
    pub fn allow(&mut self, who: &str, now: u64) -> bool {
        let window = self.window_ms;
        let recent = self.sent.entry(who.to_string()).or_default();
        recent.retain(|&t| now.saturating_sub(t) < window);
        if recent.len() >= self.burst {
            return false;
        }
        recent.push(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_is_one_short_line() {
        assert_eq!(clean("  nice   hand\n\tsir  "), "nice hand sir");
        assert_eq!(clean("a\u{0}b\u{7}c"), "a b c");
        assert_eq!(clean("   "), "");
        assert_eq!(clean("a\u{a0}\u{feff}b\u{85}"), "a b\u{85}");
        assert_eq!(clean(&"x".repeat(500)).chars().count(), CHAT_MAX);
        assert_eq!(clean(&"🂡".repeat(300)).chars().count(), CHAT_MAX);
        // Cut where a space was: no space left at the end.
        let s = format!("{} b", "a".repeat(CHAT_MAX - 1));
        assert_eq!(clean(&s), "a".repeat(CHAT_MAX - 1));
    }

    #[test]
    fn names_are_short_with_a_fallback() {
        assert_eq!(name("Allen"), "Allen");
        assert_eq!(name(" \n"), "A player");
        assert_eq!(name(&"n".repeat(40)), "n".repeat(20));
        // 19 units and an emoji (2): the emoji doesn't fit, and isn't split.
        assert_eq!(name(&format!("{}🂡", "n".repeat(19))), "n".repeat(19));
    }

    #[test]
    fn each_player_sends_a_few_at_a_time() {
        let mut lim = Limiter::new(3, 1000);
        assert!(lim.allow("a", 0));
        assert!(lim.allow("a", 10));
        assert!(lim.allow("a", 20));
        assert!(!lim.allow("a", 30), "a fourth within the window waits");
        assert!(lim.allow("b", 30), "someone else still can");
        assert!(lim.allow("a", 1001), "the oldest has aged out");
    }
}
