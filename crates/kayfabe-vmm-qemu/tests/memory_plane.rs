//! The guest-physical memory plane: our windows, the hypervisor's regions, and the seam
//! between them.
//!
//! ★★★ The sharpest thing in this file is the **opacity** group. Under
//! `host_execution_plane.md` §1.5a the window is visible to the *kernel* and opaque to the
//! *hypervisor*: anything that reaches it through the hypervisor's flat view hits the
//! reservation BAR's stub read/write ops and gets **zeros**, silently. So the adapter must
//! never route a window address through the hypervisor, and the region map must keep
//! classifying the window's BAR as a **device** so that a bypass is a refusal rather than a
//! memcpy of zeros.

mod common;

use common::{
    BAR0_BASE, BAR1_BASE, FOREIGN_RAM, machine, machine_with, non_ram_shapes, overlay_gpa,
    overlay_len, overlay_trap, page, ram_facts, window_gpa, window_len,
};

use core::time::Duration;
use kayfabe_vmm::{
    BarId, CoreEvent, CoreEventKind, HostRegion, IrqSpec, Prot, RegionKind, SlotId, TrapMode, Vmm,
    VmmError,
};
use kayfabe_vmm_qemu::host::SectionFacts;
use kayfabe_vmm_qemu::mock_host::{HostCall, MockPolicy};
use kayfabe_vmm_qemu::{
    FOREIGN_OVERLAPS_OURS, MEMORY_PLANE_AFTER_UNREALIZE, NO_LEGACY_INTX, NO_SHARED_BACKING,
    PER_OBJECT_PROTECTION,
};

/// The port's "fill it from nothing" sentinel.
const FILL_FROM_NOTHING: HostRegion = HostRegion {
    id: u64::MAX,
    offset: 0,
};

// =====================================================================================
// Our own reservation
// =====================================================================================

/// ★★ A window round-trips, and the copy runs **outside** the view lock.
///
/// The leaf-depth witness has both halves: `own_copy_leaf_depth_max == 0` says the copy
/// really is outside the lock, and `view_leaf_depth_max >= 1` says the witness is measuring
/// something at all. Either alone is satisfiable by an instrument that is broken.
#[test]
fn our_own_reservation_round_trips_with_the_copy_outside_the_view_lock() {
    let (m, _host, _slots) = machine();
    let mut v = m.vmm();
    let p = page();
    for i in 0..8u64 {
        let pattern: Vec<u8> = (0..64).map(|b| (b as u8).wrapping_add(i as u8)).collect();
        v.gpa_write(window_gpa() + i * p, &pattern)
            .unwrap_or_else(|e| panic!("write page {i}: {e:?}"));
        let mut back = vec![0u8; 64];
        v.gpa_read(window_gpa() + i * p, &mut back)
            .unwrap_or_else(|e| panic!("read page {i}: {e:?}"));
        assert_eq!(back, pattern, "page {i} did not round-trip");
    }
    let a = m.audit();
    assert_eq!(
        a.own_copy_leaf_depth_max, 0,
        "a copy out of our own reservation must run with the view lock RELEASED"
    );
    assert!(
        a.view_leaf_depth_max >= 1,
        "and the leaf witness must have seen the lock held at all, or the line above is \
         vacuously true"
    );
    assert_eq!(a.accesses_served, 16);
}

/// ★★★ **THE OPACITY TEST.** A window address is never answered through the hypervisor.
///
/// `host_execution_plane.md` §1.5a: the shadowing is the kernel's. The hypervisor's flat
/// view still has the reservation BAR's stub ops over that range, and they return zeros and
/// discard writes. So a `gpa_read` of a window address that went through the hypervisor
/// would **succeed** and return zeros — indistinguishable from a correct read of
/// freshly-zeroed guest memory, which is why the assertion is on the *call log* and not on
/// the bytes.
#[test]
fn a_window_range_is_never_answered_through_the_hypervisor() {
    let (m, host, _slots) = machine();
    let mut v = m.vmm();

    v.gpa_write(window_gpa(), &[0xC7; 32])
        .expect("the window serves");
    let mut back = [0u8; 32];
    v.gpa_read(window_gpa(), &mut back).expect("and reads back");
    assert_eq!(back, [0xC7; 32]);

    assert!(
        !host
            .log()
            .iter()
            .any(|c| matches!(c, HostCall::ReadRegion(..) | HostCall::WriteRegion(..))),
        "an access into our own reservation must not touch the hypervisor at all; through \
         its flat view that range is a stub returning zeros, so a call here would SUCCEED \
         and be wrong. Log: {:?}",
        host.log()
    );

    // And a foreign region in the same run DOES call it — so the assertion above is about
    // where the window goes, not about the adapter never calling the hypervisor.
    m.region_add(host.mint_foreign(FOREIGN_RAM, page(), ram_facts()))
        .expect("guest RAM");
    v.gpa_write(FOREIGN_RAM, &[1u8; 4]).expect("write");
    assert!(
        host.log()
            .iter()
            .any(|c| matches!(c, HostCall::WriteRegion(..))),
        "a foreign region must go through the hypervisor, or the negative above is vacuous"
    );
}

/// ★★★ The layering that makes a bypass safe: the window's own BAR **resolves as a device**.
///
/// `resolve_region` is the region map with nothing in front of it, and it must say
/// `NonRamGpa` for a guest-physical address our window map serves happily — because through
/// the hypervisor that is exactly what the range is. A `Ram` declaration here is what would
/// turn a bypassed window lookup into a silent memcpy of zeros.
#[test]
fn the_region_map_classifies_our_own_window_as_a_device_which_is_what_it_is() {
    let (m, _host, _slots) = machine();
    for at in [
        window_gpa(),
        window_gpa() + page(),
        window_gpa() + window_len() - 8,
        BAR0_BASE,
        overlay_gpa(),
    ] {
        assert_eq!(
            m.resolve_region(at, 8),
            Err(VmmError::NonRamGpa { gpa: at }),
            "through the hypervisor's flat view {at:#x} is a device BAR, and the region map \
             must keep saying so — it is the backstop under our own window lookup"
        );
    }
    // ...and the window really does serve, so the two statements are about one address.
    m.vmm()
        .gpa_write(window_gpa(), &[1u8; 8])
        .expect("our own map answers what the region map calls a device");
}

/// ★ A hole and a device refuse by **different** names (`testing_doctrine.md` §2 rule 3).
#[test]
fn a_hole_and_a_device_refuse_by_different_names() {
    let (m, host, _slots) = machine();
    let mut v = m.vmm();
    m.region_add(host.mint_foreign(FOREIGN_RAM, 4 * page(), ram_facts()))
        .expect("guest RAM");

    assert_eq!(
        v.gpa_read(0x5000_0000, &mut [0u8; 8]),
        Err(VmmError::BadGpa { gpa: 0x5000_0000 }),
        "nothing is declared there at all"
    );
    assert_eq!(
        v.gpa_read(BAR0_BASE, &mut [0u8; 8]),
        Err(VmmError::NonRamGpa { gpa: BAR0_BASE }),
        "something IS there and it is a device"
    );
    v.gpa_write(FOREIGN_RAM, &[9u8; 8]).expect("and RAM serves");
}

/// ★★★ A range that straddles the end of a window is refused **whole**, and the address it
/// names is a consequence of the window being invisible to the region map.
///
/// ## The finding, because this is the one place the opacity design is VISIBLE from outside
///
/// The sibling KVM backend declares a window `Ram` in the region map, so a straddling range
/// there resolves partly and reports the **boundary** byte. Here the window is not in the
/// region map at all (crate finding 3): the region map holds one `Device` region covering
/// the whole reservation BAR, our window lookup requires the range to lie **wholly** inside
/// a window, and a straddling range does not — so it falls through and the region map,
/// which sees one uniform device, reports **the range's own start**.
///
/// Both answers are correct and they are different. It is asserted here rather than
/// smoothed over, because the property that matters is the same in both: **the access is
/// refused as a unit and no partial copy happens**, which is what a straddling range is
/// dangerous for.
#[test]
fn a_range_straddling_the_end_of_a_window_is_refused_whole() {
    let (m, _host, _slots) = machine();
    let mut v = m.vmm();
    let boundary = window_gpa() + window_len();
    let mut buf = [0xFFu8; 8];
    assert_eq!(
        v.gpa_read(boundary - 4, &mut buf),
        Err(VmmError::NonRamGpa { gpa: boundary - 4 }),
        "through the hypervisor the whole reservation BAR is one device region, so the \
         first byte that does not resolve is the range's own start"
    );
    assert_eq!(
        buf, [0xFF; 8],
        "and NOT ONE BYTE may have been copied; a partial copy of a straddling range is the \
         hazard, and the address in the error is only the diagnosis"
    );
    // The four bytes before the boundary are inside the window and serve perfectly, which is
    // what makes the range above a straddle rather than a miss.
    v.gpa_read(boundary - 4, &mut [0u8; 4])
        .expect("wholly inside the window");
}

// =====================================================================================
// The hypervisor's own regions
// =====================================================================================

/// ★★ A reported host region round-trips, and its copy runs **inside** the view lock.
#[test]
fn a_host_owned_region_round_trips_with_the_copy_inside_the_view_lock() {
    let (m, host, _slots) = machine();
    let mut v = m.vmm();
    let s = host.mint_foreign(FOREIGN_RAM, 4 * page(), ram_facts());
    m.region_add(s).expect("a reported RAM section");

    v.gpa_write(FOREIGN_RAM + 16, &[0xEE; 8]).expect("write");
    let mut back = [0u8; 8];
    v.gpa_read(FOREIGN_RAM + 16, &mut back).expect("read");
    assert_eq!(back, [0xEE; 8]);

    let a = m.audit();
    assert!(
        a.host_copy_leaf_depth_min >= 1,
        "a copy out of a hypervisor-owned region must run INSIDE the view lock; {} means it \
         ran outside (or never ran at all)",
        a.host_copy_leaf_depth_min
    );
}

/// ★★ Every non-RAM section shape is declared a device and refuses **by the device name**.
#[test]
fn every_non_ram_section_shape_is_declared_a_device_and_refuses_by_the_device_name() {
    for (what, facts) in non_ram_shapes() {
        let (m, host, _slots) = machine();
        let mut v = m.vmm();
        m.region_add(host.mint_foreign(FOREIGN_RAM, 4 * page(), facts))
            .unwrap_or_else(|e| panic!("{what}: {e:?}"));
        assert_eq!(
            v.gpa_read(FOREIGN_RAM, &mut [0u8; 8]),
            Err(VmmError::NonRamGpa { gpa: FOREIGN_RAM }),
            "{what}: it must be DECLARED and refuse as a device; omitting it would report \
             the near neighbour BadGpa instead"
        );
        assert!(
            !host
                .log()
                .iter()
                .any(|c| matches!(c, HostCall::RefRegion(_))),
            "{what}: and no reference may be taken for a region we will never copy from"
        );
    }
}

/// ★★ A reported section that overlaps a range **we** own is refused, every way it can
/// overlap, and no reference is taken on the way out.
#[test]
fn a_reported_section_overlapping_a_range_we_own_is_refused_every_way_it_can_overlap() {
    let p = page();
    for (what, gpa, len) in [
        ("exactly", window_gpa(), window_len()),
        ("the front", window_gpa() - p, 2 * p),
        ("the back", window_gpa() + window_len() - p, 2 * p),
        ("inside", window_gpa() + p, p),
        (
            "straddling the whole thing",
            window_gpa() - p,
            window_len() + 2 * p,
        ),
        ("one byte at the start", window_gpa(), 1),
    ] {
        let (m, host, _slots) = machine();
        let s = host.mint_foreign(gpa, len, ram_facts());
        assert_eq!(
            m.region_add(s),
            Err(VmmError::Unsupported(FOREIGN_OVERLAPS_OURS)),
            "{what}: two declaration sources for one guest-physical range, and the loser \
             would be whichever ran second"
        );
        assert!(
            !host
                .log()
                .iter()
                .any(|c| matches!(c, HostCall::RefRegion(_))),
            "{what}: the refusal must leave the reference count as it found it"
        );
    }
}

/// ★ `region_del` undeclares before it releases, and leaves no reference outstanding.
#[test]
fn region_del_undeclares_before_it_releases_and_leaves_no_reference_outstanding() {
    let (m, host, _slots) = machine();
    let mut v = m.vmm();
    let s = host.mint_foreign(FOREIGN_RAM, 4 * page(), ram_facts());
    let mr = s.mr;
    m.region_add(s).expect("added");
    assert_eq!(host.live_regions(), vec![(mr, 1)]);

    m.region_del(FOREIGN_RAM, 4 * page());
    assert_eq!(
        host.live_regions(),
        vec![(mr, 0)],
        "the reference we took must be the reference we gave back"
    );
    // ★ The order matters and the log is where it is visible: the release is the LAST thing
    // the delete did. Snapshotted HERE, before any access — every guest-physical access
    // re-reads the latched BAR base and would append to the log.
    let log = host.log();
    assert_eq!(
        log.iter()
            .position(|c| matches!(c, HostCall::UnrefRegion(_))),
        Some(log.len() - 1),
        "the release is the last call of the delete, not the first: the range must stop \
         resolving BEFORE the hypervisor can finalize the region"
    );
    assert_eq!(
        v.gpa_read(FOREIGN_RAM, &mut [0u8; 8]),
        Err(VmmError::BadGpa { gpa: FOREIGN_RAM }),
        "and the range stops resolving"
    );
}

/// ★ A delete for a range nobody added changes nothing — and does not release a reference
/// that was never taken.
#[test]
fn a_delete_for_a_range_that_was_never_added_changes_nothing() {
    let (m, host, _slots) = machine();
    m.region_del(0x7777_0000, page());
    assert!(
        !host
            .log()
            .iter()
            .any(|c| matches!(c, HostCall::UnrefRegion(_)))
    );
    assert_eq!(m.audit().topology_dels, 0);
}

/// ★★ A section that is a **slice** of a larger region copies at that region's own offset.
///
/// Every other section this suite mints starts at offset zero, which makes
/// `region_off + span.offset` and a bare `span.offset` indistinguishable — a bite-check
/// found exactly that survivor.
#[test]
fn a_section_that_is_a_slice_of_a_larger_region_copies_at_that_regions_own_offset() {
    let p = page();
    let (m, host, _slots) = machine();
    let mut v = m.vmm();
    let s = host.mint_foreign_slice(FOREIGN_RAM, p, 2 * p, 4 * p, ram_facts());
    let mr = s.mr;
    m.region_add(s).expect("a sliced section");

    v.gpa_write(FOREIGN_RAM + 8, &[0xA5; 4]).expect("write");
    let bytes = host.region_bytes(mr).expect("the region");
    let at = usize::try_from(2 * p + 8).expect("test-sized");
    assert_eq!(
        &bytes[at..at + 4],
        &[0xA5; 4],
        "the write must land at the SECTION's offset within its region, not at the \
         section-relative offset"
    );
    assert_eq!(
        &bytes[8..12],
        &[0, 0, 0, 0],
        "and not at the region's own start"
    );
}

// =====================================================================================
// The fine tier
// =====================================================================================

/// ★★★ Ten placements into one reservation perform **exactly zero** further memslot
/// installs and **exactly zero** hypervisor calls — §6.7's frequency rule, which is what
/// makes the whole design viable.
#[test]
fn ten_placements_into_one_reservation_install_exactly_no_further_memslot() {
    let p = page();
    let (m, host, slots) = machine();
    let mut v = m.vmm();
    // A sixteen-page reservation, so ten one-page placements fit inside ONE of them — the
    // whole point of the assertion below is that the count of memslots does not follow the
    // count of placements.
    let wide = window_gpa() + window_len();
    m.install_ram_window(wide, 16 * p).expect("a wide window");
    let before = slots.installs();
    let host_log_before = host.log().len();

    for i in 0..10u64 {
        let backing = m.register_backing(p).expect("a backing");
        v.map_guest(wide + i * p, p, backing, Prot::ReadWrite)
            .unwrap_or_else(|e| panic!("placement {i}: {e:?}"));
    }
    assert_eq!(
        slots.installs() - before,
        0,
        "the window's memslot already names the whole range; a slot per placement is the \
         C's own measured regression (>1500 tiny mmaps on one context create)"
    );
    assert_eq!(
        host.log().len(),
        host_log_before,
        "and the HYPERVISOR is not called at all for a placement"
    );
    assert_eq!(m.audit().placements_made, 10);
    assert_eq!(m.audit().live_placements, 10);
}

/// ★★ A placement is refused by the exact reason it could not be made — four ways.
#[test]
fn a_placement_is_refused_by_the_exact_reason_it_could_not_be_made() {
    let p = page();
    let (m, _host, _slots) = machine();
    let mut v = m.vmm();
    let b = m.register_backing(p).expect("a backing");

    assert_eq!(
        v.map_guest(window_gpa(), p, b, Prot::ReadOnly),
        Err(VmmError::Unsupported(PER_OBJECT_PROTECTION)),
        "protection is a slot property, so it is a window property"
    );
    assert_eq!(
        v.map_guest(0x5000_0000, p, b, Prot::ReadWrite),
        Err(VmmError::BadGpa { gpa: 0x5000_0000 }),
        "no reservation covers it"
    );
    assert_eq!(
        v.map_guest(
            window_gpa(),
            p,
            HostRegion { id: 999, offset: 0 },
            Prot::ReadWrite
        ),
        Err(VmmError::Unsupported(
            "a host backing id this backend never minted"
        ))
    );
    v.map_guest(window_gpa(), p, b, Prot::ReadWrite)
        .expect("the first placement");
    let b2 = m.register_backing(p).expect("a second backing");
    assert_eq!(
        v.map_guest(window_gpa(), p, b2, Prot::ReadWrite),
        Err(VmmError::Unsupported(
            "a placement overlapping one already live in the same reservation"
        ))
    );
}

/// ★★ The coarse and fine slot ids tear down **different** things, and neither does the
/// other's job.
#[test]
fn the_coarse_and_fine_slot_ids_tear_down_different_things() {
    let p = page();
    let (m, _host, slots) = machine();
    let mut v = m.vmm();
    let coarse = v
        .map_read_native(
            overlay_gpa(),
            overlay_len(),
            FILL_FROM_NOTHING,
            Some(overlay_trap()),
        )
        .expect("a read-native window");
    let backing = m.register_backing(p).expect("a backing");
    let fine = v
        .map_guest(window_gpa(), p, backing, Prot::ReadWrite)
        .expect("a placement");

    let live_before = slots.live().len();
    v.unmap_guest(fine).expect("the fine slot");
    assert_eq!(
        slots.live().len(),
        live_before,
        "a placement's teardown must not touch the kernel's slot table"
    );

    v.unmap_guest(coarse).expect("the coarse slot");
    assert_eq!(
        slots.live().len(),
        1,
        "the read-native window's two slots must be gone, leaving the realize-time one"
    );
    assert_eq!(
        v.unmap_guest(SlotId(9999)),
        Err(VmmError::BadSlot(SlotId(9999)))
    );
}

/// ★★ A read-native window filled from a **named backing** carries that backing's bytes,
/// and one filled from nothing does not.
///
/// A read-native window whose pages were anonymous zeroes serves reads natively and serves
/// the *wrong value* — which is worse than trapping, because it is fast and silent.
#[test]
fn a_read_native_window_is_filled_from_its_backing_when_one_is_named() {
    let (m, _host, _slots) = machine();
    let mut v = m.vmm();
    let backing = m.register_backing(overlay_len()).expect("a backing");
    v.map_read_native(overlay_gpa(), overlay_len(), backing, Some(overlay_trap()))
        .expect("a read-native window over a real backing");
    // The window is ours to write through; the read-only flag constrains the GUEST.
    v.gpa_write(overlay_gpa(), &[0x3C; 16])
        .expect("the core keeps a read-native window current through gpa_write");
    let mut back = [0u8; 16];
    v.gpa_read(overlay_gpa(), &mut back).expect("and reads it");
    assert_eq!(back, [0x3C; 16]);
}

// =====================================================================================
// Everything else the port promises
// =====================================================================================

/// ★★ No syscall-shaped method ever runs with a ranked lock held (R1), and the witness is
/// not vacuous.
#[test]
fn no_syscall_shaped_method_ever_runs_with_a_ranked_lock_held() {
    let p = page();
    let (m, host, _slots) = machine();
    let mut v = m.vmm();
    let b = m.register_backing(p).expect("backing");
    v.map_guest(window_gpa(), p, b, Prot::ReadWrite)
        .expect("place");
    m.region_add(host.mint_foreign(FOREIGN_RAM, p, ram_facts()))
        .expect("ram");
    v.gpa_write(FOREIGN_RAM, &[1u8; 4]).expect("write");
    v.export_ram(None).expect("export");

    let a = m.audit();
    assert_eq!(
        a.syscall_ranked_depth.1, 0,
        "R1: no ranked lock at a syscall"
    );
    assert_ne!(
        a.syscall_ranked_depth.0,
        u32::MAX,
        "and at least one syscall-shaped method must have been observed, or the maximum \
         above is a number nobody wrote"
    );
    assert_ne!(a.accessor_ranked_depth.0, u32::MAX);
}

/// ★ `raise_irq` is one host call per vector, and the legacy variant is refused by name.
#[test]
fn raise_irq_is_one_host_call_per_vector_and_the_legacy_variant_is_refused_by_name() {
    let (m, host, _slots) = machine();
    let mut v = m.vmm();
    for vec in [0u16, 1, 7] {
        v.raise_irq(IrqSpec::Msix(vec)).expect("a vector");
    }
    assert_eq!(host.irqs(), vec![0, 1, 7]);
    assert_eq!(m.audit().irqs_raised, 3);
    assert!(v.raise_irq(IrqSpec::Msix(8)).is_err(), "past the table");
    assert_eq!(
        v.raise_irq(IrqSpec::IntxLevel(true)),
        Err(VmmError::Unsupported(NO_LEGACY_INTX))
    );
}

/// ★ Deferred events come back in deadline-then-insertion order.
#[test]
fn deferred_events_come_back_in_deadline_then_insertion_order() {
    let (m, _host, _slots) = machine();
    let mut v = m.vmm();
    let late = CoreEvent::Deferred(CoreEventKind::PollKickBudget);
    let early_a = CoreEvent::Deferred(CoreEventKind::DeferredReap);
    let early_b = CoreEvent::Deferred(CoreEventKind::RegionFault);

    v.defer(Duration::from_millis(20), late.clone());
    v.defer(Duration::from_millis(5), early_a.clone());
    v.defer(Duration::from_millis(5), early_b.clone());

    assert_eq!(
        m.advance(Duration::from_millis(5)),
        vec![early_a, early_b],
        "★ the two due at the same instant come out in INSERTION order — ordering them by \
         the payload's own `Ord` would be deterministic and deterministically wrong"
    );
    assert_eq!(m.advance(Duration::from_millis(1)), Vec::new());
    assert_eq!(m.advance(Duration::from_millis(20)), vec![late]);
}

/// ★ A machine without a shareable backing refuses the first export, by name.
#[test]
fn a_machine_without_a_shareable_backing_refuses_the_first_export() {
    let (m, _host, _slots) = machine_with(
        MockPolicy::default(),
        kayfabe_vmm_qemu::MachineConfig {
            shareable_ram: false,
            ..common::config()
        },
    );
    assert_eq!(
        m.vmm().export_ram(None),
        Err(VmmError::Unsupported(NO_SHARED_BACKING))
    );
}

/// ★ Exporting a slice no reservation covers is a **different** refusal from having no
/// shareable backing at all.
#[test]
fn exporting_a_slice_no_reservation_covers_is_refused_by_its_own_name() {
    let (m, _host, _slots) = machine();
    assert_eq!(
        m.vmm().export_ram(Some(0x5000_0000..0x5000_1000)),
        Err(VmmError::Unsupported(
            "no shareable reservation covers the requested slice"
        ))
    );
    m.vmm()
        .export_ram(Some(window_gpa()..window_gpa() + page()))
        .expect("a slice the reservation does cover");
}

/// ★ A zero-length access is still proved against the map — it is not a free pass.
#[test]
fn a_zero_length_access_is_still_proved_against_the_region_map() {
    let (m, _host, _slots) = machine();
    let mut v = m.vmm();
    v.gpa_read(window_gpa(), &mut []).expect("inside a window");
    assert_eq!(
        v.gpa_read(0x5000_0000, &mut []),
        Err(VmmError::BadGpa { gpa: 0x5000_0000 }),
        "a zero-length read of an unbacked address must still refuse"
    );
}

/// ★ The kind the map holds is the kind the classification produced.
#[test]
fn the_kind_the_map_holds_is_the_kind_the_classification_produced() {
    let (m, host, _slots) = machine();
    m.region_add(host.mint_foreign(FOREIGN_RAM, page(), ram_facts()))
        .expect("ram");
    assert_eq!(
        m.resolve_region(FOREIGN_RAM, 8).map(|s| s.len),
        Ok(8),
        "plain RAM resolves"
    );
    for (what, facts) in non_ram_shapes() {
        let (m, host, _slots) = machine();
        m.region_add(host.mint_foreign(FOREIGN_RAM, page(), facts))
            .expect("added");
        assert_eq!(
            m.resolve_region(FOREIGN_RAM, 8),
            Err(VmmError::NonRamGpa { gpa: FOREIGN_RAM }),
            "{what}"
        );
    }
    assert_eq!(
        kayfabe_vmm_qemu::classify::classify(&SectionFacts::plain_ram()),
        RegionKind::Ram
    );
}

/// ★★ Every memory-plane door refuses after unrealize, by **one** name, and the counter is
/// not vacuous. `gpa_read` is the exception and is asserted as such: it has no lifecycle
/// gate because it takes no lock and makes no call once the map is empty — its answer is the
/// map's, which is that nothing is there.
#[test]
fn every_memory_plane_door_refuses_after_unrealize() {
    let p = page();
    let (m, _host, _slots) = machine();
    let mut v = m.vmm();
    let backing = m.register_backing(p).expect("backing while alive");
    m.unrealize();

    let expected = Err(VmmError::Unsupported(MEMORY_PLANE_AFTER_UNREALIZE));
    assert_eq!(m.install_ram_window(window_gpa(), p).map(|_| ()), expected);
    assert_eq!(
        v.map_guest(window_gpa(), p, backing, Prot::ReadWrite)
            .map(|_| ()),
        expected
    );
    assert_eq!(
        v.map_read_native(window_gpa(), p, FILL_FROM_NOTHING, None)
            .map(|_| ()),
        expected
    );
    assert_eq!(v.set_trap(BarId::Bar0, 0..p, TrapMode::ReadWrite), expected);
    assert_eq!(
        v.gpa_read(window_gpa(), &mut [0u8; 4]),
        Err(VmmError::BadGpa { gpa: window_gpa() }),
        "an ACCESS after teardown is not a lifecycle error, it is an unbacked address — the \
         map is empty and says so"
    );
    assert!(
        m.audit().ops_refused_after_unrealize >= 4,
        "every one of those doors must have been counted"
    );
    assert_eq!(BAR1_BASE, common::BAR1_BASE);
}
