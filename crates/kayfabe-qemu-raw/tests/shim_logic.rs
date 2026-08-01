//! ★★ The shim's decisions, driven with **no hypervisor present**.
//!
//! `l2_qemu_adapter.md` §10's stage table makes a promise the rest of the milestone rests on:
//! the adapter must never become a *reason* for a test to need a machine. This file is that
//! promise applied to the raw crate. Everything here runs against
//! [`MockQemuHost`](kayfabe_vmm_qemu::mock_host::MockQemuHost) and
//! [`MockSlotPlane`](kayfabe_vmm_qemu::mock_host::MockSlotPlane); the C half's own acceptance
//! is a separate, coarser instrument that needs a hypervisor binary and gets one.
//!
//! **Every assertion below names an exact variant and an exact sentence.** `is_err()` would
//! pass for the wrong refusal, and this crate's whole job is to keep two near-neighbour
//! refusals apart across a seam that flattens everything to an `int32_t`.

use std::sync::Arc;

use kayfabe_qemu_raw::shim::{BarDesc, SectionWire, Shim, ShimConfig, Status};
use kayfabe_vmm::BarId;
use kayfabe_vmm_qemu::host::{SectionDesc, SectionFacts};
use kayfabe_vmm_qemu::mock_host::{MockPolicy, MockQemuHost, MockSlotPlane};
use kayfabe_vmm_qemu::{
    BAR_MOVED_UNDER_US, BELOW_FLOOR, MEMORY_PLANE_AFTER_UNREALIZE, NOT_ACCELERATED,
    WINDOW_IN_A_BACKED_BAR,
};

/// Guest-physical bases well inside every host's physical-address width. ★ Not a
/// convenience: the accelerator refuses a guest-physical address above the host CPU's own
/// physical-address width, which is 46 bits on some of the machines this suite runs on, and a
/// hard-coded 47-bit constant has already blamed the allocator for a CPU fact here.
const BAR0_BASE: u64 = 0x0000_0000_C000_0000;
const BAR1_BASE: u64 = 0x0000_0004_0000_0000;
const BAR2_BASE: u64 = 0x0000_0008_0000_0000;
const PAGE: u64 = 4096;

fn cfg() -> ShimConfig {
    ShimConfig {
        shareable_ram: true,
        bars: vec![
            BarDesc {
                index: 0,
                base: BAR0_BASE,
                len: 16 << 20,
            },
            BarDesc {
                index: 1,
                base: BAR1_BASE,
                len: 1 << 30,
            },
            BarDesc {
                index: 2,
                base: BAR2_BASE,
                len: 1 << 30,
            },
        ],
    }
}

fn host_with(policy: MockPolicy) -> Arc<MockQemuHost> {
    let h = Arc::new(MockQemuHost::with_policy(policy));
    h.place_bar(BarId::Bar0, BAR0_BASE);
    h.place_bar(BarId::Bar1, BAR1_BASE);
    h.place_bar(BarId::Bar2, BAR2_BASE);
    h
}

fn slots() -> Arc<MockSlotPlane> {
    Arc::new(MockSlotPlane::new(509, PAGE))
}

fn realize_with(policy: MockPolicy) -> Result<(Shim, Arc<MockQemuHost>), (Status, &'static str)> {
    let host = host_with(policy);
    let shim = Shim::realize(&cfg(), host.clone(), slots())?;
    Ok((shim, host))
}

// =====================================================================================
// The floor and the accelerator — two refusals that are NEAR NEIGHBOURS and must not merge
// =====================================================================================

#[test]
fn a_hypervisor_below_the_floor_is_refused_by_name() {
    let err = realize_with(MockPolicy {
        version: (9, 2),
        ..MockPolicy::default()
    })
    .expect_err("a 9.2 hypervisor is below the floor and realize must refuse it");
    assert_eq!(err, (Status::Unsupported, BELOW_FLOOR));
}

#[test]
fn the_floor_is_a_floor_and_not_an_equality() {
    // ★ Non-vacuity for the test above: if the check were `!= (10, 2)` rather than
    // `< (10, 2)`, that refusal test would still pass and every later release would break.
    let (_shim, _host) = realize_with(MockPolicy {
        version: (11, 0),
        ..MockPolicy::default()
    })
    .expect("a release above the floor must realize");
}

#[test]
fn an_unaccelerated_machine_is_refused_by_a_different_name() {
    let err = realize_with(MockPolicy {
        kvm_enabled: false,
        ..MockPolicy::default()
    })
    .expect_err("an unaccelerated machine must be refused");
    assert_eq!(err, (Status::Unsupported, NOT_ACCELERATED));
    // The two carry the same status class on purpose — both mean "retrying cannot help" —
    // so the SENTENCE is the only thing that separates them, and it must.
    assert_ne!(NOT_ACCELERATED, BELOW_FLOOR);
}

// =====================================================================================
// The register table is the enumeration, so it must not be able to contradict itself
// =====================================================================================

#[test]
fn a_register_index_this_port_does_not_name_is_malformed_not_refused() {
    let mut c = cfg();
    c.bars.push(BarDesc {
        index: 5,
        base: 0x1_0000_0000,
        len: PAGE,
    });
    let err = Shim::realize(&c, host_with(MockPolicy::default()), slots())
        .expect_err("a register index outside this port must be refused");
    assert_eq!(err.0, Status::Malformed);
    assert_ne!(err.0, Status::Refused);
    assert!(
        err.1.contains("index this port does not name"),
        "the sentence must name the problem, got: {}",
        err.1
    );
}

#[test]
fn a_register_table_that_names_one_register_twice_is_refused() {
    let mut c = cfg();
    c.bars.push(BarDesc {
        index: 1,
        base: BAR2_BASE,
        len: PAGE,
    });
    let err = Shim::realize(&c, host_with(MockPolicy::default()), slots())
        .expect_err("a duplicated register must be refused");
    assert_eq!(err.0, Status::Malformed);
    assert!(err.1.contains("cannot contradict itself"), "got: {}", err.1);
}

// =====================================================================================
// §8.5's -EBUSY arm — an operator's mistake, reported apart from every other refusal
// =====================================================================================

#[test]
fn the_busy_arm_is_its_own_status_and_not_a_plain_refusal() {
    use kayfabe_vmm_qemu::host::HostError;

    let err = realize_with(MockPolicy {
        discard_refuses: Some(HostError::Busy {
            what: "a memory balloon with free-page reporting",
        }),
        ..MockPolicy::default()
    })
    .expect_err("a discard requirer must make realize refuse");
    assert_eq!(err.0, Status::Busy);
    assert_ne!(err.0, Status::Refused);
    assert!(
        err.1.contains("balloon"),
        "the sentence must name what conflicts, got: {}",
        err.1
    );
}

#[test]
fn an_ordinary_host_refusal_is_refused_and_not_busy() {
    use kayfabe_vmm_qemu::host::HostError;

    // ★ The other half of the pair. A `Busy` reported as `Refused` would pass the test above
    // only if this one also passed, and vice versa — neither alone pins the split.
    let err = realize_with(MockPolicy {
        blocker_refuses: Some(HostError::Refused {
            what: "adding a migration blocker",
            errno: Some(1),
        }),
        ..MockPolicy::default()
    })
    .expect_err("a refused migration blocker must make realize refuse");
    assert_eq!(err.0, Status::Refused);
    assert_ne!(err.0, Status::Busy);
}

#[test]
fn a_runtime_refusal_carrying_the_same_number_is_not_upgraded_to_busy() {
    // ★★ The imprecision of `classify_realize`, pinned rather than left to be discovered.
    // The class is reconstructed from an errno, and that reconstruction is exact only at
    // realize. `classify` — everything else — must stay blunt: a kernel that returns the
    // same number for a memslot is not an operator's configuration mistake.
    use kayfabe_qemu_raw::shim::{classify, classify_realize};
    use kayfabe_vmm::VmmError;

    let e = VmmError::HostRefused {
        what: "a memslot install",
        errno: Some(kayfabe_vmm_qemu::slots::KERNEL_EBUSY),
    };
    assert_eq!(classify(&e).0, Status::Refused);
    assert_eq!(classify_realize(&e).0, Status::Busy);
}

// =====================================================================================
// The reservation register — the whole safety argument, asked rather than assumed
// =====================================================================================

#[test]
fn a_reservation_in_a_register_the_hypervisor_backs_is_refused_by_name() {
    let (shim, _host) = realize_with(MockPolicy {
        // Only register 2 is a pure-MMIO reservation; a reservation aimed at register 1 must
        // be refused, because a backed register gets a hypervisor-managed slot of its own
        // over the same guest-physical range and only one of the two can win.
        reservation_bars: vec![BarId::Bar2],
        ..MockPolicy::default()
    })
    .expect("realize");
    let err = shim
        .install_window(BAR1_BASE, 64 * PAGE)
        .expect_err("a reservation in a backed register must be refused");
    assert_eq!(err, (Status::Unsupported, WINDOW_IN_A_BACKED_BAR));
}

#[test]
fn a_reservation_in_an_unbacked_register_is_installed() {
    // ★ Non-vacuity for the refusal above: the same call, against a register the hypervisor
    // does not back, goes in.
    let (shim, _host) = realize_with(MockPolicy::default()).expect("realize");
    shim.install_window(BAR1_BASE, 64 * PAGE)
        .expect("a reservation in an unbacked register must install");
    let a = shim.audit();
    assert_eq!(a.live_windows, 1);
    assert_eq!(a.memslot_installs, 1);
}

#[test]
fn a_register_move_is_refused_once_a_reservation_lives_in_it() {
    let (shim, _host) = realize_with(MockPolicy::default()).expect("realize");
    // Before any reservation, a move is nobody's business.
    shim.bar_move_requested(1)
        .expect("a register with nothing in it may move freely");
    shim.install_window(BAR1_BASE, 64 * PAGE).expect("install");
    let err = shim
        .bar_move_requested(1)
        .expect_err("a register a reservation lives in may not move");
    assert_eq!(err, (Status::Unsupported, BAR_MOVED_UNDER_US));
    // A register with nothing in it is still free, so the refusal is about the reservation
    // and not about registers in general.
    shim.bar_move_requested(2)
        .expect("an untouched register is still free to move");
}

#[test]
fn the_detector_counts_a_move_the_preventer_did_not_stop() {
    let (shim, _host) = realize_with(MockPolicy::default()).expect("realize");
    shim.install_window(BAR1_BASE, 64 * PAGE).expect("install");
    assert_eq!(shim.audit().bar_moves_detected, 0);
    shim.note_bar_mapping(1, Some(BAR1_BASE + 0x1000_0000))
        .expect("a known register index");
    assert_eq!(
        shim.audit().bar_moves_detected,
        1,
        "the detector exists precisely for the move the preventer did not see"
    );
    // A register nothing was latched in reports no move.
    shim.note_bar_mapping(2, Some(BAR2_BASE)).expect("known");
    assert_eq!(shim.audit().bar_moves_detected, 1);
}

#[test]
fn a_register_index_this_port_does_not_name_is_refused_on_both_halves() {
    let (shim, _host) = realize_with(MockPolicy::default()).expect("realize");
    assert_eq!(
        shim.bar_move_requested(9).map_err(|e| e.0),
        Err(Status::Malformed)
    );
    assert_eq!(
        shim.note_bar_mapping(9, Some(0)).map_err(|e| e.0),
        Err(Status::Malformed)
    );
}

// =====================================================================================
// The topology listener
// =====================================================================================

/// The wire form of a section the mock host minted.
///
/// ★ Deliberately field-by-field rather than a helper on `SectionWire`: this direction is
/// what the C shim performs by hand, and writing it out here is what would catch an inverted
/// or dropped fact.
fn wire_of(d: SectionDesc) -> SectionWire {
    SectionWire {
        mr: d.mr.0,
        gpa: d.gpa,
        len: d.len,
        offset_within_region: d.offset_within_region,
        is_ram: d.facts.is_ram,
        is_ram_device: d.facts.is_ram_device,
        is_rom_device: d.facts.is_rom_device,
        readonly: d.facts.readonly,
        nonvolatile: d.facts.nonvolatile,
    }
}

#[test]
fn a_reported_section_is_declared_and_the_counter_says_so() {
    let (shim, host) = realize_with(MockPolicy::default()).expect("realize");
    let d = host.mint_foreign(0x1_0000_0000, 0x1_0000, SectionFacts::plain_ram());
    shim.region_add(wire_of(d))
        .expect("a plain memory section must be taken");
    assert_eq!(shim.audit().topology_adds, 1);
    shim.region_del(0x1_0000_0000, 0x1_0000);
    assert_eq!(shim.audit().topology_dels, 1);
}

#[test]
fn a_section_that_overlaps_one_of_our_own_registers_is_refused() {
    let (shim, host) = realize_with(MockPolicy::default()).expect("realize");
    let d = host.mint_foreign(BAR1_BASE, 0x1_0000, SectionFacts::plain_ram());
    let err = shim
        .region_add(wire_of(d))
        .expect_err("a section over one of our own registers must be refused");
    assert_eq!(err.0, Status::Unsupported);
    assert!(
        err.1.contains("overlaps a range this device owns"),
        "got: {}",
        err.1
    );
}

#[test]
fn the_five_facts_are_carried_across_unclassified() {
    // ★ The rule that turns five facts into a verdict lives in one module, one crate over.
    // What THIS seam owes is that it does not lose or invert a fact on the way. A
    // device-memory section is the sharpest case: it reports memory and is not.
    let (shim, host) = realize_with(MockPolicy::default()).expect("realize");
    let facts = SectionFacts {
        is_ram_device: true,
        ..SectionFacts::plain_ram()
    };
    let d = host.mint_foreign(0x2_0000_0000, 0x1_0000, facts);
    shim.region_add(wire_of(d)).expect("declared, not dropped");
    assert_eq!(shim.audit().topology_adds, 1);
    // ★ SUSPECT THE INSTRUMENT. `live_regions()` reports every minted region with its
    // reference count, so its LENGTH is 1 whatever the classifier decided — an assertion on
    // the length would have been green for both answers. The count is the instrument.
    assert_eq!(
        host.live_regions(),
        vec![(d.mr, 0)],
        "a device section must be declared but must NOT have a reference taken on it"
    );
}

#[test]
fn a_plain_memory_section_does_take_a_counted_reference() {
    // ★ Non-vacuity for the assertion above: the count is not simply always zero.
    let (shim, host) = realize_with(MockPolicy::default()).expect("realize");
    let d = host.mint_foreign(0x3_0000_0000, 0x1_0000, SectionFacts::plain_ram());
    shim.region_add(wire_of(d)).expect("taken");
    assert_eq!(host.live_regions(), vec![(d.mr, 1)]);
}

// =====================================================================================
// Teardown, and the number that is the whole memory-plane decision
// =====================================================================================

#[test]
fn no_region_is_ever_handed_to_the_hypervisor_to_back() {
    let (shim, _host) = realize_with(MockPolicy::default()).expect("realize");
    shim.install_window(BAR1_BASE, 64 * PAGE).expect("install");
    assert_eq!(
        shim.audit().regions_published,
        0,
        "the hypervisor reserves the range and backs nothing; a non-zero here is the whole \
         memory-plane decision being undone"
    );
}

#[test]
fn unrealize_withdraws_the_blocker_and_re_enables_discard() {
    let (shim, host) = realize_with(MockPolicy::default()).expect("realize");
    assert!(host.discard_disabled());
    assert_eq!(host.blockers().len(), 1);
    shim.unrealize();
    assert!(
        !host.discard_disabled(),
        "a device that leaves discard disabled after it is gone has taken a machine-wide \
         facility away permanently"
    );
    assert_eq!(
        host.blockers().len(),
        0,
        "an unwithdrawn blocker leaves the machine permanently unmigratable"
    );
}

#[test]
fn an_operation_after_unrealize_is_refused_and_counted() {
    let (shim, _host) = realize_with(MockPolicy::default()).expect("realize");
    shim.unrealize();
    let err = shim
        .install_window(BAR1_BASE, 64 * PAGE)
        .expect_err("the memory plane is gone");
    assert_eq!(err, (Status::Unsupported, MEMORY_PLANE_AFTER_UNREALIZE));
    assert_eq!(shim.audit().ops_refused_after_unrealize, 1);
}

// =====================================================================================
// The wire contract itself — the hand-mirroring hazard, made a test
// =====================================================================================

#[test]
fn the_status_codes_are_the_numbers_the_header_names() {
    // ★ `qemu/hw/misc/nvkvm/kayfabe_shim.h` spells these five numbers by hand. Nothing in the
    // build makes the two agree, so this test IS the agreement: change one side and this is
    // what fails, rather than a device that reports "busy" as "malformed" in production.
    assert_eq!(Status::Ok.code(), 0);
    assert_eq!(Status::Refused.code(), 1);
    assert_eq!(Status::Busy.code(), 2);
    assert_eq!(Status::Unsupported.code(), 3);
    assert_eq!(Status::Malformed.code(), 4);
}

#[test]
fn the_audit_structure_is_the_size_the_header_declares() {
    // Nine 64-bit counters, in the order `KayfabeAudit` lists them. A field added on one side
    // only would be a silent misalignment of every field after it.
    assert_eq!(
        size_of::<kayfabe_qemu_raw::shim::KayfabeAudit>(),
        9 * size_of::<u64>()
    );
}

#[test]
fn the_register_index_mapping_is_a_bijection_over_the_registers_this_port_names() {
    use kayfabe_qemu_raw::shim::{bar_from_index, bar_index};

    for i in 0..3u32 {
        let bar = bar_from_index(i).expect("this port names registers 0, 1 and 2");
        assert_eq!(bar_index(bar), i);
    }
    assert_eq!(bar_from_index(3), None);
    assert_eq!(bar_from_index(u32::MAX), None);
}

// =====================================================================================
// Stage Q4 — the register plane's half of the seam
// =====================================================================================

#[test]
fn the_register_plane_wire_structures_are_the_sizes_the_header_declares() {
    use kayfabe_qemu_raw::shim::{KayfabeChipIdentity, KayfabeRegAudit};
    use kayfabe_qemu_raw::shim_unsafe::KayfabeRegWrite;

    // Hand-mirrored structures: a field added on one side only misaligns every field after
    // it, and the runtime `struct_size` handshake only covers the one structure carrying
    // that field. This is the compile-time half.
    // ★ 32 -> 48 at task #127: the two window lengths, so the chip row's BAR table and
    // the apertures the hypervisor actually registers cannot disagree — `nvkvm_realize`
    // refuses a property that differs from what the emulated GSP tells the guest's RM.
    assert_eq!(size_of::<KayfabeChipIdentity>(), 48);
    assert_eq!(align_of::<KayfabeChipIdentity>(), 8);
    // ★ 32 -> 64 at stage Q5: two more (pointer, length) pairs and two more u64s, so the
    // emulated GSP's guest-RAM refusals carry their address and their reason across the
    // seam instead of only their tag. Exactly the change the ABI version exists for — the
    // `struct_size` handshake does not cover this structure, so nothing but the version and
    // this line stands between an ABI-3 shim and 32 bytes written past its allocation.
    // ★ 64 -> 112 at `#146`: the framebuffer refusal's own (pointer, length) pair plus its
    // address and length, and the landed address plus its validity flag — the same shape,
    // one aperture over. `fb_landed` needs the flag because framebuffer address ZERO is
    // where an unprogrammed window points, so a single field could not tell "landed at 0"
    // from "did not land", which are the two answers this rung is about.
    // ★ 112 -> 144 at `execution_plane_increments.md` **E2**: `doorbell` (which of the
    // three answers this write was), the token, and the refusal KIND's own (pointer,
    // length) pair.
    // ⊘ This is the FIRST time a per-write field has been added since E1 declined to add
    // one, and the difference is the increment's argument: an isolate refusal is a property
    // of a whole boot, a doorbell is a property of ONE WRITE, and E2's acceptance is that
    // *this* store at *this* instant reached the core. A per-boot counter cannot be stamped
    // against a timeline the device does not write.
    // ⚠ The KIND crosses as a pointer and the SENTENCE does not: the kind is a `FaultTag`,
    // i.e. a `&'static str` in this archive's read-only data, while the sentence is
    // `format!`ed and owned by a temporary. The sentence reaches the shell through
    // `KayfabeRegAudit::doorbell_refusal` instead.
    assert_eq!(size_of::<KayfabeRegWrite>(), 144);
    // ★ 12 -> 15 counters plus a 32-slot list at task #127: the emulated GSP's default
    // became a NAMED REFUSAL, and the guest logs `NV_ERR_NOT_SUPPORTED` quietly, so the
    // list of what nobody answered has to cross the seam or it costs a boot per entry.
    // ★ 15 -> 17 at `#102` stage C: a framebuffer-window access is device memory and not
    // an unclaimed register, and the difference has to be readable from the C side.
    // ★ 17 -> 19 counters plus a second, WIDER list at the event-notification rung: the
    // object bridge's refusals ANSWER the guest's command, so they reach no ledger, and
    // boot `alloc1` had to be diagnosed by `fn 103` being ABSENT from six lines. Each row
    // carries the FaultTag's name by value — see `KayfabeBridgeRefusal` for why a pointer
    // was not available.
    assert_eq!(
        size_of::<kayfabe_qemu_raw::shim::KayfabeBridgeRefusal>(),
        kayfabe_qemu_raw::shim::BRIDGE_REFUSAL_TAG_LEN + 2 * size_of::<u64>()
    );
    // ★ 19 -> 25 at `#146`: five framebuffer counters and the residency level, because a
    // window that serves bytes and one that drops them must not report the same numbers.
    // ★ 25 -> 30 at `#149`: the translated BAR2 window's three counters, the count of
    // published page-table roots and the root entry itself. A boot that never received a
    // root and a boot whose walk landed on the wrong byte both end in the guest's
    // NV_ERR_MEMORY_ERROR, and only these numbers tell them apart from outside the process.
    // ★ 30 -> 33 at `#151`: the CPU interrupt tree's three. A boot that stopped at
    // `NV_ERR_IRQ_NOT_FIRING` and a boot that never reached the driver's loopback self-test
    // are the same silence from outside the process without `cpu_intr_raises`.
    // ★ E1: the isolate refusal's own row, same shape and same reason as
    // `KayfabeBridgeRefusal`'s — text by value, an explicit length, and a KIND, because a
    // check keyed on a word is satisfied by writing the word.
    assert_eq!(
        size_of::<kayfabe_qemu_raw::shim::KayfabeIsolateRefusal>(),
        kayfabe_qemu_raw::shim::ISOLATE_REFUSAL_LEN + 2 * size_of::<u64>()
    );
    // ★ 33 -> 37 at `execution_plane_increments.md` E1: the isolate plane's four counters,
    // plus the refusal row. Two of the four are the increments' own headline numbers —
    // `isolates_materialized` (E0b: an isolate exists because the GUEST acted, so zero is a
    // finding) and `isolates_spawn_failed` (E1: a plane that was asked for and broke, which
    // used to be the same silence as a build that never had one).
    // ★ E2: the doorbell refusal's own row — a KIND and a SENTENCE in two arrays rather
    // than one blob, because the kind is a stable name a check may branch on
    // (`FwdFault::MalformedToken` and `FwdFault::UnknownVchid` are two diagnoses with two
    // fixes) and the sentence is prose. A single blob would make the only machine-readable
    // half a substring search. `present` is the validity flag, and it is needed because a
    // `kind_len` of zero is also what an audit nobody wrote looks like.
    assert_eq!(
        size_of::<kayfabe_qemu_raw::shim::KayfabeDoorbellRefusal>(),
        kayfabe_qemu_raw::shim::DOORBELL_KIND_LEN
            + kayfabe_qemu_raw::shim::DOORBELL_REFUSAL_LEN
            + 3 * size_of::<u64>()
    );
    // ★ 37 -> 42 at `execution_plane_increments.md` E2: the doorbell aperture's three
    // counters, the last token and its validity flag, plus the refusal row. The flag is not
    // redundant — token ZERO is a legal work-submit token (runlist 0, channel 0), so one
    // field could not tell "rang channel 0" from "never rang", which is the same
    // two-fields-for-one-fact argument `fb_landed_valid` already carries.
    assert_eq!(
        size_of::<KayfabeRegAudit>(),
        (42 + kayfabe_qemu_raw::shim::UNSERVICED_SLOTS) * size_of::<u64>()
            + kayfabe_qemu_raw::shim::BRIDGE_REFUSAL_SLOTS
                * size_of::<kayfabe_qemu_raw::shim::KayfabeBridgeRefusal>()
            + size_of::<kayfabe_qemu_raw::shim::KayfabeIsolateRefusal>()
            + size_of::<kayfabe_qemu_raw::shim::KayfabeDoorbellRefusal>()
    );
    // ⊘ And the three `kind` values are DISTINCT and NONE is zero — the property the C
    // shell's branch and the "an unwritten struct is not a diagnosis" argument both rest
    // on. Asserted rather than assumed: they are three `pub const`s that a careless edit
    // could collapse without any other test noticing.
    use kayfabe_qemu_raw::shim::{
        ISOLATE_REFUSAL_NO_PLANE, ISOLATE_REFUSAL_NONE, ISOLATE_REFUSAL_SPAWN_FAILED,
    };
    assert_eq!(ISOLATE_REFUSAL_NONE, 0);
    assert_ne!(ISOLATE_REFUSAL_NO_PLANE, ISOLATE_REFUSAL_NONE);
    assert_ne!(ISOLATE_REFUSAL_SPAWN_FAILED, ISOLATE_REFUSAL_NONE);
    assert_ne!(ISOLATE_REFUSAL_SPAWN_FAILED, ISOLATE_REFUSAL_NO_PLANE);
}

#[test]
fn the_default_chip_identity_is_what_a_stock_drivers_own_table_matches() {
    let id = kayfabe_qemu_raw::shim::chip_identity(0).expect("the table has a default row");
    assert_eq!(id.abi_version, kayfabe_qemu_raw::shim::ABI_VERSION);
    assert_eq!(id.struct_size as usize, size_of_val(&id));
    // ★ `nv_pci_table` (`ogkm-580: kernel-open/nvidia/nv-pci-table.c:39`) matches vendor
    // 0x10DE with class 0300xx / 0302xx, and the module unloads itself when nothing
    // matches. These two numbers are the whole reason the identity is not neutral.
    assert_eq!(id.vendor_id, 0x10DE);
    assert_eq!(id.class_code >> 16, 0x03);
    assert!(
        id.msix_vectors > 0,
        "the interrupt capability must be askable-for"
    );
    assert_eq!(id.regs_aperture_len, 16 << 20);
}

#[test]
fn a_device_id_the_chip_table_does_not_carry_is_unsupported_and_says_why() {
    let (status, msg) =
        kayfabe_qemu_raw::shim::chip_identity(0x1234).expect_err("no such chip row");
    // ★ `Unsupported`, not `Refused`: retrying cannot help, it is a property of this build.
    assert_eq!(status, Status::Unsupported);
    assert!(
        msg.contains("nearest-neighbour"),
        "the sentence must survive the seam: {msg}"
    );
}

#[test]
fn the_register_plane_answers_through_the_seam_the_c_shim_calls() {
    use kayfabe_qemu_raw::shim::Regs;

    let regs = Regs::create(0).expect("the default chip is servable");
    // ★★★ THE ACCEPTANCE REGISTER: `NV_PGSP` + `NV_PFALCON_FALCON_CPUCTL`, which
    // `kflcnWaitForHalt_TU102` polls. `HALTED_TRUE` is bit 4.
    assert_eq!(regs.read(0, 0x0011_0100, 4), 0x10);
    assert_eq!(regs.read(0, 0x0011_8234, 4), 0xFF);
    assert_eq!(regs.read(0, 0x0000_0000, 4), 0x1760_00A1);
    assert_eq!(regs.read(0, 0x0030_0000, 2), 0xAA55);
    // An offset nobody owns, and it must be zero rather than another register's value.
    //
    // ★★ It was `0x0077_7777` until 2026-07-31, which is inside `PRAMIN` — so this probe
    // was measuring a **framebuffer** access and calling it an unclaimed register. The two
    // are now separate answers and this line names the register one.
    assert_eq!(regs.read(0, 0x0055_5555, 4), 0);
    // ★★★ …and the framebuffer one, through the same seam. Since `#146` this reads zero
    // because the store is FRESH — memory this device advertises that nobody has written —
    // and not because the access was dropped. `fb_reads` moves and `unclaimed_reads` does
    // not.
    assert_eq!(regs.read(0, 0x0077_7777, 4), 0);

    let a = regs.audit();
    assert_eq!(a.reads, 6);
    assert_eq!(a.gsp_reads, 2);
    assert_eq!(a.boot_reg_reads, 1);
    assert_eq!(a.rom_reads, 1);
    assert_eq!(
        a.unclaimed_reads, 1,
        "the PRAMIN read must NOT be counted as an unclaimed register"
    );
    assert_eq!(
        a.fb_reads, 1,
        "…it must be counted as a SERVED framebuffer read, across the C seam"
    );
    assert_eq!(
        a.fb_refusals, 0,
        "★★★ and NOTHING was refused: the composition root installed a store sized from \
         the chip's own fb_length, so an address the guest was promised always resolves"
    );

    // ★★★ THE PROPERTY THE RUNG IS FOR, through the C seam: write a dword through the
    // moving window and read it back. This is `kbusVerifyBar2_GM107:4084-4090` in
    // miniature, and if it fails the guest reports NV_ERR_MEMORY_ERROR hundreds of
    // operations later with no clue why.
    const WINDOW_REG: u64 = 0x0000_1700;
    const PRAMIN: u64 = 0x0070_0000;
    let w = regs.write(0, WINDOW_REG, 4, 0x0002_EFBA);
    assert!(w.claimed, "the window register is a LATCH this plane owns");
    assert_eq!(
        regs.read(0, WINDOW_REG, 4),
        0x0002_EFBA,
        "★★★ it must read back: RM's own field update is a READ-MODIFY-WRITE, and RM \
         refreshes cachedBar0WindowVidOffset from this very read"
    );
    let w = regs.write(0, PRAMIN + 0xE000, 4, 0xABCD_ABCD);
    assert_eq!(
        w.fb_landed,
        Some(0x0002_EFBA_E000),
        "the write must LAND, at the address the window resolves — and say where"
    );
    assert!(w.fb_refusal.is_none() && w.fb_window.is_none());
    assert_eq!(
        regs.read(0, PRAMIN + 0xE000, 4),
        0xABCD_ABCD,
        "★★★ and read back through the same window"
    );
    // ⊘ …and NOT through a different one. The window really moves.
    let _ = regs.write(0, WINDOW_REG, 4, 0x0002_EFBB);
    assert_eq!(regs.read(0, PRAMIN + 0xE000, 4), 0);
    // A write into the framebuffer aperture is DROPPED and said so — the case that costs a
    // page-table entry rather than a register value.
    let w = regs.write(1, 0x0009_008C, 8, 0xDEAD_BEEF);
    assert!(!w.claimed);
    assert_eq!(regs.audit().fb_window_writes, 1);
    assert_eq!(
        regs.audit().unclaimed_writes,
        0,
        "a dropped framebuffer write is not a dropped register write"
    );

    // ★★★ `#149`, THROUGH THE SAME SEAM: the composition root installed a page-table
    // FORMAT, so a write into the translated instance/BAR2 window is refused because the
    // aperture is **unrooted** — the guest has not published a root page-directory entry
    // yet — and NOT because this device was built without a format. Those are a guest fact
    // and a wiring fact, and the whole reason `set_mmu` has a refusing default is that they
    // must not read the same.
    //
    // ⊘ It is the strongest assertion available here without a page-table tree: it fails if
    // the root ever stops calling `set_mmu`, and it fails LOUDLY rather than by a boot
    // ending in NV_ERR_MEMORY_ERROR with the format silently absent.
    let before = regs.audit().bar2_faults;
    let w = regs.write(2, 0x0031_2000, 4, 0xABCD_ABCD);
    assert!(!w.claimed);
    assert!(
        w.fb_landed.is_none(),
        "nothing may land through an unrooted aperture"
    );
    let why = w
        .bar2_refusal
        .expect("a refused translated write says so, whole")
        .why;
    assert_eq!(
        why,
        kayfabe_device::plane::BAR2_UNROOTED,
        "★★★ the composition root must have installed a format; without one this reads \
         `the register plane has no page-table format installed`"
    );
    assert_eq!(regs.audit().bar2_faults, before + 1);
    assert_eq!(
        regs.audit().bar_pde_updates,
        0,
        "and no root has been published, which is the other half of the same statement"
    );

    // A write that reaches guest RAM the plane does not have refuses BY NAME through the
    // seam; the pointer is into the archive's read-only data, so the C may hold it.
    let _ = regs.write(0, 0x0011_0100, 4, 0x2);
    let _ = regs.write(0, 0x0011_0040, 4, 0x1000);
    let w = regs.write(0, 0x0011_0044, 4, 0);
    assert!(w.claimed);
    assert_eq!(
        w.fault,
        Some("GspFault::GuestRam"),
        "the refusal must name itself across the seam"
    );
}

#[test]
fn a_base_address_register_the_plane_does_not_own_reads_zero() {
    let regs = kayfabe_qemu_raw::shim::Regs::create(0).expect("servable");
    assert_eq!(regs.read(1, 0x0011_0100, 4), 0);
    assert_eq!(regs.read(255, 0x0011_0100, 4), 0);
}

#[test]
fn the_counter_the_c_shim_serves_runs_at_wall_clock_rate() {
    // ★★★ THE BITE for the adapter's half of the free-running counter, and the reason it is
    // here rather than only in `kayfabe-device`: the device crate can prove the *plumbing*
    // with any clock at all, including one that advances a nanosecond per reading. Only this
    // side can prove the plumbing was given a clock that tells the time.
    //
    // Why that distinction is worth a test. Every bounded wait in the guest driver's GSP
    // bring-up exits either on success or on `gpuCheckTimeout`, which reads this counter
    // (`ogkm-580: src/nvidia/src/kernel/gpu/timer/arch/turing/timer_tu102.c:130-155`). Its
    // timeouts are wall-clock microseconds. A counter that advanced per *reading* would
    // satisfy every structural check this repository has and still turn a 4-second timeout
    // into an unpredictable number of iterations.
    use kayfabe_qemu_raw::shim::Regs;
    use std::time::{Duration, Instant};

    let regs = Regs::create(0).expect("the default chip is servable");
    let compose = || {
        let hi = regs.read(0, 0x00BB_0084, 4);
        let lo = regs.read(0, 0x00BB_0080, 4);
        (hi << 32) | lo
    };

    let host_before = Instant::now();
    let a = compose();
    std::thread::sleep(Duration::from_millis(20));
    let b = compose();
    let host_elapsed = host_before.elapsed();

    assert!(b > a, "the counter did not advance across a 20 ms sleep");
    let device_elapsed = Duration::from_nanos(b - a);
    // ★ A wide band on purpose. This asserts the counter is a CLOCK — same order of
    // magnitude as real time — not that a shared machine scheduled us promptly. A
    // per-reading counter would land four readings' worth away from this, and a stopped one
    // would have failed the line above.
    assert!(
        device_elapsed >= Duration::from_millis(10) && device_elapsed <= host_elapsed * 4,
        "the device counter moved {device_elapsed:?} while the host moved {host_elapsed:?}"
    );

    assert!(
        regs.audit().ptimer_reads >= 4,
        "the counter's readings must be separately countable across the seam"
    );
}

// =====================================================================================
// Stage Q5 — the register plane's guest-RAM port, and the three answers it can give
// =====================================================================================

/// The offsets `kgspProgramLibosBootArgsAddr_TU102` writes, in its order, plus the
/// STARTCPU that puts the boot state machine in the phase where the pair means anything.
///
/// ★ Written out rather than derived from the register model: this file's whole job is to
/// be the *second* description that would disagree if the first one moved.
const GSP_CPUCTL: u64 = 0x0011_0100;
const GSP_MAILBOX0: u64 = 0x0011_0040;
const GSP_MAILBOX1: u64 = 0x0011_0044;
const STARTCPU: u64 = 0x2;

/// Where a test guest keeps its LibOS boot-args array. Any RAM address will do; what
/// matters is that the region map either does or does not cover it.
const BOOT_ARGS_GPA: u64 = 0x1_0000_0000;

/// How much guest RAM the bind actually walks: the LibOS init-args array's own declared
/// maximum of 4096 entries at 32 bytes each
/// (`ogkm-580: src/common/uproc/os/common/include/libos_init_args.h:31, 49-56`).
///
/// ★ Not a round number picked to make a test pass. A first draft declared 64 KiB and the
/// scan ran off the end of it — reported as a refusal naming the exact address it reached,
/// which is the port doing its job on the test's own bad input. Sizing the region to the
/// scan is what makes "the array carries no RMARGS entry" the reason the bind stops.
const LIBOS_ARRAY_LEN: u64 = 4096 * 32;

/// Drive the guest's own boot-args write sequence and return the write that triggers the
/// bind — the one the emulated GSP answers by following the guest's pointer.
fn drive_the_boot_args_pair(regs: &kayfabe_qemu_raw::shim::Regs) -> kayfabe_device::WriteOutcome {
    let _ = regs.write(0, GSP_CPUCTL, 4, STARTCPU);
    let _ = regs.write(0, GSP_MAILBOX0, 4, BOOT_ARGS_GPA & 0xFFFF_FFFF);
    regs.write(0, GSP_MAILBOX1, 4, BOOT_ARGS_GPA >> 32)
}

#[test]
fn without_the_port_the_refusal_names_the_missing_wiring_not_the_guests_address() {
    // ★★ THE BITE for `attach_ram`. This is the state stage Q4 shipped in and the state a
    // shell that forgot to call `attach_ram` is in, and the two must be the same sentence —
    // otherwise "the port is missing" and "the guest asked for something that is not there"
    // read alike, which is a whole debugging session. Delete the `attach_ram` call in
    // `nvkvm_shim_realize` and every Q5 test below produces THIS sentence.
    let regs = kayfabe_qemu_raw::shim::Regs::create(0).expect("servable");
    let w = drive_the_boot_args_pair(&regs);

    assert_eq!(w.fault, Some("GspFault::GuestRam"));
    let r = w.ram_refusal.expect("a RAM refusal carries its address");
    assert_eq!(r.gpa, BOOT_ARGS_GPA);
    assert_eq!(r.why, kayfabe_device::plane::NO_RAM_PORT);
    assert_eq!(regs.audit().ram_refusals, 1);
}

#[test]
fn with_the_port_an_address_no_region_covers_is_refused_in_the_memory_planes_own_words() {
    // The port is installed and the machine is real; nothing has declared guest RAM. The
    // refusal must move from "there is no port" to "nothing is there" — a different
    // sentence, because they send a reader to different places.
    let (shim, _host) = realize_with(MockPolicy::default()).expect("realize");
    let regs = kayfabe_qemu_raw::shim::Regs::create(0).expect("servable");
    regs.attach_ram(&shim);

    let w = drive_the_boot_args_pair(&regs);
    assert_eq!(w.fault, Some("GspFault::GuestRam"));
    let r = w.ram_refusal.expect("a RAM refusal carries its address");
    assert_eq!(r.gpa, BOOT_ARGS_GPA);
    assert_ne!(
        r.why,
        kayfabe_device::plane::NO_RAM_PORT,
        "the port IS installed; saying otherwise would be the Q4 sentence surviving Q5"
    );
    assert!(
        r.why.contains("no guest-physical region covers"),
        "got: {}",
        r.why
    );
}

#[test]
fn with_the_port_a_declared_ram_region_is_actually_read_and_the_boot_moves_on() {
    // ★★★ THE POINT OF STAGE Q5. The emulated GSP follows the guest's own pointer into
    // guest memory and gets bytes back — so the refusal is no longer about memory at all.
    //
    // The bytes it finds are zeros, so the LibOS region array carries no `RMARGS` entry and
    // the bind refuses for a *protocol* reason. That is the observable difference between
    // "we could not read the guest's memory" and "we read it and it did not say what the
    // protocol requires", and it is the whole rung.
    let (shim, host) = realize_with(MockPolicy::default()).expect("realize");
    let d = host.mint_foreign(BOOT_ARGS_GPA, LIBOS_ARRAY_LEN, SectionFacts::plain_ram());
    shim.region_add(wire_of(d)).expect("plain memory is taken");

    let regs = kayfabe_qemu_raw::shim::Regs::create(0).expect("servable");
    regs.attach_ram(&shim);

    let w = drive_the_boot_args_pair(&regs);
    assert!(w.claimed);
    assert_eq!(
        w.ram_refusal, None,
        "guest memory was readable, so nothing about memory was refused"
    );
    assert_eq!(
        w.fault,
        Some("GspFault::RmargsRegionAbsent"),
        "the array was read and scanned; it simply carries no RMARGS region"
    );
    assert_eq!(
        regs.audit().ram_refusals,
        0,
        "not one guest-memory access was refused"
    );
}

#[test]
fn a_region_that_is_a_device_is_refused_apart_from_a_region_that_is_absent() {
    // ★★ `testing_doctrine.md` §2 rule 3, at the sharpest pair this port has. Both of these
    // are "we would not read that address"; one means nothing is there and the other means
    // something is there and it is REGISTERS — and serving the second would mean the
    // emulated GSP reached back into a register plane through the memory plane. A port
    // that reported them identically would make that indistinguishable from a typo.
    let (shim, host) = realize_with(MockPolicy::default()).expect("realize");
    let facts = SectionFacts {
        is_ram_device: true,
        ..SectionFacts::plain_ram()
    };
    let d = host.mint_foreign(BOOT_ARGS_GPA, LIBOS_ARRAY_LEN, facts);
    shim.region_add(wire_of(d)).expect("declared, unclassified");

    let regs = kayfabe_qemu_raw::shim::Regs::create(0).expect("servable");
    regs.attach_ram(&shim);

    let w = drive_the_boot_args_pair(&regs);
    let r = w.ram_refusal.expect("a RAM refusal carries its address");
    assert!(
        r.why.contains("device register window"),
        "the device arm must not report as the absent arm; got: {}",
        r.why
    );
}

#[test]
fn detaching_the_port_puts_the_plane_back_to_refusing_by_name() {
    // The teardown half. The register surface keeps answering across a memory-plane
    // teardown by design, so the port has to be withdrawn explicitly — and afterwards the
    // sentence must be the *wiring* one again, not a stale machine's.
    let (shim, host) = realize_with(MockPolicy::default()).expect("realize");
    let d = host.mint_foreign(BOOT_ARGS_GPA, LIBOS_ARRAY_LEN, SectionFacts::plain_ram());
    shim.region_add(wire_of(d)).expect("plain memory is taken");

    let regs = kayfabe_qemu_raw::shim::Regs::create(0).expect("servable");
    regs.attach_ram(&shim);
    assert_eq!(drive_the_boot_args_pair(&regs).ram_refusal, None);

    regs.detach_ram();
    let r = drive_the_boot_args_pair(&regs)
        .ram_refusal
        .expect("after detach every guest-memory access is refused again");
    assert_eq!(r.why, kayfabe_device::plane::NO_RAM_PORT);
}

#[test]
fn the_port_writes_guest_memory_where_the_guest_can_see_it() {
    // ★ The write direction, which the read tests above cannot cover: the status-queue tx
    // header the guest's `msgqRxLink` spins on is a WRITE into the guest's own region, and
    // a port that could only read would pass every test above and hang the guest forever.
    // Asserted against the host's own bytes, not against our return value.
    use kayfabe_device::GuestRam;

    let (shim, host) = realize_with(MockPolicy::default()).expect("realize");
    let d = host.mint_foreign(BOOT_ARGS_GPA, 0x1000, SectionFacts::plain_ram());
    shim.region_add(wire_of(d)).expect("plain memory is taken");

    let mut ram = kayfabe_qemu_raw::shim::MachineRam::new(shim.machine().vmm());
    ram.write(BOOT_ARGS_GPA + 8, &[0xDE, 0xAD, 0xBE, 0xEF])
        .expect("declared memory is writable");

    let bytes = host.region_bytes(d.mr).expect("the region exists");
    assert_eq!(&bytes[8..12], &[0xDE, 0xAD, 0xBE, 0xEF]);

    let mut back = [0u8; 4];
    ram.read(BOOT_ARGS_GPA + 8, &mut back)
        .expect("what was written reads back");
    assert_eq!(back, [0xDE, 0xAD, 0xBE, 0xEF]);
}

/// ★★★ **The header and this archive must agree on the wire ABI, and until `#151` NOTHING
/// IN THIS TREE CHECKED THAT.**
///
/// # `[measured]` run `irq1` at `bb4f48d` — the boot this test would have saved
///
/// `#151` bumped `KAYFABE_SHIM_ABI` in `kayfabe_shim.h` to 10 and left
/// `shim::ABI_VERSION` at 9. Every gate passed, the whole suite passed, the archive built,
/// the hypervisor linked, and the failure arrived as a **refused boot**:
///
/// ```text
/// qemu-system-x86_64: -device nvkvm-gpu,...: nvkvm: this shim speaks wire ABI 10 and
/// the archive it was linked against speaks 9; they are from different builds
/// ```
///
/// ⊘ The runtime check did its job — it is the reason this cost one boot rather than a
/// silent read of four bytes the archive never wrote. But it is the **last** place the
/// disagreement could have been caught, and a bench boot is the most expensive instrument
/// this project owns. The two numbers are one fact written twice; this is where they are
/// made to agree.
///
/// ★ It parses the header rather than mirroring it, because a mirrored copy would be a
/// *third* place to forget. `CARGO_MANIFEST_DIR` reaches the tree the archive is built
/// from, which is exactly the header `build_qom_shim.sh` copies into the hypervisor.
#[test]
fn the_header_and_the_archive_agree_on_the_wire_abi() {
    let header = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../qemu/hw/misc/nvkvm/kayfabe_shim.h");
    let text = std::fs::read_to_string(&header)
        .unwrap_or_else(|e| panic!("the overlay header must be readable at {header:?}: {e}"));

    const NEEDLE: &str = "#define KAYFABE_SHIM_ABI ";
    let line = text
        .lines()
        .find(|l| l.starts_with(NEEDLE))
        .expect("★ kayfabe_shim.h no longer declares KAYFABE_SHIM_ABI — this test has gone blind");
    let declared: u32 = line[NEEDLE.len()..]
        .trim()
        .trim_end_matches('u')
        .parse()
        .expect("KAYFABE_SHIM_ABI must be a plain number");

    assert_eq!(
        declared,
        kayfabe_qemu_raw::shim::ABI_VERSION,
        "★ kayfabe_shim.h declares wire ABI {declared} and this archive declares {}. They \
         are ONE fact written twice, and a build in which they disagree gets as far as the \
         hypervisor's device line before anything notices.",
        kayfabe_qemu_raw::shim::ABI_VERSION,
    );
}

// =====================================================================================
// ★★★ The isolate-plane selector — `docs/design/execution_plane_increments.md` E0
// =====================================================================================
//
// ⊘ **What these tests do NOT cover, stated first.** They drive the *pure* half of the
// decision, [`isolate_plane_from`]. They never read `KAYFABE_ISOLATES`, because a test
// that mutated a process-global would race every other test in this binary, and they
// never spawn a `real` isolate, because there is no GPU here. The env-read arm and the
// spawn are covered by the live-boot pair recorded in `execution_plane_increments.md`
// §E0-evidence — an evidence run and a negative control that differ in nothing but the
// variable. If that pair is not in the doc, this seam is untested where it matters.

use kayfabe_qemu_raw::shim::{IsolatePlane, isolate_factory, isolate_plane_from};

/// The default is the plane master shipped — **absent is not an error**.
#[test]
fn an_unset_selector_is_the_stillborn_plane_master_shipped() {
    assert_eq!(
        isolate_plane_from(None),
        Ok(IsolatePlane::Stillborn),
        "★ the default moved. Every build that does not opt in must get the refusing \
         plane; a default that spawned anything would put a host process behind every \
         guest in the tree without a single line of configuration."
    );
}

/// Every plane round-trips, quantified over [`IsolatePlane::ALL`] rather than over a list
/// spelled here — `gates_quantified_over_a_list`: a hand-written list shrinks in one place
/// with nothing going red.
#[test]
fn every_plane_round_trips_through_its_own_spelling() {
    for plane in IsolatePlane::ALL {
        assert_eq!(
            isolate_plane_from(Some(plane.as_str())),
            Ok(plane),
            "★ {plane:?} does not parse from the name it prints"
        );
    }
    // ★ Non-vacuity for the loop above: three DISTINCT spellings, so a `Display` that
    // collapsed two planes onto one string would not be able to pass by luck.
    let mut names: Vec<&str> = IsolatePlane::ALL.iter().map(|p| p.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(
        names.len(),
        IsolatePlane::ALL.len(),
        "two planes share a name"
    );
}

/// ⊘ A value that is not a plane name is a **refusal to realize**, never a quiet default.
/// This is the property the whole selector exists for: an evidence run whose variable was
/// misspelled must not be indistinguishable from its own negative control.
#[test]
fn a_value_that_is_not_a_plane_name_refuses_rather_than_defaulting() {
    for bad in [
        "",
        "Real",
        "REAL",
        "real ",
        " real",
        "stillbornn",
        "host",
        "1",
        "true",
        "\u{fffd}invalid",
    ] {
        let got = isolate_plane_from(Some(bad));
        let (status, why) = got.expect_err(&format!(
            "★ {bad:?} was ACCEPTED as a plane name. A selector that accepts a near-miss \
             is a selector that can be typo'd into the arm it was meant to leave."
        ));
        assert_eq!(
            status.code(),
            kayfabe_qemu_raw::shim::Status::Unsupported.code(),
            "the refusal for {bad:?} must be Unsupported, not a near-neighbour"
        );
        // ★ The message must name every plane, derived from `ALL` — so adding a fourth
        // plane and forgetting to mention it turns this red rather than leaving an
        // operator to guess.
        for plane in IsolatePlane::ALL {
            assert!(
                why.contains(plane.as_str()),
                "★ the refusal for {bad:?} does not name the `{}` plane; an operator \
                 reading it cannot recover. Message was: {why}",
                plane.as_str()
            );
        }
    }
}

/// The stillborn factory really is stillborn — **the seam, not the name.**
///
/// ★ This is the one assertion here that reaches past the decision into the object it
/// builds. `isolate_factory` returns a `Box<dyn IsolateFactory>` and nothing about the
/// type says what it does; a factory that spawned would be caught here and only here.
#[test]
fn the_stillborn_factory_retires_every_isolate_at_birth() {
    use kayfabe_arch::ids::GpuId;
    use kayfabe_isolate::IsolateId;

    let mut f = isolate_factory(IsolatePlane::Stillborn).expect("the default plane builds");
    let id = IsolateId::new(7, GpuId(0));
    let mut iso = f.spawn(id);
    assert_eq!(iso.id(), id);
    assert!(
        iso.is_retired(),
        "★ the default plane spawned a LIVE isolate"
    );
    assert_eq!(iso.pool_size(), 0);
    assert!(
        iso.checkout().is_none(),
        "★ a verb could be issued by default"
    );
    // ★★★ E1 — and it says WHY, by kind and by sentence, at the seam the core holds.
    // ⊘ `NoPlane` and never `SpawnFailed`: this archive is behaving exactly as
    // configured, and an operator who reads "spawn-failed" here goes and debugs a host
    // that is fine. `bench_rebuild_notes.md` §5 row 7 is the fact that these two used to
    // be the same silence.
    let r = iso
        .refusal()
        .expect("★ the shipped default plane must say why it refuses");
    assert_eq!(r.kind, kayfabe_isolate::RefusalKind::NoPlane);
    assert!(
        r.why.contains("no forwarding plane"),
        "★ the sentence is the composition root's own; it said: {}",
        r.why
    );
}

/// ★★ A build **without** `host-isolates` refuses the host planes; it does not degrade
/// them to the refusing one. Two archives, one variable, and the difference must be
/// legible from the outside.
#[cfg(not(feature = "host-isolates"))]
#[test]
fn without_the_feature_a_host_plane_is_a_named_refusal_not_a_silent_stillborn() {
    for plane in [IsolatePlane::Loopback, IsolatePlane::Real] {
        let (status, why) = isolate_factory(plane)
            .err()
            .unwrap_or_else(|| panic!("★ {plane:?} was BUILT in an archive that cannot link it"));
        assert_eq!(
            status.code(),
            kayfabe_qemu_raw::shim::Status::Unsupported.code()
        );
        assert!(
            why.contains("host-isolates"),
            "★ the refusal must name the feature to rebuild with; it said: {why}"
        );
    }
}

/// ★★ And with the feature on, both host planes **build a factory** — the linkage half of
/// E0, asserted where a `cfg` typo would otherwise be invisible.
///
/// ⊘ It does not `spawn`. Spawning `Real` needs `/dev/nvidiactl`; spawning either needs
/// `clone(CLONE_NEWUSER)`, which a container may refuse. Those belong to the live boot.
#[cfg(feature = "host-isolates")]
#[test]
fn with_the_feature_both_host_planes_build_a_factory() {
    for plane in [IsolatePlane::Loopback, IsolatePlane::Real] {
        assert!(
            isolate_factory(plane).is_ok(),
            "★ {plane:?} could not be built in an archive that links kayfabe-isolate-host"
        );
    }
}
