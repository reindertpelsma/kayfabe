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
    Ad10xHostClasses, Ga10xHostClasses, Gh100HostClasses, host_classes::pinned_host_classes,
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
    let pairs: [(&dyn Arch, &dyn HostClasses); 3] = [
        (&kayfabe_chips::Ga10xArch::default(), &Ga10xHostClasses),
        (&kayfabe_chips::Ad10xArch::default(), &Ad10xHostClasses),
        (&kayfabe_chips::Gh100Arch::default(), &Gh100HostClasses),
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

/// ★★★ All three generations decode a work-submit token **identically**, and none of them
/// answers with `MockArch`'s invented encoding any more.
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
fn every_generation_decodes_a_work_submit_token_the_same_way_and_none_uses_the_mocks() {
    use kayfabe_arch::Arch;
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
