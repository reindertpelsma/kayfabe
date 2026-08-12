//! ★★★★★ **LEG A2 — the production birth path names the GUEST'S ring, and only when the
//! supply side already made those bytes one memory.**
//!
//! `gr_doorbell_passthrough.md` §0.3 states the wall at the code: the host GR channel is born
//! through `commit_engine_object → alloc_channel → alloc_channel_at(.., None) →
//! alloc_channel_in(.., RingSource::Ours(None))`, so *"the ring is OURS"* and *"the cursor is
//! OURS"* — and `alloc_channel_over_guest_ring`, the one verb that could say otherwise, had
//! exactly one caller and it was the R31 diagnostic probe.
//!
//! This file pins the three things that changed, as **one property with three arms over one
//! fixture** — the same channel, the same declared ring, differing only in what the address
//! table says is at that ring's VA.
//!
//! # ★★★ Why the arms are three and not two
//!
//! `falsifier_blocker_vs_only_blocker` — two values cannot tell *"the adoption fires"* from
//! *"the adoption fires for the wrong reason"*. The middle arm is the one that matters: a
//! host object **exists** at the ring's VA and the adoption is still refused, because
//! `binding.host().is_some()` asks *"does a host object exist here"* and the question that
//! decides correctness is *"does the guest reach these bytes some other way"*.
//! `[measured 2026-08-11]` `representability_of` made exactly that mistake, and it is why
//! `kayfabe_mmu::BackingBytes` exists at all.
//!
//! # ⊘ WATCHED RED — four mutations, each run and each output recorded verbatim
//!
//! | mutation in `kayfabe_fwd::adopted_guest_ring` / its call site | which test failed, and how |
//! |---|---|
//! | `if true { return None; }` at the top | `the_joined_ring_is_adopted_with_the_guests_own_numbers` — the `expect` fired |
//! | the `BackingBytes` test replaced by `if false` (i.e. *"a host object exists"*) | `a_host_object_that_is_not_a_joined_window_is_refused` — `left: Some(AdoptedGuestRing { memory: HostHandle(iso1/gpu0:0xbadc0de), .. }) right: None`. ★ It adopted an **arena page** |
//! | `gp_fifo_entries: 64` (the adapter's `GPFIFO_ENTRIES`) instead of `ring.entries` | `the_joined_ring_is_adopted_with_the_guests_own_numbers` — `left: 64 right: 1024` |
//! | the `channel.is_none()` guard dropped | `a_channel_that_already_exists_is_not_re_declared` — `left: Some(AdoptedGuestRing { .. }) right: None` |
//!
//! ⚠⚠ **AND THE FIRST ATTEMPT AT MUTATION 1 SILENTLY DID NOT APPLY**, which is worth more
//! than the mutation. A `str::replace` with no match count was run against a line `cargo fmt`
//! had since reflowed across five lines; it changed nothing, the suite stayed green, and the
//! green read exactly like *"this test does not catch that defect"*. ⇒ **A mutation that did
//! not apply and a test that does not catch are the same observation** unless the harness
//! asserts the mutation landed. Every run above asserts `count == 1` before writing.
//! Same class as the `dlen=0` oracle rows and the zero-byte bench artefact.
//!
//! ⊘ **What this file does NOT establish.** That anything executes. The host channel's
//! `GP_PUT` still lives in the USERD *we* hand RM (leg B, unbuilt), so `GP_PUT == GP_GET` and
//! the engine fetches nothing — `alloc_channel_over_guest_ring`'s own doc says so. And it
//! says nothing about the *adapter* half: that a joined handle is re-checked on the far side
//! of the IPC wire is `RING_NOT_A_JOINED_WINDOW`'s job, in `kayfabe-isolate-host`.

use kayfabe_arch::Aperture;
use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::rmgraph::{AllocFacts, GpFifoRing, RmEvent};
use kayfabe_core::{ChanId, ProcId};
use kayfabe_isolate::HostHandle;
use kayfabe_mmu::{BackingBytes, Binding, HostBacking};
use kayfabe_mocks::{MockArch, MockIsolateFactory, mock_classes as mc};
use kayfabe_tests::Scenario;

const GPU: GpuId = GpuId::ZERO;
const PDB0: Pdb = Pdb(0x4002_0000);
const CLIENT: HClient = HClient(0xC1D0_000C);
const GR_VCHID: VChid = VChid(0x19);

/// ★ The guest's own numbers, and they are the bench's own: `[measured, 45 byte-identical
/// `RING-ROSTER` rows over 45 boots since `w206`]` the GR channel declares
/// `ring=0x200200000 entries=1024`, and its leaf resolves `->0x1000000/Vidmem/sz0x200000`.
const RING_VA: GpuVa = GpuVa(0x2_0020_0000);
/// ⊘ **1024, not 64 and not 512.** `RingLayout::entries` is the modulus of the ring's wrap
/// arithmetic: a channel created with the guest's ring and the adapter's `GPFIFO_ENTRIES`
/// would have two parties disagreeing about which entry a number names.
const RING_ENTRIES: u32 = 1024;
/// The leaf, from the same measurement.
const LEAF_LEN: u64 = 0x20_0000;
const LEAF_FB_PHYS: u64 = 0x0100_0000;
/// Where the isolate placed the joined object — the leaf's own VA, which is what
/// `adopt_joined_fb_leaf` binds.
const JOINED_HOST_VA: u64 = RING_VA.0;
/// The joined host object. ★ In leg A2 this is the number that must reach
/// `GuestRing::memory`.
const JOINED_MEMORY: u64 = 0x0BAD_C0DE;

/// What the address table says lives at the ring's VA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RingBacking {
    /// Nothing host-backed at all — the control, and every boot before leg A1.
    GuestDeclaredOnly,
    /// ★ A host object exists at the ring's VA and it is our own arena page, not the guest's
    /// bytes. The middle arm.
    HostButSoleBacking,
    /// ★★★★★ The join: one memory.
    Joined,
}

/// A guest whose GR channel declares its ring at [`RING_VA`], with `backing` installed at
/// that VA.
fn guest_with_a_gr_channel(backing: RingBacking) -> (Gpu, ProcId, ChanId) {
    let (factory, _rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu =
        Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa).expect("the device realizes");
    let mut s = Scenario::new();
    let dev = HObject(0x5C00_0002);
    let vas = HObject(0x5C00_0007);
    let tsg = HObject(0x5C00_0011);
    let chan = HObject(0x5C00_0019);
    s.push(RmEvent::Alloc {
        client: CLIENT,
        parent: HObject(0),
        handle: HObject(CLIENT.0),
        class: mc::CLIENT,
        facts: kayfabe_tests::user_client(CLIENT),
    });
    s.push(RmEvent::Alloc {
        client: CLIENT,
        parent: HObject(CLIENT.0),
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
        handle: chan,
        class: mc::CHANNEL_GR,
        facts: AllocFacts {
            h_vaspace: Some(vas),
            userd_flags: MockArch::userd_flags_for(GR_VCHID),
            // ★ The guest's own declaration, and the ONLY place these two numbers come from.
            gp_fifo_ring: Some(GpFifoRing {
                va: RING_VA.0,
                entries: RING_ENTRIES,
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

    let binding = match backing {
        RingBacking::GuestDeclaredOnly => {
            Binding::declared_by_guest(LEAF_FB_PHYS, Aperture::Vidmem).expect("kind 2 is legal")
        }
        // ⊘ SysmemCoherent, and not by preference: `Binding::real_gpu_memory` REFUSES a
        // `SoleBacking` over `Aperture::Vidmem` outright (`FakeFbAtRealGpuVa`, ruling 3), so
        // the interesting arm — a host object that is genuinely not the guest's bytes — is
        // only expressible over an aperture the guard treats as innocent. ★ That is the
        // whole reason the core's own `BackingBytes` test is not redundant with the mmu
        // constructor: the constructor closes the Vidmem hole, and this arm is the one it
        // deliberately leaves open.
        RingBacking::HostButSoleBacking => Binding::real_gpu_memory(
            LEAF_FB_PHYS,
            Aperture::SysmemCoherent,
            HostBacking::whole(
                HostHandle::new(kayfabe_isolate::IsolateId::new(pid.0, GPU), JOINED_MEMORY),
                JOINED_HOST_VA,
                BackingBytes::SoleBacking,
            ),
        )
        .expect("a sole-backed sysmem range is a legal kind 3"),
        RingBacking::Joined => Binding::real_gpu_memory(
            LEAF_FB_PHYS,
            Aperture::Vidmem,
            HostBacking::whole(
                HostHandle::new(kayfabe_isolate::IsolateId::new(pid.0, GPU), JOINED_MEMORY),
                JOINED_HOST_VA,
                // ★★★★★ Ruling 4 — the emulated framebuffer is no longer the store for this
                // range, so `Vidmem` here is not a second memory.
                BackingBytes::JoinsGuestWindow,
            ),
        )
        .expect("a joined window over vidmem is exactly ruling 4's carve-out"),
    };
    {
        let proc = gpu.procs.get_mut(&pid).expect("live");
        let v = proc.vases.get_mut(&(GPU, PDB0)).expect("the VAS exists");
        v.table
            .bind(PDB0, RING_VA, LEAF_LEN, binding)
            .expect("the fixture's binding installs");
    }
    (gpu, pid, cid)
}

/// The `adopt` the birth path would carry for this fixture's channel.
fn planned_adoption(
    gpu: &Gpu,
    pid: ProcId,
) -> (
    Option<kayfabe_isolate::AdoptedGuestRing>,
    Option<kayfabe_isolate::ChannelHandles>,
) {
    let route =
        kayfabe_fwd::route_engine_object(&gpu.spine, GPU, GR_VCHID, kayfabe_tests::COMPUTE_CLASS)
            .expect("the GR channel routes");
    let planned = kayfabe_fwd::plan_engine_object(
        &gpu.spine,
        &gpu.procs[&pid],
        &route,
        kayfabe_tests::COMPUTE_CLASS,
        &[],
    )
    .expect("the engine object plans");
    match planned.verbs.expect("a first alloc emits verbs") {
        kayfabe_isolate::VerbPlan::EngineObject { adopt, channel, .. } => (adopt, channel),
        other => panic!("the engine-object plan emitted {other:?}"),
    }
}

/// ⊘ **THE CONTROL, and it must stay true of every build that does not arm leg A1.** With no
/// host backing at the ring's VA the birth path names nothing — byte for byte the shape every
/// committed boot in `traces/guest_boots/` was taken with.
#[test]
fn without_a_join_the_birth_path_names_no_ring_at_all() {
    let (gpu, pid, _cid) = guest_with_a_gr_channel(RingBacking::GuestDeclaredOnly);
    let (adopt, _) = planned_adoption(&gpu, pid);
    assert_eq!(
        adopt, None,
        "★★★ the default build must birth its host channel exactly as before. This is the \
         whole of leg A2's arming: there is no second selector, and `None` here is what makes \
         `KAYFABE_GUEST_RING=off` byte-identical BY CONSTRUCTION rather than by a flag that \
         could drift out of step with the supply side's"
    );
}

/// ★★★★★ **THE RUNG.** A joined window at the ring's VA is adopted, and the two numbers RM
/// is about to be told are the **guest's**.
#[test]
fn the_joined_ring_is_adopted_with_the_guests_own_numbers() {
    let (gpu, pid, _cid) = guest_with_a_gr_channel(RingBacking::Joined);
    let (adopt, _) = planned_adoption(&gpu, pid);
    let a = adopt.expect(
        "★★★★★ a joined window at the channel's declared gpFifoOffset must reach the birth \
         path. Without this the host GR channel is born over a ring of ours that stays empty \
         forever, which is `gr_doorbell_passthrough.md` §0.3's first reason",
    );
    assert_eq!(
        a.memory.raw(),
        JOINED_MEMORY,
        "★ the adoption must name the JOINED object — the one `join_fb_leaf` minted — and \
         nothing else"
    );
    assert_eq!(
        a.ring_va, JOINED_HOST_VA,
        "★ where the object is placed, off the binding rather than re-derived"
    );
    assert_eq!(
        a.gp_fifo_va, RING_VA.0,
        "★★ the GUEST's `gpFifoOffset`, passed through untouched. ⊘ NOT `ring_va + \
         GPFIFO_OFFSET`: that is our layout applied to memory that is not laid out that way"
    );
    assert_eq!(
        a.gp_fifo_entries, RING_ENTRIES,
        "★★★ the GUEST's `gpFifoEntries`. This is the modulus of `submit_entry`'s wrap — a \
         channel told our 64 over the guest's 1024 has two parties disagreeing about which \
         entry a number names, and they wrap in different places"
    );
}

/// ★★★★★ **THE MIDDLE ARM — a host object EXISTS at the ring's VA and it is still refused.**
///
/// ⊘ This is the owner's forbidden #2 as a test: the question is not *"is there a host object
/// here"* but *"does the guest reach these bytes some other way"*. A channel born over an
/// object the guest never writes fetches GPFIFO entries out of a page nothing ever wrote,
/// decodes zeros, never advances `GP_GET`, and **reports no error at all** — the
/// self-concealing shape that cost the C artifact weeks.
#[test]
fn a_host_object_that_is_not_a_joined_window_is_refused() {
    let (gpu, pid, _cid) = guest_with_a_gr_channel(RingBacking::HostButSoleBacking);
    let (adopt, _) = planned_adoption(&gpu, pid);
    assert_eq!(
        adopt, None,
        "★★★★★ `binding.host().is_some()` is TRUE for this fixture and the adoption must \
         still be `None`. A predicate that asked only whether a host object exists would \
         adopt an arena page here and the failure would be silent in both directions"
    );
}

/// ⊘ **A channel that already exists is not re-declared.** Its ring is whatever RM was told
/// at its birth, and a second opinion here is one RM cannot be given.
#[test]
fn a_channel_that_already_exists_is_not_re_declared() {
    let (mut gpu, pid, _cid) = guest_with_a_gr_channel(RingBacking::Joined);
    // Materialize once — this is the birth, and it is the call that may adopt.
    kayfabe_fwd::forward_engine_object(&mut gpu, GPU, GR_VCHID, kayfabe_tests::COMPUTE_CLASS, &[])
        .expect("the first alloc materializes the channel");
    let route = kayfabe_fwd::route_engine_object(
        &gpu.spine,
        GPU,
        GR_VCHID,
        kayfabe_mocks::mock_classes::GRAPHICS,
    )
    .expect("a second engine class routes to the same channel");
    let planned = kayfabe_fwd::plan_engine_object(
        &gpu.spine,
        &gpu.procs[&pid],
        &route,
        kayfabe_mocks::mock_classes::GRAPHICS,
        &[],
    )
    .expect("the second alloc plans");
    match planned.verbs {
        Some(kayfabe_isolate::VerbPlan::EngineObject {
            adopt,
            channel: Some(_),
            ..
        }) => assert_eq!(
            adopt, None,
            "★★★ a channel that already exists must carry NO adoption. It was born over \
             whatever it was born over; re-stating its ring here would be a second, silent \
             opinion about a fact RM already holds and cannot be told"
        ),
        // A replay emits no verbs at all, which is also a correct answer to "did the second
        // alloc re-declare the ring" — and the assertion above is then vacuous, so say so.
        None => panic!(
            "the second class resolved as a REPLAY, so this test asserted nothing. Pick a \
             class the channel has not already hosted"
        ),
        other => panic!("the second alloc emitted {other:?}"),
    }
}
