//! k-means clustering of feature vectors, under squared Euclidean distance
//! or earth mover's distance between histograms.
//!
//! In one dimension, earth mover's distance between two histograms is the
//! L1 distance between their cumulative sums, so it's as cheap as Euclidean.
//! Centroids are the mean of their members, as is usual for this purpose.

use crate::rng::Rng;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Distance {
    /// Squared Euclidean.
    L2,
    /// Earth mover's distance between histograms with equal-width bins.
    Emd,
}

pub fn distance(a: &[f32], b: &[f32], d: Distance) -> f32 {
    match d {
        Distance::L2 => a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum(),
        Distance::Emd => {
            let (mut ca, mut cb, mut s) = (0.0f32, 0.0f32, 0.0f32);
            for (x, y) in a.iter().zip(b) {
                ca += x;
                cb += y;
                s += (ca - cb).abs();
            }
            s
        }
    }
}

/// The index of the centroid nearest to `point`.
pub fn nearest(point: &[f32], centroids: &[f32], dim: usize, d: Distance) -> usize {
    let mut best = (0, f32::INFINITY);
    for (i, c) in centroids.chunks_exact(dim).enumerate() {
        let x = distance(point, c, d);
        if x < best.1 {
            best = (i, x);
        }
    }
    best.0
}

/// Clusters `points` (`dim` values each, flattened) into at most `k`
/// clusters with k-means++ seeding and `iters` rounds of Lloyd's algorithm.
/// Returns the centroids, flattened. Deterministic for a given `seed`.
pub fn kmeans(
    points: &[f32],
    dim: usize,
    k: usize,
    iters: usize,
    seed: u64,
    d: Distance,
) -> Vec<f32> {
    let n = points.len() / dim;
    let k = k.min(n).max(1);
    let point = |i: usize| &points[i * dim..(i + 1) * dim];
    let mut rng = Rng::new(seed);
    // k-means++: each new centroid is drawn in proportion to its distance
    // from the nearest one so far.
    let mut centroids: Vec<f32> = point((rng.next_u64() % n as u64) as usize).to_vec();
    let mut near: Vec<f32> = (0..n).map(|i| distance(point(i), &centroids, d)).collect();
    while centroids.len() / dim < k {
        let total: f64 = near.iter().map(|&x| x as f64).sum();
        let next = if total <= 0.0 {
            (rng.next_u64() % n as u64) as usize
        } else {
            let mut x = rng.next_f64() * total;
            let mut pick = n - 1;
            for (i, &w) in near.iter().enumerate() {
                if x < w as f64 {
                    pick = i;
                    break;
                }
                x -= w as f64;
            }
            pick
        };
        let c = point(next).to_vec();
        for (i, w) in near.iter_mut().enumerate() {
            *w = w.min(distance(point(i), &c, d));
        }
        centroids.extend(c);
    }
    let mut assign = vec![usize::MAX; n];
    for _ in 0..iters {
        let changed = assign_all(points, dim, &centroids, d, &mut assign);
        let mut sums = vec![0.0f64; k * dim];
        let mut counts = vec![0usize; k];
        for (i, &c) in assign.iter().enumerate() {
            counts[c] += 1;
            for (s, &x) in sums[c * dim..(c + 1) * dim].iter_mut().zip(point(i)) {
                *s += x as f64;
            }
        }
        for c in 0..k {
            if counts[c] > 0 {
                for j in 0..dim {
                    centroids[c * dim + j] = (sums[c * dim + j] / counts[c] as f64) as f32;
                }
            }
        }
        if !changed {
            break;
        }
    }
    centroids
}

/// Assigns every point to its nearest centroid; returns whether any moved.
fn assign_all(
    points: &[f32],
    dim: usize,
    centroids: &[f32],
    d: Distance,
    assign: &mut [usize],
) -> bool {
    #[cfg(feature = "parallel")]
    {
        use rayon::prelude::*;
        assign
            .par_iter_mut()
            .zip(points.par_chunks_exact(dim))
            .map(|(a, p)| {
                let c = nearest(p, centroids, dim, d);
                let moved = *a != c;
                *a = c;
                moved
            })
            .reduce(|| false, |x, y| x || y)
    }
    #[cfg(not(feature = "parallel"))]
    {
        let mut moved = false;
        for (a, p) in assign.iter_mut().zip(points.chunks_exact(dim)) {
            let c = nearest(p, centroids, dim, d);
            moved |= *a != c;
            *a = c;
        }
        moved
    }
}
