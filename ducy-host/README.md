# ducy-host

Host poker tables and tournaments for remote players, on top of
[ducy-play](https://crates.io/crates/ducy-play)'s engine.

- **Tables:** `TableHost` takes each player's `Command`s and answers with `Update`s showing only what that player may see (no one sees another's cards before showdown)
- **Clocks:** turn clocks, time banks, and sit-out limits
- **Chips:** at a table with a bank, players request chips and the host approves them; stacks stay within the buy-in range
- **Tournaments:** `TournamentHost` runs a `ducy_play::Tournament` for remote players
- **Runners:** `runner` and `tourney_runner` are a host's loop: when to deal, absent players' moves at a human pace, provably fair or hidden deals, and what to send and save
- **Snapshots:** save a hosted table or tournament mid-hand and resume it after a restart

Bots take seats as any `ducy_play::Bot`; this crate has none of its own.

## Install

```toml
[dependencies]
ducy-host = "0.1"
```

Features: `serde` serializes commands, updates and snapshots.
