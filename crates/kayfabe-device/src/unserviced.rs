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
//! ## ★★★ What it has actually answered — five boots, five rungs, one entry each
//!
//! `[measured]` on the bench, a stock 580.159.04 guest driven with `nvidia-smi`, one boot
//! per revision. The whole point of the table is the **right-hand column**: the question
//! *"does refusing by default surface everything at once?"* has an answer, and it is no.
//!
//! | run | revision | `commands` | distinct unserviced | what the guest said |
//! |---|---|---|---|---|
//! | `t127a` | `f870288` | 3 | **1** — `fn 1` | `kgspInitRm_IMPL: SET_GUEST_SYSTEM_INFO failed: 0x56` |
//! | `t127b` | `0db7c61` | 5 | **1** — `fn 228` | `kgspInitGspTraceCrashBuffer … @ kernel_gsp.c:4239` |
//! | `t127c` | `110c857` | 6 | **1** — `fn 76 cmd 0x20800a36` | `_gpuInitChipInfo … @ gpu.c:886, 2124` |
//! | `t132a` | `f83ce31` | 7 | **1** — `fn 76 cmd 0x20800a41` | `gpuConstructUserRegisterAccessMap … @ gpu_register_access_map.c:244, gpu.c:2125` |
//! | `t133a` | `c88f803` | 8 | **1** — `fn 76 cmd 0x208001b0` | `gpuBuildGenericKernelFalconList … @ gpu.c:5368, 2126` |
//!
//! ★★ Five for five, and the last three walk `gpuPreInit` **one adjacent line at a time** —
//! `:2124`, `:2125`, `:2126` — so the ledger and the guest agree not merely on *which*
//! control but on the exact statement that consumed it. `[measured]` run `t132a` answered
//! `0x20800a36` from the chip row and the boot moved one line; `[measured]` run `t133a`
//! answered `0x20800a41` from the chip row and it moved one more.
//!
//! ⊘ **The `commands` column is not a queue length and does not predict the next rung.** It
//! goes 3, 5, 6, 7, 8 — one per rung — which invites reading it as *"the guest asks for one
//! more thing each time"*. It is not: it is *"exactly one new control per boot has been
//! reached at all"*, which is the same statement as the right-hand column and adds nothing
//! to it. A single rung whose caller issues four controls would move it by four.
//!
//! ★ The guest and the ledger name the **same** thing every time, from opposite sides, and
//! `0x56` is this port's own `NV_ERR_NOT_SUPPORTED` arriving verbatim — so the envelope
//! really does reach RM. ★★ The third row is why the pair key matters: `fn 76` alone would
//! have said *"a control"*, and `0x20800a36` says
//! `NV2080_CTRL_CMD_INTERNAL_GPU_GET_CHIP_INFO` — the next rung, named without a boot spent
//! finding out. The fifth row names
//! `NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO` (`ogkm-580: ctrl2080gpu.h:4472`) the
//! same way.
//!
//! ⊘ `t127c`'s counters span **two** `nvidia-smi` attempts in one QEMU life; the second
//! stopped at `_kgspBootGspRm: unexpected WPR2 already up` before sending anything, which
//! is the known one-clean-run-per-boot rule and not a new finding.
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
