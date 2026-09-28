use crate::deck::Deck;
use crate::ranking::hand_rank::HandRanking;
use crate::ranking::standard_hand_ranker::RankOrder;

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub struct BadugiRanks {
    count: u8,
    scores: [u32; 4],
}

impl HandRanking for BadugiRanks {}

impl Ord for BadugiRanks {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.count.cmp(&other.count).then_with(|| {
            for i in 0..self.count as usize {
                match other.scores[i].cmp(&self.scores[i]) {
                    std::cmp::Ordering::Equal => continue,
                    ord => return ord,
                }
            }
            std::cmp::Ordering::Equal
        })
    }
}

impl PartialOrd for BadugiRanks {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl std::fmt::Display for BadugiRanks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-card Badugi", self.count)
    }
}

pub struct BadugiRanker;

impl BadugiRanker {
    pub fn get_rank(deck: &Deck) -> BadugiRanks {
        let cards: Vec<_> = deck.iter(false).collect();
        let n = cards.len();
        let ace_low = RankOrder::AceIsLow;

        for size in (1..=n.min(4)).rev() {
            let mut best: Option<BadugiRanks> = None;

            for combo in indices_combinations(n, size) {
                let selected: Vec<_> = combo.iter().map(|&i| cards[i]).collect();

                let mut suits_ok = true;
                let mut ranks_ok = true;
                for i in 0..selected.len() {
                    for j in (i + 1)..selected.len() {
                        if selected[i].suit() == selected[j].suit() {
                            suits_ok = false;
                        }
                        if selected[i].rank() == selected[j].rank() {
                            ranks_ok = false;
                        }
                    }
                }
                if !suits_ok || !ranks_ok {
                    continue;
                }

                let mut scores: Vec<u32> = selected
                    .iter()
                    .map(|c| ace_low.get_score(&c.rank()))
                    .collect();
                scores.sort();

                let mut score_arr = [0u32; 4];
                for (i, &s) in scores.iter().enumerate() {
                    score_arr[i] = s;
                }

                let rank = BadugiRanks {
                    count: size as u8,
                    scores: score_arr,
                };

                match &best {
                    Some(b) if rank > *b => best = Some(rank),
                    None => best = Some(rank),
                    _ => {}
                }
            }

            if let Some(b) = best {
                return b;
            }
        }

        BadugiRanks {
            count: 0,
            scores: [0; 4],
        }
    }
}

fn indices_combinations(n: usize, k: usize) -> Vec<Vec<usize>> {
    let mut result = Vec::new();
    let mut combo = Vec::with_capacity(k);
    generate_combos(0, n, k, &mut combo, &mut result);
    result
}

fn generate_combos(
    start: usize,
    n: usize,
    k: usize,
    combo: &mut Vec<usize>,
    result: &mut Vec<Vec<usize>>,
) {
    if combo.len() == k {
        result.push(combo.clone());
        return;
    }
    for i in start..n {
        combo.push(i);
        generate_combos(i + 1, n, k, combo, result);
        combo.pop();
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test_util::deck_from_cards;

    #[test]
    fn test_nut_badugi() {
        // A-2-3-4 all different suits is the best possible badugi
        let nut = deck_from_cards("Ac 2d 3h 4s");
        let rank = BadugiRanker::get_rank(&nut);
        assert_eq!(rank.count, 4);
    }

    #[test]
    fn test_four_card_beats_three_card() {
        let four_card = deck_from_cards("Kc Qd Jh Ts");
        let three_card = deck_from_cards("Ac 2d 3h 3s");
        assert!(BadugiRanker::get_rank(&four_card) > BadugiRanker::get_rank(&three_card));
    }

    #[test]
    fn test_three_card_from_duplicate_suit() {
        // Two clubs: best 3-card badugi drops one club
        let hand = deck_from_cards("Ac 2c 3d 4h");
        let rank = BadugiRanker::get_rank(&hand);
        assert_eq!(rank.count, 3);
    }

    #[test]
    fn test_three_card_from_duplicate_rank() {
        // Two threes: best 3-card badugi drops one three
        let hand = deck_from_cards("Ac 2d 3h 3s");
        let rank = BadugiRanker::get_rank(&hand);
        assert_eq!(rank.count, 3);
    }

    #[test]
    fn test_lower_badugi_wins() {
        // A-2-3-4 beats 2-3-4-5 (ace is low)
        let low = deck_from_cards("Ac 2d 3h 4s");
        let high = deck_from_cards("2c 3d 4h 5s");
        assert!(BadugiRanker::get_rank(&low) > BadugiRanker::get_rank(&high));
    }

    #[test]
    fn test_same_count_tiebreaker() {
        // A-2-3-5 loses to A-2-3-4 (5 vs 4 on highest card)
        let worse = deck_from_cards("Ac 2d 3h 5s");
        let better = deck_from_cards("Ac 2d 3h 4s");
        assert!(BadugiRanker::get_rank(&better) > BadugiRanker::get_rank(&worse));
    }

    #[test]
    fn test_all_same_suit_is_one_card() {
        let hand = deck_from_cards("Ac 2c 3c 4c");
        let rank = BadugiRanker::get_rank(&hand);
        assert_eq!(rank.count, 1);
    }

    #[test]
    fn test_all_same_rank_is_one_card() {
        let hand = deck_from_cards("Kc Kd Kh Ks");
        let rank = BadugiRanker::get_rank(&hand);
        assert_eq!(rank.count, 1);
    }

    #[test]
    fn test_two_card_badugi() {
        // Two pairs of suits: Ac Kc 2d Kd — best is A-2 (clubs/diamonds) or A-K (clubs/diamonds)
        // Actually: Ac 2d is valid (different suits, different ranks), also Ac Kd, Kc 2d, Kc Kd (same rank - invalid)
        // Best 2-card: Ac 2d (lowest)
        let hand = deck_from_cards("Ac Kc 2d Kd");
        let rank = BadugiRanker::get_rank(&hand);
        assert_eq!(rank.count, 2);
    }

    #[test]
    fn test_equal_hands() {
        let hand1 = deck_from_cards("Ac 2d 3h 4s");
        let hand2 = deck_from_cards("Ac 2d 3h 4s");
        assert_eq!(
            BadugiRanker::get_rank(&hand1),
            BadugiRanker::get_rank(&hand2)
        );
    }
}
