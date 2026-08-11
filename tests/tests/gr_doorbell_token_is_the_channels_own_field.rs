//! # ★★★★★ **THE TOKEN VERDICT — guest→host translation for a GR doorbell is a FIELD READ**
//!
//! The rung brief asked for one thing to be settled *"with code, not prose"*:
//!
//! > *"**The token.** A C-era doc records three guest tokens with one matching no host
//! > token. My **inferred** reading: that is a symptom of no host GR channel existing at the
//! > time, not a real defect … **Test that reading.** If translation is a map lookup once
//! > the channel exists, say so with the lookup."*
//!
//! **The reading is correct, and the mechanism is narrower than the brief's own hope.** It
//! is not a map lookup. It is a **plain `Option` field on the channel the guest token
//! already routed to** — `kayfabe_core::gpu::Channel::host_token` — and *"a guest token
//! matching no host token"* is that `Option` being `None`, i.e. **"this channel has not
//! been materialized on the host yet"**.
//!
//! The C's own measurement says the same thing from the other side
//! (`C: docs/design/mode2_doorbell_chid.md` §16.1, `[measured 2026-06-05, GA106]`): what it
//! refuted is forwarding the guest's token **verbatim** (`0x10001` matched no host token);
//! what it prescribed is trap-and-translate against a per-channel host-token table. That
//! table is `Channel::host_token`.
//!
//! # ⊘ There is NOTHING to generalise from the CE path, and that is the finding
//!
//! The brief said *"token translation is already live on the CE path — generalise it, do
//! not invent it."* ⊘ **`plan_doorbell` never reads the engine to find a token.** The path
//! from guest token to host token contains no engine-keyed branch at any hop:
//!
//! | hop | what | where |
//! |---|---|---|
//! | 1 | `Arch::decode_doorbell(token)` → `VChid` | `kayfabe_fwd::route_doorbell` |
//! | 2 | `spine.by_vchid[(GpuId, VChid)]` → `(ProcId, ChanId)` | ditto — **the only map** |
//! | 3 | `proc.channels[cid]` | `kayfabe_fwd::plan_doorbell` |
//! | 4 | **`chan.host_channel.zip(chan.host_token)`** | ditto |
//!
//! So a GR doorbell was never one line of translation away from working. It was one `if` in
//! `crates/kayfabe-qemu-raw/src/shim.rs` away from being *asked*.
//!
//! # ★★★ THE NON-OBVIOUS HALF — the token already exists BEFORE the first doorbell
//!
//! `host_token` is written in exactly two commits, and the one that matters here is
//! **`commit_engine_object`**, not `commit_doorbell`. `[measured 2026-08-11, rev ce36a5b,
//! isolate_plane=real]` **eight host GR channels are materialized per boot** on the
//! engine-object alloc path — *upstream in time of any doorbell*. This file reproduces that
//! ordering: the engine object lands first, and the doorbell that follows **reuses** the
//! channel rather than minting a second one.
//!
//! ⊘ That reuse is not decoration. Two host channels for one guest vChid means one is
//! instantly orphaned and the guest's ring is on the wrong one — which is why
//! `commit_doorbell` refuses `Stale::Rebound` rather than overwriting. The assertion on the
//! `AllocChannel` **count** is what makes "it reused it" a fact rather than a hope.
//!
//! # ✔ WATCHED RED, TWO BREAKS, CAUGHT BY DIFFERENT ASSERTIONS
//!
//! Green-both-ways proves nothing, so each claim was made to fail by breaking exactly the
//! thing it guards, and the tree was restored afterwards.
//!
//! | break, applied temporarily | the verdict (test 1) | the reuse (test 1, fact 4) | the CE control (test 2) |
//! |---|---|---|---|
//! | **A — "forward the guest's token verbatim"**: `rm.ring_doorbell(chan.1)` → `rm.ring_doorbell(0xD000_0000_0000_0E00)` in `crates/kayfabe-isolate/src/lib.rs` | ⊘ **RED** | green | ⊘ **RED** |
//! | **B — "forget the materialized channel"**: `plan_doorbell`'s `let channel = chan.host_channel.zip(chan.host_token);` → `let channel = None;` | ⊘ **RED** | — (the run dies first) | green |
//!
//! Break **A**'s exact failure, and its shape is the whole point:
//!
//! ```text
//! ★★★★★ THE TOKEN VERDICT: the doorbell must ring the HOST token the engine-object path
//! already minted. The guest stored 0xd000000000000e00; the backend must be asked for
//! 0x200000.
//!   left: [14987979559889014272]
//!  right: [2097152]
//! ```
//!
//! ★ The number it rang is `0xd000000000000e00` — **the guest's own token**, the exact
//! class of value the C measured matching no host token on real hardware. The break
//! reproduces the C's 2026-06-05 finding from inside our own tree.
//!
//! Break **B**'s exact failure names `commit_doorbell`'s own guard, which is the mechanism
//! fact 4 asserts:
//!
//! ```text
//! a GR doorbell on a materialized channel is served: Stale(Rebound)
//! ```
//!
//! ★ Note that **A leaves the reuse assertion green and B leaves the CE control green** —
//! neither break is caught by everything, which is what `falsifier_blocker_vs_only_blocker`
//! asks for: an arm going red names *which* invariant moved.
//!
//! ⊘ **Nothing here is about a boot.** `only_live_boots_are_proof`. Invariant/contract
//! test (decision #15), mock-driven, GPU-free.

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped handle/VA literals

use std::sync::Arc;

use kayfabe_arch::fault::ErrorNotifier;
use kayfabe_arch::ids::{EngineKind, GpuId, HClient, HObject, Pdb, VChid};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::rmgraph::{AllocFacts, GpFifoRing, RmEvent};
use kayfabe_core::{ChanId, ProcId};
use kayfabe_mocks::{
    MockArch, MockIsolateFactory, MockVmm, RmVerb, SharedRecorder, mock_classes as mc,
};
use kayfabe_rt::device::{LockMode, SharedDevice};
use kayfabe_tests::{Scenario, notifier_gpa};

const GPU: GpuId = GpuId::ZERO;
const CLIENT: HClient = HClient(0xC1D_0000);
const PDB0: Pdb = Pdb(0x2010_0000);
const GR_VCHID: VChid = VChid(0x07);

/// The guest's declared ring. Nothing binds it and nothing writes it — deliberately, so
/// this file inherits the opacity pin's fixture property: **whether the doorbell is
/// forwarded must not depend on whether the ring can be read**
/// (`doorbell_is_forwarded_without_reading_the_ring.rs`).
const GR_RING_VA: u64 = 0x2_0020_0000;
const GR_RING_ENTRIES: u32 = 1024;

/// The channel handle, named so the engine object can be parented to it.
const CHAN: HObject = HObject(0xC1D_0019);

// =====================================================================================
// The guest
// =====================================================================================

/// One process, one **`GrCompute`** channel, ring declared and unbound.
///
/// ⊘ `mc::CHANNEL_GR` classifies as `EngineKind::GrCompute` until an engine object refines
/// it, which is the same rule `kayfabe_core::project` applies to
/// `AMPERE_CHANNEL_GPFIFO_A`. `mc::COMPUTE` is an `ObjectKind::EngineObject` whose engine
/// is `GrCompute`, so the refinement below **keeps** the route at `HostGr` rather than
/// moving it — which is what makes this fixture a GR fixture after the engine object as
/// well as before it.
fn gr_guest() -> (Gpu, MockVmm, SharedRecorder, ProcId, ChanId) {
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu =
        Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa).expect("the device realizes");

    let root = HObject(0xC1D_0000);
    let dev = HObject(0xC1D_0001);
    let vas = HObject(0xC1D_0010);
    let tsg = HObject(0xC1D_0012);

    let mut s = Scenario::new();
    s.push(RmEvent::Alloc {
        client: CLIENT,
        parent: root,
        handle: root,
        class: mc::CLIENT,
        facts: kayfabe_tests::user_client(CLIENT),
    });
    s.push(RmEvent::Alloc {
        client: CLIENT,
        parent: root,
        handle: dev,
        class: mc::DEVICE,
        facts: AllocFacts {
            device_instance: Some(0),
            ..Default::default()
        },
    });
    s.push(RmEvent::Alloc {
        client: CLIENT,
        parent: dev,
        handle: vas,
        class: mc::VASPACE,
        facts: AllocFacts::default(),
    });
    s.push(RmEvent::SetPageDir {
        client: CLIENT,
        vaspace: vas,
        pdb: PDB0,
    });
    s.push(RmEvent::Alloc {
        client: CLIENT,
        parent: dev,
        handle: tsg,
        class: mc::TSG,
        facts: AllocFacts {
            h_vaspace: Some(vas),
            ..Default::default()
        },
    });
    s.push(RmEvent::Alloc {
        client: CLIENT,
        parent: tsg,
        handle: CHAN,
        class: mc::CHANNEL_GR,
        facts: AllocFacts {
            h_vaspace: Some(vas),
            userd_flags: MockArch::userd_flags_for(GR_VCHID),
            error_notifier: Some(ErrorNotifier::Sysmem {
                gpa: notifier_gpa(GR_VCHID),
            }),
            gp_fifo_ring: Some(GpFifoRing {
                va: GR_RING_VA,
                entries: GR_RING_ENTRIES,
            }),
            ..Default::default()
        },
    });
    for ev in s.events {
        gpu.apply(ev).expect("applies");
    }

    let pid = *gpu.spine.by_pdb.get(&(GPU, PDB0)).expect("the VAS routed");
    let cid = *gpu.procs[&pid]
        .chan_ids
        .values()
        .next()
        .expect("the scenario's channel");

    // #177 — the guest schedules before it rings. Not this file's subject.
    kayfabe_tests::guest_schedules_every_channel(&mut gpu);

    (gpu, MockVmm::new(), rec, pid, cid)
}

/// Every host token `RmBackend::ring_doorbell` was asked for, in order. ⊘ The **verb the
/// backend received**, not a counter of our own — `measure_at_the_boundary_not_inside`.
fn doorbells(rec: &SharedRecorder) -> Vec<u64> {
    rec.lock()
        .expect("recorder")
        .log
        .iter()
        .filter_map(|(_iso, v)| match v {
            RmVerb::RingDoorbell { token } => Some(*token),
            _ => None,
        })
        .collect()
}

/// How many host channels were allocated. ⊘ The discriminator for *"the doorbell REUSED the
/// engine-object path's channel"* versus *"it minted a second one and rang that"* — two
/// worlds in which the doorbell log looks equally healthy.
fn channels_allocated(rec: &SharedRecorder) -> usize {
    rec.lock()
        .expect("recorder")
        .log
        .iter()
        .filter(|(_iso, v)| matches!(v, RmVerb::AllocChannel { .. }))
        .count()
}

// =====================================================================================
// ★★★★★ THE VERDICT
// =====================================================================================

/// ★★★★★ **The doorbell rings the HOST token that the ENGINE-OBJECT path already minted.**
///
/// Four facts in one script, in the order a boot produces them:
///
/// 1. Before anything, `Channel::host_token` is `None` — *"no host channel exists"*, which
///    is the state the C measured and read as a translation defect.
/// 2. The engine-object forward materializes the host channel and **writes the field**.
/// 3. The guest's doorbell rings **that** value, not the guest's own token.
/// 4. ⊘ And it allocates **no second channel** — the count stays at one across the doorbell.
///
/// ⊘ Fact 1 is asserted, not assumed: without it, fact 3 would be satisfiable by a fixture
/// in which the doorbell itself did the materializing, which is a different claim about a
/// different code path (`commit_doorbell`, not `commit_engine_object`) and is **not** the
/// eight-channels-per-boot ordering a real guest produces.
#[test]
fn a_gr_doorbell_rings_the_host_token_the_engine_object_path_already_minted() {
    let (gpu, mut vmm, rec, pid, cid) = gr_guest();
    let dev = Arc::new(SharedDevice::new(gpu, LockMode::Sharded));

    // ---- Non-vacuity, and fact (1). Nothing has rung; no host channel exists yet.
    assert!(
        doorbells(&rec).is_empty(),
        "★ non-vacuity: building the guest must ring nothing"
    );
    assert_eq!(
        channels_allocated(&rec),
        0,
        "★ non-vacuity: building the guest must allocate no host channel"
    );
    dev.with_proc(pid, |proc| {
        let chan = proc.channels.get(&cid).expect("the channel is live");
        assert_eq!(
            chan.engine,
            EngineKind::GrCompute,
            "the fixture's subject is a GR channel"
        );
        assert_eq!(
            chan.host_token, None,
            "★★★ FACT 1 — before the engine object, `host_token` is None. THIS is the state \
             the C read as `a guest token matching no host token`: not a translation \
             defect, an unmaterialized channel."
        );
    });

    // ---- (2) The engine-object forward — the path that materializes 8 GR channels per
    //          boot on real hardware, upstream in time of any doorbell.
    dev.forward_engine_object_by_parent(CLIENT, CHAN, mc::COMPUTE, &[])
        .expect("the Case-1 engine-object forward lands on a GR channel");

    let minted = dev
        .with_proc(pid, |proc| {
            proc.channels
                .get(&cid)
                .expect("the channel is live")
                .host_token
                .expect(
                    "★★★ FACT 2 — `commit_engine_object` writes `Channel::host_token`. If this \
                 is None the engine-object path did not materialize, and every assertion \
                 below would be about a channel the doorbell minted itself.",
                )
        })
        .expect("the proc is live");
    let after_engine_object = channels_allocated(&rec);
    assert_eq!(
        after_engine_object, 1,
        "the engine-object forward materialized exactly one host channel"
    );
    assert!(
        doorbells(&rec).is_empty(),
        "⊘ materializing a channel is not ringing it — no doorbell has been issued yet"
    );

    // ---- (3) THE DOORBELL. The guest stores ITS OWN token; the host must see the HOST's.
    let guest_token = MockArch::token_for(GR_VCHID);
    assert_ne!(
        guest_token, minted,
        "★ the fixture is non-degenerate: the guest's token and the host's are different \
         numbers, so `rang the host token` is a discriminating claim rather than a \
         tautology satisfied by forwarding verbatim"
    );

    let out = dev
        .doorbell(Some(&mut vmm), GPU, guest_token, &[])
        .expect("a GR doorbell on a materialized channel is served");

    assert_eq!(
        doorbells(&rec),
        vec![minted],
        "★★★★★ THE TOKEN VERDICT: the doorbell must ring the HOST token the engine-object \
         path already minted. The guest stored {guest_token:#x}; the backend must be asked \
         for {minted:#x}. A tree that forwarded the guest's token verbatim reproduces the C's \
         `C: mode2_doorbell_chid.md` §16.1 measurement — `0x10001` matched no host token."
    );
    assert_eq!(
        out.host_token, minted,
        "the outcome reports the same host token the backend was asked for — one fact, not \
         two projections of it"
    );
    assert_eq!(
        out.engine,
        EngineKind::GrCompute,
        "⊘ `DoorbellOutcome::engine` is the channel's own, off the same `chan` binding as \
         `host_token` — it is what `SharedDevice::doorbell` decides the content-forward on"
    );

    // ---- (4) ⊘ THE REUSE. No second channel: the doorbell adopted what already existed.
    assert_eq!(
        channels_allocated(&rec),
        after_engine_object,
        "★★★ the doorbell must REUSE the engine-object path's host channel, never mint a \
         second one. Two host channels for one guest vChid means one is instantly orphaned \
         and the guest's ring is bound to the wrong one — which is why `commit_doorbell` \
         refuses `Stale::Rebound` rather than overwriting."
    );
}

/// ★★★ **The translation is not engine-keyed** — the same script on a **CE** channel
/// produces the same shape, which is why there was nothing to *generalise*.
///
/// ⊘ This is the control for the file's central negative claim (*"`plan_doorbell` never
/// reads the engine to find a token"*). If the GR arm needed a generalisation of the CE
/// arm, the two would differ somewhere on this path; they do not differ anywhere.
#[test]
fn the_same_translation_serves_a_ce_channel_and_is_not_keyed_on_the_engine() {
    let (mut gpu, mut vmm, rec, pid, cid) = gr_guest();
    // Refine the channel to CE by landing a copy-engine object on it — the same
    // refinement `kayfabe_core::project` applies when `AMPERE_DMA_COPY_B` arrives.
    gpu.apply(RmEvent::Alloc {
        client: CLIENT,
        parent: CHAN,
        handle: HObject(0xC1D_001A),
        class: mc::DMA_COPY,
        facts: AllocFacts::default(),
    })
    .expect("a copy-engine object lands on the channel");
    let dev = Arc::new(SharedDevice::new(gpu, LockMode::Sharded));

    dev.with_proc(pid, |proc| {
        assert_eq!(
            proc.channels.get(&cid).expect("live").engine,
            EngineKind::Ce,
            "★ the control is only a control if the refinement actually moved the engine"
        );
    });

    let guest_token = MockArch::token_for(GR_VCHID);
    let out = dev
        .doorbell(Some(&mut vmm), GPU, guest_token, &[])
        .expect("a CE doorbell is served");

    assert_eq!(
        doorbells(&rec),
        vec![out.host_token],
        "the CE path rings the host token too — the SAME two hops, with no engine-keyed \
         branch between the guest's token and the host's"
    );
    assert_ne!(
        out.host_token, guest_token,
        "and it is not the guest's token on this arm either"
    );
}
