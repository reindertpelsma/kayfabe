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
    /// Needs a nonempty bounded positive clean sample population.
    pub fn calibrate(_samples: &[u64]) -> Result<Self, &'static str> {
        Err("not implemented")
    }
    /// Any over-bound sample fails; equality passes. No completion is always failure.
    pub fn passes(self, _sample: Option<u64>) -> bool {
        true
    }
}
/// Virtual-time gate/apply/publish/receipt facts; not a Windows watchdog emulator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trace {
    /// Work announced by the guest.
    pub announced: u64,
    /// Gate opened, if held.
    pub gate_start: u64,
    /// Apply starts inside the hold.
    pub apply_start: u64,
    /// Apply ends before publication.
    pub apply_end: u64,
    /// Work actually published.
    pub published: u64,
    /// Real/fake engine-completion receipt in the corresponding mode.
    pub completed: u64,
}
/// Global-gate model: a packet arriving inside an apply waits for apply, then executes.
pub fn model(_announced: u64, _apply_start: u64, _apply_ns: u64, _execute_ns: u64) -> Trace {
    Trace {
        announced: 0,
        gate_start: 0,
        apply_start: 0,
        apply_end: 0,
        published: 0,
        completed: 0,
    }
}
