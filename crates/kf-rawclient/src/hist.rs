//! A **bounded** latency histogram: a fixed array of counters, never a vector of samples.
//!
//! Four sub-buckets per power of two (relative error at most 25%), from 1 ns to 2^44 ns (about 4.9
//! hours); larger values clamp into the last bucket, and the exact `max`, `min`, `sum` and count are
//! kept beside the buckets. A quantile is reported as the **upper bound** of its bucket clamped to the
//! exact maximum, so a reported percentile is never below the true one: the safe direction for a stall
//! gate. Memory is 180 counters whatever the sample count, which is what lets the sliced diagnostic
//! (98,304 calls per refresh) share the code with the 24-call shape.

/// Highest power-of-two octave with its own buckets (2^44 ns).
pub const MAX_OCTAVE: u32 = 44;
/// Sub-buckets per octave.
pub const SUBS: u32 = 4;
/// Total bucket count.
pub const BUCKETS: usize = ((MAX_OCTAVE + 1) * SUBS) as usize;

/// A fixed-size latency histogram in nanoseconds.
#[derive(Debug, Clone)]
pub struct LogHist {
    counts: [u32; BUCKETS],
    n: u64,
    sum_ns: u64,
    max_ns: u64,
    min_ns: u64,
}

impl Default for LogHist {
    fn default() -> Self {
        Self::new()
    }
}

/// The bucket index of `ns`.
#[must_use]
pub fn bucket_of(ns: u64) -> usize {
    if ns == 0 {
        return 0;
    }
    let octave = 63 - ns.leading_zeros();
    if octave > MAX_OCTAVE {
        return BUCKETS - 1;
    }
    let sub = ((u128::from(ns) << 2) >> octave) & 3;
    (octave * SUBS) as usize + sub as usize
}

/// The `[lower, upper)` nanosecond range of bucket `idx` (`upper >= lower`).
#[must_use]
pub fn bucket_range(idx: usize) -> (u64, u64) {
    let octave = (idx as u32) / SUBS;
    let sub = u128::from((idx as u32) % SUBS);
    let lo = ((4 + sub) << octave) >> 2;
    let hi = ((5 + sub) << octave) >> 2;
    let lo = u64::try_from(lo).unwrap_or(u64::MAX);
    let hi = u64::try_from(hi).unwrap_or(u64::MAX);
    (lo, hi.max(lo))
}

impl LogHist {
    /// An empty histogram.
    #[must_use]
    pub fn new() -> Self {
        LogHist {
            counts: [0; BUCKETS],
            n: 0,
            sum_ns: 0,
            max_ns: 0,
            min_ns: u64::MAX,
        }
    }

    /// Record one sample. Counters saturate rather than wrap.
    pub fn record(&mut self, ns: u64) {
        let i = bucket_of(ns);
        self.counts[i] = self.counts[i].saturating_add(1);
        self.n = self.n.saturating_add(1);
        self.sum_ns = self.sum_ns.saturating_add(ns);
        self.max_ns = self.max_ns.max(ns);
        self.min_ns = self.min_ns.min(ns);
    }

    /// Samples recorded.
    #[must_use]
    pub fn count(&self) -> u64 {
        self.n
    }

    /// Exact maximum, 0 when empty.
    #[must_use]
    pub fn max_ns(&self) -> u64 {
        self.max_ns
    }

    /// Exact sum of samples.
    #[must_use]
    pub fn sum_ns(&self) -> u64 {
        self.sum_ns
    }

    /// The `permille`/1000 quantile (500 = median), or 0 when empty. Never below the true quantile,
    /// at most 25% above it (plus one nanosecond), and never above the exact maximum.
    #[must_use]
    pub fn quantile_ns(&self, permille: u32) -> u64 {
        if self.n == 0 {
            return 0;
        }
        let q = u64::from(permille.min(1000));
        let rank = (self.n * q).div_ceil(1000).max(1);
        let mut seen = 0u64;
        for (i, &c) in self.counts.iter().enumerate() {
            seen += u64::from(c);
            if seen >= rank {
                // The last bucket is the overflow bucket: it has no upper bound but the exact maximum.
                return if i == BUCKETS - 1 {
                    self.max_ns
                } else {
                    bucket_range(i).1.min(self.max_ns)
                };
            }
        }
        self.max_ns
    }

    /// Non-empty octaves as `(floor(log2(microseconds)), count)`, ascending; at most
    /// `MAX_OCTAVE + 1` entries. Sub-microsecond samples fall in octave 0.
    #[must_use]
    pub fn log2_us_buckets(&self) -> Vec<(u32, u64)> {
        let mut out: Vec<(u32, u64)> = Vec::new();
        for (i, &c) in self.counts.iter().enumerate() {
            if c == 0 {
                continue;
            }
            let us = (bucket_range(i).0 / 1000).max(1);
            let k = 63 - us.leading_zeros();
            match out.last_mut() {
                Some(last) if last.0 == k => last.1 += u64::from(c),
                _ => out.push((k, u64::from(c))),
            }
        }
        out
    }

    /// The `STRESS_HIST` line for operation `op`.
    #[must_use]
    pub fn line(&self, op: &str) -> String {
        let us = |ns: u64| format!("{}", ns / 1000);
        let buckets = self
            .log2_us_buckets()
            .iter()
            .map(|(k, c)| format!("{k}:{c}"))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "STRESS_HIST op={op} n={} p50_us={} p90_us={} p99_us={} max_us={} log2_buckets={}",
            self.n,
            us(self.quantile_ns(500)),
            us(self.quantile_ns(900)),
            us(self.quantile_ns(990)),
            us(self.max_ns),
            if buckets.is_empty() { "-" } else { &buckets },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn splitmix(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    #[test]
    fn memory_is_fixed_whatever_the_sample_count() {
        let mut h = LogHist::new();
        let before = std::mem::size_of_val(&h);
        for i in 0..200_000u64 {
            h.record(i * 37);
        }
        assert_eq!(std::mem::size_of_val(&h), before);
        assert_eq!(h.count(), 200_000);
        assert!(before < 1024, "{before}");
    }

    #[test]
    fn bucket_ranges_tile_the_line_and_contain_their_values() {
        for idx in 0..BUCKETS - 1 {
            let (lo, hi) = bucket_range(idx);
            let (nlo, _) = bucket_range(idx + 1);
            assert!(lo <= hi);
            assert!(hi <= nlo || hi == lo, "idx {idx}: {hi} > {nlo}");
        }
        let mut s = 7u64;
        for _ in 0..20_000 {
            let v = splitmix(&mut s) >> (splitmix(&mut s) % 60);
            if v == 0 {
                continue;
            }
            let i = bucket_of(v);
            let (lo, hi) = bucket_range(i);
            if i < BUCKETS - 1 {
                assert!(lo <= v && (v < hi || hi == lo), "v={v} idx={i} [{lo},{hi})");
            }
        }
    }

    #[test]
    fn quantiles_are_upper_bounds_within_a_quarter() {
        let mut s = 99u64;
        for trial in 0..50 {
            let n = 1 + (splitmix(&mut s) % 3000) as usize;
            let mut vals: Vec<u64> = (0..n)
                .map(|_| 1 + (splitmix(&mut s) >> 20) % (1 << (10 + trial % 24)))
                .collect();
            let mut h = LogHist::new();
            for &v in &vals {
                h.record(v);
            }
            vals.sort_unstable();
            for q in [1u32, 100, 500, 900, 990, 1000] {
                let rank = ((n as u64 * u64::from(q)).div_ceil(1000)).max(1) as usize;
                let truth = vals[rank - 1];
                let got = h.quantile_ns(q);
                assert!(got >= truth, "q={q} got={got} truth={truth}");
                assert!(
                    got <= truth + truth / 4 + 2,
                    "q={q} got={got} truth={truth}"
                );
                assert!(got <= h.max_ns());
            }
            assert!(h.quantile_ns(500) <= h.quantile_ns(900));
            assert!(h.quantile_ns(900) <= h.quantile_ns(990));
            assert!(h.quantile_ns(990) <= h.quantile_ns(1000));
            assert_eq!(h.quantile_ns(1000), *vals.last().unwrap());
        }
    }

    #[test]
    fn empty_extremes_and_clamping() {
        let h = LogHist::new();
        assert_eq!(h.quantile_ns(500), 0);
        assert!(h.log2_us_buckets().is_empty());
        assert!(h.line("map").contains("n=0") && h.line("map").ends_with("log2_buckets=-"));
        let mut h = LogHist::new();
        h.record(0);
        h.record(u64::MAX);
        assert_eq!(h.max_ns(), u64::MAX);
        assert_eq!(h.count(), 2);
        assert_eq!(h.sum_ns(), u64::MAX, "sum saturates");
        assert_eq!(bucket_of(u64::MAX), BUCKETS - 1);
        assert_eq!(h.quantile_ns(1000), u64::MAX);
    }

    #[test]
    fn line_has_the_documented_fields() {
        let mut h = LogHist::new();
        for us in [100u64, 200, 300, 4000] {
            h.record(us * 1000);
        }
        let l = h.line("map");
        for key in [
            "STRESS_HIST op=map",
            "n=4",
            "p50_us=",
            "p90_us=",
            "p99_us=",
            "max_us=4000",
            "log2_buckets=",
        ] {
            assert!(l.contains(key), "{key} in {l}");
        }
        let total: u64 = h.log2_us_buckets().iter().map(|b| b.1).sum();
        assert_eq!(total, 4);
        assert!(h.log2_us_buckets().len() <= (MAX_OCTAVE + 1) as usize);
    }
}
