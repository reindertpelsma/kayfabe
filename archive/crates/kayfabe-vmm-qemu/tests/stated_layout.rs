//! ★★★ Step 2 of the guest-RAM crossing: **the layout is STATED, never DERIVED.**
//!
//! `guest_ram_crossing.md` §5.6 closed step 1 with the line these tests exist to enforce —
//! *a census yields an EXTENT, and an extent is not a LAYOUT.* Step 1 learned how big guest
//! RAM is. Nothing in that says which guest-physical address is which byte of the file.
//!
//! # What is actually at stake, and why a unit test is the right instrument here
//!
//! The tempting shortcut is identity: assume guest-physical *is* the file offset.
//! `[measured 2026-08-10, bench `vh`, boot `w224m`]` — `traces/guest_boots/run_w224m_mtree.log`
//! shows exactly that on the bench's one command line: 12 `ram0` ranges, 0 non-identity,
//! nothing at or above 4 GiB. It is true, for `-m 2048` on `q35`. With `-m 8G` the hypervisor
//! splits RAM around the 4 GiB PCI hole and the identity breaks at the split.
//!
//! ★ And it is not even the whole truth at `-m 2048`: `[measured 2026-08-10, bench `vh`, boot
//! `w225f`, rev e1e57f6]` the hypervisor stated **four** runs, not one, because the legacy and
//! SMRAM holes punch three gaps out of the low 1 MiB.
//!
//! ⊘ That failure would be **silent**. A derived offset is a plausible number; the isolate
//! would map it, the descriptor would be built over it, and the guest would read somebody
//! else's page. There is no status code for it and no log line, which is why the property
//! has to be asserted rather than observed on a boot.
//!
//! So the negative cases below are the point of the file, not its trimmings: the split
//! layout, the address in no run, and the request that runs off the end of one.

mod common;

use kayfabe_vmm_qemu::host::{SectionBacking, SectionFacts};
use kayfabe_vmm_qemu::layout::{BackingId, GuestRamLayout, LayoutRefusal, StatedRun};

/// The block a census would have adopted.
const RAM: BackingId = BackingId {
    dev: 0x21,
    ino: 777,
};
/// Some other `is_ram` block in the same machine — video RAM is the real-world instance.
const OTHER: BackingId = BackingId {
    dev: 0x21,
    ino: 778,
};

const GIB: u64 = 1 << 30;

fn run(gpa: u64, len: u64, file_offset: u64) -> StatedRun {
    StatedRun {
        gpa,
        len,
        file_offset,
    }
}

/// ★★★ **The `-m 8G` shape**, which is the whole reason this module exists.
///
/// The hypervisor states two runs: everything below the 4 GiB PCI hole, and the remainder
/// re-based above it. The second run's file offset is **not** its guest-physical address —
/// the bytes continue where the first run left off — so an identity assumption is off by
/// exactly the size of the hole, silently.
#[test]
fn the_split_layout_is_answered_from_the_statement_and_identity_would_have_been_wrong() {
    let mut l = GuestRamLayout::new();
    l.state(RAM, run(0, 3 * GIB, 0));
    l.state(RAM, run(4 * GIB, 5 * GIB, 3 * GIB));

    // Below the hole the two happen to agree, and that agreement is a coincidence about
    // this run rather than a rule.
    let below = l.resolve(RAM, 0x1000, 0x1000).expect("stated");
    assert_eq!(below.file_offset, 0x1000);
    assert!(below.is_identity());

    // ★ Above the hole they do not, and this is the assertion an identity map fails.
    let above = l.resolve(RAM, 4 * GIB + 0x2000, 0x1000).expect("stated");
    assert_eq!(
        above.file_offset,
        3 * GIB + 0x2000,
        "the byte at gpa 4GiB+0x2000 is at file offset 3GiB+0x2000, because that is what the \
         hypervisor STATED. An identity map answers 4GiB+0x2000 — one whole PCI hole wrong, \
         with no error anywhere."
    );
    assert!(!above.is_identity());
}

/// ⊘ The hole itself is stated by nobody, and is refused by name rather than interpolated.
#[test]
fn an_address_in_the_pci_hole_is_refused_by_name_and_never_interpolated() {
    let mut l = GuestRamLayout::new();
    l.state(RAM, run(0, 3 * GIB, 0));
    l.state(RAM, run(4 * GIB, 5 * GIB, 3 * GIB));

    let err = l
        .resolve(RAM, 3 * GIB + 0x1000, 0x1000)
        .expect_err("the hole is not RAM");
    assert_eq!(err.name(), "NoStatedRun");
    assert!(matches!(err, LayoutRefusal::NoStatedRun { .. }));
}

/// ★★ A request that begins in a run and leaves it is refused, ⊘ **not clamped**.
///
/// Clamping is the dangerous answer, not the merely-imprecise one: the caller asked to pin
/// `len` bytes for a device to write into, and a shorter success is a buffer the hardware
/// will run off the end of. This is the `dlen=0` lesson one layer over — an answer that is
/// *shaped* right and *short* is worse than a refusal.
#[test]
fn a_range_that_leaves_its_run_is_refused_and_never_truncated() {
    let mut l = GuestRamLayout::new();
    l.state(RAM, run(0, 3 * GIB, 0));
    l.state(RAM, run(4 * GIB, 5 * GIB, 3 * GIB));

    let err = l
        .resolve(RAM, 3 * GIB - 0x1000, 0x8000)
        .expect_err("it leaves the run");
    match err {
        LayoutRefusal::StraddlesRuns { available, len, .. } => {
            assert_eq!(available, 0x1000);
            assert_eq!(len, 0x8000);
        }
        other => panic!("expected StraddlesRuns, got {other:?}"),
    }
}

/// ★★ RAM backed by a **different** file gets its own name.
///
/// "This address is video RAM" and "nothing is here" are different facts with different
/// fixes, and a caller that logs one when the other happened sends the next person to the
/// wrong plane.
#[test]
fn ram_backed_by_another_file_is_a_distinct_refusal_from_nothing_being_there() {
    let mut l = GuestRamLayout::new();
    l.state(RAM, run(0, 2 * GIB, 0));
    l.state(OTHER, run(0xe000_0000, 0x0100_0000, 0));

    assert_eq!(
        l.resolve(RAM, 0xe000_1000, 0x1000)
            .expect_err("not our block")
            .name(),
        "OtherBacking"
    );
    assert_eq!(
        l.resolve(RAM, 0xffff_0000_0000, 0x1000)
            .expect_err("nobody stated this")
            .name(),
        "NoStatedRun"
    );
    // ⊘ And the other block's own runs are not visible as ours.
    assert_eq!(l.contiguous_runs(RAM), vec![run(0, 2 * GIB, 0)]);
}

/// ★★★ **The empty layout refuses everything**, and that is the default.
///
/// A build whose join never happened has no runs, and every address is refused. ⊘ There is
/// deliberately no "assume identity when nothing is stated" fallback: that fallback is keyed
/// on our own ignorance, and it would turn a broken join into a working-looking boot.
#[test]
fn nothing_stated_means_everything_refused_with_no_identity_fallback() {
    let l = GuestRamLayout::new();
    for gpa in [0u64, 0x1000, 4 * GIB, 0x7fff_ffff_f000] {
        assert_eq!(
            l.resolve(RAM, gpa, 0x1000)
                .expect_err("nothing stated")
                .name(),
            "NoStatedRun",
            "gpa {gpa:#x} must be refused, not assumed to be its own file offset"
        );
    }
    assert!(l.contiguous_runs(RAM).is_empty());
    assert_eq!(l.stated_sections(), 0);
}

/// ★★ Adjacent sections coalesce **only** when contiguous in both axes.
///
/// The hypervisor slices its flat view wherever anything changes, including things with no
/// bearing on where the bytes are — that is why one 2 GiB block arrives as a dozen sections.
/// Coalescing them is what lets step 3 build one descriptor per real run. ⊘ But a pair
/// contiguous in guest-physical space and *not* in the file must stay two runs: one
/// descriptor over that pair would have its second half somewhere else entirely.
#[test]
fn sections_coalesce_only_when_both_axes_are_contiguous() {
    let mut both = GuestRamLayout::new();
    both.state(RAM, run(0, 0x1000, 0));
    both.state(RAM, run(0x1000, 0x1000, 0x1000));
    both.state(RAM, run(0x2000, 0x1000, 0x2000));
    assert_eq!(
        both.contiguous_runs(RAM),
        vec![run(0, 0x3000, 0)],
        "three sections of one block are one run"
    );
    assert_eq!(
        both.stated_sections(),
        3,
        "coalescing does not lose sections"
    );

    let mut gpa_only = GuestRamLayout::new();
    gpa_only.state(RAM, run(0, 0x1000, 0));
    // Contiguous in guest-physical space, and NOT in the file.
    gpa_only.state(RAM, run(0x1000, 0x1000, 0x9000));
    assert_eq!(
        gpa_only.contiguous_runs(RAM),
        vec![run(0, 0x1000, 0), run(0x1000, 0x1000, 0x9000)],
        "contiguity in one axis is not contiguity"
    );
}

/// ★ A range spanning two *coalesced* sections is served, because the bytes really are
/// contiguous — the section boundary was the hypervisor's bookkeeping, not a fact about the
/// file. Refusing it would refuse the ordinary case.
#[test]
fn a_range_spanning_two_adjacent_sections_of_one_block_is_served() {
    let mut l = GuestRamLayout::new();
    l.state(RAM, run(0, 0x1000, 0));
    l.state(RAM, run(0x1000, 0x1000, 0x1000));
    let r = l.resolve(RAM, 0x800, 0x1000).expect("one contiguous run");
    assert_eq!(r.file_offset, 0x800);
    assert_eq!(r.len, 0x1000);
}

/// ★ A withdrawn statement stops being answerable **immediately**.
///
/// A layout row outliving its region is a range this device would keep resolving after the
/// hypervisor stopped backing it — the memory-plane equivalent of a stale mapping.
#[test]
fn a_forgotten_run_is_refused_again() {
    let mut l = GuestRamLayout::new();
    l.state(RAM, run(0x1000, 0x1000, 0x1000));
    assert!(l.resolve(RAM, 0x1000, 0x100).is_ok());
    l.forget(0x1000);
    assert_eq!(
        l.resolve(RAM, 0x1000, 0x100).expect_err("withdrawn").name(),
        "NoStatedRun"
    );
}

/// ⊘ Degenerate requests get names too, rather than a mapping nobody can use.
#[test]
fn a_zero_length_request_and_a_wrapping_one_are_both_named() {
    let mut l = GuestRamLayout::new();
    l.state(RAM, run(0, 0x1000, 0));
    assert_eq!(
        l.resolve(RAM, 0, 0).expect_err("empty").name(),
        "EmptyRange"
    );
    assert_eq!(
        l.resolve(RAM, u64::MAX - 0x10, 0x100)
            .expect_err("wraps")
            .name(),
        "OutOfSpace"
    );
}

/// ★★ The **identity** case is reported as an observation and never acted on.
///
/// This is the assertion that keeps the shortcut out: `is_identity` exists so a boot log can
/// say what it saw, and nothing in the module may branch on it. If resolution ever consulted
/// it, the split layout above would still pass while the guarantee was gone — so the test
/// that matters is that a *non*-identity run resolves by its stated offset, which
/// `the_split_layout_...` above asserts directly.
#[test]
fn identity_is_reported_but_a_non_identity_run_resolves_by_its_statement() {
    let mut l = GuestRamLayout::new();
    l.state(RAM, run(0x8000, 0x1000, 0x8000));
    l.state(RAM, run(0x9000, 0x1000, 0x4_0000));
    let runs = l.contiguous_runs(RAM);
    assert_eq!(
        runs.len(),
        2,
        "not contiguous in the file, so not coalesced"
    );
    assert!(runs[0].is_identity());
    assert!(!runs[1].is_identity());
    assert_eq!(
        l.resolve(RAM, 0x9800, 0x100).expect("stated").file_offset,
        0x4_0800
    );
}

// =====================================================================================
// ★★★ Through the real listener path — the unit tests above assert the RULE, these assert
// that the rule is actually reached by the callback the hypervisor calls.
// =====================================================================================

/// ★★★ A reported section states a run, and the run is answerable through the machine.
///
/// ⊘ This is the non-vacuity half of the module. Every assertion above operates on a
/// `GuestRamLayout` a test filled in by hand; if `region_add` never called `state`, all of
/// them would still pass and nothing would ever be stated on a boot. This is the test that
/// fails in that world.
#[test]
fn a_reported_backed_section_states_a_run_the_machine_can_resolve() {
    let (m, host, _slots) = common::machine();
    let backing = SectionBacking {
        dev: 0x21,
        ino: 4242,
        file_offset_of_region: 0,
    };
    let id = BackingId::new(backing.dev, backing.ino);

    assert_eq!(
        m.stated_guest_ram(id).len(),
        0,
        "nothing is stated before the listener says anything"
    );
    m.region_add(host.mint_backed(
        common::FOREIGN_RAM,
        4 * common::page(),
        common::ram_facts(),
        backing,
    ))
    .expect("a reported RAM section");

    assert_eq!(
        m.stated_guest_ram(id),
        vec![StatedRun {
            gpa: common::FOREIGN_RAM,
            len: 4 * common::page(),
            file_offset: 0,
        }]
    );
    let r = m
        .resolve_guest_ram(id, common::FOREIGN_RAM + common::page(), common::page())
        .expect("inside the stated run");
    assert_eq!(r.file_offset, common::page());
}

/// ★★ `file_offset_of_region` is **added**, not ignored.
///
/// A region placed at a non-zero offset into its descriptor is a thing a hypervisor is
/// allowed to do, and every `memory-backend-memfd` this bench has booted has it at zero —
/// which is exactly why nothing would have caught the omission. The mock states a non-zero
/// one so the addition is exercised at least once.
#[test]
fn the_regions_own_offset_into_the_file_is_added_to_the_sections() {
    let (m, host, _slots) = common::machine();
    let backing = SectionBacking {
        dev: 0x21,
        ino: 4243,
        file_offset_of_region: 0x10_0000,
    };
    let id = BackingId::new(backing.dev, backing.ino);
    m.region_add(host.mint_backed(
        common::FOREIGN_RAM,
        4 * common::page(),
        common::ram_facts(),
        backing,
    ))
    .expect("a reported RAM section");

    assert_eq!(
        m.resolve_guest_ram(id, common::FOREIGN_RAM, common::page())
            .expect("stated")
            .file_offset,
        0x10_0000,
        "the section is at offset 0 within its region, and the REGION is at 0x100000 in the \
         file; answering 0 would map the wrong megabyte"
    );
}

/// ⊘ A section with **no** reported backing states nothing — `None` is unmeasured, not a
/// fact about the range, so every address in it is refused rather than attributed to
/// whichever block the caller happened to ask about.
#[test]
fn a_section_with_no_reported_backing_states_nothing() {
    let (m, host, _slots) = common::machine();
    m.region_add(host.mint_foreign(common::FOREIGN_RAM, 4 * common::page(), common::ram_facts()))
        .expect("a reported RAM section");

    let id = BackingId::new(0x21, 4244);
    assert!(m.stated_guest_ram(id).is_empty());
    assert_eq!(
        m.resolve_guest_ram(id, common::FOREIGN_RAM, common::page())
            .expect_err("nothing was stated")
            .name(),
        "NoStatedRun"
    );
    assert_eq!(
        m.stated_sections(),
        0,
        "an unbacked section is not counted as a statement"
    );
}

/// ★★ A deleted region withdraws its statement, through the real callback.
#[test]
fn region_del_withdraws_the_statement() {
    let (m, host, _slots) = common::machine();
    let backing = SectionBacking {
        dev: 0x21,
        ino: 4245,
        file_offset_of_region: 0,
    };
    let id = BackingId::new(backing.dev, backing.ino);
    m.region_add(host.mint_backed(
        common::FOREIGN_RAM,
        4 * common::page(),
        common::ram_facts(),
        backing,
    ))
    .expect("added");
    assert_eq!(m.stated_sections(), 1);

    m.region_del(common::FOREIGN_RAM, 4 * common::page());
    assert_eq!(m.stated_sections(), 0);
    assert!(m.stated_guest_ram(id).is_empty());
    assert_eq!(
        m.resolve_guest_ram(id, common::FOREIGN_RAM, common::page())
            .expect_err("withdrawn")
            .name(),
        "NoStatedRun"
    );
}

/// ★★★ **The transience finding, as a test.**
///
/// `[measured 2026-08-10, boots w225c/w225d/w225e]` the live layout is empty at *both*
/// instants this device can conveniently report at — at memory-plane attach the listener's
/// address space has not been enabled by the guest yet, and at the exit notifier teardown has
/// already replayed `region_del` over every range. In between it was correct: 8 backed
/// sections coalescing to 4 runs over 2 147 135 488 bytes.
///
/// ⊘ So the resolver and the report must read **different** tables, and this test is what
/// keeps them apart. If `resolve` ever started answering from the `ever` table it would keep
/// serving ranges the hypervisor had stopped backing — a stale mapping with a plausible
/// offset, which is the whole failure class this module exists to refuse.
#[test]
fn a_withdrawn_run_leaves_the_evidence_and_leaves_the_resolver() {
    let mut l = GuestRamLayout::new();
    l.state(RAM, run(0x1000, 0x1000, 0x1000));
    l.state(RAM, run(0x2000, 0x1000, 0x2000));
    assert_eq!(l.contiguous_runs(RAM), vec![run(0x1000, 0x2000, 0x1000)]);

    l.forget(0x1000);
    l.forget(0x2000);

    // The evidence survives, coalesced exactly as it was.
    assert_eq!(
        l.contiguous_runs_ever(RAM),
        vec![run(0x1000, 0x2000, 0x1000)],
        "a finished boot log must still be able to say what was stated"
    );
    // ⊘ And the resolver does not.
    assert!(l.contiguous_runs(RAM).is_empty());
    assert_eq!(
        l.resolve(RAM, 0x1000, 0x100)
            .expect_err("withdrawn ranges are not resolvable")
            .name(),
        "NoStatedRun"
    );
    assert_eq!(l.census().forgotten, 2);
}

/// ★★ The funnel counts every section, and counts the stages independently.
///
/// 0-of-0 and 12-of-0 are different defects in different files; a single "stated" number
/// cannot tell them apart, and the first armed boot of this module reported exactly the
/// ambiguous form.
#[test]
fn the_section_funnel_counts_all_three_stages_separately() {
    let (m, host, _slots) = common::machine();
    // A device section: reported, not RAM.
    m.region_add(host.mint_foreign(0x2000_0000, common::page(), SectionFacts::device()))
        .expect("a device section");
    // RAM with no backing: reported, RAM, unbacked.
    m.region_add(host.mint_foreign(common::FOREIGN_RAM, common::page(), common::ram_facts()))
        .expect("unbacked RAM");
    // RAM with a backing: all three.
    m.region_add(host.mint_backed(
        common::FOREIGN_RAM + common::page(),
        common::page(),
        common::ram_facts(),
        SectionBacking {
            dev: 0x21,
            ino: 909,
            file_offset_of_region: 0,
        },
    ))
    .expect("backed RAM");

    let c = m.layout_census();
    assert_eq!((c.seen, c.ram, c.backed, c.forgotten), (3, 2, 1, 0));
}

// =====================================================================================
// ★★★★★ §5.8 — THE DATA-PLANE HANDLE ASKS THE SAME TABLE, AND THE NEGATIVE CONTROL FIRES
// =====================================================================================

/// ★★★★★ **`QemuVmm` answers the layout identically to `QemuMachine`, and a GPA outside
/// every stated run is REFUSED BY NAME through it.**
///
/// # Why the accessor exists at all, and why that needed a test rather than a comment
///
/// The production caller of the guest-RAM pin sits on the **doorbell** path, which holds a
/// [`QemuVmm`] and does not hold a [`QemuMachine`]. Giving it the layout could have been
/// done two ways: carry a second copy of the table to where it is needed, or reach the one
/// that exists. This project has measured **two projections of one fact disagreeing three
/// times**, so the second way was taken — and this test is what makes "it is the same
/// table" a checked statement instead of an assertion about `Arc`s.
///
/// # ★★★ And the negative control, which is the half a reader should look at first
///
/// The pin's whole safety argument is that a guest-physical address the hypervisor never
/// stated is refused rather than assumed to be its own file offset. That refusal is
/// asserted here, **through the handle the production caller actually uses** — because a
/// control taken through a different accessor would be a control over different code.
#[test]
fn the_vmm_handle_answers_the_same_layout_and_refuses_outside_it_by_name() {
    let (m, host, _slots) = common::machine();
    let backing = SectionBacking {
        dev: 0x21,
        ino: 4244,
        file_offset_of_region: 0x20_0000,
    };
    let id = BackingId::new(backing.dev, backing.ino);
    m.region_add(host.mint_backed(
        common::FOREIGN_RAM,
        4 * common::page(),
        common::ram_facts(),
        backing,
    ))
    .expect("a reported RAM section");

    let vmm = m.vmm();
    assert_eq!(
        vmm.stated_guest_ram(id),
        m.stated_guest_ram(id),
        "one table, two accessors — not two tables"
    );
    let inside = common::FOREIGN_RAM + common::page();
    assert_eq!(
        vmm.resolve_guest_ram(id, inside, common::page()),
        m.resolve_guest_ram(id, inside, common::page()),
        "and the resolver agrees byte for byte, which is the property the doorbell path \
         depends on"
    );
    assert_eq!(
        vmm.resolve_guest_ram(id, inside, common::page())
            .expect("inside")
            .file_offset,
        0x20_0000 + common::page(),
        "⊘ the offset is the FILE's, region base included — not the guest-physical address"
    );

    // ★★★ THE NEGATIVE CONTROL, derived exactly as the production caller derives it: one
    // page past the top of the highest stated run, so it is outside by construction on any
    // machine and any `-m`.
    let top = vmm
        .stated_guest_ram(id)
        .iter()
        .map(|r| r.gpa_end())
        .max()
        .expect("a run was stated");
    let outside = u64::try_from(top).expect("fits") + common::page();
    let refused = vmm
        .resolve_guest_ram(id, outside, common::page())
        .expect_err(
            "★ a GPA in no stated run must be REFUSED, never clamped and never \
                     assumed to be its own file offset",
        );
    assert_eq!(
        refused.name(),
        "NoStatedRun",
        "and refused BY NAME — `{refused:?}`; a bare `None` here would leave the caller \
         unable to tell 'nothing is here' from 'this is video RAM'"
    );
}
