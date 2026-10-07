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
//! # ★★★★★ LEG B, added 2026-08-12 — four more watched-red mutations, each APPLICATION ASSERTED
//!
//! | mutation in `kayfabe_fwd::adopted_guest_userd` | which test failed, and how |
//! |---|---|
//! | `if true { return None; }` at the top | `the_guests_userd_inside_the_joined_leaf_is_adopted_as_an_offset` — the `expect` fired |
//! | `offset > len` instead of `offset + 512 > len` | `a_userd_slot_that_straddles_the_leafs_end_is_refused` — `left: Some(AdoptedGuestUserd { offset: 2096641 }) right: None`. ★ It adopted a slot whose last byte is **outside** the joined window |
//! | `offset = userd.offset` (forward the guest's own `userdOffset[0]`) | `the_guests_userd_inside_the_joined_leaf_is_adopted_as_an_offset` — `left: 36864 right: 8192` |
//! | `Sysmem { base, .. } => base` (fold it in with `Framebuffer`) | `a_userd_this_rung_cannot_cross_declines_rather_than_guessing` — `left: Some(AdoptedGuestUserd { offset: 8192 }) right: None`. ★ It read a **guest-physical** address as a framebuffer offset |
//!
//! ⚠ The third is only catchable because the fixture sets the guest's declared
//! `userdOffset[0]` to `0x9000` while its resolved address is the leaf base + `0x2000`. On
//! the bench those two numbers are **equal**, and a fixture that reproduced the coincidence
//! would have passed with the guest's offset forwarded into an object it is not an offset
//! into. `two_projections_of_one_fact_disagreeing`.
//!
//! ⊘ **What this file does NOT establish.** That anything executes. Both legs are now
//! present, so the channel RM is told about names the guest's ring **and** the guest's
//! cursor — but nothing here reads `GP_GET`, and `admitted_and_served_are_different_gates`.
//! It says nothing about the *adapter* half either: that the joined handles are re-checked on
//! the far side of the IPC wire is `RING_NOT_A_JOINED_WINDOW`'s and
//! `USERD_NOT_A_JOINED_WINDOW`'s job, in `kayfabe-isolate-host`. And it cannot say whether
//! host RM accepts a non-zero `userdOffset[0]` — that is hardware's answer and it is
//! `[NOT MEASURED]`.

use kayfabe_arch::Aperture;
use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::rmgraph::{AllocFacts, DeclaredUserd, GpFifoRing, RmEvent};
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
/// ★★★★★ **CONSTRAINT 26** — the ONE reserved object, in the SCRATCHPAD's namespace. A
/// per-proc worker cannot name it, which is the whole reason `RingProvenance` exists.
const STORE_OBJECT: u64 = 0x0057_0BE0;
/// Where in that object this leaf lives.
const STORE_OFFSET: u64 = 0x0400_0000;
/// The scratchpad isolate's proc id — `u32::MAX`, which can never alias a live `ProcId`.
const SCRATCHPAD_PROC: u32 = u32::MAX;
/// The scratchpad's own `NV01_MEMORY_VIRTUAL` over this `Vas`'s duped space.
const STORE_VAS: u64 = 0x00CA_FE0A;

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
    /// ★★★★★ **CONSTRAINT 26 — the ring is a SLICE of the one reserved object**, placed at
    /// the guest's own VA by the scratchpad. The binding names an arena this isolate does
    /// **not** own, so `frees_object()` is false and the handle must never reach a birth.
    StoreSlice,
}

/// ★★★★★ **LEG B — what the guest's own kernel said about this channel's USERD.**
///
/// ⊘ Five arms, and the four that decline are the point. `falsifier_blocker_vs_only_blocker`:
/// a two-value fixture cannot tell *"leg B fires"* from *"leg B fires whenever anything is
/// declared"*, and three of these four are things a real guest really sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UserdDecl {
    /// The params stopped before `userdMem` — every boot before this rung, and the state a
    /// boundary with no pinned layout is in. ⊘ NOT "the guest declared none".
    Unreadable,
    /// ★★★★★ Inside the joined leaf. `[measured, w262b]` the walling channels declare
    /// `off0x2000` into an object whose ring sits at `0x200200000` in a 2 MB leaf at fb
    /// `0x1000000`, so this is that leaf's base + `0x2000`.
    InsideTheJoinedLeaf,
    /// ★ Guest RAM — LEGAL, served by the guest-RAM pin and by no framebuffer join.
    Sysmem,
    /// ★ The descriptor was all zeros: the guest let RM allocate its own USERD.
    Undeclared,
    /// ★★★ A framebuffer address whose 512-byte slot ENDS one byte past the leaf. The arm
    /// that separates a containment check from a start-only bounds test.
    StraddlingTheLeafEnd,
}

/// A guest whose GR channel declares its ring at [`RING_VA`], with `backing` installed at
/// that VA.
fn guest_with_a_gr_channel(backing: RingBacking) -> (Gpu, ProcId, ChanId) {
    guest_with_a_gr_channel_and_userd(backing, UserdDecl::Unreadable)
}

/// The same fixture, plus what the guest's kernel said about the USERD.
fn guest_with_a_gr_channel_and_userd(
    backing: RingBacking,
    userd: UserdDecl,
) -> (Gpu, ProcId, ChanId) {
    let (factory, _rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::new(std::sync::Arc::new(MockArch::new()), Box::new(factory), gpa)
        .expect("the device realizes");
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
        // ⊘ The TEST default. The PRODUCTION path must never assume it:
        // a sysmem-rooted PDB read as vidmem walks the wrong memory and
        // reports success, because a wrong-aperture read returns zeros.
        pdb_aperture: Some(kayfabe_arch::Aperture::Vidmem),
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
            // ★★★★★ LEG B's supply side. ⊘ `handle`/`offset` are the guest's CLIENT-side
            // declaration and NOTHING here reads them — they are carried so a boot can state
            // them. `resolved` is what the guest's own KERNEL put on the wire.
            userd: Some(DeclaredUserd {
                handle: 0x5C00_0014,
                // ⊘⊘ **DELIBERATELY NOT `0x2000`.** On the bench these two numbers happen to
                // be equal — the guest's object starts at the leaf's base — and a fixture
                // that reproduced the coincidence could not tell *"we derived the offset
                // into OUR object"* from *"we forwarded the guest's offset into ITS
                // object"*. They are different quantities and this makes them different
                // numbers. `two_projections_of_one_fact_disagreeing`.
                offset: 0x9000,
                resolved: match userd {
                    UserdDecl::Unreadable => None,
                    UserdDecl::InsideTheJoinedLeaf => Some(kayfabe_arch::UserdMem::Framebuffer {
                        base: LEAF_FB_PHYS + 0x2000,
                        size: 512,
                    }),
                    UserdDecl::Sysmem => Some(kayfabe_arch::UserdMem::Sysmem {
                        base: LEAF_FB_PHYS + 0x2000,
                        size: 512,
                    }),
                    UserdDecl::Undeclared => {
                        Some(kayfabe_arch::UserdMem::Undeclared { address_space: 0 })
                    }
                    // The last 511 bytes of the leaf: the slot's FIRST byte is inside and its
                    // last is not.
                    UserdDecl::StraddlingTheLeafEnd => Some(kayfabe_arch::UserdMem::Framebuffer {
                        base: LEAF_FB_PHYS + LEAF_LEN - 511,
                        size: 512,
                    }),
                },
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
        // ★★★★★ **CONSTRAINT 26.** `HostBacking::slice` over an arena whose `IsolateId` is the
        // SCRATCHPAD's — a handle this proc's worker could never name. ⊘ The length must be
        // the leaf's, or `AddressTable::bind` refuses `SliceLenMismatch`, which is the check
        // that keeps a slice's extent and its row's extent one fact.
        RingBacking::StoreSlice => Binding::real_gpu_memory(
            LEAF_FB_PHYS,
            Aperture::Vidmem,
            HostBacking::slice(
                HostHandle::new(
                    kayfabe_isolate::IsolateId::new(SCRATCHPAD_PROC, GPU),
                    STORE_OBJECT,
                ),
                JOINED_HOST_VA,
                kayfabe_mmu::HostSlice::new(STORE_OFFSET, LEAF_LEN)
                    .expect("the fixture's slice is non-empty and does not wrap"),
                BackingBytes::JoinsGuestWindow,
            ),
        )
        .expect("a store slice at a guest VA is one memory, which is ruling 4's carve-out"),
    };
    {
        let proc = gpu.procs.get_mut(&pid).expect("live");
        let v = proc.vas_by_pdb_mut(GPU, PDB0).expect("the VAS exists");
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
        None,
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
    // ★★★★★ **CONSTRAINT 26** — the adoption now carries a `RingProvenance`, and a JOINED
    // window is `OwnObject`: the isolate minted it, so the far side can and does re-check it.
    // ⊘ Asserted as the ARM as well as the handle: a `StoreSlice` here would mean the ring
    // was read as a slice of the reserved object, which is a different chain entirely.
    assert_eq!(
        a.ring
            .handle()
            .expect(
                "★★★ a JOINED window must be `RingProvenance::OwnObject` — it is this \
                 isolate's own `OS_DESCRIPTOR`, and the far-side `RING_NOT_A_JOINED_WINDOW` \
                 check is only reachable for that arm"
            )
            .raw(),
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
        None,
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

// ═══════════════════════════════════════════════════════════════════════════════════════
// ★★★★★ LEG B — the guest's own USERD, adopted AT CREATION
// ═══════════════════════════════════════════════════════════════════════════════════════

/// The `adopt.userd` the birth path would carry for this fixture's channel.
fn planned_userd(
    backing: RingBacking,
    userd: UserdDecl,
) -> Option<kayfabe_isolate::AdoptedGuestUserd> {
    let (gpu, pid, _cid) = guest_with_a_gr_channel_and_userd(backing, userd);
    planned_adoption(&gpu, pid).0.and_then(|a| a.userd)
}

/// ★★★★★ **THE RUNG.** The guest's own kernel resolved its USERD to a framebuffer address
/// inside the leaf its ring was joined through, and the birth path names it as an **offset
/// into the joined object**.
///
/// ⊘ Note what is asserted about `offset`: it is `base - binding.phys()`, i.e. the guest's
/// address expressed relative to an object we hold. **The guest's raw address never reaches
/// hardware.** That is the whole difference between this and handing RM a guest-controlled
/// physical address.
#[test]
fn the_guests_userd_inside_the_joined_leaf_is_adopted_as_an_offset() {
    let u = planned_userd(RingBacking::Joined, UserdDecl::InsideTheJoinedLeaf).expect(
        "★★★★★ leg B: a resolved framebuffer USERD inside the ring's joined leaf must reach \
         the birth path. Without it the host channel's GP_PUT lives in a USERD of ours that \
         only we advance, so GP_PUT == GP_GET forever and the engine fetches nothing",
    );
    // ⊘ Compared through `handle()` and by RAW value, not as a whole `HostHandle`: the
    // isolate id is the fixture's business and pinning it here would make this assertion
    // about which isolate ran rather than about which object leg B named. The first draft of
    // this w755l edit hardcoded `IsolateId::NONE` and failed against `iso1` — the original
    // `.raw()` comparison was avoiding exactly that and I had to be told by the test.
    assert_eq!(
        u.object.handle().map(|h| h.raw()),
        Some(JOINED_MEMORY),
        "★ leg B must name the SAME joined object the ring names — one leaf, one join. \
         ⊘ w755l: `object` is now a VARIANT, because a store-slice USERD names the one \
         reserved object WITHOUT a handle — see `UserdObject::TheStore`."
    );
    assert!(
        matches!(u.object, kayfabe_isolate::UserdObject::Joined(_)),
        "★★★ a USERD inside a JOINED leaf must be the `Joined` variant. `TheStore` would \
         send this birth to the birth client B, which is right for a store slice and wrong \
         here — and the two are indistinguishable once only the offset is compared."
    );
    assert_eq!(
        u.offset, 0x2000,
        "★★★ the offset is the guest's resolved address MINUS the joined leaf's framebuffer \
         base (0x2000). ⊘ NOT the guest's declared `userdOffset[0]`, which this fixture sets \
         to 0x9000 precisely so the two cannot be confused: that one is an offset into the \
         GUEST'S object, this one is an offset into OURS. They coincide on the bench only \
         because the guest's object starts at the leaf's base"
    );
}

/// ⊘ **THE CONTROL, and it must stay true of every build that does not arm leg A1.**
#[test]
fn without_a_join_leg_b_names_no_userd_at_all() {
    assert_eq!(
        planned_userd(
            RingBacking::GuestDeclaredOnly,
            UserdDecl::InsideTheJoinedLeaf
        ),
        None,
        "★★★ leg B's arming is INHERITED: it is reachable only inside an `AdoptedGuestRing`, \
         so a build whose supply side is disarmed is `None` BY CONSTRUCTION. A second \
         selector here could drift out of step with the first"
    );
}

/// ★★★ **The three ways the guest's own declaration says no, and all three are normal.**
///
/// ⊘ Arms, not a wildcard: `Sysmem` is a REAL and legal USERD location this rung has no
/// crossing for, `Undeclared` is the guest saying it allocated none, and `Unreadable` is a
/// statement about **us**. Folding any of them into the others would make three findings look
/// like one decode that failed — the `dlen=0` error.
#[test]
fn a_userd_this_rung_cannot_cross_declines_rather_than_guessing() {
    for decl in [
        UserdDecl::Unreadable,
        UserdDecl::Sysmem,
        UserdDecl::Undeclared,
    ] {
        assert_eq!(
            planned_userd(RingBacking::Joined, decl),
            None,
            "{decl:?} must decline. ⚠ Adopting here would hand RM an address in the wrong \
             address space, or an address of zero — and RM ZEROES a caller-supplied USERD at \
             creation, so the symptom would be silence rather than an error"
        );
    }
}

/// ★★★★★ **The containment check is on the LAST byte, not the first.**
///
/// ⚠ A USERD whose first 8 bytes are inside the joined window and whose `GP_GET` is not would
/// be accepted by a start-only bounds test. The channel would then be created, scheduled and
/// doorbelled, and would fetch forever from a slot RM zeroed — `admitted_and_served_are_
/// different_gates`, with no error anywhere.
#[test]
fn a_userd_slot_that_straddles_the_leafs_end_is_refused() {
    assert_eq!(
        planned_userd(RingBacking::Joined, UserdDecl::StraddlingTheLeafEnd),
        None,
        "★★★★★ the 512-byte slot's LAST byte must be inside the joined leaf. `offset + 512 > \
         len` is the check; `offset < len` would accept this fixture"
    );
}

/// ⊘ **Leg B cannot fire without leg A2, and it is the TYPE that says so.**
///
/// `AdoptedGuestUserd` is reachable only through `AdoptedGuestRing::userd`, so *"the guest's
/// cursor over a ring of ours"* is unspellable — a channel in that state would have `GP_PUT`
/// racing ahead of a queue nothing writes. This asserts the reachability rather than the
/// comment: the middle arm has a host object at the ring's VA and leg B still declines.
#[test]
fn leg_b_is_unreachable_when_leg_a2_refused() {
    assert_eq!(
        planned_userd(
            RingBacking::HostButSoleBacking,
            UserdDecl::InsideTheJoinedLeaf
        ),
        None,
        "★★★★★ the ring's adoption was refused (an arena page, not the guest's bytes) and \
         leg B must be refused with it. A USERD adopted here would advance a cursor into a \
         GPFIFO the guest never writes"
    );
}

// =====================================================================================
// ★★★★★ CONSTRAINT 26 — "IS THIS RING A SLICE OF THE ONE OBJECT?", the restated question
// =====================================================================================

/// A [`kayfabe_fwd::RingSliceOracle`] that answers what it was built to answer, and records
/// what it was asked.
///
/// ⊘ It records the ARGUMENTS as well as the count: an oracle that answered `true` for a
/// different VA than the one under test would make every assertion below pass for the wrong
/// reason, and `a_probe_that_shares_the_allocator_is_not_an_observer` is this tree's name for
/// that class.
struct FixedOracle {
    answer: bool,
    asked: std::sync::Mutex<Vec<(u64, u64, u64)>>,
}

impl FixedOracle {
    fn new(answer: bool) -> FixedOracle {
        FixedOracle {
            answer,
            asked: std::sync::Mutex::new(Vec::new()),
        }
    }
}

impl kayfabe_fwd::RingSliceOracle for FixedOracle {
    fn is_slice_of_the_store(&self, vas: HostHandle, at: GpuVa, len: u64) -> bool {
        self.asked
            .lock()
            .expect("oracle")
            .push((vas.raw(), at.0, len));
        self.answer
    }
}

/// Record the scratchpad's range on the `Vas`, as the hand-over would.
fn hand_the_vas_over(gpu: &mut Gpu, pid: ProcId) {
    let proc = gpu.procs.get_mut(&pid).expect("live");
    let v = proc.vas_by_pdb_mut(GPU, PDB0).expect("the VAS exists");
    v.store_vas = Some(HostHandle::new(
        kayfabe_isolate::IsolateId::new(SCRATCHPAD_PROC, GPU),
        STORE_VAS,
    ));
}

/// The `adopt` the **birth** path would carry — the path that HAS the oracle, unlike
/// [`planned_adoption`]'s latch.
fn planned_birth(
    gpu: &Gpu,
    oracle: Option<&dyn kayfabe_fwd::RingSliceOracle>,
) -> Result<Option<kayfabe_isolate::VerbPlan>, kayfabe_fwd::FwdFault> {
    let route = kayfabe_fwd::route_channel_birth(&gpu.spine, CLIENT, HObject(0x5C00_0019))?;
    let planned =
        kayfabe_fwd::plan_channel_birth(&gpu.spine, &gpu.procs[&route.proc], &route, None, oracle)?;
    Ok(planned.verbs)
}

#[test]
fn a_store_slice_ring_is_adopted_only_when_the_mapper_says_it_placed_one() {
    let (mut gpu, pid, _cid) = guest_with_a_gr_channel(RingBacking::StoreSlice);
    hand_the_vas_over(&mut gpu, pid);

    // ---- ★★★ THE POSITIVE. The party that owns the object says it placed the slice.
    let yes = FixedOracle::new(true);
    let verbs = planned_birth(&gpu, Some(&yes))
        .expect("the birth plans")
        .expect("a first birth emits verbs");
    let kayfabe_isolate::VerbPlan::ChannelBirth { adopt, .. } = &verbs else {
        panic!("the birth emitted {verbs:?}");
    };
    assert_eq!(
        adopt.ring,
        kayfabe_isolate::RingProvenance::StoreSlice {
            offset: STORE_OFFSET,
            len: LEAF_LEN,
        },
        "★★★★★ CONSTRAINT 26 — a ring bound as a slice of the one object must be adopted AS \
         one, carrying the slice's own offset and length off the binding rather than a \
         handle. Anything else means the birth is about to name an object this isolate does \
         not hold."
    );
    assert_eq!(
        adopt.ring.handle(),
        None,
        "★★★★★ …and it must name NO HANDLE. The handle is what `Worker::execute`'s \
         foreign-handle gate refuses, and `[established from alloc_channel_in]` it never \
         reaches RM on this arm anyway — it was an authorization token, not an operand."
    );

    // ⊘ The oracle was asked about THE RING'S OWN VA, in THE SCRATCHPAD'S range. An oracle
    // asked about something else would make the `true` above meaningless.
    assert_eq!(
        yes.asked.lock().expect("oracle").as_slice(),
        &[(STORE_VAS, JOINED_HOST_VA, LEAF_LEN)],
        "★ NON-VACUITY: the oracle must be asked exactly once, about the scratchpad's range, \
         at the binding's own start, for the binding's own length"
    );

    // ---- ★★★★★ THE GATE: the foreign-handle gate has nothing to refuse.
    let foreign: Vec<_> = verbs
        .handles()
        .into_iter()
        .filter(|h| !h.belongs_to(kayfabe_isolate::IsolateId::new(pid.0, GPU)))
        .collect();
    assert!(
        foreign.is_empty(),
        "★★★★★ CONSTRAINT 26 — the birth plan still offers the per-proc worker a handle it \
         does not own: {foreign:?}. `Worker::execute` refuses the whole plan \
         `ForeignHandle` before RM is reached, which is the refusal that made the ownership \
         split unbuildable in the first place."
    );
}

#[test]
fn a_store_slice_ring_is_refused_when_no_one_can_vouch_for_it() {
    // ⊘⊘ **FAIL CLOSED, and this is the arm that says so.** A missing checker must never
    // read as a passed check — the worst direction of `a check that reports is not a check
    // that gates`, applied to the one question that separates a real ring from a blank twin.
    let (mut gpu, pid, _cid) = guest_with_a_gr_channel(RingBacking::StoreSlice);
    hand_the_vas_over(&mut gpu, pid);
    let err = planned_birth(&gpu, None).expect_err(
        "★★★★★ CONSTRAINT 26 — with NO ring-slice oracle installed, nobody holds the one \
         reserved object and nobody can say this range was ever mapped. Adopting it would \
         birth a channel over a blank twin: it fetches zeros, never advances GP_GET, and \
         reports NO ERROR AT ALL.",
    );
    assert!(
        matches!(
            err,
            kayfabe_fwd::FwdFault::PassthroughRingNotAdoptable { .. }
        ),
        "the refusal must be the ring's own, by name: got {err:?}"
    );
}

#[test]
fn a_store_slice_ring_is_refused_when_the_mapper_says_it_placed_nothing() {
    let (mut gpu, pid, _cid) = guest_with_a_gr_channel(RingBacking::StoreSlice);
    hand_the_vas_over(&mut gpu, pid);
    let no = FixedOracle::new(false);
    let err = planned_birth(&gpu, Some(&no)).expect_err(
        "★★★★★ CONSTRAINT 26 — the mapper says it placed NO slice here and the ring was \
         adopted anyway. This is `RING_NOT_A_JOINED_WINDOW` restated, and it is the same \
         silent failure: zeros fetched forever with no error anywhere.",
    );
    assert!(matches!(
        err,
        kayfabe_fwd::FwdFault::PassthroughRingNotAdoptable { .. }
    ));
    assert_eq!(
        no.asked.lock().expect("oracle").len(),
        1,
        "★ NON-VACUITY: the refusal must come from the oracle having been ASKED, not from \
         the plan failing earlier for an unrelated reason"
    );
}

#[test]
fn a_store_slice_ring_is_refused_when_the_vas_was_never_handed_over() {
    // ⊘ The `Vas` carries no scratchpad range, so there is nothing to ask the oracle ABOUT.
    // ⚠ This is a different failure from the two above and must not collapse into them: it
    // means the hand-over never happened, i.e. the scratchpad was never given this address
    // space — so every slice in it is one nobody placed.
    let (gpu, _pid, _cid) = guest_with_a_gr_channel(RingBacking::StoreSlice);
    let yes = FixedOracle::new(true);
    let err = planned_birth(&gpu, Some(&yes)).expect_err(
        "★★★★★ CONSTRAINT 26 — a store slice in a VA space the scratchpad never adopted was \
         adopted anyway",
    );
    assert!(matches!(
        err,
        kayfabe_fwd::FwdFault::PassthroughRingNotAdoptable { .. }
    ));
    assert!(
        yes.asked.lock().expect("oracle").is_empty(),
        "★ …and the oracle must not even be asked: with no `store_vas` there is no range to \
         name, and asking about a handle we do not have would be asking a question whose \
         `true` would mean nothing"
    );
}

#[test]
fn a_joined_window_is_still_an_own_object_and_still_names_its_handle() {
    // ⊘ THE CONTROL for all four arms above, and it is the `KAYFABE_VAS_OWNER=isolate` arm:
    // nothing about the pre-constraint-26 chain may have moved. An oracle is installed and
    // must NOT be consulted — a join is this isolate's own object and the far-side
    // `RING_NOT_A_JOINED_WINDOW` check is the one that governs it.
    let (gpu, pid, _cid) = guest_with_a_gr_channel(RingBacking::Joined);
    let yes = FixedOracle::new(true);
    let verbs = planned_birth(&gpu, Some(&yes))
        .expect("the birth plans")
        .expect("a first birth emits verbs");
    let kayfabe_isolate::VerbPlan::ChannelBirth { adopt, .. } = &verbs else {
        panic!("the birth emitted {verbs:?}");
    };
    assert_eq!(
        adopt.ring,
        kayfabe_isolate::RingProvenance::OwnObject(HostHandle::new(
            kayfabe_isolate::IsolateId::new(pid.0, GPU),
            JOINED_MEMORY
        )),
        "★ a joined window is the isolate's OWN object and must still be named as one"
    );
    assert!(
        yes.asked.lock().expect("oracle").is_empty(),
        "★★★ NON-VACUITY OF THE SPLIT: the oracle must NOT be consulted for a join. If it \
         is, then the two chains have been collapsed into one and the far-side \
         `RING_NOT_A_JOINED_WINDOW` check — which still governs this arm — is being asked a \
         question it is not the answer to."
    );
}
