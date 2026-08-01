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
//! An entry classified [`SweepDisposition::AmputationUnsurvivable`] that is not in
//! [`crate::inittables::WantedTable::ALL`] is a compile-and-test failure, not a boot-time
//! `Oops`. `tests/sweep_triage.rs` is where it fails.
//!
//! ## ⊘ What this table is NOT
//!
//! It is not a list of controls to implement, and it must never become one. Two of its four
//! entries are deliberately **refused** — the sweep's amputation is the *correct* behaviour
//! for a chip that genuinely lacks the engine, and RM has its own vocabulary for exactly
//! that. Padding this table with things to serve would invert its purpose.
//!
//! It is also not the engine order. That is `gpuChildOrderList_GM200`
//! (`ogkm-580: src/nvidia/src/kernel/gpu/arch/maxwell/kern_gpu_gm107.c:605`), it is a fact
//! about the guest and not about us, and `docs/design/preinit_sweep_loop.md` §4.1 carries
//! the reading. This table carries only *our decision* per control.

use crate::inittables::WantedTable;

/// What `gpuStatePreInit_IMPL`'s sweep does with a refusal of one control, and whether this
/// port is willing to let it.
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
    /// Something downstream dereferences the engine pointer with no `NULL` check, so
    /// refusing trades a named refusal for a guest-kernel fault attributed to the wrong
    /// subsystem. `tests/sweep_triage.rs` refuses to let an entry in this class be absent
    /// from [`WantedTable::ALL`].
    AmputationUnsurvivable,
}

/// One control the guest asks from inside the engine sweep, and what this port decided.
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
/// The test pins the length as well as the contents.
pub static SWEEP_TRIAGE: &[SweepControl] = &[
    SweepControl {
        cmd: kayfabe_abi::memsysconfig::NV2080_CTRL_CMD_INTERNAL_MEMSYS_GET_STATIC_CONFIG,
        engine: "KernelMemorySystem",
        disposition: SweepDisposition::AmputationUnsurvivable,
        why: "GPU_GET_KERNEL_MEMORY_SYSTEM is dereferenced with no NULL check by \
              memmgrGetBlackListPagesForHeap_GM107 (ogkm-580: mem_mgr_gm107.c:1719-1725), \
              through an NVOC vtable load that faults before any callee body runs; \
              [measured] run t134a, a stock 580.159.04 guest at 1c79474",
    },
    SweepControl {
        cmd: 0x2080_0a4b,
        engine: "KernelDisplay",
        disposition: SweepDisposition::AmputationIntended,
        why: "ENG_KERNEL_DISPLAY is one of the three engines the sweep removes on purpose \
              (ogkm-580: gpu.c:2178-2182, via gpuRemoveMissingEngineClasses), and \
              kdispStatePreInitLocked_IMPL returns this very status itself when the display \
              fuse is clear (ogkm-580: kern_disp.c:329-330); this device serves no display \
              plane",
    },
    SweepControl {
        cmd: 0x2080_0a87,
        engine: "KernelNvlink",
        disposition: SweepDisposition::AmputationIntended,
        why: "a GeForce GA106 has no NVLink; the caller handles the status itself with \
              NV_PRINTF(LEVEL_INFO, \"NVLink is unavailable\") (ogkm-580: \
              kernel_nvlink.c:1826-1830), and a real GA106's own GSP answers this control \
              0x56 too (C: mode2_initctrl_ga106.h:6251)",
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

/// ★★★ **The gate, as a function rather than only as a test** — every control whose
/// amputation this port has judged unsurvivable, and which nothing in
/// [`WantedTable::ALL`] serves.
///
/// A non-empty result is a port that will amputate an engine RM then dereferences. It is
/// computed from [`SWEEP_TRIAGE`] and [`WantedTable::ALL`] rather than restated, so neither
/// list can be shortened into agreement.
#[must_use]
pub fn unsurvivable_and_unserved() -> Vec<&'static SweepControl> {
    SWEEP_TRIAGE
        .iter()
        .filter(|c| c.disposition == SweepDisposition::AmputationUnsurvivable)
        .filter(|c| WantedTable::from_cmd(c.cmd).is_none())
        .collect()
}

kayfabe_util::assert_send_sync!(SweepControl, SweepDisposition);
