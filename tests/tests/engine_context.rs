//! Batch-2 engine + context seams (`execution_plane.md` §2.1/§2.2/§2.5/§3): the
//! `EngineKind` routing tag, the shallow GR/CE context lifecycle (Case-1 forward →
//! host self-promotes its own golden ctx; Case-2 ack-only), and — the CRITICAL
//! anti-bolt-on property — that **the host verb surface does NOT grow to add an
//! engine**: a new engine is new arch encoding rows + core routing arms, ZERO new
//! host reach.
//!
//! Invariant/contract tests (decision #15), mock-driven, GPU-free.

#![allow(clippy::unusual_byte_groupings)]

use nvkvm_arch::ids::{ClassId, EngineKind, HClient, Pdb, VChid};
use nvkvm_arch::Arch;
use nvkvm_core::gpu::Gpu;
use nvkvm_core::gpa::GpaSpace;
use nvkvm_fwd::{ControlRoute, FwdFault, forward_engine_object, route_control};
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
