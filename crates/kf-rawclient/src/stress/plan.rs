//! The memory plan: how much anonymous memory the arm may hold at its peak, decided before it allocates
//! anything, from `MemAvailable` and the caps. A plan that does not fit is NOTRUN (rc 3), **never an
//! out-of-memory kill** (V3_RAW_CLIENT.md §4.2.3).

use super::calibration::{PEAK_MEM_PERMILLE, PRESSURE_MAX_BYTES, PRESSURE_PERMILLE};
use super::params::StressParams;
use crate::outcome::NotRunReason;

/// Page size the pressure region is rounded to.
const ROUND: u64 = 4096;

/// What the arm will allocate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemPlan {
    /// Bytes of the pressure region (0 when pressure is off).
    pub pressure_bytes: u64,
    /// Bytes of the workload region.
    pub workload_bytes: u64,
    /// Bytes of the staging region.
    pub staging_bytes: u64,
}

impl MemPlan {
    /// The peak anonymous memory: all three regions live at once (the pressure region is released only
    /// after qualification).
    #[must_use]
    pub fn peak_bytes(&self) -> u64 {
        self.pressure_bytes + self.workload_bytes + self.staging_bytes
    }
}

/// Plan the memory for `p` given `avail` bytes of `MemAvailable`.
///
/// `X = min(0.45 x avail, 1 GiB, cap - workload - staging)`, rounded down to a page. The peak
/// `X + workload + staging` must be at most `0.8 x avail`.
///
/// # Errors
/// [`NotRunReason::TooLittleMemory`] with the numbers.
pub fn plan_memory(p: &StressParams, avail: u64) -> Result<MemPlan, (NotRunReason, String)> {
    let workload = p.workload_bytes();
    let staging = p.staging_bytes();
    let cap = p.mem_cap_mib << 20;
    let fixed = workload + staging;
    let no = |what: &str, peak: u64| {
        (
            NotRunReason::TooLittleMemory,
            format!(
                "{what} peak_mib={} avail_mib={} cap_mib={}",
                peak >> 20,
                avail >> 20,
                p.mem_cap_mib
            ),
        )
    };
    if fixed > cap {
        return Err(no("workload-exceeds-cap", fixed));
    }
    let limit = avail / 1000 * PEAK_MEM_PERMILLE;
    let pressure = if p.pressure {
        let x = (avail / 1000 * PRESSURE_PERMILLE)
            .min(PRESSURE_MAX_BYTES)
            .min(cap - fixed);
        x / ROUND * ROUND
    } else {
        0
    };
    let plan = MemPlan {
        pressure_bytes: pressure,
        workload_bytes: workload,
        staging_bytes: staging,
    };
    if plan.peak_bytes() > limit {
        return Err(no("peak-exceeds-80pct-of-avail", plan.peak_bytes()));
    }
    Ok(plan)
}

/// `MemAvailable` out of `/proc/meminfo` text, bytes.
#[must_use]
pub fn parse_mem_available(meminfo: &str) -> Option<u64> {
    let line = meminfo.lines().find(|l| l.starts_with("MemAvailable:"))?;
    let kib: u64 = line
        .trim_start_matches("MemAvailable:")
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()?;
    kib.checked_mul(1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1 << 30;

    #[test]
    fn a_roomy_machine_gets_the_documented_pressure() {
        let p = StressParams::default();
        let m = plan_memory(&p, 8 * GIB).unwrap();
        assert_eq!(m.workload_bytes, 384 << 20);
        assert_eq!(m.staging_bytes, 6 << 20);
        // min(0.45 x 8 GiB = 3.6 GiB, 1 GiB, 1536 - 390 MiB = 1146 MiB) = 1 GiB.
        assert_eq!(m.pressure_bytes, GIB);
        assert!(m.peak_bytes() <= 8 * GIB / 10 * 8);
    }

    #[test]
    fn pressure_shrinks_with_available_memory_and_with_the_cap() {
        let p = StressParams::default();
        // 2 GiB available: 0.45 x 2 GiB = 921 MiB; peak 921 + 390 = 1311 MiB <= 0.8 x 2048 = 1638.
        let m = plan_memory(&p, 2 * GIB).unwrap();
        assert!(
            m.pressure_bytes < GIB && m.pressure_bytes > 900 << 20,
            "{m:?}"
        );
        assert_eq!(m.pressure_bytes % 4096, 0);
        // A tight cap leaves room for the workload and a little pressure only.
        let p = StressParams {
            mem_cap_mib: 512,
            ..StressParams::default()
        };
        let m = plan_memory(&p, 8 * GIB).unwrap();
        assert_eq!(m.pressure_bytes, (512 << 20) - (390 << 20));
        assert!(m.peak_bytes() <= 512 << 20);
    }

    #[test]
    fn too_little_memory_is_notrun_not_an_oom() {
        let p = StressParams::default();
        // 500 MiB available: the 390 MiB fixed part alone is 0.78 -> with pressure it exceeds 80%.
        let (r, d) = plan_memory(&p, 500 << 20).unwrap_err();
        assert_eq!(r, NotRunReason::TooLittleMemory);
        assert!(d.contains("peak-exceeds-80pct-of-avail"), "{d}");
        // Zero and tiny availability never underflow or panic.
        for avail in [0u64, 1, 4096, 1 << 20] {
            assert!(plan_memory(&p, avail).is_err(), "{avail}");
        }
        // Without pressure the same machine may still be enough for the bare workload.
        let np = StressParams {
            pressure: false,
            ..StressParams::default()
        };
        assert_eq!(plan_memory(&np, 500 << 20).unwrap().pressure_bytes, 0);
        assert!(plan_memory(&np, 400 << 20).is_err());
    }

    #[test]
    fn the_peak_never_exceeds_the_stated_fractions_for_any_availability() {
        use crate::stress::rng::SplitMix64;
        let mut rng = SplitMix64::new(77);
        for _ in 0..2000 {
            let avail = rng.below(64 * GIB);
            for pressure in [true, false] {
                let p = StressParams {
                    pressure,
                    ..StressParams::default()
                };
                if let Ok(m) = plan_memory(&p, avail) {
                    assert!(m.peak_bytes() <= avail / 1000 * 800, "avail={avail}");
                    assert!(m.peak_bytes() <= p.mem_cap_mib << 20);
                    assert!(m.pressure_bytes <= GIB);
                }
            }
        }
    }

    #[test]
    fn meminfo_parsing_is_strict() {
        let t = "MemTotal:  8000000 kB\nMemAvailable:    6291456 kB\nBuffers: 1 kB\n";
        assert_eq!(parse_mem_available(t), Some(6 * GIB));
        assert_eq!(parse_mem_available("MemTotal: 1 kB\n"), None);
        assert_eq!(parse_mem_available("MemAvailable: lots\n"), None);
        assert_eq!(
            parse_mem_available("MemAvailable: 18446744073709551615 kB\n"),
            None,
            "overflow is refused"
        );
        assert_eq!(parse_mem_available(""), None);
    }
}
