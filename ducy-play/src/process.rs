//! Bots that run as separate programs and talk JSON over stdin/stdout.

use std::{
    io::{self, BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use crate::{
    bot::{Bot, HandSummary, Observation},
    hand::Action,
};

/// A message sent to a bot program, one JSON object per line.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Message<'a> {
    /// The bot must answer with one reply line.
    Act {
        id: u64,
        observation: &'a Observation,
    },
    /// For information; no reply expected.
    HandOver { summary: &'a HandSummary },
}

/// A bot program's answer to an `act` message.
#[derive(Deserialize)]
struct Reply {
    #[serde(default)]
    id: Option<u64>,
    action: String,
    #[serde(default)]
    amount: Option<u64>,
}

impl Reply {
    fn into_action(self) -> Option<Action> {
        match (self.action.as_str(), self.amount) {
            ("fold", _) => Some(Action::Fold),
            ("check", _) => Some(Action::Check),
            ("call", _) => Some(Action::Call),
            ("all_in", _) => Some(Action::AllIn),
            ("bet", Some(to)) => Some(Action::Bet(to)),
            ("raise", Some(to)) => Some(Action::Raise(to)),
            _ => None,
        }
    }
}

/// A bot running as its own program, in any language.
///
/// The protocol is one JSON object per line:
///
/// - To the bot, when it must act:
///   `{"type": "act", "id": 7, "observation": {...}}` where `observation`
///   is an [`Observation`].
/// - From the bot, one line in reply:
///   `{"id": 7, "action": "raise", "amount": 12}`. `action` is `fold`,
///   `check`, `call`, `bet`, `raise` or `all_in`; `bet` and `raise` need
///   `amount`, the total for the street. Echoing `id` is optional but lets
///   late replies be told apart.
/// - To the bot, after every hand: `{"type": "hand_over", "summary": {...}}`
///   where `summary` is a [`HandSummary`]. No reply.
///
/// Anything the bot writes to stderr is passed through. If the bot doesn't
/// reply within the timeout, replies with something unreadable, or exits,
/// it checks when it can and folds otherwise, and the game carries on.
pub struct ProcessBot {
    child: Child,
    stdin: Option<ChildStdin>,
    replies: Receiver<String>,
    timeout: Duration,
    next_id: u64,
}

impl ProcessBot {
    /// Starts `program` with `args`. Replies time out after 5 seconds.
    pub fn spawn(program: &str, args: &[&str]) -> io::Result<Self> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdin = child.stdin.take();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("bot has no stdout"))?;
        let (tx, replies) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            replies,
            timeout: Duration::from_secs(5),
            next_id: 0,
        })
    }

    /// The same bot with a different reply timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn send(&mut self, message: &Message) -> bool {
        let Some(stdin) = self.stdin.as_mut() else {
            return false;
        };
        let Ok(mut line) = serde_json::to_string(message) else {
            return false;
        };
        line.push('\n');
        if stdin
            .write_all(line.as_bytes())
            .and_then(|()| stdin.flush())
            .is_err()
        {
            // The bot has exited; stop writing to it.
            self.stdin = None;
            return false;
        }
        true
    }
}

impl Bot for ProcessBot {
    fn act(&mut self, obs: &Observation) -> Option<Action> {
        self.next_id += 1;
        let id = self.next_id;
        if !self.send(&Message::Act {
            id,
            observation: obs,
        }) {
            return None;
        }
        let deadline = Instant::now() + self.timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = match self.replies.recv_timeout(left) {
                Ok(line) => line,
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return None,
            };
            let Ok(reply) = serde_json::from_str::<Reply>(&line) else {
                return None;
            };
            match reply.id {
                // A late answer to an earlier question; keep waiting.
                Some(old) if old < id => continue,
                _ => return reply.into_action(),
            }
        }
    }

    fn hand_over(&mut self, summary: &HandSummary) {
        self.send(&Message::HandOver { summary });
    }
}

impl Drop for ProcessBot {
    fn drop(&mut self) {
        // Closing stdin lets a well-behaved bot exit; kill it if it doesn't.
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
