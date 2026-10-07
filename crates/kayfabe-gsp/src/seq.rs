//! Boot **sequences** — implementations of [`BootSequence`], not the shape of one.
//!
//! ## ★★★ What this module is, and what it is deliberately not
//!
//! `mmio_write` used to match [`GspReg`] arm by arm, and those arms *were* one
//! generation's boot ordering: `Sec2FalconCpuctl` meant "the RM firmware is now loaded",
//! `GspQueueHead` meant "doorbell", the mailbox pair meant "publish". That is a real and
//! correct description of the falcon/secure-booter regime — the defect was never that it
//! existed. **The defect was that it was unhooked**: it sat in the FSM's own flow, so a
//! generation that reaches those same truths through different registers could only be
//! added by editing it.
//!
//! So the ordering moved out of the flow and into a value. [`FalconSecureBooterBoot`]
//! below is that regime, unchanged in behaviour, now stated as *one implementation of*
//! [`BootSequence`] that a generation opts into by name. A generation outside the regime
//! writes its own, and the FSM does not change.
//!
//! ## Why this regime's implementation lives in a logic crate, and the next one does not
//!
//! It carries **no offsets and no chip facts**. It is written entirely in [`GspReg`]
//! — the shared vocabulary — and in [`GspModel`]'s predicates, so every value it depends
//! on (which bit is STARTCPU, which argument means Unload, which bit clears the status
//! edge) comes from the generation's own model at call time. That is what makes it
//! *shareable* between generations rather than *privileged*: two generations select it,
//! neither is defaulted into it, and nothing in it names either.
//!
//! A generation whose boot is driven by registers the shared vocabulary cannot name has
//! nothing to gain from living here — its offsets are the whole content — so it lives
//! with its offsets, in the adapter crate. The asymmetry is the vocabulary's, not a
//! ranking.

use kayfabe_arch::gsp::{
    ArchBootState, BootContext, BootPhase, BootSequence, BootStageDesc, BootStep, BootStepKind,
    BootSteps, GspModel, GspReg, RegWrite,
};

/// Latch slot holding the secure-booter argument — see [`FalconSecureBooterBoot`].
const LATCH_BOOTER_ARG: usize = 0;

/// The **falcon + secure-booter** boot regime.
///
/// One implementation of [`BootSequence`], selected by a generation, never defaulted to.
/// In NVIDIA's own generated HAL this regime spans `TU102…AD107` for every function the
/// shared [`GspReg`] vocabulary models
/// (`ogkm-580: src/nvidia/generated/g_gpu_nvoc.c:2374-2385`), which is why two adapter
/// crates name it and neither duplicates it.
///
/// The ordering, in the driver's words
/// (`kgspBootstrap_TU102`: `ogkm-580: src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_tu102.c:493-578`,
/// `ogkm-610: :522-618` — the ordering is identical at both tags):
///
/// 1. the GSP falcon `CPUCTL` STARTCPU that runs FWSEC, which raises the protected region;
/// 2. the LibOS boot-args address into the falcon's `MAILBOX0`/`MAILBOX1`
///    (`kgspProgramLibosBootArgsAddr_TU102`, `ogkm-580: :362-374`);
/// 3. the SEC2 Booter Load, which starts the RM firmware.
///
/// ⚠ The FSM's *own* ordering constraint is looser than that list and deliberately so:
/// the boot-args pair publishes from either [`BootPhase::ProtectedRegionUp`] **or**
/// [`BootPhase::Booted`], because the guest programs the mailboxes at step 2 *and* again
/// on a resume path (`ogkm-580: kernel_gsp_tu102.c:933`,
/// `src/nvidia/src/kernel/gpu/gsp/arch/ampere/kernel_gsp_ga102.c:162`), and refusing the
/// second one would strand a re-acquiring driver.
///
/// # State
///
/// One latch, [`LATCH_BOOTER_ARG`]: the secure-booter argument, written to SEC2's
/// `MAILBOX0` one write before the `CPUCTL` that acts on it, and the only thing that
/// distinguishes a Load from an Unload at our boundary
/// (`C: src/qemu/nvkvm_gpu_emul.c:4222-4234`). It used to be a `sec2_mailbox0` field on
/// the FSM itself — one regime's convention living in the arch-independent value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FalconSecureBooterBoot;

impl FalconSecureBooterBoot {
    /// The regime.
    #[must_use]
    pub fn new() -> FalconSecureBooterBoot {
        FalconSecureBooterBoot
    }
}

/// See [`BootSequence::stages`] — the cold-boot stages of this regime, in order.
const FALCON_BOOTER_STAGES: &[BootStageDesc] = &[
    BootStageDesc {
        name: "GSP falcon STARTCPU runs FWSEC and raises the protected region",
        step: BootStepKind::StartProcessor,
    },
    BootStageDesc {
        name: "SEC2 Booter Load starts the RM firmware",
        step: BootStepKind::FirmwareLoaded,
    },
    BootStageDesc {
        name: "LibOS boot-args address, low half, into the GSP falcon MAILBOX0",
        step: BootStepKind::BootArgsLo,
    },
    BootStageDesc {
        name: "LibOS boot-args address, high half, into MAILBOX1",
        step: BootStepKind::BootArgsHi,
    },
    BootStageDesc {
        name: "the completed pair publishes the message queues",
        step: BootStepKind::PublishBootArgs,
    },
];

impl BootSequence for FalconSecureBooterBoot {
    fn stages(&self) -> &'static [BootStageDesc] {
        FALCON_BOOTER_STAGES
    }

    fn on_write(
        &self,
        model: &dyn GspModel,
        w: &RegWrite,
        ctx: &BootContext,
        state: &mut ArchBootState,
    ) -> BootSteps {
        let val = w.val;
        // An offset the shared vocabulary cannot name means nothing in this regime: every
        // write it reacts to has a `GspReg`. A generation outside the regime is exactly
        // the case where that is false, and it does not use this sequence.
        let Some(reg) = w.reg else {
            return BootSteps::none();
        };
        match reg {
            GspReg::GspFalconCpuctl if model.is_startcpu(val) => {
                BootSteps::one(BootStep::StartProcessor)
            }
            GspReg::Sec2FalconMailbox0 => {
                // Latched, not acted on: Load and Unload differ only by this argument.
                state.set_latch(LATCH_BOOTER_ARG, val);
                BootSteps::none()
            }
            GspReg::Sec2FalconCpuctl if model.is_startcpu(val) => {
                #[allow(clippy::cast_possible_truncation)]
                let arg = state.latch(LATCH_BOOTER_ARG) as u32;
                if model.is_booter_unload(arg) {
                    // The Booter Unload. WPR2 must read down afterwards, and the guest
                    // asserts exactly that
                    // (`ogkm-610: kernel_gsp_booter_tu102.c:175-187`, `ogkm-580: :179-191`
                    // — identical code at both tags, including the GC6 arm that requires
                    // WPR2 to stay *up*).
                    BootSteps::one(BootStep::Teardown)
                } else if ctx.obs.stage == BootPhase::ProtectedRegionUp {
                    BootSteps::one(BootStep::FirmwareLoaded)
                } else {
                    BootSteps::none()
                }
            }
            GspReg::GspFalconMailbox0 | GspReg::GspFalconMailbox1 => {
                #[allow(clippy::cast_possible_truncation)]
                let half = val as u32;
                let (mut lo, mut hi) = (ctx.obs.boot_args_lo, ctx.obs.boot_args_hi);
                let (mut lo_seen, mut hi_seen) = ctx.boot_args_seen;
                let mut steps = BootSteps::none();
                if reg == GspReg::GspFalconMailbox0 {
                    lo = half;
                    lo_seen = true;
                    steps.push(BootStep::BootArgsLo(half));
                } else {
                    hi = half;
                    hi_seen = true;
                    steps.push(BootStep::BootArgsHi(half));
                }
                // ★ [inferred] I4, taking the plan's own robust reading: the pair is
                // complete when **both halves have been written since the last reset**,
                // not when a particular half arrives. The C keys on MAILBOX1
                // (`C:4298-4302`) and ogkm writes lo then hi
                // (`kgspProgramLibosBootArgsAddr_TU102`,
                // `ogkm-610: kernel_gsp_tu102.c:392-403`, `ogkm-580: :363-374` — identical
                // at both tags) — a write *order*, not a protocol guarantee, so keying on
                // one half would be an assumption where a conjunction costs nothing.
                if lo_seen
                    && hi_seen
                    && matches!(
                        ctx.obs.stage,
                        BootPhase::ProtectedRegionUp | BootPhase::Booted
                    )
                {
                    steps.push(BootStep::PublishBootArgs(
                        (u64::from(hi) << 32) | u64::from(lo),
                    ));
                }
                steps
            }
            GspReg::GspQueueHead(_) => BootSteps::one(BootStep::CommandDoorbell),
            // The guest's ISR clears the edge before draining the queue
            // (`C: src/qemu/nvkvm_gpu_emul.c:4193-4200`).
            GspReg::GspFalconIrqsclr if model.is_swgen0_clear(val) => {
                BootSteps::one(BootStep::ClearStatusIrq)
            }
            _ => BootSteps::none(),
        }
    }
}

kayfabe_util::assert_send_sync!(FalconSecureBooterBoot);
