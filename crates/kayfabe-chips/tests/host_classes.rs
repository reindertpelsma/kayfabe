//! # The host-class profiles, checked against NVIDIA's own per-chip tables (`#156`)
//!
//! ## ★★★ What the observer is, and why it is not the thing under test
//!
//! A test that asserted `Gh100HostClasses.ce_object() == HOPPER_DMA_COPY_A` would be
//! reading the same constant the implementation reads and calling the agreement evidence.
//! It would pass against a profile that had every role wired to the wrong *role*, and a
//! mutation that swapped two accessors would survive it.
//!
//! So the oracle here is a **different artifact**: NVIDIA's generated per-chip class
//! table, `ogkm-580: src/nvidia/generated/g_gpu_class_list.c`, transcribed below as raw
//! `(class id, line)` rows — plus a re-implementation of the **selection rule RM's own
//! client applies to it**, `findDeviceClasses` (`ogkm-580:
//! src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:8630-8699`): take the numerically largest
//! member of each family the device reports. The profile is never consulted to compute
//! the expectation; it is only compared against it.
//!
//! ⊘ **The limit, stated rather than implied.** This is still a transcription, and a
//! transcription cannot detect a shared misreading of the source. What it *can* detect —
//! and what the bites in `scripts/bite_host_classes.py` watch it detect — is a wrong
//! number, a swapped role, and a generation silently inheriting another's answer. It
//! establishes nothing about whether an Ada or Hopper board accepts any of it: **compiling
//! for a generation is not booting on one**, and no board of either kind has ever run this.
//!
//! ## ★★★ The half this file CANNOT reach, and what took it over (`#166`)
//!
//! "A swapped role" above means *a profile that answers the channel number when asked for
//! usermode* — a defect inside an `impl HostClasses`, and this file's whole business. It
//! does **not** mean *a call site that asks the profile for the wrong role*. That second
//! thing is in `crates/kayfabe-isolate-host`, it is where the class id actually reaches a
//! real `NV_ESC_RM_ALLOC`, and no test in this file can see it: every value it could
//! observe would be a correct one, just fetched for the wrong hole.
//!
//! It was measured uncovered — `PROFILE 9/9 caught, WIRING 0/3 caught` at `36f746a` — and
//! it is now refused by the **type system** instead: `HostClasses`' three methods return
//! `ChannelClass` / `UsermodeClass` / `CeObjectClass` and every host-path consumer names
//! the role, so the swap does not compile. ★ Note the division of labour, because neither
//! instrument covers the other's half: **types cannot see a wrong number under a right
//! role tag** (`UsermodeClass::new(ClassId(AMPERE_CHANNEL_GPFIFO_A))` type-checks), which
//! is exactly what this file exists for. `tests/tests/host_class_role_wiring.rs` guards
//! the type refusal's own edges.
//!
//! ## The wrinkle this file records rather than hides
//!
//! For the *usermode* role, RM's UVM-facing client does **not** use the max-in-family
//! rule: `gpuDeviceMapUsermodeRegion` hardcodes `VOLTA_USERMODE_A` and upgrades to
//! `HOPPER_USERMODE_A` only for Hopper-plus (`ogkm-580: nv_gpu_ops.c:5542-5557`). It picks
//! an *older* class than the part supports, below Hopper. This port picks the newest
//! listed one, which is what the C artifact's proven host self-test allocated on GA10x.
//! The two rules **agree on GH100** — the case that motivated the seam — and differ only
//! in how conservative they are below it. Both classes are in both lists.

use kayfabe_arch::HostClasses;
use kayfabe_chips::{
    Ad10xHostClasses, Ga10xHostClasses, Gb20xHostClasses, Gh100HostClasses,
    host_classes::pinned_host_classes,
};

/// ★★★ **The one place in this file that unwraps a role type** (`#166`).
///
/// Since `#166` the three roles are three distinct Rust types
/// (`kayfabe_arch::{ChannelClass, UsermodeClass, CeObjectClass}`), so the profile's
/// answers can no longer be compared to each other, or to the oracle's raw ids, without
/// saying which role each one is. That is the point — but this file's oracle is a table
/// of bare `u32`s read out of `g_gpu_class_list.c`, so *somewhere* the tag has to come
/// off.
///
/// It comes off **here, once, in role order**, and `tests/tests/host_class_role_wiring.rs`
/// pins that this file contains exactly these three unwraps. A fourth one appearing
/// anywhere in the tree is a red test, which is what keeps "the role is a type" from
/// decaying into "the role is a type except where it was inconvenient".
fn roles(p: &dyn HostClasses) -> (u32, u32, u32) {
    (
        p.gpfifo_channel().channel_id().0,
        p.usermode().usermode_id().0,
        p.ce_object().ce_object_id().0,
    )
}

// ── The oracle's vocabulary: NVIDIA class ids, from the class headers ────────────────
//
// ★ These are transcribed HERE rather than imported from `kayfabe-abi` on purpose. Six of
// the twelve are ids no production code in this tree spells, and importing the other six
// from the same module the profiles read would collapse the oracle back onto the thing it
// observes.
//
// `ogkm-580: src/common/sdk/nvidia/inc/class/…`
const NV50_CHANNEL_GPFIFO: u32 = 0x0000_506f; // cl506f.h:34
const GF100_CHANNEL_GPFIFO: u32 = 0x0000_906f; // cl906f.h:42
const VOLTA_CHANNEL_GPFIFO_A: u32 = 0x0000_c36f; // clc36f.h:43
const TURING_CHANNEL_GPFIFO_A: u32 = 0x0000_c46f; // clc46f.h:43
const AMPERE_CHANNEL_GPFIFO_A: u32 = 0x0000_c56f; // clc56f.h:43
const HOPPER_CHANNEL_GPFIFO_A: u32 = 0x0000_c86f; // clc86f.h:27
const VOLTA_USERMODE_A: u32 = 0x0000_c361; // clc361.h:27
const TURING_USERMODE_A: u32 = 0x0000_c461; // clc461.h:27
const AMPERE_USERMODE_A: u32 = 0x0000_c561; // clc561.h:27
const HOPPER_USERMODE_A: u32 = 0x0000_c661; // clc661.h:26
const AMPERE_DMA_COPY_B: u32 = 0x0000_c7b5; // clc7b5.h:33
const HOPPER_DMA_COPY_A: u32 = 0x0000_c8b5; // clc8b5.h:27
const BLACKWELL_CHANNEL_GPFIFO_A: u32 = 0x0000_c96f; // clc96f.h:27
const BLACKWELL_CHANNEL_GPFIFO_B: u32 = 0x0000_ca6f; // clc96f.h:33
const BLACKWELL_USERMODE_A: u32 = 0x0000_c761; // clc761.h:27
const BLACKWELL_DMA_COPY_B: u32 = 0x0000_cab5; // clcab5.h:27

/// One chip's class list, as `g_gpu_class_list.c` states it, restricted to the three
/// families the host-forwarding path allocates from.
///
/// ★ Restricted, not filtered by the code under test: a class is in a row below because a
/// human read it out of the chip's `gpuGetEngClassDescriptorList_<CHIP>` (or, for
/// `GF100_CHANNEL_GPFIFO`, its `gpuGetNoEngClassList_<CHIP>`) at the cited line.
struct ChipClassList {
    chip: &'static str,
    /// `isClassHost` family — every class `CliGetChannelClassInfo` types as
    /// `CHANNEL_CLASS_TYPE_GPFIFO` (`ogkm-580: nv_gpu_ops.c:8543-8549`).
    gpfifo: &'static [u32],
    /// The usermode family. `nv_gpu_ops` has no `isClassUsermode` — see the module note.
    usermode: &'static [u32],
    /// `isClassCE` family — the enumerated switch at `ogkm-580: nv_gpu_ops.c:8552-8582`.
    ce: &'static [u32],
}

/// GA106 — `gpuGetEngClassDescriptorList_GA106` at `g_gpu_class_list.c:1108`,
/// `gpuGetNoEngClassList_GA106` at `:1056`.
const GA106: ChipClassList = ChipClassList {
    chip: "GA106",
    // :1144, :1064, :1168, :1166, :1113
    gpfifo: &[
        NV50_CHANNEL_GPFIFO,
        GF100_CHANNEL_GPFIFO,
        VOLTA_CHANNEL_GPFIFO_A,
        TURING_CHANNEL_GPFIFO_A,
        AMPERE_CHANNEL_GPFIFO_A,
    ],
    // :1169, :1167, :1120
    usermode: &[VOLTA_USERMODE_A, TURING_USERMODE_A, AMPERE_USERMODE_A],
    // :1115-1119 (ENG_CE(0..4)) — the ONLY CE class GA106 lists
    ce: &[AMPERE_DMA_COPY_B],
};

/// AD106 — `gpuGetEngClassDescriptorList_AD106` at `g_gpu_class_list.c:1732`,
/// `gpuGetNoEngClassList_AD106` at `:1680`.
///
/// ★ Row-for-row identical to [`GA106`] in all three families. Ada defines no
/// `ADA_CHANNEL_GPFIFO_*`, `ADA_USERMODE_*` or `ADA_DMA_COPY_*`; its one `ADA_*` row is
/// `ADA_COMPUTE_A` (`:1737`), which is a compute object and not one of these roles.
const AD106: ChipClassList = ChipClassList {
    chip: "AD106",
    // :1768, :1688, :1800, :1798, :1738
    gpfifo: &[
        NV50_CHANNEL_GPFIFO,
        GF100_CHANNEL_GPFIFO,
        VOLTA_CHANNEL_GPFIFO_A,
        TURING_CHANNEL_GPFIFO_A,
        AMPERE_CHANNEL_GPFIFO_A,
    ],
    // :1801, :1799, :1744
    usermode: &[VOLTA_USERMODE_A, TURING_USERMODE_A, AMPERE_USERMODE_A],
    // :1739-1743 (ENG_CE(0..4))
    ce: &[AMPERE_DMA_COPY_B],
};

/// GH100 — `gpuGetEngClassDescriptorList_GH100` at `g_gpu_class_list.c:1992`,
/// `gpuGetNoEngClassList_GH100` at `:1936`.
///
/// ★★★ The two rows that make this a seam: `AMPERE_CHANNEL_GPFIFO_A` (`:1996`) and
/// `AMPERE_USERMODE_A` (`:1997`) are **present**. A Hopper host will happily allocate
/// either. `AMPERE_DMA_COPY_B` is the one that is absent.
const GH100: ChipClassList = ChipClassList {
    chip: "GH100",
    // :2040, :1944, :2068, :2066, :1996, :2009
    gpfifo: &[
        NV50_CHANNEL_GPFIFO,
        GF100_CHANNEL_GPFIFO,
        VOLTA_CHANNEL_GPFIFO_A,
        TURING_CHANNEL_GPFIFO_A,
        AMPERE_CHANNEL_GPFIFO_A,
        HOPPER_CHANNEL_GPFIFO_A,
    ],
    // :2069, :2067, :1997, :2029
    usermode: &[
        VOLTA_USERMODE_A,
        TURING_USERMODE_A,
        AMPERE_USERMODE_A,
        HOPPER_USERMODE_A,
    ],
    // :2018-2027 (ENG_CE(0..9))
    ce: &[HOPPER_DMA_COPY_A],
};

/// GB202 — `gpuGetEngClassDescriptorList_GB202` at `g_gpu_class_list.c:2832`,
/// `gpuGetNoEngClassList_GB202` at `:2776`.
///
/// ★★★ The chip that makes the max-in-family rule and the MEASUREMENT disagree. Its
/// channel family holds **two** Blackwell members, `_A` (`:2845`) and `_B` (`:2846`), and
/// a real RTX 5090 was measured allocating `_A` — see
/// [`the_blackwell_channel_pick_departs_from_the_max_rule_because_a_5090_answered`].
const GB202: ChipClassList = ChipClassList {
    chip: "GB202",
    // :2899, :2784, :2932, :2930, :2839, :2883, :2845, :2846
    gpfifo: &[
        NV50_CHANNEL_GPFIFO,
        GF100_CHANNEL_GPFIFO,
        VOLTA_CHANNEL_GPFIFO_A,
        TURING_CHANNEL_GPFIFO_A,
        AMPERE_CHANNEL_GPFIFO_A,
        HOPPER_CHANNEL_GPFIFO_A,
        BLACKWELL_CHANNEL_GPFIFO_A,
        BLACKWELL_CHANNEL_GPFIFO_B,
    ],
    // :2933, :2931, :2840, :2885, :2867
    usermode: &[
        VOLTA_USERMODE_A,
        TURING_USERMODE_A,
        AMPERE_USERMODE_A,
        HOPPER_USERMODE_A,
        BLACKWELL_USERMODE_A,
    ],
    // :2855-2862 (ENG_CE(0..7)) — the ONLY CE class GB202 lists. Note what is ABSENT:
    // no AMPERE_DMA_COPY_B, no HOPPER_DMA_COPY_A, and no BLACKWELL_DMA_COPY_A either.
    ce: &[BLACKWELL_DMA_COPY_B],
};

/// `findDeviceClasses`' rule, re-implemented: `NV_MAX` across the family
/// (`ogkm-580: nv_gpu_ops.c:8684-8689`).
///
/// A function rather than a literal per chip, so that the expectation is *derived* from
/// the transcribed list. Editing a list row changes what the test demands; a literal
/// would not.
fn newest(family: &[u32]) -> u32 {
    *family
        .iter()
        .max()
        .expect("every family here has at least one member")
}

/// The three chips, paired with the profile that claims to describe each.
fn cases() -> Vec<(ChipClassList, &'static dyn HostClasses)> {
    vec![
        (GA106, &Ga10xHostClasses as &dyn HostClasses),
        (AD106, &Ad10xHostClasses as &dyn HostClasses),
        (GH100, &Gh100HostClasses as &dyn HostClasses),
    ]
}

/// ★★★ Every role of every profile is the class NVIDIA's own selection rule would pick
/// from that chip's own class list.
#[test]
fn each_profile_names_the_class_the_drivers_own_rule_selects_for_that_chip() {
    let mut checked = 0usize;
    for (chip, profile) in cases() {
        let (chan, user, ce) = roles(profile);
        for (role, expect, got) in [
            ("gpfifo_channel", newest(chip.gpfifo), chan),
            ("usermode", newest(chip.usermode), user),
            ("ce_object", newest(chip.ce), ce),
        ] {
            assert_eq!(
                got,
                expect,
                "★ {} :: {role} — the profile {:?} answers {got:#06x}, but the newest \
                 member of that family in {}'s own class list \
                 (g_gpu_class_list.c) is {expect:#06x}. A wrong class id here is an \
                 NV_ESC_RM_ALLOC on a real board with the wrong number in it",
                chip.chip,
                profile.name(),
                chip.chip
            );
            checked += 1;
        }
    }
    assert_eq!(
        checked, 9,
        "★ NON-VACUITY: three profiles × three roles must have been compared, not {checked}"
    );
}

/// ★★ The result that makes GH100 the load-bearing member: **all three roles differ from
/// Ampere's**, and the two channel/usermode ones would have been served silently.
#[test]
fn the_hopper_profile_differs_in_all_three_roles_and_two_of_them_fail_silently() {
    let amp = roles(&Ga10xHostClasses);
    let hop = roles(&Gh100HostClasses);

    assert_ne!(amp.0, hop.0, "channel");
    assert_ne!(amp.1, hop.1, "usermode");
    assert_ne!(amp.2, hop.2, "CE object");

    // ★★★ …and this is the half a `!=` cannot say. The Ampere channel and usermode ids are
    // IN GH100's class list, so picking them there is not refused — it is served. Only the
    // CE object is absent and fails loudly.
    assert!(
        GH100.gpfifo.contains(&amp.0),
        "GH100 lists AMPERE_CHANNEL_GPFIFO_A (g_gpu_class_list.c:1996) — the wrong pick \
         ALLOCATES. If this ever stops being true the seam is less urgent, not more"
    );
    assert!(
        GH100.usermode.contains(&amp.1),
        "GH100 lists AMPERE_USERMODE_A (g_gpu_class_list.c:1997) — the wrong pick ALLOCATES"
    );
    assert!(
        !GH100.ce.contains(&amp.2),
        "AMPERE_DMA_COPY_B is ABSENT from GH100's list — this is the ONE role of three \
         whose wrong pick fails at alloc"
    );
}

/// ★ Ada's answer is identical to GA10x's, and that is an ASSERTION rather than an
/// accident of copy-paste.
///
/// The crate docs already record Ada as the easy member of the universe for the GSP
/// register seam. It is the easy member here too — and if a future edit makes
/// `Ad10xHostClasses` differ, this test says so, which is the point: the sameness is
/// sourced (`g_gpu_class_list.c:1738/:1744/:1739-1743`), not assumed.
#[test]
fn the_ada_profile_is_identical_to_the_ampere_one_because_the_class_lists_are() {
    assert_eq!(roles(&Ga10xHostClasses), roles(&Ad10xHostClasses));

    // Non-vacuity in the other direction: AD106's transcribed lists must really be the
    // same rows as GA106's, or "identical because the class lists are" is a claim about
    // nothing.
    assert_eq!(AD106.gpfifo, GA106.gpfifo);
    assert_eq!(AD106.usermode, GA106.usermode);
    assert_eq!(AD106.ce, GA106.ce);
    assert_ne!(
        GH100.ce, GA106.ce,
        "★ NON-VACUITY: if every chip's CE list were the same, the two assertions above \
         would be true of a table that models no variation at all"
    );
}

/// ⊘ **The three roles are not interchangeable.** A profile that wired `usermode()` to the
/// channel class would pass a per-chip max check for neither, but a profile that wired
/// two roles to the *same* accessor could slip past a careless one. Pin the shape.
#[test]
fn the_three_roles_are_three_distinct_classes_within_every_profile() {
    for (chip, p) in cases() {
        let (c, u, e) = roles(p);
        assert_ne!(c, u, "{}: channel and usermode collapsed", chip.chip);
        assert_ne!(c, e, "{}: channel and CE object collapsed", chip.chip);
        assert_ne!(u, e, "{}: usermode and CE object collapsed", chip.chip);
    }
}

/// ★★ The pin is a pin, and it points at the measured part.
///
/// `pinned_host_classes` is the single decision the host adapter makes about generation.
/// It must be GA10x — not because GA10x is newest (it is not) but because it is the only
/// generation any host-path measurement in this project exists for.
#[test]
fn the_isolates_pinned_profile_is_the_one_generation_that_was_measured() {
    let pinned = roles(pinned_host_classes());
    assert_eq!(pinned, roles(&Ga10xHostClasses));
    assert_eq!(
        pinned.1, AMPERE_USERMODE_A,
        "★ the pinned usermode class is the one the C artifact's proven host self-test \
         allocated; changing it changes what a bring-up failure would mean"
    );
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

// ══════════════════════════════════════════════════════════════════════════════════════
// GB202 — the chip where the class table, the selection rule and a MEASUREMENT disagree
// ══════════════════════════════════════════════════════════════════════════════════════

/// ★★★ **The value oracle for the four Blackwell ids, and it is a DIFFERENT artifact.**
///
/// The four constants in `host_classes.rs` are typed by hand, because `kayfabe-abi`'s
/// generated class table carries no Blackwell row and this change may not edit that crate.
/// A typed class id is exactly the residue that module's own Ada/Hopper arms record as
/// forbidden — so it is pinned here against a table the profile does not read:
/// `kayfabe_abi::capability`'s vendored nvproxy surface, looked up **by NVIDIA's own
/// name**.
///
/// ⊘ What this can and cannot catch, stated rather than implied. It catches a **wrong
/// number** (a typo, a transposition, the `0xcbb5` that `nvkvm-pv` carried for
/// `BLACKWELL_DMA_COPY_A` for months and that NVIDIA does not ship at all). It cannot
/// catch two tables sharing one misreading, and it says nothing about whether a Blackwell
/// board accepts any of it.
#[test]
fn blackwell_ids_match_the_vendored_capability_table() {
    use kayfabe_abi::capability::CAPS_580_65_06;

    let want: [(&str, u32); 4] = [
        ("BLACKWELL_CHANNEL_GPFIFO_A", BLACKWELL_CHANNEL_GPFIFO_A),
        ("BLACKWELL_USERMODE_A", BLACKWELL_USERMODE_A),
        ("BLACKWELL_DMA_COPY_B", BLACKWELL_DMA_COPY_B),
        // ⊘ The compute id is not one of `roles()`' three, so it is unwrapped through the
        // trait's own `Option` rather than through the pinned helper.
        (
            "BLACKWELL_COMPUTE_B",
            Gb20xHostClasses
                .compute_object()
                .expect("the Blackwell profile declares a compute object")
                .compute_object_id()
                .0,
        ),
    ];

    let mut checked = 0usize;
    for (name, ours) in want {
        let theirs = CAPS_580_65_06
            .all_classes()
            .find(|e| e.name == name)
            .unwrap_or_else(|| {
                panic!(
                    "★ {name} is not in kayfabe_abi::capability's vendored table at all. \
                     Either the name is wrong or this oracle has stopped covering the \
                     generation — do NOT weaken the test; find the row"
                )
            });
        assert_eq!(
            ours, theirs.class,
            "★ {name}: this crate says {ours:#06x}, the vendored capability table says \
             {:#06x}. A hand-typed class id drifted — which is the whole reason this \
             test exists",
            theirs.class
        );
        checked += 1;
    }
    assert_eq!(
        checked, 4,
        "★ NON-VACUITY: four ids must have been compared, not {checked}"
    );
}

/// ★★★ **The deliberate departure from `findDeviceClasses`' max-in-family rule**, asserted
/// in both directions so neither half can be lost.
///
/// GB202's channel family holds two Blackwell members. The rule this file's main test
/// applies — `NV_MAX` (`ogkm-580: nv_gpu_ops.c:8684-8689`) — selects
/// `BLACKWELL_CHANNEL_GPFIFO_B` (`0xca6f`, `g_gpu_class_list.c:2846`). The profile answers
/// `BLACKWELL_CHANNEL_GPFIFO_A` (`0xc96f`, `:2845`) instead, because `_A` is the id a
/// **real RTX 5090's RM accepted** — `alloc hClass=0xc96f ap_size=368 status=0x0`,
/// `/workspace/nvkvm-pv/tests/BOOT_MATRIX.md:1050-1062` — and `_B` is an id nothing has
/// ever sent to a board.
///
/// ★ Measured beats derived. But a departure nobody wrote down becomes a bug six months
/// later, so this test pins **both** the choice and the rule it departs from: if either
/// changes, it goes red and somebody has to decide again.
#[test]
fn the_blackwell_channel_pick_departs_from_the_max_rule_because_a_5090_answered() {
    let (chan, user, ce) = roles(&Gb20xHostClasses);

    assert_eq!(
        chan, BLACKWELL_CHANNEL_GPFIFO_A,
        "★ the profile must answer the MEASURED channel class"
    );
    assert_eq!(
        newest(GB202.gpfifo),
        BLACKWELL_CHANNEL_GPFIFO_B,
        "★ NON-VACUITY: if the max rule ever agreed with the measurement, this test would \
         be recording a departure that no longer exists"
    );
    assert_ne!(
        chan,
        newest(GB202.gpfifo),
        "★ the departure is the point — see this test's docs"
    );

    // The other two roles DO follow the rule, and that is worth asserting: a profile that
    // departed everywhere would mean the rule had simply been abandoned.
    assert_eq!(
        user,
        newest(GB202.usermode),
        "usermode follows the max rule"
    );
    assert_eq!(ce, newest(GB202.ce), "CE object follows the max rule");
    assert_eq!(
        ce, BLACKWELL_DMA_COPY_B,
        "★ …and the CE object is ALSO the measured one — `alloc hClass=0xcab5 ap_size=8 \
         status=0x0` on the same 5090. Rule and measurement agree here"
    );
}

/// ★★ Every Blackwell role differs from Ampere's, and — unlike Hopper — the wrong pick is
/// **loud in two of the three**, because GB202's CE family carries no Ampere class at all.
#[test]
fn the_blackwell_profile_differs_in_every_role_and_the_ce_pick_fails_loudly() {
    let amp = roles(&Ga10xHostClasses);
    let bw = roles(&Gb20xHostClasses);
    assert_ne!(amp.0, bw.0, "channel");
    assert_ne!(amp.1, bw.1, "usermode");
    assert_ne!(amp.2, bw.2, "CE object");

    // Same shape as Hopper: the legacy channel and usermode ids ARE listed, so the wrong
    // pick allocates and carries the wrong notifier geometry with no status to say so.
    assert!(
        GB202.gpfifo.contains(&amp.0),
        "GB202 lists AMPERE_CHANNEL_GPFIFO_A (g_gpu_class_list.c:2839) — the wrong pick \
         ALLOCATES"
    );
    assert!(
        GB202.usermode.contains(&amp.1),
        "GB202 lists AMPERE_USERMODE_A (:2840) — the wrong pick ALLOCATES"
    );
    assert!(
        !GB202.ce.contains(&amp.2),
        "AMPERE_DMA_COPY_B is ABSENT from GB202's list (:2855-2862 carries only \
         BLACKWELL_DMA_COPY_B) — the CE pick is the one that fails at alloc"
    );
    // ⊘ And Hopper's CE class is absent too, so "inherit the previous generation" is wrong
    // here in a way it was not for Ada.
    assert!(
        !GB202.ce.contains(&HOPPER_DMA_COPY_A),
        "HOPPER_DMA_COPY_A must also be absent from GB202, or 'each generation needs its \
         own CE class' is not what this table shows"
    );
}

/// ⊘ **`findDeviceClasses` cannot answer the compute role on this chip at all** — recorded
/// so the compute id is never mistaken for a rule-derived one.
///
/// `isClassCompute` at 580 enumerates nothing past `HOPPER_COMPUTE_A`
/// (`ogkm-580: nv_gpu_ops.c:8584-8603`), and GB202's class list carries no compute class
/// other than `BLACKWELL_COMPUTE_B` (`g_gpu_class_list.c:2847-2854`). So on a Blackwell
/// board that function leaves `computeClass` at **zero**. The profile's compute id comes
/// from the class list alone — a strictly weaker instrument than the one behind the
/// channel and CE rows, and the difference is the reason this test exists rather than a
/// comment.
#[test]
fn the_blackwell_compute_class_comes_from_the_class_list_not_the_selection_rule() {
    // The 580 `isClassCompute` switch, transcribed. ★ Deliberately a literal list and not
    // a range: the defect it models is an ENUMERATION that stopped being extended.
    const IS_CLASS_COMPUTE_580: &[u32] = &[
        0x0000_b0c0, // MAXWELL_COMPUTE_A
        0x0000_b1c0, // MAXWELL_COMPUTE_B
        0x0000_c0c0, // PASCAL_COMPUTE_A
        0x0000_c1c0, // PASCAL_COMPUTE_B
        0x0000_c3c0, // VOLTA_COMPUTE_A
        0x0000_c4c0, // VOLTA_COMPUTE_B
        0x0000_c5c0, // TURING_COMPUTE_A
        0x0000_c6c0, // AMPERE_COMPUTE_A
        0x0000_c7c0, // AMPERE_COMPUTE_B
        0x0000_cbc0, // HOPPER_COMPUTE_A
    ];
    let ours = Gb20xHostClasses
        .compute_object()
        .expect("the Blackwell profile declares a compute object")
        .compute_object_id()
        .0;
    assert!(
        !IS_CLASS_COMPUTE_580.contains(&ours),
        "★ if BLACKWELL_COMPUTE_B were in `isClassCompute`, the rule WOULD answer and this \
         test's premise is stale — re-derive it against the current ogkm rather than \
         deleting it"
    );
    // Non-vacuity: the transcribed switch must actually cover the generations it claims to,
    // or "Blackwell is missing from it" is a statement about an empty list.
    assert!(
        IS_CLASS_COMPUTE_580.contains(&0x0000_c7c0),
        "★ NON-VACUITY: AMPERE_COMPUTE_B must be in the transcribed switch"
    );
    assert!(
        !IS_CLASS_COMPUTE_580.contains(&0x0000_c9c0),
        "★ ADA_COMPUTE_A is missing from it too — Blackwell is the second instance of this \
         gap, not the first, and the Ada profile has the same provenance caveat"
    );
}
