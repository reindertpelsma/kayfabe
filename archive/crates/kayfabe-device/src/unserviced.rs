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
//! ## ★★★ What it has actually answered — five boots of one rung each, and then `t134a`
//!
//! `[measured]` on the bench, a stock 580.159.04 guest driven with `nvidia-smi`, one boot
//! per revision. The table's point was always the **right-hand column**: the question
//! *"does refusing by default surface everything at once?"* had an answer, and it was no —
//! **for five boots**. The sixth is the counterexample, and it is the more useful row.
//!
//! | run | revision | `commands` | distinct unserviced | what the guest said |
//! |---|---|---|---|---|
//! | `t127a` | `f870288` | 3 | **1** — `fn 1` | `kgspInitRm_IMPL: SET_GUEST_SYSTEM_INFO failed: 0x56` |
//! | `t127b` | `0db7c61` | 5 | **1** — `fn 228` | `kgspInitGspTraceCrashBuffer … @ kernel_gsp.c:4239` |
//! | `t127c` | `110c857` | 6 | **1** — `fn 76 cmd 0x20800a36` | `_gpuInitChipInfo … @ gpu.c:886, 2124` |
//! | `t132a` | `f83ce31` | 7 | **1** — `fn 76 cmd 0x20800a41` | `gpuConstructUserRegisterAccessMap … @ gpu_register_access_map.c:244, gpu.c:2125` |
//! | `t133a` | `c88f803` | 8 | **1** — `fn 76 cmd 0x208001b0` | `gpuBuildGenericKernelFalconList … @ gpu.c:5368, 2126` |
//! | `t134a` | `1c79474` | **27** | **6** — `0x20800a87`, `0x20800a40`, `0x20800a1c`, `0x20800a4b`, `0x20800af3`, `0x20800aac` | `gpuConstructDeviceInfoTable_HAL … @ kernel_fifo.c:2208`, then a guest-kernel `Oops` |
//! | `t135a` | `c84ef52` | **28** | **6** — `0x20800a87`, `0x20800a40`, `0x20800a4b`, `0x20800af3`, `0x20800aac`, **`0x20802a08`** | `gpuConstructDeviceInfoTable_HAL … @ kernel_fifo.c:2208` — now the guest's **first** line, and a *different* `Oops` |
//!
//! ★★ The first five walk `gpuPreInit` **one adjacent line at a time** — `:2124`, `:2125`,
//! `:2126` — so the ledger and the guest agreed not merely on *which* control but on the
//! exact statement that consumed it.
//!
//! ## ★★★ `t134a`: the one-rung property was a property of `gpuPreInit`, not of refusing
//!
//! `[measured]` at `1c79474`, the boot that served the constructed-falcon inventory left
//! `gpuPreInit` altogether. `commands` went 8 → **27** and distinct unserviced went 1 → **6**
//! in a single step. Nothing about the default changed; what changed is *which loop the
//! guest was in*.
//!
//! `gpuPreInit`'s statements are a chain of `NV_ASSERT_OK_OR_GOTO`s: the first refusal ends
//! the function, so exactly one control can be reached per boot and the ledger reads like a
//! ladder. `gpuStatePreInit_IMPL` is **not** that shape — it iterates the engine list, logs
//! *"disallowing NV_ERR_NOT_SUPPORTED PreInit removal of untracked engine"*
//! (`ogkm-580: gpu.c:2204`) and **carries on**. So one boot now surfaces every control the
//! whole engine sweep wants, which is why `kfifoConstructEngineList_HAL` /
//! `gpuConstructDeviceInfoTable_HAL` appear a dozen times over.
//!
//! ⊘ **So the `commands` column never was a queue length, and now it visibly is not.** It
//! went 3, 5, 6, 7, 8 — one per rung — which invited reading it as *"the guest asks for one
//! more thing each time"*. It never meant that; it meant *"exactly one new control per boot
//! has been reached at all"*, and `t134a` shows what the column does the moment that stops
//! holding.
//!
//! ⚠ **`t134a` ended in a guest-kernel NULL dereference, and the chain is worth naming.**
//! `kmemsysInitStaticConfig_HAL` was refused (`ogkm-580: kern_mem_sys.c:122`), so
//! `gpuStatePreInit` declined to remove `KernelMemorySystem` and continued with it
//! unconstructed; `kern_mem_sys.c:364`'s `pKernelMemorySystem != NULL` is an
//! `nvAssertFailedNoLog` that **does not return**; and
//! `memmgrGetBlackListPagesForHeap_GM107` then dereferenced it under
//! `heapInit_IMPL ← memmgrCreateHeap_IMPL ← memmgrStateInitLocked_IMPL ← gpuStateInit_IMPL`.
//! `nvidia-smi` therefore **hangs** rather than failing, which is a different observable
//! from every earlier rung and must not be read as progress stalling. The refusal is
//! correct; RM's handling of it past `gpuPreInit` is not fail-safe.
//!
//! ★ The guest and the ledger name the **same** thing every time, from opposite sides, and
//! `0x56` is this port's own `NV_ERR_NOT_SUPPORTED` arriving verbatim — so the envelope
//! really does reach RM. ★★ The third row is why the pair key matters: `fn 76` alone would
//! have said *"a control"*, and `0x20800a36` says
//! `NV2080_CTRL_CMD_INTERNAL_GPU_GET_CHIP_INFO` — the next rung, named without a boot spent
//! finding out. The fifth row names
//! `NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO` (`ogkm-580: ctrl2080gpu.h:4472`) the
//! same way. ★★★ The sixth row names **six**, and `0x20800a40` is the one the guest's own
//! first `LEVEL_ERROR` agrees with.
//!
//! ## ★★★ `t135a`: what the sweep's counters do when a rung IS cleared
//!
//! `[measured]` at `c84ef52`, a stock 580.159.04 guest, `nvidia-smi`, one boot — the first
//! rung served under the sweep rather than under `gpuPreInit`
//! (`docs/design/preinit_sweep_loop.md`). Read the row above against `t134a`'s:
//!
//! - `commands` 27 → **28**. One more control was *reached*.
//! - distinct unserviced **6 → 6**, and the set is not the same set: `0x20800a1c` left it
//!   (served) and **`0x20802a08`** entered it — `NV2080_CTRL_CMD_CE_GET_FAULT_METHOD_BUFFER_SIZE`
//!   (`ogkm-580: src/nvidia/src/kernel/gpu/ce/kernel_ce.c:843`), a control nothing had ever
//!   got far enough to ask.
//!
//! ⊘ **So the distinct count is not monotone either, and it is not a progress bar.** The
//! design doc's §4.4 says `commands` rising means *more engines reached* and distinct
//! falling means *more engines survived*; `t135a` is the case it did not name — progress
//! that holds the count flat because clearing one rung **reveals** the next. The set churns.
//! Watch the membership, never the cardinality.
//!
//! ★★ **The guest agrees, from the other side, and more sharply than the count does.**
//! `t134a`'s first `LEVEL_ERROR` was `gpuConstructDeviceInfoTable_HAL @ kernel_fifo.c:2208`
//! with `kmemsysInitStaticConfig_HAL @ kern_mem_sys.c:122` and
//! `"disallowing … (KernelMemorySystem:0)"` alongside it. At `c84ef52` **both of those lines
//! are gone** and `:2208` is the first and only caller that complains. The engine survived.
//!
//! ⚠ **The `Oops` is not gone — it moved, and saying it is gone would be wrong.**
//!
//! ```text
//! t134a  BUG: … address: 0000000000000268
//!        RIP: memmgrGetBlackListPagesForHeap_GM107+0x23/0x140
//!          heapInit_IMPL ← memmgrCreateHeap_IMPL ← memmgrStateInitLocked_IMPL
//!
//! t135a  BUG: … address: 0000000000000201
//!        RIP: memmgrCalcReservedFbSpaceHal_GM107+0x40e/0x7b0
//!          memmgrCalcReservedFbSpace_IMPL ← memmgrRegionSetupForPma_IMPL
//!          ← heapInitInternal_IMPL ← memmgrCreateHeap_IMPL ← memmgrStateInitLocked_IMPL
//! ```
//!
//! ★ Both are inside `memmgrCreateHeap_IMPL`, and that is the evidence rather than a
//! coincidence: `heapInit_IMPL`'s **first** statement is the blacklist walk that faulted at
//! `t134a` (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/objheap.c:41`). At `t135a` it
//! returns and the fault is in `heapInitInternal_IMPL`, further down the same call. The
//! served reply moved the boot past the exact statement it was supposed to.
//!
//! ★★★ **And the new one names the next rung, which is already written.**
//! `memmgrCalcReservedFbSpace` sizes the channel and copy-engine reservations, and this boot
//! failed `gpuConstructDeviceInfoTable_HAL` (`0x20800a40`, 20 times), then
//! `kfifoConstructEngineList_HAL @ kernel_fifo_gm107.c:713`, then
//! `NV2080_CTRL_CMD_CE_GET_FAULT_METHOD_BUFFER_SIZE @ kernel_ce.c:843` — so `KernelFifo` has
//! no engine list and `KernelCE` no buffer size when the heap asks them for one.
//! `[inferred]`, not measured: the faulting field at `+0x201` has not been identified, only
//! its two starved suppliers.
//!
//! ⊘ `0x20800a40` is therefore triaged as a **second** `AmputationUnsurvivable` in
//! [`crate::sweep::SWEEP_TRIAGE`], and by a worse mechanism than the first: it is asked from
//! `gpuStateInit`, whose loop maps `NV_ERR_NOT_SUPPORTED` to `NV_OK` and does **not** remove
//! the engine (`ogkm-580: gpu.c:2286-2287`). PreInit at least NULLs the pointer, so a NULL
//! check can catch it; StateInit leaves a constructed-but-uninitialised object that every
//! NULL check passes.
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
/// ★ Small and fixed, like `crate::plane::UNCLAIMED_SAMPLE_MAX`. [`UnservicedLog::distinct`]
/// says how many; this says which.
///
/// ⊘⊘ **32 was not enough, and the shortfall was SILENT — this is the cap that produced a
/// wrong root cause.** `[measured 2026-08-09, boot `gt1431_ff7a0ea`,
/// `/workspace/bench/run_gt1431_ff7a0ea_qemu.log`]` that boot's summary read
/// `67 UNSERVICED …, 32 distinct` and printed exactly 32 rows — the set was **saturated**,
/// and every command first seen after the thirty-second was dropped with no line anywhere.
/// `execution_plane_increments.md` §14.31 read `0x20801303`'s absence from that list as
/// *"the command never reaches the emulated GSP; the guest kernel refuses it from its own
/// state"*, and built a rung on it. The RPC does go out.
///
/// ⇒ ★★ **An absence from a saturated list is not evidence of absence** — the third
/// distinct way this project has been bitten by a check that cannot fail
/// (`pgrep_comm_truncation_trap`, `gate_read_through_grep_cannot_fail`). Raised to 64, and
/// — far more important than the number — [`UnservicedLog::distinct`] now counts the
/// **true** distinct total so a saturated list says so out loud. See
/// [`kayfabe_abi::fbinfo`] for the whole measurement.
pub const UNSERVICED_SAMPLE_MAX: usize = 64;

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
    distinct: Arc<AtomicU64>,
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

    /// ★★★ **How many distinct commands were seen — the truth, past
    /// [`UNSERVICED_SAMPLE_MAX`].**
    ///
    /// ⊘ This exists because its absence produced a wrong root cause. `UNSERVICED_SLOTS`'s
    /// own doc asserted *"`unserviced_len` reports the truth even when it exceeds this, so
    /// a full array is never mistaken for a complete list"* — and `unserviced_len` was
    /// [`Self::sample`]`.len()`, which is **clamped by construction and can never exceed
    /// it**. A load-bearing rationale that was false, in the shape
    /// `safety_comment_is_not_the_check` names.
    /// `[measured 2026-08-09, boot `gt1431_ff7a0ea`, its own
    /// `/workspace/bench/run_gt1431_ff7a0ea_qemu.log`]` that boot printed `32 distinct` out
    /// of a saturated 32-slot list, and `execution_plane_increments.md` §14.31 built a rung
    /// on the resulting miss. `tests/unserviced_ledger.rs`'s
    /// `a_saturated_sample_says_so_rather_than_reading_as_complete` is the test that fails
    /// if this stops being true.
    /// [`crate::census::ControlCensusLog`] had kept a separate distinct counter all along,
    /// which is why the served list's count was truthful and this one's was not.
    #[must_use]
    pub fn distinct(&self) -> u64 {
        self.distinct.load(Ordering::Relaxed)
    }

    /// Whether the remembered set is **shorter than the truth** — i.e. the sample is
    /// saturated and rows have been dropped.
    ///
    /// ★ A named question rather than a comparison a reader has to think of, because the
    /// whole failure was nobody thinking of it.
    #[must_use]
    pub fn truncated(&self) -> bool {
        self.distinct() > UNSERVICED_SAMPLE_MAX as u64
    }

    /// The distinct commands remembered, in first-seen order.
    #[must_use]
    pub fn sample(&self) -> Vec<UnservicedCommand> {
        let s = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        s.clone()
    }

    /// Record one. Idempotent for the distinct set; always counted in the total.
    ///
    /// ⊘ The distinct counter is incremented **under the same lock** as the membership
    /// test and **before** the capacity test, so it counts every first-seen command whether
    /// or not there was a slot for it. Counting after the `push` is what made the old
    /// length agree with the sample instead of with reality.
    pub fn note(&self, entry: UnservicedCommand) {
        self.total.fetch_add(1, Ordering::Relaxed);
        let mut s = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        if s.contains(&entry) {
            return;
        }
        self.distinct.fetch_add(1, Ordering::Relaxed);
        if s.len() < UNSERVICED_SAMPLE_MAX {
            s.push(entry);
        }
    }
}

/// The terminal chain link: writes down what it was asked, and answers nothing.
///
/// ★ **Its sticky-answer argument is the same one sentence, and it is worth stating because
/// this link sees every fn-76 control the port declines.** The guest's control cache is
/// populated only from a reply the RPC layer accepted (`ogkm-580:
/// src/nvidia/src/kernel/vgpu/rpc.c:11093-11104`, `ogkm-610: :10898-10909`), and `respond`
/// here is `None` unconditionally — recording is not answering, so there is no reply for
/// either branch to persist. See [`crate::sticky`]; `tests/tests/sticky_answer.rs` executes
/// the claim against this type rather than trusting this paragraph.
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
