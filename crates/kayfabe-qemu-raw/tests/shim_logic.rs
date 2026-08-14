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
        // ★ The backing facts round-trip through the wire exactly as the other nine do.
        // `SectionDesc::backing` is `None` for every section the mock mints by default, so
        // these are zeros unless a test asked for a backed one.
        fd_backed: d.backing.is_some(),
        backing_dev: d.backing.map_or(0, |b| b.dev),
        backing_ino: d.backing.map_or(0, |b| b.ino),
        file_offset_of_region: d.backing.map_or(0, |b| b.file_offset_of_region),
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
    // ★★★★ §16.56 — plus `REFUSAL_IDS_PER_TAG` ids and their length. A `FaultTag` is a
    // `&'static str`, so a refusal ABOUT A VALUE (an `hClass`, a control `cmd`) lost that
    // value the instant it became a census key. `[measured 2026-08-10, over
    // traces/guest_boots/*_qemu.log]` `grep -c hClass` over every committed device log
    // returns ZERO — this port had never once named a class it refused.
    assert_eq!(
        size_of::<kayfabe_qemu_raw::shim::KayfabeBridgeRefusal>(),
        kayfabe_qemu_raw::shim::BRIDGE_REFUSAL_TAG_LEN
            + 3 * size_of::<u64>()
            + kayfabe_qemu_raw::shim::REFUSAL_IDS_PER_TAG * size_of::<u32>()
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
    // ★ 42 -> 43 at `#128`: `ptimer_writes_refused`. Its own field rather than a share of
    // `unclaimed_writes`, because the two mean opposite things — unclaimed is "this port
    // does not model that offset", this is "it models it and says no" — and a guest RM
    // issuing `tmrSetCurrentTime` should show up in exactly one of them. This assertion is
    // the reason the wire ABI had to move to 13: nothing else covers this structure's size.
    // ★ 43 -> 47 at `execution_plane_increments.md` §8.2.2: the GPFIFO-ring census —
    // `declarations`, `nonzero`, the first non-zero ring `va` and its `entries`. `nonzero`
    // doubles as the validity flag for the two below it, for the reason
    // `doorbell_last_token_valid` is a field of its own: `gpFifoOffset = 0` is a
    // declaration the driver makes ON PURPOSE for its golden-context channel
    // (`ogkm-580: kernel_graphics.c:2420-2424`), so one field could not tell "declared
    // address zero" from "declared nothing". This is the reason the wire ABI moved to 14.
    // ★ 47 -> 51 at the control census: `served_total`/`served_len` and
    // `arming_total`/`arming_len`, plus the two row arrays below. The census is the
    // report's third state — a refusal that ANSWERS (`rpc_result != 0`) reaches neither
    // the unserviced list nor the bridge census, and 0x20800301 was the control named in
    // the guest line that killed a boot while absent from every list the report printed.
    // This is the reason the wire ABI moved to 15.
    // ★ 51 -> 52 at the probe-arm report: `probe_arm_len` plus the 8-entry u32 array
    // (4 u64-equivalents) below. The set a boot ran with must appear in the boot's own
    // report — three boots ran probe-off while looking armed from the launching shell,
    // when the probe was a process env var. This is the reason the wire ABI moved to 16.
    // ★ 52 -> 55 at `execution_plane_increments.md` §14.10: `gvas_pub_total`,
    // `gvas_pub_len` and `gvas_pub_undecodable`, plus the row array below. The rows are
    // the ONLY boot-path statement of a page-directory root at all — `[measured
    // 2026-08-08]` over `traces/real_ga106/rpc_transcript_real_ga106.txt`,
    // `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` (the one control the port turns into a
    // page-directory base) occurs ZERO times in the whole boot while `0x90f10106` occurs
    // four times and `0x20800a9f` once. `gvas_pub_undecodable` is a separate counter for
    // `bar_pde_updates`' refusal half's reason: "published something unreadable" and
    // "published nothing" are different diagnoses and an absence cannot tell them apart.
    // This is the reason the wire ABI moved to 17.
    // ★ +`KayfabeDoorbellServing` at `execution_plane_increments.md` §14.15 / E10e item
    // (c): the LAST doorbell the shell's own CPU copy-engine executor served, and what it
    // did. ⊘ Its own structure and not a second `KayfabeDoorbellRefusal` — the two carry
    // the same bytes and mean opposite things, and a header in which a serving is declared
    // as a refusal reads as a bug. This is the reason the wire ABI moved to 19.
    // ★ 55 -> 57 for the channel-bind census: `bind_total`, `bind_len` and the row array
    // below. ★★★ The rows are the ONLY place the scrubber's chosen copy engine becomes
    // observable to this device — `ceutilsGetFirstAsyncCe` picks it inside the guest
    // (`ogkm-580: ce_utils.c:66-81`) and `kchannelBindToRunlist_IMPL` RPCs it to us as
    // `engineType` (`ogkm-580: kernel_channel.c:2762-2785`) — and on GA106 the GRCE test
    // that drives the pick walks the partner list over the device-info table THIS PORT
    // SERVES (`kceGetGrceMaskReg` is the `NV_ERR_NOT_SUPPORTED` stub below GB202,
    // `ogkm-580: g_kernel_ce_nvoc.c:847-858`), so inferring the answer from our own table
    // is circular. This is the reason the wire ABI moved to 20.
    // ★ 57 -> 60 at `execution_plane_increments.md` §14.18: `nonstall_raises`,
    // `nonstall_unvectored` and `nonstall_masked` — whether the completion this device
    // WITNESSED was actually announced to the guest. ⊘ `nonstall_unvectored` is the one
    // that must be zero: it counts copies this shell really performed and never notified,
    // which is the promise made by serving notifier index 35 being broken quietly. Three
    // numbers and not one, because "we announced it", "we could not" and "we did and the
    // guest's own LEAF_EN hides it" are three different next moves for an operator. This
    // is the reason the wire ABI moved to 21.
    // ★★★ 60 -> 63 at `execution_plane_increments.md` §14.23: `gvas_pub_seen`,
    // `gvas_pub_applied` and `gvas_pub_unexpected` — the page-directory publication counted
    // by the seat that CARRIES IT INTO THE OBJECT MODEL, beside the three already counted
    // by the recorder that only logs it. ⊘ Two counts of one event, deliberately: until
    // 2026-08-08 the port decoded this control, answered `NV_OK` and dropped the value, so
    // `gvas_pub_total` read 5 while `Vas::pdb` was empty and every promote-ctx refused. A
    // single number could not have said that, and `seen == 0` beside a non-zero `total` is
    // what a front seat that was never filled now looks like. This is the reason the wire
    // ABI moved to 22.
    // ★★★ 63 -> 67 at `execution_plane_increments.md` §14.41: `fault_buffers_registered`,
    // `fault_buffer_size`, `fault_buffer_pages` and `fault_buffers_malformed` — the
    // replayable fault buffer the guest registers and this port now ANSWERS `NV_OK` to.
    // ⊘ The count is not the point; it is the printer's TRIGGER. Serving `0x20800a9b` buys
    // registration and nothing else — nothing in this build raises a replayable fault or
    // advances `MMU_FAULT_BUFFER_PUT(1)` — and a served row in the control census reads as
    // "handled". So the C shell prints the delivery-unbuilt sentence beside a non-zero count,
    // which makes "serve the control" and "state what serving it did not buy" one act rather
    // than two commits. FOUR numbers and not one for the same reason the three above are
    // three: a re-registration (`> 1`) is a finding this port deliberately does not model,
    // `size` and `pages` check each other, and a malformed ask is a different finding from no
    // ask at all. This is the reason the wire ABI moved to 24.
    // ★★★ 67 -> 72 at §14.41's SECOND rung: `shadow_fault_buffers_{registered,malformed}`,
    // `shadow_fault_buffer_{size,pages,type}`. ⊘ Five NEW numbers rather than five reused
    // ones, and that is the decision: `0x20800a9d` registers a different buffer under a
    // different promise — there the GSP is the declared WRITER of a queue in the guest's own
    // sysmem — so a shared counter could not say which promise a boot took on, and the C
    // printer emits a different sentence for each. `type` is carried RAW because anything but
    // `0` needs Confidential Compute and is therefore a finding, not a configuration. This is
    // the reason the wire ABI moved to 25.
    // ★★★ 72 -> 76 at §14.41's THIRD rung: `access_cntr_buffers_{registered,malformed}`,
    // `access_cntr_buffer_{size,pages}`. Four rather than five — there is no `type` field,
    // because this buffer has only one. ⊘ A third count for a third buffer, and this one is
    // the sharpest: it is the only buffer whose SIZE this device also invents. This is the
    // reason the wire ABI moved to 26.
    // ★★★★ 76 -> 77 at §15.8: `gvas_pub_roots_refused`. ⊘ ONE number, and it is the only
    // thing that says the page-directory ROOT TABLE is still complete. `[measured
    // 2026-08-09, boot `uvm1_b731e3c`]` `ceresolve::published_root` was looking VA spaces up
    // in the EIGHT-ROW report sample during a boot that published ELEVEN distinct, so three
    // address spaces were refused with `CeResolve::NoPublication` — *"the guest published no
    // page-directory root"* — about a guest that had published one. The lookup now has its
    // own, far larger table, and this counts what that table had to refuse; a non-zero value
    // invalidates every `NoPublication` refusal in the same boot. ⊘ The TABLE does not cross
    // (up to 256 rows of 184-byte bodies); its COMPLETENESS does, which is the property a
    // reader of a refusal actually needs. This is the reason the wire ABI moved to 27.
    // ★★★★ 77 -> 81 at §16.13: `fb_resident_valid`, `fb_resident_lo`, `fb_resident_hi`,
    // `fb_resident_pages` — the framebuffer residency CENSUS beside its total. `[measured
    // 2026-08-09, boot `bar1_03a679f`]` the ring's page dumped `nz0/4096` and the total
    // (`resident 368640 bytes`) could not say whether that page was NEVER WRITTEN or
    // WRITTEN WITH ZEROS — `FbStore::read` returns zero *and* `Ok` for an unwritten address,
    // so the two print identically. ⊘ FOUR fields and not two: `_valid` is the PRECONDITION
    // (a device with no framebuffer port has no residency to report, which is not the same
    // fact as a framebuffer in which nothing is resident, and `lo = hi = 0` would be a
    // positive claim about the first), and `_pages` is carried rather than divided out of
    // the byte total so a disagreement between the two is visible instead of arithmetic.
    // This is the reason the wire ABI moved to 30.
    //
    // ★★★★ 81 -> 91, and the wire ABI moved to 31 (§16.16). TEN more words, in two groups:
    //  - `fb_origin_by_writer[5]` — the FIRST-WRITER census. MEASURED at tree e394b69: the
    //    whole tagging mechanism existed and NOTHING CALLED IT, so every write recorded
    //    `Unattributed` and a boot would have measured only the instrument's own default.
    //    The census crosses the wire so the `UNATTRIBUTED` slot is READABLE — that slot is
    //    how a reader tells "a write path is not instrumented" (a fact about us) from a
    //    finding about the guest.
    //  - `fb_sweep_*` (5) — the GPFIFO FORWARD SEARCH. Every other instrument in this file
    //    descends the guest's page tables from the guest's declared ring VA; all of them
    //    share the premise that the table being descended is the right one, and a second
    //    projection of one computation cannot audit the first. The sweep asks the converse
    //    over raw bytes and consults no walk, so its answer and the descent's can genuinely
    //    disagree — which is the entire point.
    // ★★★★ 91 -> 96, and the bump was MISSED by §16.18 (`bar1_reads`, `bar1_writes`,
    // `bar1_faults`, `bar1_pde_base`, `bar1_root_published` — five words). ⊘ The C header
    // WAS updated in the same commit (`kayfabe_shim.h:635-639`, same five names in the same
    // order), so the wire ABI is consistent and it is this arithmetic that was stale — but
    // the gate that would have said so was itself red, which is why nobody read it.
    // ★★★★ 96 -> 105 at §16.30: the `0x00801813 SET_PAGE_DIRECTORY` install record — nine
    // words (`set_page_dir_total`, `_refused`, `_valid`, `_client`, `_object`,
    // `_h_vaspace`, `_phys`, `_num_entries`, `_flags`). This is the reason the wire ABI
    // moved to 33.
    //
    // ⊘ NINE and not two, and `_valid` is the one that earns its word twice over. Every
    // other field is ambiguous at zero and `_h_vaspace` is the worst of them: `0` is a
    // REAL handle naming the client/device pair's implicit VA space
    // (`ogkm-580: ctrl0080dma.h:812-815`), so without a separate latched bit the report
    // could not tell "the guest installed a root into the implicit VAS" — the whole
    // hypothesis §16.30 tests — from "no SET ever arrived". ★ `_num_entries` and `_flags`
    // are carried because they, not the address, decide whether RM's LOCAL
    // `gvaspaceExternalRootDirCommit` survives after this port answers `NV_OK`
    // (`ogkm-580: gpu_vaspace.c:3085-3109`), which is exactly the way this rung can half-
    // succeed.
    //
    // ★ This gate BIT when the fields landed (16800 vs 16728, a 72-byte delta that is
    // 9 x 8 exactly), which is what makes the arithmetic below a check rather than a
    // restatement — cf. §16.18, whose bump was missed because the gate was already red.
    //
    // ★★★★ 106 -> 115 at §16.65: the doorbell census's nine words — the `served` SPLIT
    // (`doorbells_served_locally`, `_forwarded`) and the per-engine partition
    // (`doorbells_by_engine[ENGINE_KINDS]` plus `doorbells_engine_unrouted`). This is the
    // reason the wire ABI moved to 36.
    //
    // ⊘ The array is written as `ENGINE_KINDS` and not as `6`, so a variant added to
    // `kayfabe_rt::EngineKind` moves BOTH sides of this equation and the gate keeps
    // checking rather than quietly agreeing with itself — the same reason `PROBE_ARM_SLOTS`
    // is named above. ★ This gate BIT when the fields landed (26672 vs 26600, a 72-byte
    // delta that is 9 x 8 exactly).
    //
    // ★★★★★ 115 -> 133 at §16.76: the os-event wakeup plane's eighteen words — the GSP
    // stall-vector raises (`gsp_event_raises`, `_unvectored`, `_masked`), the opener
    // (`status_irq_cleared`), the registry's five, the delivery gate's five, and the JOIN's
    // three. This is the reason the wire ABI moved to 37, and this gate BIT when they
    // landed (26816 vs 26672, a 144-byte delta that is 18 x 8 exactly).
    //
    // ⊘ The JOIN's three are here rather than derived at the C side on purpose: a shell that
    // computed "did anything execute" from the doorbell counters would be answering it at
    // TEARDOWN, and the question is about the instant of each announcement.
    assert_eq!(
        size_of::<KayfabeRegAudit>(),
        (124 + 3
            + kayfabe_qemu_raw::shim::ENGINE_KINDS
            + kayfabe_qemu_raw::shim::PROBE_ARM_SLOTS / 2
            + kayfabe_qemu_raw::shim::UNSERVICED_SLOTS)
            * size_of::<u64>()
            // ★★★★ §16.40 — the promote-ctx diagnosis: one row per refusal KIND, plus the
            // distinct count (the 106th u64 above). ⊘ Per-kind and not per-boot because a
            // boot-global "first" latched kernel RM's refusal and never the one the rung
            // was about (`s36_3a0146c_vascensus`).
            + kayfabe_qemu_raw::shim::PROMOTE_DIAG_SLOTS
                * size_of::<kayfabe_qemu_raw::shim::KayfabePromoteDiag>()
            + kayfabe_qemu_raw::shim::BRIDGE_REFUSAL_SLOTS
                * size_of::<kayfabe_qemu_raw::shim::KayfabeBridgeRefusal>()
            + size_of::<kayfabe_qemu_raw::shim::KayfabeIsolateRefusal>()
            + size_of::<kayfabe_qemu_raw::shim::KayfabeDoorbellRefusal>()
            + size_of::<kayfabe_qemu_raw::shim::KayfabeDoorbellServing>()
            + kayfabe_qemu_raw::shim::SERVED_CONTROL_SLOTS
                * size_of::<kayfabe_qemu_raw::shim::KayfabeServedControl>()
            + kayfabe_qemu_raw::shim::NOTIFIER_ARMING_SLOTS
                * size_of::<kayfabe_qemu_raw::shim::KayfabeNotifierArming>()
            + kayfabe_qemu_raw::shim::GVAS_PUBLICATION_SLOTS
                * size_of::<kayfabe_qemu_raw::shim::KayfabeGvasPublication>()
            + kayfabe_qemu_raw::shim::CHANNEL_BIND_SLOTS
                * size_of::<kayfabe_qemu_raw::shim::KayfabeChannelBind>()
    );
    // ★ The bind row's own size, so the C header's arithmetic and this crate's cannot
    // drift apart silently: five u32s, an explicit `reserved` u32 so the u64 count lands
    // on its natural alignment with no HIDDEN padding, and the count — 32 bytes.
    assert_eq!(size_of::<kayfabe_qemu_raw::shim::KayfabeChannelBind>(), 32);
    // ★ The publication rows' own sizes, so the C header's arithmetic and this crate's
    // cannot drift apart silently: 24 bytes per level, 200 per row, no hidden padding.
    // ⊘ `page_shift` is a `u32` here and an `NvU8` on NVIDIA's wire; this is OUR structure
    // and the narrowing already happened in `kayfabe_abi::gvaspacepdes::PdeLevel`, so a
    // `u8` would only have bought three bytes of implicit padding in a hand-mirrored C
    // layout — the exact class the `sizeof` handshake exists to catch late.
    assert_eq!(
        size_of::<kayfabe_qemu_raw::shim::KayfabePdeLevel>(),
        2 * size_of::<u64>() + 2 * size_of::<u32>()
    );
    assert_eq!(
        size_of::<kayfabe_qemu_raw::shim::KayfabeGvasPublication>(),
        6 * size_of::<u32>()
            + 4 * size_of::<u64>()
            + kayfabe_qemu_raw::shim::GVAS_MAX_LEVELS
                * size_of::<kayfabe_qemu_raw::shim::KayfabePdeLevel>()
    );
    // ★ The census rows' own sizes, so the C header's arithmetic and this crate's cannot
    // drift apart silently: 16 bytes and 32 bytes, no hidden padding.
    assert_eq!(
        size_of::<kayfabe_qemu_raw::shim::KayfabeServedControl>(),
        2 * size_of::<u32>() + size_of::<u64>()
    );
    assert_eq!(
        size_of::<kayfabe_qemu_raw::shim::KayfabeNotifierArming>(),
        6 * size_of::<u32>() + size_of::<u64>()
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
    // A write into the framebuffer aperture is REFUSED BY NAME — the case that costs a
    // page-table entry rather than a register value.
    //
    // ★★ §16.18 gave BAR1 an address model — `ChipProfile::bar1_pde_base` is non-zero on GA106, and `plane.rs:2208` falls back to a raw window only when it is ZERO. `[measured 2026-08-09, boots s17/s19]` 15 BAR1 writes resolved through the GMMU, 0 refused. This assertion was left behind by that change. ⊘ It is not a weakening:
    // a dropped window is silent about WHY, and a fault says so at the instant the bytes
    // are lost.
    let w = regs.write(1, 0x0009_008C, 8, 0xDEAD_BEEF);
    assert!(!w.claimed);
    assert_eq!(
        regs.audit().fb_window_writes,
        0,
        "⊘ BAR1 is no longer a dropped window — it has an address model"
    );
    assert_eq!(regs.audit().bar1_faults, 1, "★ it is a NAMED refusal");
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

/// Pull the text a single `info_report` call actually PRINTS out of the C source — the
/// literals only, never the file, for the reason in the test's docs.
fn printed_sentence(text: &str, opens: &str) -> String {
    let at = text
        .find(opens)
        .unwrap_or_else(|| panic!("★ nvkvm.c no longer emits {opens:?} — this test is blind"));
    let tail = &text[at..];
    let end = tail
        .find(");")
        .expect("the info_report call must terminate");
    // The literal pieces are the ODD indices of a split on the quote character: index 0 is
    // `info_report(`, 1 is the first literal, 2 the whitespace between literals, and so on.
    // ⊘ No escaped quotes appear in these sentences; one carrying `\\"` would need a real
    // lexer, and the length assertion at each call site is what would surface that.
    tail[..end].split('"').skip(1).step_by(2).collect()
}

fn assert_two_descriptions_agree(printed: &str, declared: &str, clauses: &[&str]) {
    assert!(
        printed.len() > 120,
        "★ only {} characters of printed sentence were extracted — the shape of the call \
         changed and this test is reading the wrong thing: {printed:?}",
        printed.len(),
    );
    for clause in clauses {
        assert!(
            printed.contains(clause),
            "★ the PRINTED sentence does not carry {clause:?}. The device would report a \
             SERVED control and omit what serving it did not buy, which is the reading the \
             sentence exists to prevent. Printed: {printed:?}"
        );
        assert!(
            declared.contains(clause),
            "★ the ABI constant lost {clause:?} — the two descriptions have drifted. \
             Checked on BOTH sides deliberately: a clause deleted from the constant alone \
             would otherwise leave this test green while the boot report and the ABI \
             disagreed about what is unbuilt."
        );
    }
}

/// ★★★ The delivery-unbuilt sentence is ONE fact written twice, and this is where they are
/// made to agree.
///
/// `kayfabe_abi::faultbuffer::DELIVERY_UNBUILT` defines what serving `0x20800a9b` did **not**
/// buy; `nvkvm.c` is what actually prints it, and it prints its own copy because a `char[]`
/// carried through `KayfabeRegAudit` for a compile-time constant would be an array the guest
/// can never influence. ⊘ That is the right trade **only while the two cannot drift** — and
/// this repository's most-repeated defect is exactly a second description that stops matching
/// the first while every gate stays green (`the_header_and_the_archive_agree_on_the_wire_abi`
/// above is the same shape, and it cost a boot).
///
/// ⊘⊘ **The first version of this test COULD NOT FAIL, and the bite check is what said so.**
/// It searched the whole of `nvkvm.c` for each clause. Mutating the printed string from
/// *"becomes a HANG"* to *"becomes a stall"* left it **green**, because the word `HANG` also
/// appears in the C comment three lines above the call. A gate that reads a whole file passes
/// on any mention anywhere — `gate_read_through_grep_cannot_fail` in its substring form. It
/// now extracts the **printed literal** and searches only that, and the same mutation is red.
///
/// ★ It checks load-bearing CLAUSES rather than the whole string, because the C sentence is
/// wrapped across several string literals and reflowing it must not be a red test, while
/// dropping *"HANG"* — the clause that tells a reader there is no error to look for — must be.
///
/// ⚠ **The clause list is HAND-WRITTEN, and that is this test's remaining weakness, named.**
/// It is checked against BOTH descriptions, so neither can shorten alone; but a clause added
/// to the constant and not to this list is unchecked, which is the
/// `gates_quantified_over_a_list` shape.
///
/// ⊘ There are now TWO sentences and two constants, so the doc above sits on the test and the
/// extraction is a helper: `0x20800a9b` and `0x20800a9d` name different gaps and must keep
/// naming them differently.
#[test]
fn the_c_shell_prints_the_same_unbuilt_half_the_abi_declares() {
    let printer =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../qemu/hw/misc/nvkvm/nvkvm.c");
    let text = std::fs::read_to_string(&printer)
        .unwrap_or_else(|e| panic!("the device source must be readable at {printer:?}: {e}"));

    // ⊘ Non-vacuity first: if the printer stops reading the counts, every clause check below
    // would pass for the wrong reason.
    for counter in [
        "fault_buffers_registered",
        "shadow_fault_buffers_registered",
    ] {
        assert!(
            text.contains(counter),
            "★ nvkvm.c no longer reads {counter} — this test has gone blind"
        );
    }

    assert_two_descriptions_agree(
        &printed_sentence(&text, "info_report(\"nvkvm:   ⊘ fault DELIVERY is UNBUILT"),
        kayfabe_abi::faultbuffer::DELIVERY_UNBUILT,
        &[
            "fault DELIVERY is UNBUILT",
            "MMU_FAULT_BUFFER_PUT(1)",
            "HANG",
            "resume_from_fault.md",
        ],
    );

    // ★★ The SECOND sentence, and it must stay a second one. `0x20800a9d`'s gap is not
    // `0x20800a9b`'s: there we decline to move a register the guest polls, here we decline to
    // be the WRITER the guest is waiting on — and "unbuilt" alone would be false, because the
    // RC + error notifier IS built and is what happens instead.
    assert_two_descriptions_agree(
        &printed_sentence(
            &text,
            "info_report(\"nvkvm:   ⊘ shadow-queue PUSH is UNBUILT",
        ),
        kayfabe_abi::faultbuffer::SHADOW_DELIVERY_UNBUILT,
        &[
            "shadow-queue PUSH is UNBUILT",
            "WRITER",
            "RC on the channel",
            "simulated_gpu_fault.md",
        ],
    );

    // ★★ The THIRD sentence. Its gap is the sharpest of the three: this is the buffer whose
    // SIZE this port also invents, so the sentence has to carry the register that was faked
    // as well as the delivery that was not built.
    assert_two_descriptions_agree(
        &printed_sentence(
            &text,
            "info_report(\"nvkvm:   ⊘ access-counter NOTIFICATION is UNBUILT",
        ),
        kayfabe_abi::faultbuffer::ACCESS_COUNTER_DELIVERY_UNBUILT,
        &[
            "access-counter NOTIFICATION is UNBUILT",
            "0xB83110",
            "deliberate fiction",
            "resume_from_fault.md",
        ],
    );

    // ★★ The FOURTH sentence, §14.42's, and the only one of the four that is a MEASUREMENT
    // rather than a prediction: boot `ce1442` served these controls and then timed out
    // waiting for a CE completion, so the sentence names an observed failure shape. ⊘ Its
    // gap is also the only one that is not a *buffer* — the other three are about writing
    // into memory the guest allocated; this one is about retiring a payload.
    assert_two_descriptions_agree(
        &printed_sentence(&text, "info_report(\"nvkvm:   ⊘ CE COMPLETION is UNBUILT"),
        kayfabe_abi::cepce::CE_COMPLETION_UNBOUGHT,
        &[
            "CE COMPLETION is UNBUILT",
            "supported=NV_TRUE",
            "ce_utils.c:349",
            "TIMEOUT",
            "execution_plane_increments.md",
        ],
    );

    let all = [
        kayfabe_abi::faultbuffer::DELIVERY_UNBUILT,
        kayfabe_abi::faultbuffer::SHADOW_DELIVERY_UNBUILT,
        kayfabe_abi::faultbuffer::ACCESS_COUNTER_DELIVERY_UNBUILT,
        kayfabe_abi::cepce::CE_COMPLETION_UNBOUGHT,
    ];
    for (i, x) in all.iter().enumerate() {
        for y in all.iter().skip(i + 1) {
            assert_ne!(
                x, y,
                "★ two different gaps must not collapse into one sentence"
            );
        }
    }
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

use kayfabe_qemu_raw::shim::{
    GuestRamSource, IsolatePlane, guest_ram_is_reachable_on, guest_ram_source_from,
    isolate_factory, isolate_plane_from,
};

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

    let (f, backing, _exports) = isolate_factory(IsolatePlane::Stillborn, GuestRamSource::None)
        .expect("the default plane builds");
    // ★ §5.7 — a plane that adopted no guest-RAM block claims no backing identity. `None`
    // here is what makes the stated-layout report silent on an unarmed boot, which is what
    // keeps the negative control comparable to the armed run.
    assert!(
        backing.is_none(),
        "★ the stillborn plane named a guest-RAM block it never adopted"
    );
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
        let (status, why) = isolate_factory(plane, GuestRamSource::None)
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
            isolate_factory(plane, GuestRamSource::None).is_ok(),
            "★ {plane:?} could not be built in an archive that links kayfabe-isolate-host"
        );
    }
}

// ── The guest-RAM crossing's selector ──────────────────────────────────────────────

/// A source name this build does not know is a **refusal**, never a quiet `none`.
///
/// ⊘ The failure this guards is not a typo per se: `none` is the arm in which every
/// isolate is blind to guest memory, so a mistyped value that defaulted to it produces a
/// run that *looks* armed, behaves exactly like its own negative control, and only says so
/// at the first doorbell — twenty seconds later, inside a log full of refusals.
#[test]
fn an_unknown_guest_ram_source_is_refused_rather_than_defaulted() {
    assert_eq!(guest_ram_source_from(None), Ok(GuestRamSource::None));
    assert_eq!(
        guest_ram_source_from(Some("memfd")),
        Ok(GuestRamSource::HypervisorMemfd)
    );
    for bad in ["", "Memfd", "MEMFD", "memfd ", "yes", "1", "true", "shared"] {
        let (status, why) = guest_ram_source_from(Some(bad))
            .expect_err("★ an unknown source must refuse the device");
        assert_eq!(status.code(), Status::Unsupported.code());
        assert!(
            why.contains("KAYFABE_GUEST_RAM"),
            "the refusal must name the variable to fix: {why}"
        );
    }
    // Every arm round-trips through its own spelling — quantified over `ALL`, so a source
    // added tomorrow is checked tonight rather than added to a list here.
    for s in GuestRamSource::ALL {
        assert_eq!(GuestRamSource::parse(s.as_str()), Some(s));
    }
}

/// ★★ Asking for guest RAM on the plane that has **no isolates** is refused by name.
///
/// ⊘ Not a no-op. `stillborn` retires every isolate at birth, so the grant has no holder;
/// a run configured that way is indistinguishable from its own control, which is the one
/// thing this file refuses everywhere else. The control is the other two planes: they must
/// pass, or this test would be asserting that the crossing is simply unavailable.
#[test]
fn guest_ram_on_the_stillborn_plane_is_refused_and_reachable_on_the_others() {
    let (status, why) =
        guest_ram_is_reachable_on(IsolatePlane::Stillborn, GuestRamSource::HypervisorMemfd)
            .expect_err("★ a crossing into a plane with no isolates must refuse");
    assert_eq!(status.code(), Status::Unsupported.code());
    assert!(
        why.contains("KAYFABE_ISOLATES") && why.contains("KAYFABE_GUEST_RAM"),
        "the refusal must name BOTH variables, since either one is the fix: {why}"
    );
    assert_eq!(
        guest_ram_is_reachable_on(IsolatePlane::Stillborn, GuestRamSource::None),
        Ok(()),
        "the shipped default is not a refusal"
    );
    for plane in [IsolatePlane::Loopback, IsolatePlane::Real] {
        assert_eq!(
            guest_ram_is_reachable_on(plane, GuestRamSource::HypervisorMemfd),
            Ok(()),
            "★ the control: {plane:?} really can hold the grant"
        );
    }
}

/// ★★★★★ **The rung itself: the shim reaches the hypervisor's own guest-RAM descriptor,
/// and refuses BY NAME AT STARTUP when there is none.**
///
/// `guest_ram_crossing.md` §4.4: the isolate side landed and *"no VMM code calls
/// `with_guest_ram`"*. This is the call, asserted at the seam a live boot exercises.
///
/// # ⊘ Why both halves are ONE test, in this order
///
/// The refusal alone would pass against an arm that is simply unimplemented — *"returns
/// `Unsupported` always"* satisfies it perfectly. So the same call is made twice in the
/// same process with **one thing changed**: a shared-mapped `memfd` of the hypervisor's
/// name appears between them. Absent → refused, naming the launch flag; present →
/// accepted. That is a negative control rather than a pair of independent assertions, and
/// it cannot be split into two `#[test]`s because this binary runs its tests in parallel
/// threads of one process — the "absent" half would see the other half's block.
///
/// ⊘ It does not assert that anything was *mapped*. Nothing is: the grant is handed to a
/// factory, and mapping happens only when a `GuestRamGrant` orders it. Claiming more here
/// would be the shape §4.5 already refuses.
#[cfg(feature = "host-isolates")]
#[test]
fn the_shim_finds_guest_ram_when_it_is_there_and_refuses_by_name_when_it_is_not() {
    use kayfabe_linux_raw::{
        Backing, CachePolicy, HostPageSize, HostProt, MappedRegion, SharedRam,
    };
    use kayfabe_qemu_raw::shim::QEMU_MACHINE_RAM_MEMFD;

    // --- absent -----------------------------------------------------------------
    let (status, why) = isolate_factory(IsolatePlane::Loopback, GuestRamSource::HypervisorMemfd)
        .err()
        .expect("★ with no shared guest-RAM memfd in this process the shim must refuse");
    assert_eq!(status.code(), Status::Unsupported.code());
    assert!(
        why.contains("memory-backend-memfd") && why.contains("share=on"),
        "★ the refusal must name the LAUNCH FLAG, because that is the deployment fact no \
         code gate can observe and the only thing an operator can change: {why}"
    );

    // --- the one thing that changes ---------------------------------------------
    let page = HostPageSize::query();
    let name = std::ffi::CString::new(QEMU_MACHINE_RAM_MEMFD).expect("a name with no NUL");
    let ram = SharedRam::create_named(&name, 4 * page.bytes()).expect("a shared block");
    let _mapped = MappedRegion::map(
        Backing::SharedFile {
            fd: ram.as_backing_fd(),
            offset: 0,
        },
        4 * page.bytes(),
        HostProt::ReadWrite,
        CachePolicy::WriteBack,
        page,
    )
    .expect("★ the block must be MAPPED SHARED — that is the property the census keys on");

    // --- present ----------------------------------------------------------------
    assert!(
        isolate_factory(IsolatePlane::Loopback, GuestRamSource::HypervisorMemfd).is_ok(),
        "★★★ the same call, one shared-mapped memfd later, must now find it — otherwise \
         the refusal above is about an unimplemented arm and not about the descriptor"
    );
}

// ── The probe-arm device property: strict parse, reported set ──────────────────────

#[test]
fn a_probe_string_with_junk_refuses_the_device_rather_than_booting_probe_off() {
    use kayfabe_qemu_raw::shim::{Regs, Status};

    // ⊘ Exact-variant assertions. The predecessor env-var parser dropped unparseable
    // tokens silently, which is how three boots ran probe-off while looking armed.
    let (status, msg) = Regs::create_probed(0, "3S").expect_err("junk must refuse");
    assert_eq!(status, Status::Malformed);
    assert!(
        msg.contains("probe-arm-notifier"),
        "the refusal names the property so the operator knows which knob to fix: {msg}"
    );
    let (status, _) = Regs::create_probed(0, "1,2,3,4,5,6,7,8,9").expect_err("too many");
    assert_eq!(status, Status::Malformed);
}

#[test]
fn the_audit_reports_the_probe_set_the_device_ran_with() {
    use kayfabe_qemu_raw::shim::Regs;

    // The shipping constructor: empty, and REPORTED as empty.
    let stock = Regs::create(0).expect("servable");
    assert_eq!(stock.audit().probe_arm_len, 0);
    assert_eq!(stock.audit().probe_arm, [0u32; 8]);

    // A probed device: the audit states the set, values and count both — this is the
    // line in the boot's own report that proves what it ran with.
    let probed = Regs::create_probed(0, "35,37").expect("servable");
    let audit = probed.audit();
    assert_eq!(audit.probe_arm_len, 2);
    assert_eq!(audit.probe_arm[0], 35);
    assert_eq!(audit.probe_arm[1], 37);
    assert_eq!(&audit.probe_arm[2..], &[0u32; 6]);
}

// ── §16.6: the two REPORT caps that hid the row deciding the wall ──────────────────

/// ★★★ **A sentence that did not fit must SAY it did not fit.**
///
/// `[measured 2026-08-09, boot `vaspan_994bbdc`]` the doorbell refusal was 262 bytes in a
/// 448-byte wire buffer filled by a bare `min()`. §16.6 appends the deciding VA space's
/// whole publication body to that sentence — four `PdeLevel`s, ~180 bytes — which lands it
/// on the cap, and the levels are at the **END**, so they are the first thing a silent
/// truncation eats. ⊘ The old copy produced a byte-identical log line whether it had clipped
/// or not, which is the shape this project has now paid for nine times in one night: a
/// bounded collection that cannot report its own saturation makes absence and truncation the
/// same observation.
#[test]
fn a_clipped_refusal_sentence_states_that_it_was_clipped_and_how_long_it_really_was() {
    use kayfabe_qemu_raw::shim::copy_sentence;

    // Fits: byte-for-byte, and NO marker — a marker on a complete sentence would be its own
    // false statement.
    let mut buf = [0u8; 64];
    let n = copy_sentence(&mut buf, "root=0x4000/ap1/sh47") as usize;
    assert_eq!(&buf[..n], b"root=0x4000/ap1/sh47");
    assert!(
        !String::from_utf8_lossy(&buf[..n]).contains("CLIPPED"),
        "a sentence that fitted must not claim it was cut"
    );

    // Does not fit: the marker is present, it names the TRUE length, and the write stayed
    // inside the buffer.
    let long = "L0=0x4000/sz0x20/ap1/sh47".repeat(40);
    let mut buf = [0u8; 128];
    let n = copy_sentence(&mut buf, &long) as usize;
    assert!(n <= buf.len(), "the copy must never run past the buffer");
    let got = std::str::from_utf8(&buf[..n]).expect("a sentence is UTF-8 or it is unreadable");
    assert!(
        got.contains("CLIPPED"),
        "a clipped sentence must be distinguishable from a short one: {got}"
    );
    assert!(
        got.contains(&long.len().to_string()),
        "the marker carries the TRUE length, which is what decides whether to widen the \
         buffer or shorten the sentence: {got}"
    );

    // ⊘ And it clips on a CHARACTER boundary: these sentences carry `⊘`, `★` and `—`, and a
    // byte cut prints as a replacement character in the one line an operator reads.
    let stars = "★".repeat(200);
    let mut buf = [0u8; 100];
    let n = copy_sentence(&mut buf, &stars) as usize;
    std::str::from_utf8(&buf[..n]).expect("clipped mid-character — the cut must land on a char");
}

/// ★★★★ **The report may not clip over the row that decides the boot.**
///
/// `[measured 2026-08-09]` six consecutive boots (`uvm1_b731e3c` … `vaspan_994bbdc`) each
/// published **11 distinct** VA spaces and printed the first **eight**, so
/// `(hClient 0xc1d0000a, hObject 0xcaf00005)` — the pair every one of those boots names in
/// its doorbell refusal — had its body printed in none of them. §16.3 repaired the *lookup*
/// (`GvasPubSnapshot::roots`, 256 rows) and left the *report* at eight.
///
/// ⊘ This is not "8 is wrong and 32 is right". It is: the report's cap must exceed what a
/// real boot publishes, and the number that boot published is **11**. Asserted against the
/// measurement so shrinking the cap back under it goes red here rather than three boots
/// later.
#[test]
#[allow(clippy::assertions_on_constants)]
fn the_publication_report_shows_more_rows_than_a_real_boot_publishes() {
    // The measured distinct-publication count of every boot 2026-08-09.
    const MEASURED_DISTINCT_PUBLICATIONS: usize = 11;
    // ⊘ Yes, both sides are constants — that IS the property, and clippy's `const { … }`
    // suggestion is declined on purpose: a const block cannot carry the interpolated
    // sentence that names the boot their number is now below, and a failure whose message
    // rots into "assertion failed" is a failure somebody silences.
    assert!(
        kayfabe_qemu_raw::shim::GVAS_PUBLICATION_SLOTS > MEASURED_DISTINCT_PUBLICATIONS,
        "the wire report holds {} rows and a measured boot publishes {MEASURED_DISTINCT_PUBLICATIONS} \
         distinct VA spaces — the row the doorbell refusal NAMES would print nowhere",
        kayfabe_qemu_raw::shim::GVAS_PUBLICATION_SLOTS
    );
    // ★ And the two halves must agree: the archive fills `GVAS_PUBLICATION_SLOTS` from a
    // `Vec` the device caps at `GVAS_PUBLICATION_SAMPLE_MAX`, so a slots array larger than
    // the sample is dead space and a smaller one is a second, silent clip.
    assert_eq!(
        kayfabe_qemu_raw::shim::GVAS_PUBLICATION_SLOTS,
        kayfabe_device::gvaspub::GVAS_PUBLICATION_SAMPLE_MAX,
        "the wire array and the device's sample must be the same width or one of them clips \
         without saying so"
    );
}

/// ★★★★ **The row carries every field the three §16.6 outcomes are separated by.**
///
/// The rung is: print the publication row for `(0xc1d0000a, 0xcaf00005)` — whole body, all
/// four levels — and the three fixes it can point at are *a real root RM had not yet
/// backed*, *a stale publication last-write-wins picked over a later real one*, and *the
/// body was decoded from the wrong arm*. Each needs a different field, so a row that prints
/// the root address alone (which is what six boots printed) separates none of them.
///
/// ⊘ **This proves the formatter and nothing downstream of it.** Whether the string reaches
/// a boot log is decided by the doorbell sentence buffer and by who calls the probe; the
/// only oracle for that is a boot, and this test must not be mistaken for one — observability
/// failure #6 of 2026-08-09 was an acceptance predicate satisfied by a test calling the
/// function directly.
#[test]
fn the_publication_row_prints_the_arm_the_count_and_every_declared_level() {
    use kayfabe_abi::gvaspacepdes::{PdeLevel, ServerReservedPdes};
    use kayfabe_device::gvaspub::{GvasPubLog, GvasPublication};

    let mut levels = [PdeLevel::default(); kayfabe_qemu_raw::shim::GVAS_MAX_LEVELS];
    // `[measured 2026-08-09, boot `vaspan_994bbdc`, rev `994bbdc10`]` the shape every
    // healthy root in that boot publishes, and the anomaly beside it: root `0x4000` where
    // the working VA spaces sit at `~0x2efa_xxxx`.
    levels[0] = PdeLevel {
        phys_address: 0x4000,
        size: 0x20,
        aperture: 1,
        page_shift: 47,
    };
    levels[1] = PdeLevel {
        phys_address: 0x2efa_9b000,
        size: 0x1000,
        aperture: 1,
        page_shift: 38,
    };
    levels[2] = PdeLevel {
        phys_address: 0x2efa_9a000,
        size: 0x1000,
        aperture: 1,
        page_shift: 29,
    };
    levels[3] = PdeLevel {
        phys_address: 0x2efa_99000,
        size: 0x1000,
        aperture: 1,
        page_shift: 21,
    };
    let pdes = ServerReservedPdes {
        h_subdevice: 0,
        subdevice_id: 0,
        page_size: 0x200000,
        virt_addr_lo: 0x1_0000_0000,
        virt_addr_hi: 0x1_1fff_ffff,
        num_levels: 4,
        levels,
    };
    let log = GvasPubLog::new();
    log.note(GvasPublication {
        cmd: 0x90f1_0106,
        client: 0xc1d0_000a,
        object: 0xcaf0_0005,
        pdes,
        count: 1,
    });
    let snap = log.snapshot();
    let s = kayfabe_qemu_raw::shim::publication_row(&snap, 0xc1d0_000a, 0xcaf0_0005);

    // The ARM — separates "decoded from the wrong arm" from the other two.
    assert!(s.contains("arm0x90f10106"), "{s}");
    // The COUNT — `> 1` is the stale-publication finding, so it must be printed even at 1.
    assert!(s.contains("x1"), "{s}");
    // ALL FOUR levels, each with the size and aperture the root projection drops.
    assert!(s.contains("L0=0x4000/sz0x20/ap1/sh47"), "{s}");
    assert!(s.contains("L1=0x2efa9b000/sz0x1000/ap1/sh38"), "{s}");
    assert!(s.contains("L2=0x2efa9a000/sz0x1000/ap1/sh29"), "{s}");
    assert!(s.contains("L3=0x2efa99000/sz0x1000/ap1/sh21"), "{s}");
    // ⊘ And NOT the meaningless tail: `levels[4..]` decode so the re-encode is faithful and
    // carry no claim, so printing them would put addresses in the log the guest never made.
    assert!(
        !s.contains("L4="),
        "levels past num_levels are not the guest's claim: {s}"
    );

    // ★ An ABSENT row is qualified by the table's own completeness — "no row for this pair"
    // means "the guest published none" ONLY while nothing was refused by the cap, and §16.3
    // is the boot where confusing those two was the entire bug.
    let miss = kayfabe_qemu_raw::shim::publication_row(&snap, 0xc1d0_000a, 0xdead_beef);
    assert!(miss.contains("ABSENT-FROM-ROOT-TABLE"), "{miss}");
    assert!(miss.contains("REFUSED-BY-CAP"), "{miss}");
}

/// ★★★★ **§16.8: the framebuffer dump reports REFUSED, ZERO and DATA as three things.**
///
/// The rung is *"dump the 32 bytes our framebuffer holds at `0x4000` and the 4 KiB at
/// `0x5000`"*, and its two outcomes are *plausible page-directory entries ⇒ a real pool
/// whose base we lack* versus *zeros ⇒ the walk has been descending noise*. ⊘ There is a
/// **third** outcome the rung does not name and the instrument must not collapse into the
/// second: an address the store does not back at all. `FbStore::read` returns **zero and
/// `Ok`** for an unwritten address *inside* the framebuffer, so "empty" and "refused" are
/// genuinely different facts — and this project's own
/// `c_oracle_empty_rows_are_wrong` is the same mistake in the other direction.
///
/// ⊘ Proves the formatter, not that any boot log contains it — see `publication_row`'s note.
#[test]
fn the_framebuffer_dump_separates_refused_from_empty_from_written() {
    use kayfabe_device::fbwin::{FbStore, SparseFb};

    // A framebuffer that covers 1 MiB. `0x4000` is inside it and unwritten; `0x900000` is
    // outside it entirely.
    let mut fb = SparseFb::new(1 << 20);
    // Written data at 0x8000, so the census has something to count.
    fb.write(0x8000, &[0xAB; 64]).expect("inside the aperture");

    // ⊘ Read back through the store the dump reads, so the fixture cannot claim a
    // capability the real path lacks (2026-08-09 observability failure #2: a fixture that
    // normalised away the field whose corruption was the bug).
    let mut head = [0u8; 32];
    fb.read(0x4000, &mut head).expect("unwritten but inside");
    assert_eq!(
        head, [0u8; 32],
        "an unwritten address inside the FB reads ZERO and Ok"
    );
    let mut d = [0u8; 32];
    fb.read(0x8000, &mut d).expect("written");
    assert_eq!(&d[..8], &[0xABu8; 8], "written bytes read back");
    let mut o = [0u8; 32];
    assert!(
        fb.read(0x900000, &mut o).is_err(),
        "⊘ an address the store does not back must REFUSE, never read as zeros — otherwise \
         the dump's 'empty' outcome absorbs a wiring fault"
    );
}

/// ★★★★ **§16.65 — the per-engine census's LABELS cannot drift from its BUCKETS.**
///
/// # ⊘ What this catches that nothing else can
///
/// `KayfabeRegAudit::doorbells_by_engine` is an array whose meaning is entirely
/// **positional**: bucket `i` means whatever `kayfabe_rt::EngineKind::ALL[i]` is. The C
/// printer asks `kayfabe_shim_engine_kind_name` for the label, so there is one ordering —
/// but the C strings themselves are literals in `shim_unsafe.rs`, and a variant inserted
/// into `EngineKind` would shift every bucket **without breaking any build**. The census
/// would keep printing all six columns, with plausible numbers under the wrong names, and
/// a reader comparing two boots would compare two different questions.
///
/// ⊘ That failure mode is this campaign's own, twice over: a correct instrument answering
/// the wrong question, and two projections of one fact that disagree with the weaker one
/// being the only thing anybody reads. So the literals are pinned to the enum here.
///
/// ★ Non-vacuity is structural: the loop below is over the enum's own `ALL`, so an added
/// variant lengthens it and the length assertion fails before the names are ever compared.
#[test]
fn the_engine_census_labels_match_the_enum_they_bucket() {
    let rust = kayfabe_rt::engine_kind_names();
    assert_eq!(
        rust.len(),
        kayfabe_qemu_raw::shim::ENGINE_KINDS,
        "★ the census's width and the enum's variant count are the same number, and the C \
         loop is bounded by the first while the buckets are filled by the second",
    );
    for (idx, want) in rust.iter().enumerate() {
        // ⊘ The SAFE half of the entry point, deliberately: `kayfabe_shim_engine_kind_name`
        // is `engine_kind_c_name(..).as_ptr()` and nothing else, so this checks the whole of
        // its decision — and the test tree does not have to name the relaxation keyword and
        // breach the `*_unsafe.rs` naming audit to reach an FFI table.
        let got = kayfabe_qemu_raw::shim_unsafe::engine_kind_c_name(idx as u32);
        assert_eq!(
            got.to_str().expect("the labels are ASCII"),
            *want,
            "★ bucket {idx} is labelled '{}' but holds {want} doorbells — a census whose \
             columns are mislabelled reads as a complete partition of the wrong thing",
            got.to_string_lossy(),
        );
    }
    // ⊘ And the out-of-range answer is a printable label, never empty and never absent: the
    // C hands this straight to `%s`, so anything but a real static string is undefined
    // behaviour in the caller.
    assert_eq!(
        kayfabe_qemu_raw::shim_unsafe::engine_kind_c_name(rust.len() as u32),
        c"?",
    );
}

// =====================================================================================
// ★★★★★ §16.80 — the `Ce` EXECUTOR selector, which is a DIFFERENT question from the plane
// =====================================================================================
//
// ⊘ Same scope caveat as the plane-selector block above: these drive the pure half
// ([`ce_executor_from`]) and never read the process-global. What they DO pin, and what
// the plane block could not, is the **composition of the two selectors** — the boolean
// `SharedDoorbell::local_ce_is_the_only_executor` is built from.
//
// The measurement behind it: `[boot `w219_fe65678_realbase`, rev `fe65678`]` with
// `KAYFABE_ISOLATES=real` and no CE selector, the host CE path refused all three of the
// scrubber's submissions BEFORE submission (`by=Ours src=Constant(0)`) and the guest died
// at `RmInitAdapter failed! (0x25:0x65:1249)` — 40 s before `cuCtxCreate` exists.

use kayfabe_qemu_raw::shim::{CeExecutorChoice, ce_executor_from};

/// The composition, stated as a function so the test and the shim cannot drift: the
/// shell's CPU executor is the only executor when **either** no plane can issue a host
/// verb **or** the operator did not ask for the host CE arm.
fn only_local(plane: IsolatePlane, ce: CeExecutorChoice) -> bool {
    plane == IsolatePlane::Stillborn || ce == CeExecutorChoice::Local
}

/// ⊘ **The default must not change the plane's default arm at all.** The shipped
/// configuration (`KAYFABE_ISOLATES` unset) already had `local_ce_is_the_only_executor ==
/// true`; adding a second selector must be a no-op there, or every `s`-series and `w`-series
/// ctl boot in `traces/guest_boots/` stops being comparable to the next one.
#[test]
fn the_default_ce_executor_leaves_the_shipped_arm_byte_identical() {
    assert_eq!(ce_executor_from(None), Ok(CeExecutorChoice::Local));
    assert!(
        only_local(IsolatePlane::Stillborn, CeExecutorChoice::Local),
        "★ the shipped arm lost its CPU copy-engine executor"
    );
    // ★ And it is unchanged even if somebody asks for the host arm on a plane that
    // provably cannot serve it — the first term still holds, so a `Stillborn` build
    // cannot be configured into having no executor at all.
    assert!(
        only_local(IsolatePlane::Stillborn, CeExecutorChoice::Host),
        "★ `KAYFABE_CE_EXECUTOR=host` on a stillborn plane took the only executor away \
         and put nothing in its place — which is exactly the w219 failure, reproduced by \
         configuration instead of by inference"
    );
}

/// ★★★ **The refutation, as an executable statement.** On a live plane the DEFAULT keeps
/// the local executor, and only an explicit opt-in hands `Ce` to the host — which is the
/// arm measured to kill `RmInitAdapter`.
#[test]
fn a_live_plane_keeps_the_local_ce_executor_unless_asked_otherwise() {
    for plane in [IsolatePlane::Loopback, IsolatePlane::Real] {
        assert!(
            only_local(plane, CeExecutorChoice::Local),
            "★ {plane:?} + the default took the CPU copy-engine executor away. \
             `w219_fe65678_realbase` is what that costs: three `CE-SUBMIT … REFUSED \
             BEFORE SUBMISSION` lines and `RmInitAdapter failed! (0x25:0x65:1249)`."
        );
        assert!(
            !only_local(plane, CeExecutorChoice::Host),
            "★ {plane:?} + `host` no longer reaches the forwarding plane — the \
             previously-measured arm (`p2_29e7c25_planereal`, `w209_ffc80f8_real`, \
             `w219_fe65678_realbase`) has been deleted rather than made optional, and a \
             deleted configuration cannot be a control."
        );
    }
}

/// Every executor round-trips through its own spelling, quantified over `ALL`, with the
/// same non-vacuity check the plane block uses.
#[test]
fn every_ce_executor_round_trips_through_its_own_spelling() {
    for ce in CeExecutorChoice::ALL {
        assert_eq!(
            ce_executor_from(Some(ce.as_str())),
            Ok(ce),
            "★ {ce:?} does not parse from the name it prints"
        );
    }
    let mut names: Vec<&str> = CeExecutorChoice::ALL.iter().map(|c| c.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(
        names.len(),
        CeExecutorChoice::ALL.len(),
        "two executors share a name"
    );
}

/// ⊘ A near-miss is a refusal to realize, never a quiet default — the property that keeps
/// an evidence run distinguishable from its own negative control.
#[test]
fn a_value_that_is_not_a_ce_executor_name_refuses_rather_than_defaulting() {
    for bad in [
        "",
        "Local",
        "LOCAL",
        "local ",
        " local",
        "hosts",
        "real",
        "cpu",
        "1",
        "true",
        "\u{fffd}invalid",
    ] {
        let (status, why) = ce_executor_from(Some(bad))
            .expect_err(&format!("★ {bad:?} was ACCEPTED as an executor name"));
        assert_eq!(
            status.code(),
            kayfabe_qemu_raw::shim::Status::Unsupported.code()
        );
        for ce in CeExecutorChoice::ALL {
            assert!(
                why.contains(ce.as_str()),
                "★ the refusal for {bad:?} does not name `{}`; message was: {why}",
                ce.as_str()
            );
        }
    }
}

// =====================================================================================
// ★★★★★ §16.81 — WHOSE `Ce` DOORBELL IS IT? The term the executor gate never asked.
// =====================================================================================
//
// ⊘ These drive the pure predicate [`forwarding_plane_owns_ce`], which is the whole of the
// gate at `SharedDoorbell::try_ce_submission` — extracted precisely so it can be asserted
// without a device, a plane or a guest.
//
// The measurement they are written against: `[boot `w231a_ad4ed3c_ceexec_host`, rev
// `ad4ed3c`, `KAYFABE_ISOLATES=real KAYFABE_CE_EXECUTOR=host`]` — ONE doorbell arrived,
// `proc=0 chan=1 pdb=0x2efa9c000`; the gate handed it to the forwarding plane; the pin
// refused the same token by name (`REFUSED SystemDataPlane`); the isolate refused the copy
// (`by=Ours src=Constant(0)`); `RmInitAdapter failed! (0x25:0x65:1249)`.

use std::collections::{BTreeMap, BTreeSet};

use kayfabe_core::ProcId;
use kayfabe_core::channel_kind::{GuestChannelKind, HostChannelKind};
use kayfabe_core::gpu::Gpu;
use kayfabe_core::project::{ProcBoundary, SYSTEM_ANCHOR};
use kayfabe_core::rmgraph::ClientKey;
use kayfabe_qemu_raw::shim::forwarding_plane_owns_ce;

/// ★★★ **The kind a channel of `proc` carries, obtained from the PRODUCTION derivation.**
///
/// ⊘ It deliberately does not restate the rule. `ProcBoundary::channel_kind` is the one
/// derivation the projection itself uses (`Gpu::sync_proc_to_boundary` calls exactly this
/// method to stamp `Channel::kind`), so a test that spelled `if proc == SYSTEM_PROC` would
/// be asserting the gate against a private copy of the rule — the second projection this
/// whole rung exists to delete.
///
/// The only thing restated here is the **routing** step the projection performs elsewhere:
/// the system component's channels route to [`Gpu::SYSTEM_PROC`] and a user component's do
/// not (`Spine::refresh`, `by_vchid`). That join is what
/// `tests/tests/channel_kind_declaration.rs` asserts on a live device, over real RM events,
/// which is the half a unit test cannot reach.
fn kind_of(proc: ProcId) -> GuestChannelKind {
    let anchor = if proc == Gpu::SYSTEM_PROC {
        SYSTEM_ANCHOR
    } else {
        // A user component's anchor is the smallest client DECLARATION in it. Any
        // non-reserved value will do; `RmGraph::apply` refuses `RESERVED_CLIENT` as guest
        // input, so no user component can ever collide with `SYSTEM_ANCHOR`.
        kayfabe_core::ProcAnchor(ClientKey {
            client: kayfabe_arch::ids::HClient(0xc1d0_0000 | proc.0),
            incarnation: 0,
        })
    };
    ProcBoundary {
        anchor,
        clients: BTreeSet::new(),
        vases: BTreeMap::new(),
        channels: BTreeMap::new(),
    }
    .channel_kind()
}

/// ★★★★★ **THE NON-CHANGE PROOF — the predecessor, exhaustively, on every input.**
///
/// `[2026-08-11]` The third term stopped being `proc != Gpu::SYSTEM_PROC` computed inside
/// this gate and became a read of the declared
/// [`kayfabe_core::channel_kind::GuestChannelKind`]. That term is load-bearing and **12
/// boots** of `RmInitAdapter failed! (0x25:0x65:1249)` paid for it, so the obligation is
/// not *"the new gate is sensible"* — it is **the truth table did not move**.
///
/// ⊘ The predecessor is written out here as an oracle rather than cited, because a
/// deleted function cannot be differentialled against. It is a verbatim transcription of
/// `forwarding_plane_owns_ce`'s body at `6fcedac`:
///
/// ```text
/// has_vas_pdb && !local_ce_is_the_only_executor && proc != Gpu::SYSTEM_PROC
/// ```
///
/// ★ The quantifier is the whole point: **every** proc in the fixture × both values of
/// `has_vas_pdb` × both values of `local_ce_is_the_only_executor` — the complete input
/// space of the two boolean terms, and both sides of the one that changed.
#[test]
fn the_new_third_term_has_exactly_the_predecessors_truth_table() {
    fn predecessor(proc: ProcId, has_vas_pdb: bool, local_only: bool) -> bool {
        has_vas_pdb && !local_only && proc != Gpu::SYSTEM_PROC
    }
    let mut saw_true = 0usize;
    let mut saw_false = 0usize;
    for proc in [Gpu::SYSTEM_PROC, ProcId(1), ProcId(2), ProcId(4242)] {
        for has_vas_pdb in [true, false] {
            for local_only in [true, false] {
                let was = predecessor(proc, has_vas_pdb, local_only);
                let now = forwarding_plane_owns_ce(kind_of(proc), has_vas_pdb, local_only);
                assert_eq!(
                    now, was,
                    "★ THE TRUTH TABLE MOVED at proc={proc:?} has_vas_pdb={has_vas_pdb} \
                     local_only={local_only}: the predecessor said {was}, the declared-kind \
                     gate says {now}. This term is what `w231a_ad4ed3c_ceexec_host` bought \
                     with `1 arrived, 0 served, 1 REFUSED` → `RmInitAdapter failed! \
                     (0x25:0x65:1249)`; naming the axis was not licensed to change it."
                );
                if was { saw_true += 1 } else { saw_false += 1 }
            }
        }
    }
    // ⊘ NON-VACUITY. A predicate that answered `false` everywhere would satisfy every
    // assertion above, and it is exactly the cheap wrong fix the negative control below
    // guards against — asserted here too, so the sweep cannot pass by being uniform.
    assert!(
        saw_true > 0 && saw_false > 0,
        "★ the sweep never observed both answers ({saw_true} true / {saw_false} false), so \
         agreeing with the predecessor proves nothing about the term."
    );
}

/// ★★★ **THE PROPOSITION, and it is the one the target evaluates.** In the exact
/// configuration `w231a` ran — a live plane (`local_ce_is_the_only_executor == false`) and a
/// channel the core CAN address (`vas_pdb == Some`) — an **emulated** channel's doorbell
/// (the guest kernel's; the system proc's) must stay the shell's.
#[test]
fn an_emulated_channels_ce_doorbell_is_never_the_forwarding_planes() {
    assert!(
        !forwarding_plane_owns_ce(GuestChannelKind::Emulated, true, false),
        "★ an emulated (guest-KERNEL) channel's `Ce` doorbell was handed to the forwarding \
         plane. `l1_concurrency.md` §12.26 (carried in `FwdFault::SystemDataPlane`'s own \
         docs) names the CeUtils scrub as work that is FORGED and NEVER FORWARDED, and \
         `w231a_ad4ed3c_ceexec_host` is what handing it away costs: `1 arrived, 0 served, \
         1 REFUSED` → `RmInitAdapter failed! (0x25:0x65:1249)`."
    );
    // ★ And the same statement through the projection that produces the kind on the live
    // path, so a change to `ProcBoundary::channel_kind` that broke the join would land
    // here rather than only on a boot.
    assert!(!forwarding_plane_owns_ce(
        kind_of(Gpu::SYSTEM_PROC),
        true,
        false
    ));
}

/// ★★★★ **THE NEGATIVE CONTROL — the fix must not be a wholesale disarm.**
///
/// ⊘ The cheap wrong fix is to stop handing `Ce` doorbells over at all, and it would make
/// the assertion above pass, keep the guest alive, and quietly delete the forwarding plane's
/// only reachable population. This is the proposition that fails if it did: **in the very
/// same configuration, a PASSTHROUGH channel's doorbell must still be the forwarding
/// plane's.**
#[test]
fn a_passthrough_channels_ce_doorbell_is_still_the_forwarding_planes_in_the_same_configuration() {
    assert!(
        forwarding_plane_owns_ce(GuestChannelKind::Passthrough, true, false),
        "★ no passthrough channel reaches the forwarding plane. The emulated-channel term \
         was supposed to remove ONE population from the hand-off, not the hand-off — and a \
         build in which nothing falls through cannot answer whether the fall-through works, \
         while still looking healthy because the guest boots."
    );
    for pid in [ProcId(1), ProcId(2), ProcId(4242)] {
        assert_ne!(pid, Gpu::SYSTEM_PROC, "the fixture must use a USER proc");
        assert!(
            forwarding_plane_owns_ce(kind_of(pid), true, false),
            "★ {pid:?}'s channel no longer reaches the forwarding plane through the \
             production derivation of its kind."
        );
    }
}

/// ★★★ **The rule is keyed on the CHANNEL'S KIND, not on our ignorance of its address
/// space.**
///
/// ⚠ `accuracy_is_fatal_when_a_fallback_was_keyed_on_ignorance`, and §14.24 is this
/// campaign's own instance: the gate used to read `vas_pdb.is_none()`, i.e. *"a channel the
/// core cannot address is ours"* — and the day the core could address it, the executor
/// vanished. `w231a` prints `pdb=0x2efa9c000` for the system proc's channel, so **both**
/// values of `has_vas_pdb` are reachable for it and the answer must not move.
#[test]
fn the_emulated_term_does_not_depend_on_whether_we_can_address_its_vas() {
    for has_vas_pdb in [true, false] {
        for only_local in [true, false] {
            assert!(
                !forwarding_plane_owns_ce(GuestChannelKind::Emulated, has_vas_pdb, only_local),
                "★ an emulated channel changed hands at has_vas_pdb={has_vas_pdb} \
                 local_only={only_local} — the rule is about the proc's LIFETIME REGIME \
                 (§12.26), not about what we happen to know about its address space."
            );
        }
    }
}

/// ⊘ **Both historic arms are untouched, so both remain controls.** With
/// `local_ce_is_the_only_executor == true` (the default, and `KAYFABE_ISOLATES` unset)
/// nothing is ever the forwarding plane's — which is what keeps every committed `ctl` boot
/// in `traces/guest_boots/` comparable to the next one.
#[test]
fn the_shipped_arm_hands_over_nothing_at_all() {
    for kind in GuestChannelKind::ALL {
        for has_vas_pdb in [true, false] {
            assert!(
                !forwarding_plane_owns_ce(kind, has_vas_pdb, true),
                "★ a {kind} channel changed hands on the arm where the shell's CPU executor \
                 is the ONLY executor — there is nobody to hand it to."
            );
        }
    }
    // ★ And a channel the core cannot address is still nobody's to forward, on every arm.
    assert!(!forwarding_plane_owns_ce(
        GuestChannelKind::Passthrough,
        false,
        false
    ));
}

/// ★★★★★ **THE OWNER'S TWO KINDS BOTH APPEAR IN THE GATE, and the host one is what it
/// branches on.**
///
/// ⊘ This is not a restatement of the tests above. They fix the gate's *answers*; this
/// fixes *which question produces them*. §12.26's rule is about **whose channel would
/// carry the work** — the forwarding plane owns a `Ce` doorbell exactly when the host
/// channel permitted to back it is a
/// [`kayfabe_core::channel_kind::HostChannelKind::Shadow`], i.e. one inside that guest
/// process's own isolate. A gate that happened to agree while branching on something else
/// would pass every assertion above.
///
/// ★ The property asserted is the biconditional over the whole guest-kind enum, in the one
/// configuration where the other two terms are both satisfied: **owned ⟺ hosted by a
/// Shadow**. Quantified over [`GuestChannelKind::ALL`] rather than over two literals, so a
/// third kind cannot be added without landing here.
#[test]
fn the_gate_hands_over_exactly_the_kinds_a_shadow_host_channel_may_back() {
    for kind in GuestChannelKind::ALL {
        assert_eq!(
            forwarding_plane_owns_ce(kind, true, false),
            kind.hosted_by() == HostChannelKind::Shadow,
            "★ a {kind} channel is {} by the gate but its permitted host backing is a {} \
             — the gate and the owner's model disagree about which channel carries this \
             doorbell's work. A `Scratchpad` backing is OURS (§12.26: forged, never \
             forwarded); a `Shadow` backing lives in that guest process's own isolate.",
            if forwarding_plane_owns_ce(kind, true, false) {
                "handed away"
            } else {
                "kept"
            },
            kind.hosted_by(),
        );
    }
}

// =====================================================================================
// ★★★★★ THE GR ROUTE selector — the PURE half, and the DISPOSITION it feeds
// =====================================================================================
//
// ⊘ Same scope caveat as the two selector blocks above: these drive `gr_route_from` and
// `kayfabe_rt::shell_disposition` and never read the process-global. The plumbing from the
// variable to the routing decision is pinned end to end, through a real guest MMIO write,
// by `tests/gr_route_passthrough.rs` — which has to be its own binary to write the global
// safely, and says so.
//
// Why the selector exists at all: the arm it opens was CLOSED ON EVIDENCE at §16.65, and
// the evidence still stands — `docs/design/gr_doorbell_passthrough.md` §0.2-§0.3.

use kayfabe_qemu_raw::shim::{GrRouteArm, gr_route_from};
use kayfabe_rt::{DoorbellRoute, ShellDisposition, shell_disposition};

/// ⊘ **The default must leave every prior boot comparable.** `KAYFABE_GR_ROUTE` unset is
/// `Refuse`, and `Refuse` disposes a `HostGr` doorbell exactly as the `!=  CpuCe` bool did.
#[test]
fn the_default_gr_route_leaves_the_shipped_arm_byte_identical() {
    assert_eq!(gr_route_from(None), Ok(GrRouteArm::Refuse));
    assert!(
        !GrRouteArm::Refuse.gr_passthrough(),
        "★ the default arm opened the route"
    );
    assert_eq!(
        shell_disposition(DoorbellRoute::HostGr, GrRouteArm::Refuse.gr_passthrough()),
        ShellDisposition::RefuseByRoute,
        "★ on the default arm a GR doorbell must still be refused by name, or every \
         committed `ctl` boot in `traces/guest_boots/` stops being comparable to the next"
    );
}

/// ★★★★★ **The arming is the ONLY thing that opens the route**, and it opens it for
/// `HostGr` and for nothing else.
///
/// ⊘ The `Unserved` row is the one that matters: before this rung the shim's decision was
/// `route != CpuCe`, which could not have opened one of the two without opening both.
#[test]
fn the_arming_opens_hostgr_and_only_hostgr() {
    assert!(GrRouteArm::Passthrough.gr_passthrough());
    assert_eq!(
        shell_disposition(
            DoorbellRoute::HostGr,
            GrRouteArm::Passthrough.gr_passthrough()
        ),
        ShellDisposition::HandToCore,
        "★★★★★ THE RUNG: armed, a GR doorbell is handed to the core"
    );
    for armed in [false, true] {
        assert_eq!(
            shell_disposition(DoorbellRoute::Unserved, armed),
            ShellDisposition::RefuseByRoute,
            "⊘ NVENC/NVDEC must be refused on BOTH arms — the GR arming is not a general \
             `stop refusing` switch, and folding them into one bucket is exactly the defect \
             `DoorbellRoute` exists to prevent (armed={armed})"
        );
        assert_eq!(
            shell_disposition(DoorbellRoute::CpuCe, armed),
            ShellDisposition::MayServeLocally,
            "⊘ the copy-engine route is untouched by this rung on BOTH arms (armed={armed})"
        );
    }
}

/// ★★★ **The content forward follows the SAME authority as the route** — one rule, not two.
///
/// ⊘ This is the property that keeps `forward_ring`'s `Err` from turning a rung host
/// doorbell into a `Refused` report. Quantified over every `EngineKind` so a new engine
/// cannot slip in on the wrong side of it.
#[test]
fn ring_content_is_forwardable_exactly_where_the_cpu_ce_executor_owns_the_route() {
    use kayfabe_core::channel_kind::GuestChannelKind;
    // ★★★★★ **w287 — the route half of the rule is now scoped to `Emulated`.** Quantified
    // over BOTH kinds so the engine axis and the kind axis are each checked, rather than one
    // being fixed at a value that happens to make the other pass.
    for engine in kayfabe_arch::ids::EngineKind::ALL {
        assert_eq!(
            kayfabe_rt::device::ring_content_is_forwardable(engine, GuestChannelKind::Emulated),
            kayfabe_rt::device::route_of_engine(engine) == DoorbellRoute::CpuCe,
            "★ {engine:?} disagrees between the content-forward predicate and the route \
             classifier. Two tables for one question is §16.64's defect, and this one is \
             load-bearing: a GR doorbell whose ring is parsed by the copy-engine codec can \
             be reported `Refused` after its host channel was rung."
        );
    }
    // ★★★★★ **THE CUT ITSELF, as a test: no engine forwards ring CONTENT on a passthrough
    // channel.** This is the fallback the owner asked to have removed — the decode-and-re-emit
    // path that sat beside every green run and made *"passthrough worked"* and *"the fallback
    // caught it"* indistinguishable. If this loop ever goes green-by-exception, the rung it
    // graded was measuring the wrong mechanism.
    for engine in kayfabe_arch::ids::EngineKind::ALL {
        assert!(
            !kayfabe_rt::device::ring_content_is_forwardable(engine, GuestChannelKind::Passthrough),
            "★★★ {engine:?}: a PASSTHROUGH channel's ring is the GUEST's. `ce_copy` drives a \
             channel and adoption means the guest drives it — both cannot hold, and w283d \
             measured one CE doorbell doing both on two different host channels."
        );
    }
    // Non-vacuity: the predicate is not constant in either direction.
    assert!(kayfabe_rt::device::ring_content_is_forwardable(
        kayfabe_arch::ids::EngineKind::Ce,
        GuestChannelKind::Emulated
    ));
    assert!(!kayfabe_rt::device::ring_content_is_forwardable(
        kayfabe_arch::ids::EngineKind::GrCompute,
        GuestChannelKind::Emulated
    ));
}

/// Every arm round-trips through its own spelling, with the same non-vacuity check the
/// executor block uses.
#[test]
fn every_gr_route_arm_round_trips_through_its_own_spelling() {
    for arm in GrRouteArm::ALL {
        assert_eq!(
            gr_route_from(Some(arm.as_str())),
            Ok(arm),
            "★ {arm:?} does not parse from the name it prints"
        );
    }
    let mut names: Vec<&str> = GrRouteArm::ALL.iter().map(|a| a.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), GrRouteArm::ALL.len(), "two arms share a name");
}

/// ⊘ A near-miss REFUSES TO REALIZE rather than defaulting quietly.
///
/// ★ The stakes here are higher than for the executor selector, and that is why `on` and
/// `1` are in the list: the two arms of this experiment differ in **one routing decision**,
/// so a disarmed evidence run and its control produce identical logs — no new lines, and a
/// full census of `Route::NotACopyEngineChannel` — which is also exactly what a *correct*
/// control produces.
#[test]
fn a_value_that_is_not_a_gr_route_arm_refuses_rather_than_defaulting() {
    for bad in [
        "",
        "on",
        "1",
        "true",
        "yes",
        "Refuse",
        "REFUSE",
        "refuse ",
        " refuse",
        "pass",
        "passthru",
        "hostgr",
        "\u{fffd}invalid",
    ] {
        let (status, why) = gr_route_from(Some(bad))
            .expect_err(&format!("★ {bad:?} was ACCEPTED as a GR route arm"));
        assert_eq!(
            status.code(),
            kayfabe_qemu_raw::shim::Status::Unsupported.code()
        );
        for arm in GrRouteArm::ALL {
            assert!(
                why.contains(arm.as_str()),
                "★ the refusal for {bad:?} does not name `{}`; message was: {why}",
                arm.as_str()
            );
        }
    }
}

// ---------------------------------------------------------------------------------------
// ★★★★★ LEG A — `KAYFABE_GUEST_RING`, the arm that gives the framebuffer join a SECOND
// SOURCE: the channel's own GPFIFO ring, walked and joined at the engine-object latch.
//
// ⊘ Why it is a SECOND selector and not a third arm of `KAYFABE_GR_ROUTE`: they arm
// different legs of the same stool and a boot must be able to run either without the other.
// `w260` measured the supply side moving while the execution side did not — folding the two
// into one word would make that measurement unexpressible.
// ---------------------------------------------------------------------------------------

use kayfabe_qemu_raw::shim::{GuestRingArm, guest_ring_from};

/// ⊘ **The default must leave every prior boot comparable**, and here that is sharper than
/// for the route: the ring source runs on the REGISTER-WRITE path, which every boot takes
/// millions of times. A default that armed would change the shape of every log ever taken.
#[test]
fn the_default_guest_ring_arm_leaves_the_shipped_path_byte_identical() {
    assert_eq!(guest_ring_from(None), Ok(GuestRingArm::Off));
    assert!(
        !GuestRingArm::Off.adopts_ring(),
        "★ the default arm presented the channel's ring to the join"
    );
}

/// ★★★★★ THE RUNG: `ring` is the only arm that adopts, and it says so through one predicate
/// that both the shim and this test read — never through `== Ring` spelled at a call site.
#[test]
fn only_the_ring_arm_adopts_and_the_predicate_is_the_join() {
    assert!(GuestRingArm::Ring.adopts_ring());
    let adopting: Vec<GuestRingArm> = GuestRingArm::ALL
        .into_iter()
        .filter(|a| a.adopts_ring())
        .collect();
    assert_eq!(
        adopting,
        vec![GuestRingArm::Ring],
        "★ exactly one arm may adopt the guest's ring; a second one is a second experiment \
         wearing one flag's name"
    );
}

/// Every arm round-trips through its own spelling, and no two arms share a name.
#[test]
fn every_guest_ring_arm_round_trips_through_its_own_spelling() {
    for arm in GuestRingArm::ALL {
        assert_eq!(
            guest_ring_from(Some(arm.as_str())),
            Ok(arm),
            "★ {arm:?} does not parse from the name it prints"
        );
    }
    let mut names: Vec<&str> = GuestRingArm::ALL.iter().map(|a| a.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(
        names.len(),
        GuestRingArm::ALL.len(),
        "two arms share a name"
    );
}

/// ⊘ A near-miss REFUSES TO REALIZE rather than defaulting quietly.
///
/// ★ The stakes are the same as the route's and the failure is quieter: a disarmed evidence
/// run prints **no `GR-RING-JOIN` line at all**, which is exactly what a correct control
/// prints. Absence cannot distinguish them, so the spelling must.
#[test]
fn a_value_that_is_not_a_guest_ring_arm_refuses_rather_than_defaulting() {
    for bad in [
        "",
        "on",
        "1",
        "true",
        "yes",
        "Off",
        "OFF",
        "off ",
        " off",
        "Ring",
        "rings",
        "adopt",
        "guest",
        "\u{fffd}invalid",
    ] {
        let (status, why) = guest_ring_from(Some(bad))
            .expect_err(&format!("★ {bad:?} was ACCEPTED as a guest-ring arm"));
        assert_eq!(
            status.code(),
            kayfabe_qemu_raw::shim::Status::Unsupported.code()
        );
        for arm in GuestRingArm::ALL {
            assert!(
                why.contains(arm.as_str()),
                "★ the refusal for {bad:?} does not name `{}`; message was: {why}",
                arm.as_str()
            );
        }
    }
}

/// ★★★ **THE TWO LEGS ARE INDEPENDENT, and that is asserted rather than assumed.**
///
/// ⊘ Nothing in the shim may make one arm imply the other. A boot must be able to run
/// `KAYFABE_GUEST_RING=ring` with `KAYFABE_GR_ROUTE=refuse` (the supply side alone, which is
/// exactly the `w260` shape) and `passthrough` with `off` (the transport alone, which is
/// `b734995`'s shape). All four cells are reachable, and the product below is the statement.
#[test]
fn the_ring_arm_and_the_route_arm_are_four_independent_cells() {
    let mut cells = Vec::new();
    for ring in GuestRingArm::ALL {
        for route in GrRouteArm::ALL {
            cells.push((ring.adopts_ring(), route.gr_passthrough()));
        }
    }
    cells.sort_unstable();
    cells.dedup();
    assert_eq!(
        cells.len(),
        4,
        "★ the two selectors do not span four cells — one of them constrains the other, and \
         the supply side and the transport would stop being separately measurable"
    );
}

/// ★★★ **w290's publication arm is three-valued and NEVER defaulted.**
///
/// A typo that silently disarmed the publication would make an evidence run and its own
/// control indistinguishable — the reason every arm in this shim refuses an unknown value
/// instead of falling back. ⊘ `on`/`1` are deliberately rejected: this is a three-arm
/// experiment, and a boolean cannot express the `assert` control that censuses without
/// issuing a host verb.
#[test]
fn the_vas_publish_arm_is_three_valued_and_never_defaulted() {
    use kayfabe_qemu_raw::shim::{VasPublishArm, vas_publish_from};
    assert_eq!(vas_publish_from(None), Ok(VasPublishArm::Off));
    assert_eq!(vas_publish_from(Some("off")), Ok(VasPublishArm::Off));
    assert_eq!(vas_publish_from(Some("assert")), Ok(VasPublishArm::Assert));
    assert_eq!(
        vas_publish_from(Some("publish")),
        Ok(VasPublishArm::Publish)
    );
    assert_eq!(
        vas_publish_from(Some("pinrate")),
        Ok(VasPublishArm::PinRate)
    );
    assert_eq!(vas_publish_from(Some("both")), Ok(VasPublishArm::Both));
    assert_eq!(vas_publish_from(Some("drain")), Ok(VasPublishArm::Drain));
    for bad in ["on", "1", "true", "yes", "", "Publish"] {
        assert!(
            vas_publish_from(Some(bad)).is_err(),
            "⊘ `{bad}` must be REFUSED, not silently disarmed"
        );
    }
    // ★ The two predicates are DIFFERENT questions, and `assert` is the row that proves it:
    // the census runs and no host verb is issued.
    assert!(!VasPublishArm::Off.observes() && !VasPublishArm::Off.publishes());
    assert!(VasPublishArm::Assert.observes() && !VasPublishArm::Assert.publishes());
    assert!(VasPublishArm::Publish.observes() && VasPublishArm::Publish.publishes());
    // ★★★ w291: `pinrate` OBSERVES but does NOT publish — it measures a different chain over
    // a different population. ⊘ If `publishes()` ever returned true for it, one line's
    // `published=` would count two mechanisms and the count could not see the substitution.
    assert!(VasPublishArm::PinRate.observes() && !VasPublishArm::PinRate.publishes());
    assert!(VasPublishArm::PinRate.measures_pin_rate());
    assert!(!VasPublishArm::Publish.measures_pin_rate() && !VasPublishArm::Off.measures_pin_rate());
    // ★★★ w291 `both` is BOTH halves; ★★★★★ w292 `drain` is `both` plus ONE scoped change.
    assert!(VasPublishArm::Both.publishes() && VasPublishArm::Both.measures_pin_rate());
    assert!(VasPublishArm::Drain.publishes() && VasPublishArm::Drain.measures_pin_rate());
    // ★★★★★ **THE PREDICATE THAT KEEPS THE BUDGET SCOPED, ASSERTED RATHER THAN COMMENTED.**
    // If `drains_doorbelled_vas()` were ever true of `both`, this rung's control would be
    // running the rung's own change and the two boots could not be compared. And it must be
    // FALSE of every other arm, or the brief's *"⊘⊘ DO NOT RAISE THE BUDGET BLINDLY ACROSS
    // THE BOARD"* would hold only by convention.
    assert!(VasPublishArm::Drain.drains_doorbelled_vas());
    for other in [
        VasPublishArm::Off,
        VasPublishArm::Assert,
        VasPublishArm::Publish,
        VasPublishArm::PinRate,
        VasPublishArm::Both,
    ] {
        assert!(
            !other.drains_doorbelled_vas(),
            "⊘ `{}` must NOT drain: only `drain` may, or the control runs the change",
            other.as_str()
        );
    }
}

/// ★★★★★ **w318 — THE DIRTY GATE IS OFF BY DEFAULT AND A TYPO CANNOT ARM IT.**
///
/// Every other arm in this shim is off-by-default because arming it would make an instrument
/// fire unasked. This one is the more dangerous direction: arming it makes a
/// **correctness-relevant pass stop running** on a clean doorbell, and `VAS_PUBLISH` ablated
/// **red** — a publication skipped that the engine then needs is a GPU fault, not a slow path.
///
/// ⇒ ⊘ **Absent is `off`, an unknown value is an ERROR, and the runtime selector reads any
/// error as `off`.** The safe direction for a flag that removes work is always to do the work.
#[test]
fn the_w318_dirty_gate_is_off_by_default_and_refuses_an_unknown_value() {
    use kayfabe_qemu_raw::shim::dirty_gate_from;
    assert_eq!(dirty_gate_from(None), Ok(false), "absent is OFF");
    assert_eq!(dirty_gate_from(Some("off")), Ok(false));
    assert_eq!(dirty_gate_from(Some("on")), Ok(true));
    for bad in ["1", "true", "yes", "", "On", "ON", "enabled"] {
        assert!(
            dirty_gate_from(Some(bad)).is_err(),
            "⊘ `{bad}` must be REFUSED — a mistyped value that armed this gate would skip a \
             publication nobody decided to skip"
        );
    }
}
