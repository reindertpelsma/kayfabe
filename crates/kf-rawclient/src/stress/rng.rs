//! Seeded, dependency-free randomness: splitmix64, a Fisher-Yates permutation and distinct sampling.
//! Deterministic for a given seed, so a printed seed reproduces a run's memory layout order and sample.

/// splitmix64 (Vigna). Also the page-signature mixer.
#[derive(Debug, Clone)]
pub struct SplitMix64 {
    state: u64,
}

/// One splitmix64 finalisation of `x`: a pure mixing function.
#[must_use]
pub fn mix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl SplitMix64 {
    /// A generator from `seed`.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        SplitMix64 { state: seed }
    }

    /// The next 64 bits.
    pub fn next_u64(&mut self) -> u64 {
        let out = mix(self.state);
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        out
    }

    /// A value in `0..bound` (`bound > 0`), by rejection so there is no modulo bias.
    pub fn below(&mut self, bound: u64) -> u64 {
        assert!(bound > 0, "below(0)");
        let zone = u64::MAX - (u64::MAX % bound);
        loop {
            let v = self.next_u64();
            if v < zone {
                return v % bound;
            }
        }
    }
}

/// A uniformly random permutation of `0..n` (Fisher-Yates).
///
/// # Panics
/// If `n` does not fit `u32` (callers bound it far lower: `MAX_TOTAL_PAGES`).
#[must_use]
pub fn permutation(n: u32, seed: u64) -> Vec<u32> {
    let mut v: Vec<u32> = (0..n).collect();
    let mut rng = SplitMix64::new(seed);
    for i in (1..v.len()).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        v.swap(i, j);
    }
    v
}

/// `k` distinct values of `lo..hi`, in the order drawn (a partial Fisher-Yates over a sparse map: memory
/// is `O(k)`, not `O(hi - lo)`).
///
/// # Panics
/// If `k > hi - lo`.
#[must_use]
pub fn choose_distinct(lo: u32, hi: u32, k: u32, rng: &mut SplitMix64) -> Vec<u32> {
    let n = u64::from(hi - lo);
    assert!(u64::from(k) <= n, "choose_distinct: k > range");
    let mut moved: std::collections::BTreeMap<u64, u64> = std::collections::BTreeMap::new();
    let mut out = Vec::with_capacity(k as usize);
    for i in 0..u64::from(k) {
        let j = i + rng.below(n - i);
        let vj = *moved.get(&j).unwrap_or(&j);
        let vi = *moved.get(&i).unwrap_or(&i);
        moved.insert(j, vi);
        out.push(lo + u32::try_from(vj).expect("below hi"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permutations_are_permutations_deterministic_and_seed_sensitive() {
        for n in [0u32, 1, 2, 3, 100, 4096] {
            let p = permutation(n, 42);
            let mut sorted = p.clone();
            sorted.sort_unstable();
            assert!(sorted.iter().copied().eq(0..n), "n={n}");
            assert_eq!(p, permutation(n, 42));
        }
        assert_ne!(permutation(4096, 1), permutation(4096, 2));
        // A random permutation of 4096 is not the identity and not nearly sorted.
        let p = permutation(4096, 9);
        let fixed = p
            .iter()
            .enumerate()
            .filter(|(i, v)| *i as u32 == **v)
            .count();
        assert!(fixed < 40, "{fixed} fixed points");
        let ascents = p.windows(2).filter(|w| w[1] == w[0] + 1).count();
        assert!(ascents < 40, "{ascents} adjacent ascents");
    }

    #[test]
    fn distinct_choices_are_distinct_in_range_and_bounded_in_memory() {
        let mut rng = SplitMix64::new(5);
        for (lo, hi, k) in [
            (0u32, 10u32, 10u32),
            (100, 5000, 62),
            (7, 8, 1),
            (0, 1 << 30, 100),
        ] {
            let v = choose_distinct(lo, hi, k, &mut rng);
            assert_eq!(v.len(), k as usize);
            let mut s = v.clone();
            s.sort_unstable();
            s.dedup();
            assert_eq!(s.len(), k as usize, "{lo}..{hi} k={k}");
            assert!(v.iter().all(|&x| (lo..hi).contains(&x)));
        }
        assert!(choose_distinct(3, 9, 0, &mut rng).is_empty());
    }

    #[test]
    #[should_panic(expected = "k > range")]
    fn asking_for_more_than_exists_panics() {
        let mut rng = SplitMix64::new(1);
        let _ = choose_distinct(0, 3, 4, &mut rng);
    }

    #[test]
    fn below_is_unbiased_enough_and_in_range() {
        let mut rng = SplitMix64::new(11);
        let mut hits = [0u32; 6];
        for _ in 0..60_000 {
            hits[rng.below(6) as usize] += 1;
        }
        for h in hits {
            assert!((9_000..11_000).contains(&h), "{hits:?}");
        }
        assert_eq!(rng.below(1), 0);
    }

    #[test]
    fn mix_is_a_pure_avalanche() {
        assert_eq!(mix(1), mix(1));
        assert_ne!(mix(1), mix(2));
        // Flipping one input bit flips about half the output bits.
        let d = (mix(0x1234) ^ mix(0x1235)).count_ones();
        assert!((16..=48).contains(&d), "{d}");
    }
}
