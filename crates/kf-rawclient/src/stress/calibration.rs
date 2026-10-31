//! **Every threshold of the refresh stress, in one place.**
//!
//! Each constant is tagged with how much evidence stands behind it. `CALIBRATION-NEEDED` means: no
//! run behind this number; it is a starting guess, it is **report-only** until the calibration
//! step (V3_RAW_CLIENT.md §5.2 S7, at least 10 runs on at least 4 dies from the normal gate stream)
//! sets it from data, and changing it needs the owner's confirmation. Nothing outside this file may
//! contain a literal that decides a verdict.

/// CALIBRATION-NEEDED. Whether the guest/native ratios can fail the run. `false` until calibrated:
/// the ratio is printed and the compare step exits 0 even when a ratio exceeds its bound
/// (`exceeds-advisory`). Flipping this is the one-line change S7 describes.
pub const RATIO_GATES: bool = false;

/// CALIBRATION-NEEDED (no data behind it; V3_RAW_CLIENT.md §4.7 "my starting guess is K1 = 30").
/// Upper bound of `guest critical_ms median / native critical_ms median`.
pub const K1_CRITICAL_RATIO: f64 = 30.0;

/// CALIBRATION-NEEDED (no data behind it; §4.7 "K2 = 40"). Upper bound of
/// `guest map p99 / native map p99`.
pub const K2_MAP_P99_RATIO: f64 = 40.0;

/// CALIBRATION-NEEDED. The ratio may gate only if its coefficient of variation over repeated runs on
/// one box is below this (falsifier F4). Above it, fall back to the absolute caps and the Xid watch.
pub const MAX_RATIO_CV: f64 = 0.30;

/// The fewest repeated runs on which a coefficient of variation is believed (falsifier F4 says 5).
pub const MIN_RUNS_FOR_CV: usize = 5;

/// INFERRED, not observed on this arm: the floor of `runs / pages` per batch below which the memory
/// is not fragmented enough to look like the recorded Windows input (44,274 stitch pieces over
/// 12 x 4,096 pages = 0.9007, with that row's page count itself assumed). Parts per thousand. The
/// caller may raise or lower it with `--stress-min-run-ratio`; a run reports the value it used.
pub const QUALIFY_MIN_RUN_PERMILLE: u32 = 900;

/// A single RM call or fence wait longer than this is the "hang class" (G2). Nanoseconds. This is the
/// design's 10 s, the same as the per-phase watchdog default; `--stress-phase-s` moves both.
pub const STALL_NS: u64 = 10_000_000_000;

/// 2 s is WDDM's default TDR delay. The arm counts calls above it (`stall_over_2s`) and does **not** gate
/// on it, because a nested Vast box inflates guest time by an unknown factor (§4.7 G2).
pub const TDR_REFERENCE_NS: u64 = 2_000_000_000;

/// The fewest completed refreshes a PASS needs: with one, the epoch check (a stale translation from the
/// previous refresh) cannot be made, and a single timing is not a median.
pub const MIN_COMPLETED_REFRESHES: u32 = 2;

/// Fraction (per thousand) of `MemAvailable` the arm's peak anonymous memory may reach (§4.2.3: 0.8).
pub const PEAK_MEM_PERMILLE: u64 = 800;

/// Fraction (per thousand) of `MemAvailable` the pressure region may take (§4.2.2: 0.45).
pub const PRESSURE_PERMILLE: u64 = 450;

/// Ceiling of the pressure region (§4.2.2: 1 GiB).
pub const PRESSURE_MAX_BYTES: u64 = 1 << 30;
