//! ducy's bindings for ducy.cards (wasm-pack). The default `site` feature
//! has everything the site uses; the relay's build (`--no-default-features`)
//! has only what clubs need (ducy-web #463).

mod book;
mod club;
mod fair;
mod hands;
mod host;
mod identity;
mod member;
mod money;
mod oneoff;
mod overview;
// The bot tables, the GTO bots and seated bots: the site's, not the relay's
// (ducy-web #463).
#[cfg(feature = "site")]
mod play;
mod promos;
mod registry;
mod room;
mod rules;
#[cfg(feature = "site")]
mod seat_bot;
mod seats;
mod service;
mod shuffle;
// Equity, ranges and hand analysis: the site's study pages.
#[cfg(feature = "site")]
mod study;
mod tournament;
mod tourney_host;
mod tourneys;
// The study tools' rules (ducy-training): the site's.
#[cfg(feature = "site")]
mod training;
mod util;
mod views;
#[cfg(feature = "site")]
pub use play::{BotTable, MultiTable};
