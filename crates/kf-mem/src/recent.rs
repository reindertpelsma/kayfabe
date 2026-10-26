//! ENS5 bounded first-and-last diagnostic retention and slow-apply accounting.
use std::sync::{Mutex, OnceLock};
/// Retain four initial records and twelve recent records. Capacity is host-chosen.
pub fn retain<T>(rows: &mut Vec<T>, item: T) {
    if rows.len() >= 16 {
        rows.remove(4);
    }
    rows.push(item);
}
/// Fixed diagnostic categories, never a guest-controlled map key.
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    /// Refused VA work.
    Refusal,
    /// Split completed with absent leaves.
    Unsettled,
    /// Sparse refresh retry error.
    SparseRetry,
    /// VA history automatic capture.
    Vahist,
}
#[derive(Default)]
struct Log {
    rows: [Vec<String>; 4],
    counts: [u64; 4],
}
fn log() -> &'static Mutex<Log> {
    static LOG: OnceLock<Mutex<Log>> = OnceLock::new();
    LOG.get_or_init(|| Mutex::new(Log::default()))
}
/// Save bounded ASCII text; return a saturating category count for first-few emission.
pub fn note(kind: Kind, text: &str) -> u64 {
    let mut state = log()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let i = kind as usize;
    let text: String = text
        .chars()
        .filter(|c| c.is_ascii_graphic() || *c == ' ')
        .take(320)
        .collect();
    if let Some(rows) = state.rows.get_mut(i) {
        retain(rows, text);
    }
    if let Some(n) = state.counts.get_mut(i) {
        *n = n.saturating_add(1);
        *n
    } else {
        0
    }
}
/// At most64 records of320 bytes, copied without hardware or mapping reads.
pub fn snapshot() -> Vec<String> {
    let state = log()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.rows.iter().flatten().cloned().collect()
}
/// Counts remain visible after immediate log-print limits fill.
pub fn summary() -> String {
    let state = log()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    format!("ens5_recent_counts={:?}", state.counts)
}
/// Default-on, fixed CPU accounting; returned labels are diagnostic data only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Slow {
    /// Maximum observed apply duration.
    pub apply_ns_max: u64,
    /// Applies at or above200ms.
    pub slow: u64,
    /// Counts by dominant pipeline timer: admit, host RM, walk, invalidate.
    pub by_cost: [u64; 4],
    /// Latest dominant pipeline timer; timers can overlap and are never added.
    pub last_cost: &'static str,
}
impl Slow {
    /// Return the first slow apply's largest measured timer label exactly once.
    pub fn observe(&mut self, apply: u64, costs: [u64; 4]) -> Option<&'static str> {
        self.apply_ns_max = self.apply_ns_max.max(apply);
        if apply < 200_000_000 {
            return None;
        }
        self.slow = self.slow.saturating_add(1);
        let (i, cost) = costs
            .iter()
            .enumerate()
            .max_by_key(|(_, n)| **n)
            .unwrap_or((0, &0));
        let label = if *cost == 0 {
            "unattributed"
        } else {
            if let Some(n) = self.by_cost.get_mut(i) {
                *n = n.saturating_add(1);
            }
            ["admit", "host_rm", "walk", "invalidate"]
                .get(i)
                .copied()
                .unwrap_or("unattributed")
        };
        self.last_cost = label;
        (self.slow == 1).then_some(label)
    }
}
