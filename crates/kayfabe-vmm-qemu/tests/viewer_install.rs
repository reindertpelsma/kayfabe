//! ★★★ **The consumer of the GPGA viewer index, and the merge key that makes consolidation
//! safe.**
//!
//! The index landed with no consumer, deliberately. This file drives the consumer: a real
//! `ViewerIndex`, real objects, a real drain, and a real memslot plane underneath — the mock
//! one, because the whole allocator and the whole span algebra run without a kernel and the
//! project's runner has none.
//!
//! # ★★ What each half of this file is for
//!
//! - **The merge key.** Consolidating adjacent objects into one mapping is what makes the
//!   design feasible at all — memslots have a hard ceiling, so one per page is impossible
//!   rather than slow. But a *wrong* merge produces **fewer** mappings, which is exactly
//!   what a naive test rewards. So the tests below assert on the **key**, and every axis of
//!   it gets a pair of runs that are contiguous in both the view and the host backing and
//!   must **still** not merge.
//! - **The installer.** That index state becomes memslots, that revocation removes them,
//!   that unvouchable content gets none, and that the refusals are by name.

mod common;

use std::sync::Arc;

use kayfabe_arch::ids::GpuId;
use kayfabe_arch::{Aperture, FbWindow};
use kayfabe_isolate::IsolateId;
use kayfabe_linux_raw::{CachePolicy, HostPageSize};
use kayfabe_mmu::gpga::{GpgaRegion, ObjectChange, UnwitnessedTransport, ViewerIndex, ViewerKind};
use kayfabe_vmm::Prot;
use kayfabe_vmm_qemu::mock_host::MockSlotPlane;
use kayfabe_vmm_qemu::slots::{OUR_SLOT_BUDGET, Tier};
use kayfabe_vmm_qemu::viewer_install::{
    AlignmentCensus, BackingUnknown, CoveredRun, HUGE_PAGE_BYTES, HostBacking, HugePages,
    InstallRefusal, MergeKey, ObjectBacking, ViewInstaller, cacheability_of, census_of,
    huge_pages_for, mergeable, tier_at,
};
use kayfabe_vmm_qemu::{MachineConfig, QemuMachine};

// ─────────────────────────────────────────────────────────────────────────────────────
// Harness
// ─────────────────────────────────────────────────────────────────────────────────────

/// The viewer window: far enough into the aperture BAR that it never meets the realize-time
/// reservation, and big enough that a 2 MiB run fits when a test wants one.
fn win_gpa() -> u64 {
    common::BAR1_BASE + 64 * common::page()
}
fn win_len() -> u64 {
    64 * common::page()
}

/// A machine whose realize-time configuration declares **no** window, so the only memslots
/// in it are the ones the installer puts there. ★ Otherwise every count below would carry a
/// constant nobody set, which is how a count stops being evidence.
fn machine() -> (QemuMachine, Arc<MockSlotPlane>) {
    let host = common::host_with(kayfabe_vmm_qemu::mock_host::MockPolicy::default());
    let slots = common::slot_plane();
    let cfg = MachineConfig {
        windows: vec![],
        traps: vec![],
        ..common::config()
    };
    let m = QemuMachine::realize(cfg, host, Arc::clone(&slots) as _)
        .expect("the mock host places both BARs where the configuration says");
    (m, slots)
}

/// A backing source that hands out host offsets from a table, and can be told that an
/// object's bytes are on the real device.
#[derive(Default)]
struct Table {
    /// object id -> host offset in the window's reservation
    at: std::collections::BTreeMap<u64, u64>,
    /// object ids whose bytes are the real device's
    on_device: std::collections::BTreeSet<u64>,
}

impl ObjectBacking for Table {
    fn backing_for(
        &self,
        object: kayfabe_mmu::gpga::ObjectId,
        _aperture: Aperture,
        _gpga_base: u64,
        _len: u64,
    ) -> Result<HostBacking, BackingUnknown> {
        if self.on_device.contains(&object.0) {
            return Ok(HostBacking::HostGpuFramebuffer);
        }
        self.at
            .get(&object.0)
            .map(|&offset| HostBacking::VmmOwned { offset })
            .ok_or(BackingUnknown)
    }
}

/// Allocate one object at `base` of `len` bytes in video memory, owned by `owner`.
fn alloc(ix: &mut ViewerIndex, base: u64, len: u64, owner: u32) -> kayfabe_mmu::gpga::ObjectId {
    let change = ObjectChange::Allocated {
        region: GpgaRegion::new(Aperture::Vidmem, base, len).expect("a well-formed region"),
        owner: IsolateId::new(owner, GpuId(0)),
    };
    let plan = ix.plan(&change).expect("nothing else occupies it");
    ix.apply(&plan)
        .expect("the plan was built against this generation")
        .object
        .expect("an allocation mints an object")
}

/// A key that differs from `base` in exactly one field, so a test names the axis it is
/// varying instead of rebuilding the whole struct.
fn key(owner: u32, viewers: &[u32]) -> MergeKey {
    MergeKey {
        owner: IsolateId::new(owner, GpuId(0)),
        cache: CachePolicy::WriteBack,
        prot: Prot::ReadWrite,
        witnessed: true,
        viewers: viewers
            .iter()
            .map(|&v| kayfabe_mmu::gpga::ViewerId(v))
            .collect(),
    }
}

fn run(view_off: u64, len: u64, host_off: u64, k: MergeKey) -> CoveredRun {
    CoveredRun {
        view_off,
        len,
        host_off,
        objects: vec![kayfabe_mmu::gpga::ObjectId(0)],
        key: k,
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────
// ★★★ The merge key — every axis, asserted on the KEY and not on the count
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★★★ **Contiguity is not a licence to merge.** Five pairs, every one of them contiguous in
/// both the view and the host backing, and every one of them refused — each for a different
/// field of the key.
///
/// This is the test that would be green for a merge that is wrong if it counted mappings
/// instead of interrogating the condition.
#[test]
fn two_runs_that_are_adjacent_in_both_dimensions_still_do_not_merge_across_any_key_field() {
    let base = key(1, &[7]);
    let prev = run(0, 0x1000, 0x1000, base.clone());

    // The control: identical key, contiguous in both — this one MUST merge, or every
    // refusal below is vacuous because nothing ever merges.
    assert!(
        mergeable(&prev, 0x1000, 0x2000, &base),
        "the positive case must merge, or the negatives below prove nothing"
    );

    // 1. OWNER — one mapping cannot have two owning handle namespaces.
    let other_owner = MergeKey {
        owner: IsolateId::new(2, GpuId(0)),
        ..base.clone()
    };
    assert!(
        !mergeable(&prev, 0x1000, 0x2000, &other_owner),
        "a run owned by another isolate merged into this one; that is boundary 2 flattened \
         by an optimisation"
    );

    // 2. ★★★ VIEWERS — the cross-viewer leak, and the reason `viewers_of` exists.
    let wider_view = key(1, &[7, 9]);
    assert!(
        !mergeable(&prev, 0x1000, 0x2000, &wider_view),
        "a run visible to {{7,9}} merged with one visible to {{7}}; viewer 9 now sees bytes \
         the index never said it could"
    );
    // ...and it is the SET that matters, not its size: same count, different member.
    let same_size_different_member = key(1, &[8]);
    assert!(
        !mergeable(&prev, 0x1000, 0x2000, &same_size_different_member),
        "the viewer set is compared as a set, not by cardinality"
    );

    // 3. CACHE — #111's axis: a memory type that is wrong SILENTLY.
    let other_cache = MergeKey {
        cache: CachePolicy::WriteCombining,
        ..base.clone()
    };
    assert!(
        !mergeable(&prev, 0x1000, 0x2000, &other_cache),
        "a write-combining run merged with a write-back one; one of the two now has a \
         memory type nobody asked for and nothing will report it"
    );

    // 4. PROT — the stronger permission wins a merge, which is the wrong direction.
    let other_prot = MergeKey {
        prot: Prot::ReadOnly,
        ..base.clone()
    };
    assert!(
        !mergeable(&prev, 0x1000, 0x2000, &other_prot),
        "a read-only run merged with a read-write one and became writable"
    );

    // 5. WITNESS — making unvouchable bytes native is the one thing miss = fault forbids.
    let unvouchable = MergeKey {
        witnessed: false,
        ..base.clone()
    };
    assert!(
        !mergeable(&prev, 0x1000, 0x2000, &unvouchable),
        "an unwitnessed run merged into a witnessed one and became native"
    );

    // And the two contiguity clauses, which are necessary but famously not sufficient.
    assert!(
        !mergeable(&prev, 0x2000, 0x2000, &base),
        "a gap in the view is not contiguity"
    );
    assert!(
        !mergeable(&prev, 0x1000, 0x9000, &base),
        "a memslot names ONE contiguous host range; runs that abut in the view but not in \
         host memory cannot share one"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────
// Consolidation, driven through the real index
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★★★ **Many objects, one mapping** — the consolidation ruling, measured end to end
/// (2026-07-31).
///
/// Eight objects, adjacent in GPGA and adjacent in the host reservation, all with the same
/// key. The window must end up with **one** memslot, and the report must carry both numbers
/// so the ratio is a measurement rather than a claim.
#[test]
fn eight_adjacent_objects_with_one_key_consolidate_into_a_single_memslot() {
    let (m, slots) = machine();
    let p = common::page();
    let mut ix = ViewerIndex::new();
    let viewer = ix.add_view(ViewerKind::Window(FbWindow::InstanceWindow));

    let mut table = Table::default();
    for i in 0..8u64 {
        let id = alloc(&mut ix, i * p, p, 1);
        table.at.insert(id.0, i * p);
    }
    ix.map_into_view(
        viewer,
        GpgaRegion::new(Aperture::Vidmem, 0, 8 * p).expect("well-formed"),
        0,
    )
    .expect("a fresh view over unowned-by-anyone-else objects");

    let mut inst = ViewInstaller::new(
        FbWindow::InstanceWindow,
        viewer,
        win_gpa(),
        win_len(),
        HostPageSize::query(),
    );
    let r = inst
        .drain_and_install(&mut ix, &table, &m)
        .expect("every object is backed by ordinary host memory");

    assert_eq!(r.objects, 8, "all eight objects are placed");
    assert_eq!(
        r.mappings_before_merge, 8,
        "eight objects would have been eight mappings without consolidation"
    );
    assert_eq!(
        r.mappings,
        1,
        "eight contiguous objects sharing one key are ONE mapping; got the layout {:?}",
        inst.layout().covered
    );
    // ...and the run really is the whole eight, not one object with seven dropped.
    let covered = &inst.layout().covered;
    assert_eq!(covered.len(), 1);
    assert_eq!(
        covered[0].len,
        8 * p,
        "the merged run spans all eight objects"
    );
    assert_eq!(covered[0].objects.len(), 8, "and it names all eight");

    // The kernel was told exactly that.
    let live = slots.live();
    assert_eq!(
        live.len(),
        1,
        "the mock kernel holds one memslot for eight objects, not eight: {live:?}"
    );
    assert_eq!(live[0].gpa, win_gpa());
    assert_eq!(live[0].len, 8 * p);
    assert!(!live[0].readonly, "a passthrough tier is a read-write slot");
    assert_eq!(
        slots.replaces(),
        0,
        "a silent same-number replace is never acceptable"
    );
}

/// ★★★ **The bite for the clause above.** Two objects that are adjacent in the view but sit
/// at *different* owners must stay two mappings — driven through the real index, not through
/// the pure helper, so the key is proven to reach the installer (2026-07-31).
#[test]
fn two_owners_side_by_side_stay_two_mappings_however_adjacent_they_are() {
    let (m, slots) = machine();
    let p = common::page();
    let mut ix = ViewerIndex::new();
    // ★ A WINDOW viewer, because a window is exempt from the foreign-object refusal by
    // design — which is exactly why the merge key has to carry the owner: the index will
    // happily show one window two isolates' objects, and it is our merge that must not
    // flatten them.
    let viewer = ix.add_view(ViewerKind::Window(FbWindow::InstanceWindow));

    let mut table = Table::default();
    let a = alloc(&mut ix, 0, p, 1);
    let b = alloc(&mut ix, p, p, 2); // adjacent in GPGA, DIFFERENT owner
    table.at.insert(a.0, 0);
    table.at.insert(b.0, p); // adjacent in the host reservation too

    ix.map_into_view(
        viewer,
        GpgaRegion::new(Aperture::Vidmem, 0, 2 * p).expect("well-formed"),
        0,
    )
    .expect("a window may see both");

    let mut inst = ViewInstaller::new(
        FbWindow::InstanceWindow,
        viewer,
        win_gpa(),
        win_len(),
        HostPageSize::query(),
    );
    let r = inst
        .drain_and_install(&mut ix, &table, &m)
        .expect("both are backed");

    assert_eq!(r.mappings_before_merge, 2);
    assert_eq!(
        r.mappings, 2,
        "two isolates' objects merged into one mapping; adjacency is not a licence"
    );
    let covered = &inst.layout().covered;
    assert_eq!(covered.len(), 2);
    // ★ Assert the REASON, not just the count: the two runs differ in the key's owner field
    // and are identical everywhere else, which is what makes this a merge-key test.
    assert_ne!(covered[0].key.owner, covered[1].key.owner);
    assert_eq!(covered[0].key.cache, covered[1].key.cache);
    assert_eq!(covered[0].key.viewers, covered[1].key.viewers);
    assert_eq!(
        covered[0].view_off + covered[0].len,
        covered[1].view_off,
        "they really are adjacent, so nothing but the key kept them apart"
    );
    assert_eq!(
        covered[0].host_off + covered[0].len,
        covered[1].host_off,
        "and adjacent in the host backing too"
    );
    // ★★★ **The distinction that a first draft of this file got wrong, kept as the lesson.**
    // The two runs stay two MAPPINGS — that is the key doing its job. They are nonetheless
    // served by ONE memslot, because they are adjacent and neither is an observe hole, so
    // in guest-physical terms they are one contiguous backed range. That collapse is safe
    // and it is not the key being ignored: a memslot answers "backed or trapping", while
    // *who owns these bytes* is decided by the placements the mappings become, and those
    // are what stayed apart.
    assert_eq!(
        slots.live().len(),
        1,
        "two adjacent native runs are one contiguous backed range: {:?}",
        slots.live()
    );
    assert_eq!(
        r.memslots, 1,
        "and the report says so rather than reporting the mapping count"
    );
    assert_ne!(
        r.mappings, r.memslots,
        "this is the case where the two numbers differ, which is the whole point of \
         reporting both"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────
// The refusals
// ─────────────────────────────────────────────────────────────────────────────────────

/// ⊘ **THE FINDING, as a named refusal.** An object whose bytes are the real device's
/// framebuffer needs a descriptor only an isolate can mint and a verb that does not exist.
/// It is refused by name, and — the part that matters — **nothing is installed**.
#[test]
fn an_object_backed_by_the_real_device_is_refused_by_name_and_installs_nothing() {
    let (m, slots) = machine();
    let p = common::page();
    let mut ix = ViewerIndex::new();
    let viewer = ix.add_view(ViewerKind::Window(FbWindow::FbAperture));

    let mut table = Table::default();
    let a = alloc(&mut ix, 0, p, 1);
    let b = alloc(&mut ix, p, p, 1);
    table.at.insert(a.0, 0);
    table.on_device.insert(b.0); // this one's bytes are on the card

    ix.map_into_view(
        viewer,
        GpgaRegion::new(Aperture::Vidmem, 0, 2 * p).expect("well-formed"),
        0,
    )
    .expect("fresh view");

    let mut inst = ViewInstaller::new(
        FbWindow::FbAperture,
        viewer,
        win_gpa(),
        win_len(),
        HostPageSize::query(),
    );
    let err = inst
        .drain_and_install(&mut ix, &table, &m)
        .expect_err("a device-backed object cannot be mapped by this port");
    assert_eq!(
        err,
        InstallRefusal::HostGpuBackingHasNoVerb {
            object: b,
            aperture: Aperture::Vidmem,
        },
        "the refusal must name the object and its aperture, not merely fail"
    );
    // ★★ And the whole drain stopped: a window half real and half absent is a state neither
    // side can reason about, so the first object's mapping must NOT be there either.
    assert_eq!(
        slots.live().len(),
        0,
        "a refused drain installed a memslot anyway: {:?}",
        slots.live()
    );
    assert_eq!(slots.installs(), 0);
}

/// ★★ **Unvouchable content gets no memslot.** The object is still mapped — the index says
/// so — but its bytes arrived by a transport this port cannot observe, so making them native
/// is the one thing miss = fault forbids. It becomes an observe run: **no slot**, so the
/// guest traps.
#[test]
fn content_this_port_cannot_vouch_for_is_demoted_out_of_the_native_tier() {
    let (m, slots) = machine();
    let p = common::page();
    let mut ix = ViewerIndex::new();
    let viewer = ix.add_view(ViewerKind::Window(FbWindow::InstanceWindow));

    let mut table = Table::default();
    for i in 0..3u64 {
        let id = alloc(&mut ix, i * p, p, 1);
        table.at.insert(id.0, i * p);
    }
    ix.map_into_view(
        viewer,
        GpgaRegion::new(Aperture::Vidmem, 0, 3 * p).expect("well-formed"),
        0,
    )
    .expect("fresh view");

    let mut inst = ViewInstaller::new(
        FbWindow::InstanceWindow,
        viewer,
        win_gpa(),
        win_len(),
        HostPageSize::query(),
    );
    let before = inst
        .drain_and_install(&mut ix, &table, &m)
        .expect("all backed");
    assert_eq!(before.mappings, 1, "three adjacent objects, one mapping");

    // Now a copy-engine write lands on the middle one, through a transport nothing observes.
    let change = ObjectChange::UnwitnessedWrite {
        region: GpgaRegion::new(Aperture::Vidmem, p, p).expect("well-formed"),
        transport: UnwitnessedTransport::FbToFbCopyEngine,
    };
    let plan = ix.plan(&change).expect("planning an unwitnessed write");
    ix.apply(&plan).expect("fresh plan");

    let after = inst
        .drain_and_install(&mut ix, &table, &m)
        .expect("still backed");
    assert!(
        after.unwitnessed.iter().any(|r| matches!(
            r,
            InstallRefusal::UnwitnessedRunNotMadeNative {
                transport: UnwitnessedTransport::FbToFbCopyEngine,
                ..
            }
        )),
        "the demotion must be reported by name, not silently applied: {:?}",
        after.unwitnessed
    );
    assert_eq!(
        after.mappings, 2,
        "the unvouchable object split the run in two and took no mapping of its own"
    );
    // ★ The decisive assertion: the middle page is NOT in the native tier.
    assert_eq!(tier_at(inst.layout(), p), Tier::Observe);
    assert_eq!(tier_at(inst.layout(), 0), Tier::Passthrough);
    assert_eq!(tier_at(inst.layout(), 2 * p), Tier::Passthrough);
    let live = slots.live();
    assert_eq!(
        live.len(),
        2,
        "two slots with a hole between them: {live:?}"
    );
    assert_eq!(
        live[0].gpa + live[0].len,
        win_gpa() + p,
        "the first slot stops where the unvouchable page begins"
    );
    assert_eq!(
        live[1].gpa,
        win_gpa() + 2 * p,
        "and the second starts after it — the hole is exactly one page"
    );
}

/// ★★ The slot budget is a **named** refusal carrying both numbers.
#[test]
fn a_layout_needing_more_mappings_than_the_budget_is_refused_with_both_numbers() {
    let (m, _slots) = machine();
    let p = common::page();
    let mut ix = ViewerIndex::new();
    let viewer = ix.add_view(ViewerKind::Window(FbWindow::InstanceWindow));

    // Deliberately non-consolidatable: every object sits at a host offset that is NOT
    // adjacent to its neighbour, so the merge cannot help and the count is the object count.
    let n = u64::from(OUR_SLOT_BUDGET) + 1;
    let mut table = Table::default();
    for i in 0..n {
        let id = alloc(&mut ix, i * p, p, 1);
        table.at.insert(id.0, (n - i) * p * 4); // scattered, descending
    }
    ix.map_into_view(
        viewer,
        GpgaRegion::new(Aperture::Vidmem, 0, n * p).expect("well-formed"),
        0,
    )
    .expect("fresh view");

    let mut inst = ViewInstaller::new(
        FbWindow::InstanceWindow,
        viewer,
        win_gpa(),
        (n + 1) * p,
        HostPageSize::query(),
    );
    let err = inst
        .drain_and_install(&mut ix, &table, &m)
        .expect_err("more runs than the budget");
    assert_eq!(
        err,
        InstallRefusal::SlotBudgetWouldBeExceeded {
            needed: usize::try_from(n).expect("small"),
            budget: OUR_SLOT_BUDGET,
        },
        "the refusal must carry BOTH numbers; 'too many' without the ceiling is not actionable"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────
// Revocation, idempotence, and the pull
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★★ A drain that changes nothing installs nothing. A memslot reinstall tears down
/// everything the guest was using, so doing it on an unchanged layout is not merely wasteful.
#[test]
fn a_drain_with_no_change_does_not_touch_the_kernel() {
    let (m, slots) = machine();
    let p = common::page();
    let mut ix = ViewerIndex::new();
    let viewer = ix.add_view(ViewerKind::Window(FbWindow::Pramin));
    let mut table = Table::default();
    let id = alloc(&mut ix, 0, 2 * p, 1);
    table.at.insert(id.0, 0);
    ix.map_into_view(
        viewer,
        GpgaRegion::new(Aperture::Vidmem, 0, 2 * p).expect("well-formed"),
        0,
    )
    .expect("fresh view");

    let mut inst = ViewInstaller::new(
        FbWindow::Pramin,
        viewer,
        win_gpa(),
        win_len(),
        HostPageSize::query(),
    );
    let first = inst.drain_and_install(&mut ix, &table, &m).expect("backed");
    assert!(first.reinstalled, "the first drain must install");
    let installs = slots.installs();
    assert_eq!(installs, 1);

    let second = inst
        .drain_and_install(&mut ix, &table, &m)
        .expect("nothing changed");
    assert_eq!(second.updates_drained, 0, "the queue was already empty");
    assert!(
        !second.reinstalled,
        "an unchanged layout reinstalled the window; that is a teardown of everything the \
         guest was using, for nothing"
    );
    assert_eq!(
        slots.installs(),
        installs,
        "the kernel was asked to install a slot for an unchanged layout"
    );
    assert_eq!(slots.clears(), 0, "and nothing was torn down");
}

/// ★★ A revocation removes the coverage from the page tables, not merely from a queue.
#[test]
fn a_freed_object_loses_its_memslot() {
    let (m, slots) = machine();
    let p = common::page();
    let mut ix = ViewerIndex::new();
    let viewer = ix.add_view(ViewerKind::Window(FbWindow::InstanceWindow));
    let mut table = Table::default();
    let a = alloc(&mut ix, 0, p, 1);
    let b = alloc(&mut ix, p, p, 1);
    table.at.insert(a.0, 0);
    table.at.insert(b.0, p);
    ix.map_into_view(
        viewer,
        GpgaRegion::new(Aperture::Vidmem, 0, 2 * p).expect("well-formed"),
        0,
    )
    .expect("fresh view");

    let mut inst = ViewInstaller::new(
        FbWindow::InstanceWindow,
        viewer,
        win_gpa(),
        win_len(),
        HostPageSize::query(),
    );
    inst.drain_and_install(&mut ix, &table, &m).expect("backed");
    assert_eq!(slots.live()[0].len, 2 * p, "one merged run of two pages");

    let plan = ix
        .plan(&ObjectChange::Freed { object: b })
        .expect("b is live");
    ix.apply(&plan).expect("fresh plan");
    let r = inst
        .drain_and_install(&mut ix, &table, &m)
        .expect("a still backed");

    assert_eq!(r.objects, 1, "one object left");
    assert!(
        r.reinstalled,
        "the coverage shrank, so the window is reinstalled"
    );
    let live = slots.live();
    assert_eq!(live.len(), 1);
    assert_eq!(
        live[0].len, p,
        "the freed object's page is no longer served natively; it is {live:?}"
    );
    assert_eq!(tier_at(inst.layout(), p), Tier::Observe);
}

/// ★★ **DRAIN is a pull, and a viewer that never drains delays nobody.**
///
/// Two windows over the same objects. One installer drains; the other never does. The
/// draining one must reach a complete, correct layout regardless — and the index must have
/// committed the change either way, which is what makes the hanging viewer harmless.
#[test]
fn a_viewer_that_never_drains_does_not_hold_up_the_one_that_does() {
    let (m, slots) = machine();
    let p = common::page();
    let mut ix = ViewerIndex::new();
    let draining = ix.add_view(ViewerKind::Window(FbWindow::InstanceWindow));
    let hanging = ix.add_view(ViewerKind::Window(FbWindow::Pramin));

    let mut table = Table::default();
    for i in 0..4u64 {
        let id = alloc(&mut ix, i * p, p, 1);
        table.at.insert(id.0, i * p);
    }
    let region = GpgaRegion::new(Aperture::Vidmem, 0, 4 * p).expect("well-formed");
    ix.map_into_view(draining, region, 0).expect("fresh view");
    ix.map_into_view(hanging, region, 0).expect("fresh view");

    let mut inst = ViewInstaller::new(
        FbWindow::InstanceWindow,
        draining,
        win_gpa(),
        win_len(),
        HostPageSize::query(),
    );
    let r = inst.drain_and_install(&mut ix, &table, &m).expect("backed");

    // ★ The hanging viewer's queue is NOT empty — it really did not drain — and that changed
    // nothing for the one that did.
    assert!(
        ix.pending_len(hanging).expect("registered") > 0,
        "the hanging viewer must actually be holding undelivered updates, or this test is \
         a test of two draining viewers"
    );
    assert_eq!(r.objects, 4);
    assert_eq!(
        r.mappings, 1,
        "and the draining window got its full, merged layout"
    );
    assert_eq!(slots.live().len(), 1);

    // ★★ The two viewers see the same GPGA, so the merge key's viewer set carries BOTH of
    // them — which is the field that would silently disappear if `viewers_of` were not
    // consulted.
    assert_eq!(
        inst.layout().covered[0].key.viewers.len(),
        2,
        "both viewers cover this GPGA, so the key must say so: {:?}",
        inst.layout().covered[0].key
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────
// Geometry — page size, huge pages, alignment
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★★★ **Huge pages, and the three conditions — one of which has no error path anywhere.**
#[test]
fn a_huge_page_needs_length_alignment_and_congruence_and_the_third_is_the_silent_one() {
    // All three hold.
    assert_eq!(
        huge_pages_for(HUGE_PAGE_BYTES, HUGE_PAGE_BYTES, HUGE_PAGE_BYTES),
        HugePages::Reachable
    );
    // ...congruence, not equality: the host offset may differ by a multiple of 2 MiB.
    assert_eq!(
        huge_pages_for(HUGE_PAGE_BYTES, 4 * HUGE_PAGE_BYTES, HUGE_PAGE_BYTES),
        HugePages::Reachable
    );
    // Too short — PRAMIN's entire case.
    assert!(matches!(
        huge_pages_for(HUGE_PAGE_BYTES, HUGE_PAGE_BYTES, HUGE_PAGE_BYTES - 1),
        HugePages::OutOfReach(_)
    ));
    // Misaligned guest-physical base.
    assert!(matches!(
        huge_pages_for(
            HUGE_PAGE_BYTES + 0x1000,
            HUGE_PAGE_BYTES + 0x1000,
            4 * HUGE_PAGE_BYTES
        ),
        HugePages::OutOfReach(_)
    ));
    // ★ Aligned, long enough, and NOT congruent — installs perfectly, is never promoted,
    // and nothing anywhere reports it.
    assert!(matches!(
        huge_pages_for(HUGE_PAGE_BYTES, 0x1000, 4 * HUGE_PAGE_BYTES),
        HugePages::OutOfReach(_)
    ));
}

/// ★★ **PRAMIN can never reach a huge page**, and the census says so with numbers rather
/// than with a sentence. The window is 1 MiB; a 2 MiB entry does not fit in it at any
/// alignment, ever.
#[test]
fn pramins_one_mebibyte_window_puts_huge_pages_permanently_out_of_reach() {
    const PRAMIN_LEN: u64 = 1 << 20;
    // ⊘ Not asserted as an inequality: both sides are constants, so the compiler folds it
    // and the assertion carries no information. What CAN fail, and is asserted below, is the
    // census the two constants produce.
    let layout = kayfabe_vmm_qemu::viewer_install::ViewLayout {
        covered: vec![run(0, PRAMIN_LEN, 0, key(1, &[0]))],
        observe: vec![],
        coalesced_from: 1,
    };
    // Even at the friendliest possible alignment — base zero, host offset zero — it is out
    // of reach, because length alone forbids it.
    let c = census_of(&layout, 0);
    assert_eq!(
        c,
        AlignmentCensus {
            huge_aligned: 1,
            congruent: 1,
            long_enough: 0,
            huge_reachable: 0,
            runs: 1,
        },
        "PRAMIN is aligned and congruent and STILL cannot have a large entry"
    );
}

/// ★★ The census is a distribution with a denominator, and both of its interesting cells are
/// reachable — otherwise it is a counter that only ever prints zero.
#[test]
fn the_alignment_census_reports_both_reachable_and_unreachable_runs() {
    let k = key(1, &[0]);
    let layout = kayfabe_vmm_qemu::viewer_install::ViewLayout {
        covered: vec![
            // Reachable: long, aligned, congruent.
            run(0, 4 * HUGE_PAGE_BYTES, 0, k.clone()),
            // Long and aligned but NOT congruent — the silent one.
            run(8 * HUGE_PAGE_BYTES, 4 * HUGE_PAGE_BYTES, 0x1000, k.clone()),
            // Short.
            run(16 * HUGE_PAGE_BYTES, 0x1000, 16 * HUGE_PAGE_BYTES, k),
        ],
        observe: vec![],
        coalesced_from: 3,
    };
    let c = census_of(&layout, 0);
    assert_eq!(c.runs, 3, "the denominator");
    assert_eq!(c.huge_aligned, 3, "all three bases are 2 MiB aligned");
    assert_eq!(c.long_enough, 2);
    assert_eq!(
        c.congruent, 2,
        "the second run's host offset breaks congruence"
    );
    assert_eq!(
        c.huge_reachable, 1,
        "only the run where all three hold can actually be promoted"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────
// The memory type
// ─────────────────────────────────────────────────────────────────────────────────────

/// ★★★ Every window states its host memory type **and its reason**, and the reasons are
/// three different reasons rather than one copied three times.
#[test]
fn each_window_names_its_own_memory_type_and_why() {
    let ws = [
        FbWindow::Pramin,
        FbWindow::FbAperture,
        FbWindow::InstanceWindow,
    ];
    let mut reasons = std::collections::BTreeSet::new();
    for w in ws {
        let c = cacheability_of(w);
        assert_eq!(
            c.host,
            CachePolicy::WriteBack,
            "{}: every backing this port can mint is ordinary memory, which is write-back \
             and nothing else",
            w.name()
        );
        assert!(!c.because.is_empty());
        reasons.insert(c.because);
    }
    assert_eq!(
        reasons.len(),
        3,
        "the three windows must give three reasons; one reason copied three times is one \
         window's argument applied to three"
    );
}

/// ★★★ **The effective type is read back, and an unreadable instrument is not a pass.**
///
/// ⚠ This is the assertion that distinguishes *requesting* write-back from *getting* it.
/// It runs against the address of nothing at all, where the kernel's answer is *unknown* —
/// and the point is that unknown does not silently become "fine".
#[test]
fn an_effective_memory_type_that_cannot_be_read_is_not_reported_as_holding() {
    let inst = ViewInstaller::new(
        FbWindow::Pramin,
        kayfabe_mmu::gpga::ViewerId(0),
        win_gpa(),
        win_len(),
        HostPageSize::query(),
    );
    // A host-physical address in nothing: not System RAM, not reserved by anybody.
    let m = inst
        .assert_effective_cacheability(0xFFFF_FF00_0000_0000, 0x1000)
        .expect("an unknown type is not a refusal");
    if let Some(m) = m {
        assert!(
            !m.holds(),
            "an address the kernel says nothing about was reported as holding write-back"
        );
    }
    // And ordinary memory, which every host has, does hold.
    let ram = first_system_ram();
    if let Some(base) = ram {
        let m = inst
            .assert_effective_cacheability(base, 0x1000)
            .expect("ordinary memory is not a downgrade")
            .expect("the instrument answered");
        assert!(
            m.holds(),
            "ordinary memory did not satisfy a write-back requirement: {m:?}"
        );
    }
}

fn first_system_ram() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/iomem").ok()?;
    for line in text.lines() {
        let (range, name) = line.trim_start().split_once(" : ")?;
        if name == "System RAM" {
            return u64::from_str_radix(range.split('-').next()?, 16).ok();
        }
    }
    None
}
