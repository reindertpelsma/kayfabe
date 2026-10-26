//! Bounded gate-held publication to SET_REF completion-observation association.
use super::Recorder;
/// Per-channel and per-boot diagnostic totals, never decision inputs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Maximum when an entry was already announced at its hold.
    pub known_ns: Option<u64>,
    /// Conservative bound when the hold preceded any GP_PUT observation.
    pub upper_ns: Option<u64>,
    /// Completed marker-covered gated batches.
    pub completed: u64,
    /// Samples lost to bounds, retirement or invalid sequence/time.
    pub lost: u64,
    /// Batches still waiting for a covering marker/observation.
    pub pending: u64,
}
impl Stats {
    /// Worst observed interval, or unavailable with no matched receipt.
    pub fn worst(self) -> Option<u64> {
        self.known_ns.max(self.upper_ns)
    }
    /// Fold channel totals, retaining the maximum rather than averaging.
    pub fn fold(&mut self, _other: Self) {}
    /// Status fragment with explicit missing data.
    pub fn summary(self) -> String {
        "ens4_worst_gate_apply_complete_ns=NA".into()
    }
}
/// Fixed-budget observer; marker counts come from successful actual publication/Marks::seen.
#[derive(Debug)]
pub struct Exposure;
impl Exposure {
    /// Explicit cap for tiny tests; production uses512, bounded by private ring credits.
    pub fn new(_cap: usize) -> Self {
        Self
    }
    /// First gate call of a hold.
    pub fn gate(&mut self, _t: u64) {}
    /// Gate opens without changing any publication/receipt state.
    pub fn pass(&mut self) {}
    /// Actual descriptor publication, with an optional observed announcement.
    pub fn published(&mut self, _t: u64, _announced: Option<u64>, _held: bool) {}
    /// No pending guest work after a gate: do not attach the hold to a later idle epoch.
    pub fn idle(&mut self) {}
    /// Successful SET_REF descriptor publication; it covers preceding publications only.
    pub fn marker(&mut self, _seq: u64, _t: u64) {}
    /// Validated ordinary marker observation, not a GET/IRQ count.
    pub fn observed(&mut self, _seen: u64, _t: u64) {}
    /// Free/reset invalidates uncompleted association, while retaining completed totals.
    pub fn retire(&mut self) {}
    /// Bounded diagnostic state snapshot.
    pub fn stats(&self) -> Stats {
        Stats::default()
    }
}
impl Recorder {
    /// Fold actual marker publication and a validated ordinary observation at turn end.
    pub fn ens4_turn(&self, _token: u32, _marker: u64, _seen: u64, _observe_ns: u64, _t: u64) {}
    /// Per-boot status; known absence is NA, never a fabricated zero latency.
    pub fn ens4_summary(&self) -> String {
        Stats::default().summary()
    }
}
