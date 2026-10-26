//! One bounded, host-triggered first-153 snapshot; no guest reads or decisions.

use super::*;
use std::path::Path;

/// Bounded trigger input.
pub const TRIGGER_BYTES: usize = 512;
/// Apply history slots.
pub const APPLY_N: usize = 1024;
/// Explicit runs retained per apply, with omitted count.
pub const RUNS_N: usize = 8;
/// Dump lines, each capped at the existing LINE_MAX.
pub const LINES_MAX: usize = 12288;

/// Optional host-selected chid/VA-space; absent selectors mean all tracked channels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    /// Host channel id, low16 bits of the host token.
    pub chid: Option<u32>,
    /// Guest client<<32 | VA-space handle.
    pub space: Option<u64>,
}

/// Parse only `event=153 [chid=0x...] [space=0x...]`, no fake Xid or VA required.
pub fn parse(_text: &[u8]) -> Option<Selection> {
    None
}

/// Apply phases, on the same VA-manager thread as the host timing buckets.
#[derive(Clone, Copy, Debug, Default)]
pub struct Timing {
    /// Apply wall time only.
    pub wall_ns: u64,
    /// Submit-to-collect wall time, including GPU and scheduling (not additive).
    pub walk_ns: u64,
    /// GPU-reported walk duration, separate from apply.
    pub gpu_us: u64,
    /// Same-thread RM/admission interval counters.
    pub costs: kf_host::diag::Snapshot,
}

/// Trusted-ring progress, copied from normal publication/observation, never polled for this dump.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    /// Actual successful private PUT publication time.
    pub publish_ns: u64,
    /// Absolute private descriptor publication sequence.
    pub put: u64,
    /// Last marker issued with that publication.
    pub marker: u64,
    /// Last ordinary hardware-word observation.
    pub observe_ns: u64,
    /// Raw private GET.
    pub get: u32,
    /// Raw private REFERENCE (tagged marker word).
    pub reference: u32,
}

/// One-shot poller: invalid/missing input never consumes the successful snapshot.
#[derive(Default)]
pub struct Poller;

impl Poller {
    /// Bounded read of a regular host file; no guest/vCPU involvement.
    pub fn poll(&mut self, _path: &Path, _now: Instant) -> Option<Selection> {
        None
    }
}

impl Recorder {
    /// Begin an apply before any map/unmap/RM operation.
    pub fn ens3_begin(&self, _space: u64, _walk: u64, _runs: &[DiffRun]) {}
    /// Complete its timing; preserves the pre-apply timestamp.
    pub fn ens3_end(&self, _space: u64, _walk: u64, _timing: Timing) {}
    /// Copy the real relay's successful publication or hardware-word observation.
    pub fn ens3_progress(&self, _token: u32, _progress: Progress) {}
    /// No-Xid snapshot, selection optional, independently bounded from Xid/shutdown dumps.
    pub fn snapshot_153(&self, _selection: Selection, _t: u64) -> Vec<String> {
        Vec::new()
    }
}
