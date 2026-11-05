//! The native-versus-guest ratio, computed from two result logs by **pure functions**.
//!
//! The client cannot compute a ratio itself: one process cannot know the other run. `--refresh-compare
//! NATIVE.log GUEST.log` reads the `RAWCLIENT_CELL`, `STRESS_REFRESH`, `STRESS_HIST` and `RUNG_` lines of
//! two runs and prints `STRESS_RATIO`. A ratio is meaningful only per arm and per box (the existing
//! guest/bare ratios run 0.4x to 22x, V3_RAW_CLIENT.md §2.1), so the comparison **refuses** logs whose
//! cells differ in anything but the mode, whose revision is unknown, whose mode labels are wrong, or
//! whose run did not pass.
//!
//! The thresholds are [`super::calibration`]'s; until `RATIO_GATES` is flipped by the calibration step an
//! exceeded bound is reported as `exceeds-advisory` and the exit code stays 0.

use super::calibration::{
    K1_CRITICAL_RATIO, K2_MAP_P99_RATIO, MAX_RATIO_CV, MIN_RUNS_FOR_CV, RATIO_GATES,
};
use crate::mode::Mode;
use std::collections::BTreeMap;

/// Largest log read, bytes.
pub const MAX_LOG_BYTES: usize = 1 << 20;
/// Most lines examined.
pub const MAX_LOG_LINES: usize = 20_000;
/// Most refresh records kept per log.
pub const MAX_REFRESH_RECORDS: usize = 64;

/// What was read from one log.
#[derive(Debug, Clone, PartialEq)]
pub struct ArmLog {
    /// The `RAWCLIENT_CELL` fields.
    pub cell: BTreeMap<String, String>,
    /// The mode in the cell.
    pub mode: Mode,
    /// The `RUNG_refresh_stress=` word.
    pub rung: Option<String>,
    /// `critical_ms` of each `STRESS_REFRESH`.
    pub critical_ms: Vec<f64>,
    /// `p99_us` of `STRESS_HIST op=map`.
    pub map_p99_us: Option<f64>,
}

/// A log that could not be read as a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub &'static str);

/// Cell fields that must be equal for two runs to be compared (everything but the mode).
const SAME_CELL: [&str; 6] = ["die", "family", "mmu", "drv", "kmod", "rev"];

fn fields(line: &str) -> BTreeMap<String, String> {
    line.split_whitespace()
        .skip(1)
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

/// Parse one log. Bounded in size, lines and records.
///
/// # Errors
/// [`ParseError`] naming what is missing or oversized.
pub fn parse_log(text: &str) -> Result<ArmLog, ParseError> {
    if text.len() > MAX_LOG_BYTES {
        return Err(ParseError("log-too-large"));
    }
    let mut cell = None;
    let mut rung = None;
    let mut critical_ms = Vec::new();
    let mut map_p99_us = None;
    for line in text.lines().take(MAX_LOG_LINES) {
        if line.starts_with("RAWCLIENT_CELL ") {
            if cell.replace(fields(line)).is_some() {
                return Err(ParseError("two-cell-lines"));
            }
        } else if line.starts_with("STRESS_REFRESH ") {
            if critical_ms.len() >= MAX_REFRESH_RECORDS {
                return Err(ParseError("too-many-refresh-records"));
            }
            let v = fields(line)
                .get("critical_ms")
                .and_then(|x| x.parse::<f64>().ok())
                .filter(|x| x.is_finite() && *x >= 0.0)
                .ok_or(ParseError("bad-critical-ms"))?;
            critical_ms.push(v);
        } else if line.starts_with("STRESS_HIST ") {
            let f = fields(line);
            if f.get("op").map(String::as_str) == Some("map") {
                map_p99_us = Some(
                    f.get("p99_us")
                        .and_then(|x| x.parse::<f64>().ok())
                        .filter(|x| x.is_finite() && *x >= 0.0)
                        .ok_or(ParseError("bad-map-p99"))?,
                );
            }
        } else if let Some(rest) = line.strip_prefix("RUNG_refresh_stress=") {
            rung = rest.split_whitespace().next().map(str::to_owned);
        }
    }
    let cell = cell.ok_or(ParseError("no-cell-line"))?;
    let mode = cell
        .get("mode")
        .and_then(|m| Mode::from_word(m))
        .ok_or(ParseError("bad-mode"))?;
    Ok(ArmLog {
        cell,
        mode,
        rung,
        critical_ms,
        map_p99_us,
    })
}

/// The median (mean of the middle two for an even count); `None` when empty.
#[must_use]
pub fn median(v: &[f64]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let n = s.len();
    Some(if n % 2 == 1 {
        s[n / 2]
    } else {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    })
}

/// Coefficient of variation (population standard deviation over the mean). `None` for fewer than two
/// values or a non-positive mean.
#[must_use]
pub fn cv(v: &[f64]) -> Option<f64> {
    if v.len() < 2 {
        return None;
    }
    let mean = v.iter().sum::<f64>() / v.len() as f64;
    if !mean.is_finite() || mean <= 0.0 {
        return None;
    }
    let var = v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / v.len() as f64;
    Some(var.sqrt() / mean)
}

/// Whether a set of repeated-run ratios is stable enough for the ratio to gate (falsifier F4):
/// at least [`MIN_RUNS_FOR_CV`] runs and a CV at most [`MAX_RATIO_CV`].
#[must_use]
pub fn ratio_gate_usable(ratios: &[f64]) -> bool {
    ratios.len() >= MIN_RUNS_FOR_CV && cv(ratios).is_some_and(|c| c <= MAX_RATIO_CV)
}

/// What the comparison concluded.
#[derive(Debug, Clone, PartialEq)]
pub enum CompareState {
    /// Both ratios are within their bounds.
    Within,
    /// A bound is exceeded and the ratio does not gate yet (`RATIO_GATES == false`).
    ExceedsAdvisory,
    /// A bound is exceeded and the ratio gates.
    Exceeds,
    /// The two logs cannot be compared; the reason is a stable word.
    Unusable(String),
}

/// The comparison.
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    /// `guest median critical_ms / native median critical_ms`.
    pub critical_ratio: Option<f64>,
    /// `guest map p99 / native map p99`.
    pub map_p99_ratio: Option<f64>,
    /// The conclusion.
    pub state: CompareState,
}

/// Thresholds, injectable so the rules are tested at their edges without editing the constants.
#[derive(Debug, Clone, Copy)]
pub struct Thresholds {
    /// Bound on the critical-time ratio.
    pub k1: f64,
    /// Bound on the map p99 ratio.
    pub k2: f64,
    /// Whether an exceeded bound fails.
    pub gates: bool,
}

impl Thresholds {
    /// The calibration constants.
    pub const CURRENT: Thresholds = Thresholds {
        k1: K1_CRITICAL_RATIO,
        k2: K2_MAP_P99_RATIO,
        gates: RATIO_GATES,
    };
}

fn unusable(reason: impl Into<String>) -> Comparison {
    Comparison {
        critical_ratio: None,
        map_p99_ratio: None,
        state: CompareState::Unusable(reason.into()),
    }
}

/// Compare a native run with a guest run.
#[must_use]
pub fn compare(native: &ArmLog, guest: &ArmLog, th: Thresholds) -> Comparison {
    if native.mode != Mode::Native {
        return unusable("native-log-mode-not-bare");
    }
    if guest.mode != Mode::Guest {
        return unusable("guest-log-mode-not-guest");
    }
    for k in SAME_CELL {
        let (a, b) = (native.cell.get(k), guest.cell.get(k));
        if a.is_none() || a != b {
            return unusable(format!("cells-differ-{k}"));
        }
    }
    if native.cell.get("rev").map(String::as_str) == Some("unknown") {
        return unusable("rev-unknown");
    }
    if native.rung.as_deref() != Some("PASS") {
        return unusable("native-not-pass");
    }
    if guest.rung.as_deref() != Some("PASS") {
        return unusable("guest-not-pass");
    }
    if native.critical_ms.len() < 2 || guest.critical_ms.len() < 2 {
        return unusable("too-few-refreshes");
    }
    let (Some(nm), Some(gm)) = (median(&native.critical_ms), median(&guest.critical_ms)) else {
        return unusable("too-few-refreshes");
    };
    if nm.is_nan() || nm <= 0.0 {
        return unusable("native-critical-zero");
    }
    let critical = gm / nm;
    let map = match (native.map_p99_us, guest.map_p99_us) {
        (Some(n), Some(g)) if n > 0.0 => Some(g / n),
        _ => None,
    };
    let Some(map_ratio) = map else {
        return Comparison {
            critical_ratio: Some(critical),
            map_p99_ratio: None,
            state: CompareState::Unusable("map-p99-missing-or-zero".into()),
        };
    };
    let over = critical > th.k1 || map_ratio > th.k2;
    Comparison {
        critical_ratio: Some(critical),
        map_p99_ratio: Some(map_ratio),
        state: match (over, th.gates) {
            (false, _) => CompareState::Within,
            (true, false) => CompareState::ExceedsAdvisory,
            (true, true) => CompareState::Exceeds,
        },
    }
}

impl Comparison {
    /// The exit code: 0 within or advisory, 1 exceeds (when gating), 3 unusable.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        match self.state {
            CompareState::Within | CompareState::ExceedsAdvisory => crate::outcome::EXIT_PASS,
            CompareState::Exceeds => crate::outcome::EXIT_FAIL,
            CompareState::Unusable(_) => crate::outcome::EXIT_NOTRUN,
        }
    }

    /// The `STRESS_RATIO` line.
    #[must_use]
    pub fn line(&self, th: Thresholds, cell: Option<&BTreeMap<String, String>>) -> String {
        let f = |x: Option<f64>| x.map_or_else(|| "-".to_owned(), |v| format!("{v:.3}"));
        let get = |k: &str| cell.and_then(|c| c.get(k)).map_or("-", String::as_str);
        let (state, reason) = match &self.state {
            CompareState::Within => ("within", "-".to_owned()),
            CompareState::ExceedsAdvisory => ("exceeds-advisory", "-".to_owned()),
            CompareState::Exceeds => ("exceeds", "-".to_owned()),
            CompareState::Unusable(r) => ("unusable", r.clone()),
        };
        format!(
            "STRESS_RATIO die={} drv={} rev={} critical_ratio={} map_p99_ratio={} k1={} k2={} gating={} state={state} reason={reason}",
            get("die"),
            get("drv"),
            get("rev"),
            f(self.critical_ratio),
            f(self.map_p99_ratio),
            th.k1,
            th.k2,
            if th.gates { "on" } else { "off" },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(mode: &str, rev: &str, rung: &str, crit: &[f64], p99: f64) -> String {
        let mut s = format!(
            "RAWCLIENT_CELL die=GA106 family=Ampere mmu=Ver2 drv=580.159.04 kmod=open mode={mode} rev={rev}\n"
        );
        for (i, c) in crit.iter().enumerate() {
            s += &format!(
                "STRESS_REFRESH n={i} pin_ms=1.0 map_ms=1.0 inval_ms=1.0 oracle_ms=1.0 unmap_ms=1.0 free_ms=1.0 critical_ms={c} max_call_ms=1.0\n"
            );
        }
        s += &format!(
            "STRESS_HIST op=map n=72 p50_us=1 p90_us=2 p99_us={p99} max_us=9 log2_buckets=0:1\n"
        );
        s += &format!("RUNG_refresh_stress={rung}\n");
        s
    }

    const T: Thresholds = Thresholds {
        k1: 30.0,
        k2: 40.0,
        gates: false,
    };
    const TG: Thresholds = Thresholds {
        k1: 30.0,
        k2: 40.0,
        gates: true,
    };

    fn cmp(n: &str, g: &str, th: Thresholds) -> Comparison {
        compare(&parse_log(n).unwrap(), &parse_log(g).unwrap(), th)
    }

    #[test]
    fn the_calibration_thresholds_do_not_gate_yet() {
        assert!(
            !std::hint::black_box(Thresholds::CURRENT.gates),
            "report-only until calibrated (S7)"
        );
        assert_eq!(
            (Thresholds::CURRENT.k1, Thresholds::CURRENT.k2),
            (30.0, 40.0)
        );
    }

    #[test]
    fn median_and_cv() {
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[3.0]), Some(3.0));
        assert_eq!(median(&[5.0, 1.0, 3.0]), Some(3.0));
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), Some(2.5));
        assert_eq!(cv(&[1.0]), None);
        assert_eq!(cv(&[0.0, 0.0]), None);
        let c = cv(&[10.0, 10.0, 10.0]).unwrap();
        assert!(c.abs() < 1e-12);
        let c = cv(&[1.0, 3.0]).unwrap();
        assert!((c - 0.5).abs() < 1e-12, "{c}");
        // Not fooled by NaN.
        assert_eq!(cv(&[f64::NAN, 1.0]), None);
    }

    #[test]
    fn gate_usability_needs_five_runs_and_a_small_cv() {
        assert!(
            !ratio_gate_usable(&[10.0, 10.0, 10.0, 10.0]),
            "four runs are not five"
        );
        assert!(ratio_gate_usable(&[10.0, 10.5, 9.5, 10.2, 9.8]));
        assert!(
            !ratio_gate_usable(&[1.0, 10.0, 1.0, 10.0, 1.0]),
            "CV above 0.30"
        );
        assert!(!ratio_gate_usable(&[]));
    }

    #[test]
    fn a_ratio_inside_both_bounds_is_within() {
        let n = log("bare", "r1", "PASS", &[10.0, 12.0, 11.0], 100.0);
        let g = log("guest", "r1", "PASS", &[110.0, 130.0, 120.0], 1000.0);
        let c = cmp(&n, &g, T);
        assert_eq!(c.state, CompareState::Within);
        assert!((c.critical_ratio.unwrap() - 120.0 / 11.0).abs() < 1e-9);
        assert!((c.map_p99_ratio.unwrap() - 10.0).abs() < 1e-9);
        assert_eq!(c.exit_code(), 0);
        let line = c.line(T, parse_log(&n).ok().as_ref().map(|l| &l.cell));
        assert!(line.starts_with("STRESS_RATIO die=GA106 drv=580.159.04 rev=r1 critical_ratio=10.909 map_p99_ratio=10.000 k1=30 k2=40 gating=off state=within"), "{line}");
    }

    #[test]
    fn each_bound_is_exclusive_and_advisory_does_not_fail() {
        let n = log("bare", "r1", "PASS", &[10.0, 10.0], 100.0);
        // Exactly K1: not exceeded. Just above: exceeded.
        let at = log("guest", "r1", "PASS", &[300.0, 300.0], 4000.0);
        assert_eq!(cmp(&n, &at, TG).state, CompareState::Within);
        let over1 = log("guest", "r1", "PASS", &[300.1, 300.1], 100.0);
        assert_eq!(cmp(&n, &over1, TG).state, CompareState::Exceeds);
        assert_eq!(cmp(&n, &over1, T).state, CompareState::ExceedsAdvisory);
        let over2 = log("guest", "r1", "PASS", &[10.0, 10.0], 4000.1);
        assert_eq!(cmp(&n, &over2, TG).state, CompareState::Exceeds);
        assert_eq!(cmp(&n, &over1, TG).exit_code(), 1);
        assert_eq!(cmp(&n, &over1, T).exit_code(), 0, "advisory exits 0");
    }

    #[test]
    fn logs_that_cannot_be_compared_are_unusable_by_name() {
        let n = log("bare", "r1", "PASS", &[10.0, 12.0], 100.0);
        let g = log("guest", "r1", "PASS", &[100.0, 120.0], 1000.0);
        let reason = |n: &str, g: &str| match cmp(n, g, T).state {
            CompareState::Unusable(r) => r,
            other => panic!("{other:?}"),
        };
        assert_eq!(reason(&g, &g), "native-log-mode-not-bare");
        assert_eq!(reason(&n, &n), "guest-log-mode-not-guest");
        assert_eq!(
            reason(&log("unknown", "r1", "PASS", &[1.0, 1.0], 1.0), &g),
            "native-log-mode-not-bare"
        );
        assert_eq!(reason(&n, &g.replace("GA106", "AD104")), "cells-differ-die");
        assert_eq!(
            reason(&n, &g.replace("580.159.04", "590.1")),
            "cells-differ-drv"
        );
        assert_eq!(
            reason(&n, &g.replace("rev=r1", "rev=r2")),
            "cells-differ-rev"
        );
        assert_eq!(
            reason(&n, &g.replace("kmod=open", "kmod=proprietary")),
            "cells-differ-kmod"
        );
        assert_eq!(
            reason(
                &n.replace("rev=r1", "rev=unknown"),
                &g.replace("rev=r1", "rev=unknown")
            ),
            "rev-unknown"
        );
        assert_eq!(
            reason(&log("bare", "r1", "FAIL", &[10.0, 12.0], 1.0), &g),
            "native-not-pass"
        );
        assert_eq!(
            reason(&log("bare", "r1", "NOTRUN", &[10.0, 12.0], 1.0), &g),
            "native-not-pass"
        );
        assert_eq!(
            reason(&n, &log("guest", "r1", "FAIL", &[1.0, 2.0], 1.0)),
            "guest-not-pass"
        );
        assert_eq!(
            reason(&n, &log("guest", "r1", "PASS", &[1.0], 1.0)),
            "too-few-refreshes"
        );
        assert_eq!(
            reason(&log("bare", "r1", "PASS", &[0.0, 0.0], 1.0), &g),
            "native-critical-zero"
        );
        assert_eq!(
            reason(&log("bare", "r1", "PASS", &[1.0, 1.0], 0.0), &g),
            "map-p99-missing-or-zero"
        );
        assert_eq!(
            cmp(&n, &log("guest", "r1", "FAIL", &[1.0, 2.0], 1.0), T).exit_code(),
            3
        );
    }

    #[test]
    fn hostile_logs_are_refused_or_bounded() {
        let e = |t: &str| parse_log(t).unwrap_err().0;
        assert_eq!(e(""), "no-cell-line");
        assert_eq!(e("RAWCLIENT_CELL mode=weird\n"), "bad-mode");
        let two = "RAWCLIENT_CELL mode=bare\nRAWCLIENT_CELL mode=bare\n";
        assert_eq!(e(two), "two-cell-lines");
        let big = "x".repeat(MAX_LOG_BYTES + 1);
        assert_eq!(e(&big), "log-too-large");
        let bad = "RAWCLIENT_CELL mode=bare\nSTRESS_REFRESH n=0 critical_ms=NaN\n";
        assert_eq!(e(bad), "bad-critical-ms");
        let neg = "RAWCLIENT_CELL mode=bare\nSTRESS_REFRESH n=0 critical_ms=-1\n";
        assert_eq!(e(neg), "bad-critical-ms");
        let mut many = String::from("RAWCLIENT_CELL mode=bare\n");
        for i in 0..=MAX_REFRESH_RECORDS {
            many += &format!("STRESS_REFRESH n={i} critical_ms=1\n");
        }
        assert_eq!(e(&many), "too-many-refresh-records");
        let nop99 = "RAWCLIENT_CELL mode=bare\nSTRESS_HIST op=map n=1 p99_us=x\n";
        assert_eq!(e(nop99), "bad-map-p99");
        // Text that merely looks like a result line inside another line's value is not one.
        let ok = parse_log("RAWCLIENT_CELL mode=bare note=RUNG_refresh_stress=PASS\n").unwrap();
        assert_eq!(ok.rung, None);
    }
}
