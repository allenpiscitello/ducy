//! Six-handed pot-limit Omaha preflop (#290). The hold'em solve's 169
//! classes become the PLO abstraction's preflop buckets (500 for the live
//! abstractions): [`PloDealer`] deals six random hands and buckets them, and
//! [`bucket_equity`] samples each bucket's pot share against each other
//! bucket and against one to five random hands, as [`PreflopEquity`] does
//! for hold'em. Play to the flop is [`PreflopGame::pot_limit`]'s tree.
//!
//! A bucket's equity is sampled through a pool of hands that land in it:
//! random hands are dealt and bucketed until every bucket's pool is full (or
//! a draw limit is reached, for the rarest buckets), then each sample draws a
//! hand from each pool, skips the pair if they share a card, and deals a
//! board from what's left.
//!
//! [`PreflopGame::pot_limit`]: super::preflop::PreflopGame::pot_limit

use std::thread;

use super::{
    equity::{MAX_OPPONENTS, PreflopEquity},
    preflop::SEATS,
};
use crate::{
    Rng,
    holdem::{
        cards::{Card, bit, mask},
        hunl::Buckets,
    },
    omaha::{
        abstraction::PloAbstraction,
        showdown::{RiverBoard, draw},
    },
};

/// Deals PLO hands and buckets them, for the six-handed solver.
pub struct PloDealer<'a> {
    pub cards: &'a PloAbstraction,
}

impl PloDealer<'_> {
    /// Six hands, one per seat, as their preflop buckets.
    pub fn deal(&self, rng: &mut Rng) -> [usize; SEATS] {
        let k = self.cards.config.hole_cards;
        let mut used = 0u64;
        let mut out = [0; SEATS];
        let mut hole = [0 as Card; 6];
        for b in &mut out {
            for c in hole.iter_mut().take(k) {
                *c = draw(&mut used, rng);
            }
            *b = self.cards.preflop_bucket(&hole[..k]) as usize;
        }
        out
    }

    /// Hand classes: the abstraction's preflop buckets.
    pub fn classes(&self) -> usize {
        self.cards.bucket_count(0)
    }
}

/// Up to `pool` hands per bucket, from random deals (at most `pool` × buckets
/// × 50 of them, so the rarest buckets may end with fewer, or none).
pub fn bucket_pools(cards: &PloAbstraction, pool: usize, rng: &mut Rng) -> Vec<Vec<Vec<Card>>> {
    let n = cards.bucket_count(0);
    let k = cards.config.hole_cards;
    let mut pools: Vec<Vec<Vec<Card>>> = vec![Vec::new(); n];
    let mut full = 0;
    for _ in 0..pool * n * 50 {
        let mut used = 0u64;
        let hole: Vec<Card> = (0..k).map(|_| draw(&mut used, rng)).collect();
        let b = cards.preflop_bucket(&hole) as usize;
        if pools[b].len() < pool {
            pools[b].push(hole);
            if pools[b].len() == pool {
                full += 1;
                if full == n {
                    break;
                }
            }
        }
    }
    pools
}

/// The pot share of `hands[0]` on `board`, ties split.
fn share(hands: &[&[Card]], board: &[Card; 5]) -> f64 {
    let river = RiverBoard::new(board);
    let scores: Vec<u32> = hands.iter().map(|h| river.score(h)).collect();
    let best = *scores.iter().max().expect("hands");
    if scores[0] < best {
        return 0.0;
    }
    1.0 / scores.iter().filter(|&&s| s == best).count() as f64
}

fn board(mut used: u64, rng: &mut Rng) -> [Card; 5] {
    std::array::from_fn(|_| draw(&mut used, rng))
}

fn pick<'p>(pool: &'p [Vec<Card>], rng: &mut Rng) -> &'p [Card] {
    &pool[(rng.next_u64() % pool.len() as u64) as usize]
}

/// Bucket `a` against bucket `b`: the mean pot share over `samples` deals
/// (0.5 if either pool is empty).
fn sample_pair(pools: &[Vec<Vec<Card>>], a: usize, b: usize, samples: u32, rng: &mut Rng) -> f32 {
    if pools[a].is_empty() || pools[b].is_empty() {
        return 0.5;
    }
    let (mut total, mut done, mut tries) = (0.0, 0u32, 0u32);
    while done < samples && tries < samples * 20 {
        tries += 1;
        let ha = pick(&pools[a], rng);
        let hb = pick(&pools[b], rng);
        let used = mask(ha) | mask(hb);
        if ha.iter().any(|&c| mask(hb) & bit(c) != 0) {
            continue;
        }
        total += share(&[ha, hb], &board(used, rng));
        done += 1;
    }
    if done == 0 {
        0.5
    } else {
        (total / done as f64) as f32
    }
}

/// Bucket `a` against `opponents` random hands of the same size.
fn sample_random(
    pools: &[Vec<Vec<Card>>],
    a: usize,
    opponents: usize,
    samples: u32,
    rng: &mut Rng,
) -> f32 {
    if pools[a].is_empty() {
        return 1.0 / (opponents + 1) as f32;
    }
    let k = pools[a][0].len();
    let mut total = 0.0;
    for _ in 0..samples {
        let ha = pick(&pools[a], rng);
        let mut used = mask(ha);
        let others: Vec<Vec<Card>> = (0..opponents)
            .map(|_| (0..k).map(|_| draw(&mut used, rng)).collect())
            .collect();
        let mut hands: Vec<&[Card]> = vec![ha];
        hands.extend(others.iter().map(|h| h.as_slice()));
        total += share(&hands, &board(used, rng));
    }
    (total / samples as f64) as f32
}

/// Preflop equity over the abstraction's preflop buckets: `samples` deals
/// per bucket pair and per bucket and opponent count, from pools of `pool`
/// hands per bucket, over `threads` threads.
pub fn bucket_equity(
    cards: &PloAbstraction,
    pool: usize,
    samples: u32,
    seed: u64,
    threads: usize,
) -> PreflopEquity {
    let pools = bucket_pools(cards, pool, &mut Rng::new(seed));
    equity_from_pools(&pools, samples, seed, threads)
}

/// [`bucket_equity`] from pools already drawn.
pub fn equity_from_pools(
    pools: &[Vec<Vec<Card>>],
    samples: u32,
    seed: u64,
    threads: usize,
) -> PreflopEquity {
    let n = pools.len();
    let threads = threads.max(1);
    let rows: Vec<(usize, Vec<f32>, [f32; MAX_OPPONENTS])> = thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                scope.spawn(move || {
                    (t..n)
                        .step_by(threads)
                        .map(|a| {
                            let mut rng = Rng::for_iteration(seed ^ 0x9e37, a as u64);
                            // Only b ≥ a is sampled; the other half mirrors it.
                            let row = (0..n)
                                .map(|b| {
                                    if b >= a {
                                        sample_pair(pools, a, b, samples, &mut rng)
                                    } else {
                                        0.0
                                    }
                                })
                                .collect();
                            let mut multi = [0f32; MAX_OPPONENTS];
                            for (k, m) in multi.iter_mut().enumerate() {
                                *m = sample_random(pools, a, k + 1, samples, &mut rng);
                            }
                            (a, row, multi)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("thread"))
            .collect()
    });
    let mut heads_up = vec![0f32; n * n];
    let mut vs_random = vec![vec![0f32; n]; MAX_OPPONENTS];
    for (a, row, multi) in rows {
        for b in a..n {
            heads_up[a * n + b] = row[b];
            heads_up[b * n + a] = 1.0 - row[b];
        }
        for k in 0..MAX_OPPONENTS {
            vs_random[k][a] = multi[k];
        }
    }
    PreflopEquity::from_tables(n, heads_up, vs_random)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        holdem::cards::parse,
        omaha::abstraction::PloAbstractionConfig,
        sixmax::{
            preflop::{Act, Node, PreflopGame},
            realize::Realization,
            solver::PreflopSolver,
        },
    };

    /// A small PLO4 abstraction: 24 preflop buckets.
    fn small() -> PloAbstraction {
        PloAbstraction::build(
            PloAbstractionConfig {
                preflop: 24,
                flop: 8,
                turn: 8,
                river: 8,
                fit_hands: 600,
                equity_samples: 60,
                ..PloAbstractionConfig::default()
            },
            |_| {},
        )
    }

    #[test]
    fn buckets_carry_equity_and_aces_learn_to_play() {
        let cards = small();
        let n = cards.bucket_count(0);
        let eq = bucket_equity(&cards, 24, 120, 5, 4);
        assert_eq!(eq.classes(), n);
        for a in 0..n {
            for b in 0..n {
                assert!((eq.heads_up(a, b) + eq.heads_up(b, a) - 1.0).abs() < 1e-5);
            }
        }
        assert_eq!(PreflopEquity::from_bytes(&eq.to_bytes()), Ok(eq.clone()));
        let aces = cards.preflop_bucket(&parse("As Ad Ks Kd").unwrap()) as usize;
        let trash = cards.preflop_bucket(&parse("2c 3d 7h 9s").unwrap()) as usize;
        assert!(eq.vs_random(aces, 1) > 0.6, "{}", eq.vs_random(aces, 1));
        assert!(eq.vs_random(trash, 1) < 0.48, "{}", eq.vs_random(trash, 1));
        assert!(
            eq.heads_up(aces, trash) > 0.62,
            "{}",
            eq.heads_up(aces, trash)
        );

        // A short six-handed solve: UTG plays the aces and folds the trash.
        let game = PreflopGame::pot_limit(100, Realization::uniform(n));
        let dealer = PloDealer { cards: &cards };
        let deal = |rng: &mut Rng| dealer.deal(rng);
        let solver = PreflopSolver::with_dealer(&game, &eq, &deal, 3);
        solver.run(40_000, 4);
        let Node::Decision { id, actions, .. } = &game.nodes[game.root()] else {
            panic!("root");
        };
        let fold = actions.iter().position(|(a, _)| *a == Act::Fold).unwrap();
        let aa = solver.strategy(*id, aces);
        let junk = solver.strategy(*id, trash);
        assert!(aa[fold] < 0.1, "aces fold {aa:?}");
        assert!(junk[fold] > aa[fold] + 0.3, "trash {junk:?}, aces {aa:?}");

        // Six of the bot play whole PLO hands: legal, preflop from the solve.
        use crate::sixmax::bot::{PreflopStrategy, SixMaxBot};
        use ducy_bots::personality::Personality;
        use ducy_play::{Deal, Hand, TableRules, Variant, play_hand};
        use std::sync::Arc;
        let strategy = Arc::new(PreflopStrategy::from_solver(&game, &solver));
        let cards = Arc::new(cards);
        let rules =
            TableRules::pot_limit_omaha(1, 2).with_variant(Variant::Omaha { hole_cards: 4 });
        let mut bots: Vec<SixMaxBot<_>> = (0..SEATS as u64)
            .map(|s| {
                SixMaxBot::new(strategy.clone(), Personality::DougPoker.bot(Some(s)), s)
                    .with_plo_buckets(cards.clone())
            })
            .collect();
        for d in 0..60u64 {
            let deal = Deal::random(rules.variant, SEATS, Some(d)).expect("deal");
            let mut hand = Hand::new(rules, &[200; SEATS], d as usize % SEATS, deal).expect("hand");
            let mut seated: Vec<&mut dyn ducy_play::Bot> = bots
                .iter_mut()
                .map(|b| b as &mut dyn ducy_play::Bot)
                .collect();
            let out = play_hand(&mut hand, &mut seated).expect("a legal hand");
            assert_eq!(out.result.net.iter().sum::<i64>(), 0);
        }
        let solved: u64 = bots.iter().map(|b| b.solved).sum();
        let off: u64 = bots.iter().map(|b| b.off_tree).sum();
        assert!(
            solved > 10 * off.max(1),
            "{solved} from the solve, {off} off the tree"
        );
    }
}
