//! ★★ **The ledger of what this port has not built** — the host-side answer to *"which
//! commands is the guest asking for that nobody answers?"*
//!
//! ## Why it exists, and why it is not optional instrumentation
//!
//! Task #127 made the emulated GSP's default a **named refusal** rather than an echo
//! (`kayfabe_gsp::GspFsm::answer` carries the measurement that forced it). That is the
//! right default and it is a **quiet** one, which is the problem this module exists for.
//!
//! `[inferred]` from the guest's own source: `rpcRmApiControl_GSP` singles
//! `NV_ERR_NOT_SUPPORTED` out — with `NV_ERR_OBJECT_NOT_FOUND` — as a status to log
//! *quietly*, dropping its `GspRmControl failed: … cmd=…` line from `LEVEL_WARNING` to
//! `LEVEL_INFO` (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:11108-11120`). The envelope
//! check one layer up is `LEVEL_WARNING` at best (`:1999-2005`). Neither is `LEVEL_ERROR`,
//! and `[measured]` the bench's own trap list records that `NVreg_ResmanDebugLevel=-1`
//! yields zero extra lines on a 580.159.04 module — so turning them up is not available
//! either.
//!
//! What the guest *does* print is the `LEVEL_ERROR` at whichever caller could not continue.
//! That is exactly one rung per boot, and it is the property that makes a refusal cheap to
//! **act** on — it names the first thing the driver actually needed. It is not a property
//! that lets anyone answer *how long the list is*. This does, in one boot.
//!
//! ⊘ Stated as inference, not measurement: what a release module prints for a refused
//! control has not been observed on this branch. The first boot against it settles it, and
//! if the refusals turn out to be loud in the guest this module is still the cheaper
//! answer — it is a set, not a log to grep.
//!
//! ## ★ Recording is not answering
//!
//! [`UnservicedLedger`] is a [`CommandPolicy`] that always returns `None`. It goes **last**
//! in the chain, sees exactly the commands every earlier link declined, writes them down,
//! and declines them itself — leaving the FSM to post the refusal. A link that both
//! recorded and answered would be a policy whose diagnostics could change what the guest
//! sees.
//!
//! ## ⊘ Bounded, and deliberately keyed on the pair
//!
//! The distinct set is capped: an unbounded one is a guest-driven allocation, and a driver
//! that retries a refused control in a loop must not be able to grow it. The key is
//! `(function, cmd)` rather than `function`, because every control in the driver arrives as
//! function 76 and a ledger of *"76, 4 913 times"* answers nothing.

use core::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use kayfabe_abi::versions::DriverAbiTable;
use kayfabe_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

/// How many distinct unserviced commands are remembered.
///
/// ★ Small and fixed, like `crate::plane::UNCLAIMED_SAMPLE_MAX`. The counter says how
/// many; this says which.
pub const UNSERVICED_SAMPLE_MAX: usize = 32;

/// One command nothing answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnservicedCommand {
    /// The wire function id (`NV_VGPU_MSG_FUNCTION_*`), as sent.
    pub function: u32,
    /// The `NV2080_CTRL_*`/`NV0080_CTRL_*` command, when the function was
    /// `GSP_RM_CONTROL` and its header decoded. `None` for every other function, and for a
    /// control whose payload was too short to hold a header — which is itself a fact worth
    /// seeing rather than papering over with a zero.
    pub cmd: Option<u32>,
}

/// The shared record. Cloneable so the plane and the chain link hold the same one.
#[derive(Debug, Clone, Default)]
pub struct UnservicedLog {
    seen: Arc<Mutex<Vec<UnservicedCommand>>>,
    total: Arc<AtomicU64>,
}

impl UnservicedLog {
    /// A fresh, empty log.
    #[must_use]
    pub fn new() -> UnservicedLog {
        UnservicedLog::default()
    }

    /// How many commands went unserviced in total, including repeats and anything past
    /// [`UNSERVICED_SAMPLE_MAX`].
    #[must_use]
    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// The distinct commands remembered, in first-seen order.
    #[must_use]
    pub fn sample(&self) -> Vec<UnservicedCommand> {
        let s = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        s.clone()
    }

    /// Record one. Idempotent for the distinct set; always counted in the total.
    pub fn note(&self, entry: UnservicedCommand) {
        self.total.fetch_add(1, Ordering::Relaxed);
        let mut s = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        if s.len() < UNSERVICED_SAMPLE_MAX && !s.contains(&entry) {
            s.push(entry);
        }
    }
}

/// The terminal chain link: writes down what it was asked, and answers nothing.
#[derive(Debug, Clone)]
pub struct UnservicedLedger {
    driver: DriverAbiTable,
    log: UnservicedLog,
}

impl UnservicedLedger {
    /// Build a ledger writing into `log`.
    #[must_use]
    pub fn new(driver: DriverAbiTable, log: UnservicedLog) -> UnservicedLedger {
        UnservicedLedger { driver, log }
    }
}

impl CommandPolicy for UnservicedLedger {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        let control = if cmd.function == RpcFunction::RmControl {
            self.driver
                .decode_rpc_control(&cmd.payload)
                .ok()
                .map(|r| r.cmd)
        } else {
            None
        };
        self.log.note(UnservicedCommand {
            function: cmd.code,
            cmd: control,
        });
        // ⊘ Always `None`. See this module's docs: recording is not answering.
        None
    }
}

kayfabe_util::assert_send_sync!(UnservicedCommand, UnservicedLog, UnservicedLedger);
