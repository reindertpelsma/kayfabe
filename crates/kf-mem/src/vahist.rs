//! ★ MEASUREMENT ONLY — `KF3_DIAG_VAHIST=1` (EXPECTED RED stub: the API only; the tests in
//! `tests/vahist.rs` fail until the implementation lands).

use crate::apply::DiffRun;
use kf_cuda::abi::KfFormat;
use std::path::Path;
use std::time::Instant;

/// Last applied runs kept per VA space (all ops, coalesced).
pub const LAST_RUNS: usize = 64;
/// Absence events (UNMAP / REFUSED / WITHHELD, coalesced) kept per VA space.
pub const ABSENCE_EVENTS: usize = 2048;
/// Live VA spaces tracked at once.
pub const MAX_SPACES: usize = 256;
/// Retired VA spaces kept for the reuse question.
pub const RETIRED_KEPT: usize = 8;
/// Coalesced row extents snapshotted per retired space.
pub const RETIRE_EXTENTS: usize = 1024;
/// Channel lifetime records kept in memory.
pub const CHAN_RECORDS: usize = 512;
/// `CHAN_LIFE` lines printed per process (births and frees together).
pub const CHAN_LINES_MAX: usize = 4096;
/// Lookups answered per process (trigger file).
pub const MANUAL_DUMPS: u32 = 4;
/// Automatic dumps per process (one per space, on a refusal).
pub const AUTO_DUMPS: u32 = 16;
/// Bytes of a trigger looked at.
pub const TRIGGER_BYTES: usize = 512;
/// Minimum gap between two looks at the trigger file.
pub const TRIGGER_POLL_MS: u64 = 500;
/// The longest line printed.
pub const LINE_MAX: usize = 512;

/// `true` only for exactly `"1"`.
#[must_use]
pub fn flag_on(_v: Option<&str>) -> bool {
    false
}

/// A lookup request: the host's fault VA and, when given, its channel id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trigger {
    /// The faulting VA.
    pub va: u64,
    /// The host channel id the Xid named.
    pub chid: Option<u32>,
}

/// Parse a trigger.
#[must_use]
pub fn parse_trigger(_text: &[u8]) -> Option<Trigger> {
    None
}

/// A channel birth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChanBirth {
    /// Guest client.
    pub client: u32,
    /// Guest handle.
    pub handle: u32,
    /// Guest token index.
    pub guest_token: u32,
    /// Host work-submit token.
    pub host_token: u32,
    /// Guest process id, as declared.
    pub pid: Option<u32>,
    /// Engine.
    pub engine: u32,
    /// GP entries.
    pub gp_entries: u32,
    /// GPFIFO VA.
    pub gpfifo_va: u64,
    /// The VA space key.
    pub vas: u64,
}

/// What a lookup may ask of the live VA plane.
pub trait SpaceView {
    /// Keys of live spaces.
    fn live_keys(&self) -> Vec<u64>;
    /// Our row covering `va` in `key`: `(start, len, ram, offset)`.
    fn row_at(&self, key: u64, va: u64) -> Option<(u64, u64, bool, u64)>;
    /// The guest's PTE for `va` in `key`, printed.
    fn guest_pte(&self, key: u64, va: u64) -> Option<String>;
}

/// Ring sizes of one tracked space (tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpaceStats {
    /// Entries in the last-runs ring.
    pub last_len: usize,
    /// Entries in the absence ring.
    pub absent_len: usize,
    /// Absence entries evicted.
    pub absent_evicted: u64,
}

/// The recorder.
#[derive(Debug, Default)]
pub struct Recorder;

impl Recorder {
    /// A fresh recorder.
    #[must_use]
    pub fn new() -> Recorder {
        Recorder
    }
    /// A space got its host space.
    pub fn note_space(&self, _key: u64, _host_space: u64, _recycled: bool) {}
    /// One apply.
    pub fn record_apply(
        &self,
        _key: u64,
        _walk: u64,
        _runs: &[DiffRun],
        _codes: &[u8],
        _withholds_priv: bool,
        _walk_refused_mask: u32,
    ) -> bool {
        false
    }
    /// Ring sizes.
    #[must_use]
    pub fn space_stats(&self, _key: u64) -> Option<SpaceStats> {
        None
    }
    /// Auto dump.
    #[must_use]
    pub fn auto_dump(&self, _key: u64, _why: &str) -> Vec<String> {
        Vec::new()
    }
    /// Retire.
    pub fn retire(&self, _key: u64, _extents: Vec<(u64, u64)>, _live: u64) {}
    /// Birth line.
    #[must_use]
    pub fn chan_birth(&self, _b: ChanBirth) -> Option<String> {
        None
    }
    /// Class.
    pub fn chan_class(&self, _client: u32, _handle: u32, _class: u32) {}
    /// Free line.
    #[must_use]
    pub fn chan_free(&self, _client: u32, _handle: u32, _ok: bool) -> Option<String> {
        None
    }
    /// Trigger file.
    #[must_use]
    pub fn poll_trigger(&self, _path: &Path, _now: Instant) -> Option<Result<Trigger, String>> {
        None
    }
    /// Lookup.
    #[must_use]
    pub fn lookup(&self, _t: &Trigger, _view: &dyn SpaceView) -> Vec<String> {
        Vec::new()
    }
}

/// Probe the guest's PTE.
#[must_use]
pub fn probe_pte(
    _fmt: &KfFormat,
    _pdb: u64,
    _va: u64,
    _read8: &mut dyn FnMut(u64) -> Option<u64>,
) -> String {
    String::new()
}
