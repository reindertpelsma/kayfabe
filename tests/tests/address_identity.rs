//! ★★★ `#102` — **ADDRESS IDENTITY**: a host-published range is addressable at the guest
//! VA the guest named, or it is not published at all.
//!
//! ## The bug this file exists for
//!
//! A forwarded pushbuffer names *guest* virtual addresses. Nothing rewrites them — that is
//! the point of forwarding rather than emulating. So when the host GPU runs that ring, its
//! MMU walks the **host** address space looking for exactly those numbers. If our mapping
//! is somewhere else, the submission is accepted, looks published in every piece of core
//! state, and dies inside the copy engine as `Xid 31 FAULT_PDE`.
//!
//! Before this change the mapping port had **no address parameter at all**
//! (`map_gpu_va(vas, memory, len)`) while `unmap_gpu_va(vas, gpu_va)` had one — the
//! asymmetry was the tell — and the real backend sent `flags: 0, dma_offset: 0`, i.e. *"put
//! it wherever you like and tell me where"*. The fix is the C's own primitive:
//! `DMA_OFFSET_FIXED_TRUE` (bit 15, `0x8000`), which makes `dmaOffset` an **[IN]**
//! parameter (`C: nvkvm_gpu_emul.c:7663-7692`, *"the irreducible primitive the whole data
//! plane rests on"*; `ogkm-580: src/common/sdk/nvidia/inc/nvos.h:2094-2096`).
//!
//! ## ★★ What this does NOT weaken
//!
//! #14. Two guest processes' identical guest VAs now land at the **same host VA** — inside
//! **different host VASes**, on **different isolates**, over **different backing**.
//! Per-address-*space* separation is #14's proven fix; per-*address* separation never was,
//! and the suite asserting it (`sim_14_two_process.rs`, and six other sites) encoded a
//! wrong reading. Those assertions were corrected, not removed.
//!
//! ## The three guards, and the proof each one can fire
//!
//! Every guard here was **induced to fail before being trusted**
//! (`suspect_the_instrument_first`; six controls that could never fire have been found in
//! this repo). The lever is `RmRecorder::placement_drift`, which scripts a backend into
//! ignoring the fixed-offset request — the one thing that models a host, or a port, that
//! does not honour it.
//!
//! | guard | where | induced by |
//! |---|---|---|
//! | `RmError::PlacementRefused` | `Worker::execute`, at the backend seam | `placement_drift` |
//! | `AddressFault::HostVaMismatch` | `AddressTable::bind`, the table's only entrance | a hand-built `Binding` |
//! | `AddressTable::audit_identity` | a whole-table walk | a direct private-map write, from inside `kayfabe-mmu` (see below) |

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped handle/VA literals

use kayfabe_arch::Aperture;
use kayfabe_arch::ids::{GpuId, GpuVa, HClient, Pdb};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_fwd::{FwdFault, publish_backing, resolve, unpublish_backing};
use kayfabe_isolate::{IsolateFactory, RmError, VerbPlan, VerbReply};
use kayfabe_mmu::{AddressFault, AddressTable, BackingBytes, Binding, HostBacking};
use kayfabe_mocks::{MockArch, MockIsolateFactory, RmVerb, SharedRecorder};
use kayfabe_tests::{Guarded, Scenario, identical_handles};

const GPU: GpuId = GpuId::ZERO;
const A_PDB: Pdb = Pdb(0x3401_000);
const B_PDB: Pdb = Pdb(0x3405_000);
/// The identical guest VA both processes use (#14's working-set base).
const SHARED_VA: GpuVa = GpuVa(0x2_0020_0000);

fn two_process_gpu() -> (Guarded<Gpu>, SharedRecorder) {
    let arch = Box::new(MockArch::new());
    let (factory, recorder) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");
    let mut s = Scenario::new();
    s.compute_process(HClient(0xAA), A_PDB, identical_handles(0x10, 0x11));
    s.compute_process(HClient(0xBB), B_PDB, identical_handles(0x20, 0x21));
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies cleanly");
    }
    (
        Guarded::new("address_identity", gpu, recorder.clone()),
        recorder,
    )
}

// =================================================================================
// 1 — the positive law, through the ports
// =================================================================================

/// ★★★ Every host-published range is mapped **at the VA it is bound at**, and the host
/// verb log agrees: the `MapGpuVa` the backend actually saw named that same address.
///
/// The verb-log half is the non-vacuous one. Asserting `binding.host_va() == va` alone
/// would pass even if the core wrote the right number into its own table and asked the
/// host for something else entirely — which is precisely the shape of the bug (core state
/// that looks published, host state that is not there).
#[test]
fn a_published_range_is_mapped_at_the_guest_va_and_the_host_verb_says_so() {
    let (mut gpu, rec) = two_process_gpu();
    let pid = *gpu.spine.by_pdb.get(&(GPU, A_PDB)).expect("routed");

    // Three ranges, deliberately not in address order, so a bump-allocating backend
    // could not accidentally satisfy this.
    let vas = [
        GpuVa(SHARED_VA.0 + 0x40_0000),
        SHARED_VA,
        GpuVa(SHARED_VA.0 + 0x10_0000),
    ];
    for va in vas {
        let p = publish_backing(gpu.procs.get_mut(&pid).unwrap(), GPU, A_PDB, va, 0x10000)
            .expect("publishes");
        assert_eq!(p.host_va, va.0, "published somewhere other than {va:?}");
        let (b, off) = resolve(&gpu, GPU, A_PDB, GpuVa(va.0 + 0x1234)).expect("resolves");
        assert_eq!((b.host_va(), off), (Some(va.0), 0x1234));
    }

    // The HOST saw the same three addresses — not "three maps happened".
    let asked: Vec<u64> = rec
        .lock()
        .expect("recorder")
        .log
        .iter()
        .filter_map(|(_, v)| match v {
            RmVerb::MapGpuVa { va, .. } => Some(*va),
            _ => None,
        })
        .collect();
    assert_eq!(
        asked,
        vas.iter().map(|v| v.0).collect::<Vec<_>>(),
        "the host was asked to map at exactly the guest's addresses, in order"
    );

    // And the law holds over a walk of the whole table, not just the ranges above.
    gpu.procs[&pid].vases[&(GPU, A_PDB)]
        .table
        .audit_identity(A_PDB)
        .expect("clean");
}

/// ★★ The #14 arrangement, stated positively: identical guest VAs, **same** host VA,
/// **different** host VASes and host objects. This is the assertion that replaced
/// `assert_ne!(host_va_a, host_va_b)` across the suite, gathered in one place with the
/// reasoning attached.
#[test]
fn identical_guest_vas_share_a_host_va_and_share_nothing_else() {
    let (mut gpu, _rec) = two_process_gpu();
    let pid_a = *gpu.spine.by_pdb.get(&(GPU, A_PDB)).expect("A routed");
    let pid_b = *gpu.spine.by_pdb.get(&(GPU, B_PDB)).expect("B routed");

    let pa = publish_backing(
        gpu.procs.get_mut(&pid_a).unwrap(),
        GPU,
        A_PDB,
        SHARED_VA,
        0x10000,
    )
    .expect("A publishes");
    let pb = publish_backing(
        gpu.procs.get_mut(&pid_b).unwrap(),
        GPU,
        B_PDB,
        SHARED_VA,
        0x10000,
    )
    .expect("B publishes at the SAME guest VA");

    assert_eq!(
        (pa.host_va, pb.host_va),
        (SHARED_VA.0, SHARED_VA.0),
        "★ identical guest VAs are host-mapped at the SAME host VA — by design"
    );
    assert_ne!(pa.gpa, pb.gpa, "…over disjoint GPA arenas");
    assert_ne!(pa.memory, pb.memory, "…over disjoint host memory objects");
    assert_ne!(
        pa.memory.isolate(),
        pb.memory.isolate(),
        "…minted by different isolates (boundary 2)"
    );
    assert_ne!(
        gpu.procs[&pid_a].vases[&(GPU, A_PDB)].host_vas,
        gpu.procs[&pid_b].vases[&(GPU, B_PDB)].host_vas,
        "…and mapped into different host VASes — THE #14 separation"
    );
}

/// ★ The collision that must still be loud: **one** proc, the same VA twice. Under
/// address identity this is refused in the PLAN, before a single host verb exists — so
/// nothing is allocated and nothing needs unwinding — and it becomes legal again after an
/// eager unbind. (Before `#102` the plan could not know, because the host chose the
/// address; now it can, and it must, or the refusal arrives from the driver as a bare
/// `NoMemory` after the allocations.)
#[test]
fn the_same_va_twice_in_one_vas_is_a_loud_overlap_with_no_host_work() {
    let (mut gpu, rec) = two_process_gpu();
    let pid = *gpu.spine.by_pdb.get(&(GPU, A_PDB)).expect("routed");

    publish_backing(
        gpu.procs.get_mut(&pid).unwrap(),
        GPU,
        A_PDB,
        SHARED_VA,
        0x10000,
    )
    .expect("first publish");
    let verbs_before = rec.lock().expect("recorder").log.len();

    assert_eq!(
        publish_backing(
            gpu.procs.get_mut(&pid).unwrap(),
            GPU,
            A_PDB,
            SHARED_VA,
            0x10000
        ),
        Err(FwdFault::Address(AddressFault::Overlap {
            pdb: A_PDB,
            va: SHARED_VA
        })),
        "a re-publication is a loud overlap in OUR vocabulary, not the driver's"
    );
    assert_eq!(
        rec.lock().expect("recorder").log.len(),
        verbs_before,
        "…refused in the plan: ZERO host verbs ran, so there is nothing to orphan"
    );

    // Unmap eager, then rebind: legal, and back at the same address.
    //
    // ★ The release has to actually RUN. `unpublish_backing` only hands back the host
    // objects; until the caller executes that release plan the host VAS is still occupied
    // at this address, and under `#102` the rebind is then refused by the driver rather
    // than silently relocated. That is the fixed-offset contract working — and it is a
    // new coupling: before address identity, a caller that dropped its `Orphans` leaked
    // quietly and every subsequent map still succeeded somewhere else.
    let orphans = unpublish_backing(gpu.procs.get_mut(&pid).unwrap(), GPU, A_PDB, SHARED_VA)
        .expect("the range is owed back");
    {
        let proc = gpu.procs.get_mut(&pid).unwrap();
        let mut w = proc
            .isolate_mut(GPU)
            .expect("materialized isolate")
            .checkout()
            .expect("a free worker");
        assert_eq!(w.execute(&orphans.release_plan()), Ok(VerbReply::Released));
        proc.isolate_mut(GPU).expect("isolate").checkin(w);
    }
    let again = publish_backing(
        gpu.procs.get_mut(&pid).unwrap(),
        GPU,
        A_PDB,
        SHARED_VA,
        0x10000,
    )
    .expect("rebind after an eager unbind");
    assert_eq!(again.host_va, SHARED_VA.0);
}

// =================================================================================
// 2 — the guards, each PROVEN to fire
// =================================================================================

/// ★★★ **`RmError::PlacementRefused` fires**, and the failed publication leaves no host
/// state behind.
///
/// The induced condition is a backend that ignores the fixed-offset request and places
/// the mapping 4 KiB away — a real, reachable host condition (an RM that does not honour
/// `DMA_OFFSET_FIXED_TRUE`, or a port that forgot to set it, which is exactly what
/// `flags: 0` was). Without the check in `Worker::execute` this drift would be adopted
/// silently: the core would record `host_va = at + 0x1000`, every core-state assertion
/// would pass, and the failure would surface on hardware as `Xid 31`.
///
/// **Instrument check (`suspect_the_instrument_first`), performed 2026-07-30:** with the
/// `if host_va != at.0` block deleted from `Worker::execute`, this test fails on the
/// `expect_err` (the drifted publication is accepted and `host_va` comes back as
/// `SHARED_VA + 0x1000`). Restored. The guard is live.
#[test]
fn a_backend_that_ignores_the_placement_request_is_refused_and_unwound() {
    let (mut gpu, rec) = two_process_gpu();
    let pid = *gpu.spine.by_pdb.get(&(GPU, A_PDB)).expect("routed");

    rec.lock().expect("recorder").placement_drift = Some(0x1000);
    let err = publish_backing(
        gpu.procs.get_mut(&pid).unwrap(),
        GPU,
        A_PDB,
        SHARED_VA,
        0x10000,
    )
    .expect_err("a mapping we cannot address must never be adopted");
    assert_eq!(
        err,
        FwdFault::Rm(RmError::PlacementRefused {
            want: SHARED_VA.0,
            got: SHARED_VA.0 + 0x1000,
        }),
        "the refusal names both addresses — a bare failure would not say what went wrong"
    );

    // Nothing was bound: no half-published range survives the refusal.
    assert!(
        resolve(&gpu, GPU, A_PDB, SHARED_VA).is_err(),
        "a refused placement leaves the address plane untouched"
    );
    // …and the host state it minted was released, not leaked. The unmap uses the VA the
    // backend actually produced, which is the only one the host knows about.
    let log = rec.lock().expect("recorder");
    let unmapped: Vec<u64> = log
        .log
        .iter()
        .filter_map(|(_, v)| match v {
            RmVerb::UnmapGpuVa { va, .. } => Some(*va),
            _ => None,
        })
        .collect();
    assert_eq!(
        unmapped,
        vec![SHARED_VA.0 + 0x1000],
        "the drifted mapping was undone AT THE ADDRESS IT LANDED, not at the one we asked for"
    );
    assert!(
        log.ledger()
            .leaked_on(kayfabe_isolate::IsolateId::new(pid.0, GPU))
            .is_empty(),
        "the whole chain unwound: the isolate holds no live host objects"
    );
}

/// ★★ The same guard at the **plan/verb** seam directly, with no core around it — so the
/// refusal is a property of `Worker::execute` and not of anything `kayfabe-fwd` does
/// before or after it.
#[test]
fn the_worker_itself_refuses_a_drifted_placement() {
    let (factory, rec) = MockIsolateFactory::new();
    let mut iso = factory.spawn(kayfabe_isolate::IsolateId::new(7, GPU));
    let mut w = iso.checkout().expect("fresh pool");

    // Baseline: with no drift the same plan succeeds AND lands where asked. Without this
    // the negative case below could be passing for any reason at all.
    let ok = w
        .execute(&VerbPlan::Publish {
            host_vas: None,
            len: 0x1000,
            at: SHARED_VA,
        })
        .expect("the honest chain runs");
    let VerbReply::Published { host_va, .. } = ok else {
        panic!("wrong reply: {ok:?}")
    };
    assert_eq!(host_va, SHARED_VA.0);

    rec.lock().expect("recorder").placement_drift = Some(0x20_0000);
    let failure = w
        .execute(&VerbPlan::Publish {
            host_vas: None,
            len: 0x1000,
            at: GpuVa(SHARED_VA.0 + 0x100_0000),
        })
        .expect_err("a drifted placement is refused at the seam");
    assert_eq!(
        failure.err,
        RmError::PlacementRefused {
            want: SHARED_VA.0 + 0x100_0000,
            got: SHARED_VA.0 + 0x100_0000 + 0x20_0000,
        }
    );
    assert!(
        failure.orphans.is_empty(),
        "the worker ran its own unwind: nothing is handed back for the caller to dispose"
    );
}

/// ★★★ **`AddressFault::HostVaMismatch` fires**: the table's only entrance refuses a
/// binding that claims a host publication at some other address.
///
/// This is the belt to `PlacementRefused`'s braces, and it is not redundant — it catches
/// the shape regardless of *how* the wrong number got there (a future bulk-populate path,
/// a mis-threaded commit, a hand-built binding like this one), whereas the seam check
/// only sees what came back from a backend.
///
/// **Instrument check, performed 2026-07-30:** with the `if h.host_va != va.0` block
/// deleted from `AddressTable::bind`, this test fails on the first `assert_eq!` (the bind
/// returns `Ok(())` and the lying binding enters the table). Restored.
#[test]
fn a_binding_published_at_the_wrong_address_cannot_enter_the_table() {
    let mut t = AddressTable::new();
    let va = SHARED_VA;
    let lying = Binding {
        phys: 0x8000_0000,
        aperture: Aperture::SysmemCoherent,
        // one page off — the whole bug, in one argument
        host: Some(HostBacking::whole(
            kayfabe_isolate::HostHandle::new(kayfabe_isolate::IsolateId::new(1, GPU), 9),
            va.0 + 0x1000,
            BackingBytes::SoleBacking,
        )),
    };
    assert_eq!(
        t.bind(A_PDB, va, 0x10000, lying),
        Err(AddressFault::HostVaMismatch {
            pdb: A_PDB,
            va,
            host_va: va.0 + 0x1000
        }),
        "a host-backed binding must be published AT the VA it is bound at"
    );
    assert!(
        t.resolve(A_PDB, va).is_err(),
        "…and the refusal is a refusal: nothing entered the table"
    );

    // The honest form of the same binding is accepted, so the guard is not simply
    // rejecting everything (the failure mode a negative-only test cannot see).
    let honest = Binding {
        host: Some(HostBacking::whole(
            lying.host.expect("set above").memory(),
            va.0,
            BackingBytes::SoleBacking,
        )),
        ..lying
    };
    t.bind(A_PDB, va, 0x10000, honest)
        .expect("an honest binding binds");
    assert_eq!(
        t.resolve(A_PDB, va).expect("resolves").0.host_va(),
        Some(va.0)
    );

    // A binding with NO host backing is unconstrained by the law — it is a declaration,
    // not a publication, and there is nothing yet to be at the wrong address.
    let declared = Binding {
        phys: 0x9000_0000,
        aperture: Aperture::Vidmem,
        host: None,
    };
    t.bind(A_PDB, GpuVa(va.0 + 0x10000), 0x1000, declared)
        .expect("an unpublished declaration is legal");
}

/// ★ The **positive** half of the table walk at integration level: a table built
/// entirely through the real publish path audits clean, and an *unpublished* declaration
/// is not a violation (the law is about host backings, not about every binding).
///
/// ★★ The walk's **negative** half deliberately does not live here, and the reason is a
/// finding rather than an omission: `AddressTable::bind` is the map's only entrance and
/// it already refuses a mismatched binding, so **no caller of the public API can build a
/// table for `audit_identity` to fail on**. A control exercised only through a door that
/// cannot produce the state it looks for is a control that never fires. It is fired
/// instead from inside `kayfabe-mmu`'s own unit tests
/// (`taddr_audit_identity_catches_a_binding_that_bypassed_the_entrance`), which reach the
/// private map directly — which is precisely the future path the walk exists for.
#[test]
fn a_table_built_through_the_publish_path_audits_clean() {
    let (mut gpu, _rec) = two_process_gpu();
    let pid = *gpu.spine.by_pdb.get(&(GPU, A_PDB)).expect("routed");
    for k in 0..8u64 {
        publish_backing(
            gpu.procs.get_mut(&pid).unwrap(),
            GPU,
            A_PDB,
            GpuVa(SHARED_VA.0 + k * 0x10000),
            0x10000,
        )
        .expect("publishes");
    }
    let table = &gpu.procs[&pid].vases[&(GPU, A_PDB)].table;
    assert_eq!(
        table.iter().count(),
        8,
        "non-vacuity: the walk has work to do"
    );
    assert_eq!(table.audit_identity(A_PDB), Ok(()));
}
