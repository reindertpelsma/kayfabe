//! The refresh stress's parameters: flags, safe defaults taken from the design, and validation.
//!
//! Defaults are the Windows refresh shape (V3_RAW_CLIENT.md §4.1): 24 batches x 4,096 pages of
//! 4 KiB = 98,304 pages = 384 MiB, three refreshes, a 1,536-page oracle sample. Every value is validated
//! against a bound that is named here; a value outside it is **refused by name**, never clamped. The
//! bounds exist because the client may run as guest userspace and its arguments come from a harness
//! that a hostile guest could have influenced: no flag can make it allocate, loop or run without limit.

use super::calibration::{QUALIFY_MIN_RUN_PERMILLE, STALL_NS};
use std::fmt;

/// Most batches per refresh.
pub const MAX_BATCHES: u32 = 64;
/// Most pages per batch.
pub const MAX_PAGES_PER_BATCH: u32 = 16_384;
/// Most pages in a whole refresh (1 GiB of 4 KiB pages). Bounds every per-page array.
pub const MAX_TOTAL_PAGES: u64 = 262_144;
/// Most refreshes.
pub const MAX_REFRESHES: u32 = 8;
/// Most oracle sample pages per refresh (also the staging slot count).
pub const MAX_SAMPLE: u32 = 8_192;
/// Smallest allowed leaf (one 4 KiB page).
pub const MIN_LEAF_BYTES: u64 = 4096;
/// Largest allowed leaf (64 KiB: the next GMMU page size up; the arm only qualifies on the small page).
pub const MAX_LEAF_BYTES: u64 = 65_536;
/// Longest accepted flag value, in bytes.
pub const MAX_VALUE_BYTES: usize = 32;

/// How a batch is described to RM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// One OS descriptor and one `MAP_MEMORY_DMA` per batch: the host-stitch shape.
    Whole,
    /// One map call per page (98,304 per refresh): the per-run shape, a diagnostic.
    Sliced,
}

impl Shape {
    /// The flag word.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Shape::Whole => "whole",
            Shape::Sliced => "sliced",
        }
    }
}

/// A refused flag value: the flag's stable name and the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamError {
    /// Stable kebab-case name of the refusal.
    pub name: &'static str,
    /// The offending value and the bound.
    pub detail: String,
}

impl fmt::Display for ParamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "refused {}: {}", self.name, self.detail)
    }
}

impl std::error::Error for ParamError {}

fn err(name: &'static str, detail: impl Into<String>) -> ParamError {
    ParamError {
        name,
        detail: detail.into(),
    }
}

/// The validated parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StressParams {
    /// Map batches per refresh.
    pub batches: u32,
    /// Pages per batch.
    pub pages_per_batch: u32,
    /// Bytes per page (the leaf): the grain every mapping and the census work in.
    pub leaf_bytes: u64,
    /// Refreshes to attempt (a refresh may be skipped by the deadline).
    pub refreshes: u32,
    /// Oracle sample pages per refresh (a multiple of `batches`).
    pub sample: u32,
    /// Sampled pages the CPU rewrites after the map to prove the PTE points at the live page.
    pub live: u32,
    /// Seed; `None` = the caller draws one and prints it.
    pub seed: Option<u64>,
    /// Stop starting new refreshes after this many seconds (soft cap).
    pub deadline_s: u32,
    /// Hard cap on the whole run; the watchdog exits 124 when it passes.
    pub hard_cap_s: u32,
    /// Hard cap on one phase or one RM call; the watchdog and the stall gate both use it.
    pub phase_s: u32,
    /// Cap on peak anonymous memory, MiB.
    pub mem_cap_mib: u64,
    /// Required `runs / pages` per batch, parts per thousand.
    pub min_run_permille: u32,
    /// Build the pressure region before the workload (default on).
    pub pressure: bool,
    /// Whole-batch or per-page maps.
    pub shape: Shape,
}

impl Default for StressParams {
    fn default() -> Self {
        StressParams {
            batches: 24,
            pages_per_batch: 4096,
            leaf_bytes: 4096,
            refreshes: 3,
            sample: 1536,
            live: 32,
            seed: None,
            deadline_s: 50,
            hard_cap_s: 90,
            phase_s: u32::try_from(STALL_NS / 1_000_000_000).unwrap_or(10),
            mem_cap_mib: 1536,
            min_run_permille: QUALIFY_MIN_RUN_PERMILLE,
            pressure: true,
            shape: Shape::Whole,
        }
    }
}

fn int<T: std::str::FromStr + PartialOrd + Copy + fmt::Display>(
    name: &'static str,
    v: &str,
    lo: T,
    hi: T,
) -> Result<T, ParamError> {
    let n: T = v
        .parse()
        .map_err(|_| err(name, format!("{v:?} is not a whole number")))?;
    if n < lo || n > hi {
        return Err(err(name, format!("{n} is outside {lo}..={hi}")));
    }
    Ok(n)
}

fn parse_seed(v: &str) -> Result<u64, ParamError> {
    let h = v.strip_prefix("0x").unwrap_or(v);
    if h.is_empty() || h.len() > 16 {
        return Err(err("seed", format!("{v:?} is not 1-16 hex digits")));
    }
    u64::from_str_radix(h, 16).map_err(|_| err("seed", format!("{v:?} is not hex")))
}

/// `0.90` -> 900 (parts per thousand), exactly three decimals at most.
fn parse_ratio(v: &str) -> Result<u32, ParamError> {
    let bad = || err("min-run-ratio", format!("{v:?} is not a ratio like 0.90"));
    let (whole, frac) = v.split_once('.').unwrap_or((v, ""));
    if frac.len() > 3 || whole.is_empty() && frac.is_empty() {
        return Err(bad());
    }
    let w: u32 = if whole.is_empty() {
        0
    } else {
        whole.parse().map_err(|_| bad())?
    };
    let mut f: u32 = if frac.is_empty() {
        0
    } else {
        frac.parse().map_err(|_| bad())?
    };
    for _ in frac.len()..3 {
        f *= 10;
    }
    let permille = w
        .checked_mul(1000)
        .and_then(|x| x.checked_add(f))
        .ok_or_else(bad)?;
    if !(500..=1000).contains(&permille) {
        return Err(err(
            "min-run-ratio",
            format!("{v} is outside 0.5..=1.0 (a lower floor would qualify unfragmented memory)"),
        ));
    }
    Ok(permille)
}

impl StressParams {
    /// Parse the arm's flags (everything after the arm name and the global flags) and validate.
    ///
    /// # Errors
    /// A [`ParamError`] naming the first refusal.
    pub fn parse(args: &[String]) -> Result<StressParams, ParamError> {
        let mut p = StressParams::default();
        let mut seen: Vec<&str> = Vec::new();
        let mut it = args.iter();
        while let Some(flag) = it.next() {
            let key = flag.as_str();
            if !key.starts_with("--stress-") {
                return Err(err("unknown-flag", format!("{key:?}")));
            }
            if seen.contains(&key) {
                return Err(err("duplicate-flag", key.to_owned()));
            }
            let v = it
                .next()
                .ok_or_else(|| err("missing-value", format!("{key} needs a value")))?;
            if v.len() > MAX_VALUE_BYTES {
                return Err(err(
                    "value-too-long",
                    format!("{key} value is {} bytes", v.len()),
                ));
            }
            let v = v.as_str();
            match key {
                "--stress-batches" => p.batches = int("batches", v, 1, MAX_BATCHES)?,
                "--stress-pages-per-batch" => {
                    p.pages_per_batch = int("pages-per-batch", v, 1, MAX_PAGES_PER_BATCH)?;
                }
                "--stress-leaf-bytes" => {
                    p.leaf_bytes = int("leaf-bytes", v, MIN_LEAF_BYTES, MAX_LEAF_BYTES)?
                }
                "--stress-refreshes" => p.refreshes = int("refreshes", v, 1, MAX_REFRESHES)?,
                "--stress-sample" => p.sample = int("sample", v, 2, MAX_SAMPLE)?,
                "--stress-live" => p.live = int("live", v, 1, MAX_SAMPLE)?,
                "--stress-seed" => p.seed = Some(parse_seed(v)?),
                "--stress-deadline-s" => p.deadline_s = int("deadline-s", v, 5, 600)?,
                "--stress-hard-cap-s" => p.hard_cap_s = int("hard-cap-s", v, 10, 900)?,
                "--stress-phase-s" => p.phase_s = int("phase-s", v, 1, 60)?,
                "--stress-mem-cap-mib" => p.mem_cap_mib = int("mem-cap-mib", v, 64, 65_536)?,
                "--stress-min-run-ratio" => p.min_run_permille = parse_ratio(v)?,
                "--stress-pressure" => {
                    p.pressure = match v {
                        "on" => true,
                        "off" => false,
                        other => return Err(err("pressure", format!("{other:?} is not on|off"))),
                    };
                }
                "--stress-shape" => {
                    p.shape = match v {
                        "whole" => Shape::Whole,
                        "sliced" => Shape::Sliced,
                        other => {
                            return Err(err("shape", format!("{other:?} is not whole|sliced")));
                        }
                    };
                }
                "--stress-invalidate" => match v {
                    "ctl" => {}
                    "memop" => {
                        return Err(err(
                            "invalidate-memop-not-built",
                            "the MEM_OP_A..D pushbuffer invalidate is a later step; only ctl exists",
                        ));
                    }
                    other => return Err(err("invalidate", format!("{other:?} is not ctl|memop"))),
                },
                other => return Err(err("unknown-flag", format!("{other:?}"))),
            }
            seen.push(key);
        }
        p.validate()?;
        Ok(p)
    }

    /// Cross-field validation: relations between values that no single flag can check.
    ///
    /// # Errors
    /// A [`ParamError`] naming the relation that fails.
    pub fn validate(&self) -> Result<(), ParamError> {
        if !self.leaf_bytes.is_power_of_two() || self.leaf_bytes < MIN_LEAF_BYTES {
            return Err(err(
                "leaf-bytes",
                format!("{} is not a power of two >= 4096", self.leaf_bytes),
            ));
        }
        let total = u64::from(self.batches) * u64::from(self.pages_per_batch);
        if total > MAX_TOTAL_PAGES {
            return Err(err(
                "total-pages",
                format!(
                    "{} batches x {} pages = {total} > {MAX_TOTAL_PAGES}",
                    self.batches, self.pages_per_batch
                ),
            ));
        }
        if !self.sample.is_multiple_of(self.batches) {
            return Err(err(
                "sample-not-multiple-of-batches",
                format!(
                    "{} is not a multiple of {} batches",
                    self.sample, self.batches
                ),
            ));
        }
        let per_batch = self.sample / self.batches;
        if per_batch < 2 {
            return Err(err(
                "sample-too-small",
                format!(
                    "{per_batch} per batch; the first and last page of every batch are always sampled"
                ),
            ));
        }
        if per_batch > self.pages_per_batch {
            return Err(err(
                "sample-exceeds-batch",
                format!(
                    "{per_batch} per batch > {} pages per batch",
                    self.pages_per_batch
                ),
            ));
        }
        if self.live > self.sample {
            return Err(err(
                "live-exceeds-sample",
                format!("{} > {}", self.live, self.sample),
            ));
        }
        if self.deadline_s > self.hard_cap_s {
            return Err(err(
                "deadline-exceeds-hard-cap",
                format!("{} > {}", self.deadline_s, self.hard_cap_s),
            ));
        }
        if self.workload_bytes() + self.staging_bytes() > self.mem_cap_mib << 20 {
            return Err(err(
                "workload-exceeds-mem-cap",
                format!(
                    "workload {} MiB + staging {} MiB > cap {} MiB",
                    self.workload_bytes() >> 20,
                    self.staging_bytes() >> 20,
                    self.mem_cap_mib
                ),
            ));
        }
        Ok(())
    }

    /// Pages in one refresh.
    #[must_use]
    pub fn total_pages(&self) -> u32 {
        self.batches * self.pages_per_batch
    }

    /// Bytes of one batch.
    #[must_use]
    pub fn batch_bytes(&self) -> u64 {
        u64::from(self.pages_per_batch) * self.leaf_bytes
    }

    /// Bytes of the whole workload region.
    #[must_use]
    pub fn workload_bytes(&self) -> u64 {
        u64::from(self.total_pages()) * self.leaf_bytes
    }

    /// Bytes of the staging region (one slot per sampled page).
    #[must_use]
    pub fn staging_bytes(&self) -> u64 {
        u64::from(self.sample) * self.leaf_bytes
    }

    /// Sample pages per batch.
    #[must_use]
    pub fn sample_per_batch(&self) -> u32 {
        self.sample / self.batches
    }

    /// The `STRESS_PARAMS` line (with the seed actually used).
    #[must_use]
    pub fn line(&self, seed: u64) -> String {
        format!(
            "STRESS_PARAMS batches={} pages_per_batch={} leaf={} refreshes={} sample={} live={} shape={} pressure={} deadline_s={} hard_cap_s={} phase_s={} mem_cap_mib={} min_run_ratio={:.3} seed={seed:#x}",
            self.batches,
            self.pages_per_batch,
            self.leaf_bytes,
            self.refreshes,
            self.sample,
            self.live,
            self.shape.word(),
            if self.pressure { "on" } else { "off" },
            self.deadline_s,
            self.hard_cap_s,
            self.phase_s,
            self.mem_cap_mib,
            f64::from(self.min_run_permille) / 1000.0,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(a: &[&str]) -> Result<StressParams, ParamError> {
        StressParams::parse(&a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn the_defaults_are_the_measured_shape() {
        let p = parse(&[]).unwrap();
        assert_eq!(p, StressParams::default());
        assert_eq!(p.total_pages(), 98_304);
        assert_eq!(p.batch_bytes(), 16 << 20);
        assert_eq!(p.workload_bytes(), 384 << 20);
        assert_eq!(p.staging_bytes(), 6 << 20, "1,536 pages of 4 KiB");
        assert_eq!(p.sample_per_batch(), 64);
        assert_eq!(
            (p.refreshes, p.live, p.deadline_s, p.phase_s),
            (3, 32, 50, 10)
        );
        assert!(p.pressure && p.shape == Shape::Whole);
        assert_eq!(p.min_run_permille, 900);
        // The default must itself be inside every bound it is checked against.
        assert!(p.validate().is_ok());
    }

    #[test]
    fn a_complete_valid_command_line_parses() {
        let p = parse(&[
            "--stress-batches",
            "12",
            "--stress-pages-per-batch",
            "2048",
            "--stress-sample",
            "384",
            "--stress-live",
            "8",
            "--stress-refreshes",
            "2",
            "--stress-seed",
            "0xDEADbeef",
            "--stress-deadline-s",
            "20",
            "--stress-hard-cap-s",
            "40",
            "--stress-phase-s",
            "5",
            "--stress-mem-cap-mib",
            "900",
            "--stress-min-run-ratio",
            "0.95",
            "--stress-pressure",
            "off",
            "--stress-shape",
            "sliced",
            "--stress-invalidate",
            "ctl",
            "--stress-leaf-bytes",
            "4096",
        ])
        .unwrap();
        assert_eq!(
            (p.batches, p.pages_per_batch, p.sample, p.live),
            (12, 2048, 384, 8)
        );
        assert_eq!(p.seed, Some(0xDEAD_BEEF));
        assert_eq!(p.min_run_permille, 950);
        assert!(!p.pressure && p.shape == Shape::Sliced);
        assert!(p.line(0xDEAD_BEEF).contains("seed=0xdeadbeef"));
    }

    #[test]
    fn hostile_and_odd_values_are_refused_by_name() {
        let name = |a: &[&str]| parse(a).unwrap_err().name;
        let cases: &[(&[&str], &str)] = &[
            (&["--stress-batches", "0"], "batches"),
            (&["--stress-batches", "65"], "batches"),
            (&["--stress-batches", "-1"], "batches"),
            (&["--stress-batches", "18446744073709551616"], "batches"),
            (&["--stress-batches", "24.5"], "batches"),
            (&["--stress-batches", ""], "batches"),
            (&["--stress-pages-per-batch", "0"], "pages-per-batch"),
            (&["--stress-pages-per-batch", "16385"], "pages-per-batch"),
            (
                &["--stress-batches", "64", "--stress-pages-per-batch", "8192"],
                "total-pages",
            ),
            (&["--stress-leaf-bytes", "4095"], "leaf-bytes"),
            (&["--stress-leaf-bytes", "6144"], "leaf-bytes"),
            (&["--stress-leaf-bytes", "2097152"], "leaf-bytes"),
            (&["--stress-leaf-bytes", "0"], "leaf-bytes"),
            (&["--stress-refreshes", "0"], "refreshes"),
            (&["--stress-refreshes", "9"], "refreshes"),
            (&["--stress-sample", "1"], "sample"),
            (&["--stress-sample", "8193"], "sample"),
            (
                &["--stress-sample", "100"],
                "sample-not-multiple-of-batches",
            ),
            (
                &["--stress-batches", "24", "--stress-sample", "24"],
                "sample-too-small",
            ),
            (
                &[
                    "--stress-batches",
                    "1",
                    "--stress-pages-per-batch",
                    "8",
                    "--stress-sample",
                    "9",
                ],
                "sample-exceeds-batch",
            ),
            (&["--stress-live", "0"], "live"),
            (&["--stress-live", "2000"], "live-exceeds-sample"),
            (&["--stress-seed", ""], "seed"),
            (&["--stress-seed", "xyz"], "seed"),
            (&["--stress-seed", "0x"], "seed"),
            (&["--stress-seed", "12345678901234567"], "seed"),
            (&["--stress-deadline-s", "4"], "deadline-s"),
            (&["--stress-deadline-s", "601"], "deadline-s"),
            (
                &["--stress-deadline-s", "100", "--stress-hard-cap-s", "90"],
                "deadline-exceeds-hard-cap",
            ),
            (&["--stress-hard-cap-s", "9"], "hard-cap-s"),
            (&["--stress-phase-s", "0"], "phase-s"),
            (&["--stress-phase-s", "61"], "phase-s"),
            (&["--stress-mem-cap-mib", "63"], "mem-cap-mib"),
            (&["--stress-mem-cap-mib", "100"], "workload-exceeds-mem-cap"),
            (&["--stress-min-run-ratio", "0.49"], "min-run-ratio"),
            (&["--stress-min-run-ratio", "1.001"], "min-run-ratio"),
            (&["--stress-min-run-ratio", "0.9999"], "min-run-ratio"),
            (&["--stress-min-run-ratio", "abc"], "min-run-ratio"),
            (&["--stress-min-run-ratio", "."], "min-run-ratio"),
            (&["--stress-min-run-ratio", "-0.9"], "min-run-ratio"),
            (&["--stress-pressure", "yes"], "pressure"),
            (&["--stress-shape", "bulk"], "shape"),
            (
                &["--stress-invalidate", "memop"],
                "invalidate-memop-not-built",
            ),
            (&["--stress-invalidate", "both"], "invalidate"),
            (&["--stress-nonsense", "1"], "unknown-flag"),
            (&["--batches", "4"], "unknown-flag"),
            (&["--stress-batches"], "missing-value"),
            (
                &["--stress-batches", "4", "--stress-batches", "5"],
                "duplicate-flag",
            ),
        ];
        for (args, want) in cases {
            assert_eq!(name(args), *want, "{args:?}");
        }
        let long = "9".repeat(MAX_VALUE_BYTES + 1);
        assert_eq!(name(&["--stress-batches", &long]), "value-too-long");
    }

    #[test]
    fn the_extreme_accepted_values_are_actually_bounded() {
        // The largest accepted workload is 1 GiB of 4 KiB pages; with 64 KiB leaves the page count shrinks
        // so that the memory bound still holds under --stress-mem-cap-mib.
        let p = parse(&[
            "--stress-batches",
            "64",
            "--stress-pages-per-batch",
            "4096",
            "--stress-mem-cap-mib",
            "2048",
            "--stress-sample",
            "8192",
        ])
        .unwrap();
        assert_eq!(p.total_pages(), 262_144);
        assert!(p.workload_bytes() + p.staging_bytes() <= 2048 << 20);
        // And the memory cap really is checked in bytes, with the leaf included.
        let e = parse(&[
            "--stress-leaf-bytes",
            "65536",
            "--stress-mem-cap-mib",
            "1536",
        ])
        .unwrap_err();
        assert_eq!(e.name, "workload-exceeds-mem-cap");
    }

    #[test]
    fn property_every_accepted_parameter_set_has_bounded_arrays() {
        // Deterministic sweep (no external property library in this workspace): whatever combination of
        // flag values parses, the per-page arrays the arm allocates stay inside the stated bounds.
        use crate::stress::rng::SplitMix64;
        let mut rng = SplitMix64::new(0xC0FFEE);
        let mut accepted = 0u32;
        for _ in 0..4000 {
            let a = [
                "--stress-batches".to_owned(),
                (rng.below(80)).to_string(),
                "--stress-pages-per-batch".to_owned(),
                (rng.below(20_000)).to_string(),
                "--stress-sample".to_owned(),
                (rng.below(9000)).to_string(),
                "--stress-leaf-bytes".to_owned(),
                (4096u64 << rng.below(5)).to_string(),
            ];
            if let Ok(p) = StressParams::parse(&a) {
                accepted += 1;
                assert!(u64::from(p.total_pages()) <= MAX_TOTAL_PAGES);
                assert!(p.sample <= MAX_SAMPLE && p.sample % p.batches == 0);
                assert!(p.workload_bytes() + p.staging_bytes() <= p.mem_cap_mib << 20);
                assert!(p.sample_per_batch() >= 2 && p.sample_per_batch() <= p.pages_per_batch);
            }
        }
        assert!(
            accepted > 20,
            "the sweep must reach the accepted region ({accepted})"
        );
    }
}
