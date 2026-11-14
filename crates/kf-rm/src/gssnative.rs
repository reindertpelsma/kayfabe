//! ⊘ **REMOVED** — the `KF3_GSS_NATIVE` experiment. `STATUS: SUPERSEDED, 2026-11-13` (security
//! review 1, F-06 / X-01; owner: "fix it"; `docs/design/V3_GSS_NATIVE.md`).
//!
//! The experiment carried a non-privileged GSS-legacy control (`(cmd & 0xC000) == 0x8000`, class
//! `0x2080`) from the guest to the HOST RM with the guest's parameter bytes UNCHANGED, and wrote
//! the host's reply bytes back into the guest's reply. That is a guest-authored host action
//! (owner principle A.2: host verbs are authored, never forwarded), with no per-command
//! allow-list and parameter blocks whose contents kayfabe cannot read. Nothing in the gates, the
//! CI or the docs depended on it, so the forwarding path is gone, not fenced:
//!
//! - [`enabled`] is always `false` (`KF3_GSS_NATIVE=1` is said once as ignored);
//! - [`crate::rmrpc::ObjectPolicy::with_gss_native`] drops its seat unread: no link classifies,
//!   copies or queues anything, so every such control falls through to the unserviced ledger and
//!   the FSM refuses it `NV_ERR_NOT_SUPPORTED`, exactly as with the experiment off;
//! - [`execute`] never calls its [`GssHost`]: it resolves the cell `NV_ERR_NOT_SUPPORTED`.
//! - `kf_gsp::Deferred`'s reply patch (the host's bytes into a held reply) is deleted.
//!
//! ⚠ What is left is a **compile shell** for the `kf-qemu` glue (`device.rs`, `chan.rs`,
//! `failclosed.rs`), which another agent owned when this was removed. Coordinator follow-up:
//! delete that glue, then this module and `kf_host::HostRm::raw_control_opaque`'s experiment
//! docs. Tests: `crates/kf-rm/tests/gss_native_removed.rs`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use kf_gsp::Deferred;

/// The removed environment switch (read only to say it is ignored).
pub const FLAG: &str = "KF3_GSS_NATIVE";
/// The plane's act thread name (still a service thread in `kf-qemu`'s fail-closed set).
pub const ACT_THREAD: &str = "kf3-chan-act";
/// The old experiment's parameter cap (only quoted by the dead `kf-qemu` banner).
pub const MAX_PARAMS: usize = 64 * 1024;
/// The old experiment's per-boot cap (only quoted by the dead `kf-qemu` banner).
pub const BOOT_CAP: u64 = 20_000;
/// `NV_ERR_NOT_SUPPORTED`: what [`execute`] resolves every cell with.
pub const NV_ERR_NOT_SUPPORTED: u32 = 0x56;

static SAID_IGNORED: AtomicBool = AtomicBool::new(false);

/// Always `false`: the experiment is removed. `KF3_GSS_NATIVE=1` is said once as ignored.
#[must_use]
pub fn enabled() -> bool {
    if std::env::var(FLAG).as_deref() == Ok("1") && !SAID_IGNORED.swap(true, Ordering::Relaxed) {
        eprintln!(
            "kf3: {FLAG}=1 IGNORED: the experiment was removed (security review 1, F-06); GSS-legacy \
             controls are refused as unserviced"
        );
    }
    false
}

/// A request shape kept for the `kf-qemu` glue. Nothing in this crate builds one any more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GssRequest {
    /// The guest's client.
    pub client: u32,
    /// The guest's object.
    pub object: u32,
    /// The command.
    pub cmd: u32,
    /// Offset of `params[]` in the reply payload.
    pub params_at: usize,
    /// The guest's params.
    pub params: Vec<u8>,
}

/// What the plane says to a request (kept for the `kf-qemu` glue).
#[derive(Debug)]
pub enum GssAnswer {
    /// Queued as an act.
    Deferred(Deferred),
    /// Not queued.
    Refused {
        /// Why.
        why: String,
    },
}

/// Where requests would go. Never called: [`crate::rmrpc::ObjectPolicy::with_gss_native`] drops it.
pub type GssSink = Arc<dyn Fn(GssRequest) -> GssAnswer + Send + Sync>;

/// The host seam (kept for the `kf-qemu` glue). [`execute`] never calls it.
pub trait GssHost {
    /// One control on the host subdevice.
    ///
    /// # Errors
    /// The transport's refusal, as text.
    fn control(&self, cmd: u32, params: &mut [u8]) -> Result<u32, String>;
}

/// Counters (atomics only).
#[derive(Default)]
pub struct Stats {
    /// Requests that reached [`execute`] and were refused (must stay 0: nothing builds one).
    pub refused: AtomicU64,
}

impl core::fmt::Debug for Stats {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.status())
    }
}

impl Stats {
    /// Fresh counters.
    #[must_use]
    pub fn new() -> Stats {
        Stats::default()
    }

    /// Device reset: nothing per boot is left to clear.
    pub fn new_boot(&self) {}

    /// The status-line segment.
    #[must_use]
    pub fn status(&self) -> String {
        format!(
            "gss[removed refused={}]",
            self.refused.load(Ordering::Relaxed)
        )
    }
}

/// The seat (kept for the `kf-qemu` glue; dropped unread by the object seat).
#[derive(Clone)]
pub struct GssSeat {
    /// The plane's inbox (never called).
    pub sink: GssSink,
    /// The counters.
    pub stats: Arc<Stats>,
}

/// Refuse: never calls `host`, resolves `d` with [`NV_ERR_NOT_SUPPORTED`] (never `NV_OK`).
pub fn execute(stats: &Stats, host: &dyn GssHost, req: GssRequest, d: &Deferred) {
    let _ = (host, req);
    stats.refused.fetch_add(1, Ordering::Relaxed);
    d.resolve(NV_ERR_NOT_SUPPORTED);
}
