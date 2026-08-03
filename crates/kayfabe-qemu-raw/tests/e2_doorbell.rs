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
    ] {
        dev.apply(ev).expect("the bridge's object model accepts it");
    }

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
    // …and the refusal it DOES give is the one the shipped archive's plane owes: there is
    // no forwarding isolate, and it says so rather than parking on a pool gate that can
    // never signal (`kayfabe_fwd::FwdFault::IsolateRetired`).
    assert_eq!(
        kind, "FwdFault::IsolateRetired",
        "★ the route resolved and the refusal came from the ISOLATE plane — the first \
         refusal in this port's life that is downstream of routing"
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
    assert!(
        code.contains("SharedDoorbell(Arc::clone(&device))"),
        "…and the doorbell port must ring the same one"
    );
}
