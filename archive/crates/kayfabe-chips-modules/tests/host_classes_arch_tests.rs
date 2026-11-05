//! # Arch-level host-class tests (RETIRED with the chip modules, 2026-10-31)
//!
//! **STATUS: SUPERSEDED, 2026-10-31.** These three tests were the part of
//! `crates/kayfabe-chips/tests/host_classes.rs` that constructed `kayfabe_chips::{Ga10xArch,
//! Ad10xArch, Gh100Arch, Gb20xArch}` and `kayfabe_mocks::MockArch`. Those types moved to
//! `archive/crates/kayfabe-chips-modules/src/` with this file, so the tests no longer build; the
//! profile tests that need only `host_classes` stayed in `crates/kayfabe-chips/tests/host_classes.rs`.
//! This file is a verbatim extract, not a workspace member.

use kayfabe_arch::HostClasses;
use kayfabe_chips::{Ad10xHostClasses, Ga10xHostClasses, Gb20xHostClasses, Gh100HostClasses};

/// Verbatim copy of the helper of the same name in `tests/host_classes.rs`.
fn roles(p: &dyn HostClasses) -> (u32, u32, u32) {
    (
        p.gpfifo_channel().channel_id().0,
        p.usermode().usermode_id().0,
        p.ce_object().ce_object_id().0,
    )
}

/// ★★★ `MockArch` must NOT answer this seam, and neither may any arch that has no host
/// profile — a mock that invented three class ids would let a wrong number reach a real
/// `NV_ESC_RM_ALLOC` with every test green.
///
/// This is the same shape as `Arch::gsp`'s `None`, and it is tested for the same reason:
/// a refusal nobody watched refuse is not a refusal.
#[test]
fn an_arch_with_no_host_profile_refuses_by_name_rather_than_inventing_one() {
    use kayfabe_arch::Arch;
    let mock = kayfabe_mocks::MockArch::default();
    assert!(
        mock.host_classes().is_none(),
        "★ MockArch answered the host-class seam. Three invented class ids on a \
         forwarding path is exactly the residue this seam exists to remove"
    );
    // …and the three real ones DO answer, or the assertion above is about a seam nobody
    // implements.
    for a in [
        &kayfabe_chips::Ga10xArch::default() as &dyn Arch,
        &kayfabe_chips::Ad10xArch::default() as &dyn Arch,
        &kayfabe_chips::Gh100Arch::default() as &dyn Arch,
        &kayfabe_chips::Gb20xArch::default() as &dyn Arch,
    ] {
        assert!(
            a.host_classes().is_some(),
            "★ NON-VACUITY: {} declares no host classes, so the None above proves nothing",
            a.name()
        );
    }
}

/// ★★ Each `Arch` hands back **its own** profile — the delegation trap, watched.
///
/// `Ad10xArch` and `Gh100Arch` delegate almost every `Arch` method to a composed
/// `MockArch`. Delegating this one would have compiled, returned `None`, and read as
/// "unbuilt" for the one generation whose host classes actually differ.
#[test]
fn each_arch_declares_its_own_profile_and_not_a_composed_ones() {
    use kayfabe_arch::Arch;
    let pairs: [(&dyn Arch, &dyn HostClasses); 4] = [
        (&kayfabe_chips::Ga10xArch::default(), &Ga10xHostClasses),
        (&kayfabe_chips::Ad10xArch::default(), &Ad10xHostClasses),
        (&kayfabe_chips::Gh100Arch::default(), &Gh100HostClasses),
        (&kayfabe_chips::Gb20xArch::default(), &Gb20xHostClasses),
    ];
    for (arch, want) in pairs {
        let got = arch.host_classes().expect("declared above");
        assert_eq!(
            (roles(got), got.name()),
            (roles(want), want.name()),
            "★ {} returned the wrong profile",
            arch.name()
        );
    }
}

// ── The doorbell decode, no longer `MockArch`'s invention on two generations (`#156`) ──

/// ★★★ The three generations RM binds to `kfifoGenerateWorkSubmitTokenHal_GA100` decode a
/// work-submit token **identically**, and none of them answers with `MockArch`'s invented
/// encoding any more.
///
/// ⊘⊘ **THE NAME OF THIS TEST USED TO SAY "every generation", AND THAT IS NO LONGER TRUE.**
/// GB202 binds its own encoder, which sets `RUNLIST_DOORBELL` at bit 30, and is therefore
/// **deliberately absent** from the list below — `kayfabe_chips::Gb20xArch` would fail every
/// assertion in it, correctly. The counterexample is asserted positively in
/// `crates/kayfabe-chips/tests/gb20x_doorbell_and_regs.rs::\
/// the_ampere_decoder_refuses_every_gb202_token_and_that_is_the_whole_point`, because a
/// sameness test over a list that quietly stops short of the member that breaks it is the
/// *"gate quantified over a shortened list"* this crate's own docs name. ⚠ If a fifth
/// generation is added, decide which of the two lists it joins — do not default it here.
///
/// `execution_plane_increments.md` §2.1 names a wrong doorbell decode as the one
/// execution-plane error that **cannot fail loudly** — on the Mode-2 path we are the GSP,
/// so a ring routed to the wrong channel has no second party to notice. `Ad10xArch` and
/// `Gh100Arch` were delegating that seam to their composed `MockArch`, whose encoding is
/// deliberately made up. They now share GA10x's, which is what RM does: the driver's own
/// dispatch table binds `kfifoGenerateWorkSubmitTokenHal_GA100` to
/// `GA100..GA107 | AD102..AD107 | GH100` (`ogkm-580: g_kernel_fifo_nvoc.c:648-652`), and
/// neither `ada/ad102/` nor `hopper/gh100/` carries a `dev_ctrl.h` to override the field
/// positions.
///
/// ⊘ Still not a run on Ada or Hopper silicon. It replaces an INVENTED answer with the
/// implementation the vendored driver binds to those parts.
#[test]
fn the_ga100_bound_generations_decode_a_token_the_same_way_and_none_uses_the_mocks() {
    use kayfabe_arch::Arch;
    // ⊘ GB202 is NOT here, and its absence is the finding — see this test's docs.
    let arches: [&dyn Arch; 3] = [
        &kayfabe_chips::Ga10xArch::default(),
        &kayfabe_chips::Ad10xArch::default(),
        &kayfabe_chips::Gh100Arch::default(),
    ];
    // Well-formed tokens (runlist 22:16, chid 11:0) and malformed ones (a bit RM's
    // encoder starts from zero and never sets).
    let probes: [u64; 8] = [
        0x0000_0000,
        0x0000_0004,
        0x0000_0FFF,
        0x007F_0FFF,
        0x0003_0001,
        0x0000_1000,   // bit 12 — in the 15:12 hole
        0x0080_0000,   // bit 23 — above RUNLIST_ID
        0x1_0000_0000, // wider than the u32 RM writes
    ];
    let mut refusals = 0usize;
    for t in probes {
        let base = kayfabe_chips::ga10x::decode_work_submit_token(t);
        if base.is_none() {
            refusals += 1;
        }
        for a in arches {
            assert_eq!(
                a.decode_doorbell(t),
                base,
                "★ {} decodes token {t:#x} differently from the shared encoder RM binds \
                 to all three generations",
                a.name()
            );
        }
    }
    assert_eq!(
        refusals, 3,
        "★ NON-VACUITY: three of the probes are malformed and must be REFUSED, or this \
         test is comparing three functions that all say Some(0)"
    );

    // ★★ And the delegation really changed something: `MockArch` must disagree, or
    // "no longer the mock's" is a statement about two identical functions.
    let mock = kayfabe_mocks::MockArch::default();
    let disagreement = probes
        .iter()
        .any(|t| mock.decode_doorbell(*t) != kayfabe_chips::ga10x::decode_work_submit_token(*t));
    assert!(
        disagreement,
        "★ NON-VACUITY: MockArch's INVENTED encoding agrees with RM's on every probe, so \
         switching away from it proves nothing. Pick probes that separate them"
    );
}

