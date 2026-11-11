//! Summaries of the samples of a workload on a stack.

use crate::run::Sample;

/// The middle of `values`, which must be sorted. The mean of the two in the middle for an even count.
fn median(sorted: &[u64]) -> u64 {
    let n = sorted.len();
    match n {
        0 => 0,
        _ if n % 2 == 1 => sorted[n / 2],
        _ => u64::midpoint(sorted[n / 2 - 1], sorted[n / 2]),
    }
}

/// The nearest-rank percentile `p` of `values`, which must be sorted.
fn percentile(sorted: &[u64], p: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (p * sorted.len()).div_ceil(100).clamp(1, sorted.len());
    sorted[rank - 1]
}

fn sorted(values: impl Iterator<Item = u64>) -> Vec<u64> {
    let mut values: Vec<u64> = values.collect();
    values.sort_unstable();
    values
}

#[expect(clippy::cast_precision_loss, reason = "the counts are far below 2^52")]
fn to_f64(value: usize) -> f64 {
    value as f64
}

/// Nanoseconds as microseconds.
fn micros(ns: u64) -> f64 {
    f64::from(u32::try_from(ns).unwrap_or(u32::MAX)) / 1000.0
}

fn mean(values: impl Iterator<Item = usize>) -> f64 {
    let (sum, count) = values.fold((0, 0), |(sum, count), value| (sum + value, count + 1));
    if count == 0 {
        0.0
    } else {
        to_f64(sum) / to_f64(count)
    }
}

/// A workload on a stack, over every frame of every repeat. Times are in microseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Summary {
    /// Timed frames in one repeat.
    pub(crate) frames: usize,
    pub(crate) render_p50: f64,
    pub(crate) render_p95: f64,
    pub(crate) encode_p50: f64,
    pub(crate) encode_p95: f64,
    pub(crate) bytes_mean: f64,
    pub(crate) bytes_max: usize,
    pub(crate) allocations: f64,
    pub(crate) alloc_bytes: f64,
}

/// Summarizes the repeats of one workload on one stack.
pub(crate) fn summarize(repeats: &[Vec<Sample>]) -> Summary {
    let all = || repeats.iter().flatten();
    let render = sorted(all().map(|s| s.render_ns));
    let encode = sorted(all().map(|s| s.encode_ns));
    Summary {
        frames: repeats.first().map_or(0, Vec::len),
        render_p50: micros(median(&render)),
        render_p95: micros(percentile(&render, 95)),
        encode_p50: micros(median(&encode)),
        encode_p95: micros(percentile(&encode, 95)),
        bytes_mean: mean(all().map(|s| s.bytes)),
        bytes_max: all().map(|s| s.bytes).max().unwrap_or(0),
        allocations: mean(all().map(|s| s.allocations)),
        alloc_bytes: mean(all().map(|s| s.alloc_bytes)),
    }
}

/// How far the median of each repeat strayed from the others. Times are in microseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Spread {
    pub(crate) repeats: usize,
    pub(crate) render_min: f64,
    pub(crate) render_max: f64,
    pub(crate) encode_min: f64,
    pub(crate) encode_max: f64,
}

impl Spread {
    /// The largest repeat's median over the smallest, as a percentage above it.
    pub(crate) fn percent(min: f64, max: f64) -> f64 {
        if min > 0.0 {
            (max - min) / min * 100.0
        } else {
            0.0
        }
    }
}

/// The range of the per-repeat medians.
pub(crate) fn spread(repeats: &[Vec<Sample>]) -> Spread {
    let medians = |pick: fn(&Sample) -> u64| -> Vec<f64> {
        repeats
            .iter()
            .map(|samples| micros(median(&sorted(samples.iter().map(pick)))))
            .collect()
    };
    let range = |values: Vec<f64>| {
        values.iter().fold((f64::MAX, 0.0_f64), |(min, max), v| {
            (min.min(*v), max.max(*v))
        })
    };
    let (render_min, render_max) = range(medians(|s| s.render_ns));
    let (encode_min, encode_max) = range(medians(|s| s.encode_ns));
    Spread {
        repeats: repeats.len(),
        render_min,
        render_max,
        encode_min,
        encode_max,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(render_ns: u64, encode_ns: u64, bytes: usize, allocations: usize) -> Sample {
        Sample {
            render_ns,
            encode_ns,
            bytes,
            allocations,
            alloc_bytes: allocations * 10,
        }
    }

    #[test]
    fn medians_of_odd_and_even_counts() {
        assert_eq!(median(&[]), 0);
        assert_eq!(median(&[5]), 5);
        assert_eq!(median(&[1, 3, 9]), 3);
        assert_eq!(median(&[1, 3, 5, 9]), 4);
    }

    #[test]
    fn percentiles_by_nearest_rank() {
        let values: Vec<u64> = (1..=100).collect();
        assert_eq!(percentile(&values, 95), 95);
        assert_eq!(percentile(&values, 50), 50);
        assert_eq!(percentile(&values, 100), 100);
        assert_eq!(percentile(&[7], 95), 7);
        assert_eq!(percentile(&[1, 2, 3, 4], 95), 4);
        assert_eq!(percentile(&[], 95), 0);
    }

    #[test]
    fn a_summary_pools_the_repeats() {
        let first: Vec<Sample> = (1..=4)
            .map(|n| sample(n * 1000, n * 2000, 100 * usize::try_from(n).unwrap(), 2))
            .collect();
        let second: Vec<Sample> = (5..=8)
            .map(|n| sample(n * 1000, n * 2000, 100 * usize::try_from(n).unwrap(), 4))
            .collect();
        let summary = summarize(&[first, second]);
        assert_eq!(summary.frames, 4);
        assert!((summary.render_p50 - 4.5).abs() < 1e-9);
        assert!((summary.render_p95 - 8.0).abs() < 1e-9);
        assert!((summary.encode_p50 - 9.0).abs() < 1e-9);
        assert!((summary.bytes_mean - 450.0).abs() < 1e-9);
        assert_eq!(summary.bytes_max, 800);
        assert!((summary.allocations - 3.0).abs() < 1e-9);
        assert!((summary.alloc_bytes - 30.0).abs() < 1e-9);
    }

    #[test]
    fn the_spread_is_the_range_of_the_medians() {
        let steady = vec![sample(10_000, 20_000, 1, 0); 5];
        let slow = vec![sample(12_000, 30_000, 1, 0); 5];
        let spread = spread(&[steady.clone(), slow, steady]);
        assert_eq!(spread.repeats, 3);
        assert!((spread.render_min - 10.0).abs() < 1e-9);
        assert!((spread.render_max - 12.0).abs() < 1e-9);
        assert!((spread.encode_min - 20.0).abs() < 1e-9);
        assert!((spread.encode_max - 30.0).abs() < 1e-9);
        assert!((Spread::percent(20.0, 30.0) - 50.0).abs() < 1e-9);
        assert!(Spread::percent(0.0, 5.0).abs() < 1e-9);
    }

    #[test]
    fn no_samples_summarize_to_zeros() {
        let summary = summarize(&[]);
        assert_eq!(summary.frames, 0);
        assert!(summary.bytes_mean.abs() < 1e-9);
        assert_eq!(summary.bytes_max, 0);
    }
}
