//! ★★ `kayfabe-trace` under contact — the replay vocabulary driven by **real core
//! traffic**, not by hand-built events.
//!
//! `testing_doctrine.md` §3.1: integration over unit, mean over happy path. So the
//! vocabulary is exercised the way it will actually be used — a two-GPU, multi-process
//! world built through the mocks, driven through `Gpu::apply` / `publish_backing` /
//! `resolve` / `handle_doorbell` / the completion plane, with every plane transition
//! observed at the seam and emitted as a [`TraceEvent`]. What the tests then assert is
//! the three claims the crate makes:
//!
//! 1. **The vocabulary can express what the planes actually return** — every event is
//!    built from a real return value, so a shape the core produces and the vocabulary
//!    cannot say is a compile error in the observer, not an opinion.
//! 2. **The order is faithful** — one recorder, dense and strictly increasing, and the
//!    protocol facts survive: a bind precedes the doorbell that gated on it, a posted
//!    batch precedes its drain, the interleaved wire plane keeps "the tx header was
//!    written *before* the interrupt".
//! 3. **The instrument is not vacuous** — and this is the one a tracing crate has to
//!    work hardest at, because the failure mode is a sink that records nothing while
//!    every test passes. `a_sink_that_silently_drops_is_caught_by_the_counters` builds
//!    that exact broken sink and shows the counters catch it;
//!    `check_dense_order_bites_on_a_planted_gap` does the same for the order checker.
//!
//! ## ★ What is wired, and what is not
//!
//! The observer sits **at the seam** — it calls the real plane entry points and emits
//! from their real results. No `&mut Trace` argument was threaded through
//! `kayfabe-fwd`'s signatures, because that is ~30 call-site files of churn for no new
//! evidence *about the vocabulary*. What WAS wired into production code is the part that
//! cannot be tested from outside: the compiler-checked bridges
//! (`kayfabe_core::trace`, `kayfabe_fwd::trace`), so that a new `RmEvent`, `FwdFault`,
//! `RmGraphError` or `Stale` variant fails the build until the trace vocabulary names it.

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped handle/VA literals

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::thread;

use kayfabe_arch::Aperture;
use kayfabe_arch::ids::{ControlCmd, Gpa, GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_completion::OsEventRef;
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::rmgraph::RmEvent;
use kayfabe_fwd::{
    ControlRoute, FwdFault, forward_engine_object, handle_doorbell, publish_backing, resolve,
    route_control, route_pdb, signal_golden_capture,
};
use kayfabe_isolate::{HostHandle, Txn, VerbPlan};
use kayfabe_mmu::AddressFault;
use kayfabe_mocks::{MockArch, MockIsolateFactory, mock_ctrl};
use kayfabe_tests::{Guarded, Scenario, identical_handles};
use kayfabe_trace::{
    AsRmVerb, Bar, CompletionOp, Counters, Dispatched, EventKind, FaultTag, Faulted, IrqSpec,
    Outcome, ProcRef, Record, Recorder, Resolved, RouteKey, Routed, Seq, Trace, TraceEvent,
    TraceLog, TraceSink, VerbOutcome, VerbTag, Width, check_dense_order, diff,
};
use kayfabe_util::Instant;

// =================================================================================
// The world: two guest processes on two GPUs, identical guest handles (#14 shape).
// =================================================================================

const A_CLIENT: HClient = HClient(0xAA);
const B_CLIENT: HClient = HClient(0xBB);
const A_PDB: Pdb = Pdb(0x3401_000);
const B_PDB: Pdb = Pdb(0x3405_000);
const A_GR: VChid = VChid(0x10);
const A_CE: VChid = VChid(0x11);
const B_GR: VChid = VChid(0x20);
const B_CE: VChid = VChid(0x21);
/// The identical guest VA both processes use — #14's collision shape.
const SHARED_VA: GpuVa = GpuVa(0x2_0020_0000);
/// A VA nothing ever publishes: the ring-gate's negative probe.
const VA_NEVER: GpuVa = GpuVa(0x7000_0000_0000);
const H_DEVICE: HObject = HObject(0x5c00_0001);
const H_VASPACE: HObject = HObject(0x5c00_0010);
const MEM: HObject = HObject(0x6000_0000);
const GPU0: GpuId = GpuId::ZERO;
const GPU1: GpuId = GpuId(1);

/// Build the two-process, two-GPU device. A on GPU0, B on GPU1, byte-identical guest
/// handle values, distinct PDBs and vChids.
fn world() -> Guarded<Gpu> {
    let arch = Box::new(MockArch::new());
    let (factory, recorder) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x10_0000_0000..0x1000_0000_0000, 0x10_0000_0000);
    let mut gpu = Gpu::realize(arch, Box::new(factory), gpa, &[GPU0, GPU1])
        .expect("the two-GPU device realizes");

    let mut s = Scenario::new();
    s.compute_process_on_gpu(A_CLIENT, A_PDB, identical_handles(A_GR.0, A_CE.0), None);
    s.memory(A_CLIENT, H_DEVICE, MEM, 0x9_0000_0000);
    s.compute_process_on_gpu(B_CLIENT, B_PDB, identical_handles(B_GR.0, B_CE.0), Some(1));
    s.memory(B_CLIENT, H_DEVICE, MEM, 0x9_1000_0000);
    for ev in s.events {
        gpu.apply(ev).expect("the scenario applies cleanly");
    }
    // #177: `plan_doorbell` now refuses a channel the guest never scheduled via
    // `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE`; declare every channel scheduled so the
    // doorbells this world's callers ring reach their actual subject, not `NotScheduled`.
    kayfabe_tests::guest_schedules_every_channel(&mut gpu);
    Guarded::new("trace_replay::world", gpu, recorder)
}

// =================================================================================
// ★ The observer: every emit is derived from a REAL plane return value.
//
// This is what makes the vocabulary claim falsifiable rather than decorative. If a
// plane returned something the vocabulary could not carry, THIS file would not
// compile — which is the same discipline the `Faulted`/`AsRmVerb` bridges enforce
// one layer down.
// =================================================================================

/// Apply an RM event and trace it, whatever it answers.
fn tr_apply(gpu: &mut Gpu, tr: &mut Trace<'_>, target: GpuId, client: HClient, ev: RmEvent) {
    let verb = ev.as_rm_verb();
    let handle = match ev {
        RmEvent::Alloc { handle, .. } | RmEvent::Free { handle, .. } => handle,
        RmEvent::Dup { dst, .. } => dst.handle,
        RmEvent::SetPageDir { vaspace, .. }
        | RmEvent::MapMemoryDma { vaspace, .. }
        | RmEvent::Unmap { vaspace, .. } => vaspace,
    };
    let outcome = match gpu.apply(ev) {
        Ok(()) => Outcome::Ok,
        Err(e) => Outcome::Refused(e.fault_tag()),
    };
    tr.emit(|| TraceEvent::RmApply {
        gpu: Some(target),
        client,
        handle,
        verb,
        outcome,
    });
}

/// Route a PDB and trace the decision.
fn tr_route_pdb(gpu: &Gpu, tr: &mut Trace<'_>, target: GpuId, pdb: Pdb) -> Result<u32, FwdFault> {
    let out = route_pdb(&gpu.spine, target, pdb);
    let outcome = match &out {
        Ok(p) => Routed::To(ProcRef(p.0)),
        Err(e) => Routed::Refused(e.fault_tag()),
    };
    tr.emit(|| TraceEvent::Route {
        gpu: target,
        key: RouteKey::Pdb(pdb),
        outcome,
    });
    out.map(|p| p.0)
}

/// Publish backing for `va` and trace the bind.
fn tr_publish(
    gpu: &mut Gpu,
    tr: &mut Trace<'_>,
    target: GpuId,
    pdb: Pdb,
    va: GpuVa,
    len: u64,
) -> Result<u64, FwdFault> {
    let pid = route_pdb(&gpu.spine, target, pdb)?;
    let p = gpu.procs.get_mut(&pid).expect("a routed proc is live");
    let out = publish_backing(p, target, pdb, va, len);
    match &out {
        Ok(published) => {
            let (phys, host_va) = (published.gpa, published.host_va);
            tr.emit(|| TraceEvent::AddressBind {
                gpu: target,
                pdb,
                va,
                len,
                phys,
                aperture: Aperture::SysmemCoherent,
                outcome: Outcome::Ok,
            });
            Ok(host_va)
        }
        Err(e) => {
            let tag = e.fault_tag();
            tr.emit(|| TraceEvent::AddressBind {
                gpu: target,
                pdb,
                va,
                len,
                phys: 0,
                aperture: Aperture::SysmemCoherent,
                outcome: Outcome::Refused(tag),
            });
            Err(*e)
        }
    }
}

/// Resolve a VA through the address table and trace the answer, hit or fault.
fn tr_resolve(gpu: &Gpu, tr: &mut Trace<'_>, target: GpuId, pdb: Pdb, va: GpuVa) {
    let outcome = match resolve(gpu, target, pdb, va) {
        Ok((binding, offset)) => Resolved::Hit {
            offset,
            host: binding.host(),
        },
        // The plane's fault is an `FwdFault`; the address-plane arm is the one this
        // event can carry, and anything else is a routing failure, not a resolve.
        Err(FwdFault::Address(f)) => Resolved::Fault(f),
        Err(other) => panic!("resolve reported a non-address fault: {other:?}"),
    };
    tr.emit(|| TraceEvent::AddressResolve {
        gpu: target,
        pdb,
        va,
        outcome,
    });
}

/// Ring a doorbell with a working set and trace the dispatch.
fn tr_doorbell(gpu: &mut Gpu, tr: &mut Trace<'_>, target: GpuId, vchid: VChid, ws: &[GpuVa]) {
    let token = MockArch::token_for(vchid);
    let outcome = match handle_doorbell(gpu, target, token, ws) {
        Ok(o) => Dispatched::Rung {
            proc: ProcRef(o.proc.0),
            host_token: o.host_token,
            scheduled_now: o.scheduled_now,
        },
        Err(e) => Dispatched::Refused(e.fault_tag()),
    };
    tr.emit(|| TraceEvent::Doorbell {
        gpu: target,
        vchid,
        token,
        outcome,
    });
}

/// Poll the completion plane for one proc and trace what it posted.
fn tr_poll(gpu: &mut Gpu, tr: &mut Trace<'_>, target: GpuId, pdb: Pdb, now: u64) {
    let pid = route_pdb(&gpu.spine, target, pdb).expect("a live proc");
    // The core's own poll: this observer sits at the CORE seam, so it does not need the
    // `Vmm` the `kayfabe_fwd::poll_completions` composition raises the edge through.
    let batch = gpu.completion_poll(target, pid, Instant(now));
    let (posted, len) = match &batch {
        Some(b) => (Some(b.batch), b.events.len()),
        None => (None, 0),
    };
    tr.emit(|| TraceEvent::Completion {
        gpu: target,
        proc: ProcRef(pid.0),
        op: CompletionOp::Polled {
            posted,
            outstanding: len,
        },
    });
    if let Some(b) = batch {
        let (id, n) = (b.batch, b.events.len());
        tr.emit(|| TraceEvent::Completion {
            gpu: target,
            proc: ProcRef(pid.0),
            op: CompletionOp::Posted { batch: id, len: n },
        });
        gpu.completions_drained(target);
        tr.emit(|| TraceEvent::Completion {
            gpu: target,
            proc: ProcRef(pid.0),
            op: CompletionOp::Drained { batch: id },
        });
    }
}

/// Forward a Case-1 engine object and trace the control that rides on it.
fn tr_control(
    gpu: &mut Gpu,
    tr: &mut Trace<'_>,
    target: GpuId,
    pdb: Pdb,
    vchid: VChid,
    cmd: u32,
) -> HostHandle {
    let pid = route_pdb(&gpu.spine, target, pdb).expect("a live proc");
    let f = forward_engine_object(gpu, target, vchid, kayfabe_tests::COMPUTE_CLASS, &[])
        .expect("the Case-1 engine-object forward");
    let cmd = ControlCmd(cmd);
    let mut payload = [0u8; 16];
    let outcome = match route_control(gpu, target, pid, f.host_object, cmd, &mut payload) {
        Ok(ControlRoute::Forwarded | ControlRoute::AckOnly) => Outcome::Ok,
        Err(e) => Outcome::Refused(e.fault_tag()),
    };
    let engine = f.engine;
    tr.emit(|| TraceEvent::Control {
        gpu: target,
        cmd,
        engine: Some(engine),
        outcome,
    });
    f.host_object
}

/// ★ Run ONE host-verb chain on a checked-out pool worker and trace it — the isolate
/// plane, driven through the real door (`Worker::execute`, the only way to reach a host
/// verb) rather than reconstructed from a log afterwards, which is what makes the
/// `IsolateVerb` event's `(isolate, worker, txn)` triple a real observation.
fn tr_isolate_verb(
    gpu: &mut Gpu,
    tr: &mut Trace<'_>,
    target: GpuId,
    pdb: Pdb,
    obj: HostHandle,
    cmd: u32,
    txn: u64,
) {
    let pid = route_pdb(&gpu.spine, target, pdb).expect("a live proc");
    let proc = gpu.procs.get_mut(&pid).expect("a routed proc is live");
    let mut worker = kayfabe_fwd::checkout(proc, target)
        .expect("checkout")
        .expect("the pool has a free worker");
    worker.begin_txn(Txn(txn));
    let plan = VerbPlan::Control {
        obj,
        cmd: ControlCmd(cmd),
        payload: vec![0u8; 16],
    };
    let (isolate, wid, t) = (worker.isolate(), worker.id(), worker.txn());
    let outcome = match worker.execute(&plan) {
        Ok(_) => VerbOutcome::Ok,
        Err(f) => VerbOutcome::Failed(f.err),
    };
    kayfabe_fwd::checkin(proc, target, worker);
    tr.emit(|| TraceEvent::IsolateVerb {
        isolate,
        worker: wid,
        txn: t,
        verb: VerbTag::of(&plan),
        outcome,
    });
}

// =================================================================================
// The scripted run — one function, so two recorders can be handed the SAME script
// (the determinism differential) and so the mean run can call it per thread.
// =================================================================================

/// Drive a realistic per-process lifecycle: route → publish → read back → ring
/// (ungated, then gated on its own publication) → control → completions → RM churn.
/// Returns the number of publications it made.
fn lifecycle(gpu: &mut Gpu, tr: &mut Trace<'_>, target: GpuId, pdb: Pdb, gr: VChid, ce: VChid) {
    tr_route_pdb(gpu, tr, target, pdb).expect("the process routes by its PDB");
    tr_publish(gpu, tr, target, pdb, SHARED_VA, 0x10000).expect("publishes");
    tr_resolve(gpu, tr, target, pdb, GpuVa(SHARED_VA.0 + 0x40));
    tr_doorbell(gpu, tr, target, gr, &[]);
    tr_doorbell(gpu, tr, target, gr, &[SHARED_VA]);
    tr_doorbell(gpu, tr, target, ce, &[]);
    let obj = tr_control(gpu, tr, target, pdb, gr, mock_ctrl::FORWARDABLE.0);
    tr_control(gpu, tr, target, pdb, gr, mock_ctrl::PROMOTE_CTX.0);
    tr_isolate_verb(gpu, tr, target, pdb, obj, mock_ctrl::FORWARDABLE.0, 0x77);
    tr_poll(gpu, tr, target, pdb, 1);
}

/// The whole two-process script, from a fresh world, into `tr`.
fn scripted_run(tr: &mut Trace<'_>) -> Guarded<Gpu> {
    let mut gpu = world();
    lifecycle(&mut gpu, tr, GPU0, A_PDB, A_GR, A_CE);
    lifecycle(&mut gpu, tr, GPU1, B_PDB, B_GR, B_CE);
    // RM churn through the graph — a device WRITE traced as an apply.
    tr_apply(
        &mut gpu,
        tr,
        GPU0,
        A_CLIENT,
        RmEvent::MapMemoryDma {
            client: A_CLIENT,
            vaspace: H_VASPACE,
            memory: MEM,
            va: GpuVa(0x80_0000_0000),
            offset: 0,
            len: 0x1000,
        },
    );
    tr_apply(
        &mut gpu,
        tr,
        GPU0,
        A_CLIENT,
        RmEvent::Unmap {
            client: A_CLIENT,
            vaspace: H_VASPACE,
            va: GpuVa(0x80_0000_0000),
        },
    );
    gpu
}

/// The kinds this script drives. Anything outside it is honestly absent, and the
/// tests say which is which rather than checking "not empty".
const CORE_KINDS: [EventKind; 8] = [
    EventKind::RmApply,
    EventKind::Route,
    EventKind::AddressBind,
    EventKind::AddressResolve,
    EventKind::Doorbell,
    EventKind::Completion,
    EventKind::IsolateVerb,
    EventKind::Control,
];

/// The wire-plane kinds, plus the one core kind the script never drives.
const NOT_DRIVEN: [EventKind; 7] = [
    EventKind::MmioRead,
    EventKind::MmioWrite,
    EventKind::GuestRead,
    EventKind::GuestWrite,
    EventKind::IrqRaise,
    EventKind::Clock,
    EventKind::AddressUnbind,
];

// =================================================================================
// 1. The vocabulary carries a real run, in order
// =================================================================================

/// ★ THE integration case: a realistic two-process, two-GPU lifecycle, observed at every
/// plane seam, comes out as one densely-ordered stream whose protocol facts survive.
#[test]
fn a_two_process_lifecycle_traces_every_driven_plane_in_protocol_order() {
    let mut rec = Recorder::new(TraceLog::new());
    let gpu = scripted_run(&mut rec.trace());
    drop(gpu);

    // (a) One recorder ⇒ one total order, dense from zero. The EXACT `Ok`, not `is_ok`.
    assert_eq!(
        check_dense_order(rec.sink().records(), Seq(0)),
        Ok(()),
        "one recorder's stream must be dense and strictly increasing"
    );

    // (b) ★ Non-vacuity, stated as a bound on what the instrument SAW — and stated
    // exactly, so a plane that silently stopped emitting fails here rather than being
    // absorbed by some other plane's traffic (testing_doctrine.md §1 rule 2).
    let c: &Counters = rec.counters();
    for k in CORE_KINDS {
        assert!(
            c.of(k) > 0,
            "the script drives {k} but the trace has none — the instrument did not reach it"
        );
    }
    assert_eq!(
        c.silent_kinds(),
        NOT_DRIVEN.to_vec(),
        "exactly the undriven kinds are silent: an unexpected silence is a plane that \
         stopped emitting, and an unexpected NOISE is an event nobody asked for"
    );
    assert_eq!(
        c.total(),
        rec.sink().len() as u64,
        "the counters and the sink must agree — a disagreement is a dropping sink"
    );
    assert_eq!(c.wire_total(), 0, "this script drives no device wire plane");
    assert_eq!(c.core_total(), c.total());

    // (c) ★ The order is the payload: the bind for SHARED_VA precedes the doorbell that
    // gated on it. A stream that lost that fact cannot answer the #14 question at all.
    let recs = rec.sink().records();
    let bind_a = recs
        .iter()
        .position(|r| {
            matches!(&r.ev, TraceEvent::AddressBind { pdb, va, .. } if *pdb == A_PDB && *va == SHARED_VA)
        })
        .expect("A's publication was traced");
    let gated_a = recs
        .iter()
        .position(|r| {
            matches!(&r.ev,
                TraceEvent::Doorbell { gpu, vchid, outcome: Dispatched::Rung { .. }, .. }
                    if *gpu == GPU0 && *vchid == A_GR)
        })
        .expect("A's gated ring was traced");
    assert!(
        bind_a < gated_a,
        "the bind must precede the ring that gated on it (bind #{bind_a}, ring #{gated_a})"
    );

    // (d) Posted precedes drained, for the same batch id.
    let posted = recs.iter().position(|r| {
        matches!(
            &r.ev,
            TraceEvent::Completion {
                op: CompletionOp::Posted { .. },
                ..
            }
        )
    });
    if let Some(p) = posted {
        let batch = match &recs[p].ev {
            TraceEvent::Completion {
                op: CompletionOp::Posted { batch, .. },
                ..
            } => *batch,
            _ => unreachable!(),
        };
        let drained = recs
            .iter()
            .position(|r| {
                matches!(&r.ev, TraceEvent::Completion { op: CompletionOp::Drained { batch: b }, .. } if *b == batch)
            })
            .expect("a posted batch is drained");
        assert!(p < drained, "post must precede drain of the same batch");
    }

    // (e) The two processes' identical guest VA produced DIFFERENT backing in the trace —
    // the #14 property, visible in the projection alone.
    let mut phys = Vec::new();
    for r in recs {
        if let TraceEvent::AddressBind {
            va, phys: ph, pdb, ..
        } = &r.ev
            && *va == SHARED_VA
        {
            phys.push((*pdb, *ph));
        }
    }
    assert_eq!(phys.len(), 2, "both processes published the identical VA");
    assert_ne!(
        phys[0].1, phys[1].1,
        "identical guest VA in two VASes must trace to distinct backing"
    );
}

// =================================================================================
// 2. Determinism — the differential's precondition
// =================================================================================

/// The decoded projection is a deterministic function of the driven traffic: two
/// independent runs of the same script produce identical streams. This is what makes a
/// replay differential meaningful at all.
#[test]
fn the_same_script_twice_produces_an_identical_projection() {
    let mut a = Recorder::new(TraceLog::new());
    drop(scripted_run(&mut a.trace()));
    let mut b = Recorder::new(TraceLog::new());
    drop(scripted_run(&mut b.trace()));

    assert_eq!(
        diff(&a.sink().projection(), &b.sink().projection()),
        None,
        "the same script must produce the same decoded projection"
    );
    assert_eq!(a.counters().total(), b.counters().total());
    assert!(a.counters().total() > 20, "the run is substantial");
}

/// ★ The non-vacuity arm of the differential: perturb the run by exactly one operation
/// and the diff must name the FIRST position that disagrees, with both sides. A `diff`
/// that always returned `None` would pass the test above and fail this one.
#[test]
fn the_differential_names_the_first_position_that_diverges() {
    let mut a = Recorder::new(TraceLog::new());
    drop(scripted_run(&mut a.trace()));

    let mut b = Recorder::new(TraceLog::new());
    {
        let mut tr = b.trace();
        let mut gpu = world();
        // Same prefix, then ONE different decision: resolve a VA that was never bound.
        tr_route_pdb(&gpu, &mut tr, GPU0, A_PDB).expect("routes");
        tr_publish(&mut gpu, &mut tr, GPU0, A_PDB, SHARED_VA, 0x10000).expect("publishes");
        tr_resolve(&gpu, &mut tr, GPU0, A_PDB, VA_NEVER);
    }

    let d = diff(&a.sink().projection(), &b.sink().projection())
        .expect("the perturbed run must diverge");
    assert_eq!(
        d.at, 2,
        "the first two events are identical; the third is not"
    );
    assert_eq!(
        d.actual,
        Some(TraceEvent::AddressResolve {
            gpu: GPU0,
            pdb: A_PDB,
            va: VA_NEVER,
            outcome: Resolved::Fault(AddressFault::Miss {
                pdb: A_PDB,
                va: VA_NEVER
            }),
        }),
        "the divergence carries the offending event whole, MISS=FAULT and all"
    );
    assert!(matches!(
        d.expected,
        Some(TraceEvent::AddressResolve {
            outcome: Resolved::Hit { .. },
            ..
        })
    ));
}

// =================================================================================
// 3. ★ The negative trace class (mode2_gsp_port_plan.md §6.3): assert the EXACT
//    refusal, and a ZERO COUNT of a named event — with the positive run as the
//    non-vacuity arm.
// =================================================================================

/// A hostile/degenerate stream traces the exact refusal variants and **zero** forwarded
/// work. Each refusal is asserted by its [`FaultTag`], which the `Faulted` bridge derives
/// from an exhaustive `match` — so this is `testing_doctrine.md` §2's "name the variant",
/// not "an error happened".
#[test]
fn a_hostile_stream_traces_exact_refusals_and_forwards_nothing() {
    let mut rec = Recorder::new(TraceLog::new());
    let gpu = {
        let mut tr = rec.trace();
        let mut gpu = world();

        // (1) A token that does not decode.
        let bad = 0xdead_beef_dead_beefu64;
        let outcome = match handle_doorbell(&mut gpu, GPU0, bad, &[]) {
            Ok(_) => panic!("a malformed token must not ring"),
            Err(e) => Dispatched::Refused(e.fault_tag()),
        };
        tr.emit(|| TraceEvent::Doorbell {
            gpu: GPU0,
            vchid: VChid(0),
            token: bad,
            outcome,
        });

        // (2) A vChid that decodes but belongs to nobody.
        tr_doorbell(&mut gpu, &mut tr, GPU0, VChid(0x7ff), &[]);

        // (3) A PDB nobody declared.
        let _ = tr_route_pdb(&gpu, &mut tr, GPU0, Pdb(0xbad0_0000));

        // (4) ★ MG-3: A's vChid on the OTHER GPU. Byte-identical value, different
        // namespace — a routing map that collapsed `(GpuId, VChid)` would ring here.
        tr_doorbell(&mut gpu, &mut tr, GPU1, A_GR, &[]);

        // (5) The #14 ring-gate: a working-set VA that was never published.
        tr_publish(&mut gpu, &mut tr, GPU0, A_PDB, SHARED_VA, 0x1000).expect("publishes");
        tr_doorbell(&mut gpu, &mut tr, GPU0, A_GR, &[VA_NEVER]);

        // (6) An address resolve that misses.
        tr_resolve(&gpu, &mut tr, GPU0, A_PDB, VA_NEVER);

        // (7) ★ An RM apply into a namespace that never declared a client root —
        // §12.38's use-before-exist refusal, and the one path on which a `GpuError`
        // reaches the trace at all. (Added because the bite check found the
        // `GpuError::Graph` delegation was UNEXERCISED: every apply in the positive
        // script succeeds, so nothing distinguished a delegating bridge from a
        // flattening one.)
        tr_apply(
            &mut gpu,
            &mut tr,
            GPU0,
            HClient(0xCC),
            RmEvent::Alloc {
                client: HClient(0xCC),
                parent: H_DEVICE,
                handle: HObject(0x5c00_00aa),
                class: kayfabe_tests::COMPUTE_CLASS,
                facts: Default::default(),
            },
        );
        gpu
    };
    drop(gpu);

    let tags: Vec<Option<FaultTag>> = rec
        .sink()
        .records()
        .iter()
        .map(|r| r.ev.refusal())
        .collect();
    assert_eq!(
        tags,
        vec![
            Some(FaultTag("FwdFault::MalformedToken")),
            Some(FaultTag("FwdFault::UnknownVchid")),
            Some(FaultTag("FwdFault::UnknownPdb")),
            Some(FaultTag("FwdFault::UnknownVchid")),
            None, // the publication succeeded — the non-vacuity arm, inline
            Some(FaultTag("AddressFault::Miss")),
            Some(FaultTag("AddressFault::Miss")),
            // ★ Delegated through `GpuError::Graph`: the tag names WHICH protocol rule
            // was broken, not merely that the graph refused.
            Some(FaultTag("RmGraphError::UndeclaredClient")),
        ],
        "every refusal names its exact variant, in order, and the one success is visible"
    );

    // ★ The zero-count half: NOTHING was forwarded. A count of a named event, never an
    // absence — and the positive arm below is what makes the zero meaningful.
    let rung = rec
        .sink()
        .filter(|e| {
            matches!(
                e,
                TraceEvent::Doorbell {
                    outcome: Dispatched::Rung { .. },
                    ..
                }
            )
        })
        .len();
    assert_eq!(rung, 0, "not one doorbell may reach the host in this run");

    // ★ Non-vacuity arm: the SAME harness on a legal stream does ring.
    let mut ok = Recorder::new(TraceLog::new());
    {
        let mut tr = ok.trace();
        let mut gpu = world();
        tr_publish(&mut gpu, &mut tr, GPU0, A_PDB, SHARED_VA, 0x1000).expect("publishes");
        tr_doorbell(&mut gpu, &mut tr, GPU0, A_GR, &[SHARED_VA]);
    }
    assert_eq!(
        ok.sink()
            .filter(|e| matches!(
                e,
                TraceEvent::Doorbell {
                    outcome: Dispatched::Rung { .. },
                    ..
                }
            ))
            .len(),
        1,
        "the same observer DOES record a ring when one happens — so the zero above is a \
         fact about the hostile run, not about the instrument"
    );
}

// =================================================================================
// 4. The device wire plane, interleaved with the core planes (§6.1)
// =================================================================================

/// The wire plane carries a GSP-boot-shaped prefix, **interleaved** into the same stream
/// as the core planes, and the facts §6.1 says the C's own trace must not lose survive:
/// a served MMIO read value, the bytes a DMA read returned, and — the one the port plan
/// calls out by name — that the tx header was *written before* the interrupt was raised.
#[test]
fn the_wire_plane_interleaves_with_the_core_planes_in_one_stream() {
    let mut rec = Recorder::new(TraceLog::new());
    {
        let mut tr = rec.trace();
        let mut gpu = world();

        tr.emit(|| TraceEvent::Clock { ns: 1_000 });
        // The guest kicks the falcon, reads back a status register, we DMA-read the
        // queue's tx header out of guest RAM, write our own, and raise.
        tr.emit(|| TraceEvent::MmioWrite {
            bar: Bar(0),
            off: 0x110040,
            size: Width::B4,
            val: 0x2,
        });
        // ★ Decoded from a captured `(bar, off, size_u8, val)` tuple, which is what a
        // replay of a recorded binary stream actually does — so the closed `Width` is
        // exercised as a DECODE GUARD rather than only as a spelling.
        let captured_size = Width::from_bytes(4).expect("4 bytes is a register access");
        tr.emit(|| TraceEvent::MmioRead {
            bar: Bar(0),
            off: 0x110044,
            size: captured_size,
            val: 0x8000_0000, // ← what we SERVED, not what the register nominally holds
        });
        tr.emit(|| TraceEvent::GuestRead {
            gpa: Gpa(0x9_0000_0000),
            bytes: vec![0x01, 0x00, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00],
        });
        // A core-plane event lands BETWEEN the read and the write: the interleaving is
        // the payload, and a two-stream recorder would lose exactly this.
        tr_publish(&mut gpu, &mut tr, GPU0, A_PDB, SHARED_VA, 0x1000).expect("publishes");
        tr.emit(|| TraceEvent::GuestWrite {
            gpa: Gpa(0x9_0000_1000),
            bytes: vec![0x02, 0x00, 0x00, 0x00],
        });
        tr.emit(|| TraceEvent::IrqRaise {
            spec: IrqSpec::Msix(0),
        });
        tr.emit(|| TraceEvent::Clock { ns: 2_000 });
    }

    // A width a register access cannot have is a decode REFUSAL, not a silent number —
    // the reason this field is a closed enum and not the port plan's `size: u8`.
    assert_eq!(Width::from_bytes(3), None);
    assert_eq!(Width::from_bytes(0), None);
    assert_eq!(Width::from_bytes(8).map(Width::bytes), Some(8));

    let recs = rec.sink().records();
    assert_eq!(check_dense_order(recs, Seq(0)), Ok(()));
    assert_eq!(rec.counters().wire_total(), 7);
    assert_eq!(rec.counters().core_total(), 1);

    // The served value survived.
    assert!(recs.iter().any(|r| matches!(
        &r.ev,
        TraceEvent::MmioRead {
            val: 0x8000_0000,
            ..
        }
    )));
    // The bytes a DMA read returned survived — without them a replay is not hermetic.
    let read_bytes = recs
        .iter()
        .find_map(|r| match &r.ev {
            TraceEvent::GuestRead { bytes, .. } => Some(bytes.clone()),
            _ => None,
        })
        .expect("the DMA read is in the stream");
    assert_eq!(read_bytes, vec![0x01, 0x00, 0x00, 0x00, 0x20, 0, 0, 0]);

    // ★ §6.1's own example: the write precedes the raise, in ONE order.
    let w = recs
        .iter()
        .position(|r| matches!(&r.ev, TraceEvent::GuestWrite { .. }))
        .unwrap();
    let irq = recs
        .iter()
        .position(|r| matches!(&r.ev, TraceEvent::IrqRaise { .. }))
        .unwrap();
    let bind = recs
        .iter()
        .position(|r| matches!(&r.ev, TraceEvent::AddressBind { .. }))
        .unwrap();
    let dma = recs
        .iter()
        .position(|r| matches!(&r.ev, TraceEvent::GuestRead { .. }))
        .unwrap();
    assert!(w < irq, "the tx header must be written before the raise");
    assert!(
        dma < bind && bind < w,
        "the core-plane event is interleaved BETWEEN two wire-plane events \
         (dma #{dma}, bind #{bind}, write #{w}) — two separate streams would lose this"
    );
}

// =================================================================================
// 5. ★★ THE ANTI-VACUITY GUARDS — the instrument, checked against itself
// =================================================================================

/// A sink that implements the port and silently drops everything. This is the failure
/// mode a tracing crate invites, and it is the shape `testing_doctrine.md` §1 names:
/// every assertion about *behaviour* still passes, and only a check on the instrument
/// itself notices.
#[derive(Debug, Default)]
struct DroppingSink {
    offered: usize,
}

impl TraceSink for DroppingSink {
    fn record(&mut self, _rec: Record) {
        self.offered += 1; // counted here ONLY so the test can prove it was offered
    }
}

/// ★ The guard: a sink that records nothing is caught, because the counters live in the
/// recorder rather than in the sink. Both arms are asserted — the broken sink disagrees
/// with the counters, the honest one agrees — so the check cannot pass by being blind.
#[test]
fn a_sink_that_silently_drops_is_caught_by_the_counters() {
    let mut broken = Recorder::new(DroppingSink::default());
    drop(scripted_run(&mut broken.trace()));
    let offered = broken.sink().offered as u64;
    assert!(offered > 0, "the run drove real traffic");
    assert_eq!(
        broken.counters().total(),
        offered,
        "the recorder counts what it OFFERED, so it sees the events the sink threw away"
    );

    // ★ And the fan-out case, which is how an adapter runs in production: the full log
    // for the differential PLUS a live sink. Both must see the same records with the same
    // sequence numbers, because sequencing happens above them in the recorder.
    let mut teed = Recorder::new(kayfabe_trace::Tee {
        a: TraceLog::new(),
        b: DroppingSink::default(),
    });
    drop(scripted_run(&mut teed.trace()));
    assert_eq!(
        teed.sink().a.len(),
        teed.sink().b.offered,
        "a Tee must deliver every record to BOTH sinks"
    );
    assert_eq!(teed.counters().total(), teed.sink().a.len() as u64);
    assert_eq!(
        check_dense_order(teed.sink().a.records(), Seq(0)),
        Ok(()),
        "fanning out must not perturb the order"
    );

    let mut honest = Recorder::new(TraceLog::new());
    drop(scripted_run(&mut honest.trace()));
    assert_eq!(
        honest.counters().total(),
        honest.sink().len() as u64,
        "an honest sink agrees with the counters"
    );
    assert_eq!(
        honest.counters().total(),
        offered,
        "the two runs drove the same traffic — so 'stored == 0' below is a fact about \
         the sink, not about the workload"
    );
    // The whole point, stated as the comparison a harness would make:
    assert_ne!(
        broken.counters().total(),
        0,
        "a broken sink still leaves the counters non-zero — which is the tell"
    );
}

/// A `Counters` that can only ever read zero is a constant function (§1 rule 2). This
/// pins that it reads non-zero on a real run AND zero on a fresh one, so the reading is
/// a measurement rather than a constant.
#[test]
fn the_counters_read_zero_before_and_non_zero_after() {
    let fresh = Counters::new();
    assert!(fresh.is_silent());
    assert_eq!(fresh.silent_kinds().len(), EventKind::COUNT);
    assert_eq!(fresh.seen_kinds(), Vec::new());

    let mut rec = Recorder::new(TraceLog::new());
    drop(scripted_run(&mut rec.trace()));
    assert!(!rec.counters().is_silent());
    assert_eq!(rec.counters().seen_kinds(), CORE_KINDS.to_vec());
}

/// ★ Bite check for the order checker itself: plant a gap and a repeat, and require the
/// EXACT variant with its exact payload. A checker that returned `Ok` unconditionally
/// would pass every other test in this file.
#[test]
fn check_dense_order_bites_on_a_planted_gap_and_a_planted_repeat() {
    let mut rec = Recorder::new(TraceLog::new());
    {
        let mut tr = rec.trace();
        for ns in 0..4 {
            tr.emit(|| TraceEvent::Clock { ns });
        }
    }
    let good = rec.sink().records().to_vec();
    assert_eq!(check_dense_order(&good, Seq(0)), Ok(()));

    let mut gapped = good.clone();
    gapped.remove(2);
    assert_eq!(
        check_dense_order(&gapped, Seq(0)),
        Err(kayfabe_trace::OrderingError::Gap {
            index: 2,
            expected: Seq(2),
            seq: Seq(3),
        }),
        "a hole in the stream is a trace that cannot be replayed"
    );

    let mut repeated = good.clone();
    repeated[3].seq = Seq(2);
    assert_eq!(
        check_dense_order(&repeated, Seq(0)),
        Err(kayfabe_trace::OrderingError::NotIncreasing {
            index: 3,
            prev: Seq(2),
            seq: Seq(2),
        }),
        "two records claiming one position is exactly what a merged multi-recorder \
         stream looks like"
    );
}

// =================================================================================
// 6. ★ Cost when disabled — proved structurally, measured separately
// =================================================================================

/// The disabled path never CONSTRUCTS the event. Proved by making construction
/// observable: the closure's side effect is the witness, and the enabled run is the
/// non-vacuity arm. This is a structural assertion, so unlike a timing threshold it
/// cannot pass because the box was fast (`testing_doctrine.md` §3 rule 3).
#[test]
fn a_disabled_trace_never_constructs_the_event() {
    const N: usize = 10_000;

    let mut built = 0usize;
    let mut off = Trace::off();
    assert!(!off.is_enabled());
    assert_eq!(off.next_seq(), None);
    for i in 0..N {
        off.emit(|| {
            built += 1;
            // A payload whose construction genuinely costs: a heap allocation, which is
            // what the guest-RAM arms of the vocabulary carry.
            TraceEvent::GuestRead {
                gpa: Gpa(i as u64),
                bytes: vec![0u8; 4096],
            }
        });
    }
    assert_eq!(
        built, 0,
        "a disabled trace must not run the closure — no allocation, no field copies"
    );

    // ★ The non-vacuity arm: identical call site, enabled.
    let mut rec = Recorder::new(TraceLog::new());
    let mut built_on = 0usize;
    {
        let mut on = rec.trace();
        assert!(on.is_enabled());
        assert_eq!(on.next_seq(), Some(Seq(0)));
        for i in 0..N {
            on.emit(|| {
                built_on += 1;
                TraceEvent::GuestRead {
                    gpa: Gpa(i as u64),
                    bytes: vec![0u8; 4096],
                }
            });
        }
    }
    assert_eq!(
        built_on, N,
        "enabled, the same call site builds every event"
    );
    assert_eq!(rec.sink().len(), N);
    assert_eq!(rec.counters().of(EventKind::GuestRead), N as u64);
}

/// A **measurement**, printed rather than asserted — a wall-clock threshold would be
/// exactly the "passes because the box was fast" test §3 rule 3 forbids, in reverse.
/// The structural proof above is the gate; this is the number to quote.
///
/// ★★ Getting this loop to measure anything at all took three attempts, and the failures
/// are worth recording because each one printed a *plausible* number:
///
/// 1. a plain loop over `Trace::off()` — the compiler proves the `Option` is `None` and
///    deletes the loop. **0.00 ns/emit**, true and useless.
/// 2. `#[inline(never)]` on the loop — interprocedural constant propagation specialises a
///    copy of the callee for the known `None`. Still **0.00**.
/// 3. `black_box` on the argument — the load of `self.0` is loop-invariant, so it is
///    hoisted and the body deleted anyway. Still **0.00**.
///
/// It takes `black_box(&mut *tr)` **inside** the loop, which is the honest model of a
/// real call site (each plane's emit is a separate call the optimiser cannot pre-resolve).
/// A benchmark that reports 0.00 is the timing-shaped version of the vacuous instrument
/// this whole file is about.
#[inline(never)]
fn emit_clocks(tr: &mut Trace<'_>, n: usize) {
    for i in 0..n {
        std::hint::black_box(&mut *tr).emit(|| TraceEvent::Clock {
            ns: std::hint::black_box(i as u64),
        });
    }
}

/// The same loop carrying a HEAP payload — the guest-RAM arms of the vocabulary, which
/// is where laziness actually earns its keep.
#[inline(never)]
fn emit_dma(tr: &mut Trace<'_>, n: usize) {
    for i in 0..n {
        std::hint::black_box(&mut *tr).emit(|| TraceEvent::GuestRead {
            gpa: Gpa(std::hint::black_box(i as u64)),
            bytes: vec![0u8; 256],
        });
    }
}

#[test]
fn measure_the_cost_of_a_disabled_emit() {
    use std::time::Instant as Wall;
    const N: usize = 2_000_000;
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };

    // ★ `black_box` on the ARGUMENT, not just inside the loop: `#[inline(never)]` alone
    // does not stop interprocedural constant propagation from specialising a copy of the
    // callee for a statically-known `None`, which again reports 0.00 ns/emit.
    let t0 = Wall::now();
    emit_clocks(std::hint::black_box(&mut Trace::off()), N);
    let off_cheap = t0.elapsed();

    let t1 = Wall::now();
    emit_dma(std::hint::black_box(&mut Trace::off()), N);
    let off_heap = t1.elapsed();

    let mut counted = Recorder::new(kayfabe_trace::NoTrace);
    let t2 = Wall::now();
    emit_clocks(&mut counted.trace(), N);
    let on_counted = t2.elapsed();

    let mut logged = Recorder::new(TraceLog::new());
    let t3 = Wall::now();
    emit_clocks(&mut logged.trace(), N);
    let on_logged = t3.elapsed();

    let ns = |d: std::time::Duration| d.as_nanos() as f64 / N as f64;
    println!(
        "TRACE-COST ({profile}, {N} emits): DISABLED {:.2} ns/emit (Clock) | \
         DISABLED {:.2} ns/emit (GuestRead+256B heap payload) | \
         ENABLED+NoTrace {:.2} | ENABLED+TraceLog {:.2}",
        ns(off_cheap),
        ns(off_heap),
        ns(on_counted),
        ns(on_logged),
    );
    // The only assertions are the ones that cannot be a timing coincidence: the disabled
    // runs stored and counted nothing, the enabled ones counted everything.
    assert_eq!(counted.counters().total(), N as u64);
    assert_eq!(logged.sink().len(), N);
}

// =================================================================================
// 7. ★ Ordering across threads — the guarantee, and the anti-guarantee
// =================================================================================

/// ★ The ANTI-guarantee, pinned so nobody assumes it: two recorders do NOT share an
/// order. Both start at zero, so merging their streams by [`Seq`] fabricates an
/// interleaving — and the order checker says so, by name.
#[test]
fn two_recorders_do_not_share_an_order() {
    let mut a = Recorder::new(TraceLog::new());
    let mut b = Recorder::new(TraceLog::new());
    {
        let (mut ta, mut tb) = (a.trace(), b.trace());
        ta.emit(|| TraceEvent::Clock { ns: 10 });
        tb.emit(|| TraceEvent::Clock { ns: 20 });
        ta.emit(|| TraceEvent::Clock { ns: 30 });
        tb.emit(|| TraceEvent::Clock { ns: 40 });
    }
    // Each stream is internally fine…
    assert_eq!(check_dense_order(a.sink().records(), Seq(0)), Ok(()));
    assert_eq!(check_dense_order(b.sink().records(), Seq(0)), Ok(()));

    // …and the merge is nonsense, loudly.
    let mut merged: Vec<Record> = a.sink().records().to_vec();
    merged.extend(b.sink().records().iter().cloned());
    merged.sort_by_key(|r| r.seq);
    assert_eq!(
        check_dense_order(&merged, Seq(0)),
        Err(kayfabe_trace::OrderingError::NotIncreasing {
            index: 1,
            prev: Seq(0),
            seq: Seq(0),
        }),
        "two recorders' streams merged by Seq collide at the very first position — the \
         counter orders records into ONE recorder and nothing else"
    );
}

/// The guarantee, under real threads: one shared recorder totally orders concurrent
/// emitters. Dense, gapless, and the count is exact — assertions that are structural, so
/// no interleaving the scheduler picks can make this pass or fail by luck.
#[test]
fn a_shared_recorder_totally_orders_concurrent_threads() {
    const THREADS: usize = 8;
    const OPS: usize = 500;

    let rec = Arc::new(Mutex::new(Recorder::new(TraceLog::new())));
    let mut handles = Vec::new();
    for t in 0..THREADS {
        let rec = Arc::clone(&rec);
        handles.push(thread::spawn(move || {
            for k in 0..OPS {
                let mut g = rec.lock().expect("recorder");
                g.trace().emit(|| TraceEvent::Clock {
                    ns: (t * OPS + k) as u64,
                });
            }
        }));
    }
    for h in handles {
        h.join().expect("no emitter panicked");
    }

    let g = rec.lock().expect("recorder");
    assert_eq!(
        check_dense_order(g.sink().records(), Seq(0)),
        Ok(()),
        "a shared recorder is a total order across threads"
    );
    assert_eq!(
        g.counters().total(),
        (THREADS * OPS) as u64,
        "nothing lost, nothing duplicated"
    );
    // Every emitted value appears exactly once — conservation, not just a count.
    let seen: BTreeSet<u64> = g
        .sink()
        .records()
        .iter()
        .map(|r| match &r.ev {
            TraceEvent::Clock { ns } => *ns,
            other => panic!("unexpected event {other:?}"),
        })
        .collect();
    assert_eq!(seen.len(), THREADS * OPS);
}

// =================================================================================
// 8. The bridges — the mechanism that keeps the vocabulary from drifting
// =================================================================================

/// Every `RmEvent` shape the graph accepts converts to an [`kayfabe_trace::RmVerb`], and
/// the conversion carries the fields a differential needs. The value of this test is
/// smaller than the value of the impl (an exhaustive `match` in `kayfabe_core::trace`,
/// which fails the BUILD on a new variant) — it exists to pin the field mapping, which
/// the compiler cannot check.
#[test]
fn every_rm_event_converts_to_a_trace_verb_carrying_its_identifying_fields() {
    use kayfabe_trace::RmVerb;
    let alloc = RmEvent::Alloc {
        client: A_CLIENT,
        parent: H_DEVICE,
        handle: H_VASPACE,
        class: kayfabe_tests::COMPUTE_CLASS,
        facts: Default::default(),
    };
    assert_eq!(
        alloc.as_rm_verb(),
        RmVerb::Alloc {
            class: kayfabe_tests::COMPUTE_CLASS,
            parent: H_DEVICE
        }
    );
    let map = RmEvent::MapMemoryDma {
        client: A_CLIENT,
        vaspace: H_VASPACE,
        memory: MEM,
        va: SHARED_VA,
        offset: 0x40,
        len: 0x1000,
    };
    assert_eq!(
        map.as_rm_verb(),
        RmVerb::MapMemoryDma {
            memory: MEM,
            va: SHARED_VA,
            len: 0x1000
        }
    );
    let free = RmEvent::Free {
        client: A_CLIENT,
        handle: H_VASPACE,
    };
    assert_eq!(free.as_rm_verb(), RmVerb::Free);
    assert_eq!(
        RmEvent::SetPageDir {
            client: A_CLIENT,
            vaspace: H_VASPACE,
            pdb: A_PDB
        }
        .as_rm_verb(),
        RmVerb::SetPageDir { pdb: A_PDB }
    );
}

/// ★ §12.10's lesson as a test: a staleness refusal must tag its `Stale` variant, not
/// collapse to `FwdFault::Stale`. Near-neighbour faults must be distinguishable, or a
/// canary passes for the wrong reason.
#[test]
fn a_staleness_refusal_tags_which_revalidation_failed() {
    use kayfabe_core::ProcId;
    use kayfabe_fwd::Stale;
    assert_eq!(
        FwdFault::Stale(Stale::Proc(ProcId(3))).fault_tag(),
        FaultTag("Stale::Proc")
    );
    assert_eq!(
        FwdFault::Stale(Stale::Rebound).fault_tag(),
        FaultTag("Stale::Rebound")
    );
    assert_ne!(
        FwdFault::Stale(Stale::Proc(ProcId(3))).fault_tag(),
        FwdFault::Stale(Stale::Rebound).fault_tag(),
        "two staleness shapes must not read as each other"
    );
    // And an address fault delegates rather than flattening.
    assert_eq!(
        FwdFault::Address(AddressFault::Overlap {
            pdb: A_PDB,
            va: SHARED_VA
        })
        .fault_tag(),
        FaultTag("AddressFault::Overlap")
    );
    assert_ne!(
        FwdFault::Address(AddressFault::Overlap {
            pdb: A_PDB,
            va: SHARED_VA
        })
        .fault_tag(),
        FwdFault::Address(AddressFault::Miss {
            pdb: A_PDB,
            va: SHARED_VA
        })
        .fault_tag(),
        "a miss and an overlap are different findings"
    );
}

/// The system proc's forged completion is traceable as system traffic, and it is the one
/// place `signal_golden_capture` writes — so the vocabulary can express the L5/#12
/// kernel-vs-user split it exists to keep visible.
#[test]
fn a_system_forged_completion_traces_as_the_system_procs_completion() {
    let mut rec = Recorder::new(TraceLog::new());
    {
        let mut tr = rec.trace();
        let mut gpu = world();
        let ev = signal_golden_capture(&mut gpu, OsEventRef(0xE001)).expect("the forge path");
        tr.emit(|| TraceEvent::Completion {
            gpu: GPU0,
            proc: ProcRef(u32::MAX), // the system component's reserved label
            op: CompletionOp::Observed { event: ev },
        });
    }
    assert_eq!(rec.counters().of(EventKind::Completion), 1);
    assert!(matches!(
        &rec.sink().records()[0].ev,
        TraceEvent::Completion {
            op: CompletionOp::Observed {
                event: OsEventRef(0xE001)
            },
            ..
        }
    ));
}
