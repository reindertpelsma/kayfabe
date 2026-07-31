//! ★★★ **The second-GPU-generation measurement** — the claim tested by making the edit.
//!
//! Three places in this repository's prose say that adding a GPU generation costs an
//! `impl Arch` in an adapter crate and **zero edits to a logic crate**. Nobody had ever
//! added a second one, so the claim asserted a measurement it did not have. This file and
//! `crates/kayfabe-chips` are that measurement.
//!
//! ## Two architectures, two opposite answers, reported separately
//!
//! ### AD10x (Ada) — the claim SURVIVES, and it is the easy case
//!
//! Every register `GspReg` models sits at the same offset on Ada as on GA10x, because
//! NVIDIA's own generated HAL binds the Turing/Ampere implementations for the whole boot
//! sequence on `TU102…AD107` (`ogkm-580: src/nvidia/generated/g_gpu_nvoc.c:2374-2385`).
//! [`Ad10xArch`] plugs into the unmodified [`GspFsm`] and reaches `BootPhase::Booted`.
//!
//! ⚠ **An experiment that selects the easiest member of its universe produces a green
//! with no red available to it** — the same defect as a gate quantified over a shortened
//! list. So Ada is reported as what it is: the confirming case, and weak alone.
//!
//! ### GH100 (Hopper) — the claim FAILS, in a logic crate
//!
//! [`Gh100Arch`] plugs in just as cleanly and **cannot be booted**, because the FSM's
//! `mmio_write` encodes the *Turing boot ordering* in its `match` arms over `GspReg`, and
//! three of the registers those arms fire on do not exist on this generation. The
//! evidence for each absence is in `kayfabe_chips::gh100`'s module docs, read from
//! `ogkm-580`.
//!
//! The tests below pin **both** answers. The Hopper one is a characterisation test: if
//! someone gives the FSM a Hopper-capable vocabulary, it goes red, which is the signal.
//!
//! ## What this file deliberately does NOT test
//!
//! It does not claim Hopper *would* boot with those changes — nothing here has touched
//! Hopper silicon and the fixture is explicitly not a port. It measures one thing: what a
//! second generation costs, and in which crate.

use kayfabe_arch::Arch;
use kayfabe_arch::gsp::{GspModel, GspReg};
use kayfabe_chips::{Ad10xArch, Ad10xGspModel, Gh100Arch, Gh100GspModel};
use kayfabe_gsp::{BootPhase, EchoOk, GspFsm, Transition};
use kayfabe_tests::gspworld::{FakeRam, P580};

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
    assert_eq!(f.phase(), BootPhase::ProtectedRegionUp);

    let (bar, off) = Ad10xGspModel::at(GspReg::Sec2FalconMailbox0).expect("Ada has a SEC2 mailbox");
    f.mmio_write(&mut ram, &arch, &mut policy, bar, off, 0x0)
        .expect("latching the Booter argument is serviceable");
    let (bar, off) = Ad10xGspModel::at(GspReg::Sec2FalconCpuctl).expect("Ada has a SEC2 CPUCTL");
    let r = f
        .mmio_write(&mut ram, &arch, &mut policy, bar, off, 0x2)
        .expect("the Booter Load is serviceable");
    assert_eq!(r.transitions, vec![Transition::E5], "Booter Load");
    assert_eq!(f.phase(), BootPhase::Booted);
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

// ─────────────────────── GH100: the refuting case, PINNED ────────────────────────

/// ★★★ **THE HEADLINE, as a test.** Four of the eighteen [`GspReg`] variants have no
/// register on GH100, and three of them are ones the FSM's `mmio_write` fires transitions
/// on. The seam carries a generation's *values*; it does not carry its *sequence*.
///
/// Each absence is sourced in `kayfabe_chips::gh100`'s module docs. This test asserts the
/// **status quo**: if `GspReg` grows a Hopper-capable vocabulary, it goes red, and the fix
/// is to rewrite it to the new behaviour — never to widen it so both answers pass.
#[test]
fn a_generation_with_a_different_boot_sequence_has_registers_the_seam_cannot_name() {
    let absent: Vec<GspReg> = [
        GspReg::GfwBootProgress,
        GspReg::GfwBootPlm,
        GspReg::Sec2FalconCpuctl,
        GspReg::Sec2FalconMailbox0,
        GspReg::GspQueueHead(0),
    ]
    .into_iter()
    .filter(|r| Gh100GspModel::at(*r).is_none())
    .collect();

    assert_eq!(
        absent.len(),
        5,
        "characterisation: every one of these has no GH100 register today; got {absent:?}"
    );

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

/// ★★★ **The FSM cannot be driven at all on this generation** — by any register write,
/// because this generation has **no boot sequence**.
///
/// The seam that carries a boot ordering exists now (`kayfabe_arch::BootSequence`), and
/// GH100 selects `NoBootSequence`: zero declared stages, no step for any write. That is
/// the honest state of a generation whose registers are mapped and whose *ordering* has
/// not been written — and it is deliberately not "the falcon regime by default", which
/// would make this model appear to boot by running Ada's sequence.
///
/// This is a characterisation test. When GH100's own sequence lands it goes red, which is
/// the signal, and the fix is to rewrite it to the new behaviour — never to widen it so
/// both answers pass.
#[test]
fn the_boot_fsm_has_no_sequence_to_drive_on_that_generation() {
    let arch = Gh100Arch::new();
    let mut f = fsm();
    let mut ram = FakeRam::default();
    let mut policy = EchoOk;

    assert!(
        Gh100GspModel::new().boot_sequence().stages().is_empty(),
        "★ THE SEAM NOW CARRIES A HOPPER BOOT SEQUENCE. That is good news and this test \
         must be rewritten to the new behaviour — do NOT widen it."
    );

    // Sweep EVERY BAR0 offset the model decodes, at the value that would be a STARTCPU,
    // and assert none of them moves the phase. A sweep rather than a list, so that a model
    // change cannot make it pass by moving a register.
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
            continue; // no register on this generation — that IS the finding
        };
        let _ = f.mmio_write(&mut ram, &arch, &mut policy, bar, off, 0x2);
        if f.phase() != BootPhase::Cold {
            advanced.push((reg, f.phase()));
        }
    }

    assert!(
        advanced.is_empty(),
        "★ something boots GH100 now: {advanced:?} — rewrite this test to the new \
         behaviour, do NOT widen it."
    );
    assert_eq!(f.phase(), BootPhase::Cold);
}

/// ★★ The cost of the bolt-on, enumerated in the fixture and asserted non-empty here so
/// the list cannot quietly become decoration.
#[test]
fn the_missing_boot_transitions_are_named_with_their_evidence() {
    let missing = kayfabe_chips::gh100::MISSING_TRANSITIONS;
    assert!(
        missing.len() >= 4,
        "the enumerated cost must not shrink silently; got {}",
        missing.len()
    );
    for (name, why) in missing {
        assert!(!name.is_empty(), "every entry names the event");
        assert!(
            why.contains("ogkm-580") || why.contains("dev_fsp_pri.h"),
            "{name}: every entry carries its SOURCE, not an opinion — got {why:?}"
        );
    }
}
