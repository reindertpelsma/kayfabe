//! Batch-2 engine + context seams (`execution_plane.md` §2.1/§2.2/§2.5/§3): the
//! `EngineKind` routing tag, the shallow GR/CE context lifecycle (Case-1 forward →
//! host self-promotes its own golden ctx; Case-2 ack-only), and — the CRITICAL
//! anti-bolt-on property — that **the host verb surface does NOT grow to add an
//! engine**: a new engine is new arch encoding rows + core routing arms, ZERO new
//! host reach.
//!
//! Invariant/contract tests (decision #15), mock-driven, GPU-free.

#![allow(clippy::unusual_byte_groupings)]

use nvkvm_arch::ids::{ClassId, EngineKind, GpuVa, HClient, HObject, Pdb, VChid};
use nvkvm_arch::Arch;
use nvkvm_completion::{CompletionError, MAX_FENCE_JUMP, OsEventRef};
use nvkvm_core::gpu::Gpu;
use nvkvm_core::gpa::GpaSpace;
use nvkvm_core::rmgraph::{AllocFacts, RmEvent};
use nvkvm_fwd::{
    CompletionArm, ControlRoute, FwdFault, arm_fence, completion_arm, fence_observed,
    forward_engine_object, handle_doorbell, publish_backing, route_control,
};
use nvkvm_mocks::{MockArch, MockIsolateFactory, RmVerb, SharedRecorder, mock_classes as mc, mock_ctrl};
use nvkvm_tests::{Scenario, identical_handles};

const CLIENT: HClient = HClient(0xAA);
const PDB: Pdb = Pdb(0x3401_000);
const GR_VCHID: VChid = VChid(0x10);
const CE_VCHID: VChid = VChid(0x11);

fn compute_gpu() -> (Gpu, SharedRecorder) {
    let arch = Box::new(MockArch::new());
    let (factory, recorder) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");
    let mut s = Scenario::new();
    s.compute_process(CLIENT, PDB, identical_handles(GR_VCHID.0, CE_VCHID.0));
    for ev in s.events {
        gpu.apply(ev).expect("applies");
    }
    (gpu, recorder)
}

/// `Arch::engine_of_object` maps object classes to the `EngineKind` routing tag; a
/// non-engine class returns `None` (recorded, never guessed).
#[test]
fn engine_of_object_classifies_all_kinds() {
    let arch = MockArch::new();
    assert_eq!(arch.engine_of_object(mc::COMPUTE), Some(EngineKind::GrCompute));
    assert_eq!(arch.engine_of_object(mc::GRAPHICS), Some(EngineKind::GrGraphics));
    assert_eq!(arch.engine_of_object(mc::DMA_COPY), Some(EngineKind::Ce));
    assert_eq!(arch.engine_of_object(mc::NVENC), Some(EngineKind::NvEnc));
    assert_eq!(arch.engine_of_object(mc::MEMORY), None, "memory is not an engine object");
    assert_eq!(arch.engine_of_object(ClassId(0xdead)), None, "unknown class → None, no guess");
}

/// Case-1 forward: the engine-object alloc materializes the host channel and allocs
/// the object on it, through the OWNING proc's isolate — the host builds its own ctx.
#[test]
fn case1_forwards_engine_object_on_own_isolate() {
    let (mut gpu, recorder) = compute_gpu();

    let out = forward_engine_object(&mut gpu, GR_VCHID, mc::COMPUTE, &[])
        .expect("GR compute object forwards");
    assert_eq!(out.engine, EngineKind::GrCompute);
    assert!(out.materialized_channel, "first forward materializes the host channel");

    // The host saw: a channel alloc THEN an engine-object alloc on that channel, both
    // on ONE isolate (the owning proc's).
    let log = recorder.lock().unwrap();
    let chan = log.log.iter().find_map(|(iso, v)| match v {
        RmVerb::AllocChannel { handle, .. } => Some((*iso, *handle)),
        _ => None,
    });
    let (chan_iso, chan_h) = chan.expect("host channel allocated");
    let obj = log.log.iter().find_map(|(iso, v)| match v {
        RmVerb::AllocEngineObject { chan, class, .. } => Some((*iso, *chan, *class)),
        _ => None,
    });
    let (obj_iso, obj_chan, obj_class) = obj.expect("engine object allocated");
    assert_eq!(obj_chan, chan_h, "engine object allocated ON the host channel");
    assert_eq!(obj_class, mc::COMPUTE);
    assert_eq!(obj_iso, chan_iso, "channel + object forwarded on the SAME isolate");
}

/// A second forward on the same channel does not re-materialize it (idempotent
/// per-proc lifecycle, nothing one-shot).
#[test]
fn case1_second_forward_reuses_channel() {
    let (mut gpu, _rec) = compute_gpu();
    let first = forward_engine_object(&mut gpu, GR_VCHID, mc::COMPUTE, &[]).unwrap();
    assert!(first.materialized_channel);
    let second = forward_engine_object(&mut gpu, GR_VCHID, mc::COMPUTE, &[]).unwrap();
    assert!(!second.materialized_channel, "host channel already materialized");
}

/// ★ Engine-object forward idempotency (§2.2: "re-sends are idempotent"): a REPLAYED
/// Case-1 engine-object alloc — same channel, same declared class, re-sent any number
/// of times (the protocol is order-/repeat-independent) — yields exactly ONE host
/// engine object. The replay resolves from the channel's idempotency table and
/// returns the ORIGINAL host handle; the host backend never sees a duplicate alloc
/// (the same retried-RPC discipline the graph applies to alloc/DUP replay).
#[test]
fn replayed_engine_object_alloc_forwards_exactly_one_host_object() {
    let (mut gpu, recorder) = compute_gpu();

    let first = forward_engine_object(&mut gpu, GR_VCHID, mc::COMPUTE, &[]).unwrap();
    assert!(!first.reused, "first forward is the real alloc");
    for _ in 0..3 {
        let replay = forward_engine_object(&mut gpu, GR_VCHID, mc::COMPUTE, &[]).unwrap();
        assert!(replay.reused, "replay resolves from the idempotency table");
        assert_eq!(replay.host_object, first.host_object, "the ORIGINAL host object");
        assert_eq!(replay.engine, EngineKind::GrCompute);
    }

    // The host saw EXACTLY ONE engine-object alloc — no duplicate host object.
    let log = recorder.lock().unwrap();
    let engine_allocs = log
        .log
        .iter()
        .filter(|(_, v)| matches!(v, RmVerb::AllocEngineObject { .. }))
        .count();
    assert_eq!(engine_allocs, 1, "a replayed alloc must NOT create a duplicate host object");

    // A DIFFERENT class on the same channel is a genuinely new object, not a replay.
    drop(log);
    let ce = forward_engine_object(&mut gpu, GR_VCHID, mc::DMA_COPY, &[]).unwrap();
    assert!(!ce.reused);
    assert_ne!(ce.host_object, first.host_object);
}

/// ★ The fine `EngineKind` lands ON the `Channel` (§2.2 "what the core tracks"),
/// graph-derived: the channel class's declared kind, refined by the engine object
/// allocated on it — so NVENC vs GR-compute is distinguishable at the channel
/// (routing + completion-arm selection), not just at parse. Order-independent and
/// replay-tolerant like everything graph-derived.
#[test]
fn engine_kind_lands_on_the_channel_via_the_graph() {
    let (mut gpu, _rec) = compute_gpu();
    let kind_of = |gpu: &Gpu, vchid: VChid| {
        let (pid, cid) = gpu.by_vchid[&vchid];
        gpu.procs[&pid].channels[&cid].engine
    };

    // Channel-class defaults: GR-class channel = GrCompute, CE-class channel = Ce.
    assert_eq!(kind_of(&gpu, GR_VCHID), EngineKind::GrCompute);
    assert_eq!(kind_of(&gpu, CE_VCHID), EngineKind::Ce);

    // An NVENC session object allocated ON the GR channel refines the channel's
    // context to NvEnc — the distinguishability the completion tie-in keys on.
    let nvenc_alloc = RmEvent::Alloc {
        client: CLIENT,
        parent: HObject(0x5c00_0019), // the GR channel handle
        handle: HObject(0x5c00_0030),
        class: mc::NVENC,
        facts: AllocFacts::default(),
    };
    gpu.apply(nvenc_alloc).expect("engine-object alloc applies");
    assert_eq!(kind_of(&gpu, GR_VCHID), EngineKind::NvEnc, "engine object refined the channel");
    assert_eq!(kind_of(&gpu, CE_VCHID), EngineKind::Ce, "the other channel is untouched");

    // Replay tolerance: the identical alloc re-sent changes nothing.
    gpu.apply(nvenc_alloc).expect("replayed alloc is idempotent");
    assert_eq!(kind_of(&gpu, GR_VCHID), EngineKind::NvEnc);

    // The refinement is graph-derived, so it SURVIVES unrelated re-derivations
    // (any later apply re-syncs channels from the projection).
    let mut s2 = Scenario::new();
    s2.compute_process(HClient(0xBB), Pdb(0x3405_000), identical_handles(0x20, 0x21));
    for ev in s2.events {
        gpu.apply(ev).expect("second proc applies");
    }
    assert_eq!(kind_of(&gpu, GR_VCHID), EngineKind::NvEnc, "refinement survives re-derivation");
}

/// Case-2 controls are ACK-ONLY: never reach the host backend (replaying an
/// unprivileged GSP-internal control is "wrong layer"). Case-1 controls forward.
#[test]
fn case2_controls_are_ack_only_never_forwarded() {
    let (mut gpu, recorder) = compute_gpu();
    let pid = *gpu.by_pdb.get(&PDB).unwrap();

    // First materialize a host object to issue a Case-1 control against.
    let out = forward_engine_object(&mut gpu, GR_VCHID, mc::COMPUTE, &[]).unwrap();
    let mut payload = [0u8; 8];

    // Case-2: PROMOTE_CTX — ack-only, no host op.
    let route = route_control(&mut gpu, pid, out.host_object, mock_ctrl::PROMOTE_CTX, &mut payload)
        .expect("promote_ctx routes");
    assert_eq!(route, ControlRoute::AckOnly);
    // Case-2: GET_CTX_BUFFER_INFO — ack-only.
    let route = route_control(&mut gpu, pid, out.host_object, mock_ctrl::GET_CTX_BUFFER_INFO, &mut payload)
        .expect("get_ctx_buffer_info routes");
    assert_eq!(route, ControlRoute::AckOnly);

    // No Control verb reached the backend for the two Case-2 controls.
    {
        let log = recorder.lock().unwrap();
        assert!(
            !log.log.iter().any(|(_, v)| matches!(v, RmVerb::Control { .. })),
            "Case-2 controls must NEVER reach the host backend"
        );
    }

    // Case-1: a forwardable control DOES reach the host.
    let route = route_control(&mut gpu, pid, out.host_object, mock_ctrl::FORWARDABLE, &mut payload)
        .expect("forwardable control routes");
    assert_eq!(route, ControlRoute::Forwarded);
    let log = recorder.lock().unwrap();
    assert!(
        log.log.iter().any(|(_, v)| matches!(v, RmVerb::Control { cmd, .. } if *cmd == mock_ctrl::FORWARDABLE)),
        "a Case-1 control forwards to the host"
    );
}

/// A class the arch does not recognize as an engine object is a loud fault — never
/// guessed into a GR/CE object.
#[test]
fn forwarding_a_non_engine_class_is_a_loud_fault() {
    let (mut gpu, _rec) = compute_gpu();
    assert!(matches!(
        forward_engine_object(&mut gpu, GR_VCHID, mc::MEMORY, &[]),
        Err(FwdFault::NotAnEngine(_))
    ));
}

/// ★ THE anti-bolt-on property: forwarding EACH engine kind (compute, graphics, CE,
/// NVENC) uses the SAME host verb (`AllocEngineObject`) — the host verb surface does
/// NOT grow per engine. Only the arch's class→EngineKind row differs.
#[test]
fn host_verb_surface_does_not_grow_per_engine() {
    // For each distinct engine class, forward it on a fresh compute channel and record
    // which host verb kinds were used. The SET of verb variants must be identical
    // across engines — no engine introduces a new host verb.
    let engines = [
        (mc::COMPUTE, EngineKind::GrCompute),
        (mc::GRAPHICS, EngineKind::GrGraphics),
        (mc::DMA_COPY, EngineKind::Ce),
        (mc::NVENC, EngineKind::NvEnc),
    ];

    let verb_kind = |v: &RmVerb| -> &'static str {
        match v {
            RmVerb::AllocChannel { .. } => "AllocChannel",
            RmVerb::AllocVaSpace { .. } => "AllocVaSpace",
            RmVerb::AllocEngineObject { .. } => "AllocEngineObject",
            RmVerb::AllocSysmem { .. } => "AllocSysmem",
            RmVerb::Alloc { .. } => "Alloc",
            RmVerb::Schedule { .. } => "Schedule",
            RmVerb::MapGpuVa { .. } => "MapGpuVa",
            RmVerb::UnmapGpuVa { .. } => "UnmapGpuVa",
            RmVerb::RingDoorbell { .. } => "RingDoorbell",
            RmVerb::Free { .. } => "Free",
            RmVerb::Control { .. } => "Control",
            RmVerb::ExportSurface { .. } => "ExportSurface",
        }
    };

    let mut reference: Option<std::collections::BTreeSet<&'static str>> = None;
    for (class, kind) in engines {
        let (mut gpu, recorder) = compute_gpu();
        let out = forward_engine_object(&mut gpu, GR_VCHID, class, &[])
            .unwrap_or_else(|_| panic!("{kind:?} forwards"));
        assert_eq!(out.engine, kind);

        let used: std::collections::BTreeSet<&'static str> =
            recorder.lock().unwrap().log.iter().map(|(_, v)| verb_kind(v)).collect();
        match &reference {
            None => reference = Some(used),
            Some(r) => assert_eq!(
                &used, r,
                "engine {kind:?} used a DIFFERENT host verb set — the host surface grew per engine (anti-bolt-on violated)"
            ),
        }
    }
    // And the verb set is exactly the intent verbs, including the one engine verb.
    let r = reference.unwrap();
    assert!(r.contains("AllocEngineObject"), "every engine forwards via the one engine verb");
    assert!(!r.contains("Alloc"), "no engine falls back to a generic raw alloc");
}

/// ★ GR-1 — the C's `dma_copy_class_alloc_params` wrong-runlist class, pinned at the
/// core (memory: missing engine typing → `engineType=0` → wrong runlist →
/// cuCtxCreate 401): every host channel alloc carries the channel's graph-derived
/// `EngineKind`, so a graphics/NVENC/CE channel is materialized on ITS OWN
/// engine/runlist — never a compute default the adapter would have to guess.
/// Covers BOTH materialization sites: the engine-object forward and the doorbell.
#[test]
fn channel_materialization_declares_its_engine_to_the_backend() {
    // The recorded engine of the ONE AllocChannel in a recorder's log.
    let recorded_engine = |rec: &SharedRecorder| -> EngineKind {
        let log = rec.lock().unwrap();
        let engines: Vec<EngineKind> = log
            .log
            .iter()
            .filter_map(|(_, v)| match v {
                RmVerb::AllocChannel { engine, .. } => Some(*engine),
                _ => None,
            })
            .collect();
        assert_eq!(engines.len(), 1, "exactly one host channel alloc");
        engines[0]
    };

    // Site 1 (`forward_engine_object`): for each engine kind, refine the GR channel's
    // context through the graph (the guest's engine-object alloc), then let the
    // Case-1 forward materialize the host channel — the alloc must carry THAT kind.
    for (class, kind) in [
        (mc::COMPUTE, EngineKind::GrCompute),
        (mc::GRAPHICS, EngineKind::GrGraphics),
        (mc::DMA_COPY, EngineKind::Ce),
        (mc::NVENC, EngineKind::NvEnc),
    ] {
        let (mut gpu, recorder) = compute_gpu();
        gpu.apply(RmEvent::Alloc {
            client: CLIENT,
            parent: HObject(0x5c00_0019), // the GR channel handle
            handle: HObject(0x5c00_0030),
            class,
            facts: AllocFacts::default(),
        })
        .expect("engine object refines the channel");
        forward_engine_object(&mut gpu, GR_VCHID, class, &[])
            .unwrap_or_else(|_| panic!("{kind:?} forwards"));
        assert_eq!(
            recorded_engine(&recorder),
            kind,
            "{kind:?} channel materialized on a WRONG engine/runlist (the 401 class)"
        );
    }

    // Site 2 (`handle_doorbell` lazy materialization): the CE channel's first ring
    // materializes it as Ce — not the GR/compute default.
    let (mut gpu, recorder) = compute_gpu();
    handle_doorbell(&mut gpu, MockArch::token_for(CE_VCHID), &[])
        .expect("CE doorbell materializes + rings");
    assert_eq!(
        recorded_engine(&recorder),
        EngineKind::Ce,
        "doorbell-materialized CE channel must land on the CE runlist"
    );
}

// =================================================================================
// Completion pattern (e) — the mapped-fence arm (`execution_plane.md` §1.2/§2.4;
// NVENC's fence-not-event shape, `nvenc_101`). Arm selection keys on the CHANNEL's
// EngineKind; firing is value-observation, never event delivery.
// =================================================================================

const FENCE_VA: GpuVa = GpuVa(0x2_0060_0000);

/// Build a compute proc whose GR channel is refined to an NVENC context (session
/// object on the channel, through the graph) with a published fence page.
fn nvenc_gpu() -> (Gpu, nvkvm_core::ProcId, nvkvm_core::ChanId) {
    let (mut gpu, _rec) = compute_gpu();
    gpu.apply(RmEvent::Alloc {
        client: CLIENT,
        parent: HObject(0x5c00_0019), // the GR channel handle
        handle: HObject(0x5c00_0030),
        class: mc::NVENC,
        facts: AllocFacts::default(),
    })
    .expect("NVENC session allocs on the channel");
    let (pid, cid) = gpu.by_vchid[&GR_VCHID];
    assert_eq!(gpu.procs[&pid].channels[&cid].engine, EngineKind::NvEnc);
    publish_backing(gpu.procs.get_mut(&pid).unwrap(), PDB, FENCE_VA, 0x1000)
        .expect("fence page publishes into the channel's own host VAS");
    (gpu, pid, cid)
}

/// ★ Pattern (e), end to end: an NVENC-shaped channel arms a mapped-fence
/// completion; it fires when the observed value reaches/passes the target — and it
/// is DISTINCT from the event-delivery path: nothing enters the completion queue,
/// nothing posts, no SWGEN0 (the guest worker reads the mapped value directly,
/// `nvenc_101`).
#[test]
fn nvenc_mapped_fence_arms_and_fires_distinct_from_event_delivery() {
    let (mut gpu, pid, cid) = nvenc_gpu();

    // Arm at current=7, target=9 (the encoder will advance the fence to 9).
    assert_eq!(arm_fence(&mut gpu, pid, cid, FENCE_VA, 7, 9, OsEventRef(0xF0)), Ok(None));

    // Below target: not fired. At target: fired, exactly once.
    assert_eq!(fence_observed(&mut gpu, PDB, FENCE_VA, 8), Ok(None));
    assert_eq!(fence_observed(&mut gpu, PDB, FENCE_VA, 9), Ok(Some(OsEventRef(0xF0))));
    assert_eq!(fence_observed(&mut gpu, PDB, FENCE_VA, 10), Ok(None), "fired exactly once");

    // Distinct from the event path: the queue observed NOTHING, nothing to post.
    assert!(
        !gpu.procs[&pid].completion.has_outstanding(),
        "a fired fence never enters the completion queue (fence-not-event)"
    );
    assert!(gpu.pump_completions().is_none(), "nothing rides the GSP/SWGEN0 path");
}

/// The #12 wrap guard on the arm: a fence sequence crossing the u32 boundary fires
/// correctly, and a BACKWARDS (stale/foreign) value is a loud refusal that never
/// counts as completion progress.
#[test]
fn nvenc_fence_wrap_guard_fires_across_wrap_and_refuses_backwards_jumps() {
    let (mut gpu, pid, cid) = nvenc_gpu();

    // Arm just below the u32 boundary; the target is past the wrap.
    assert_eq!(
        arm_fence(&mut gpu, pid, cid, FENCE_VA, u32::MAX - 1, 2, OsEventRef(0xF1)),
        Ok(None)
    );
    // A stale/backwards value (a huge forward step under wrap arithmetic) is LOUD.
    let stale = u32::MAX - 1 - (MAX_FENCE_JUMP as u32 + 1);
    assert_eq!(
        fence_observed(&mut gpu, PDB, FENCE_VA, stale),
        Err(FwdFault::Completion(CompletionError::FenceJump {
            last: u32::MAX - 1,
            value: stale
        })),
        "the #12 backwards-jump class is refused, never treated as completion"
    );
    // The genuine advance across the wrap still fires.
    assert_eq!(fence_observed(&mut gpu, PDB, FENCE_VA, 1), Ok(None), "before target");
    assert_eq!(fence_observed(&mut gpu, PDB, FENCE_VA, 2), Ok(Some(OsEventRef(0xF1))));
}

/// Arm selection is exact AT the channel: a GR-compute or CE channel (shared-sema
/// arm) REFUSES a mapped-fence arm — the NVENC-vs-compute distinction the coarse
/// `EngineClass` could not express. And an unmapped fence address is a loud MISS
/// even on an NVENC channel (the fence must live in the channel's own Vas).
#[test]
fn fence_arm_selection_is_exact_at_the_channel() {
    // Selection table: only NVENC rides the fence arm (NVDEC = honest gap, sema).
    assert_eq!(completion_arm(EngineKind::NvEnc), CompletionArm::MappedFence);
    for k in [EngineKind::GrCompute, EngineKind::GrGraphics, EngineKind::Ce, EngineKind::NvDec] {
        assert_eq!(completion_arm(k), CompletionArm::SharedSema, "{k:?} is not fence-signalled");
    }

    // A compute-shaped channel refuses the fence arm loudly.
    let (mut gpu, _rec) = compute_gpu();
    let (pid, cid) = gpu.by_vchid[&GR_VCHID];
    publish_backing(gpu.procs.get_mut(&pid).unwrap(), PDB, FENCE_VA, 0x1000).unwrap();
    assert!(
        matches!(
            arm_fence(&mut gpu, pid, cid, FENCE_VA, 0, 1, OsEventRef(0xF2)),
            Err(FwdFault::WrongArm { engine: EngineKind::GrCompute, .. })
        ),
        "arming a fence on a sema-signalled channel is a WrongArm fault"
    );

    // An NVENC channel with an UNMAPPED fence address is a loud MISS (never armed).
    let (mut gpu, pid, cid) = nvenc_gpu();
    assert!(matches!(
        arm_fence(&mut gpu, pid, cid, GpuVa(0xdead_0000), 0, 1, OsEventRef(0xF3)),
        Err(FwdFault::Address(_))
    ));
}
