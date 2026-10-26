//! Deterministic latency endpoint shared by the Linux arm and a virtual-time model.
/// Frozen clean calibration, in nanoseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bound {
    /// Baseline nearest-rank p99.9.
    pub p999_ns: u64,
    /// Fourfold baseline margin.
    pub limit_ns: u64,
}
impl Bound {
    /// At least10000 and at most1000000 positive clean engine-completion samples.
    pub fn calibrate(samples: &[u64]) -> Result<Self, &'static str> {
        if !(10_000..=1_000_000).contains(&samples.len()) || samples.contains(&0) {
            return Err("need 10000..1000000 positive clean receipt latencies");
        }
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        let rank = samples
            .len()
            .checked_mul(999)
            .ok_or("quantile overflow")?
            .div_ceil(1000);
        let p999_ns = *sorted
            .get(rank.saturating_sub(1))
            .ok_or("quantile missing")?;
        let limit_ns = p999_ns.checked_mul(4).ok_or("latency bound overflow")?;
        Ok(Self { p999_ns, limit_ns })
    }
    /// Any over-bound sample fails; equality passes. No completion is always failure.
    pub fn passes(self, sample: Option<u64>) -> bool {
        sample.is_some_and(|ns| ns <= self.limit_ns)
    }
}
/// Virtual-time facts; no Windows watchdog policy is simulated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trace {
    /// Work announced by the guest.
    pub announced: u64,
    /// Global gate opens when the apply starts.
    pub gate_start: u64,
    /// Apply starts inside the hold.
    pub apply_start: u64,
    /// Apply ends before a gated publication.
    pub apply_end: u64,
    /// Work actually published.
    pub published: u64,
    /// Engine receipt, synthetic only in the virtual-time model.
    pub completed: u64,
}
/// Global-gate model: a packet arriving inside an apply waits, then executes.
/// An earlier publication is never delayed retroactively. Apply is inside gate time.
pub fn model(announced: u64, apply_start: u64, apply_ns: u64, execute_ns: u64) -> Trace {
    let apply_end = apply_start.saturating_add(apply_ns);
    let published = if announced >= apply_start && announced < apply_end {
        apply_end
    } else {
        announced
    };
    Trace {
        announced,
        gate_start: apply_start,
        apply_start,
        apply_end,
        published,
        completed: published.saturating_add(execute_ns),
    }
}
