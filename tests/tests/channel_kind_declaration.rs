//! ★★★★★ **A CHANNEL'S GUEST-FACING KIND IS DECLARED, AND IT AGREES WITH THE ROUTING** —
//! the owner's 2026-08-11 split, asserted on a live device over real `RmEvent`s.
//!
//! # ⊘ Why this file exists SEPARATELY from the gate's own unit tests
//!
//! `crates/kayfabe-qemu-raw/tests/shim_logic.rs` proves the *predicate*:
//! `forwarding_plane_owns_ce`'s truth table did not move when its third term stopped being
//! `proc != Gpu::SYSTEM_PROC` and became a read of
//! [`kayfabe_core::channel_kind::GuestChannelKind`]. It cannot prove the *join* — that the
//! kind a real channel carries is the one its proc implies — because it has no device, no
//! projection and no guest. A green there with a broken join is exactly the shape this
//! campaign keeps paying for: `a_green_test_can_hold_a_wall_in_place`.
//!
//! ⇒ Here the kind is not constructed. It is **read off channels the guest's own RM events
//! created**, and checked against the proc the projection routed each of them to.
//!
//! # What is asserted, and in both directions
//!
//! 1. **Every** channel of the system proc is [`GuestChannelKind::Emulated`], and **every**
//!    channel of a user proc is [`GuestChannelKind::Passthrough`] — the biconditional, over
//!    a device that has both populations live at once.
//! 2. **Non-vacuity**: both populations are non-empty in the fixture, so a projection that
//!    materialized no kernel channel at all could not pass by having nothing to check
//!    (`a_census_zero_needs_a_known_positive`).
//! 3. The kind survives a **refresh**: it is re-derived on every `Gpu::apply`, and a later,
//!    unrelated event must not flip it.

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped handle/VA literals

use kayfabe_arch::ids::{HClient, HObject, Pdb, VChid};
use kayfabe_core::ProcId;
use kayfabe_core::channel_kind::{GuestChannelKind, HostChannelKind};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::{Gpu, VasCensusRow};
use kayfabe_core::rmgraph::{AllocFacts, RmEvent};
use kayfabe_mocks::{MockArch, MockIsolateFactory, WireClassArch, mock_classes};
use kayfabe_tests::{Scenario, identical_handles};

/// The guest kernel's namespace — UVM's session client / RM's internal clients. Its
/// declared [`kayfabe_arch::ClientKind`] is `Kernel`, which is the ONLY thing that puts
/// its channels in the reserved system component (§12.27).
const K_CLIENT: HClient = HClient(0xC1D0_000A);
const K_DEVICE: HObject = HObject(0xC1D0_0101);
const K_VASPACE: HObject = HObject(0xC1D0_0110);
const K_CHANNEL: HObject = HObject(0xC1D0_0119);
const K_VCHID: VChid = VChid(0x40);
const K_PDB: Pdb = Pdb(0x2_efa9_c000);

const A_CLIENT: HClient = HClient(0xC1D0_0001);
const A_PDB: Pdb = Pdb(0x1_0000_0000);
const B_CLIENT: HClient = HClient(0xC1D0_0002);
const B_PDB: Pdb = Pdb(0x1_1000_0000);

/// The guest kernel's own subgraph: a KERNEL client root → device → VASpace(+PDB) →
/// **one CE channel**, which is the shape of the CeUtils/scrubber channel every boot
/// creates before a CUDA process exists.
///
/// ⊘ Scripted by hand rather than through [`Scenario::compute_process`], because that
/// helper emits a `user_client` root by construction — the guest-kernel population has no
/// helper, which is itself part of why this axis was only ever a routing fact.
fn kernel_channel(s: &mut Scenario) {
    s.push(RmEvent::Alloc {
        client: K_CLIENT,
        parent: HObject(K_CLIENT.0),
        handle: HObject(K_CLIENT.0),
        class: mock_classes::CLIENT,
        facts: kayfabe_tests::kernel_client(),
    });
    s.push(RmEvent::Alloc {
        client: K_CLIENT,
        parent: HObject(K_CLIENT.0),
        handle: K_DEVICE,
        class: mock_classes::DEVICE,
        facts: AllocFacts {
            device_instance: Some(0),
            ..Default::default()
        },
    });
    s.push(RmEvent::Alloc {
        client: K_CLIENT,
        parent: K_DEVICE,
        handle: K_VASPACE,
        class: mock_classes::VASPACE,
        facts: AllocFacts::default(),
    });
    s.push(RmEvent::SetPageDir {
        client: K_CLIENT,
        vaspace: K_VASPACE,
        pdb: K_PDB,
    });
    s.push(RmEvent::Alloc {
        client: K_CLIENT,
        parent: K_DEVICE,
        handle: K_CHANNEL,
        class: mock_classes::CHANNEL_CE,
        facts: AllocFacts {
            h_vaspace: Some(K_VASPACE),
            userd_flags: MockArch::userd_flags_for(K_VCHID),
            ..Default::default()
        },
    });
}

/// A device with BOTH populations live: two CUDA processes and the guest kernel's own
/// channel.
fn world() -> Gpu {
    let (factory, _rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut g =
        Gpu::new(Box::new(WireClassArch::new()), Box::new(factory), gpa).expect("device realizes");
    let mut s = Scenario::new();
    s.compute_process(A_CLIENT, A_PDB, identical_handles(0x10, 0x11));
    s.compute_process(B_CLIENT, B_PDB, identical_handles(0x20, 0x21));
    kernel_channel(&mut s);
    for ev in s.events {
        g.apply(ev).expect("scenario applies cleanly");
    }
    g
}

/// Every live channel as a census row — the public read path, which is also the one a boot
/// log prints.
fn rows(g: &Gpu) -> Vec<VasCensusRow> {
    // ⊘ `Gpu::vas_census` and NOT a private enumeration: it is the exact row set
    // `vas_census_string` formats into a boot log, so nothing asserted here can be true of
    // a row shape the log does not print.
    g.vas_census()
}

/// ★★★★★ **THE JOIN.** A channel's declared kind is `Emulated` **if and only if** the
/// projection routed it to the reserved system proc.
///
/// ⊘ This is the proposition `forwarding_plane_owns_ce`'s unit tests structurally cannot
/// state. They quantify over a `GuestChannelKind` they construct; this quantifies over the
/// kinds a **guest** produced, and checks them against the routing decision that used to be
/// the gate's only input.
#[test]
fn a_channels_declared_kind_is_emulated_exactly_when_it_routes_to_the_system_proc() {
    let g = world();
    let all = rows(&g);
    let mut emulated = 0usize;
    let mut passthrough = 0usize;
    for r in &all {
        let routed_to_system = r.proc == Gpu::SYSTEM_PROC;
        let declared_emulated = r.kind == GuestChannelKind::Emulated;
        assert_eq!(
            declared_emulated,
            routed_to_system,
            "★ THE JOIN BROKE on p{}/c{} (vc{:#x}, c0x{:x}/0x{:x}): it routes to {} and \
             declares itself {}. The kind and the ProcId are two projections of ONE pass \
             over ONE `ProcBoundary` (`Gpu::sync_proc_to_boundary`); if they can disagree, \
             then `forwarding_plane_owns_ce` reading the kind is not the same gate that \
             12 boots paid for, and its unit tests are green over a fiction.",
            r.proc.0,
            r.chan.0,
            r.vchid.0,
            r.client,
            r.handle,
            if routed_to_system {
                "the SYSTEM proc"
            } else {
                "a USER proc"
            },
            r.kind,
        );
        if declared_emulated {
            emulated += 1;
        } else {
            passthrough += 1;
        }
    }
    // ⊘ NON-VACUITY, both ways. `a_census_zero_needs_a_known_positive`: a projection that
    // materialized no guest-kernel channel would satisfy every assertion above by having
    // nothing to check, and so would one that put everything in the system proc.
    assert!(
        emulated > 0,
        "★ the fixture produced NO emulated channel ({} rows total). The guest-kernel \
         population is the one the whole rule is about; a run without it cannot fail the \
         assertion above and must not be read as passing it.",
        all.len()
    );
    assert!(
        passthrough > 0,
        "★ the fixture produced NO passthrough channel ({} rows total) — the negative \
         control is missing, so *everything is emulated* would also pass.",
        all.len()
    );
}

/// ★★★ **The kind is what the forwarding-plane gate would read, and the two populations
/// get OPPOSITE answers from it** — the live-device half of
/// `the_gate_hands_over_exactly_the_kinds_a_shadow_host_channel_may_back`.
///
/// ⊘ It does not call the gate (that lives in `kayfabe-qemu-raw`, which this suite does
/// not depend on); it asserts the term the gate branches on, over channels a guest made.
#[test]
fn the_guest_kernels_channel_may_only_be_hosted_by_a_scratchpad_and_a_cuda_processs_by_a_shadow() {
    let g = world();
    for r in rows(&g) {
        let want = if r.proc == Gpu::SYSTEM_PROC {
            HostChannelKind::Scratchpad
        } else {
            HostChannelKind::Shadow
        };
        assert_eq!(
            r.kind.hosted_by(),
            want,
            "★ p{}/c{} ({}) may be hosted by a {} — a guest-KERNEL channel hosted by a \
             guest PROCESS's isolated channel is the confused deputy #14 designed out, and \
             a CUDA process's channel hosted by our own scratchpad is work we would have to \
             forge.",
            r.proc.0,
            r.chan.0,
            r.kind,
            r.kind.hosted_by(),
        );
    }
}

/// ★★★ **A later, unrelated event must not flip a kind.**
///
/// `Channel::kind` is re-assigned on every `Gpu::apply`'s refresh, with the other declared
/// facts. ⚠ That is deliberate — a field refreshed on a different pass from the resolution
/// that produced it is how two projections come to disagree — but it means a refresh is a
/// place a kind could *move*. `[measured]` it cannot: a `Proc` is one `ProcBoundary`, and
/// `RmGraph::apply` refuses `RESERVED_CLIENT` as guest input, so no component can migrate
/// across the system anchor. This asserts the consequence rather than the argument.
#[test]
fn a_third_processs_arrival_does_not_move_any_existing_channels_kind() {
    let mut g = world();
    let before: Vec<(ProcId, u32, GuestChannelKind)> = rows(&g)
        .iter()
        .map(|r| (r.proc, r.handle, r.kind))
        .collect();
    assert!(!before.is_empty(), "the fixture has channels to preserve");

    let mut s = Scenario::new();
    s.compute_process(
        HClient(0xC1D0_0003),
        Pdb(0x1_2000_0000),
        identical_handles(0x30, 0x31),
    );
    for ev in s.events {
        g.apply(ev).expect("a third process applies cleanly");
    }

    let after: Vec<(ProcId, u32, GuestChannelKind)> = rows(&g)
        .iter()
        .map(|r| (r.proc, r.handle, r.kind))
        .collect();
    for (proc, handle, kind) in &before {
        let found = after
            .iter()
            .find(|(p, h, _)| p == proc && h == handle)
            .unwrap_or_else(|| {
                panic!(
                    "★ p{}/0x{handle:x} vanished when a third process arrived",
                    proc.0
                )
            });
        assert_eq!(
            found.2, *kind,
            "★ p{}/0x{handle:x} was declared {kind} and is now {} after an unrelated \
             process's events. A kind that moves under refresh is a kind a consumer cannot \
             carry.",
            proc.0, found.2,
        );
    }
    assert!(
        after.len() > before.len(),
        "★ the third process added no channels, so nothing about the refresh was exercised \
         ({} before, {} after)",
        before.len(),
        after.len()
    );
}

/// ⊘ The census a boot log prints now NAMES the kind, instead of leaving a reader to
/// re-derive it from `p0` meaning *"the reserved proc"*.
///
/// ★ Asserted on the string because that is the artefact a human reads out of a boot: a
/// field on a struct nobody prints would not have closed the gap this rung is about.
#[test]
fn the_census_string_names_both_kinds() {
    let g = world();
    let census = g.vas_census_string(None);
    assert!(
        census.contains(GuestChannelKind::Emulated.name()),
        "★ the census does not name the emulated population: {census}"
    );
    assert!(
        census.contains(GuestChannelKind::Passthrough.name()),
        "★ the census does not name the passthrough population: {census}"
    );
}
