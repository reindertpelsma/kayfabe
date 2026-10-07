//! ★★★ **The second-GPU-generation measurement** — the claim tested by making the edit.
//!
//! ## The question, and how it was reframed
//!
//! Three places in this repository's prose used to say that adding a GPU generation costs
//! an `impl Arch` in an adapter crate and **zero edits to a logic crate**. Nobody had ever
//! added a second one, so the claim asserted a measurement it did not have. This file and
//! `crates/kayfabe-chips` are that measurement.
//!
//! #118 (2026-07-30) took it and found the claim false: `GspFsm::mmio_write` matched
//! `GspReg` arm by arm and *encoded the falcon/secure-booter boot ordering in the match*,
//! so a generation booting through an FSP command queue needed edits to `kayfabe-arch`
//! and `kayfabe-gsp`, both logic crates.
//!
//! ★★★ **The owner reframed it, and the reframe is what this file now measures.**
//! Arch-independent logic stays arch-independent — that half held and is not in question.
//! But *"for a series of archs you can't have code that's only for those archs"* is not
//! true: such code is legitimate and must exist somewhere. It just must not be in core
//! logic; it must be **an implementation**, and the implementation must be constructed so
//! that any arch can hook or override it. So the criterion is not
//!
//! ⊘ *"adding an arch costs zero logic-crate edits"*
//!
//! but
//!
//! ★ **"a new arch can be implemented by ADDING ALONGSIDE, without MODIFYING the existing
//! arch's implementation."**
//!
//! ## The three generations, and why all three are here
//!
//! - **AD10x (Ada)** — the confirming case, and **weak alone**: NVIDIA's own generated HAL
//!   binds the Turing/Ampere implementations across `TU102…AD107`
//!   (`ogkm-580: src/nvidia/generated/g_gpu_nvoc.c:2374-2385`), so for the registers
//!   `GspReg` models Ada is byte-identical to GA10x. ⚠ An experiment that selects the
//!   easiest member of its universe produces a green with no red available to it.
//! - **GH100 (Hopper)** — the hard case: a different boot ordering, driven by registers no
//!   `GspReg` variant names. It is where the seam was built and where it is measured.
//! - **GA10x** — the north star, not tested here but by the whole `gsp_boot` suite. It must
//!   keep booting, bit-identically.
//!
//! ## What this file deliberately does NOT test
//!
//! It does not claim Hopper *would* boot on silicon — nothing here has touched Hopper
//! hardware and `Gh100FspBoot` documents its own limits. It measures one thing: whether a
//! second boot ordering can be **added** rather than **merged into** the first.

use kayfabe_arch::Arch;
use kayfabe_arch::gsp::{
    ArchBootState, BootContext, BootPhase, BootStepKind, GspModel, GspObservation, GspReg, RegWrite,
};
use kayfabe_chips::{Ad10xArch, Ad10xGspModel, Gh100Arch, Gh100GspModel};
use kayfabe_device::ga10x::Ga10xGspModel;
use kayfabe_gsp::{BootPhase as FsmPhase, EchoOk, GspFsm, QueueState, Transition};
use kayfabe_tests::gspworld::{FakeRam, GspWorld, MODEL_A, P580};

/// The FSM under test, built from the production version table. Axis A is a parameter,
/// Axis B is the argument to each `mmio_*` call — neither is a constant here.
fn fsm() -> GspFsm {
    GspFsm::new(P580.abi())
}

// ─────────────────────────── AD10x: the confirming case ───────────────────────────

/// ★ **AD10x boots the UNMODIFIED FSM.** The whole cost of this generation was a struct
/// in an adapter crate and a `VBIOS_PROFILES` row.
#[test]
fn a_second_generation_boots_the_unmodified_fsm() {
    let arch = Ad10xArch::new();
    let mut f = fsm();
    let mut ram = FakeRam::default();
    let mut policy = EchoOk;

    let (bar, off) = Ad10xGspModel::at(GspReg::GspFalconCpuctl).expect("Ada has a GSP CPUCTL");
    let r = f
        .mmio_write(&mut ram, &arch, &mut policy, bar, off, 0x2)
        .expect("STARTCPU is serviceable");
    assert_eq!(r.transitions, vec![Transition::E1], "FWSEC ran");
    assert_eq!(f.phase(), FsmPhase::ProtectedRegionUp);

    let (bar, off) = Ad10xGspModel::at(GspReg::Sec2FalconMailbox0).expect("Ada has a SEC2 mailbox");
    f.mmio_write(&mut ram, &arch, &mut policy, bar, off, 0x0)
        .expect("latching the Booter argument is serviceable");
    let (bar, off) = Ad10xGspModel::at(GspReg::Sec2FalconCpuctl).expect("Ada has a SEC2 CPUCTL");
    let r = f
        .mmio_write(&mut ram, &arch, &mut policy, bar, off, 0x2)
        .expect("the Booter Load is serviceable");
    assert_eq!(r.transitions, vec![Transition::E5], "Booter Load");
    assert_eq!(f.phase(), FsmPhase::Booted);
}

/// ★ **The second generation's identity is a real table row**, not a synthetic one: the
/// VBIOS generator produces an image for it through the same path GA106 uses.
#[test]
fn the_second_generations_rom_comes_from_the_shipped_table() {
    use kayfabe_abi::vbios::{VbiosWire, build, profile_for_device_id};

    let ada = profile_for_device_id(0x2803).expect("the AD106 row is in VBIOS_PROFILES");
    assert_eq!(ada.name, "AD106");
    let img = build(ada, VbiosWire::Tu102Bit).expect("the AD106 row builds an image");

    let ga = profile_for_device_id(0x2504).expect("the GA106 row is in VBIOS_PROFILES");
    let ga_img = build(ga, VbiosWire::Tu102Bit).expect("the GA106 row builds an image");

    assert_ne!(
        img, ga_img,
        "the two rows must not produce byte-identical ROMs — if they do, the device id \
         is not reaching the image and the table is decorative"
    );
}

/// ★ **Non-vacuity for the whole file**: the two models are genuinely different objects,
/// so a test that "passed on Ada and failed on Hopper" is not passing on one model twice.
#[test]
fn the_two_new_generations_are_distinguishable() {
    assert_ne!(Ad10xArch::new().name(), Gh100Arch::new().name());
    // Ada answers a register Hopper has no offset for. That single asymmetry is the
    // entire refutation, in one line.
    assert!(Ad10xGspModel::at(GspReg::GfwBootProgress).is_some());
    assert!(Gh100GspModel::at(GspReg::GfwBootProgress).is_none());
}

// ─────────────────────── GH100: the hard case, now IMPLEMENTED ────────────────────

/// The FSP registers GH100's boot is driven by, and the offset of the boot-args pointer
/// inside its command payload.
///
/// ⊘ Spelled as **literals here**, deliberately: reading `kayfabe_chips::gh100`'s own
/// constants back at itself would make every assertion below pass against whatever value
/// the implementation happened to hold. The sourcing lives in that module; these are the
/// numbers a real driver puts on the wire.
mod gh100_wire {
    /// `NV_PFSP_EMEMC(0)` (`ogkm-580: hopper/gh100/dev_fsp_pri.h:26`).
    pub const EMEMC: u64 = 0x008F_2AC0;
    /// `NV_PFSP_EMEMD(0)` (`ogkm-580: dev_fsp_pri.h:40`).
    pub const EMEMD: u64 = 0x008F_2AC4;
    /// `NV_PFSP_QUEUE_HEAD(0)` (`ogkm-580: dev_fsp_pri.h:53`).
    pub const QUEUE_HEAD: u64 = 0x008F_2C00;
    /// `NV_PFSP_QUEUE_TAIL(0)` (`ogkm-580: dev_fsp_pri.h:57`).
    pub const QUEUE_TAIL: u64 = 0x008F_2C04;
    /// 2 DWORDs of MCTP+NVDM header, then `NVDM_PAYLOAD_COT.gspBootArgsSysmemOffset` at
    /// +852 (`ogkm-580: kern_fsp.c:51-52,488-506`, `kern_fsp_cot_payload.h:36-55`).
    pub const BOOT_ARGS_OFF: u64 = 860;

    /// `NV_PFSP_EMEMC` with `OFFS`/`BLK` set for a byte offset, and `AINCW` set — `OFFS`
    /// is `7:2`, `BLK` is `15:8`, `AINCW` is `24:24`, 64 DWORDs per block
    /// (`ogkm-580: dev_fsp_pri.h:28,30,32`; `kern_fsp_gh100.c:57`).
    #[must_use]
    pub fn ememc_at(byte_off: u64) -> u64 {
        let dw = byte_off / 4;
        ((dw / 64) << 8) | ((dw % 64) << 2) | (1 << 24)
    }
}

/// ★★★ **THE CONVERTED PINNING TEST — and here is exactly what it used to assert.**
///
/// It was `the_boot_fsm_cannot_be_driven_past_fwsec_on_that_generation`, and it asserted
/// the **status quo defect**: after one shared GSP-falcon STARTCPU, *no* write to *any*
/// offset `Gh100GspModel` decoded could advance the phase, because the writes carrying the
/// next transitions had no register on this generation. Its own failure message said so:
/// *"THE SEAM NOW CARRIES THE HOPPER BOOT SEQUENCE … this test must be rewritten to the
/// new behaviour — do NOT widen it."*
///
/// The seam now carries it, so this is that rewrite. It asserts the **new correct thing**:
/// GH100 boots through its **own** sequence, driven by its **own** registers, reaching the
/// same stages by a different route — and it is the same kind of test, one that goes red
/// if the behaviour changes, not a widened one that passes either way.
#[test]
fn gh100_boots_through_its_own_sequence_not_the_falcon_one() {
    // A well-formed guest image: the LibOS region array, the init args and both rings,
    // planted by the shared world builder. The GH100 model's `libos_region_layout` is
    // identical to the falcon regime's — that half of a generation genuinely is shared —
    // so the same memory serves both.
    let world = GspWorld::new(P580, MODEL_A);
    assert_eq!(
        Gh100GspModel::new().libos_region_layout().rmargs_id,
        MODEL_A.rmargs_id,
        "non-vacuity: the planted image must be the one this model looks for"
    );
    let boot_args_gpa = world.guest.boot_args_gpa;
    let mut ram = world.ram;

    let arch = Gh100Arch::new();
    let mut f = fsm();
    let mut policy = EchoOk;

    macro_rules! wr {
        ($off:expr, $val:expr) => {
            f.mmio_write(&mut ram, &arch, &mut policy, 0, $off, $val)
                .expect("every step of this generation's boot is serviceable")
        };
    }

    // 1. Stream the boot-args pointer into the FSP EMEM window, exactly as
    //    `_kfspConfigEmemc_GH100` + `_kfspWriteToEmem_GH100` do: point the cursor, then
    //    push DWORDs through the single data port.
    wr!(
        gh100_wire::EMEMC,
        gh100_wire::ememc_at(gh100_wire::BOOT_ARGS_OFF)
    );
    wr!(gh100_wire::EMEMD, boot_args_gpa & 0xFFFF_FFFF);
    wr!(gh100_wire::EMEMD, boot_args_gpa >> 32);
    assert_eq!(
        f.phase(),
        FsmPhase::Cold,
        "streaming a payload is not a boot: nothing has been rung yet"
    );

    // 2. TAIL then HEAD — head last, because it is the write that interrupts FSP.
    let r = wr!(gh100_wire::QUEUE_TAIL, 0xD4);
    assert!(
        r.transitions.is_empty() && f.phase() == FsmPhase::Cold,
        "the TAIL write alone must do nothing; got {:?}",
        r.transitions
    );

    let r = wr!(gh100_wire::QUEUE_HEAD, 0);

    // ★ One write, three stages: this generation has no separate FWSEC, no separate
    //   Booter Load and no boot-args mailbox pair.
    assert!(
        r.transitions.len() >= 3,
        "expected at least the three boot transitions; got {:?}",
        r.transitions
    );
    assert_eq!(
        &r.transitions[..3],
        &[Transition::E1, Transition::E5, Transition::E6],
        "the FSP command must raise the protected region, load the firmware and publish, \
         in that order; got {:?}",
        r.transitions
    );
    assert_eq!(f.phase(), FsmPhase::Booted);
    assert!(
        matches!(f.queue(), QueueState::Bound(_)),
        "the publish must have bound the queues out of the address the COT payload carried"
    );
    assert!(
        r.raise_status_irq,
        "GSP_INIT_DONE was posted, so the status interrupt must be announced"
    );
}

/// ★★★ **The other half of the same assertion: the falcon regime's registers are INERT on
/// this generation.**
///
/// This is the sweep the pinning test used to do, kept and made *stronger*. Before, it
/// showed that GH100 could not be driven past the first phase; the GSP falcon STARTCPU
/// still moved it, because the FSM's hard-coded match fired on a register GH100 happens to
/// have. Now GH100's own sequence says that register is not its boot trigger, so the phase
/// does not move **at all** — which is the correct answer and a different one.
#[test]
fn no_falcon_regime_register_boots_gh100() {
    let arch = Gh100Arch::new();
    let mut f = fsm();
    let mut ram = FakeRam::default();
    let mut policy = EchoOk;

    let mut advanced = Vec::new();
    for reg in [
        GspReg::GspFalconCpuctl,
        GspReg::GspFalconHwcfg2,
        GspReg::GspFalconDmatrfcmd,
        GspReg::GspFalconMailbox0,
        GspReg::GspFalconMailbox1,
        GspReg::GspFalconIrqstat,
        GspReg::GspFalconIrqmask,
        GspReg::GspFalconIrqdest,
        GspReg::GspFalconIrqsclr,
        GspReg::GspRiscvCpuctl,
        GspReg::Wpr2AddrLo,
        GspReg::Wpr2AddrHi,
        GspReg::GfwBootProgress,
        GspReg::GfwBootPlm,
        GspReg::Sec2FalconCpuctl,
        GspReg::Sec2FalconMailbox0,
        GspReg::Sec2FalconDmatrfcmd,
        GspReg::GspQueueHead(0),
    ] {
        let Some((bar, off)) = Gh100GspModel::at(reg) else {
            continue; // no register on this generation
        };
        let _ = f.mmio_write(&mut ram, &arch, &mut policy, bar, off, 0x2);
        if f.phase() != FsmPhase::Cold {
            advanced.push((reg, f.phase()));
        }
    }

    assert!(
        advanced.is_empty(),
        "★ a falcon-regime register booted GH100: {advanced:?}. This generation's boot is \
         the FSP command queue; if one of these now drives it, either the model or the \
         sequence has been given the other regime's meaning."
    );
    assert_eq!(f.phase(), FsmPhase::Cold);
}

/// ★★ **The stage lists are data, and they are checked against an executed boot rather
/// than trusted.** A declared list nothing compares against is decoration.
///
/// Each generation's sequence is driven with its own registers and its own evolving
/// context, and the ordered step kinds it emits must equal what `stages()` declares. The
/// two lists have **different lengths** — five against three — which is the finding in one
/// number.
#[test]
fn each_generations_declared_stages_match_the_steps_it_actually_emits() {
    // ── the falcon/secure-booter regime, through Ada's model ──
    let ada = Ad10xGspModel::new();
    let seq = ada.boot_sequence();
    let mut st = ArchBootState::default();
    let mut kinds: Vec<BootStepKind> = Vec::new();
    let mut ctx = BootContext::default();

    let drive = |kinds: &mut Vec<BootStepKind>,
                 st: &mut ArchBootState,
                 ctx: &BootContext,
                 reg: GspReg,
                 val: u64| {
        let (bar, off) = Ad10xGspModel::at(reg).expect("Ada has this register");
        let w = RegWrite {
            bar,
            off,
            val,
            reg: Some(reg),
        };
        let steps = seq.on_write(&ada, &w, ctx, st);
        assert!(!steps.overflowed());
        kinds.extend(steps.iter().map(kayfabe_arch::gsp::BootStep::kind));
    };

    drive(&mut kinds, &mut st, &ctx, GspReg::GspFalconCpuctl, 0x2);
    ctx.obs.stage = BootPhase::ProtectedRegionUp;
    drive(&mut kinds, &mut st, &ctx, GspReg::Sec2FalconMailbox0, 0);
    drive(&mut kinds, &mut st, &ctx, GspReg::Sec2FalconCpuctl, 0x2);
    ctx.obs.stage = BootPhase::Booted;
    drive(&mut kinds, &mut st, &ctx, GspReg::GspFalconMailbox0, 0x1000);
    ctx.obs.boot_args_lo = 0x1000;
    ctx.boot_args_seen = (true, false);
    drive(&mut kinds, &mut st, &ctx, GspReg::GspFalconMailbox1, 0);

    let declared: Vec<BootStepKind> = seq.stages().iter().map(|s| s.step).collect();
    assert_eq!(
        kinds, declared,
        "the falcon regime's declared stages must be the steps it emits, in order"
    );
    assert_eq!(
        declared.len(),
        5,
        "five stages: two pokes and a mailbox pair"
    );

    // ── the FSP command-queue regime ──
    let hop = Gh100GspModel::new();
    let hseq = hop.boot_sequence();
    let mut hst = ArchBootState::default();
    let hctx = BootContext::default();
    let mut hkinds: Vec<BootStepKind> = Vec::new();
    for (off, val) in [
        (
            gh100_wire::EMEMC,
            gh100_wire::ememc_at(gh100_wire::BOOT_ARGS_OFF),
        ),
        (gh100_wire::EMEMD, 0x4000_0000),
        (gh100_wire::EMEMD, 0),
        (gh100_wire::QUEUE_TAIL, 0xD4),
        (gh100_wire::QUEUE_HEAD, 0),
    ] {
        let w = RegWrite {
            bar: 0,
            off,
            val,
            reg: hop.decode_reg(0, off),
        };
        let steps = hseq.on_write(&hop, &w, &hctx, &mut hst);
        assert!(!steps.overflowed());
        hkinds.extend(steps.iter().map(kayfabe_arch::gsp::BootStep::kind));
    }
    let hdeclared: Vec<BootStepKind> = hseq.stages().iter().map(|s| s.step).collect();
    assert_eq!(
        hkinds, hdeclared,
        "the FSP regime's declared stages must be the steps it emits, in order"
    );
    assert_eq!(
        hdeclared.len(),
        3,
        "three stages: one command does the work of four writes"
    );
    assert_ne!(
        declared, hdeclared,
        "if the two generations declared the same stages, one of them would be inheriting \
         the other's ordering and this whole file would be measuring nothing"
    );
}

/// ★★★ **Could two GPUs carry different boot implementations under this shape?**
///
/// The owner's constraint on the seam is that a hook must be reachable **per GPU
/// instance**, never fixed per build or per process — otherwise heterogeneous multi-GPU
/// (today a policy refusal, `kayfabe_core::gpu::CoreFault::HeterogeneousArch`) becomes a
/// *structural* impossibility and lifting it later is exactly the bolt-on the reframe
/// rules out.
///
/// This test answers it by construction: three register models, from two different crates,
/// live as values side by side in one process, and each answers with its own boot
/// ordering. Nothing here is a type parameter, a `const` chosen at compile time, or keyed
/// on "the arch this binary was built for".
#[test]
fn boot_orderings_are_per_instance_values_not_a_build_time_choice() {
    let models: Vec<Box<dyn GspModel>> = vec![
        Box::new(Ga10xGspModel::new()),
        Box::new(Ad10xGspModel::new()),
        Box::new(Gh100GspModel::new()),
    ];
    let stage_counts: Vec<usize> = models
        .iter()
        .map(|m| m.boot_sequence().stages().len())
        .collect();
    assert_eq!(
        stage_counts,
        vec![5, 5, 3],
        "two instances selecting the falcon regime and one selecting the FSP regime, all \
         three alive at once — so a GpuId axis can hand each GPU its own"
    );

    // …and the values are not the same object: each declines the other's boot trigger.
    let ctx = BootContext::default();
    let mut st = ArchBootState::default();
    assert!(
        models[0]
            .boot_sequence()
            .on_write(
                &*models[0],
                &RegWrite {
                    bar: 0,
                    off: gh100_wire::QUEUE_HEAD,
                    val: 0,
                    reg: None,
                },
                &ctx,
                &mut st
            )
            .is_empty(),
        "the falcon regime must not react to an FSP command-queue write"
    );
    let (bar, off) = Ga10xGspModel::at(GspReg::Sec2FalconCpuctl).expect("GA10x has a SEC2 CPUCTL");
    assert!(
        models[2]
            .boot_sequence()
            .on_write(
                &*models[2],
                &RegWrite {
                    bar,
                    off,
                    val: 0x2,
                    reg: models[2].decode_reg(bar, off),
                },
                &ctx,
                &mut st
            )
            .is_empty(),
        "the FSP regime must not react to a SEC2 Booter start"
    );
}

/// ★★ The arch-local boot events, enumerated with their evidence, and asserted non-empty
/// here so the list cannot quietly become decoration.
///
/// ★ It also asserts that the list still names something **unsolved**. Three of the four
/// entries are now served through the boot-sequence seam; the fourth
/// (Confidential-Compute WPR2 suppression) is a missing *observable*, not a missing
/// ordering, and the seam does not address it. A list that only records solved problems is
/// a trophy cabinet.
#[test]
fn the_arch_local_boot_events_are_named_with_their_evidence() {
    let events = kayfabe_chips::gh100::ARCH_LOCAL_BOOT_EVENTS;
    assert!(
        events.len() >= 4,
        "the enumerated list must not shrink silently; got {}",
        events.len()
    );
    for (name, why) in events {
        assert!(!name.is_empty(), "every entry names the event");
        assert!(
            why.contains("ogkm-580") || why.contains("dev_fsp_pri.h"),
            "{name}: every entry carries its SOURCE, not an opinion — got {why:?}"
        );
    }
    assert!(
        events
            .iter()
            .any(|(_, why)| why.contains("STILL UNMODELLED")),
        "the list must still name what the seam does NOT solve"
    );
    assert!(
        events.iter().any(|(_, why)| why.contains("SERVED:")),
        "…and what it does"
    );
}

/// ★ The GH100 model answers its own FSP registers on the READ path too — through the same
/// seam, because no `GspReg` variant names them.
#[test]
fn gh100_serves_its_own_registers_on_the_read_path() {
    let arch = Gh100Arch::new();
    let mut f = fsm();
    let mut ram = FakeRam::default();
    let mut policy = EchoOk;

    // The command queue reads drained — the predicate `kfspCanSendPacket_GH100` gates the
    // next send on (`ogkm-580: kern_fsp_gh100.c:241-256`).
    assert_eq!(f.mmio_read(&arch, 0, gh100_wire::QUEUE_HEAD), Some(Ok(0)));
    assert_eq!(f.mmio_read(&arch, 0, gh100_wire::QUEUE_TAIL), Some(Ok(0)));

    // The EMEM window reads back what the guest wrote through it.
    let cursor = gh100_wire::ememc_at(64);
    f.mmio_write(&mut ram, &arch, &mut policy, 0, gh100_wire::EMEMC, cursor)
        .expect("pointing the cursor is serviceable");
    f.mmio_write(
        &mut ram,
        &arch,
        &mut policy,
        0,
        gh100_wire::EMEMD,
        0xDEAD_BEEF,
    )
    .expect("streaming a DWORD is serviceable");
    // AINCW moved the cursor on; point it back and read.
    f.mmio_write(&mut ram, &arch, &mut policy, 0, gh100_wire::EMEMC, cursor)
        .expect("re-pointing the cursor is serviceable");
    assert_eq!(
        f.mmio_read(&arch, 0, gh100_wire::EMEMD),
        Some(Ok(0xDEAD_BEEF)),
        "the EMEM window must read back the guest's own DWORD"
    );

    // ⊘ …and an offset that is nobody's is still nobody's: `None`, never a defaulted zero.
    assert!(f.mmio_read(&arch, 0, 0x0099_9990).is_none());
}

/// ★★ **The correction #118 got wrong, pinned so it cannot silently revert.**
///
/// #118 recorded `GspQueueHead` as absent on GH100 because `hopper/gh100/dev_gsp.h` does
/// not define `NV_PGSP_QUEUE_HEAD`. But `kgspSetCmdQueueHead` is halified and the *Turing*
/// binding is the fallback for every chip that is not a VF or a Tegra part
/// (`ogkm-580: src/nvidia/generated/g_kernel_gsp_nvoc.c:664-679`), so GH100 writes
/// `0x110c00+(i)*8` on every RPC send (`ogkm-580: kernel_gsp.c:425`).
#[test]
fn gh100_has_the_command_doorbell_the_turing_hal_writes() {
    let (bar, off) =
        Gh100GspModel::at(GspReg::GspQueueHead(0)).expect("GH100 binds kgspSetCmdQueueHead_TU102");
    assert_eq!((bar, off), (0, 0x0011_0C00));
    let (_, off7) = Gh100GspModel::at(GspReg::GspQueueHead(7)).expect("__SIZE_1 is 8");
    assert_eq!(off7, 0x0011_0C38);
    assert!(
        Gh100GspModel::at(GspReg::GspQueueHead(8)).is_none(),
        "past __SIZE_1 there is no queue"
    );
    // …and it round-trips through the decoder, so a doorbell is recognised as one.
    assert_eq!(
        Gh100GspModel::new().decode_reg(0, 0x0011_0C00),
        Some(GspReg::GspQueueHead(0))
    );
}

/// ★ The registers that really are absent on GH100 stay absent, and are still refused
/// rather than defaulted.
#[test]
fn the_registers_gh100_genuinely_lacks_are_still_refused() {
    let absent: Vec<GspReg> = [
        GspReg::GfwBootProgress,
        GspReg::GfwBootPlm,
        GspReg::Sec2FalconCpuctl,
        GspReg::Sec2FalconMailbox0,
        GspReg::Sec2FalconDmatrfcmd,
    ]
    .into_iter()
    .filter(|r| Gh100GspModel::at(*r).is_none())
    .collect();
    assert_eq!(absent.len(), 5, "got {absent:?}");

    // MISS = FAULT, not zero.
    let m = Gh100GspModel::new();
    for reg in absent {
        assert!(
            m.encode(reg, &GspObservation::default()).is_none(),
            "{reg:?} must be unserviceable, never a defaulted zero"
        );
    }

    // …and the counterpart, so the assertion is not vacuously about a model that answers
    // nothing at all: the registers Hopper DOES share are served.
    for reg in [
        GspReg::GspFalconCpuctl,
        GspReg::GspFalconMailbox1,
        GspReg::Wpr2AddrHi,
        GspReg::GspRiscvCpuctl,
    ] {
        assert!(
            Gh100GspModel::at(reg).is_some(),
            "{reg:?} is at the same offset on GH100 and must be served"
        );
    }
}

/// ★★ **One register, two regimes, two meanings — and the stage is what tells them
/// apart.** `NV_PGSP_FALCON_HWCFG2` is read for `RISCV_ENABLE` on the falcon regime and
/// for `RISCV_BR_PRIV_LOCKDOWN` on this one (`ogkm-580: kernel_gsp_gh100.c:489-535`).
/// This is the observable that made `GspObservation::stage` necessary.
#[test]
fn gh100_hwcfg2_reports_lockdown_until_the_fsp_command_has_booted_the_fmc() {
    let m = Gh100GspModel::new();
    let cold = m
        .encode(GspReg::GspFalconHwcfg2, &GspObservation::default())
        .expect("HWCFG2 is served");
    let booted = m
        .encode(
            GspReg::GspFalconHwcfg2,
            &GspObservation {
                stage: BootPhase::Booted,
                wpr2_up: true,
                ..GspObservation::default()
            },
        )
        .expect("HWCFG2 is served");

    // `RISCV_BR_PRIV_LOCKDOWN` is 13:13 and `_LOCK` is 1 (`ogkm-580: hopper/gh100/
    // dev_falcon_v4.h:79-81`). A literal, not the module's own constant read back.
    assert_eq!(
        cold & (1 << 13),
        1 << 13,
        "locked down before the FMC boots"
    );
    assert_eq!(booted & (1 << 13), 0, "unlocked after");
    // Both must be non-zero and not the 0xBADF41xx priv-error pattern, or
    // `_kfspIsGspTargetMaskReleased` never returns true
    // (`ogkm-580: kern_fsp_gh100.c:1096-1114`).
    for v in [cold, booted] {
        assert_ne!(v, 0);
        assert_ne!(v & 0xFFFF_FF00, 0xBADF_4100);
    }
}
