//! Axis-B seam: the **GSP-facing register model** for one GPU generation.
//!
//! `mode2_gsp_port_plan.md` §3.5 records the finding this module answers: [`Arch`]
//! exposed `mmu()`, `userd()` and `pushbuffer()` but **nothing that can express a
//! register offset, a WPR2 encoding or a falcon-boot convention** — so a faked-GSP boot
//! FSM had nowhere to put them except inside itself, which CLAUDE.md rule 1 forbids
//! (`kayfabe-gsp` is a logic crate: no generation name, no offset, no `#[repr(C)]`).
//!
//! The split this seam draws is the plan's §3.2 table, stated as a rule: **a register
//! whose served value is a function of the GSP boot FSM's state belongs here; every
//! other register does not.** The FSM names [`GspReg::Wpr2AddrHi`] and hands over a
//! [`GspObservation`]; the *encoding* — which offset, which bit, which sentinel — is the
//! implementation's, one per generation.
//!
//! ## ★★★ Two seams, not one (task #121, 2026-07-31)
//!
//! [`GspModel`] carries a generation's **values** — which offset, which bit, which
//! sentinel. That half worked from the start. It never carried a generation's
//! **sequence**, and the consequence was measured: the boot FSM matched [`GspReg`] arm by
//! arm and encoded the first generation's ordering in the match, so a second generation
//! could be added only by *editing the first one's implementation*.
//!
//! [`BootSequence`] is the missing half. A write is decoded by the model, handed to the
//! sequence, and comes back as [`BootStep`]s — an arch-independent vocabulary of eight
//! effects the FSM knows how to perform. The FSM's flow is now the same on every
//! generation; the *ordering* is the generation's, stated in its own implementation and
//! declared as data in [`BootSequence::stages`].
//!
//! ## Why the boot *sequence* is a parameter and not a protocol
//!
//! Because ogkm says so, generation by generation. `kgspBootstrap_TU102`
//! (`ogkm-610: src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_tu102.c:522-618`,
//! `ogkm-580: :493-578`) is the Turing→Ada regime: FWSEC, then boot-args in
//! `MAILBOX0/1`, then the SEC2 Booter Load that raises WPR2. **That ordering is identical
//! at both tags** — 610's only addition inside the function is a `kgspSendInitRpcs` call
//! after the Booter Load, which is not part of the register regime. Hopper+ replaces the
//! whole chain with FSP secure boot and a RISC-V BCR path, and *requires `MAILBOX0` to
//! read back 0* (`ogkm-610: .../hopper/kernel_gsp_gh100.c:248-263, 500-544, 730-776`;
//! `ogkm-580: :239-254, 491-535, 710-756` — the same three regions, the only textual
//! difference being that 610 names the CC-regkey mailbox index
//! `NV_PGSP_MAILBOX_REGISTER_CC_REGKEYS` where 580 writes `0`), while
//! `kgspIsWpr2Up_GH100` returns FALSE unconditionally under Confidential Compute
//! (`ogkm-610: :236-245`, `ogkm-580: :227-236`, byte-identical). A model that hard-coded
//! "mailbox0 carries the boot-args pointer" would be wrong on the next generation in a
//! way no test on this one could see.
//!
//! ## What is deliberately NOT here
//!
//! - **Interrupt delivery.** The plan sketched `fn status_queue_irq(&self) -> IrqSpec`,
//!   but `IrqSpec` lives in `kayfabe-vmm` and this crate does not depend on it. The FSM
//!   emits an abstract "announce the status queue" action instead and the device shell,
//!   which already owns the VMM vocabulary, chooses the delivery. Promoting `IrqSpec`
//!   (or `BarId`) into a crate both `kayfabe-arch` and `kayfabe-vmm` can see is an owner
//!   decision about the lattice, not a side effect of building the GSP port.
//! - **Every non-GSP register** (PTIMER, fuses, the PCI-config mirror, PRAMIN). Plan
//!   §11-O1, still open. The boundary proposed there is exactly the rule above.

/// The GSP-facing registers the boot FSM reacts to, or serves.
///
/// Abstract: **no offsets**. Each variant is one row of `mode2_gsp_port_plan.md` §3.2's
/// guest-observable table, or one of the writes §3.3's transitions trigger on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GspReg {
    /// GFW boot progress. The guest polls it for "complete"
    /// (`ogkm-610: src/nvidia/src/kernel/gpu/arch/turing/kern_gpu_tu102.c:391-479`,
    /// `ogkm-580: :381-469` — byte-identical, a pure 10-line shift).
    GfwBootProgress,
    /// The privilege-level mask guarding the progress scratch register; the guest
    /// requires it fully lowered before it trusts the progress value (same citation).
    GfwBootPlm,
    /// GSP falcon `CPUCTL`. Written to start the core; read for `HALTED`.
    GspFalconCpuctl,
    /// GSP falcon `HWCFG2` — carries `RISCV_ENABLE`.
    ///
    /// ★ **PHANTOM CORRECTED.** The citation here used to be an untagged `ogkm` pointer
    /// at `kernel_gsp_tu102.c:534-538`, which says this at **neither** tag — those lines
    /// are inside `kgspBootstrap_TU102`'s scrubber preamble at 610 and mid-function at
    /// 580, and `HWCFG2` is never read in
    /// `kernel_gsp_tu102.c` in either tree. The reader is `kflcnIsRiscvCpuEnabled_TU102`
    /// (`ogkm-610:`/`ogkm-580:`
    /// `src/nvidia/src/kernel/gpu/falcon/arch/turing/kernel_falcon_tu102.c:130-132` —
    /// same path and same lines at both tags), testing
    /// `NV_PFALCON_FALCON_HWCFG2_RISCV_ENABLE`, whose value `0x1` is declared in
    /// `ogkm-610:`/`ogkm-580:`
    /// `src/common/inc/swref/published/turing/tu102/dev_falcon_v4.h:64` and
    /// `.../ampere/ga102/dev_falcon_v4.h:103` — also identical at both.
    GspFalconHwcfg2,
    /// GSP falcon `DMATRFCMD` — the ucode-load DMA command/status.
    GspFalconDmatrfcmd,
    /// GSP falcon `MAILBOX0`. Carries the low half of the LibOS boot-args address on
    /// write (`kgspProgramLibosBootArgsAddr_TU102`:
    /// `ogkm-610: kernel_gsp_tu102.c:391-403`, `ogkm-580: :362-374`, byte-identical), and
    /// the suspend sentinel on read.
    ///
    /// ★★ **The sentinel is a VERSION SEAM, and it is the kind that fails silently.**
    ///
    /// | tag | `_kgspIsProcessorSuspended` | the predicate |
    /// |---|---|---|
    /// | `ogkm-580:` | `kernel_gsp_tu102.c:1225-1239` | `mailbox == 0x80000000` — **exact equality**, an inline literal; the symbol `INTERRUPT_PROCESSOR_SUSPENDED_VALUE` does not exist anywhere at 580 |
    /// | `ogkm-610:` | `kernel_gsp_tu102.c:335-349` | `(mailbox & INTERRUPT_PROCESSOR_SUSPENDED_VALUE) != 0`, the `#define` at `:333` |
    ///
    /// So an implementation that ORs the sentinel into the echoed `boot_args_lo` passes at
    /// 610 and makes the bench's own driver poll until `gpuTimeoutCondWait` expires — on
    /// the teardown path, where a hang is a wedged GPU. ★ **580 governs: `encode` must
    /// replace the whole `MAILBOX0` value with the sentinel, never OR it in.** (Note the
    /// old untagged citation to `kernel_gsp_tu102.c:333` is a *610* line number that
    /// happens to land on unrelated code at 580 — exactly the drift the tag rule exists
    /// for.)
    GspFalconMailbox0,
    /// GSP falcon `MAILBOX1` — the high half of the boot-args address.
    GspFalconMailbox1,
    /// GSP falcon `IRQSTAT`.
    GspFalconIrqstat,
    /// GSP falcon `IRQMASK`.
    GspFalconIrqmask,
    /// GSP falcon `IRQDEST`.
    GspFalconIrqdest,
    /// GSP falcon `IRQSCLR` — write-1-to-clear; the guest's ISR clears the edge before
    /// draining the status queue.
    GspFalconIrqsclr,
    /// The RISC-V core's `CPUCTL`, whose `ACTIVE` bit the guest's liveness gate reads
    /// (`kflcnIsRiscvActive_GA102`: `ogkm-610:`/`ogkm-580:`
    /// `src/nvidia/src/kernel/gpu/falcon/arch/ampere/kernel_falcon_ga102.c:53-55` — same
    /// path, same lines, byte-identical at both tags).
    GspRiscvCpuctl,
    /// ★★★★★ **§16.77 — `NV_PRISCV_RISCV_IRQMASK`, and it is one of the two registers
    /// that decide whether the guest's ISR believes its own interrupt.**
    ///
    /// `kgspService_TU102` opens with `kflcnGetPendingHostInterrupts`
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_tu102.c:1155`),
    /// which branches on `kflcnIsRiscvMode` (`kernel_falcon.c:84-90`):
    ///
    /// - falcon mode → `IRQSTAT & FALCON_IRQMASK & FALCON_IRQDEST`;
    /// - **RISC-V mode → `IRQSTAT & NV_PRISCV_RISCV_IRQMASK & NV_PRISCV_RISCV_IRQDEST`**
    ///   (`kflcnRiscvReadIntrStatus_GA102`: `kernel_falcon_ga102.c:311-321`).
    ///
    /// ⚠ **The mode is not probed at service time; it is LATCHED at bootstrap.**
    /// `kflcnResetIntoRiscv_GA102` calls `kflcnSetRiscvMode(…, NV_TRUE)` unconditionally
    /// (`kernel_falcon_ga102.c:84-95`), reached from the GSP boot sequencer's
    /// `GSP_SEQ_BUF_OPCODE_CORE_RESUME` (`kernel_gsp_ga102.c:161`), and
    /// `kflcnIsRiscvMode` caches the tristate forever after. So a GSP-offload adapter is
    /// in RISC-V mode by the time any interrupt is ever serviced, and `BCR_CTRL` is never
    /// consulted — which is why modelling `BCR_CTRL` would not have helped and is not done
    /// here (`C: src/qemu/nvkvm_gpu_emul.c:1562-1573` does not model it either).
    ///
    /// ⇒ Leaving this register undecoded made it read as an unclaimed **zero**, so
    /// `intrStatus` was `IRQSTAT & 0 & 0 = 0` no matter what `IRQSTAT` said, and the guest
    /// took the *"nothing to do"* exit at `kernel_gsp_tu102.c:1158-1162` **without writing
    /// `IRQSCLR` and without draining the status queue**. `[measured 2026-08-10, boot
    /// w212 at e309a85]`: `NVRM: nvAssertFailedNoLog: Assertion failed: KGSP service
    /// called when no KGSP interrupt pending @ kernel_gsp_tu102.c:1160`, with `0 IRQSCLR
    /// cleared` and `348 gated` in the same boot's device report.
    ///
    /// ⊘ **This is why "raised into a masked leaf" was the wrong diagnosis.** The guest's
    /// ISR *did* find the vector — reaching `kgspService` at all proves the interrupt
    /// tree scan attributed it to `MC_ENGINE_IDX_GSP` — and the stall scan does not
    /// consult `LEAF_EN` at all (`intrGetPendingStallEngines_TU102`:
    /// `intr_tu102.c:895-899`, *"We skip checking if it is enabled in the leaf register"*;
    /// `intrGetLeafStatus_TU102:1108-1141` reads `LEAF` raw).
    GspRiscvIrqmask,
    /// `NV_PRISCV_RISCV_IRQDEST` — the second half of the RISC-V AND. See
    /// [`GspReg::GspRiscvIrqmask`] for the whole story; the two are only separate because
    /// the register block is.
    GspRiscvIrqdest,
    /// SEC2 falcon `CPUCTL` — the Booter's start register.
    Sec2FalconCpuctl,
    /// SEC2 falcon `MAILBOX0` — the Booter's argument, and the only thing that
    /// distinguishes a Load from an Unload at our boundary.
    Sec2FalconMailbox0,
    /// SEC2 falcon `DMATRFCMD`.
    Sec2FalconDmatrfcmd,
    /// WPR2 region base, low half.
    ///
    /// ★★ **CORRECTED (2026-07-31): this register has TWO readers and they disagree
    /// about how much of it matters.** The doc that used to sit on the GA10x encoding
    /// said only zero-vs-nonzero was load-bearing and cited `kgspIsWpr2Up_TU102` — but
    /// that reader only ever looks at [`GspReg::Wpr2AddrHi`]. The *low* register's reader
    /// is `kgspExecuteFwsec_TU102`
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_frts_tu102.c:514-524`),
    /// and it is an **exact compare** against `frtsOffset >> ALIGNMENT` — the driver's own
    /// arithmetic over the **usable FB size**, read from `NV_USABLE_FB_SIZE_IN_MB`
    /// (`kmemsysReadUsableFbSize_GA102`,
    /// `ogkm-580: src/nvidia/src/kernel/gpu/mem_sys/arch/ampere/kern_mem_sys_ga102.c:48`).
    /// So this value is a *function of the advertised FB size*, not a free constant.
    ///
    /// ⚠ **That makes this variant a cross-plane dependency, and the FB-size register is
    /// deliberately NOT a [`GspReg`].** Its served value is not a function of the boot
    /// FSM's state — devinit publishes it before the driver looks — so this module's own
    /// rule puts it on the *other* side of the seam, with PTIMER and the fuses
    /// (`mode2_gsp_port_plan.md` §11-O1, still open). Whichever plane ends up owning it
    /// must share one FB-size constant with whatever serves this register, or the two
    /// disagree silently and the guest refuses to boot. `docs/design/gsp_boot_gate_spec.md`
    /// §1 gates G5.2/G6.3b carry the arithmetic.
    Wpr2AddrLo,
    /// WPR2 region base, high half. "WPR2 is up" is `_VAL != 0` on this register
    /// (`kgspIsWpr2Up_TU102`: `ogkm-610: kernel_gsp_tu102.c:1172-1180`,
    /// `ogkm-580: :1251-1261` — byte-identical).
    Wpr2AddrHi,
    /// The command-queue doorbell for queue `i`.
    ///
    /// ★ Indexed deliberately: `NV_PGSP_QUEUE_HEAD(i)` is an array
    /// (`ogkm-610:`/`ogkm-580: src/common/inc/swref/published/ampere/ga102/dev_gsp.h:38`
    /// — same path, same line, `(0x110c00+(i)*8)` with `__SIZE_1 = 8` at both tags), and the C
    /// artifact hard-codes queue 0 (`C: src/qemu/mode2_regs_ga10x.h:69`).
    GspQueueHead(u8),
}

/// How far the boot has got — **what is true**, never which register made it true.
///
/// ★★ This lives here, not in the FSM crate, because two different planes need it and
/// neither owns it: [`GspModel::encode`] needs it to answer a register whose *meaning*
/// changes with the stage, and [`BootSequence`] needs it to decide what the next write
/// means. It is the arch-independent half of the boot — a generation supplies the
/// sequence that walks it, not the vocabulary.
///
/// ★ **Renamed 2026-07-31, and the rename is the point.** The `ProtectedRegionUp` variant
/// used to be called `FwsecRan`, which is one firmware's name for one generation's way of
/// getting here: on the Turing→Ada regime the GSP falcon runs FWSEC and FWSEC raises the
/// protected region (`kgspExecuteFwsec_TU102`,
/// `ogkm-580: src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_frts_tu102.c:514-524`).
/// A later generation reaches the same *truth* with no FWSEC anywhere: the FSP raises
/// FRTS from a command payload (`kgspBootstrap_GH100` →`kfspSendBootCommands_HAL`,
/// `ogkm-580: src/nvidia/src/kernel/gpu/gsp/arch/hopper/kernel_gsp_gh100.c:851-869`).
/// Naming the stage after the first implementation to reach it is how one generation's
/// sequence becomes "the" sequence.
///
/// Ordered so that "the protected region is up" is a range test, exactly as
/// `mode2_gsp_port_plan.md` §3.2's observable table states it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum BootPhase {
    /// Power-on, or post-reset. WPR2 down.
    #[default]
    Cold,
    /// The protected region (WPR2/FRTS) is up and the processor reports active. The RM
    /// firmware is **not** yet loaded.
    ProtectedRegionUp,
    /// The RM firmware is loaded and running.
    Booted,
    /// `GSP_INIT_DONE` was posted **and drained by the guest**: the RPC plane is live and
    /// unsolicited events are allowed (§7-G7).
    Running,
    /// fn-47 has been serviced: `MAILBOX0` reports the suspend sentinel.
    Suspending,
    /// Torn down. WPR2 down, no binding.
    Halted,
}

impl BootPhase {
    /// Is WPR2 up in this phase? `phase >= ProtectedRegionUp && phase < Halted`
    /// (`ogkm-610: kernel_gsp_tu102.c:1172-1180`, `ogkm-580: :1252-1260` is the guest's own
    /// test — `kgspIsWpr2Up_TU102`, byte-identical at both tags — and
    /// `ogkm-610: kernel_gsp.c:4805-4809`, `ogkm-580: :3873-3877` is the boot gate that
    /// reads it, likewise identical). No version seam here.
    #[must_use]
    pub fn wpr2_up(self) -> bool {
        self >= BootPhase::ProtectedRegionUp && self < BootPhase::Halted
    }
}

/// The boot FSM's **abstract** state, as the register model needs to see it.
///
/// Every field is a fact the FSM knows; no field is an encoding. This is the *whole*
/// input to [`GspModel::encode`], which is what makes `mode2_gsp_port_plan.md` §3.2's
/// "nothing else may be a function of GSP state" checkable: a register whose value needs
/// something not in this struct is, by construction, not a GSP register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GspObservation {
    /// How far the boot has got.
    ///
    /// ★ Added 2026-07-31 for the second architecture, and it is not a convenience. The
    /// derived booleans below are a *Turing-regime* summary of the stage: they were
    /// enough while one generation existed, and they are not enough for a generation on
    /// which **one register means three different things at three different stages**.
    /// `NV_PFALCON_FALCON_HWCFG2` on GH100 is read for target-mask release
    /// (`_kfspIsGspTargetMaskReleased`, `ogkm-580:
    /// src/nvidia/src/kernel/gpu/fsp/arch/hopper/kern_fsp_gh100.c:1096-1114`), then for
    /// `RISCV_BR_PRIV_LOCKDOWN` (`_kgspLockdownReleasedOrFmcError`,
    /// `ogkm-580: kernel_gsp_gh100.c:489-535`) — where the Turing regime reads
    /// `RISCV_ENABLE` off the same offset (`kflcnIsRiscvCpuEnabled_TU102`,
    /// `ogkm-580: src/nvidia/src/kernel/gpu/falcon/arch/turing/kernel_falcon_tu102.c:130-132`).
    /// A model that cannot see the stage cannot serve all three.
    pub stage: BootPhase,
    /// WPR2 is up — true from the protected-region boot step until teardown.
    pub wpr2_up: bool,
    /// The RISC-V core reports active.
    pub riscv_active: bool,
    /// The processor has reported itself suspended (the teardown poll's answer).
    pub suspended: bool,
    /// A status-queue interrupt is latched and not yet cleared by the guest.
    pub swgen0_pending: bool,
    /// The low half of the boot-args address the guest last wrote, echoed back.
    pub boot_args_lo: u32,
    /// The high half of the boot-args address the guest last wrote, echoed back.
    pub boot_args_hi: u32,
}

/// The geometry of the LibOS memory-region init-args array the guest publishes.
///
/// Values are the driver's, not ours: the entry is
/// `{ LibosAddress id8; LibosAddress pa; LibosAddress size; NvU8 kind; NvU8 loc; }`
/// with the array's declared maximum of 4096 entries
/// (`ogkm-610:`/`ogkm-580: src/common/uproc/os/common/include/libos_init_args.h:31-56` —
/// the header is byte-identical at both tags, same path, same lines). The C artifact
/// caps its scan at **16** entries (`C: src/qemu/nvkvm_gpu_emul.c:3388-3407`), which is a
/// parameter it never wrote down as one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LibosRegionLayout {
    /// Bytes per array entry.
    pub entry_stride: usize,
    /// Byte offset of the 8-byte region id within an entry.
    pub id_offset: usize,
    /// Byte offset of the 8-byte physical address within an entry.
    pub pa_offset: usize,
    /// Byte offset of the 8-byte size within an entry.
    pub size_offset: usize,
    /// The upper bound on entries the array may declare.
    pub max_entries: usize,
    /// The id8 value of the region carrying `GSP_ARGUMENTS_CACHED` ("RMARGS").
    pub rmargs_id: u64,
}

// ══════════════════════════════════════════════════════════════════════════════════════
// The boot-SEQUENCE seam (task #121)
// ══════════════════════════════════════════════════════════════════════════════════════

/// One arch-independent effect a [`BootSequence`] asks the boot FSM to perform.
///
/// ★★★ **This enum is the whole seam.** Before it, the FSM matched [`GspReg`] arm by arm
/// and *encoded one generation's boot ordering in the match itself* — `Sec2FalconCpuctl`
/// meant "the firmware is now loaded", `GspQueueHead` meant "doorbell", and a generation
/// that reaches those same truths through different registers had nowhere to say so. The
/// FSM now knows only these eight effects; **which write produces which effect, in which
/// order, is the generation's answer**, and a second generation supplies it by adding a
/// [`BootSequence`] alongside rather than by editing the first one's.
///
/// The variants name what becomes *true*, never the register that did it. Where a
/// generation's mechanism is worth recording, it is in the variant's own docs as an
/// example — an implementation of the step, not its definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BootStep {
    /// The boot processor was started, and the protected region comes up with it.
    ///
    /// The FSM classifies it on the current stage — a cold start, the trailing teardown
    /// start, or a no-op — so a sequence does **not** need to know which of those it is.
    ///
    /// *Turing regime:* the GSP falcon `CPUCTL` STARTCPU that runs FWSEC.
    StartProcessor,
    /// The RM firmware is loaded and the processor is running it.
    ///
    /// *Turing regime:* the SEC2 Booter Load.
    FirmwareLoaded,
    /// The protected region comes down and this device life ends.
    ///
    /// *Turing regime:* the SEC2 Booter Unload.
    Teardown,
    /// Shadow the low half of the boot-args address.
    BootArgsLo(u32),
    /// Shadow the high half of the boot-args address.
    BootArgsHi(u32),
    /// The boot-args address is complete at this guest-physical address: walk the LibOS
    /// region array, bind the queues, publish the status header, post `GSP_INIT_DONE`.
    ///
    /// ★ The address is **carried in the step**, not recomputed by the FSM from the
    /// shadowed halves. That is deliberate: on the Turing regime it *is* the two halves,
    /// and on a generation whose boot-args pointer arrives inside a command payload
    /// written through a register window there are no halves to recompute it from.
    PublishBootArgs(u64),
    /// The guest rang the command-queue doorbell.
    CommandDoorbell,
    /// The guest acknowledged the status-queue interrupt edge.
    ClearStatusIrq,
}

impl BootStep {
    /// This step without its payload — what [`BootStageDesc`] declares and what a test
    /// compares an executed boot against.
    #[must_use]
    pub fn kind(self) -> BootStepKind {
        match self {
            BootStep::StartProcessor => BootStepKind::StartProcessor,
            BootStep::FirmwareLoaded => BootStepKind::FirmwareLoaded,
            BootStep::Teardown => BootStepKind::Teardown,
            BootStep::BootArgsLo(_) => BootStepKind::BootArgsLo,
            BootStep::BootArgsHi(_) => BootStepKind::BootArgsHi,
            BootStep::PublishBootArgs(_) => BootStepKind::PublishBootArgs,
            BootStep::CommandDoorbell => BootStepKind::CommandDoorbell,
            BootStep::ClearStatusIrq => BootStepKind::ClearStatusIrq,
        }
    }
}

/// [`BootStep`] with the payload dropped — the comparable identity of a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BootStepKind {
    /// See [`BootStep::StartProcessor`].
    StartProcessor,
    /// See [`BootStep::FirmwareLoaded`].
    FirmwareLoaded,
    /// See [`BootStep::Teardown`].
    Teardown,
    /// See [`BootStep::BootArgsLo`].
    BootArgsLo,
    /// See [`BootStep::BootArgsHi`].
    BootArgsHi,
    /// See [`BootStep::PublishBootArgs`].
    PublishBootArgs,
    /// See [`BootStep::CommandDoorbell`].
    CommandDoorbell,
    /// See [`BootStep::ClearStatusIrq`].
    ClearStatusIrq,
}

/// The steps one register write means — at most [`BootSteps::CAPACITY`] of them.
///
/// Inline rather than a `Vec` because this is on the MMIO path and because a bound that
/// exists is a bound a hostile guest cannot grow: [`BootSteps::push`] past the capacity is
/// a **refusal**, reported by [`BootSteps::overflowed`], never a silent drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BootSteps {
    steps: [Option<BootStep>; BootSteps::CAPACITY],
    n: u8,
    overflowed: bool,
}

impl BootSteps {
    /// The most steps one write may mean.
    ///
    /// Four, because the widest sequence in the tree needs three — a generation on which a
    /// single command-queue doorbell starts the processor, loads the firmware and hands
    /// over the boot-args pointer — and a capacity equal to the widest current user is a
    /// bound that has never been *seen* to hold.
    pub const CAPACITY: usize = 4;

    /// No steps: this write means nothing on this generation.
    #[must_use]
    pub fn none() -> BootSteps {
        BootSteps::default()
    }

    /// One step.
    #[must_use]
    pub fn one(step: BootStep) -> BootSteps {
        let mut s = BootSteps::none();
        s.push(step);
        s
    }

    /// Append a step. Past [`BootSteps::CAPACITY`] the step is dropped and
    /// [`BootSteps::overflowed`] latches true.
    pub fn push(&mut self, step: BootStep) {
        if usize::from(self.n) >= BootSteps::CAPACITY {
            self.overflowed = true;
            return;
        }
        self.steps[usize::from(self.n)] = Some(step);
        self.n += 1;
    }

    /// Did a `push` exceed the capacity? The FSM turns this into a named refusal rather
    /// than executing a truncated sequence.
    #[must_use]
    pub fn overflowed(self) -> bool {
        self.overflowed
    }

    /// How many steps.
    #[must_use]
    pub fn len(self) -> usize {
        usize::from(self.n)
    }

    /// No steps at all.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.n == 0
    }

    /// The steps, in order.
    pub fn iter(self) -> impl Iterator<Item = BootStep> {
        (0..usize::from(self.n)).filter_map(move |i| self.steps[i])
    }
}

impl IntoIterator for BootSteps {
    type Item = BootStep;
    type IntoIter = core::iter::Flatten<core::array::IntoIter<Option<BootStep>, 4>>;
    fn into_iter(self) -> Self::IntoIter {
        self.steps.into_iter().flatten()
    }
}

/// One named stage of a generation's boot, and the step that reaches it.
///
/// ★ **Data, so a port can be read and diffed without executing it** — and so a test can
/// drive the generation's own registers and check the steps it emits arrive in the
/// declared order. A stage list nothing compares against is decoration; this one is
/// compared, which is why the step is a field rather than prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootStageDesc {
    /// What the stage is, in this generation's own words.
    pub name: &'static str,
    /// The step that reaches it.
    pub step: BootStepKind,
}

/// What a [`BootSequence`] may look at when deciding what a write means.
///
/// Everything here is a fact the FSM already knows. A sequence that needs something not
/// in this struct is asking the FSM for state it does not model, and the answer is to
/// widen this struct deliberately — not to latch a shadow copy in the adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BootContext {
    /// The registers' current answers, including [`GspObservation::stage`].
    pub obs: GspObservation,
    /// Which halves of the boot-args address have been shadowed **since the last publish
    /// or teardown** — the pair-completion term, which is protocol state the FSM
    /// maintains and the sequence only reads.
    pub boot_args_seen: (bool, bool),
}

/// Generation-local boot state the FSM **stores and never interprets**.
///
/// ★★ The third part of the shape, after the methods and the flow: *data*. A sequence is
/// `&self` — one immutable value shared by every vCPU thread, like every other Axis-B
/// seam (decision #17) — so it cannot hold a latch of its own. Anything it must remember
/// between writes lives here, inside the FSM's value, which means
/// `GspFsm::device_reset` clears it for free and a generation cannot leak state across a
/// device life by forgetting to.
///
/// Two shapes, because two are enough for both generations in the tree and neither
/// subsumes the other:
///
/// - **latches** — a few scalars. *Turing regime:* the SEC2 Booter argument, which is the
///   only thing distinguishing a Load from an Unload and arrives one write before the
///   `CPUCTL` that acts on it (`C: src/qemu/nvkvm_gpu_emul.c:4222-4234`). This used to be
///   a named `sec2_mailbox0` field **on the FSM**, i.e. one generation's convention
///   sitting in the arch-independent value.
/// - **a window** — a byte buffer a generation streams a command payload through.
///   *FSP regime:* `NV_PFSP_EMEMD` is a single register the driver writes a whole
///   command packet through, auto-incrementing an offset held in `NV_PFSP_EMEMC`
///   (`ogkm-580: src/nvidia/src/kernel/gpu/fsp/arch/hopper/kern_fsp_gh100.c:600-645`).
///
/// The FSM reads neither. It stores them, resets them, and compares them for equality.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ArchBootState {
    latches: [u64; ArchBootState::LATCHES],
    window: Vec<u8>,
}

impl ArchBootState {
    /// How many scalar latches a generation may hold.
    pub const LATCHES: usize = 4;

    /// The hard cap on the window, in bytes.
    ///
    /// A guest chooses the offset it writes at, so this is an attacker-controlled index
    /// and the cap is a **security** bound, not a capacity estimate. 4 KiB is four times
    /// the largest command channel in the tree (`FSP_EMEM_CHANNEL_RM_SIZE` = 1024,
    /// `ogkm-580: src/nvidia/arch/nvalloc/common/inc/fsp/fsp_emem_channels.h:37`).
    pub const WINDOW_LIMIT: usize = 4096;

    /// Read a latch. Out of range reads zero — a slot that does not exist holds nothing.
    #[must_use]
    pub fn latch(&self, slot: usize) -> u64 {
        self.latches.get(slot).copied().unwrap_or(0)
    }

    /// Write a latch. Out of range is a no-op.
    pub fn set_latch(&mut self, slot: usize, value: u64) {
        if let Some(l) = self.latches.get_mut(slot) {
            *l = value;
        }
    }

    /// Write one little-endian `u32` into the window at a **byte** offset.
    ///
    /// Returns `false` — and writes nothing — if the offset is not 4-byte aligned or the
    /// write would cross [`ArchBootState::WINDOW_LIMIT`]. The window grows to fit,
    /// zero-filled, so a guest that writes only the tail still reads defined zeros before
    /// it.
    pub fn window_write_u32(&mut self, byte_off: usize, value: u32) -> bool {
        if !byte_off.is_multiple_of(4) || byte_off.saturating_add(4) > ArchBootState::WINDOW_LIMIT {
            return false;
        }
        if self.window.len() < byte_off + 4 {
            self.window.resize(byte_off + 4, 0);
        }
        self.window[byte_off..byte_off + 4].copy_from_slice(&value.to_le_bytes());
        true
    }

    /// Read one little-endian `u32` back, or `None` if the window never reached there.
    #[must_use]
    pub fn window_read_u32(&self, byte_off: usize) -> Option<u32> {
        let end = byte_off.checked_add(4)?;
        let bytes: [u8; 4] = self.window.get(byte_off..end)?.try_into().ok()?;
        Some(u32::from_le_bytes(bytes))
    }

    /// Read one little-endian `u64` back, or `None` if the window never reached there.
    #[must_use]
    pub fn window_read_u64(&self, byte_off: usize) -> Option<u64> {
        let lo = self.window_read_u32(byte_off)?;
        let hi = self.window_read_u32(byte_off + 4)?;
        Some((u64::from(hi) << 32) | u64::from(lo))
    }

    /// How many bytes of the window the guest has reached.
    #[must_use]
    pub fn window_len(&self) -> usize {
        self.window.len()
    }
}

/// One trapped register write, as a [`BootSequence`] sees it.
///
/// A struct rather than four parameters because the *set* is the interesting thing: a
/// sequence gets the raw `(bar, off, val)` **and** the shared vocabulary's opinion of it,
/// and either half may be the one that matters. Widening it later is a field, not a
/// signature change every implementation has to follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegWrite {
    /// The raw PCI BAR index.
    pub bar: u8,
    /// The byte offset within that BAR.
    pub off: u64,
    /// The value written.
    pub val: u64,
    /// [`GspModel::decode_reg`]'s answer — `None` for an offset the shared vocabulary
    /// cannot name, which is **not** a dead case: a generation whose boot is driven by
    /// registers no other generation has sees its own offsets here and nowhere else.
    pub reg: Option<GspReg>,
}

/// Axis-B: **the boot ordering** for one GPU generation — which register writes drive
/// which transitions, and in what order.
///
/// ★★★ The sibling of [`GspModel`], and the split between them is the finding this seam
/// is built on: the register model carries a generation's **values** (offsets, bit
/// positions, sentinels) and it always did; nothing carried a generation's **sequence**,
/// so the first generation's sequence lived in the FSM's `match` and a second one could
/// only be added by editing it.
///
/// `Send + Sync` for the same reason [`GspModel`] is: one immutable value, shared by
/// every vCPU thread. Everything mutable is in [`ArchBootState`].
pub trait BootSequence: Send + Sync {
    /// The stages this generation's boot passes through, **in order**.
    ///
    /// Declarative, and compared against an executed boot rather than trusted.
    fn stages(&self) -> &'static [BootStageDesc];

    /// What this register write means on this generation.
    ///
    /// [`RegWrite`] carries both the raw offset and the shared vocabulary's opinion of it;
    /// a generation whose boot is driven by an FSP command queue or an EMEM window reads
    /// the former, and one inside the vocabulary reads the latter.
    ///
    /// `model` is this generation's own register model, passed back so a sequence can use
    /// its predicates (`is_startcpu`, `is_booter_unload`, `is_swgen0_clear`) without
    /// hard-coding a bit position: the *values* stay in the model, the *ordering* is
    /// here.
    fn on_write(
        &self,
        model: &dyn GspModel,
        w: &RegWrite,
        ctx: &BootContext,
        state: &mut ArchBootState,
    ) -> BootSteps;

    /// Serve a **generation-local** register read — one whose value is a function of boot
    /// state but which the shared [`GspReg`] vocabulary cannot name.
    ///
    /// Consulted only after [`GspModel::decode_reg`] has declined, so it can never shadow
    /// a shared register. `None` means the offset is not this generation's either, and
    /// the caller must **not** default it to zero.
    fn on_read(
        &self,
        _model: &dyn GspModel,
        _bar: u8,
        _off: u64,
        _ctx: &BootContext,
        _state: &ArchBootState,
    ) -> Option<u64> {
        None
    }
}

/// **A generation whose boot ordering has not been implemented**, said out loud.
///
/// ★ The alternative to this type is a `BootSequence` default on [`GspModel`], and the
/// default would have to be *some* generation's ordering — which is how one generation's
/// sequence silently becomes the shape every other generation is a deviation from. A
/// model whose registers are mapped but whose boot is not yet written selects this
/// instead: it declares **no** stages and answers **no** step, so the FSM stays in
/// [`BootPhase::Cold`] and the gap is visible in a test rather than hidden behind a
/// borrowed boot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoBootSequence;

impl BootSequence for NoBootSequence {
    /// None. Not "the same as the neighbour's" — none.
    fn stages(&self) -> &'static [BootStageDesc] {
        &[]
    }

    fn on_write(
        &self,
        _model: &dyn GspModel,
        _w: &RegWrite,
        _ctx: &BootContext,
        _state: &mut ArchBootState,
    ) -> BootSteps {
        BootSteps::none()
    }
}

/// Axis-B: the GSP register model for one GPU generation.
///
/// `Send + Sync` for the same reason every other Axis-B seam is (decision #17): the
/// composition root stores one `Box<dyn Arch>` that vCPU threads share, and an encoding
/// table is immutable.
pub trait GspModel: Send + Sync {
    /// Which GSP register a trapped access names, if any. `None` = not ours, and the
    /// caller must **not** treat that as a default value — it means a different model
    /// owns the offset.
    ///
    /// `bar` is the raw PCI BAR index. It is a `u8` rather than one of the repo's two
    /// existing BAR newtypes (`kayfabe_vmm::BarId`, `kayfabe_trace::Bar`) because
    /// unifying them means giving `kayfabe-arch` a dependency it does not have today;
    /// see this module's docs.
    fn decode_reg(&self, bar: u8, off: u64) -> Option<GspReg>;

    /// Does this written value mean STARTCPU on a falcon `CPUCTL`?
    fn is_startcpu(&self, value: u64) -> bool;

    /// Do these SEC2 Booter arguments mean *Unload* — i.e. WPR2 must come down?
    ///
    /// A predicate rather than a constant on purpose: on GA10x the C distinguishes a
    /// normal Unload from a Load solely by `SEC2 MAILBOX0 == 0xff`
    /// (`C: src/qemu/nvkvm_gpu_emul.c:4222-4234`), and that is a generation-local
    /// convention, not a protocol.
    fn is_booter_unload(&self, sec2_mailbox0: u32) -> bool;

    /// Does this written value clear the status-queue interrupt edge?
    ///
    /// Write-1-to-clear on the falcon's `IRQSCLR`, bit 6 on GA10x
    /// (`C: src/qemu/nvkvm_gpu_emul.c:4193-4200`) — a bit position, hence a predicate
    /// rather than a mask the caller applies. (Not in the plan's §3.5 sketch: the FSM
    /// needs it for transition E10, and hard-coding bit 6 in a logic crate is exactly
    /// what this seam exists to prevent.)
    fn is_swgen0_clear(&self, value: u64) -> bool;

    /// The value to serve for `reg` given the FSM's abstract state.
    ///
    /// This is where WPR2 geometry, the HALTED/ACTIVE bit positions and the suspend
    /// sentinel live. A register this model decodes but cannot serve returns `None`,
    /// which the caller reports as a fault rather than as zero.
    fn encode(&self, reg: GspReg, obs: &GspObservation) -> Option<u64>;

    /// The LibOS region-array geometry this driver regime publishes.
    fn libos_region_layout(&self) -> LibosRegionLayout;

    /// **The boot ordering this generation follows.**
    ///
    /// ⊘ **Deliberately not defaulted.** A default would have to be *some* generation's
    /// sequence, and whichever one it was would silently become the shape every other
    /// generation is a deviation from — which is the exact defect this seam was built to
    /// remove. Every generation states its own, even when the answer is "the same regime
    /// as the one next door": saying so is one line, and inheriting it by omission is
    /// unreadable.
    fn boot_sequence(&self) -> &dyn BootSequence;
}

kayfabe_util::assert_send_sync!(
    GspReg,
    GspObservation,
    LibosRegionLayout,
    BootPhase,
    BootStep,
    BootStepKind,
    BootSteps,
    BootStageDesc,
    BootContext,
    RegWrite,
    ArchBootState,
    NoBootSequence,
    dyn GspModel,
    dyn BootSequence
);
