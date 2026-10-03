use crate::{
    bot::{Bot, play_hand},
    deal::Deal,
    error::PlayError,
    hand::Hand,
    rules::TableRules,
};

/// How to run a match between bots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchConfig {
    /// Game, betting structure and blinds.
    pub rules: TableRules,
    /// Every seat starts every hand with this many chips, so results don't
    /// depend on how earlier hands went.
    pub starting_stack: u64,
    /// Number of distinct deals.
    pub deals: usize,
    /// Seed for the deals; the same seed replays the same cards.
    pub seed: u64,
    /// Replay each deal once per seat rotation so every bot plays every seat
    /// with the same cards. This cancels most of the luck of the deal, so
    /// far fewer hands are needed to tell bots apart.
    pub duplicate: bool,
}

impl MatchConfig {
    /// A match of `deals` deals with 100 big blind stacks, not duplicate.
    pub fn new(rules: TableRules, deals: usize, seed: u64) -> Self {
        Self {
            rules,
            starting_stack: rules.big_blind * 100,
            deals,
            seed,
            duplicate: false,
        }
    }

    /// The same config in duplicate mode.
    pub fn duplicate(mut self) -> Self {
        self.duplicate = true;
        self
    }

    /// The same config with a different starting stack.
    pub fn with_starting_stack(mut self, stack: u64) -> Self {
        self.starting_stack = stack;
        self
    }
}

/// Totals for each bot over a match, indexed like the `bots` passed to
/// [`run_match`].
#[derive(Clone, Debug, PartialEq)]
pub struct MatchResult {
    /// Hands played (deals × rotations in duplicate mode).
    pub hands: usize,
    /// Chips won or lost by each bot.
    pub net: Vec<i64>,
    /// How often each bot gave no action or an illegal one.
    pub fallbacks: Vec<u32>,
    /// The big blind, for [`Self::bb_per_100`].
    pub big_blind: u64,
}

impl MatchResult {
    /// Big blinds won per 100 hands, the usual win-rate measure.
    pub fn bb_per_100(&self, bot: usize) -> f64 {
        if self.hands == 0 {
            return 0.0;
        }
        self.net[bot] as f64 / self.big_blind as f64 / self.hands as f64 * 100.0
    }
}

/// Plays `bots` against each other, one bot per seat, resetting every stack
/// to `starting_stack` before each hand and moving the button each deal.
///
/// In duplicate mode each deal is played once per rotation: in rotation `r`,
/// seat `s` is played by bot `(s + r) % bots.len()`.
pub fn run_match(
    config: &MatchConfig,
    bots: &mut [Box<dyn Bot>],
) -> Result<MatchResult, PlayError> {
    let n = bots.len();
    let rotations = if config.duplicate { n } else { 1 };
    let stacks = vec![config.starting_stack; n];
    let mut result = MatchResult {
        hands: 0,
        net: vec![0; n],
        fallbacks: vec![0; n],
        big_blind: config.rules.big_blind,
    };
    for d in 0..config.deals {
        let deal = Deal::random(
            config.rules.variant,
            n,
            Some(config.seed.wrapping_add(d as u64)),
        )?;
        let button = d % n.max(1);
        for r in 0..rotations {
            let mut hand = Hand::new(config.rules, &stacks, button, deal.clone())?;
            let mut seated: Vec<&mut dyn Bot> = Vec::with_capacity(n);
            for bot in bots.iter_mut() {
                seated.push(&mut **bot);
            }
            seated.rotate_left(r);
            let outcome = play_hand(&mut hand, &mut seated)?;
            for seat in 0..n {
                let bot = (seat + r) % n;
                result.net[bot] += outcome.result.net[seat];
                result.fallbacks[bot] += outcome.fallbacks[seat];
            }
            result.hands += 1;
        }
    }
    Ok(result)
}
