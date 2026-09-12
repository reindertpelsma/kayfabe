//! ★★★ **E2 — the acceptance, at the composition root.**
//!
//! `docs/design/execution_plane_increments.md`:
//!
//! > **E2** — the doorbell reaches the core: a guest MMIO write to the usermode doorbell
//! > aperture arrives at `kayfabe_rt::SharedDevice::doorbell`.
//! > *acceptance that could fail*: a boot in which a guest doorbell write produces a
//! > `DoorbellOutcome`-or-named-`FwdFault`, **counted**.
//! > *control*: a non-doorbell BAR write in the same run produces neither.
//!
//! `crates/kayfabe-device/tests/doorbell_aperture.rs` drives the transport against a
//! **recording** port and says in its own docs that it therefore witnesses nothing about the
//! core. This file is the other half, and it is the only place the join can be asserted:
//! [`Regs::create`] is what installs the real port, and the port is a handle onto the
//! **same** `SharedDevice` the object bridge declares `GSP_RM_ALLOC` into.
//!
//! ## ★★★ What makes the green here mean something, and what it still does not
//!
//! - The refusal's **kind** is `FwdFault::…`, produced by `kayfabe-fwd`'s own exhaustive
//!   `Faulted` match. It is *unreachable* without a real call into
//!   `SharedDevice::doorbell` → `route_doorbell` → `Arch::decode_doorbell`. A shim that
//!   dropped the write, or wired the port to nothing, answers
//!   `Device::NoDoorbellPort` — a different kind, from a different crate — and every
//!   assertion below distinguishes the two explicitly.
//! - Two **different** guest tokens produce two **different** named refusals, which no
//!   constant answer can imitate.
//! - ⊘ It does **not** witness a `DoorbellOutcome`. Serving one needs a channel on the
//!   spine, which needs a guest that allocated one, which is increments E4-E6. What E2
//!   claims is the transport and the refusal vocabulary, and this file claims exactly that.
//! - ⊘ It is an in-process test, so it cannot attribute a ring to a *guest*. That is the
//!   live boot's job, and the attributing instrument there is the device's own
//!   timestamped per-write line bracketed by a guest-side command — see
//!   `scripts/bench/e2_doorbell_witness.sh`.

use kayfabe_qemu_raw::shim::Regs;

/// The register aperture's logical index, as the C shim passes it.
const BAR_REGS: u32 = 0;
/// The instance/BAR2 aperture.
const BAR_INST: u32 = 2;

/// `0x00B8_0000` (the physical function's `NV_VIRTUAL_FUNCTION` offset) + `0x0003_0090`
/// (`NV_VIRTUAL_FUNCTION_DOORBELL`).
///
/// ★ Spelled out here rather than imported from `kayfabe_device::doorbell_reg`, and that is
/// deliberate: this file is the **acceptance**, and an acceptance that asks the code under
/// test where to write is one that passes wherever the code decides to listen. The
/// derivation is asserted against the chip's advertisement in
/// `kayfabe-device/tests/doorbell_aperture.rs`; here the number is stated, from the header.
const DOORBELL: u64 = 0x00BB_0090;

/// A well-formed work-submit token: runlist 7, channel 5. RM's encoder can emit it
/// (`VECTOR` 11:0, `RUNLIST_ID` 22:16 — `doorbell_token_encoding.md` §1), so the decode
/// **succeeds** and the refusal that follows is a routing one.
const GOOD_TOKEN: u64 = 0x0007_0005;

/// A token RM's encoder can **never** emit: bit 12 is in the gap between the two fields, and
/// `kfifoGenerateWorkSubmitTokenHal_GA100` starts from `val = 0` and sets only the two.
const MALFORMED_TOKEN: u64 = 0x0000_1005;

/// The kind a plane whose port was never installed answers with — the near neighbour every
/// assertion here has to be kept apart from.
const NO_PORT: &str = "Device::NoDoorbellPort";

fn regs() -> Regs {
    // ★★★★★ **w512 — THESE TESTS ARE ABOUT THE ROUTING CHAIN, SO THEY PIN THE ARM THAT
    // RUNS IT ON THE CALLER.**
    //
    // ⊘ Six tests in this file went red at w467 and stayed red, and the reason is not a
    // routing regression: the async doorbell arm became the DEFAULT, so `ring` now enqueues
    // and answers `Scheduled`, and the refusal these tests are named for is produced later,
    // on the worker. `kind_of` then panicked with *"expected a named refusal, got
    // Scheduled"* — a true message about a test that had stopped asking its own question.
    //
    // ★ The routing chain itself is UNCHANGED and shared: both arms reach it through
    // `SharedDoorbell::ring_inline`. Pinning the arm off here runs exactly the same code,
    // synchronously, where a test can see its answer.
    //
    // ⚠ And that is a real cost, stated rather than hidden: with the arm pinned, this file
    // no longer exercises the SHIPPING configuration's trap path. That half is asserted
    // separately by `the_shipping_arm_enqueues_instead_of_routing_on_the_caller` below — ⊘
    // without which "the doorbell works" would be a claim about a configuration nobody runs.
    //
    // Process-global, and sound for the same reason the `KAYFABE_ISOLATES` note below gives:
    // this is its own test binary.
    //
    // SAFETY: single-threaded test setup, before any `Regs::create` in this process.
    unsafe {
        std::env::set_var(kayfabe_qemu_raw::shim::DOORBELL_ASYNC_ENV, "off");
    }
    // `0` selects the chip table's default row (GA106).
    //
    // ⚠ This reads `KAYFABE_ISOLATES`, process-globally, and the default is `stillborn`.
    // Its own test binary, so nothing else in this process can have set it — and if
    // something did, `Regs::create` would refuse to build rather than degrade, which is the
    // selector's whole design.
    Regs::create(0).expect("the shipped chip row realizes")
}

fn kind_of(r: &kayfabe_device::DoorbellReport) -> &'static str {
    r.refusal()
        .unwrap_or_else(|| panic!("expected a named refusal, got {r:?}"))
        .kind
        .0
}

// =====================================================================================
// THE ACCEPTANCE
// =====================================================================================

/// ★★★ **A guest MMIO write to the doorbell aperture reaches
/// `kayfabe_rt::SharedDevice::doorbell`, and comes back with a named `FwdFault`.**
///
/// The routing chain this asserts, end to end:
/// `Regs::write` → `RegPlane::write` (doorbell classification) → `SharedDoorbell::ring` →
/// `SharedDevice::doorbell` → `kayfabe_fwd::route_doorbell` → `Ga10xArch::decode_doorbell`
/// (E3) → the spine's `by_vchid` map → **miss** → `FwdFault::UnknownVchid`.
///
/// ★ `UnknownVchid` and not merely "some refusal": it is the fault that can only be reached
/// **after** a successful token decode and a real lookup in a real spine. A shim that never
/// called the core cannot produce it, and neither can one whose token decode refused.
#[test]
fn a_guest_write_to_the_doorbell_aperture_reaches_the_core_and_is_named() {
    let r = regs();
    let out = r.write(BAR_REGS, DOORBELL, 4, GOOD_TOKEN);

    let report = out
        .doorbell
        .as_ref()
        .expect("★ the write must be CLASSIFIED as a doorbell");
    assert_eq!(report.token(), GOOD_TOKEN);
    assert_eq!(
        kind_of(report),
        "FwdFault::UnknownVchid",
        "★ the kind must come from kayfabe-fwd's own exhaustive Faulted match, which is \
         unreachable without a real route through a real spine"
    );
    assert_ne!(kind_of(report), NO_PORT, "the port is WIRED, not defaulted");
    // ★ The sentence carries the payload, so an operator reading one line learns which
    // channel was asked for and on which GPU — not merely that something was refused.
    let why = &report.refusal().expect("named").why;
    assert!(why.contains("UnknownVchid"), "{why}");
    assert!(why.contains("VChid(5)"), "the DECODED channel, {why}");

    // …and it is COUNTED, which is the other half of the acceptance's wording.
    let a = r.audit();
    assert_eq!(a.doorbells, 1);
    assert_eq!(a.doorbells_refused, 1);
    assert_eq!(a.doorbells_served, 0);
    assert_eq!(a.doorbell_last_token, GOOD_TOKEN);
    assert_eq!(a.doorbell_last_token_valid, 1);
    assert_eq!(a.doorbell_refusal.present, 1);
    let klen = usize::try_from(a.doorbell_refusal.kind_len).expect("fits");
    assert_eq!(&a.doorbell_refusal.kind[..klen], b"FwdFault::UnknownVchid");
}

/// ★★★ **THE CONTROL.** In the **same run**, on the **same** `Regs`, a set of non-doorbell
/// BAR writes produce **neither** a `DoorbellOutcome` nor a `FwdFault` — no report, and no
/// movement in any of the three counters.
///
/// ★ Quantified over a list, and the list is chosen so that no entry is a free pass: two
/// offsets that are 4 bytes away from the doorbell, one that is the *same offset on another
/// aperture*, one register the device really does model (the BAR0 window latch), and one it
/// models nothing at.
#[test]
fn non_doorbell_writes_in_the_same_run_produce_neither() {
    let r = regs();

    let controls: &[(u32, u64, &str)] = &[
        (BAR_REGS, DOORBELL - 4, "the dword below the doorbell"),
        (BAR_REGS, DOORBELL + 4, "the dword above it"),
        (BAR_INST, DOORBELL, "the SAME offset on the instance window"),
        (BAR_REGS, 0x0000_1700, "the BAR0 moving window's own latch"),
        (BAR_REGS, 0x0000_1000, "an offset nothing claims"),
    ];
    for &(bar, off, what) in controls {
        let out = r.write(bar, off, 4, GOOD_TOKEN);
        assert!(
            out.doorbell.is_none(),
            "{what} (bar {bar}, +{off:#x}) must produce NEITHER answer"
        );
    }
    let a = r.audit();
    assert_eq!(
        (a.doorbells, a.doorbells_served, a.doorbells_refused),
        (0, 0, 0),
        "★ {} non-doorbell writes moved a doorbell counter",
        controls.len()
    );
    assert_eq!(a.doorbell_last_token_valid, 0, "and nothing was recorded");
    assert_eq!(a.doorbell_refusal.present, 0);

    // ★★ Non-vacuity: the very same `Regs`, one write later, DOES ring. Without this the
    // control above would be satisfied by a device that classifies nothing at all.
    let out = r.write(BAR_REGS, DOORBELL, 4, GOOD_TOKEN);
    assert!(out.doorbell.is_some());
    assert_eq!(r.audit().doorbells, 1);
}

/// ★★★ Two different guest tokens produce two **different** named refusals, from two
/// different points in the chain.
///
/// This is the assertion a constant answer cannot imitate. `MALFORMED_TOKEN` sets bit 12,
/// which lies in the gap between `VECTOR` (11:0) and `RUNLIST_ID` (22:16) and which RM's own
/// encoder cannot write — so `Ga10xArch::decode_doorbell` answers `None` and the chain stops
/// at `FwdFault::MalformedToken`, one step **earlier** than `GOOD_TOKEN` gets to.
///
/// ⊘ The two must not collapse: *"that is not a token"* and *"that is a token for a channel
/// nobody allocated"* are different diagnoses, and only the second one becomes a success
/// once E5 populates the routing map.
#[test]
fn a_malformed_token_and_a_routable_one_refuse_differently() {
    let r = regs();

    let bad = r.write(BAR_REGS, DOORBELL, 4, MALFORMED_TOKEN);
    let good = r.write(BAR_REGS, DOORBELL, 4, GOOD_TOKEN);

    let bad = bad.doorbell.as_ref().expect("a doorbell");
    let good = good.doorbell.as_ref().expect("a doorbell");
    assert_eq!(kind_of(bad), "FwdFault::MalformedToken");
    assert_eq!(kind_of(good), "FwdFault::UnknownVchid");
    assert_ne!(kind_of(bad), kind_of(good));

    let a = r.audit();
    assert_eq!((a.doorbells, a.doorbells_refused), (2, 2));
    // ⊘ The FIRST refusal is the one kept, so a later, different refusal must not overwrite
    // the diagnosis — while the last TOKEN is still the last one.
    let klen = usize::try_from(a.doorbell_refusal.kind_len).expect("fits");
    assert_eq!(
        &a.doorbell_refusal.kind[..klen],
        b"FwdFault::MalformedToken"
    );
    assert_eq!(a.doorbell_last_token, GOOD_TOKEN);
}

/// ⊘ **Token zero is a legal ring**, and the audit's `_valid` flag is what says so.
///
/// Runlist 0, channel 0 is a token RM really emits. A device that used `0` as its "never
/// rang" sentinel would report this guest's submission as an absence — which, in a boot log
/// read at 2am, is indistinguishable from the transport being broken.
#[test]
fn a_token_of_zero_is_reported_as_a_ring_not_as_an_absence() {
    let r = regs();
    assert_eq!(r.audit().doorbell_last_token_valid, 0);

    let out = r.write(BAR_REGS, DOORBELL, 4, 0);
    assert_eq!(out.doorbell.as_ref().expect("a doorbell").token(), 0);

    let a = r.audit();
    assert_eq!(a.doorbell_last_token, 0);
    assert_eq!(
        a.doorbell_last_token_valid, 1,
        "★ the flag is the only thing that separates 'rang channel 0' from 'never rang'"
    );
    assert_eq!(a.doorbells, 1);
}

/// The three counters are exact and none absorbs another, over a mixed run that also
/// contains ordinary register traffic.
#[test]
fn the_counters_are_exact_over_a_mixed_run() {
    let r = regs();
    for i in 0..7u64 {
        let _ = r.write(BAR_REGS, DOORBELL, 4, i);
        // ordinary traffic between the rings, so the doorbell path is not the only thing
        // the plane is doing
        let _ = r.write(BAR_REGS, 0x0000_1000 + i * 4, 4, i);
        let _ = r.read(BAR_REGS, 0x0000_0000, 4);
    }
    let a = r.audit();
    assert_eq!(a.doorbells, 7);
    assert_eq!(a.doorbells_served + a.doorbells_refused, a.doorbells);
    assert_eq!(
        a.doorbells_served, 0,
        "⊘ nothing can be SERVED yet: no channel exists on the spine until E4-E6"
    );
    assert_eq!(a.doorbell_last_token, 6);
}

/// ★★ A power-on reset clears what this device life saw, so the next guest's report cannot
/// carry the previous one's token.
#[test]
fn a_device_reset_clears_the_doorbell_report() {
    let r = regs();
    let _ = r.write(BAR_REGS, DOORBELL, 4, GOOD_TOKEN);
    assert_eq!(r.audit().doorbell_last_token_valid, 1);

    r.reset();
    let a = r.audit();
    assert_eq!(a.doorbell_last_token_valid, 0);
    assert_eq!(a.doorbell_refusal.present, 0);

    // ⊘ …and the port survives, because it is the composition root's wiring and a reset is
    // the guest's event. A reset that silently unwired the plane would make every ring
    // after it read as `Device::NoDoorbellPort`, i.e. as a missing shell.
    let out = r.write(BAR_REGS, DOORBELL, 4, GOOD_TOKEN);
    assert_eq!(
        kind_of(out.doorbell.as_ref().expect("a doorbell")),
        "FwdFault::UnknownVchid"
    );
}

// =====================================================================================
// ★★★ E6 (debt Q24) — THE PROPERTY, NOW ASSERTED BY RUNNING
// =====================================================================================

/// ★★★ **The doorbell port and the object bridge reach the SAME `Gpu`** — measured, not
/// counted in this file's own source.
///
/// # The debt, in E2's own words
///
/// > *The behavioural witness would be: declare a channel through the bridge, ring its
/// > vChid through the doorbell, watch `FwdFault::UnknownVchid` become a served outcome.
/// > … It is therefore an **E6** assertion.*
///
/// This is it. [`Regs::object_model`] hands back the *same* `Arc<SharedDevice>` the boxed
/// object policy declares into and the *same* one the doorbell port rings, so the channel
/// declared below and the token rung below cross the real join.
///
/// # ★★★ What makes it a WITNESS rather than a tautology
///
/// The discriminator is **which** refusal comes back, and the two candidates are on
/// opposite sides of the routing lookup:
///
/// - `FwdFault::UnknownVchid` — the spine's `by_vchid` had no entry. That is what a
///   **second** `Gpu` behind the doorbell produces, forever, with nothing else going red.
/// - anything downstream of it — the route **resolved**, so the doorbell found the channel
///   the bridge declared. Only one object model can produce that.
///
/// ★ **The token is `0` because the channel below DECLARES chid 0**, and that is now a
/// choice rather than a floor. It used to be forced: `Ga10xArch::vchid_from_userd_flags`
/// answered `VChid(0)` for every channel — a stated refusal — so `VChid(0)` was the only
/// vChid a GA10x channel could be filed under. That refusal is gone; the channel's
/// `userd_flags` below is the word CPU-RM would put on the wire for chid 0, built with
/// `MockArch::userd_flags_for`, whose encoding `tests/tests/userd_chid_oracle.rs`
/// differentials against NVIDIA's own compiled writer. `0x0000_0000` is the token RM's own
/// encoder emits for `(runlist 0, chid 0)`, so the two halves still meet.
///
/// ⊘ What this still does NOT establish is unchanged: a real guest's scrubber channel is
/// not chid 0 in general, and nothing here is a live boot (`only_live_boots_are_proof`).
/// What the USERD decode bought is that a channel at ANY chid would now route; this
/// fixture exercises one of them.
#[test]
fn the_doorbell_reaches_the_same_object_model_the_bridge_declares_into() {
    use kayfabe_arch::ClientKind;
    use kayfabe_arch::ids::{ClassId, HClient, HObject, Pdb};
    use kayfabe_core::rmgraph::{AllocFacts, RmEvent};

    use kayfabe_fwd::FwdFault;

    let r = regs();
    let dev = r.object_model();

    // ---- non-vacuity FIRST: with nothing declared, the ring misses the routing map.
    let before = r.write(BAR_REGS, DOORBELL, 4, 0);
    assert_eq!(
        kind_of(before.doorbell.as_ref().expect("a doorbell")),
        "FwdFault::UnknownVchid",
        "★ so the change below is the DECLARATION's doing and not the fixture's"
    );

    // ---- the guest declares a GA10x process with one channel, through the bridge's own
    //      object model. NVIDIA's real class ids, because `Ga10xArch` classifies those.
    const CLIENT: HClient = HClient(0x5c00_0000);
    const PDB: Pdb = Pdb(0x4E60_0000);
    let h = |off: u32| HObject(0x5c00_0000 + off);
    let (root, device, vas, tsg, chan) = (h(0), h(1), h(0x10), h(0x12), h(0x19));
    use kayfabe_abi::generated::classes as nv;
    for ev in [
        RmEvent::Alloc {
            client: CLIENT,
            parent: root,
            handle: root,
            class: ClassId(nv::NV01_ROOT),
            facts: AllocFacts {
                client_kind: Some(ClientKind::User { pid: CLIENT.0 }),
                ..Default::default()
            },
        },
        RmEvent::Alloc {
            client: CLIENT,
            parent: root,
            handle: device,
            class: ClassId(nv::NV01_DEVICE_0),
            facts: AllocFacts {
                device_instance: Some(0),
                ..Default::default()
            },
        },
        RmEvent::Alloc {
            client: CLIENT,
            parent: device,
            handle: vas,
            class: ClassId(nv::FERMI_VASPACE_A),
            facts: AllocFacts::default(),
        },
        RmEvent::SetPageDir {
            client: CLIENT,
            vaspace: vas,
            pdb: PDB,
            // ⊘ The TEST default; the PRODUCTION path must never assume it.
            pdb_aperture: Some(kayfabe_arch::Aperture::Vidmem),

        },
        RmEvent::Alloc {
            client: CLIENT,
            parent: device,
            handle: tsg,
            class: ClassId(nv::KEPLER_CHANNEL_GROUP_A),
            facts: AllocFacts {
                h_vaspace: Some(vas),
                ..Default::default()
            },
        },
        RmEvent::Alloc {
            client: CLIENT,
            parent: tsg,
            handle: chan,
            class: ClassId(nv::AMPERE_CHANNEL_GPFIFO_A),
            facts: AllocFacts {
                h_vaspace: Some(vas),
                // ★ The `NVOS04_FLAGS` word CPU-RM writes for chid 0. `AllocFacts::default()`
                // (a zero word) would name NO channel at all — RM's own reader leaves the
                // chid to the allocator when `_PAGE_FIXED` is clear — and the projection
                // refuses it by name (`ProjectionError::UnnamedVchid`). A channel has to
                // declare a chid to be routable, which is the point of the decode.
                userd_flags: kayfabe_mocks::MockArch::userd_flags_for(kayfabe_arch::ids::VChid(0)),
                ..Default::default()
            },
        },
        // ★★★★ **§16.65 — THE ENGINE OBJECT, and it is load-bearing as of this rung.**
        //
        // Without it this channel is `EngineKind::GrCompute`, because there is exactly ONE
        // GPFIFO class per architecture and `Ga10xArch::classify` must default
        // (`kayfabe-chips/src/ga10x.rs:161`: *"a CE channel becomes one only when its
        // `AMPERE_DMA_COPY_B` engine object arrives and the core's refinement pass rewrites
        // it"*). `SharedDoorbell::try_ce_submission` now routes on that field, so a
        // GR-labelled channel is refused `Route::NotACopyEngineChannel` **before** it can
        // reach the isolate plane — and this test's whole subject is the refusal that is
        // *downstream of routing*.
        //
        // ⊘⊘ It is spelled out here rather than quietly fixed because the fixture's
        // previous shape was itself the discovery: **a channel's `EngineKind` is a fact
        // with a LIFETIME**, `GrCompute` until its engine object lands. An end-of-boot
        // census that reports a channel as `Ce` says nothing about what it was when its
        // first doorbell rang. See this rung's report.
        RmEvent::Alloc {
            client: CLIENT,
            parent: chan,
            handle: h(0x1A),
            class: ClassId(nv::AMPERE_DMA_COPY_B),
            facts: AllocFacts::default(),
        },
    ] {
        dev.apply(ev).expect("the bridge's object model accepts it");
    }

    // ★ #177 — the guest schedules before it rings; this test's subject is the join and
    // the isolate plane downstream of routing, not the scheduling gate itself.
    dev.schedule_channel(CLIENT, chan, true)
        .expect("the guest schedules the channel it just declared");

    // ⊘⊘ REWRITTEN 2026-09-09 (w392p birth-at-alloc). What stood here: the doorbell was
    // asserted to refuse `FwdFault::IsolateRetired` — the isolate-plane refusal — because
    // the DOORBELL used to be where a passthrough host channel was born. As of w392p the
    // host channel is born at the guest's CHANNEL ALLOC: `SharedDevice::apply` latches the
    // birth and the shim's `Regs::write` tail drains it (`report_channel_birth_drain` →
    // `run_pending_channel_births`) — in production, in the tail of the RPC write that
    // carried the alloc, BEFORE any doorbell. `dev.apply` above is the bridge's object
    // model called directly, so no write tail has run yet: drain the latch here, in the
    // order production does. The refusal is now met one verb earlier, at the birth, and
    // it is still DOWNSTREAM OF ROUTING: `route_channel_birth` resolved the channel first,
    // and the refusal names the routed proc and chan. ⊘ Measured 2026-09-09: this fixture
    // declares no ring (`gp_fifo_ring: None`), so `plan_channel_birth` refuses
    // `PassthroughRingNotAdoptable { ring_va: None }` BEFORE the isolate plane is asked —
    // the isolate-plane witness (`IsolateRetired`) is no longer reachable from a fixture
    // with no adoptable ring; it lives with the birth tests in `tests/tests/`.
    let births = dev.run_pending_channel_births(&[]);
    assert_eq!(
        births.len(),
        1,
        "★ exactly the one channel declared above was latched for birth: {births:?}"
    );
    assert_eq!((births[0].client, births[0].channel), (CLIENT, chan));
    assert!(
        matches!(
            births[0].out,
            Err(FwdFault::PassthroughRingNotAdoptable { ring_va: None, .. })
        ),
        "★ the route resolved (the refusal names the routed proc/chan) and the birth was \
         refused BY NAME — no ring declared, so nothing to adopt — at the birth, which is \
         where the passthrough host channel is made as of w392p: {:?}",
        births[0].out
    );

    // ---- ★ THE WITNESS: the same token, now routed.
    let after = r.write(BAR_REGS, DOORBELL, 4, 0);
    let report = after.doorbell.as_ref().expect("a doorbell");
    let kind = kind_of(report);
    assert_ne!(
        kind, NO_PORT,
        "the port is the one the composition root installed"
    );
    assert_ne!(
        kind, "FwdFault::UnknownVchid",
        "★★★ THE DEBT. The doorbell still cannot see the channel the bridge declared, \
         which is exactly the shape a SECOND `Gpu` behind the port produces — and it is \
         invisible to every other test in this crate."
    );
    // …and the refusal it DOES give is the NEW contract's: a doorbell never births a
    // passthrough channel (`kayfabe_fwd::plan_doorbell` refuses it BY NAME), so with the
    // alloc-time birth refused above the channel is routed — not `UnknownVchid` — and the
    // doorbell is refused as unborn, never silently born over our own ring.
    assert_eq!(
        kind, "FwdFault::PassthroughDoorbellBirth",
        "★ the route resolved and the doorbell refused BY NAME: a doorbell on a routed but \
         unborn passthrough channel is `PassthroughDoorbellBirth` (w392p) — the isolate-plane \
         refusal now lives at the birth, asserted above"
    );

    let a = r.audit();
    assert_eq!((a.doorbells, a.doorbells_refused), (2, 2));
    assert_eq!(
        a.doorbells_served, 0,
        "⊘ still zero, and correctly: the shipped default plane serves no verb"
    );
}

// =====================================================================================
// ★★★ THE PROPERTY THIS FILE CANNOT ASSERT BY RUNNING, AND HOW IT IS GUARDED INSTEAD
// =====================================================================================

/// ★★★ **The archive realizes exactly ONE object model**, and both the object bridge and
/// the doorbell port are handles onto it.
///
/// # ⊘ Why this is a source check and not a behavioural one, stated plainly
///
/// The behavioural witness would be: declare a channel through the bridge, ring its vChid
/// through the doorbell, watch `FwdFault::UnknownVchid` become a served outcome. That needs
/// a channel on the spine, which needs an `RmEvent` chain the composition root has no seam
/// to inject and which `kayfabe-mocks`' `Scenario` cannot build here (this crate must never
/// depend on the mocks, and `Ga10xArch` classifies real NVIDIA class ids, not the mock's).
/// It is therefore an **E6** assertion, and pretending otherwise here would be the worse
/// option: a green that does not mean what it appears to.
///
/// What *can* be checked mechanically is the thing that would break — a **second** `Gpu`.
/// If the doorbell ever rang a model the bridge does not declare into, this port would have
/// a routing table that can never resolve, and nothing anywhere would go red: the acceptance
/// above would still see `FwdFault::UnknownVchid`, forever. So this test quantifies over the
/// crate's own source, as `wire_mirror.rs` already does for the ABI: **one** `Gpu::new`,
/// **one** `SharedDevice::new`, and every consumer built from a clone of that one handle.
#[test]
fn the_archive_realizes_exactly_one_object_model() {
    let src = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/shim.rs");
    let text = std::fs::read_to_string(&src).expect("the shim's source is readable");
    // ⊘ Comments are stripped first: this file's own prose says "Gpu::new" and a naive
    // count would be satisfied by a doc rewrite. A gate that a comment can turn green is
    // not a gate.
    let code: String = text
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(
        code.matches("Gpu::new(").count(),
        1,
        "★ a SECOND object model in the composition root is the one regression that would \
         leave the doorbell permanently unroutable with nothing going red"
    );
    assert_eq!(
        code.matches("SharedDevice::new(").count(),
        1,
        "★ one shell, so the bridge and the doorbell cannot be handed different ones"
    );
    // ★ And both consumers are built from a CLONE of that one handle, by name.
    assert!(
        code.contains("SharedObjectModel(Arc::clone(&device))"),
        "the object bridge must declare into the shared handle"
    );
    // ★ The doorbell port grew a second field (a `Weak<RegPlane>`, for the addressing
    // probe), so the spelling moved from a tuple struct to a named one. The PROPERTY is
    // unchanged and is what is asserted: its `device` is a clone of the one handle above,
    // not a second shell.
    assert!(
        code.contains("SharedDoorbell {") && code.contains("device: Arc::clone(&device),"),
        "…and the doorbell port must ring the same one"
    );
    // ⊘ And the plane handle it now also holds must be WEAK. The plane owns this port; a
    // strong handle is a cycle that never frees, and nothing else in the archive would say
    // so — the device would simply leak one register plane per realize.
    assert!(
        code.contains("plane: Arc::downgrade(&plane),"),
        "the doorbell port's back-reference to its own plane must be weak"
    );
}

/// ★★★★ **§16.65 — a GR channel's doorbell is refused BY THE ROUTING FACT, and a CE
/// channel's is not.** The first test of any kind over `SharedDoorbell::try_ce_submission`.
///
/// # ⊘ What was wrong
///
/// `[measured 2026-08-10, boots s49/s50]` the shell's CPU **copy-engine** executor claimed
/// every doorbell whose channel had a VA space and a ring — `Ce` and `GrCompute` alike —
/// because the only gate in front of it asked about the isolate *plane*, never the
/// *engine*, and on the shipping `Stillborn` configuration that gate never fires. The
/// defect was **86 doorbells wide** (`[measured 2026-08-10, boot s51_d502ac6_engroute]`:
/// `GrCompute=86 Ce=362`, summing to 448).
///
/// ⊘ Nothing was forged, and that is exactly why nothing caught it — a GR ring decodes to
/// `Opaque` at a class-gated codec, so the refusal was *true of the bytes* and *silent
/// about the cause*.
///
/// ⊘ **The `SubmissionHasNoLaunch { methods: 3, opaque: 2 }` this doc used to name as the
/// symptom was NOT one of the 86.** Its own printed pushbuffer is `SET_OBJECT →
/// AMPERE_DMA_COPY_B`: a CE push on a CE channel at the CE executor. It was §16.66's
/// four-word semaphore release, and it survived `w202` unchanged. A test asserting "it refuses" would have passed
/// throughout. This one asserts **which name**, because the name is the whole increment.
///
/// # ★★★★ The non-vacuity, and it is the finding this rung actually turned up
///
/// The two halves differ by **one event**: the `AMPERE_DMA_COPY_B` engine object. There is
/// one GPFIFO class per architecture, so `Ga10xArch::classify` labels every channel
/// `GrCompute` and only the engine-object refinement rewrites it
/// (`kayfabe-chips/src/ga10x.rs:161`). ⇒ **A channel's `EngineKind` is a fact with a
/// LIFETIME.** A CE channel is `GrCompute` from its alloc until its engine object lands,
/// and an end-of-boot census reporting it as `Ce` cannot say what it was when its first
/// doorbell rang. That is the difference this test makes visible, on purpose.
#[test]
fn a_gr_channel_is_refused_by_route_and_the_engine_object_is_what_moves_it() {
    use kayfabe_abi::generated::classes as nv;
    use kayfabe_arch::ClientKind;
    use kayfabe_arch::ids::{ClassId, HClient, HObject, Pdb, VChid};
    use kayfabe_core::rmgraph::{AllocFacts, RmEvent};

    // ⊘ One fixture, parameterised by the single event under study, so the two outcomes
    // cannot differ by anything else. A second hand-written fixture could drift.
    let ring =
        |with_engine_object: bool| -> (String, kayfabe_rt::completion_watch::WatchStats, String) {
            let r = regs();
            let dev = r.object_model();
            const CLIENT: HClient = HClient(0x5c00_0000);
            const PDB: Pdb = Pdb(0x4E60_0000);
            let h = |off: u32| HObject(0x5c00_0000 + off);
            let (root, device, vas, tsg, chan) = (h(0), h(1), h(0x10), h(0x12), h(0x19));
            let mut events = vec![
                RmEvent::Alloc {
                    client: CLIENT,
                    parent: root,
                    handle: root,
                    class: ClassId(nv::NV01_ROOT),
                    facts: AllocFacts {
                        client_kind: Some(ClientKind::User { pid: CLIENT.0 }),
                        ..Default::default()
                    },
                },
                RmEvent::Alloc {
                    client: CLIENT,
                    parent: root,
                    handle: device,
                    class: ClassId(nv::NV01_DEVICE_0),
                    facts: AllocFacts {
                        device_instance: Some(0),
                        ..Default::default()
                    },
                },
                RmEvent::Alloc {
                    client: CLIENT,
                    parent: device,
                    handle: vas,
                    class: ClassId(nv::FERMI_VASPACE_A),
                    facts: AllocFacts::default(),
                },
                RmEvent::SetPageDir {
                    client: CLIENT,
                    vaspace: vas,
                    pdb: PDB,
                    // ⊘ The TEST default; the PRODUCTION path must never assume it.
                    pdb_aperture: Some(kayfabe_arch::Aperture::Vidmem),

                },
                RmEvent::Alloc {
                    client: CLIENT,
                    parent: device,
                    handle: tsg,
                    class: ClassId(nv::KEPLER_CHANNEL_GROUP_A),
                    facts: AllocFacts {
                        h_vaspace: Some(vas),
                        ..Default::default()
                    },
                },
                RmEvent::Alloc {
                    client: CLIENT,
                    parent: tsg,
                    handle: chan,
                    class: ClassId(nv::AMPERE_CHANNEL_GPFIFO_A),
                    facts: AllocFacts {
                        h_vaspace: Some(vas),
                        userd_flags: kayfabe_mocks::MockArch::userd_flags_for(VChid(0)),
                        ..Default::default()
                    },
                },
            ];
            if with_engine_object {
                events.push(RmEvent::Alloc {
                    client: CLIENT,
                    parent: chan,
                    handle: h(0x1A),
                    class: ClassId(nv::AMPERE_DMA_COPY_B),
                    facts: AllocFacts::default(),
                });
            }
            for ev in events {
                dev.apply(ev).expect("the bridge's object model accepts it");
            }
            dev.schedule_channel(CLIENT, chan, true)
                .expect("the guest schedules the channel it just declared");
            // ★ w392p (rewritten 2026-09-09): the passthrough host channel is born at the
            // guest's alloc, in the tail of the register write that carried it — before any
            // doorbell. `dev.apply` bypasses that tail, so drain the latch here, in production's
            // order. See `the_doorbell_reaches_the_same_object_model_the_bridge_declares_into`.
            let births = dev.run_pending_channel_births(&[]);
            assert_eq!(
                births.len(),
                1,
                "one channel, one latched birth: {births:?}"
            );
            let birth = format!("{:?}", births[0].out);
            let after = r.write(BAR_REGS, DOORBELL, 4, 0);
            (
                kind_of(after.doorbell.as_ref().expect("a doorbell")).to_string(),
                r.completion_watch().stats(),
                birth,
            )
        };

    assert_eq!(
        ring(false).0,
        "Route::NotACopyEngineChannel",
        "★ a GR-labelled channel's doorbell must be refused by the ROUTING fact — not \
         handed to the copy-engine codec to decline by the shape of bytes it was never \
         meant to read",
    );
    // ⊘⊘ REWRITTEN 2026-09-09 (w392p birth-at-alloc). What stood here was
    // `FwdFault::IsolateRetired` — the isolate-plane refusal the DOORBELL birth used to
    // reach. That refusal is now met at the channel's alloc-time birth (the third tuple
    // element — `PassthroughRingNotAdoptable`, this fixture declaring no ring), and the
    // doorbell on the unborn channel is refused BY NAME.
    let ce = ring(true);
    assert!(
        ce.2.starts_with("Err(PassthroughRingNotAdoptable"),
        "★ the CE channel's alloc-time birth was refused BY NAME, downstream of routing \
         (`route_channel_birth` resolved it first; this fixture declares no ring, so there \
         is nothing to adopt — measured 2026-09-09): {}",
        ce.2
    );
    assert_eq!(
        ce.0, "FwdFault::PassthroughDoorbellBirth",
        "★ and a CE channel is untouched by the ROUTING gate: it falls through routing to \
         the birth gate and is refused there BY NAME (a doorbell never births a passthrough \
         channel, w392p), which is what 'additive' has to MEAN — the GR arm refuses by \
         ROUTE, the CE arm downstream of it",
    );
    // ⊘ Non-vacuity: the two names really are different, so the fixture's single varied
    // event is what decides — and the gate is not answering the same thing to everything.
    assert_ne!(ring(false).0, ring(true).0);

    // ★★★★★ **THE COMPLETION OBSERVER IS REACHED — and only on the arm that walls.**
    //
    // `docs/design/completion_wait_architecture.md` §4(b) is about a correct observer that
    // no guest action could reach. This is the reachability half of that, asserted at the
    // CALLER: a `GrCompute` doorbell — the one `cuCtxCreate` rings 86 times while it spins
    // on `SET_REPORT_SEMAPHORE` — must ENTER the declare path, and a `Ce` doorbell (which
    // is served by the shell's own executor and completes there) must not.
    //
    // ⊘ `attempts`, not `declared`, and the distinction is the whole point: this fixture
    // attaches no guest memory, so nothing CAN be declared. *"The observer was never
    // reached"* and *"the observer was reached and had nothing to read"* are the two
    // readings a `declared` count cannot separate, and only the first is a severance.
    let gr = ring(false).1;
    let ce = ring(true).1;
    assert_eq!(
        gr.attempts, 1,
        "★★★ THE SEVERANCE: a GR doorbell must reach the completion observer's declare \
         path. Saw {gr:?}",
    );
    assert_eq!(
        gr.declared, 0,
        "⊘ and it must declare NOTHING here — this fixture has no memory plane, so a \
         declaration would mean the observer invented one. Saw {gr:?}",
    );
    assert_eq!(
        ce.attempts, 0,
        "⊘ the CE arm is untouched: it is served by the shell's own executor above the \
         routing gate and never reaches this path. Saw {ce:?}",
    );
}

/// ★★★★★ **THE STRUCTURAL GUARD — the constraint as a test, not a sentence.**
///
/// The owner has stated the same two rules repeatedly and they were violated anyway, twice in
/// one day, by two different authors, in reviewed and tested code:
/// *"no vas publish in doorbells"* and *"vCPU thread no allowance for such blocking things"*.
///
/// `OffVcpu` makes both a compile error. This test guards the ONE remaining way to undo that:
/// minting the witness somewhere new. ⊘ It is a source-text census on purpose — the same shape
/// as `guest_ring_census` — because the property is *"where may this appear"*, and no runtime
/// assertion can answer that.
#[test]
fn the_publication_capability_is_minted_in_exactly_one_place() {
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shim.rs"),
    )
    .expect("shim.rs is readable");
    // ⊘ Comment lines are excluded: the audit is about CODE, and the doc on the constructor
    // names the function repeatedly on purpose.
    let mints = src
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .filter(|l| l.contains("for_publication_worker"))
        .count();
    assert_eq!(
        mints, 2,
        "`OffVcpu::for_publication_worker` must appear exactly twice in code — its own \
         definition and the ONE call in the publication worker's loop. A third occurrence is \
         somebody handing the doorbell or a vCPU the right to publish, which is the exact \
         regression this capability exists to make impossible. If a second worker genuinely \
         needs it, that is a DECISION: change this number in the same commit and say which \
         thread it runs on and why it is not a vCPU."
    );
}

/// ★★★★★ **THE WORKER'S ARM MUST REACH THE GUEST-RAM PIN PASS.**
///
/// `[measured w415llm]` the publication worker forced `VasPublishArm::Publish`, whose
/// `measures_pin_rate()` is **false** — so `measure_guest_ram_pin_rate`, the only pass that pins
/// guest-RAM rows for anything but a channel's ring, **never ran**. The boot logged `NO DRAIN`
/// zero times. The LLM's operand address was then `ABSENT-FROM-ROOT-TABLE` and `CE3_PBDMA0`
/// faulted reading it.
///
/// ⊘ A source census, because the property is *"which arm does the worker choose"* and no unit
/// test reaches that line without a device. ⚠ If a later change needs `Publish` there, it must
/// also give guest-RAM rows another route — and change this test in the same commit.
#[test]
fn the_publication_worker_uses_an_arm_that_also_pins_guest_ram() {
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shim.rs"),
    )
    .expect("shim.rs is readable");
    let code: String = src
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        code.matches("ctx.vas_publish = VasPublishArm::Publish;").count(),
        0,
        "the worker must NOT force `Publish`: it publishes framebuffer leaves and nothing else, \
         and its measures_pin_rate() is false, so it silently disables the only pass that pins \
         guest-RAM operand rows"
    );
    assert_eq!(
        code.matches("ctx.vas_publish = VasPublishArm::Drain;").count(),
        3,
        "all THREE worker lanes must use an arm that publishes AND pins: rpc-bind, \
         invalidate, and — since w559 — CHANNEL BIRTH. ⊘ The third was added because a \
         channel that returns to the guest is a channel the guest may ring, and `[measured \
         w557, LLM boot]` one did: `CE2_PBDMA0` took `Xid 31 … FAULT_PDE` reading its own \
         GPFIFO ring at an address our table binds and our own ADOPT-WHY line calls ADOPTABLE \
         sixty-four times. ⚠ This count is the acknowledgement this test's own doc demands — \
         a fourth lane must be argued for here, not appear here."
    );
}

/// ★★★★★ **SYNCHRONIZATION POINT (3) MUST BE CONSUMED, INSTALLED, AND ORDERED BEFORE THE
/// FORWARD.**
///
/// The guest's UVM announces every page-table change it makes as a channel
/// `MEM_OP MMU_TLB_INVALIDATE`. `[measured w428]` we decoded it into `out.invalidates` and read
/// it in **no production code** — only four tests — so the rows were found later by a sweep,
/// and `[measured w425]` the pin of the LLM's faulting range landed in the SAME SECOND as the
/// `FAULT_PDE ACCESS_TYPE_VIRT_WRITE` that hit it.
///
/// ⊘ A source census over THREE properties, because each has failed on its own this session:
/// a mechanism can be written (1), never installed (2) — four times — or installed but
/// consulted after the thing it was meant to gate (3).
#[test]
fn the_channel_invalidate_is_consumed_installed_and_ordered_before_the_forward() {
    let dev = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../kayfabe-rt/src/device.rs"),
    )
    .expect("device.rs is readable");
    let shim = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shim.rs"),
    )
    .expect("shim.rs is readable");

    // (1) CONSUMED — the decoded invalidate drives a refresh.
    assert!(
        dev.contains("refresh.refresh_pdb(pid, *pdb)"),
        "`parsed.invalidates` must drive a refresh; decoding it and dropping it is the bug \
         this replaced"
    );

    // (2) INSTALLED — a seam nobody installs is the session's most-repeated failure.
    assert!(
        shim.contains("set_invalidate_refresh("),
        "the shim must INSTALL the refresh seam: four mechanisms this session were built, \
         wired, tested and never reached"
    );

    // (3) ORDERED — the refresh must precede the forward in the SAME function, or it does not
    // block anything. ⚠ This is the assertion that makes the other two mean something.
    let at_refresh = dev
        .find("refresh.refresh_pdb(pid, *pdb)")
        .expect("checked above");
    let at_forward = dev
        .find("let fwd = self.forward_ce(pid, cid, &parsed.ce_spans)?;")
        .expect("the ring forward is still where it was");
    assert!(
        at_refresh < at_forward,
        "the refresh must come BEFORE the forward. The guest's completion semaphore is in the \
         same pushbuffer as the invalidate, so the engine cannot reach it until we forward — \
         that ordering IS the block, and reversing it removes the barrier while leaving every \
         other line in place"
    );
}

/// ★★★★★ **ALL THREE ENTRY POINTS MUST CONSUME, AND THE CLIENT GRADES ON ALL THREE.**
///
/// > **Owner, 2026-09-11:** *"remember all three entrypoints need to work to get raw client
/// > passing"*
///
/// The mean client's ladder IS the three entry points:
///
/// ```text
///   P1 rm-invalidate  -> (1) the TLB-invalidate register
///   P2 uvm-memop      -> (3) the channel MEM_OP
///   P3 rpc-bind       -> (2) GPU_PROMOTE_CTX, which nobody announces because we ARE the GSP
/// ```
///
/// ⊘ (2) is the one with no barrier coming: on a GSP part the mapping happens inside the GSP
/// and a real one invalidates in microcode, so the only thing that knows is the handler.
#[test]
fn all_three_synchronization_points_consume_their_barrier() {
    let dev = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../kayfabe-rt/src/device.rs"),
    )
    .expect("device.rs is readable");
    let shim = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shim.rs"),
    )
    .expect("shim.rs is readable");

    // (3) the channel MEM_OP — consumed in forward_ring, before the forward.
    assert!(
        dev.contains("refresh.refresh_pdb(pid, *pdb)"),
        "entry point (3): the channel TLB invalidate must drive a refresh"
    );

    // (2) the RM call — consumed in promote_ctx, before the reply.
    assert!(
        dev.contains("refresh.refresh_pdb(route.proc, route.pdb)"),
        "entry point (2): GPU_PROMOTE_CTX must BACK the rows it binds. Nothing will announce \
         them — on a GSP part the mapping happens inside the GSP and a real one invalidates \
         in microcode, so this handler is the only thing that knows"
    );

    // ⚠ (2) REACHABILITY, not just presence. `promote_ctx` exists on BOTH `Gpu` and
    // `SharedDevice`, and `[w404]` recorded that production passes `SharedObjectModel` — the
    // hook went into the wrong one of those twice before. An edit to the unreached copy would
    // satisfy the assertion above and change nothing at runtime, which is this session's
    // single most-repeated failure.
    assert!(
        shim.contains("fn promote_ctx(")
            && shim.contains("self.0.promote_ctx(p)"),
        "the production `SharedObjectModel` must delegate promote_ctx to the `SharedDevice` \
         whose handler does the backing; without that delegation the refresh above is dead code"
    );

    // (1) the invalidate register — its lane refreshes on the worker and only then completes.
    assert!(
        shim.contains("refresh_page_tables(off_vcpu)"),
        "entry point (1): the invalidate lane must refresh off the vCPU before completing"
    );

    // ⚠ And the seam they all share must be installed, or all three are inert at once.
    assert!(
        shim.contains("set_invalidate_refresh("),
        "one uninstalled seam disables ALL THREE entry points together"
    );
}


// =====================================================================================
// THE OTHER HALF — what the SHIPPING arm does
// =====================================================================================

/// ★★★★★ **The shipping default must NOT route on the caller, and this is the only test
/// that says so.**
///
/// Every other test in this file pins `KAYFABE_DOORBELL_ASYNC=off` so it can observe the
/// routing chain's answer synchronously. That pin is safe only while something still checks
/// the arm the bench and the product actually run — otherwise the file would assert a
/// configuration nobody uses and read as full coverage.
///
/// ⊘ The owner's standing invariant is the reason the default moved:
/// *"a doorbell is only a token + channel ... the vcpu thread doesn't know and shouldn't
/// care what's inside that channel"*. `[measured w510]` routing on the caller holds the
/// plane's rank-0 mutex for **8.2 ms** inside `ce_session_with_root`, and a vCPU's
/// `RegPlane::write` blocks for exactly that long.
#[test]
fn the_shipping_arm_enqueues_instead_of_routing_on_the_caller() {
    // SAFETY: single-threaded, and this test builds its own `Regs` immediately below.
    unsafe {
        std::env::set_var(kayfabe_qemu_raw::shim::DOORBELL_ASYNC_ENV, "on");
    }
    let r = Regs::create(0).expect("the shipped chip row realizes");
    let out = r.write(BAR_REGS, DOORBELL, 4, GOOD_TOKEN);
    let report = out
        .doorbell
        .as_ref()
        .expect("★ the write must still be CLASSIFIED as a doorbell on either arm");
    assert!(
        report.refusal().is_none(),
        "the shipping arm must ENQUEUE and return, not route on the caller and refuse:          {report:?}"
    );
    // Put it back, so a later test in this binary is not silently run on the other arm.
    // SAFETY: as above.
    unsafe {
        std::env::set_var(kayfabe_qemu_raw::shim::DOORBELL_ASYNC_ENV, "off");
    }
}
