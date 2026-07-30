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

/// The boot FSM's **abstract** state, as the register model needs to see it.
///
/// Every field is a fact the FSM knows; no field is an encoding. This is the *whole*
/// input to [`GspModel::encode`], which is what makes `mode2_gsp_port_plan.md` §3.2's
/// "nothing else may be a function of GSP state" checkable: a register whose value needs
/// something not in this struct is, by construction, not a GSP register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GspObservation {
    /// WPR2 is up — true from the FWSEC/Booter-Load boot step until teardown.
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
}

kayfabe_util::assert_send_sync!(GspReg, GspObservation, LibosRegionLayout, dyn GspModel);
