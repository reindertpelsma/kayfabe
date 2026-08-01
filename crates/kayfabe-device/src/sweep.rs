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
              0x56 too — ★ that corroboration was [inferred] from the C's captured row \
              (C: mode2_initctrl_ga106.h:6251) and is now [measured] first-hand: an RTX 3060 \
              on open 580.159.04 answers `cmd=0x20800a87 psize=1056 gspst=0x56` \
              (traces/real_ga106/rpc_transcript_real_ga106.txt)",
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
        disposition: SweepDisposition::AmputationUnsurvivable,
        why: "★ RECLASSIFIED from RefusalIsInvisible by boot irq1 at bb4f48d, then \
              re-argued: the refusal IS invisible as a status \
              (gpuGetCeFaultMethodBufferSize_KERNEL returns NV_OK unconditionally and \
              leaves *size unwritten, ogkm-580: gpu.c:6031-6043) and the NV_ASSERT at \
              kernel_channel_group_gv100.c:78 does not return — so execution CONTINUES with \
              bufSizeInBytes = 0 into memdescCreate, which rejects a zero length with \
              NV_ERR_INVALID_ARGUMENT (mem_desc.c:239-241). That is RmInitAdapter \
              0x25:0x1f:1249. ⊘ Nothing 'converts' the 0x56 — it is DISCARDED, and an \
              independent 0x1f is manufactured eleven frames later from the zero it left. \
              ⚠ The old row cited the oracle's empty ctl_20802a08[] as corroboration that a \
              real GA106 answers 0; that is REFUTED — a real RTX 3060 running open \
              580.159.04, asked directly, answers NV_OK size = 20480 \
              ([measured] traces/real_ga106/, and see kayfabe_abi::fmbsize for the five \
              other dlen=0 oracle rows hardware contradicts). Served",
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
              holds', not 'we did it'. ⊘ ★★★ This row USED to add 'Corroborated by the \
              oracle: a real GA106's own GSP answers this NV_OK with a four-zero body (C: \
              mode2_initctrl_ga106.h:6245, {0x20800a6cu, 0x0u, 4u, 0u})' — REFUTED. That \
              row is one of the eleven with NO captured body, and a real GA106 asked \
              directly ECHOES the flags it was sent: 31 00 00 00 and 11 00 00 00, the two \
              words kbusVerifyBar2_GM107's call sites pass ([measured] 2026-08-01, \
              traces/real_ga106/rpc_bodies_real_ga106.txt; kayfabe_abi::oracle). The NV_OK \
              stands on the argument above alone; the body this port sends is knowingly \
              this port's own, and unread. ⊘ Three named \
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
        // ★★★ CORRECTED, and by a boot rather than by a re-reading. This row said
        // `RefusalHalts` and gave as its reason that the NV_ASSERT_OK_OR_RETURN at
        // gpu_vaspace.c:4148 "aborts ... at a named statement". That is true of the
        // FUNCTION and false of the BOOT: the status travels up through
        // _kgmmuCreateGlobalVASpace (kern_gmmu.c:245) into kgmmuStatePostLoad_IMPL and
        // then into gpuStatePostLoad, which maps NV_ERR_NOT_SUPPORTED to NV_OK at
        // gpu.c:3438 exactly like every other post-gpuPreInit loop. [measured] run gmmu1
        // at 12b001f: three assertion lines, and the boot carried on regardless.
        // ⊘ The lesson is the table's own: "halts" is a claim about where the status ENDS
        // UP, and a local `OR_RETURN` says nothing about that.
        disposition: SweepDisposition::RefusalFailsOpen,
        why: "_gvaspaceCopyServerReservedPdes issues it under NV_ASSERT_OK_OR_RETURN after \
              populating the PDE entries it is about to publish (ogkm-580: \
              gpu_vaspace.c:4144-4152) — but gpuStatePostLoad swallows the 0x56 (gpu.c:3438), \
              so the refusal is INVISIBLE to the boot and leaves a GPU group whose \
              pGlobalVASpace was assigned before its constructor failed (virt_mem_mgr.c:126 \
              vs :134); [measured] run gmmu1 at 12b001f. Served: a page-table publication, \
              decoded and validated against ctrl90f1.h's own rules rather than echoed — see \
              kayfabe_abi::gvaspacepdes and docs/design/mode2_address_table.md",
    },
    // ── seq 49, 50 and 51 ──────────────────────────────────────────────────────────
    // ★★★ The GR static-info rows below were `RefusalHalts` and are now
    // `AmputationUnsurvivable`. The old reason — "the cleanup arm propagates" — described
    // kgraphicsLoadStaticInfo_KERNEL correctly and the BOOT incorrectly: the status it
    // propagates lands in gpuStatePostLoad, which maps NV_ERR_NOT_SUPPORTED to NV_OK at
    // gpu.c:3438. Nothing halts. What actually happens is that `cleanup:` sets
    // bInitialized = NV_FALSE (:1544), kgraphicsGetStaticInfo returns NULL forever
    // (:556-564), and TWENTY-ONE ENGINES LATER KernelFifo's own statePostLoad runs the
    // callback GR registered and dies on it with NV_ERR_INVALID_STATE (:485) — which
    // gpu.c:3440 does not swallow. [measured] run gmmu1 at 12b001f:
    // `RmInitAdapter failed! (0x25:0x40:1249)`.
    //
    // ⊘ That is the amputation class by definition: the refusal is silent and the
    // subsystem that dies is not the one that asked.
    SweepControl {
        cmd: 0x2080_0a1f,
        engine: "KernelGraphics",
        disposition: SweepDisposition::AmputationUnsurvivable,
        why: "kgraphicsLoadStaticInfo issues GET_CAPS under NV_CHECK_OK_OR_GOTO(cleanup) \
              into a portMemSet-zeroed params block (ogkm-580: kernel_graphics.c:1210-1225); \
              cleanup sets bInitialized = NV_FALSE (:1544) so kgraphicsGetStaticInfo returns \
              NULL forever (:556-564), and _kgraphicsPostSchedulingEnableHandler — run from \
              KernelFifo's statePostLoad, not GR's — returns NV_ERR_INVALID_STATE on it \
              (:485), which gpu.c:3440 does NOT swallow. [measured] run gmmu1 at 12b001f. \
              184 bytes (C: mode2_initctrl_ga106.h:6218). Served",
    },
    SweepControl {
        cmd: 0x2080_0a2a,
        engine: "KernelGraphics",
        // ★★★ RECLASSIFIED from `AmputationIntended` by boot `fmb1` at `93191ee`. The old
        // row's reading of the CALL SITE was correct and its conclusion was not: the caller
        // tolerating a refusal says nothing about who else reads what the caller failed to
        // allocate. This is the third control to sit in that shape.
        disposition: SweepDisposition::AmputationUnsurvivable,
        why: "GET_INFO, 3712 bytes. kgraphicsLoadStaticInfo takes its status into a bare \
              `if (status == NV_OK)` with NO else arm and only allocates pGrInfo inside it \
              (ogkm-580: kernel_graphics.c:1228-1248) — so the CALLER tolerates a refusal, \
              and that is not who dies. kfifoGetMaxSubcontextFromGr_KERNEL asserts \
              pGrInfo != NULL and RETURNS 0 (kernel_fifo.c:2789-2792); \
              kchangrpapiSetLegacyMode rejects that zero at `numMax != 0` \
              (kernel_channel_group_api.c:913) with NV_ERR_INVALID_STATE, which gpu.c:3440 \
              does NOT swallow. [measured] run fmb1 at 93191ee, stock 580.159.04 guest: \
              RmInitAdapter failed! (0x25:0x40:1249). \
              ★ The citation carries the CONTENT, not the address: the C's row is \
              {0x20800a2au, 0x0u, 3712u, 3712u} (C: mode2_initctrl_ga106.h:6219) — a FULL \
              body, dlen == psize — and it is byte-identical to a real GA106 for all 3712 \
              bytes ([measured] 2026-08-01, RTX 3060 on open 580.159.04, \
              traces/real_ga106/rpc_bodies_real_ga106.txt). Its infoList[0x2c] \
              MAX_SUBCONTEXT_COUNT is 64, and 0 is the value that rebuilds the wall. \
              ⊘ Contrast kayfabe_abi::oracle: a dlen=0 row would have cited the same file \
              and been worth nothing. Served",
    },
    SweepControl {
        cmd: 0x2080_0a26,
        engine: "KernelGraphics",
        disposition: SweepDisposition::AmputationUnsurvivable,
        why: "GET_FLOORSWEEPING_MASKS, 3008 bytes (C: mode2_initctrl_ga106.h:6220), issued \
              under NV_CHECK_OK_OR_GOTO (ogkm-580: kernel_graphics.c:1253-1260) with the \
              same silent-then-fatal consequence as 0x20800a1f. ⚠ Its gpcMask is read \
              DIRECTLY by _kgraphicsPostSchedulingEnableHandler, which returns NV_OK early \
              when it is 0 (:486) — so a zero here would skip the golden-image channel and \
              buy a longer green log with a lie about the die. This device publishes 0x7. \
              Served",
    },
    SweepControl {
        cmd: 0x2080_0a22,
        engine: "KernelGraphics",
        disposition: SweepDisposition::AmputationUnsurvivable,
        why: "GET_GLOBAL_SM_ORDER under NV_CHECK_OK_OR_GOTO (ogkm-580: \
              kernel_graphics.c:1284-1292); 34592 bytes, the largest reply this port encodes \
              and nine message-queue elements (C: mode2_initctrl_ga106.h:6221, whose own \
              capture kept only the first 16376 of them). Same silent-then-fatal path as \
              0x20800a1f. Served",
    },
    SweepControl {
        cmd: 0x2080_0a30,
        engine: "KernelGraphics",
        disposition: SweepDisposition::AmputationIntended,
        why: "GET_PPC_MASKS; kgraphicsLoadStaticInfo converts NV_ERR_NOT_SUPPORTED to NV_OK \
              explicitly, with the comment \"Some chips don't support this call, so just \
              keep the pPpcMasks pointer as NULL, but don't return error\" (ogkm-580: \
              kernel_graphics.c:1316-1327). ★ The caller's own tolerance, written down by \
              NVIDIA — refusing IS the sanctioned way to say this chip has no PPC masks",
    },
    SweepControl {
        cmd: 0x2080_0a2c,
        engine: "KernelGraphics",
        // ★★ The correction this rung is proudest of, because the `else if` reads the
        // other way. See kayfabe_abi::grstatic's header.
        disposition: SweepDisposition::AmputationIntended,
        why: "GET_ZCULL_INFO. ⚠ Its else-if forgives NV_ERR_NOT_SUPPORTED only under MIG \
              (ogkm-580: kernel_graphics.c:1345-1356), which is OFF here — so read alone it \
              looks mandatory. It is not: the arm does not branch, and the next paragraph's \
              first act is `status = pRmApi->Control(...)` for ROP at :1360, an ASSIGNMENT \
              that discards it. The 0x56 is dead before anything tests it",
    },
    SweepControl {
        cmd: 0x2080_0a2e,
        engine: "KernelGraphics",
        disposition: SweepDisposition::AmputationIntended,
        why: "GET_ROP_INFO, the same shape as 0x20800a2c and the same reason (ogkm-580: \
              kernel_graphics.c:1375-1386): its MIG-only else-if leaves 0x56 in `status`, and \
              SM_ISSUE_RATE_MODIFIER's call at :1391 overwrites it — and that one \
              normalises NV_ERR_NOT_SUPPORTED to NV_OK unconditionally at :1410. ⊘ The \
              oracle's own GA106 answered ROP_INFO with three zeros anyway \
              (C: mode2_initctrl_ga106.h:6224)",
    },
    SweepControl {
        cmd: 0x2080_0a3d,
        engine: "KernelGraphics",
        disposition: SweepDisposition::AmputationUnsurvivable,
        why: "GET_FECS_RECORD_SIZE under NV_CHECK_OK_OR_GOTO (ogkm-580: \
              kernel_graphics.c:1465-1476); 32 bytes, one NvU32 per engine, 128 on this part \
              (C: mode2_initctrl_ga106.h:6246). Same silent-then-fatal path as 0x20800a1f, \
              and the value is a divisor in the FECS buffer's record arithmetic so zero is \
              not merely a small answer. Served",
    },
    SweepControl {
        cmd: 0x2080_0a3f,
        engine: "KernelGraphics",
        disposition: SweepDisposition::AmputationIntended,
        why: "GET_FECS_TRACE_DEFINES; kgraphicsLoadStaticInfo converts NV_ERR_NOT_SUPPORTED \
              to NV_OK explicitly and leaves pFecsTraceDefines NULL (ogkm-580: \
              kernel_graphics.c:1494-1501). This device delivers no FECS trace, so declining \
              to define its record layout is the consistent statement",
    },
    SweepControl {
        cmd: 0x2080_0a48,
        engine: "KernelGraphics",
        disposition: SweepDisposition::AmputationUnsurvivable,
        why: "GET_PDB_PROPERTIES under NV_CHECK_OK_OR_GOTO (ogkm-580: \
              kernel_graphics.c:1504-1518) — the LAST mandatory one, so it is the control \
              that decides whether bInitialized becomes NV_TRUE on the next line (:1521). \
              Eight bytes, one NvBool per engine; bPerSubCtxheaderSupported is true on \
              Ampere (C: mode2_initctrl_ga106.h:6252) and is consumed immediately by \
              kgraphicsSetPerSubcontextContextHeaderSupported. Served",
    },
    SweepControl {
        cmd: 0x2080_0a32,
        engine: "KernelGraphics",
        // ★★★ The row `#150` REFUSED to write, and the boot wrote it. That rung left
        // `kgraphicsShouldDeferContextInit` unevaluated on purpose — `bInitialized` is set
        // at :1521 BEFORE the branch, so the two outcomes are distinguishable in one log
        // and an argument was not needed. [measured] run `stateload1` at `041b4f1`: the
        // branch IS taken and this control IS issued. ⊘ Recording the absence of a
        // measurement beat borrowing one, and it cost nothing.
        disposition: SweepDisposition::AmputationUnsurvivable,
        why: "GET_CONTEXT_BUFFERS_INFO, reached from kgraphicsInitializeDeferredStaticData \
              under NV_CHECK_OK_OR_GOTO (ogkm-580: kernel_graphics.c:743-751) which \
              kgraphicsLoadStaticInfo calls at :1527 behind \
              !kgraphicsShouldDeferContextInit — FALSE on this chip, [measured] run \
              stateload1 at 041b4f1. Refusing sends the whole function to cleanup: and \
              un-sets bInitialized, so it is the same KernelFifo NV_ERR_INVALID_STATE as \
              0x20800a1f by a different route. 1664 bytes \
              (C: mode2_initctrl_ga106.h:6226). Served",
    },
    SweepControl {
        cmd: 0x2080_0a38,
        engine: "KernelGraphics",
        disposition: SweepDisposition::AmputationIntended,
        why: "GET_FECS_TRACE_HW_ENABLE, asked from fecsBufferDisableHw — a function that \
              returns VOID and whose NV_ASSERT_OR_RETURN_VOID therefore cannot propagate \
              anything (ogkm-580: fecs_event_list.c:1623, :1643). ⚠ It appears in this \
              port's dmesg only on the TEARDOWN path that runs after RmInitAdapter has \
              already decided to fail, so it is a symptom of the failure and never a cause \
              of one",
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
    // ── ★★★ The first control that is not a description of silicon at all ──────────
    SweepControl {
        cmd: 0xa06f_0103,
        engine: "KernelChannel (CE scrubber)",
        disposition: SweepDisposition::RefusalHalts,
        why: "★★★ NVA06F_CTRL_CMD_GPFIFO_SCHEDULE, and the first row in this table whose \
              decision is NOT 'which bytes'. Its params are three NvBools, ALL [IN] \
              (ogkm-580: ctrla06fgpfifo.h:69-73), so there is no reply to get right; the \
              load-bearing half is the status. ⚠ [measured] 2026-08-01, boot grinfo1: the \
              refusal halts at _memmgrMemUtilsScrubInitScheduleChannel, which swallows the \
              status and returns NV_ERR_GENERIC (ogkm-580: mem_utils.c:1976-1987) — and \
              THAT is what proves the message names its own cause, because the sibling \
              NVA06F_CTRL_CMD_BIND arm in the same function returns rmStatus verbatim \
              (:1969), so a BIND failure would have surfaced as 0x56 at mem_utils.c:2006 \
              rather than 0xFFFF. The later 'Assertion failed: 0 @ kernel_fifo.c:3129' is \
              NV_ASSERT(0) in the postSchedulingEnable callback loop's abort arm and does \
              not alter status (ogkm-580: kernel_fifo.c:3126-3131) — it PROPAGATES. \
              ⊘ NOT SERVED, and the reason is the quadrant this control sits in. \
              0x20800a6c is served because its postcondition ('the next read misses a \
              cache') holds STRUCTURALLY on this device and a caller checks it. This one \
              has the checking caller and NOT the structural argument: CPU-RM does only \
              bookkeeping — kchannelIsSchedulable_HAL, then kchannelSetRunlistSet, then it \
              RPCs to GSP under the comment 'All real hardware management is done in the \
              host' (ogkm-580: kernel_channel.c:3105-3130) — so the runlist write, the \
              RAMFC update and the runlist submit are ALL on our side of the line, and the \
              postcondition 'work submitted to this channel executes' is simply FALSE here. \
              An NV_OK would be a fabricated completion, not a modelled yes. \
              ★★★ [measured] 2026-08-01, boot schedprobe1 (0bf7eb7 + a throwaway serve arm, \
              NEVER LANDED; /workspace/bench/run_schedprobe1_dmesg.log): serving NV_OK moves \
              the wall forward EXACTLY ONE STEP, from mem_utils.c:2006 to mem_utils.c:2022 \
              (_memmgrMemUtilsScrubInitRegisterCallback: 'event notification control \
              failed'), with the verdict line 0x25:0xffff:1249 UNCHANGED. ⊘ That refutes the \
              prediction this row was first written with — that a fabricated schedule would \
              HANG on a CE wait. It does not hang, because at least two more setup steps \
              (the callback registration and kfifoRmctrlGetWorkSubmitToken) stand between \
              the schedule and any submission. The cost of the lie is therefore DEFERRED, \
              not absent, and that is the worse shape, not the better one: the first \
              consumer that actually waits is memmgrTestCeUtils, which memsets and copies \
              via CE and then compares the read-back (ogkm-580: mem_mgr.c:407-470, called \
              at :4158). ⇒ this is an EXECUTION-PLANE rung and cannot be closed by a reply. \
              ⚠ Its oracle row is one of the eleven with NO captured body (C: \
              mode2_initctrl_ga106.h:6234, psize 3, dlen 0); a real GA106 answers 01 00 00 \
              ([measured] 2026-08-01, traces/real_ga106/rpc_bodies_real_ga106.txt), which is \
              bEnable=NV_TRUE echoed back — an [IN] echo and NOT an answer, so even a \
              correct capture of it would license nothing here",
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
