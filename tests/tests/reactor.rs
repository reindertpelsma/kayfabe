//! ★ The completion-source reactor, driven through the REAL core lifecycle
//! (`l1_concurrency.md` §6, decision #37).
//!
//! The unit properties of the model (handle non-reuse, typed dispatch, MISS=FAULT,
//! proc-scoped deregistration, order-independence, wake requests) are pinned in
//! `kayfabe-core/src/reactor.rs`'s own tests. What can only be pinned HERE is the
//! **integration** one: that the registry is wired into the real proc-retire path,
//! so a source signalled after its proc died is a loud `SourceFault` rather than a
//! route onto a dead proc — the C's F4 use-after-retire species.
//!
//! Note what these tests do NOT need: no threads, no reactor loop, no host readiness
//! machinery of any kind. Signalling a source is `registry.dispatch(source)`. That is
//! the §6.3 payoff — the completion flow is testable as a scripted order, with zero
//! timing dependence.

// PDB literals group by the hardware field (page-aligned), not by nibble quartets —
// same convention as the rest of the suite.
#![allow(clippy::unusual_byte_groupings)]

use kayfabe_arch::ids::{GpuId, HClient, HObject, Pdb};
use kayfabe_completion::OsEventRef;
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::reactor::{Dispatch, SourceFault, SourceKind};
use kayfabe_core::rmgraph::RmEvent;
use kayfabe_isolate::WorkerId;
use kayfabe_mocks::{MockArch, MockIsolateFactory};
use kayfabe_tests::{Scenario, identical_handles};

const A_PDB: Pdb = Pdb(0x1234_000);
const B_PDB: Pdb = Pdb(0x5678_000);
const CLIENT_ROOT: HObject = HObject(0x5c00_0000);

fn fresh_gpu() -> Gpu {
    let arch = Box::new(MockArch::new());
    let (factory, _rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x11_0000_0000, 0x1_0000_0000);
    Gpu::new(arch, Box::new(factory), gpa).expect("device realizes")
}

/// Build one compute proc on physical GPU `instance` and return its `ProcId`.
fn build_proc(gpu: &mut Gpu, client: HClient, pdb: Pdb, instance: u32) -> kayfabe_core::ProcId {
    let mut s = Scenario::new();
    s.compute_process_on_gpu(client, pdb, identical_handles(0x10, 0x11), Some(instance));
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies");
    }
    *gpu.spine
        .by_pdb
        .get(&(GpuId(instance), pdb))
        .expect("routed by PDB")
}

/// ★ **The retire integration** (the F4 species, killed at the model level): arm
/// completion sources for a proc across BOTH of its GPU targets, then retire the proc
/// through the real core path (the guest frees its client root). Every one of its
/// sources must stop resolving *immediately* — a late signal is a loud `SourceFault`,
/// and no dispatch anywhere in the registry can still name the dead proc.
///
/// The other proc's sources — including one carrying the SAME numeric os-event and
/// worker ids on the same GPUs — survive untouched: the routing key is the whole
/// identity tuple, never a numeric coincidence.
#[test]
fn reactor_sources_of_a_retired_proc_fault_never_use_after_retire() {
    let mut gpu = fresh_gpu();
    let pid_a = build_proc(&mut gpu, HClient(0xAA), A_PDB, 0);
    let pid_b = build_proc(&mut gpu, HClient(0xBB), B_PDB, 1);
    assert_ne!(pid_a, pid_b);

    // Arm A on BOTH targets (a proc spanning GPUs has a separate isolate/completion
    // path per target, MG-5/MG-6) and B with IDENTICAL numeric ids.
    let mut arm = |kind| {
        let (s, wake) = gpu.spine.sources.register(kind);
        wake.latched();
        s
    };
    let a_ev0 = arm(SourceKind::OsEvent {
        proc: pid_a,
        gpu: GpuId::ZERO,
        ev: OsEventRef(0x77),
    });
    let a_ev1 = arm(SourceKind::OsEvent {
        proc: pid_a,
        gpu: GpuId(1),
        ev: OsEventRef(0x77),
    });
    let a_worker = arm(SourceKind::Worker {
        proc: pid_a,
        gpu: GpuId(1),
        worker: WorkerId(0),
    });
    let seam = arm(SourceKind::CrossIsolate {
        from: pid_b,
        to: pid_a,
    });
    let b_ev0 = arm(SourceKind::OsEvent {
        proc: pid_b,
        gpu: GpuId::ZERO,
        ev: OsEventRef(0x77),
    });
    let b_worker = arm(SourceKind::Worker {
        proc: pid_b,
        gpu: GpuId(1),
        worker: WorkerId(0),
    });
    let notify = arm(SourceKind::Notify);
    assert_eq!(gpu.spine.sources.len(), 7);

    // Everything routes while A is alive.
    assert_eq!(
        gpu.spine.sources.dispatch(a_ev0),
        Ok(Dispatch::Observe {
            proc: pid_a,
            gpu: GpuId::ZERO,
            ev: OsEventRef(0x77)
        })
    );
    let _ = gpu.spine.sources.take_pending_wake();

    // ---- The guest tears process A down: its client root is freed. ----
    gpu.apply(RmEvent::Free {
        client: HClient(0xAA),
        handle: CLIENT_ROOT,
    })
    .expect("teardown applies");
    assert!(!gpu.procs.contains_key(&pid_a), "A retired");
    assert_eq!(gpu.spine.retired.len(), 1, "staged for the deferred reap");

    // Every source of A — on BOTH GPUs, plus the cross-isolate seam it was one end of
    // — now resolves to nothing. This is the signal arriving after the retire.
    for stale in [a_ev0, a_ev1, a_worker, seam] {
        assert_eq!(
            gpu.spine.sources.dispatch(stale),
            Err(SourceFault { source: stale }),
            "{stale:?} must fault, not route onto the retired proc",
        );
        assert_eq!(
            gpu.spine.sources.kind(stale),
            Err(SourceFault { source: stale })
        );
    }

    // No dispatch reachable from the registry can name the retired proc.
    for (source, _) in gpu.spine.sources.iter() {
        match gpu.spine.sources.dispatch(source).expect("registered") {
            Dispatch::Observe { proc, .. } | Dispatch::WorkerDied { proc, .. } => {
                assert_ne!(proc, pid_a, "a live source still names the retired proc");
            }
            Dispatch::CrossSignal { from, to } => {
                assert!(from != pid_a && to != pid_a, "a live seam still names A");
            }
            Dispatch::Wake => {}
        }
    }

    // B and the device-global notifiable source are untouched.
    assert_eq!(
        gpu.spine.sources.dispatch(b_ev0),
        Ok(Dispatch::Observe {
            proc: pid_b,
            gpu: GpuId::ZERO,
            ev: OsEventRef(0x77)
        })
    );
    assert_eq!(
        gpu.spine.sources.dispatch(b_worker),
        Ok(Dispatch::WorkerDied {
            proc: pid_b,
            gpu: GpuId(1),
            worker: WorkerId(0)
        })
    );
    assert_eq!(gpu.spine.sources.dispatch(notify), Ok(Dispatch::Wake));
    assert_eq!(gpu.spine.sources.len(), 3);

    // The retire changed the source set, so the join must be told to re-read it.
    assert!(
        gpu.spine.sources.take_pending_wake().is_some(),
        "proc retire is a source-set change and must request a wake"
    );

    // The deferred reap (L10) changes nothing about the routing: retire already did it.
    assert_eq!(gpu.reap_retired().len(), 1);
    assert_eq!(gpu.spine.sources.len(), 3);
    assert_eq!(
        gpu.spine.sources.dispatch(a_ev0),
        Err(SourceFault { source: a_ev0 })
    );
}

/// The stale handle cannot be resurrected by proc-id reuse either: after A retires and
/// is reaped, a NEW proc built with A's exact client/PDB/handles gets a fresh `ProcId`
/// and fresh source handles, and A's old handles stay dead. (Handle non-reuse is the
/// unit property; this pins that the *core lifecycle* cannot smuggle around it.)
#[test]
fn reactor_stale_source_survives_a_full_teardown_rebuild_cycle() {
    let mut gpu = fresh_gpu();
    let pid_a = build_proc(&mut gpu, HClient(0xAA), A_PDB, 0);
    let (stale, wake) = gpu.spine.sources.register(SourceKind::OsEvent {
        proc: pid_a,
        gpu: GpuId::ZERO,
        ev: OsEventRef(0x99),
    });
    wake.latched();

    gpu.apply(RmEvent::Free {
        client: HClient(0xAA),
        handle: CLIENT_ROOT,
    })
    .expect("teardown applies");
    assert_eq!(gpu.reap_retired().len(), 1);

    // Rebuild with IDENTICAL client, PDB and handles.
    let pid_a2 = build_proc(&mut gpu, HClient(0xAA), A_PDB, 0);
    let (fresh, wake) = gpu.spine.sources.register(SourceKind::OsEvent {
        proc: pid_a2,
        gpu: GpuId::ZERO,
        ev: OsEventRef(0x99),
    });
    wake.latched();

    assert_ne!(stale, fresh, "the mint never recycles a handle");
    assert_eq!(
        gpu.spine.sources.dispatch(stale),
        Err(SourceFault { source: stale }),
        "the pre-teardown handle must stay permanently unresolvable",
    );
    assert!(gpu.spine.sources.dispatch(fresh).is_ok());
}
