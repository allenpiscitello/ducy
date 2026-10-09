//! Heads-up pot-limit Omaha: the parts of the GTO pipeline that differ from
//! Hold'em. The abstract game itself is [`crate::holdem::hunl::Hunl`] with
//! four hole cards ([`crate::holdem::hunl::HuPlo`]) and a pot-limit menu.
//!
//! - [`showdown`]: fast Omaha showdowns and sampled equity.

pub mod showdown;
