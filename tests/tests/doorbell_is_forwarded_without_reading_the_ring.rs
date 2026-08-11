//! # ★★★★★ **THE OPACITY PIN — a doorbell is forwarded WITHOUT the ring being readable**
//!
//! Owner ruling, 2026-08-11: *"in prod passthrough isn't parsed"*. The ring is **opaque**.
//! Everything a GR passthrough route would need is therefore a statement about a *cursor*
//! and a *token*, never about ring content — and the property that has to hold, forever,
//! is:
//!
//! > **Whether a doorbell is forwarded must not depend on whether its ring can be read.**
//!
//! `docs/design/gr_execution_boundary.md` §4.1 and `guest_ring_adoption.md` §4 both rest on
//! that property. Until this file, **nothing quantified over it**.
//!
//! # ⊘ THE BRIEF'S FEARED ENTANGLEMENT DOES NOT EXIST — and that is what this file pins
//!
//! The rung brief predicted an entanglement to be broken: *"`plan_doorbell` runs the #14
//! ring-gate over `working_set` … the GR arm's working set must be **empty so the gate is
//! vacuous**, not bypassed."* ⊘ **It is already empty**, from the only production supplier
//! (`crates/kayfabe-qemu-raw/src/shim.rs:3745` passes `&[]`), and
//! `kayfabe_rt::device::SharedDevice::doorbell`'s own doc states the choice deliberately:
//! *"The #14 ring-gate still sees an EMPTY working set, and that is stated on purpose …
//! This wiring parses the ring **after** that gate."*
//!
//! So there was nothing to build and nothing to bypass. What was missing is the **test**:
//! a documented intention is not an enforced one, and the ordering that makes it true —
//! `verb_op(plan → execute → commit)` **then** `forward_ring` — is two adjacent statements
//! in one function that any refactor may swap without a single assertion going red.
//!
//! # The three arms
//!
//! 1. **The acceptance.** A `GrCompute` channel whose GPFIFO ring has **no binding and no
//!    bytes** — unreadable by construction — still reaches `RmBackend::ring_doorbell`.
//!    ⊘ And forwards **no work**: `ce_copy` is never asked for. The two together are the
//!    whole claim; either alone is satisfiable by a wiring that is wrong in the other
//!    direction.
//! 2. **The gate is VACUOUS, not BYPASSED.** The same channel, planned twice: with an
//!    empty working set it plans; with one unpublished VA in the working set it is refused
//!    by name. ⊘ Red in both directions by construction — a gate that always passed would
//!    fail 2b, and a gate that always refused would fail 2a. This is what stops
//!    *"the gate is vacuous here"* from being written as *"the gate was removed"*.
//! 3. **Non-vacuity of the fixture**: nothing was rung, and nothing was recorded, before
//!    the doorbell under study.
//!
//! # ⊘ What a green here does NOT mean
//!
//! - ⊘ **It does not open the GR route.** `kayfabe_rt::DoorbellRoute::HostGr` still has no
//!   consumer, and `crates/kayfabe-qemu-raw/src/shim.rs`'s
//!   `Route::NotACopyEngineChannel` still refuses every `GrCompute` doorbell **before**
//!   this path is reached (pinned from the other side by
//!   `crates/kayfabe-qemu-raw/tests/e2_doorbell.rs`'s
//!   `a_gr_channel_is_refused_by_route_and_the_engine_object_is_what_moves_it`). This file
//!   drives `SharedDevice::doorbell` directly — the function a `HostGr` route would fall
//!   through **to** — so the property is pinned before the route exists rather than after.
//! - ⊘ **It says nothing about the host engine FETCHING anything.** The cursor bridge (G8)
//!   is unbuilt: nothing writes the guest's `GP_PUT` into the host channel's USERD
//!   (`guest_ring_adoption.md` §4). A rung doorbell on a channel whose `GP_PUT` is zero
//!   fetches nothing, correctly and forever.
//! - ⊘ **Nothing about a booted guest.** `only_live_boots_are_proof`.
//!
//! # ✔ WATCHED RED, BOTH DIRECTIONS, BEFORE IT WAS LANDED
//!
//! Green-both-ways proves nothing, so each arm was made to fail by breaking exactly the
//! thing it guards, and the tree was restored afterwards (`git diff --stat` empty).
//!
//! | break, applied temporarily | arm 1 | arm 2a | arm 2b |
//! |---|---|---|---|
//! | **A — "parse-then-serve"**: a ring-resolvability precondition added to `plan_doorbell` ahead of the gate | ⊘ **RED** — `Got []`, `doorbell` answered `Err(Address(Miss { pdb: Pdb(537919488), va: GpuVa(8592031744) }))` | ⊘ **RED** | green |
//! | **B — "the gate is bypassed"**: `VerbPlan::gated_doorbell`'s `is_host_published` check deleted | green | green | ⊘ **RED** — planned `DoorbellOutcome { proc: ProcId(1), chan: ChanId(0), … }` where a refusal was required |
//!
//! ★ Note the two breaks are caught by **different** arms and neither is caught by both.
//! That is the property `falsifier_blocker_vs_only_blocker` asks for: one arm going red
//! names *which* invariant moved, not merely *that* something did.
//!
//! Invariant/contract tests (decision #15), mock-driven, **GPU-free**.

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped handle/VA literals

use std::sync::Arc;

use kayfabe_arch::fault::ErrorNotifier;
use kayfabe_arch::ids::{EngineKind, GpuId, GpuVa, HClient, HObject, Pdb, VChid};
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

/// ★★★ **The ring's VA — nothing binds it and nothing wrote it.**
///
/// The number is the guest's own, from the bench: `[measured 2026-08-11]` every committed
/// boot since `w206` carries `RING-ROSTER key=0xc1d0000c:0x5c000019 ring=0x200200000
/// entries=1024` (45 identical rows over 45 boots, `traces/guest_boots/`). Using it rather
/// than a round fixture number costs nothing and makes the fixture's subject legible next
/// to a log.
///
/// ⊘ What makes it unreadable here is **not** the number: it is that no
/// `bind_ring_at` was called for it and no bytes were written to any guest-physical
/// address behind it. A binding is what a read needs, and the fixture withholds exactly
/// that one thing.
const GR_RING_VA: u64 = 0x2_0020_0000;

/// How many entries the guest declares. `[measured]` the same boots say `1024`; the value
/// is irrelevant to this file and is carried only so the declaration is a real one.
const GR_RING_ENTRIES: u32 = 1024;

/// A VA the guest never published. ⊘ Used **only** by arm 2b, to prove the #14 gate is
/// live rather than removed.
const UNPUBLISHED_VA: GpuVa = GpuVa(0x7d1e_0000_0000);

// =====================================================================================
// The guest
// =====================================================================================

/// One process with one **`GrCompute`** channel that declares a GPFIFO ring nobody bound.
///
/// ⊘ `mc::CHANNEL_GR` is `EngineKind::GrCompute` in the mock arch's own classifier
/// (`crates/kayfabe-mocks/src/lib.rs:216-218`, *"A GR-class channel is GrCompute until an
/// engine object refines it"*), which is the same rule
/// `kayfabe_core::project` applies to `AMPERE_CHANNEL_GPFIFO_A`. No engine object is
/// allocated, so nothing refines it away.
fn gr_guest_with_unreadable_ring() -> (Gpu, MockVmm, SharedRecorder, ProcId, ChanId) {
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu =
        Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa).expect("the device realizes");

    let root = HObject(0xC1D_0000);
    let dev = HObject(0xC1D_0001);
    let vas = HObject(0xC1D_0010);
    let tsg = HObject(0xC1D_0012);
    let chan = HObject(0xC1D_0019);

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
        handle: chan,
        // ★ THE SUBJECT: a GR channel, not a CE one.
        class: mc::CHANNEL_GR,
        facts: AllocFacts {
            h_vaspace: Some(vas),
            userd_flags: MockArch::userd_flags_for(GR_VCHID),
            error_notifier: Some(ErrorNotifier::Sysmem {
                gpa: notifier_gpa(GR_VCHID),
            }),
            // ★★★ The guest DECLARES its ring. Nothing binds it and nothing writes it —
            // which is the whole fixture. ⊘ Declaring it matters: a channel with no
            // `gp_fifo_ring` at all would make `forward_ring` decline for a reason that
            // has nothing to do with readability, and the test would pass vacuously.
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

    // ★ #177 — the guest schedules before it rings. Not this file's subject.
    kayfabe_tests::guest_schedules_every_channel(&mut gpu);

    (gpu, MockVmm::new(), rec, pid, cid)
}

/// Every host token `RmBackend::ring_doorbell` was asked for, in order. ⊘ The **verb** the
/// backend received, not a counter of our own — `measure_at_the_boundary_not_inside`.
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

/// Every `ce_copy` the backend was asked to perform. Must stay empty here: an unreadable
/// ring names no work.
fn copies(rec: &SharedRecorder) -> usize {
    rec.lock()
        .expect("recorder")
        .log
        .iter()
        .filter(|(_iso, v)| matches!(v, RmVerb::CeCopy { .. }))
        .count()
}

// =====================================================================================
// ★★★ ARM 1 — THE ACCEPTANCE
// =====================================================================================

/// ★★★★★ **A GR channel whose ring cannot be parsed still gets its doorbell forwarded.**
///
/// # Why this is the pin and not a restatement of the code
///
/// `SharedDevice::doorbell` is two statements: `verb_op(plan → execute → commit)`, whose
/// execute arm is the sole `RmBackend::ring_doorbell` call site in the workspace
/// (`crates/kayfabe-isolate/src/lib.rs:2412`), and **then** `forward_ring`, which is the
/// first thing in the chain that touches a ring byte. Nothing enforces that order. A
/// refactor that hoisted `forward_ring` — or that returned its `Err` before the verb ran —
/// would be a silent change from *"passthrough"* to *"parse-then-serve"*, and every other
/// test in the suite would stay green because every other test binds its ring.
///
/// ⊘ The assertion is deliberately on the **recorder**, not on `doorbell`'s `Result`: a
/// ring that cannot be read may legitimately make the *outer* call refuse (the forwarding
/// half genuinely failed), and reading that refusal as *"the doorbell was not forwarded"*
/// is the exact conflation this file exists to forbid.
#[test]
fn a_gr_doorbell_is_forwarded_although_its_ring_can_not_be_read() {
    let (gpu, mut vmm, rec, _pid, cid) = gr_guest_with_unreadable_ring();
    let dev = Arc::new(SharedDevice::new(gpu, LockMode::Sharded));

    // ★ ARM 3 — non-vacuity, first, so every number below is this doorbell's doing.
    assert!(
        doorbells(&rec).is_empty(),
        "★ non-vacuity: building the guest must not ring anything"
    );
    assert_eq!(
        copies(&rec),
        0,
        "★ non-vacuity: building the guest must not move a byte"
    );

    // The channel really is a GR channel, and it really does declare a ring. ⊘ Asserted
    // rather than assumed: a fixture that silently produced `EngineKind::Ce` would make
    // arm 1 green about the wrong engine entirely.
    dev.with_proc(_pid, |proc| {
        let chan = proc.channels.get(&cid).expect("the channel is live");
        assert_eq!(
            chan.engine,
            EngineKind::GrCompute,
            "the fixture's subject is a GR channel"
        );
    });

    let out = dev.doorbell(Some(&mut vmm), GPU, MockArch::token_for(GR_VCHID), &[]);

    // ★★★★★ THE CLAIM. Whatever the outer call answered, the host backend was asked to
    // ring — i.e. the SERVE decision did not consult the ring.
    let rung = doorbells(&rec);
    assert_eq!(
        rung.len(),
        1,
        "★★★★★ THE OPACITY PIN: a GR channel whose ring has no binding and no bytes must \
         STILL reach `RmBackend::ring_doorbell` exactly once. Got {rung:?}, and \
         `SharedDevice::doorbell` answered {out:?}. ⊘ If this is empty, the serve decision \
         now depends on the ring being readable — which is `in prod passthrough isn't \
         parsed` broken, not a test that needs updating."
    );

    // ★★★ And the other half: it forwarded no WORK. A ring nobody can read names no
    // operands, so asking the backend for a copy would mean the parse produced something
    // out of nothing.
    assert_eq!(
        copies(&rec),
        0,
        "⊘ an unreadable ring must name no work — a `ce_copy` here would be a parse that \
         invented its own operands"
    );
}

// =====================================================================================
// ★★★ ARM 2 — THE #14 RING-GATE IS VACUOUS, NOT BYPASSED
// =====================================================================================

/// ★★★★★ **The empty working set makes the gate vacuous; it does not remove it.**
///
/// `plan_doorbell` runs `VerbPlan::gated_doorbell` over the caller's `working_set`
/// (`crates/kayfabe-fwd/src/lib.rs:2707-2718`) and the only production caller passes `&[]`
/// (`crates/kayfabe-qemu-raw/src/shim.rs:3745`). Those two facts together are what make
/// passthrough compatible with #14 — and they are **only** compatible if the gate is still
/// there, live, for any caller that does supply a working set.
///
/// ⊘ **Red in both directions by construction.** A gate that had been deleted, or that
/// answered `true` for everything, fails 2b. A gate that refused an empty set — the naive
/// *"nothing is published, so refuse"* — fails 2a. Neither half can be made green by
/// weakening the other, which is what `falsifier_blocker_vs_only_blocker` asks of a
/// two-arm test.
#[test]
fn the_ring_gate_is_vacuous_on_an_empty_working_set_and_live_on_a_non_empty_one() {
    let (gpu, _vmm, _rec, _pid, _cid) = gr_guest_with_unreadable_ring();
    let dev = Arc::new(SharedDevice::new(gpu, LockMode::Sharded));
    let token = MockArch::token_for(GR_VCHID);

    // 2a — the production shape. No VA is offered, so the gate has nothing to refuse.
    dev.doorbell(None, GPU, token, &[]).expect(
        "★ 2a: an EMPTY working set must plan — the gate is vacuous, by having \
                 nothing to quantify over, and NOT by having been removed",
    );

    // 2b — the same channel, one unpublished VA offered. The gate must refuse it. ⊘ This
    // is the arm that makes 2a mean something: without it, `plan_doorbell` returning `Ok`
    // on `&[]` is equally consistent with the gate not existing.
    let refused = dev.doorbell(None, GPU, token, &[UNPUBLISHED_VA]);
    let err = refused.expect_err(
        "★★ 2b: a working set naming a VA this guest never published must be REFUSED. If \
         this planned, the #14 ring-gate is not live and 2a's green means nothing.",
    );
    let named = format!("{err:?}");
    assert!(
        named.contains(&format!("{:x}", UNPUBLISHED_VA.0)) || named.contains("Miss"),
        "★ the refusal must NAME the address it refused — got {named}. ⊘ A refusal that \
         cannot say which VA it was about is `a_wall_that_can_carry_no_name`."
    );
}
