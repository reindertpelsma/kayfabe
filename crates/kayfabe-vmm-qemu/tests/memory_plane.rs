//! ★★★ The memory plane: the two backings, the prove-RAM classification, the placements,
//! the overlays, and the R1 witnesses.
//!
//! This is stage Q1's centre. Everything here runs against
//! [`kayfabe_vmm_qemu::mock_host::MockQemuHost`] — no hypervisor, no guest, no GPU — and
//! the properties are the ones a hypervisor would only be able to tell us about by
//! hanging.

mod common;

use common::{
    FOREIGN_RAM, machine, non_ram_shapes, overlay_gpa, overlay_len, overlay_trap, page, ram_facts,
    window_gpa, window_len,
};
use kayfabe_util::lockwitness;
use kayfabe_vmm::{
    BarId, CoreEvent, CoreEventKind, HostRegion, IrqSpec, Prot, RegionKind, SlotId, TrapMode, Vmm,
    VmmError,
};
use kayfabe_vmm_qemu::host::QemuHost;
use kayfabe_vmm_qemu::host::SectionFacts;
use kayfabe_vmm_qemu::mock_host::HostCall;
use kayfabe_vmm_qemu::{
    NO_LEGACY_INTX, NO_SHARED_BACKING, NO_SUCH_OVERLAY, OVERLAY_ALREADY_CLAIMED,
    OVERLAY_BACKING_UNPLACEABLE, OVERLAY_SHAPE_MISMATCH, PER_OBJECT_PROTECTION,
};

/// The sentinel the port established for *"a read-native overlay filled from nothing"*.
const FILL_FROM_NOTHING: HostRegion = HostRegion {
    id: u64::MAX,
    offset: 0,
};

// =====================================================================================
// The two backings, and the leaf-depth split that is finding 2's whole mechanism
// =====================================================================================

/// ★★ A round trip through **our own reservation**, with the copy proved to be running
/// **outside** the view lock.
///
/// `own_copy_leaf_depth_max == 0` is the assertion; `view_leaf_depth_max >= 1` is its
/// non-vacuity half, because a witness that never saw the lock held at all would report
/// zero for both and pass the first assertion for the wrong reason.
#[test]
fn our_own_reservation_round_trips_with_the_copy_outside_the_view_lock() {
    let (m, host) = machine();
    let mut v = m.vmm();
    let pattern: Vec<u8> = (0..64u8).collect();

    v.gpa_write(window_gpa() + 8, &pattern)
        .expect("a write into our reservation");
    let mut back = vec![0u8; pattern.len()];
    v.gpa_read(window_gpa() + 8, &mut back)
        .expect("and it reads back");
    assert_eq!(back, pattern, "the bytes are the bytes");

    let a = m.audit();
    assert_eq!(
        a.own_copy_leaf_depth_max, 0,
        "★ the copy out of our own reservation runs with NO leaf lock held — the `Arc` is \
         what keeps the mapping alive across the gap, and a copy inside the lock would \
         serialise every pushbuffer read in the device against every other"
    );
    assert!(
        a.view_leaf_depth_max >= 1,
        "★ NON-VACUITY: the leaf witness really did see the view lock held. Without this \
         the assertion above is equally true of a witness that is wired to nothing"
    );
    assert_eq!(
        a.host_copy_leaf_depth_min,
        u32::MAX,
        "and no host-owned copy happened, so the other half of the split is untouched"
    );
    assert!(
        !host
            .log()
            .iter()
            .any(|c| matches!(c, HostCall::ReadRegion(..) | HostCall::WriteRegion(..))),
        "★ and the hypervisor was not called AT ALL: a copy inside our own reservation is \
         a memcpy into memory we mapped, and reaching a general accessor for it is the \
         inversion the whole region map exists to make unconstructible ({:?})",
        host.log()
    );
}

/// ★★★ A round trip through a **host-owned** region, with the copy proved to be running
/// **inside** the view lock — finding 2's mechanism, asserted rather than described.
///
/// This is the one place this adapter's ladder differs from the KVM backend's, and the
/// reason is not performance: keeping the copy inside the lock is what lets `region_del`
/// release the hypervisor's reference **on the callback's own thread**, which is the only
/// context where a finalizer is legal. Move the copy outside the lock and the release has
/// to be deferred to a thread that does not hold the global lock, which is worse than the
/// problem #57 was about.
#[test]
fn a_host_owned_region_round_trips_with_the_copy_inside_the_view_lock() {
    let (m, host) = machine();
    let p = page();
    let section = host.mint_foreign(FOREIGN_RAM, 4 * p, ram_facts());
    m.region_add(section).expect("plain RAM is admitted");

    let mut v = m.vmm();
    let pattern: Vec<u8> = (0..32u8).map(|i| i.wrapping_mul(7)).collect();
    v.gpa_write(FOREIGN_RAM + p, &pattern)
        .expect("a write into reported guest RAM");
    let mut back = vec![0u8; pattern.len()];
    v.gpa_read(FOREIGN_RAM + p, &mut back)
        .expect("and it reads back");
    assert_eq!(back, pattern, "the bytes are the bytes");

    assert_eq!(
        host.region_bytes(section.mr).expect("the region exists")
            [usize::try_from(p).expect("a test-sized page")..][..pattern.len()],
        pattern[..],
        "★ and they landed at the right OFFSET inside the region. A resolution that \
         ignored the offset would round-trip perfectly and write to the wrong page"
    );

    assert!(
        m.audit().host_copy_leaf_depth_min >= 1,
        "★★ the copy out of a host-owned region runs INSIDE the view lock. If this ever \
         reads 0, `region_del` can no longer release the hypervisor's reference on its own \
         thread and the whole of finding 2 has silently reversed"
    );
    assert_eq!(
        m.audit().own_copy_leaf_depth_max,
        0,
        "and the reservation's copies are still outside it — the split is a split, not a \
         blanket rule that happened to be applied twice"
    );
}

/// ★★★ **The classification, swept through the listener and out the other side.**
///
/// `classify.rs` unit-tests the predicate; this tests the *consequence*, which is the
/// thing that would actually hurt: a section admitted as RAM is a section the core will
/// memcpy into. Each of §5.3's three directions gets a positive case here, plus the two
/// the design does not enumerate.
///
/// The assertion is the **exact** refusal — [`VmmError::NonRamGpa`] and not its near
/// neighbour [`VmmError::BadGpa`] — because omitting the section entirely would produce
/// the second, and the two must never start reporting as each other.
#[test]
fn every_non_ram_section_shape_is_declared_a_device_and_refuses_by_the_device_name() {
    let p = page();
    for (what, facts) in non_ram_shapes() {
        let (m, host) = machine();
        let section = host.mint_foreign(FOREIGN_RAM, 4 * p, facts);
        m.region_add(section).unwrap_or_else(|e| {
            panic!("{what}: a device section is DECLARED, never dropped: {e:?}")
        });

        let mut v = m.vmm();
        let mut buf = [0u8; 8];
        assert_eq!(
            v.gpa_read(FOREIGN_RAM, &mut buf).err(),
            Some(VmmError::NonRamGpa { gpa: FOREIGN_RAM }),
            "{what}: the address resolves to SOMETHING and that something is not plain \
             memory, so it must refuse as a device — `BadGpa` would say nothing is there, \
             which is a different fact and a different bug"
        );
        assert_eq!(
            v.gpa_write(FOREIGN_RAM, &[1, 2, 3, 4]).err(),
            Some(VmmError::NonRamGpa { gpa: FOREIGN_RAM }),
            "{what}: and the write direction is the sharper one — a stray write into a \
             device register window is a side effect on hardware"
        );
        assert!(
            !host
                .log()
                .iter()
                .any(|c| matches!(c, HostCall::RefRegion(_))),
            "{what}: ★ and NO reference was taken. We only cache a region we are going to \
             copy against; taking one for a device region would be a reference we then \
             have to remember to release for a region we never use ({:?})",
            host.log()
        );
        assert_eq!(
            m.audit().accesses_refused,
            2,
            "{what}: both refusals were counted"
        );
    }

    // The positive control, in the same shape, so "everything refuses" cannot pass.
    let (m, host) = machine();
    let section = host.mint_foreign(FOREIGN_RAM, 4 * p, ram_facts());
    m.region_add(section).expect("plain RAM is admitted");
    let mut v = m.vmm();
    let mut buf = [0u8; 8];
    assert_eq!(
        v.gpa_read(FOREIGN_RAM, &mut buf),
        Ok(()),
        "★ NON-VACUITY: plain RAM is served. Without this row every assertion above holds \
         for an implementation that refuses everything"
    );
    assert_eq!(
        host.log()
            .iter()
            .filter(|c| matches!(c, HostCall::RefRegion(_)))
            .count(),
        1,
        "and exactly one reference was taken for it"
    );
}

/// ★★ The two near-neighbour refusals, asserted **apart**, over a swept set of addresses.
///
/// A hole is [`VmmError::BadGpa`]; a device is [`VmmError::NonRamGpa`]. They are adjacent
/// in the address space and adjacent in the enum, and the whole reason the second variant
/// exists is that folding them loses the fact that the guest steered at a device.
#[test]
fn a_hole_and_a_device_refuse_by_different_names_across_the_whole_layout() {
    let (m, host) = machine();
    let p = page();
    m.region_add(host.mint_foreign(FOREIGN_RAM, 4 * p, ram_facts()))
        .expect("guest RAM");
    m.region_add(host.mint_foreign(FOREIGN_RAM + 8 * p, p, SectionFacts::device()))
        .expect("another device's register window");

    let mut v = m.vmm();
    for (what, gpa, expected) in [
        ("inside reported guest RAM", FOREIGN_RAM, None),
        ("inside our own reservation", window_gpa(), None),
        (
            "the gap between the two reported sections",
            FOREIGN_RAM + 5 * p,
            Some(VmmError::BadGpa {
                gpa: FOREIGN_RAM + 5 * p,
            }),
        ),
        (
            "another device's register window",
            FOREIGN_RAM + 8 * p,
            Some(VmmError::NonRamGpa {
                gpa: FOREIGN_RAM + 8 * p,
            }),
        ),
        (
            "our own register BAR",
            common::BAR0_BASE,
            Some(VmmError::NonRamGpa {
                gpa: common::BAR0_BASE,
            }),
        ),
        (
            "the unclaimed read-native overlay, which is inside our BAR",
            overlay_gpa(),
            Some(VmmError::NonRamGpa { gpa: overlay_gpa() }),
        ),
        (
            "an address in no region at all",
            0xDEAD_0000_0000,
            Some(VmmError::BadGpa {
                gpa: 0xDEAD_0000_0000,
            }),
        ),
        (
            "an address past the end of our reservation but inside its BAR",
            window_gpa() + window_len(),
            Some(VmmError::NonRamGpa {
                gpa: window_gpa() + window_len(),
            }),
        ),
    ] {
        let mut buf = [0u8; 4];
        assert_eq!(
            v.gpa_read(gpa, &mut buf).err(),
            expected,
            "{what}: nothing else in the system reports the difference between 'a device \
             is there' and 'nothing is there', so these two must never merge"
        );
    }
}

/// ★ A range that starts in RAM and leaves it reports the **boundary** byte, not its own
/// start — swept over both kinds of neighbour, because which variant the boundary reports
/// depends on what is on the other side.
#[test]
fn a_straddling_range_reports_the_boundary_byte_and_names_what_is_on_the_other_side() {
    let (m, host) = machine();
    let p = page();
    // RAM, then a device immediately after it: the boundary lands on the device.
    m.region_add(host.mint_foreign(FOREIGN_RAM, 2 * p, ram_facts()))
        .expect("guest RAM");
    m.region_add(host.mint_foreign(FOREIGN_RAM + 2 * p, p, SectionFacts::device()))
        .expect("a device right behind it");

    let mut v = m.vmm();
    let mut buf = vec![0u8; usize::try_from(p).expect("a test-sized page")];
    assert_eq!(
        v.gpa_read(FOREIGN_RAM + 2 * p - 8, &mut buf).err(),
        Some(VmmError::NonRamGpa {
            gpa: FOREIGN_RAM + 2 * p
        }),
        "★ the report names the first byte OUTSIDE the region — so a straddling descriptor \
         names its boundary and a wholly-foreign one names itself, and the two cases stay \
         distinguishable in a log"
    );

    // Our reservation, with nothing behind it inside the BAR except the BAR's own device
    // declaration: the boundary reports the BAR.
    assert_eq!(
        v.gpa_read(window_gpa() + window_len() - 8, &mut buf).err(),
        Some(VmmError::NonRamGpa {
            gpa: window_gpa() + window_len()
        }),
        "the same rule applies to our own reservation's far edge"
    );
}

// =====================================================================================
// The topology listener
// =====================================================================================

/// ★★ `region_del` undeclares **first** and releases the reference **after** — §5.2's
/// order, with the release proved to have happened and the range proved to have stopped
/// resolving.
#[test]
fn region_del_undeclares_before_it_releases_and_leaves_no_reference_outstanding() {
    let (m, host) = machine();
    let p = page();
    let section = host.mint_foreign(FOREIGN_RAM, 4 * p, ram_facts());
    m.region_add(section).expect("guest RAM");
    assert_eq!(
        host.live_regions()
            .iter()
            .find(|(h, _)| *h == section.mr)
            .map(|(_, r)| *r),
        Some(1),
        "★ NON-VACUITY: a reference really is outstanding, so 'it was released' is a claim \
         about something that happened"
    );
    let adds = m.audit().topology_generation;

    m.region_del(FOREIGN_RAM, 4 * p);

    assert_eq!(
        m.resolve_region(FOREIGN_RAM, 8).err(),
        Some(VmmError::BadGpa { gpa: FOREIGN_RAM }),
        "the range stops resolving — and as NOTHING, not as a device: an undeclared range \
         is absent, and a stale device declaration would be a different lie"
    );
    assert_eq!(
        host.live_regions()
            .iter()
            .find(|(h, _)| *h == section.mr)
            .map(|(_, r)| *r),
        Some(0),
        "and the reference is released, or the hypervisor can never finalize the region"
    );
    let log = host.log();
    let unref_at = log
        .iter()
        .position(|c| *c == HostCall::UnrefRegion(section.mr))
        .expect("the release is in the log");
    assert_eq!(
        unref_at,
        log.len() - 1,
        "★ and it is the LAST thing that happened — the undeclare ran under our leaf lock \
         and the release ran after it was dropped, which is what keeps the release on the \
         callback's own thread where a finalizer is legal ({log:?})"
    );
    assert_eq!(
        m.audit().topology_generation,
        adds + 1,
        "the generation moved, so the listener is observably wired to something"
    );
    assert_eq!(m.audit().topology_dels, 1);
}

/// ★ A delete for a range nobody added is a **no-op**, not a panic and not a release of
/// somebody else's reference.
#[test]
fn a_delete_for_a_range_that_was_never_added_changes_nothing() {
    let (m, host) = machine();
    let before = (m.audit().topology_generation, host.log().len());
    m.region_del(0xDEAD_0000, 0x1000);
    m.region_del(window_gpa(), window_len());
    assert_eq!(
        (m.audit().topology_generation, host.log().len()),
        before,
        "★ including a delete aimed at OUR OWN reservation: the listener's map and our \
         installs are separate sources, and a callback must never be able to undeclare a \
         range this device published"
    );
    assert_eq!(
        m.resolve_region(window_gpa(), 8).map(|s| s.len),
        Ok(8),
        "and the reservation still resolves"
    );
}

/// ★★★ A reported section that overlaps a region **we** published is refused, swept over
/// every way one range can overlap another.
///
/// §5.2 never says which source wins when the two collide. Here neither does: the
/// collision is a refusal, and the refusal happens **before** any reference is taken, so
/// it leaves the hypervisor's counts exactly as it found them.
#[test]
fn a_reported_section_overlapping_a_region_we_published_is_refused_every_way_it_can_overlap() {
    let p = page();
    let bar0 = common::BAR0_BASE;
    let bar0_len = 64 * p;
    for (what, gpa, len) in [
        ("exactly our BAR", bar0, bar0_len),
        ("the first page of our BAR", bar0, p),
        ("the last page of our BAR", bar0 + bar0_len - p, p),
        ("a range wholly inside our BAR", bar0 + 4 * p, p),
        ("a range straddling our BAR's start", bar0 - p, 2 * p),
        (
            "a range straddling our BAR's end",
            bar0 + bar0_len - p,
            2 * p,
        ),
        (
            "a range containing our whole BAR",
            bar0 - p,
            bar0_len + 2 * p,
        ),
        (
            "our aperture BAR, where the reservation lives",
            common::BAR1_BASE,
            p,
        ),
    ] {
        let (m, host) = machine();
        let section = host.mint_foreign(gpa, len, ram_facts());
        assert_eq!(
            m.region_add(section).err(),
            Some(VmmError::Unsupported(
                kayfabe_vmm_qemu::FOREIGN_OVERLAPS_OURS
            )),
            "{what}: the two declaration sources would race to own the same guest-physical \
             range and the loser would be whichever ran second"
        );
        assert!(
            !host
                .log()
                .iter()
                .any(|c| matches!(c, HostCall::RefRegion(_))),
            "{what}: ★ and the refusal took NO reference — a refusal that had already \
             taken one is a leak with a tidy return value"
        );
        assert_eq!(
            m.audit().topology_adds,
            0,
            "{what}: and it was not counted as an add"
        );
    }

    // The control: a section that touches nothing of ours is admitted.
    let (m, host) = machine();
    assert_eq!(
        m.region_add(host.mint_foreign(bar0 - 2 * p, p, ram_facts())),
        Ok(()),
        "★ NON-VACUITY: a section immediately below our BAR and not touching it IS \
         admitted, so the sweep above is about overlap and not about 'refuse everything'"
    );
}

// =====================================================================================
// The fine tier
// =====================================================================================

/// ★★ §5.4's frequency claim: a publication performs **no hypervisor call at all**, so
/// ten of them leave `regions_published` exactly where realize left it.
///
/// The KVM backend's memslot-frequency gate is `memslot_installs` against
/// `placements_made`. Here the numerator is a constant by construction, which is a
/// stronger form of the same rule — and it is asserted by content, not by a ratio.
#[test]
fn ten_publications_into_one_reservation_call_the_hypervisor_exactly_zero_times() {
    let (m, host) = machine();
    let p = page();
    let published_at_realize = m.audit().regions_published;
    let log_at_realize = host.log().len();
    let backing = m.register_backing(p).expect("a host backing");

    let mut v = m.vmm();
    let mut slots = Vec::new();
    for i in 0..8u64 {
        slots.push(
            v.map_guest(window_gpa() + i * p, p, backing, Prot::ReadWrite)
                .unwrap_or_else(|e| panic!("publication {i} lands: {e:?}")),
        );
    }
    assert_eq!(
        m.audit().placements_made,
        8,
        "★ NON-VACUITY: eight placements really happened"
    );
    assert_eq!(
        m.audit().regions_published,
        published_at_realize,
        "★ and not one of them handed the hypervisor a region. A publication that did \
         would put a topology transaction on the data path, which needs the global lock \
         this adapter never takes"
    );
    assert_eq!(
        host.log().len(),
        log_at_realize,
        "and the hypervisor was not called at all ({:?})",
        &host.log()[log_at_realize..]
    );
    assert_eq!(m.audit().live_placements, 8);

    for (i, s) in slots.into_iter().enumerate() {
        assert_eq!(v.unmap_guest(s), Ok(()), "placement {i} is restored");
    }
    assert_eq!(
        m.audit().live_placements,
        0,
        "and the ledger balances — a restore that did not debit would leave the plane \
         claiming placements that are not there"
    );
    assert_eq!(
        host.log().len(),
        log_at_realize,
        "and the removals called the hypervisor no more than the placements did"
    );
}

/// ★ The four ways a placement is refused, asserted apart.
#[test]
fn a_placement_is_refused_by_the_exact_reason_it_could_not_be_made() {
    let (m, _host) = machine();
    let p = page();
    let backing = m.register_backing(p).expect("a host backing");
    let mut v = m.vmm();

    assert_eq!(
        v.map_guest(window_gpa(), p, backing, Prot::ReadOnly).err(),
        Some(VmmError::Unsupported(PER_OBJECT_PROTECTION)),
        "protection is a WINDOW property; minting something to make one page read-only is \
         the cheap fix this refusal exists to prevent"
    );
    assert_eq!(
        v.map_guest(0xDEAD_0000, p, backing, Prot::ReadWrite).err(),
        Some(VmmError::BadGpa { gpa: 0xDEAD_0000 }),
        "a guest-physical address inside no reservation"
    );
    assert_eq!(
        v.map_guest(
            window_gpa() + window_len() - p,
            2 * p,
            backing,
            Prot::ReadWrite
        )
        .err(),
        Some(VmmError::BadGpa {
            gpa: window_gpa() + window_len() - p
        }),
        "a placement that starts inside a reservation and leaves it"
    );
    assert_eq!(
        v.map_guest(
            window_gpa(),
            p,
            HostRegion { id: 999, offset: 0 },
            Prot::ReadWrite
        )
        .err(),
        Some(VmmError::Unsupported(
            "a host backing id this backend never minted"
        )),
        "a backing this backend never minted — the ids are backend-scoped and opaque, so \
         a foreign one is not a bad address, it is a bad handle"
    );

    let first = v
        .map_guest(window_gpa() + p, 2 * p, backing, Prot::ReadWrite)
        .expect("the first placement lands");
    for (what, gpa, len) in [
        ("exactly the same range", window_gpa() + p, 2 * p),
        ("a range contained in it", window_gpa() + p, p),
        ("a range straddling its start", window_gpa(), 2 * p),
        ("a range straddling its end", window_gpa() + 2 * p, 2 * p),
    ] {
        assert_eq!(
            v.map_guest(gpa, len, backing, Prot::ReadWrite).err(),
            Some(VmmError::Unsupported(
                "a placement overlapping one already live in the same reservation"
            )),
            "{what}: two placements over one range means one of them is silently not there"
        );
    }
    assert_eq!(
        v.map_guest(window_gpa() + 4 * p, p, backing, Prot::ReadWrite)
            .map(|_| ()),
        Ok(()),
        "★ NON-VACUITY: a non-overlapping placement still lands, so the sweep above is \
         about overlap and not about a reservation that stopped accepting anything"
    );
    assert_eq!(v.unmap_guest(first), Ok(()));
    assert_eq!(
        v.unmap_guest(first).err(),
        Some(VmmError::BadSlot(first)),
        "and a slot cannot be removed twice"
    );
    assert_eq!(
        v.unmap_guest(SlotId(9999)).err(),
        Some(VmmError::BadSlot(SlotId(9999))),
        "nor can one that was never minted"
    );
}

// =====================================================================================
// The read-native overlay — findings 1, 3 and 4
// =====================================================================================

/// ★★★ **Finding 3.** An unclaimed overlay refuses a write by the device name; a claimed
/// one is RAM the core can keep current; and unmapping it puts it back.
///
/// §5.4 says to classify the overlay `Device` *"so the core can never `gpa_write` through
/// it"*. That cannot hold: the port's own contract for this method is *"RAM the core keeps
/// current"*, and `gpa_write` is the only way the core has to keep anything current. The
/// three-phase assertion below is what settles it — and the middle phase is exactly what
/// the existing mean run does immediately after installing an overlay.
#[test]
fn an_overlay_is_a_device_until_it_is_claimed_and_ram_the_core_can_keep_current_after() {
    let (m, host) = machine();
    let mut v = m.vmm();
    let payload = [0xA5u8, 0x5A, 0x11, 0x22];

    assert_eq!(
        v.gpa_write(overlay_gpa(), &payload).err(),
        Some(VmmError::NonRamGpa { gpa: overlay_gpa() }),
        "★ before it is claimed nothing has told us what belongs in it, so a write must \
         refuse by name rather than scribble on a register page"
    );

    let slot = v
        .map_read_native(
            overlay_gpa(),
            overlay_len(),
            FILL_FROM_NOTHING,
            Some(overlay_trap()),
        )
        .expect("the realized overlay is claimed");
    assert_eq!(
        m.audit().overlays_claimed,
        1,
        "and the claim is counted, so 'nothing was claimed' is a failing assertion"
    );
    assert_eq!(
        v.gpa_write(overlay_gpa() + 16, &payload),
        Ok(()),
        "★★ a CLAIMED overlay is RAM the core can keep current — which is what the port's \
         own contract for this method says it is. Classifying it a device here makes this \
         backend refuse what the KVM backend serves, on the same core traffic"
    );
    let mr = match host
        .log()
        .into_iter()
        .find(|c| matches!(c, HostCall::PublishRomOverlay(_)))
    {
        Some(HostCall::PublishRomOverlay(mr)) => mr,
        other => panic!("the overlay must have been published at realize, got {other:?}"),
    };
    assert_eq!(
        host.region_bytes(mr).expect("the overlay exists")[16..20],
        payload,
        "★ and the bytes landed in the hypervisor's own allocation, at the right offset. \
         The overlay's RAM is NOT inside our reservation — there is no pointer-taking \
         constructor for a ROM-device region — so this is the only path there is"
    );
    let mut back = [0u8; 4];
    assert_eq!(v.gpa_read(overlay_gpa() + 16, &mut back), Ok(()));
    assert_eq!(back, payload, "and it reads back through the same path");

    assert_eq!(v.unmap_guest(slot), Ok(()));
    assert_eq!(m.audit().overlays_claimed, 0, "the claim is given back");
    assert_eq!(
        v.gpa_write(overlay_gpa(), &payload).err(),
        Some(VmmError::NonRamGpa { gpa: overlay_gpa() }),
        "★ and it is a device again — the hypervisor still serves reads out of it, but \
         nothing of ours is keeping it current, which is a different state from 'gone'"
    );
    assert_eq!(
        host.live_regions().len(),
        2,
        "and the region itself was NOT unpublished: that would be a topology transaction, \
         and unmapping a slot happens on whatever thread the core is on"
    );
}

/// ★★★ **Findings 1 and 4**, as four refusals that must never report as one another.
///
/// On this backend an overlay is realize-time configuration, so `map_read_native` can only
/// *claim* one. The four ways that fails are a configuration mistake (no overlay there), a
/// configuration mismatch (an overlay of a different shape), a lifecycle bug (already
/// claimed) and a portability difference (a caller-supplied backing, which cannot be
/// placed under a region the hypervisor allocated).
#[test]
fn claiming_a_read_native_overlay_fails_by_four_distinguishable_names() {
    let (m, _host) = machine();
    let p = page();
    let mut v = m.vmm();

    assert_eq!(
        v.map_read_native(
            overlay_gpa(),
            overlay_len(),
            HostRegion { id: 0, offset: 0 },
            Some(overlay_trap())
        )
        .err(),
        Some(VmmError::Unsupported(OVERLAY_BACKING_UNPLACEABLE)),
        "★ finding 4: the backing argument cannot mean here what it means on a backend \
         that places it — nothing may be placed into an overlay the hypervisor allocated. \
         And it is checked FIRST, so it cannot be masked by a shape mismatch"
    );

    for (what, gpa) in [
        ("an address with no overlay at all", common::BAR0_BASE),
        ("one page past the overlay's base", overlay_gpa() + p),
        ("inside our aperture BAR", common::BAR1_BASE),
    ] {
        assert_eq!(
            v.map_read_native(gpa, overlay_len(), FILL_FROM_NOTHING, Some(overlay_trap()))
                .err(),
            Some(VmmError::Unsupported(NO_SUCH_OVERLAY)),
            "{what}: ★ finding 1 — publishing an overlay is a topology transaction, so it \
             is realize-time configuration and this method can only claim one that exists"
        );
    }

    for (what, len, trap) in [
        ("the right base, the wrong length", p, Some(overlay_trap())),
        (
            "the right base, no write trap where one was realized",
            overlay_len(),
            None,
        ),
        (
            "the right base, a write trap of a different shape",
            overlay_len(),
            Some(overlay_gpa()..overlay_gpa() + 2 * p),
        ),
    ] {
        assert_eq!(
            v.map_read_native(overlay_gpa(), len, FILL_FROM_NOTHING, trap.clone())
                .err(),
            Some(VmmError::Unsupported(OVERLAY_SHAPE_MISMATCH)),
            "{what}: an overlay IS there, and saying 'no such overlay' would send an \
             operator looking for a missing configuration row instead of a wrong one"
        );
    }

    let slot = v
        .map_read_native(
            overlay_gpa(),
            overlay_len(),
            FILL_FROM_NOTHING,
            Some(overlay_trap()),
        )
        .expect("★ NON-VACUITY: the exactly-matching claim succeeds");
    assert_eq!(
        v.map_read_native(
            overlay_gpa(),
            overlay_len(),
            FILL_FROM_NOTHING,
            Some(overlay_trap())
        )
        .err(),
        Some(VmmError::Unsupported(OVERLAY_ALREADY_CLAIMED)),
        "a second claim is a lifecycle bug, not a configuration one"
    );
    assert_eq!(v.unmap_guest(slot), Ok(()));
    assert!(
        v.map_read_native(
            overlay_gpa(),
            overlay_len(),
            FILL_FROM_NOTHING,
            Some(overlay_trap())
        )
        .is_ok(),
        "and after unmapping it can be claimed again — 'already claimed' must be a state, \
         not a one-way door"
    );
}

// =====================================================================================
// R1, interrupts, the clock, and the export precondition
// =====================================================================================

/// ★★ R1: no syscall-shaped method ever runs with one of the core's ranked locks held.
///
/// The witness has two halves and both are load-bearing: the maximum must be zero, and
/// the minimum must not still be `u32::MAX`, which would mean no syscall-shaped method
/// ran at all and the maximum is zero for the wrong reason.
#[test]
fn no_syscall_shaped_method_ever_runs_with_a_ranked_lock_held() {
    let (m, _host) = machine();
    let p = page();
    let backing = m.register_backing(p).expect("a host backing");
    let mut v = m.vmm();

    let slot = v
        .map_guest(window_gpa(), p, backing, Prot::ReadWrite)
        .expect("a placement");
    v.unmap_guest(slot).expect("and its removal");
    v.export_ram(None).expect("an export");
    v.set_trap(BarId::Bar0, 0..p, TrapMode::ReadWrite)
        .expect("a trap registration");
    v.map_read_native(
        overlay_gpa(),
        overlay_len(),
        FILL_FROM_NOTHING,
        Some(overlay_trap()),
    )
    .expect("an overlay claim");

    let a = m.audit();
    assert_eq!(
        a.syscall_ranked_depth,
        (0, 0),
        "★ every syscall-shaped method ran lock-free. The lower half is what makes this \
         non-vacuous: a minimum still at u32::MAX would mean none of them ran"
    );

    // The in-lock-LEGAL half: the accessors are entered with a rank held, on purpose, and
    // that is what the accessor witness must have seen.
    lockwitness::note_acquired(0);
    let mut buf = [0u8; 8];
    let r = v.gpa_read(window_gpa(), &mut buf);
    lockwitness::note_released(0);
    assert_eq!(r, Ok(()));
    let a = m.audit();
    assert_eq!(
        a.accessor_ranked_depth.1, 1,
        "★ NON-VACUITY of the OTHER witness: the in-lock hazard really was exercised. \
         Without a rank held here, `own_copy_leaf_depth_max == 0` would be a statement \
         about a path the core never takes"
    );
    assert_eq!(
        a.own_copy_leaf_depth_max, 0,
        "and the copy still ran outside every lock of ours, under that rank"
    );
}

/// ★ Interrupts: the vector reaches the host exactly once per call, an out-of-range vector
/// is refused, and the legacy variant is a named backend-conditional refusal.
#[test]
fn raise_irq_is_one_host_call_per_vector_and_the_legacy_variant_is_refused_by_name() {
    let (m, host) = machine();
    let mut v = m.vmm();
    for vec in 0..8u16 {
        assert_eq!(v.raise_irq(IrqSpec::Msix(vec)), Ok(()), "vector {vec}");
    }
    assert_eq!(
        host.irqs(),
        (0..8u16).collect::<Vec<_>>(),
        "★ by exact content and in order, never by count: a backend that raised vector 0 \
         eight times would pass a count assertion and deliver every completion to the \
         wrong queue"
    );
    assert_eq!(m.audit().irqs_raised, 8);
    for vec in [8u16, 9, u16::MAX] {
        assert_eq!(
            v.raise_irq(IrqSpec::Msix(vec)).err(),
            Some(VmmError::Unsupported(
                "a vector this device was not realized with"
            )),
            "vector {vec} is outside the table this device was realized with"
        );
    }
    assert_eq!(
        v.raise_irq(IrqSpec::IntxLevel(true)).err(),
        Some(VmmError::Unsupported(NO_LEGACY_INTX)),
        "backend-conditional by design — a refusal, never a silently dropped injection"
    );
    assert_eq!(
        m.audit().irqs_raised,
        8,
        "and none of the four refusals was counted as a raise"
    );
}

/// ★ Deferred events come back in deadline-then-insertion order, shared with every other
/// backend through the port's own queue.
#[test]
fn deferred_events_come_back_in_deadline_then_insertion_order() {
    let (m, _host) = machine();
    let mut v = m.vmm();
    let late = CoreEvent::Deferred(CoreEventKind::PollKickBudget);
    let early_a = CoreEvent::Deferred(CoreEventKind::DeferredReap);
    let early_b = CoreEvent::Deferred(CoreEventKind::RegionFault);

    v.defer(core::time::Duration::from_millis(20), late.clone());
    v.defer(core::time::Duration::from_millis(5), early_a.clone());
    v.defer(core::time::Duration::from_millis(5), early_b.clone());

    assert_eq!(
        m.advance(core::time::Duration::from_millis(5)),
        vec![early_a, early_b],
        "★ the two due at the same instant come out in INSERTION order — ordering them by \
         the payload's own `Ord` would be deterministic and deterministically wrong"
    );
    assert_eq!(
        m.advance(core::time::Duration::from_millis(1)),
        Vec::new(),
        "and nothing else is due yet"
    );
    assert_eq!(m.advance(core::time::Duration::from_millis(20)), vec![late]);
}

/// ★ The deployment fact no code gate can observe: a machine realized without a shareable
/// backing refuses the **first** export, loudly, rather than at first guest DMA.
#[test]
fn a_machine_without_a_shareable_backing_refuses_the_first_export() {
    let cfg = kayfabe_vmm_qemu::MachineConfig {
        shareable_ram: false,
        ..common::config()
    };
    let (m, _host) = common::machine_with(kayfabe_vmm_qemu::mock_host::MockPolicy::default(), cfg);
    let mut v = m.vmm();
    assert_eq!(
        v.export_ram(None).err(),
        Some(VmmError::Unsupported(NO_SHARED_BACKING)),
        "an isolate cannot map a backing the machine was not launched with, and the \
         failure must name that rather than arriving as a fault inside a guest transfer"
    );
    // ★ Everything else still works, which is what makes this a *deployment* fact rather
    // than a broken device: the refusal is specific to the one capability that needs it.
    let mut buf = [0u8; 8];
    assert_eq!(
        v.gpa_read(window_gpa(), &mut buf),
        Ok(()),
        "the reservation is still readable — a machine without a shareable backing is not \
         a machine without memory"
    );
}

/// ★ The region map and the plane agree about what a reported RAM region is: the kind the
/// map returns and the kind the classification produced are the same value, swept.
#[test]
fn the_kind_the_map_holds_is_the_kind_the_classification_produced() {
    let p = page();
    let mut shapes: Vec<(&str, SectionFacts, RegionKind)> =
        vec![("plain RAM", ram_facts(), RegionKind::Ram)];
    for (what, f) in non_ram_shapes() {
        shapes.push((what, f, RegionKind::Device));
    }
    for (what, facts, expected) in shapes {
        let (m, host) = machine();
        m.region_add(host.mint_foreign(FOREIGN_RAM, p, facts))
            .unwrap_or_else(|e| panic!("{what}: {e:?}"));
        let got = m.resolve_region(FOREIGN_RAM, 8);
        match expected {
            RegionKind::Ram => assert!(
                got.is_ok(),
                "{what}: the map resolves it, so the plane and the classification agree"
            ),
            RegionKind::Device => assert_eq!(
                got.err(),
                Some(VmmError::NonRamGpa { gpa: FOREIGN_RAM }),
                "{what}: the map holds it as a device, which is what the classification said"
            ),
        }
    }
}

/// ★ A zero-length access still names an address and is still proved, per the port's first
/// argued exemption — swept over a served address, a device and a hole.
#[test]
fn a_zero_length_access_is_still_proved_against_the_region_map() {
    let (m, host) = machine();
    let p = page();
    m.region_add(host.mint_foreign(FOREIGN_RAM, p, ram_facts()))
        .expect("guest RAM");
    m.region_add(host.mint_foreign(FOREIGN_RAM + 2 * p, p, SectionFacts::device()))
        .expect("a device");
    let mut v = m.vmm();
    let empty: [u8; 0] = [];
    let mut empty_mut: [u8; 0] = [];

    assert_eq!(v.gpa_write(window_gpa(), &empty), Ok(()));
    assert_eq!(v.gpa_read(FOREIGN_RAM, &mut empty_mut), Ok(()));
    assert_eq!(
        v.gpa_write(FOREIGN_RAM + 2 * p, &empty).err(),
        Some(VmmError::NonRamGpa {
            gpa: FOREIGN_RAM + 2 * p
        }),
        "★ a zero-length transfer translates nothing, but the rule this proof exists to \
         make total is 'every address we touch was proven RAM' — an exemption whose safety \
         argument is per-backend is exactly what must not be written down"
    );
    assert_eq!(
        v.gpa_read(0xDEAD_0000, &mut empty_mut).err(),
        Some(VmmError::BadGpa { gpa: 0xDEAD_0000 })
    );
}

/// ★★★ A reported section that is a **slice** of a larger region copies at that region's
/// own offset, swept over several slice offsets.
///
/// Found by a bite-check: every other section in this suite starts at offset zero inside
/// its region, which makes `region_off + span.offset` and a bare `span.offset` produce the
/// same answer for every one of them. Deleting the section's own offset therefore survived
/// the entire suite — an adapter that read and wrote the **wrong page of guest RAM** on
/// every aliased or partially-mapped region, silently.
///
/// The two offsets are independent by construction: `offset_within_region` says where the
/// section starts inside the hypervisor's region, and the guest-physical address says
/// where it starts in the address space. A region mapped in two pieces, or aliased at a
/// partial offset, has them differ — which is the ordinary case, not an exotic one.
#[test]
fn a_section_that_is_a_slice_of_a_larger_region_copies_at_that_regions_own_offset() {
    let p = page();
    for slice_off in [p, 2 * p, 7 * p] {
        let (m, host) = machine();
        let section = host.mint_foreign_slice(FOREIGN_RAM, 2 * p, slice_off, 16 * p, ram_facts());
        m.region_add(section).expect("plain RAM is admitted");

        let mut v = m.vmm();
        let payload: Vec<u8> = (0..48u8).map(|i| i ^ 0x3C).collect();
        let inner = 24u64;
        v.gpa_write(FOREIGN_RAM + inner, &payload)
            .unwrap_or_else(|e| panic!("slice at {slice_off:#x}: {e:?}"));

        let bytes = host.region_bytes(section.mr).expect("the region exists");
        let at = usize::try_from(slice_off + inner).expect("a test-sized offset");
        assert_eq!(
            &bytes[at..at + payload.len()],
            &payload[..],
            "★ slice at {slice_off:#x}: the write must land at the SECTION's offset inside \
             the region plus the offset inside the section. Dropping the first term writes \
             to the wrong page of guest RAM and reads back its own mistake, so a round-trip \
             assertion alone cannot see it"
        );
        assert!(
            bytes[..at].iter().all(|b| *b == 0),
            "slice at {slice_off:#x}: and nothing before it was touched"
        );

        let mut back = vec![0u8; payload.len()];
        v.gpa_read(FOREIGN_RAM + inner, &mut back)
            .expect("and the read uses the same arithmetic");
        assert_eq!(
            back, payload,
            "slice at {slice_off:#x}: read and write agree"
        );

        // The read direction, proved independently of our own write: plant a byte through
        // the host and require the adapter to find it.
        let mut planted = vec![0u8; usize::try_from(2 * p).expect("fits")];
        planted[0] = 0xC7;
        host.write_region(section.mr, slice_off, &planted)
            .expect("the host can seed its own region");
        let mut first = [0u8; 1];
        v.gpa_read(FOREIGN_RAM, &mut first).expect("a read");
        assert_eq!(
            first[0], 0xC7,
            "slice at {slice_off:#x}: ★ the read direction resolves through the section's \
             offset too — asserted against a byte WE did not write through the adapter, so \
             a symmetric off-by-a-page error cannot cancel out"
        );
    }
}

/// ★★ `export_ram(Some(slice))` is refused unless a **shareable reservation covers the
/// whole slice**, swept over every way a slice can fail to be covered.
///
/// Found by a bite-check: with only the `None` case exercised, deleting the containment
/// test survived. It matters because the handle is handed to another process — an export
/// that covered less than it claimed would give an isolate a descriptor whose far end
/// stops part-way through the range the core believes it shared.
#[test]
fn exporting_a_slice_no_reservation_covers_is_refused_by_name() {
    let (m, _host) = machine();
    let p = page();
    let mut v = m.vmm();
    let uncovered = "no shareable reservation covers the requested slice";

    assert!(
        v.export_ram(None).is_ok(),
        "★ NON-VACUITY: the whole-of-memory export works, so every refusal below is about \
         the slice and not about a machine that cannot export at all"
    );
    assert!(
        v.export_ram(Some(window_gpa()..window_gpa() + window_len()))
            .is_ok(),
        "and so does a slice that is exactly the reservation"
    );
    assert!(
        v.export_ram(Some(window_gpa() + p..window_gpa() + 2 * p))
            .is_ok(),
        "and one wholly inside it"
    );

    for (what, slice) in [
        (
            "a slice that starts before the reservation",
            window_gpa() - p..window_gpa() + p,
        ),
        (
            "a slice that runs past its end",
            window_gpa() + window_len() - p..window_gpa() + window_len() + p,
        ),
        (
            "a slice that contains the reservation",
            window_gpa() - p..window_gpa() + window_len() + p,
        ),
        (
            "a slice nowhere near any reservation",
            FOREIGN_RAM..FOREIGN_RAM + p,
        ),
        (
            "a slice inside our register BAR",
            common::BAR0_BASE..common::BAR0_BASE + p,
        ),
    ] {
        assert_eq!(
            v.export_ram(Some(slice.clone())).err(),
            Some(VmmError::Unsupported(uncovered)),
            "{what}: the handle goes to another process, so a partial cover is a \
             descriptor whose far end stops before the range the core thinks it shared"
        );
    }
}
