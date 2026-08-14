//! ★★ **A backing may be a SLICE of a host object, not only a whole one**
//! (`gpga_address_space.md` §8.2 / §9.3).
//!
//! ## The state this file is about
//!
//! Reservation (§8) works by **owning and sub-allocating**: we pre-occupy GPU RAM as
//! *arenas* and hand slices of them to guest processes, so the guest's own accounting of
//! its GPU memory is true — the vGPU property. That requires **many bindings sharing one
//! host object at different offsets**, which the old `HostBacking { memory, host_va }`
//! could not say. Adding an offset was never the interesting part; deciding **who frees
//! the object** was, and that is what [`kayfabe_mmu::HostExtent`] makes an exhaustively
//! matched question instead of a convention.
//!
//! ## The bug shape being designed out
//!
//! With one object per binding, "reclaim the binding" and "free the object" are the same
//! act, and every reclaim site in the tree was written that way. Put two bindings on one
//! object and that code **frees a live object on the first release**, leaving the sibling
//! binding mapped to freed memory — a use-after-free the guest can then read, and under a
//! shared arena a cross-*process* one (§9.2's reuse leak, arrived at from the other
//! direction). The two reclaim sites are `kayfabe_fwd::unpublish_backing` and
//! `Gpu::stage_dropped_vases`, and this file drives both.
//!
//! ## ★ What is deliberately NOT claimed here
//!
//! **No slice release frees the arena — not the first and not the last.** That is the
//! design, not an omission: the arena is owned by the reservation allocator (§8.1(a)),
//! which is not built, and a refcount held by nobody is not ownership. Until that owner
//! exists the arena is held to the end of the test and **declared** as residue, so the
//! day it lands the declaration must be deleted or the suite says so. Every test below
//! therefore asserts the strongest *true* statement — the arena survives every slice cut
//! from it, and is never queued for release twice — rather than a refcount nothing
//! implements.
//!
//! ## Non-vacuity
//!
//! Each guard was induced to fail before being trusted; the exact failure text is in the
//! doc comment of the test that owns it (`suspect_the_instrument_first`).

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped handle/VA literals

use kayfabe_arch::Aperture;
use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb};
use kayfabe_core::ProcId;
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::rmgraph::RmEvent;
use kayfabe_fwd::{
    FwdFault, Published, commit_publish, plan_publish, publish_backing, unpublish_backing,
};
use kayfabe_isolate::{HostHandle, IsolateId, VerbPlan, VerbReply};
use kayfabe_mmu::{AddressFault, BackingBytes, Binding, HostBacking, HostExtent, HostSlice};
use kayfabe_mocks::{MockArch, MockIsolateFactory, SharedRecorder, VerbKind};
use kayfabe_tests::{Guarded, ResidueClaim, Scenario, identical_handles};

const GPU: GpuId = GpuId::ZERO;
const PDB: Pdb = Pdb(0x3401_000);
const CLIENT: HClient = HClient(0xAA);
/// Two guest ranges that will end up backed by ONE arena object at two offsets.
const VA_A: GpuVa = GpuVa(0x2_0020_0000);
const VA_B: GpuVa = GpuVa(0x2_0030_0000);
const LEN: u64 = 0x10000;

/// One compute process, its `Proc` routed, and the recorder behind its isolate.
fn one_process_gpu() -> (Guarded<Gpu>, ProcId, SharedRecorder, HObject) {
    let arch = Box::new(MockArch::new());
    let (factory, recorder) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");
    let handles = identical_handles(0x10, 0x11);
    let vaspace = handles.vaspace;
    let mut s = Scenario::new();
    s.compute_process(CLIENT, PDB, handles);
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies cleanly");
    }
    let pid = *gpu.spine.by_pdb.get(&(GPU, PDB)).expect("routed");
    (
        Guarded::new("arena_slice_backing", gpu, recorder.clone()),
        pid,
        recorder,
        vaspace,
    )
}

/// ★ **Build the state the product cannot yet reach**, honestly and in one place.
///
/// Nothing mints an arena slice today — `VerbReply::Published` carries no offset, and
/// that reply lives on the isolate seam this task may not edit. So the fixture publishes
/// two ranges the ordinary way and then **re-expresses** both bindings as slices of the
/// FIRST publication's object, which becomes the arena. That keeps every handle a real
/// one the mock actually minted (a fabricated handle would read as `dangling` residue and
/// prove less), and it leaves both GPA tokens in `Vas::blocks`, so
/// [`unpublish_backing`] is exercised for real rather than simulated.
///
/// `B`'s own object is freed on a worker as part of the setup — it is genuinely surplus
/// once `B` points at the arena, and leaving it would be a leak this file did not mean
/// to test. Returns the arena handle.
fn two_slices_of_one_arena(gpu: &mut Gpu, pid: ProcId) -> HostHandle {
    let a = publish_backing(gpu.procs.get_mut(&pid).unwrap(), GPU, PDB, VA_A, LEN)
        .expect("A publishes");
    let b = publish_backing(gpu.procs.get_mut(&pid).unwrap(), GPU, PDB, VA_B, LEN)
        .expect("B publishes");
    let arena = a.memory;

    let proc = gpu.procs.get_mut(&pid).unwrap();
    // Surplus once B is re-pointed: free it through the ordinary port so the ledger
    // records the disposal instead of the harness declaring one.
    {
        let mut w = proc
            .isolate_mut(GPU)
            .expect("materialized")
            .checkout()
            .expect("idle worker");
        assert_eq!(
            w.execute(&VerbPlan::Release {
                unmap: vec![],
                free: vec![b.memory],
                guest_ram: Vec::new(),
            }, &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb")),
            Ok(VerbReply::Released)
        );
        proc.isolate_mut(GPU).expect("isolate").checkin(w);
    }

    let vas = proc.vases.get_mut(&(GPU, PDB)).expect("the Vas");
    for (va, off) in [(VA_A, 0), (VA_B, LEN)] {
        let (len, old) = vas.table.unbind(va).expect("published above");
        assert_eq!(len, LEN);
        vas.table
            .bind(
                PDB,
                va,
                LEN,
                Binding::real_gpu_memory(
                    old.phys(),
                    old.aperture(),
                    HostBacking::slice(
                        arena,
                        old.host_va().expect("published"),
                        HostSlice::new(off, LEN).expect("a real range"),
                        BackingBytes::SoleBacking,
                    ),
                )
                .expect("a slice of host sysmem is kind 3"),
            )
            .expect("a slice binds at the VA it is mapped at");
    }
    arena
}

/// What one isolate's declared residue looks like when the arena is the only thing left.
fn arena_residue(pid: ProcId, objects: usize, maps: usize) -> ResidueClaim {
    ResidueClaim::on(
        IsolateId::new(pid.0, GPU),
        "the arena object outlives every slice cut from it, and §8.1(a)'s reservation \
         allocator — the thing that owns and frees an arena — is not built. Delete this \
         claim when it lands.",
    )
    .objects(VerbKind::AllocSysmem, objects)
    .maps(maps)
}

/// Run an `Orphans` release chain on the proc's own worker, as the T0 drain does.
fn release(gpu: &mut Gpu, pid: ProcId, orphans: &kayfabe_fwd::Orphans) {
    if orphans.is_empty() {
        return;
    }
    let proc = gpu.procs.get_mut(&pid).unwrap();
    let mut w = proc
        .isolate_mut(GPU)
        .expect("materialized")
        .checkout()
        .expect("idle worker");
    assert_eq!(w.execute(&orphans.release_plan(), &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb")), Ok(VerbReply::Released));
    proc.isolate_mut(GPU).expect("isolate").checkin(w);
}

// =================================================================================
// 1 — two bindings, one object, independent reclaim
// =================================================================================

/// ★★★ **Two slices of one arena reclaim independently, and NEITHER free frees the
/// arena** — not the first release and not the last.
///
/// This is the mean case the shape exists for. The unmap is per-binding and must happen
/// both times; the free is per-*object* and must happen neither time. Asserting only "the
/// first release does not free" would pass on a shape that frees on the last one by
/// accident, so both halves are named.
///
/// **Instrument check, performed 2026-07-30 — WATCHED IT FAIL.** With `if
/// h.frees_object()` removed from `kayfabe_fwd::unpublish_backing` (i.e. the pre-§8.2
/// unconditional `out.free.push(h.memory())`), this test fails on the FIRST release:
///
/// ```text
/// assertion `left == right` failed: ★ the arena is not freed by the slice that happened
///     to be released first — its siblings are still mapped into it
///   left: [HostHandle(iso1/gpu0:0x200000002)]
///  right: []
/// ```
///
/// Two sibling tests fail with it (3 of 6), and one of them shows the consequence rather
/// than the symptom — the mock refuses the *second* release of the same arena:
/// `Err(VerbFailure { err: BadHandle(HostHandle(iso1/gpu0:0x200000002)), orphans:
/// Orphans { unmap: [], free: [HostHandle(iso1/gpu0:0x200000002)] } })`. Restored.
#[test]
fn two_slices_of_one_arena_reclaim_independently_and_free_nothing() {
    let (mut gpu, pid, _rec, _vaspace) = one_process_gpu();
    let arena = two_slices_of_one_arena(&mut gpu, pid);
    let host_vas = gpu.procs[&pid].vases[&(GPU, PDB)]
        .host_vas
        .expect("materialized");

    let first = unpublish_backing(gpu.procs.get_mut(&pid).unwrap(), GPU, PDB, VA_A)
        .expect("the range is owed back");
    assert_eq!(
        first.unmap,
        vec![(host_vas, VA_A.0)],
        "the MAPPING is per-binding, so it is undone"
    );
    assert_eq!(
        first.free,
        vec![],
        "★ the arena is not freed by the slice that happened to be released first — \
         its siblings are still mapped into it"
    );
    release(&mut gpu, pid, &first);

    // The sibling is untouched and still resolves through the same arena.
    let (b, off) = kayfabe_fwd::resolve(&gpu, GPU, PDB, GpuVa(VA_B.0 + 0x20)).expect("resolves");
    assert_eq!(off, 0x20);
    let backing = b.host().expect("still published");
    assert_eq!(backing.memory(), arena);
    assert_eq!(
        backing.extent(),
        HostExtent::Slice(HostSlice::new(LEN, LEN).expect("real")),
        "…at ITS offset, not the released one's"
    );

    let last = unpublish_backing(gpu.procs.get_mut(&pid).unwrap(), GPU, PDB, VA_B)
        .expect("the range is owed back");
    assert_eq!(last.unmap, vec![(host_vas, VA_B.0)]);
    assert_eq!(
        last.free,
        vec![],
        "★ …nor by the last one: a slice never owns the object, so the arena leaves \
         through its owner or not at all"
    );
    release(&mut gpu, pid, &last);

    // And the VA is gone from the table both times — MISS=FAULT, per binding.
    for va in [VA_A, VA_B] {
        assert_eq!(
            kayfabe_fwd::resolve(&gpu, GPU, PDB, va),
            Err(FwdFault::Address(AddressFault::Miss { pdb: PDB, va })),
        );
    }
    gpu.declare_residue(arena_residue(pid, 1, 0));
}

/// ★ The **non-vacuity twin**: the same call, on a whole-object backing, DOES free.
///
/// Without this the test above is satisfiable by a reclaim path that never frees
/// anything, which is a leak wearing the shape of a fix.
#[test]
fn a_whole_object_backing_is_still_freed_by_its_own_release() {
    let (mut gpu, pid, _rec, _vaspace) = one_process_gpu();
    let p: Published =
        publish_backing(gpu.procs.get_mut(&pid).unwrap(), GPU, PDB, VA_A, LEN).expect("publishes");
    let host_vas = gpu.procs[&pid].vases[&(GPU, PDB)]
        .host_vas
        .expect("materialized");

    let binding = kayfabe_fwd::resolve(&gpu, GPU, PDB, VA_A)
        .expect("resolves")
        .0;
    let backing = binding.host().expect("published");
    assert_eq!(
        backing.extent(),
        HostExtent::Whole,
        "the ordinary publish chain allocates one object per binding"
    );
    assert!(backing.frees_object());

    let orphans =
        unpublish_backing(gpu.procs.get_mut(&pid).unwrap(), GPU, PDB, VA_A).expect("owed back");
    assert_eq!(orphans.unmap, vec![(host_vas, VA_A.0)]);
    assert_eq!(
        orphans.free,
        vec![p.memory],
        "★ a sole owner's release IS what frees the object"
    );
    release(&mut gpu, pid, &orphans);
}

// =================================================================================
// 2 — overlapping slices
// =================================================================================

/// ★★ **Overlapping slices of one object behave**: two guest VAs aliasing the same arena
/// bytes is legitimate (it is what two mappings of one buffer are), so it must bind, must
/// resolve to each VA's own binding, and must still reclaim without freeing the arena.
///
/// The interesting half is that the overlap is *invisible* to the address table — the two
/// bindings occupy disjoint VA ranges, and the table keys on VA. Nothing here may quietly
/// deduplicate or refuse them on the strength of the shared object.
#[test]
fn overlapping_slices_of_one_object_bind_resolve_and_reclaim() {
    let (mut gpu, pid, _rec, _vaspace) = one_process_gpu();
    let arena = two_slices_of_one_arena(&mut gpu, pid);

    // Re-cut B so it OVERLAPS A: A is [0, LEN), B becomes [LEN/2, 3*LEN/2).
    let overlapping = HostSlice::new(LEN / 2, LEN).expect("a real range");
    {
        let vas = gpu
            .procs
            .get_mut(&pid)
            .unwrap()
            .vases
            .get_mut(&(GPU, PDB))
            .expect("the Vas");
        let (_len, old) = vas.table.unbind(VA_B).expect("bound above");
        vas.table
            .bind(
                PDB,
                VA_B,
                LEN,
                Binding::real_gpu_memory(
                    old.phys(),
                    old.aperture(),
                    HostBacking::slice(arena, VA_B.0, overlapping, BackingBytes::SoleBacking),
                )
                .expect("a slice of host sysmem is kind 3"),
            )
            .expect("an overlapping slice is a legitimate alias, not a collision");
    }

    let a = kayfabe_fwd::resolve(&gpu, GPU, PDB, VA_A)
        .expect("A resolves")
        .0
        .host()
        .expect("published");
    let b = kayfabe_fwd::resolve(&gpu, GPU, PDB, VA_B)
        .expect("B resolves")
        .0
        .host()
        .expect("published");
    assert_eq!((a.memory(), b.memory()), (arena, arena), "one object");
    assert_ne!(a.extent(), b.extent(), "…two different parts of it");
    let (sa, sb) = (
        a.as_slice().expect("a slice"),
        b.as_slice().expect("a slice"),
    );
    assert!(
        sa.offset() < sb.end() && sb.offset() < sa.end(),
        "the fixture really does overlap: {sa:?} vs {sb:?}"
    );

    // …and both still reclaim without touching the object.
    for va in [VA_A, VA_B] {
        let o =
            unpublish_backing(gpu.procs.get_mut(&pid).unwrap(), GPU, PDB, va).expect("owed back");
        assert_eq!(o.free, vec![], "an aliased arena is still nobody's to free");
        release(&mut gpu, pid, &o);
    }
    gpu.declare_residue(arena_residue(pid, 1, 0));
}

// =================================================================================
// 3 — the owner scope (§9.3)
// =================================================================================

/// ★★★ **A backing from another isolate's namespace is REFUSED where it would enter core
/// state**, and the foreign handle is not put on our own release list.
///
/// §9.3's requirement: RM grants *objects, not ranges*, so a handle to an arena is reach
/// over **all** of it. The scope tested is [`HostHandle`]'s own `IsolateId` — the same
/// one `Worker::execute`'s foreign-handle gate uses — inherited rather than duplicated.
///
/// **Instrument check, performed 2026-07-30 — WATCHED IT FAIL.** With the
/// `ForeignBacking` block short-circuited in `commit_publish`, the commit SUCCEEDS and
/// this test fails with:
///
/// ```text
/// ★ a backing minted in another isolate's namespace must not be adopted:
///     Published { gpa: 8590000128, host_va: 8593080320,
///                 memory: HostHandle(iso9/gpu0:0x5c00beef) }
/// ```
///
/// — i.e. without the gate a foreign object is silently adopted into this proc's table.
/// Restored.
///
/// ★ **The gate's POSITION is load-bearing and was corrected by an existing test.** It
/// first sat ahead of `commit_publish`'s R5 proc-identity guard, where it also caught a
/// commit applied to the *wrong proc* — and reported "foreign handle" about a plan/proc
/// mismatch, which is §12.10's wrong-reason conflation with the root cause hidden behind
/// a symptom of it. `l1_verb_seam.rs::commit_publish_and_doorbell_proc_guards_refuse_on_\
/// either_term_alone` failed and was right; the gate moved. That test keeps the ordering
/// pinned, so it is deliberately not duplicated here.
#[test]
fn a_backing_from_another_isolate_is_refused_and_not_freed_by_us() {
    let (mut gpu, pid, _rec, _vaspace) = one_process_gpu();
    // Materialize the host VAS through an ordinary publication first, so the plan below
    // is the steady-state one (no VAS allocation to confuse the refusal's orphans).
    let _ =
        publish_backing(gpu.procs.get_mut(&pid).unwrap(), GPU, PDB, VA_A, LEN).expect("publishes");
    let host_vas = gpu.procs[&pid].vases[&(GPU, PDB)]
        .host_vas
        .expect("materialized");

    let planned =
        plan_publish(gpu.procs.get_mut(&pid).unwrap(), GPU, PDB, VA_B, LEN).expect("plans");
    let foreign_iso = IsolateId::new(pid.0 + 8, GPU);
    let foreign = HostHandle::new(foreign_iso, 0x5c00_beef);

    let refusal = commit_publish(
        gpu.procs.get_mut(&pid).unwrap(),
        &planned.plan,
        Some(VerbReply::Published {
            host_vas: None,
            memory: foreign,
            host_va: VA_B.0,
        }),
    )
    .expect_err("★ a backing minted in another isolate's namespace must not be adopted");

    assert_eq!(
        refusal.fault,
        FwdFault::ForeignBacking {
            isolate: IsolateId::new(pid.0, GPU),
            memory: foreign,
        },
        "the refusal names the boundary and the handle that crossed it"
    );
    assert!(
        !refusal.retry,
        "re-running the same wrong reply is not a fix"
    );
    assert_eq!(
        refusal.orphans.free,
        vec![],
        "★ we have no standing to FREE another isolate's object — queueing it would ask \
         our own worker to reach across the boundary this refusal is about"
    );
    assert_eq!(
        refusal.orphans.unmap,
        vec![(host_vas, VA_B.0)],
        "…but the mapping in OUR host VAS is ours to undo"
    );
    assert_eq!(
        kayfabe_fwd::resolve(&gpu, GPU, PDB, VA_B),
        Err(FwdFault::Address(AddressFault::Miss { pdb: PDB, va: VA_B })),
        "the refusal is a refusal: nothing entered the table"
    );

    // …and the gate is not a blanket one: the same commit with a home-minted handle is
    // adopted. (Without this the test above passes on a `commit_publish` that refuses
    // everything.)
    let ours = HostHandle::new(IsolateId::new(pid.0, GPU), 0x5c00_0bad);
    let published = commit_publish(
        gpu.procs.get_mut(&pid).unwrap(),
        &planned.plan,
        Some(VerbReply::Published {
            host_vas: None,
            memory: ours,
            host_va: VA_B.0,
        }),
    )
    .expect("our own namespace's object is adopted");
    assert_eq!(published.memory, ours);

    // VA_A's publication is ordinary and stays accounted. VA_B's backing is a hand-built
    // reply whose handle the mock never minted, so core state names an object and a
    // mapping the ledger has no record of — the `dangling` class, declared rather than
    // freed: freeing a handle the ledger never issued is the `FREE OF UNKNOWN`
    // corruption class, which no claim may excuse.
    gpu.declare_residue(
        ResidueClaim::on(
            IsolateId::new(pid.0, GPU),
            "VA_B's backing is a hand-built commit reply whose handle the mock never \
             minted — a harness bypass, because no host verb corresponds to it.",
        )
        .dangling(1, 1),
    );
}

// =================================================================================
// 4 — teardown: the double free the old shape would have committed
// =================================================================================

/// ★★★ **A `Vas` dropped with two slices of one arena stages TWO unmaps and ZERO frees of
/// the arena.**
///
/// `Gpu::stage_dropped_vases` walks every binding of a dying `Vas` and queues its host
/// state. Unconditionally queueing `h.memory()` was correct while one object backed one
/// binding; with an arena it queues **the same live handle once per slice**, which is a
/// double free of an object other Vases may still be mapped into. The mock's ledger calls
/// that corruption, and no `ResidueClaim` can excuse it — which is what makes this test's
/// negative direction sharp.
///
/// **Instrument check, performed 2026-07-30 — WATCHED IT FAIL.** With `if
/// h.frees_object()` removed from `Gpu::stage_dropped_vases`, this fails with:
///
/// ```text
/// assertion `left == right` failed: ★ the arena is queued ZERO times, not once per slice
///   left: [HostHandle(iso1/gpu0:0x200000002), HostHandle(iso1/gpu0:0x200000002),
///          HostHandle(iso1/gpu0:0x200000001)]
///  right: [HostHandle(iso1/gpu0:0x200000001)]
/// ```
///
/// — the arena handle appearing **twice** in one release chain, ahead of the host VAS.
/// Restored.
#[test]
fn dropping_a_vas_full_of_slices_queues_the_arena_zero_times_not_once_per_slice() {
    let (mut gpu, pid, _rec, vaspace) = one_process_gpu();
    let arena = two_slices_of_one_arena(&mut gpu, pid);
    let host_vas = gpu.procs[&pid].vases[&(GPU, PDB)]
        .host_vas
        .expect("materialized");

    // The guest frees the VASpace: the Vas dies with both slices still bound.
    gpu.apply(RmEvent::Free {
        client: CLIENT,
        handle: vaspace,
    })
    .expect("the guest may free its own object");

    let staged: Vec<_> = gpu.procs[&pid].staged_releases().collect();
    let (_g, q) = staged.first().expect("the drop staged a release");
    assert_eq!(
        q.unmap,
        vec![(host_vas, VA_A.0), (host_vas, VA_B.0)],
        "every binding's MAPPING is undone — one per slice"
    );
    assert_eq!(
        q.free,
        vec![host_vas],
        "★ the arena is queued ZERO times, not once per slice"
    );
    assert!(
        !q.free.contains(&arena),
        "…and specifically not the arena, which other Vases may still hold slices of"
    );

    // Drain it, so the assertion above is about a chain that actually runs.
    let (worker, orphans) = gpu
        .procs
        .get_mut(&pid)
        .unwrap()
        .checkout_with_pending_release(GPU);
    let mut w = worker.expect("an idle worker");
    assert_eq!(w.execute(&orphans.release_plan(), &kayfabe_util::trapwitness::OffTrap::claim("a test / adapter host verb")), Ok(VerbReply::Released));
    gpu.procs
        .get_mut(&pid)
        .unwrap()
        .isolate_mut(GPU)
        .expect("isolate")
        .checkin(w);

    gpu.declare_residue(arena_residue(pid, 1, 0));
}

// =================================================================================
// 5 — the entrance law on the slice coordinates
// =================================================================================

/// ★★ A slice whose length is not its range's is refused **at the table's entrance**, and
/// the refusal is not a blanket one.
///
/// The unit-level proof lives in `kayfabe-mmu` (it can reach the private map); this is
/// the same law seen from where a real `Vas` lives, so a future populate path that
/// bypasses `bind` is visible from the outside too.
#[test]
fn a_slice_that_disagrees_with_its_range_never_enters_a_live_vas() {
    let (mut gpu, pid, _rec, _vaspace) = one_process_gpu();
    let arena = two_slices_of_one_arena(&mut gpu, pid);
    let vas = gpu
        .procs
        .get_mut(&pid)
        .unwrap()
        .vases
        .get_mut(&(GPU, PDB))
        .expect("the Vas");
    assert_eq!(
        vas.table.audit_identity(PDB),
        Ok(()),
        "the fixture is clean"
    );

    let (_len, old) = vas.table.unbind(VA_A).expect("bound");
    assert_eq!(
        vas.table.bind(
            PDB,
            VA_A,
            LEN,
            Binding::real_gpu_memory(
                old.phys(),
                old.aperture(),
                HostBacking::slice(
                    arena,
                    VA_A.0,
                    HostSlice::new(0, LEN / 2).expect("real"),
                    BackingBytes::SoleBacking,
                )
            )
            .expect("a slice of host sysmem is kind 3"),
        ),
        Err(AddressFault::SliceLenMismatch {
            pdb: PDB,
            va: VA_A,
            len: LEN,
            slice_len: LEN / 2,
        }),
    );
    assert!(
        vas.table.resolve(PDB, VA_A).is_err(),
        "the refusal is a refusal: nothing entered the table"
    );
    // The honest form binds, so the guard is not rejecting every slice.
    vas.table
        .bind(
            PDB,
            VA_A,
            LEN,
            Binding::real_gpu_memory(
                old.phys(),
                old.aperture(),
                HostBacking::slice(
                    arena,
                    VA_A.0,
                    HostSlice::new(0, LEN).expect("real"),
                    BackingBytes::SoleBacking,
                ),
            )
            .expect("a slice of host sysmem is kind 3"),
        )
        .expect("an honest slice binds");
    assert_eq!(vas.table.audit_identity(PDB), Ok(()));

    // Aperture is untouched by any of this; assert it so a future `..old` refactor that
    // dropped it would be visible here rather than in a copy engine.
    assert_eq!(old.aperture(), Aperture::SysmemCoherent);

    for va in [VA_A, VA_B] {
        let o =
            unpublish_backing(gpu.procs.get_mut(&pid).unwrap(), GPU, PDB, va).expect("owed back");
        release(&mut gpu, pid, &o);
    }
    gpu.declare_residue(arena_residue(pid, 1, 0));
}
