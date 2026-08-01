//! ★★★ **The engine sweep's triage table — what this port has decided about each control
//! the guest asks for *after* `gpuPreInit`, and which of those decisions it is not allowed
//! to get wrong.**
//!
//! ## Why a table, and why it is not documentation
//!
//! Up to `gpuBuildGenericKernelFalconList` the guest's controls arrive from `gpuPreInit`, a
//! chain of `NV_ASSERT_OK_OR_RETURN`s (`ogkm-580: src/nvidia/src/kernel/gpu/gpu.c:2121-2126`).
//! A refusal there ends the function at a named statement and the boot stops. Refusing is
//! safe, loud, and attributable.
//!
//! Past it the guest is in `gpuStatePreInit_IMPL`'s **engine sweep**
//! (`ogkm-580: gpu.c:2152-2219`), and `NV_ERR_NOT_SUPPORTED` — this port's default answer to
//! anything it does not serve (`kayfabe_gsp::GspFsm::answer`) — stops meaning *"refused"*:
//!
//! ```c
//! if (rmStatus == NV_ERR_NOT_SUPPORTED)
//! {
//!     switch (curEngDescriptor) { /* three whitelisted engines … */ }
//!     gpuDestroyMissingEngine(pGpu, pEngstate);       // :2208 — unconditional
//!     rmStatus = gpuDeleteEngineOnPreInit(pGpu, curEngDescriptor);
//! }
//! else if (rmStatus != NV_OK) { break; }
//! ```
//!
//! It means *"this engine is absent from this chip — destroy it and carry on"*. So a
//! control this port has not built is not a refusal the guest reports; it is a **silent
//! amputation of whichever engine asked for it**, and the damage surfaces wherever RM next
//! dereferences the pointer it NULLed.
//!
//! ⚠ `[measured]` run `t134a`, a stock 580.159.04 guest at `1c79474`: `KernelMemorySystem`
//! was amputated at `ogkm-580: kern_mem_sys.c:122` and the guest died in
//! `memmgrGetBlackListPagesForHeap_GM107`, a different subsystem, under `gpuStateInit`.
//! `nvidia-smi` hung rather than failing. Nothing in this port's tests could have said that
//! was coming, because "we do not serve `0x20800a1c`" was not a statement anything read —
//! it was the *absence* of a `WantedTable` variant.
//!
//! ★★★ **This table makes that absence into a statement, and the statement into a gate.**
//! An entry classified [`SweepDisposition::AmputationUnsurvivable`] or
//! [`SweepDisposition::RefusalFailsOpen`] that is not in
//! [`crate::inittables::WantedTable::ALL`] is a compile-and-test failure, not a boot-time
//! `Oops`. `tests/sweep_triage.rs` is where it fails.
//!
//! ## ★★★ The universe is the MEASURED prefix, not a list somebody kept up to date
//!
//! Every row below is a control the C oracle's own boot is **observed** to issue, read out
//! of `traces/mode2_c_reference/cap1b_coldboot_hermetic_d6.rec` inside the replay's closure
//! limit (txn 1028 / `rpc.sequence` 51). `[measured]`, and the run is the **C artifact's
//! rather than this port's** — `cargo run -p kayfabe-crec --example cap1b_report`, whose
//! `commands decoded` block names each `fn 76` command's control id and sequence.
//!
//! ★★ And that is a gate rather than a convention:
//! `crates/kayfabe-crec/tests/cap1b_differential.rs::every_control_the_oracle_asks_is_either_served_or_triaged`
//! derives the universe from the capture and demands that each control be in
//! [`crate::inittables::WantedTable::ALL`] or in [`SWEEP_TRIAGE`]. ⊘ An untriaged control
//! reached from the sweep is exactly `t134a`'s defect, and it is now a red test rather than
//! a boot nobody has spent.
//!
//! ## ⊘ What this table is NOT
//!
//! It is not a list of controls to implement, and it must never become one. Most of its
//! entries are deliberately **refused** — the sweep's amputation is the *correct* behaviour
//! for a chip that genuinely lacks the engine, and RM has its own vocabulary for exactly
//! that. Padding this table with things to serve would invert its purpose.
//!
//! It is also not the engine order. That is `gpuChildOrderList_GM200`
//! (`ogkm-580: src/nvidia/src/kernel/gpu/arch/maxwell/kern_gpu_gm107.c:605`), it is a fact
//! about the guest and not about us, and `docs/design/preinit_sweep_loop.md` §4.1 carries
//! the reading. This table carries only *our decision* per control.

use crate::inittables::WantedTable;

/// What `gpuStatePreInit_IMPL`'s sweep — or, past it, `gpuStateInit_IMPL`'s and
/// `gpuStatePostLoad`'s looser loops — does with a refusal of one control, and whether this
/// port is willing to let it.
///
/// ★★ **Five outcomes, not three.** `docs/design/preinit_sweep_loop.md` §4.2 named three
/// (correct / wrong / unsurvivable); pre-flighting the whole measured prefix produced two
/// more that the three could not express, and collapsing them would have meant writing down
/// a consequence that is not the one the source says. The classes are distinguished by
/// **what the guest ends up in**, not by how bad it sounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweepDisposition {
    /// ★ **Amputation is correct — refuse, deliberately.**
    ///
    /// The chip genuinely lacks the engine, and `NV_ERR_NOT_SUPPORTED` is RM's own way of
    /// being told so. An entry here must carry, in [`SweepControl::why`], either the
    /// sweep's sanctioned-removal arm that handles it (`ogkm-580: gpu.c:2178-2198` lists
    /// the three: `ENG_KERNEL_DISPLAY`, `ENG_INFOROM`, `ENG_HDACODEC`) or the caller's own
    /// tolerance of the status.
    AmputationIntended,
    /// ★★★ **Amputation is unsurvivable — this control MUST be served.**
    ///
    /// Something downstream dereferences the engine pointer, or a pointer the failed path
    /// freed, with no `NULL` check — so refusing trades a named refusal for a guest-kernel
    /// fault attributed to the wrong subsystem. `tests/sweep_triage.rs` refuses to let an
    /// entry in this class be absent from [`WantedTable::ALL`].
    AmputationUnsurvivable,
    /// ★★ **The refusal is invisible AND the state it leaves is wrong — serve.**
    ///
    /// The §6 shape: RM pre-zeroes or ignores the destination, so nothing distinguishes a
    /// refusal from an answer, *and* the zeros are not what a real GSP would have said.
    /// Also a must-serve class, for a different reason to
    /// [`Self::AmputationUnsurvivable`]: not because the guest crashes, but because the
    /// port would be defaulting where it could be stating and nothing could tell.
    RefusalFailsOpen,
    /// ★ **The refusal is invisible AND the state it leaves is what a real GSP's answer
    /// leaves — refusing changes nothing observable in the guest.**
    ///
    /// ⊘ The class that is easiest to get wrong in the flattering direction. An entry here
    /// must cite the oracle's *own* captured reply for the control and show that it is the
    /// same content the refusal path leaves behind. It is not "probably fine"; it is a
    /// byte comparison against the capture.
    ///
    /// ⚠ Refusing is still **distinguishable from a real GSP at the envelope**, which sets
    /// `rpc_result = NV_OK` where we set `NV_ERR_NOT_SUPPORTED`, and RM logs the difference
    /// at `LEVEL_ERROR`. That is a diagnostic cost, not a correctness one, and it is why a
    /// control in this class may still be served.
    RefusalIsInvisible,
    /// ★★ **The refusal halts the boot at a named statement — loud, attributable, and a
    /// rung this port has not spent.**
    ///
    /// The caller turns the failure into a status `gpuStateInit_IMPL` does **not** map to
    /// `NV_OK` (anything other than `NV_ERR_NOT_SUPPORTED`), so the boot aborts rather than
    /// continuing damaged. ⊘ Refusing is *safe*. It is simply the end of the road, and
    /// every engine behind it in `gpuChildOrderList_GM200` is unreachable — which makes
    /// this the class the next batch is drawn from.
    RefusalHalts,
}

impl SweepDisposition {
    /// Whether this port is obliged to serve a control with this disposition.
    ///
    /// ★ Derived here rather than restated in the test, so the gate and the enum cannot
    /// drift. A new variant does not compile until this `match` says which side it is on.
    #[must_use]
    pub fn must_be_served(self) -> bool {
        match self {
            Self::AmputationUnsurvivable | Self::RefusalFailsOpen => true,
            Self::AmputationIntended | Self::RefusalIsInvisible | Self::RefusalHalts => false,
        }
    }
}

/// One control the guest asks from inside — or past — the engine sweep, and what this port
/// decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SweepControl {
    /// The `NV2080_CTRL_*` command id.
    pub cmd: u32,
    /// The engine whose `StatePreInit`/`StateInitLocked` issues it. For diagnostics and for
    /// the reader; never branched on.
    pub engine: &'static str,
    /// The decision.
    pub disposition: SweepDisposition,
    /// ★ **The argument, in one line, and it is required.** A disposition with no reason is
    /// the shape this table exists to outlaw: `t134a`'s defect was not a wrong decision, it
    /// was no decision.
    pub why: &'static str,
}

/// ★★ **Every control this port has triaged against the sweep.**
///
/// ⊘ Quantified over by `tests/sweep_triage.rs`, so shortening it weakens the gate — the
/// failure mode `docs/design/…` calls *"a smaller universe is a smaller true statement"*.
/// The test pins the length as well as the contents, and
/// `crates/kayfabe-crec/tests/cap1b_differential.rs` demands that every control the oracle
/// is *observed* to ask be either here or served.
///
/// ★ In `rpc.sequence` order as the oracle asks them, so the table reads as the boot does.
pub static SWEEP_TRIAGE: &[SweepControl] = &[
    // ── seq 7 ──────────────────────────────────────────────────────────────────────
    SweepControl {
        cmd: 0x2080_0a87,
        engine: "KernelNvlink",
        disposition: SweepDisposition::AmputationIntended,
        why: "a GeForce GA106 has no NVLink; the caller handles the status itself with \
              NV_PRINTF(LEVEL_INFO, \"NVLink is unavailable\") (ogkm-580: \
              kernel_nvlink.c:1826-1830), and a real GA106's own GSP answers this control \
              0x56 too (C: mode2_initctrl_ga106.h:6251)",
    },
    // ── seq 8 ──────────────────────────────────────────────────────────────────────
    SweepControl {
        cmd: kayfabe_abi::deviceinfo::NV2080_CTRL_CMD_INTERNAL_GET_DEVICE_INFO_TABLE,
        engine: "KernelFifo",
        disposition: SweepDisposition::AmputationUnsurvivable,
        why: "asked from gpuStateInit_IMPL, whose loop maps NV_ERR_NOT_SUPPORTED to NV_OK \
              and does NOT remove the engine (ogkm-580: gpu.c:2286-2287) — so unlike the \
              PreInit sweep it leaves a constructed-but-uninitialised KernelFifo with no \
              engine list, which every NULL check passes and which \
              memmgrCalcReservedFbSpaceHal_GM107 then sizes a heap reservation from; \
              [measured] run t135a, a stock 580.159.04 guest at c84ef52",
    },
    // ── seq 11 ─────────────────────────────────────────────────────────────────────
    SweepControl {
        cmd: kayfabe_abi::memsysconfig::NV2080_CTRL_CMD_INTERNAL_MEMSYS_GET_STATIC_CONFIG,
        engine: "KernelMemorySystem",
        disposition: SweepDisposition::AmputationUnsurvivable,
        why: "GPU_GET_KERNEL_MEMORY_SYSTEM is dereferenced with no NULL check by \
              memmgrGetBlackListPagesForHeap_GM107 (ogkm-580: mem_mgr_gm107.c:1719-1725), \
              through an NVOC vtable load that faults before any callee body runs; \
              [measured] run t134a, a stock 580.159.04 guest at 1c79474",
    },
    // ── seq 13 and 44 ──────────────────────────────────────────────────────────────
    SweepControl {
        cmd: kayfabe_abi::confcompute::NV2080_CTRL_CMD_INTERNAL_CONF_COMPUTE_GET_STATIC_INFO,
        engine: "ConfidentialCompute",
        disposition: SweepDisposition::RefusalFailsOpen,
        why: "asked twice under NV_ASSERT_OK_OR_RETURN, from confComputeStateInitLocked_IMPL \
              and confComputeStatePostLoad_IMPL (ogkm-580: conf_compute.c:548-566, :441-456), \
              and both loops map NV_ERR_NOT_SUPPORTED to NV_OK without removing the engine \
              (gpu.c:2286-2287, :3437-3439) — so ccStaticInfo, a zeroed NVOC member nobody \
              re-zeroes, is byte-identical whether refused or served, and the port would be \
              defaulting where it can state. Both bits clear is RM refusing to map CPR \
              vidmem through BAR1 (mapping_cpu.c:227-235), which is a refusal worth keeping \
              deliberately rather than by accident",
    },
    // ── seq 14 ─────────────────────────────────────────────────────────────────────
    SweepControl {
        cmd: kayfabe_abi::bifstatic::NV2080_CTRL_CMD_INTERNAL_BIF_GET_STATIC_INFO,
        engine: "KernelBif",
        disposition: SweepDisposition::RefusalFailsOpen,
        why: "kbifStateInitLocked_IMPL calls kbifStaticInfoInit as a BARE STATEMENT and \
              discards its status (ogkm-580: kernel_bif.c:132) while every other call in \
              that function is checked, and the params are portMemSet to zero before the \
              call (:401-409) — so a refusal is invisible twice over. Two of the four \
              NvBools are directions rather than descriptions: bIsC2CLinkUp sends \
              kmemsysStateInitLocked down a coherent chip-to-chip mapping (kern_mem_sys.c:168, \
              342) and bIsDeviceMultiFunction sends _kbifSavePcieConfigRegisters at a PCI \
              function 1 (kernel_bif_gm107.c:430-441)",
    },
    // ── seq 15, 16, 17 and 34 ──────────────────────────────────────────────────────
    SweepControl {
        cmd: kayfabe_abi::fifochannels::NV2080_CTRL_CMD_INTERNAL_FIFO_GET_NUM_CHANNELS,
        engine: "KernelFifo",
        disposition: SweepDisposition::RefusalHalts,
        why: "kfifoRunlistQueryNumChannels_KERNEL returns 0 on any failure (ogkm-580: \
              kernel_fifo.c:1330-1336) and kfifoChidMgrConstruct turns that 0 into \
              NV_ERR_INVALID_STATE (:300-308) — which is NOT NV_ERR_NOT_SUPPORTED, so \
              gpuStateInit_IMPL takes its goto rather than mapping it to NV_OK \
              (gpu.c:2288-2289) and the boot aborts at a named statement. Safe to refuse, \
              and the end of the road: every engine after KernelFifo in \
              gpuChildOrderList_GM200 is unreachable behind it. Served",
    },
    // ── seq 18 ─────────────────────────────────────────────────────────────────────
    SweepControl {
        cmd: 0x2080_2a08,
        engine: "KernelCE",
        disposition: SweepDisposition::RefusalIsInvisible,
        why: "gpuGetCeFaultMethodBufferSize_KERNEL returns NV_OK UNCONDITIONALLY and leaves \
              *size unwritten when the control fails (ogkm-580: gpu.c:6031-6043), and both \
              consumers initialise their local to 0 before calling \
              (kernel_fifo_gv100.c:302-315, kernel_channel_group_gv100.c:77) — while the \
              oracle's own GA106 answered this control with size = 0 as well \
              (C: mode2_initctrl_ga106.h:6233, {0x20802a08u, 0x0u, 4u, 0u} with an empty \
              ctl_20802a08[]). Refusing and serving the truth leave the same number",
    },
    // ── seq 19 and 20 ──────────────────────────────────────────────────────────────
    SweepControl {
        cmd: 0x2080_0afe,
        engine: "GpuUserSharedData",
        disposition: SweepDisposition::RefusalHalts,
        why: "_gpushareddataInitGsp hands GSP the physical address of the RUSD page under \
              NV_ASSERT_OK_OR_RETURN (ogkm-580: gpu_user_shared_data.c:221-238); the status \
              propagates to gpushareddataConstruct_IMPL and the RM_USER_SHARED_DATA class \
              allocation fails, which is loud and attributable. ⊘ Serving would be a LIE \
              rather than an omission — this port has no RUSD publisher, so an NV_OK would \
              promise a page that is never written",
    },
    SweepControl {
        cmd: 0x2080_0aff,
        engine: "GpuUserSharedData",
        disposition: SweepDisposition::RefusalHalts,
        why: "the polling half of the same subsystem, issued by _gpushareddataSendDataPollRpc \
              under NV_ASSERT_OK_OR_RETURN (ogkm-580: gpu_user_shared_data.c:265-274). Same \
              argument as 0x20800afe and it must be decided with it: answering one and \
              refusing the other would tell the guest a shared page exists and then never \
              refresh it",
    },
    // ── seq 25 ─────────────────────────────────────────────────────────────────────
    SweepControl {
        cmd: 0x2080_0301,
        engine: "Subdevice (event)",
        disposition: SweepDisposition::RefusalHalts,
        why: "NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION is the event-registration verb, not an \
              engine's static description; its GSP-side arm is issued under \
              NV_CHECK_OK_OR_RETURN(LEVEL_WARNING) from \
              subdeviceCtrlCmdEventSetNotification_IMPL (ogkm-580: \
              subdevice_ctrl_event_kernel.c:110-117). ★★★ SERVED as of the \
              event-notification rung, and this row's original argument is CORRECTED rather \
              than deleted. It read: 'this port gates event delivery off after \
              GSP_INIT_DONE — IrqRaise == 1 across the whole of cap1 with ZERO IRQSCLR \
              writes — so accepting a notification registration would promise an interrupt \
              nothing raises'. The observation is right and the inference is not: it \
              conflates REGISTERING an arming with DELIVERING an event, and an undelivered \
              notification costs something only for an event that can occur. The one this \
              control registers is NV2080_NOTIFIERS_POWER_RESUME (ogkm-580: \
              cl2080_notification.h:235), which fires from a power-state transition this \
              device never performs. ⊘ The promise is scoped to a LIST for exactly that \
              reason — kayfabe_abi::eventnotify::SILENT_NOTIFIERS — and every other \
              notifier index is still refused. Refusing this one halts the boot: it is the \
              last statement of memmgrStateInitLocked_IMPL, whose failure path rolls the \
              phase back through memmgrStateDestroy and DELETES the heap created ninety \
              lines earlier (ogkm-580: mem_mgr.c:625, :777, :684, :963-975). [measured] \
              2026-08-01, boots alloc1 (2ced035) and alloc2 (a6412c0)",
    },
    // ── seq 26 ─────────────────────────────────────────────────────────────────────
    SweepControl {
        cmd: kayfabe_abi::gmmustatic::NV2080_CTRL_CMD_INTERNAL_GMMU_GET_STATIC_INFO,
        engine: "KernelGmmu",
        disposition: SweepDisposition::AmputationUnsurvivable,
        why: "_kgmmuInitStaticInfo's fail: label portMemFrees pKernelGmmu->pStaticInfo and \
              does NOT NULL the field (ogkm-580: kern_gmmu.c:139-166), while \
              gpuStateInit_IMPL maps the refusal to NV_OK and carries on (gpu.c:2286-2287) — \
              so KernelGmmu survives with a DANGLING pointer, which is worse than the NULLed \
              engine pointer of 0x20800a1c because every NULL check passes it and \
              guest-reachable control handlers read through it \
              (mmu_fault_buffer_ctrl.c:84, 176). [inferred] from source; no boot has been \
              spent at a revision that serves it",
    },
    // ── seq 28 and 29 ──────────────────────────────────────────────────────────────
    SweepControl {
        cmd: 0x2080_0a70,
        engine: "KernelBus",
        disposition: SweepDisposition::RefusalIsInvisible,
        why: "★★ CORRECTED (2026-08-01, with 0x20800a6c). This row read 'its callers treat a \
              failed sysmembar as a failed flush' and classified it RefusalHalts. That is \
              FALSE on the GA106 HAL path, and the correction is one function deep: \
              kbusSendSysmembarSingle_KERNEL does return the status verbatim (ogkm-580: \
              kern_bus.c:420-433) and kbusFlushSingle_GM107 does propagate it \
              (kern_bus_gm107.c:3345-3353) — but its only caller kbusFlush_GM107 keeps \
              `NV_STATUS status = NV_OK` and overwrites it ONLY for NV_ERR_TIMEOUT \
              (kern_bus_gm107.c:3384-3405), and GA106 dispatches kbusFlush to _GM107 \
              (g_kern_bus_nvoc.c:1871-1881). So NV_ERR_NOT_SUPPORTED is swallowed, including \
              at kbusVerifyBar2_GM107:4218-4221, the one site that checks a flush. ⇒ the \
              refusal is INVISIBLE, and the byte comparison the class demands is trivial \
              because there are no bytes on either side: the control takes NULL params and \
              paramsSize 0 (kern_bus.c:428-430), and the oracle's own captured reply is \
              psize 0, dlen 0, empty array (C: mode2_initctrl_ga106.h:6244, ctl_20800a70[]). \
              ⊘ NOT served, and the reason is the one that separates it from 0x20800a6c: an \
              L2 evict's postcondition is about a READ path this port has exactly one \
              authority for, while a sysmembar's is about a WRITE path crossing to system \
              memory — the path a real host GPU's uninstrumented pci_dma_map will occupy the \
              day forwarding is on. Vacuity makes an NV_OK permissible; a caller that CHECKS \
              is what makes it necessary. This control has the first and not the second",
    },
    // ── seq 30, 31 and 32 ──────────────────────────────────────────────────────────
    SweepControl {
        cmd: 0x2080_0a6c,
        engine: "KernelMemorySystem",
        disposition: SweepDisposition::RefusalHalts,
        why: "kmemsysSendL2InvalidateEvict_IMPL returns the control's status verbatim \
              (ogkm-580: kern_mem_sys.c:1079-1093), and kbusVerifyBar2_GM107:4110-4115 turns \
              that into NV_PRINTF('L2 evict failed') and a goto — loud, named, and where the \
              bar0win boot stopped. ★★★ SERVED as of the L2-evict rung, and this row's \
              original argument is CORRECTED rather than deleted. It read: 'an L2 \
              invalidate/evict is an ACTION on the cache, and an NV_OK this port cannot back \
              would tell the guest its framebuffer view is coherent when nothing made it \
              so'. The premise is right and the conclusion does not follow: it assumes the \
              coherence has to be MADE. The operation's only observable is the read \
              kbusVerifyBar2 performs on the very next line (kern_bus_gm107.c:4106-4118), \
              and on this device that read cannot be stale — kayfabe_device::fbwin's store IS \
              the framebuffer rather than a cache over one, and the trapped write commits \
              before the vmexit returns. So NV_OK says 'the state you asked for already \
              holds', not 'we did it'. ★ Corroborated, not proven, by the oracle: a real \
              GA106's own GSP answers this NV_OK with a four-zero body (C: \
              mode2_initctrl_ga106.h:6245, {0x20800a6cu, 0x0u, 4u, 0u}). ⊘ Three named \
              futures falsify it — real host-GPU forwarding, a write-back layer of this \
              port's own, and any second writer of the framebuffer — and kayfabe_abi::l2evict \
              carries all three plus the flag-by-flag licence. ⚠ It is NOT decided with \
              0x20800a70 after all; see that row. [measured] 2026-08-01, boot l2evict1 \
              (9551dd1): 'L2 evict failed' is GONE, the control leaves the unserviced list, \
              and kbusVerifyBar2_GM107 now fails ninety lines later at :4200 — the MMU \
              test's read-back, which is past BOTH of the first two evicts",
    },
    // ── seq 33 and 38 ──────────────────────────────────────────────────────────────
    SweepControl {
        cmd: 0x2080_0a80,
        engine: "KernelPerf",
        disposition: SweepDisposition::RefusalIsInvisible,
        why: "kperfGpuBoostSyncStateInit_IMPL logs the failure and returns NV_OK REGARDLESS \
              (ogkm-580: kern_perf_gpuboostsync.c:42-79 — the function's last statement is \
              `return NV_OK` after the error label), so the refusal cannot fail anything; it \
              only skips writing sliGpuBoostSync, which stays zeroed. SLI GPU-boost \
              synchronisation is a multi-GPU clock-sharing feature this port does not offer \
              at all, so zeroed is also the true answer and the oracle's own 16-byte reply \
              (C: mode2_initctrl_ga106.h:6209) is never read by anything that branches",
    },
    // ── seq 39 ─────────────────────────────────────────────────────────────────────
    SweepControl {
        cmd: 0x2080_2a0f,
        engine: "KernelCE",
        disposition: SweepDisposition::RefusalHalts,
        why: "kceGetPceConfigForLceType issues it under NV_ASSERT_OK_OR_RETURN and copies out \
              five fields the caller has no default for — numPces, numLces, supportedPceMask, \
              supportedLceMask, pcePerHshub (ogkm-580: kernel_ce.c:1020-1034). ⊘ A served \
              answer is a PCE-to-LCE topology claim, and this port's copy-engine plane is the \
              one place a wrong topology is not diagnosable from the reply — it surfaces as a \
              copy that lands nowhere. Refusing halts at a named statement instead",
    },
    // ── seq 40 and 42 ──────────────────────────────────────────────────────────────
    SweepControl {
        cmd: 0x2080_2a06,
        engine: "KernelCE",
        disposition: SweepDisposition::RefusalHalts,
        why: "kceUpdateClassDB_KERNEL issues it and then NV_ASSERT_OK_OR_RETURNs the status \
              (ogkm-580: kernel_ce.c:618-630) before walking params.stubbedCeMask to remove \
              stubbed copy engines from the class database. ⊘ Serving it means declaring \
              WHICH LCEs have no PCEs behind them, which is the same topology claim as \
              0x20802a0f and must be decided with it — an all-zero stubbedCeMask says every \
              advertised LCE is real",
    },
    // ── seq 41 ─────────────────────────────────────────────────────────────────────
    SweepControl {
        cmd: 0x2080_2a0d,
        engine: "KernelCE",
        disposition: SweepDisposition::RefusalHalts,
        why: "the 156-byte PCE-to-LCE mapping itself (C: mode2_initctrl_ga106.h:6214, \
              {0x20802a0du, 0x0u, 156u, 156u}), issued from kceTopLevelPceLceMappingsUpdate \
              after an NV_ASSERT_OK_OR_RETURN (ogkm-580: kernel_ce.c:794-806). ⊘ The third \
              member of the copy-engine topology triple with 0x20802a0f and 0x20802a06; the \
              oracle carries all 156 bytes, so serving is possible — it is deferred because \
              a topology served in pieces is worse than one refused whole",
    },
    // ── seq 43 ─────────────────────────────────────────────────────────────────────
    SweepControl {
        cmd: 0x2080_017e,
        engine: "OBJGPU (gpuInitVmmuInfo)",
        disposition: SweepDisposition::AmputationIntended,
        why: "gpuInitVmmuInfo tests for this exact status and returns NV_OK, with the \
              comment \"Leave segment size initialized to zero to signal no VMMU present on \
              physical\" (ogkm-580: gpu.c:906-935). ★ The caller's own tolerance, written \
              down by NVIDIA — refusing IS the documented way to say this device has no VMMU, \
              and a GeForce GA106 has none",
    },
    // ── seq 45 ─────────────────────────────────────────────────────────────────────
    SweepControl {
        cmd: 0x2080_0a9f,
        engine: "OBJGVASPACE",
        disposition: SweepDisposition::RefusalHalts,
        why: "_gvaspaceCopyServerReservedPdes issues it under NV_ASSERT_OK_OR_RETURN after \
              populating the PDE entries it is about to publish (ogkm-580: \
              gpu_vaspace.c:4144-4152), so a refusal aborts the reserved-split GVASPACE \
              construction at a named statement. ⊘ Serving it is a page-table publication \
              and belongs with docs/design/mode2_address_table.md's populate sources, not \
              with a static chip row — it is the first control in this table that is not a \
              description of silicon at all",
    },
    // ── seq 49, 50 and 51 ──────────────────────────────────────────────────────────
    SweepControl {
        cmd: 0x2080_0a1f,
        engine: "KernelGraphics",
        disposition: SweepDisposition::RefusalHalts,
        why: "kgraphicsLoadStaticInfo issues GET_CAPS under NV_CHECK_OK_OR_GOTO(cleanup) \
              into a portMemSet-zeroed params block (ogkm-580: kernel_graphics.c:1210-1225), \
              and the cleanup arm propagates. ⊘ The first of the three GR static-info \
              replies and by far the smallest at 184 bytes (C: mode2_initctrl_ga106.h:6218); \
              deferred with the other two because a GR capability bitmap served without the \
              GR info and floorsweeping masks that qualify it is a partial description of \
              the one engine this port's north star runs on",
    },
    SweepControl {
        cmd: 0x2080_0a2a,
        engine: "KernelGraphics",
        disposition: SweepDisposition::RefusalHalts,
        why: "GET_INFO, the 3712-byte second member of the GR static-info triple \
              (C: mode2_initctrl_ga106.h:6219, {0x20800a2au, 0x0u, 3712u, 3712u}); \
              kgraphicsLoadStaticInfo takes its status and only allocates pGrInfo when it is \
              NV_OK (ogkm-580: kernel_graphics.c:1228-1240), so a refusal leaves pGrInfo NULL \
              rather than dangling. Deferred with 0x20800a1f and 0x20800a26",
    },
    SweepControl {
        cmd: 0x2080_0a26,
        engine: "KernelGraphics",
        disposition: SweepDisposition::RefusalHalts,
        why: "GET_FLOORSWEEPING_MASKS, the 3008-byte third member of the GR static-info \
              triple (C: mode2_initctrl_ga106.h:6220), issued under NV_CHECK_OK_OR_GOTO from \
              the same function (ogkm-580: kernel_graphics.c:1253-1260). ⊘ Floorsweeping \
              masks are the statement of which TPCs and GPCs this die actually has, so they \
              must be served together with the engine list they qualify — deferred as one \
              decision with 0x20800a1f and 0x20800a2a",
    },
    // ── NOT in the oracle's prefix: kept because the decision is still ours to make ──
    SweepControl {
        cmd: 0x2080_0a4b,
        engine: "KernelDisplay",
        disposition: SweepDisposition::AmputationIntended,
        why: "ENG_KERNEL_DISPLAY is one of the three engines the sweep removes on purpose \
              (ogkm-580: gpu.c:2178-2182, via gpuRemoveMissingEngineClasses), and \
              kdispStatePreInitLocked_IMPL returns this very status itself when the display \
              fuse is clear (ogkm-580: kern_disp.c:329-330); this device serves no display \
              plane. ⊘ The one row here the oracle's board never asked for, so it has NO \
              reply-plane coverage and must NOT be served — cap1b would then be a capture \
              that cannot exercise a served control",
    },
];

/// The dispositions this port has recorded for `cmd`, or `None` if the control has not been
/// triaged at all.
///
/// ⊘ `None` is the dangerous answer, not a neutral one: an untriaged control reached from
/// the sweep is exactly `t134a`'s defect. It is returned rather than defaulted so a caller
/// has to say what it wants to do about it.
#[must_use]
pub fn triage_for(cmd: u32) -> Option<&'static SweepControl> {
    SWEEP_TRIAGE.iter().find(|c| c.cmd == cmd)
}

/// ★★★ **The gate, as a function rather than only as a test** — every control whose refusal
/// this port has judged unsurvivable or silently wrong, and which nothing in
/// [`WantedTable::ALL`] serves.
///
/// A non-empty result is a port that will amputate an engine RM then dereferences, or answer
/// a default where it could have stated a fact with nothing able to tell. It is computed
/// from [`SWEEP_TRIAGE`] and [`WantedTable::ALL`] rather than restated, so neither list can
/// be shortened into agreement.
#[must_use]
pub fn must_serve_and_unserved() -> Vec<&'static SweepControl> {
    SWEEP_TRIAGE
        .iter()
        .filter(|c| c.disposition.must_be_served())
        .filter(|c| WantedTable::from_cmd(c.cmd).is_none())
        .collect()
}

kayfabe_util::assert_send_sync!(SweepControl, SweepDisposition);
