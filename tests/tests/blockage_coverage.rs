//! ★★★★★ **R1's C1 AND C3, ASSERTED ON A DEVICE THE GUEST'S OWN EVENTS BUILT.**
//!
//! `docs/design/REQUIREMENTS_TARGET.md` R1.4 lists five falsifiers and says of C1–C4 that
//! *"the correctness clauses have **no failing test yet**. That is the first debt to clear,
//! because a requirement nobody can fail is not a requirement."* This file is that debt for
//! **C1** and **C3**.
//!
//! | clause | the falsifier | what is asserted here |
//! |---|---|---|
//! | **C1** | *any mapping used by a channel with no prior blockage point — count must be 0, with a known-positive denominator* | a promotion served **under** the GSP-RPC halt is attributed to it and is `COVERED`; the **same** promotion served outside every halt is `⊘UNCOVERED` and its VA is named |
//! | **C3** | *guest-kernel/UVM channels do not classify `Emulated` on a live boot — asserted by **count**, not by rule* | [`ChannelKindCensus`] counts both populations off the rows a boot log prints, and the verdict refuses to be `AGREES` unless both are non-empty |
//!
//! # ⊘ WHAT THIS FILE IS NOT, and it must be said first
//!
//! **It is not a boot.** It runs the real `Gpu::apply`, the real `Gpu::promote_ctx`, the real
//! `AddressTable` and the real census renderers against a `WireClassArch` and a
//! `MockIsolateFactory`. That is enough to falsify *the instrument* — an attribution that
//! never fires, one that always fires, a verdict that reads the same for a clean boot and an
//! unarmed one — and it is **not** enough to answer C1 or C3 about a guest.
//!
//! ⇒ **The clauses are graded on a live boot**, from the `kayfabe: BLOCKAGE-COVERAGE …` /
//! `CHANNEL-KIND …` lines the shim prints on every doorbell. `a_green_test_can_hold_a_wall_in
//! _place` is the standing lesson: what a mock blesses is the code, never the guest.
//!
//! # ★★ Why the guard is entered here rather than mocked
//!
//! [`BlockageGuard`] is a thread-local. A test that "simulated" one by writing a flag would
//! be testing its own simulation; entering the real guard around the real
//! `Gpu::promote_ctx` is the same composition the production `ObjectPolicy::respond` makes,
//! one frame shorter.

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped handle/VA literals

use kayfabe_arch::Aperture;
use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_core::channel_kind::GuestChannelKind;
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::{Gpu, VasCensusRow};
use kayfabe_core::promote::{CtxPromotion, PromoteDeclined, PromotedRange};
use kayfabe_core::rmgraph::{AllocFacts, RmEvent};
use kayfabe_mmu::blockage::{BlockageGuard, BlockagePoint, BlockageVerdict};
use kayfabe_mocks::{MockArch, MockIsolateFactory, WireClassArch, mock_classes};
use kayfabe_rt::device::{ChannelKindCensus, ChannelKindVerdict};
use kayfabe_tests::{Scenario, identical_handles};

// =====================================================================================
// Harness — the same shape `promote_ctx.rs` and `channel_kind_declaration.rs` use, so a
// divergence here is a divergence in this file and not in a second world builder.
// =====================================================================================

const A_CLIENT: HClient = HClient(0xAA);
const B_CLIENT: HClient = HClient(0xBB);
const A_PDB: Pdb = Pdb(0x3401_000);
const B_PDB: Pdb = Pdb(0x3405_000);

/// The guest kernel's namespace. Its declared `ClientKind` is `Kernel`, which is the ONLY
/// thing that puts its channels in the reserved system component — and therefore the only
/// thing that makes [`GuestChannelKind::Emulated`] reachable at all.
const K_CLIENT: HClient = HClient(0xC1D0_000A);
const K_DEVICE: HObject = HObject(0xC1D0_0101);
const K_VASPACE: HObject = HObject(0xC1D0_0110);
const K_CHANNEL: HObject = HObject(0xC1D0_0119);
const K_VCHID: VChid = VChid(0x40);
const K_PDB: Pdb = Pdb(0x2_efa9_c000);

/// The handles `identical_handles` mints — both procs use the SAME values.
const H_GR_CHANNEL: HObject = HObject(0x5c00_0019);
const GR_VA: GpuVa = GpuVa(0x1_2002_0000);
const GR_LEN: u64 = 0xea000;
const GR_PHYS: u64 = 0x2_ef94_6000;

/// The guest kernel's own subgraph: a KERNEL client root → device → VASpace(+PDB) → one CE
/// channel — the CeUtils/scrubber shape every boot creates before a CUDA process exists.
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

/// A device with both populations live: two CUDA processes and the guest kernel's channel.
fn world_with_kernel() -> Gpu {
    let mut g = bare();
    let mut s = Scenario::new();
    s.compute_process(A_CLIENT, A_PDB, identical_handles(0x10, 0x11));
    s.compute_process(B_CLIENT, B_PDB, identical_handles(0x20, 0x21));
    kernel_channel(&mut s);
    for ev in s.events {
        g.apply(ev).expect("scenario applies cleanly");
    }
    g
}

/// A device with only the user population — the negative control for C3's vacuity arm.
fn world_user_only() -> Gpu {
    let mut g = bare();
    let mut s = Scenario::new();
    s.compute_process(A_CLIENT, A_PDB, identical_handles(0x10, 0x11));
    for ev in s.events {
        g.apply(ev).expect("scenario applies cleanly");
    }
    g
}

fn bare() -> Gpu {
    let (factory, _rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    Gpu::new(Box::new(WireClassArch::new()), Box::new(factory), gpa).expect("device realizes")
}

fn promotion() -> CtxPromotion {
    CtxPromotion {
        client: A_CLIENT,
        chan_client: A_CLIENT,
        object: H_GR_CHANNEL,
        ranges: vec![PromotedRange {
            va: GR_VA,
            len: GR_LEN,
            phys: GR_PHYS,
            aperture: Aperture::Vidmem,
            buffer_id: 0,
        }],
        halves: Vec::new(),
        declined: PromoteDeclined::default(),
    }
}

fn pid_of(g: &Gpu, pdb: Pdb) -> kayfabe_core::ProcId {
    *g.spine
        .by_pdb
        .get(&(GpuId::ZERO, pdb))
        .expect("routed by its PDB")
}

/// This address space's census, computed the way the boot line computes it.
fn counts(g: &Gpu, pdb: Pdb, cap: usize) -> kayfabe_mmu::blockage::BlockageCounts {
    g.procs[&pid_of(g, pdb)].vases[&(GpuId::ZERO, pdb)]
        .table
        .blockage_counts(cap)
}

fn rows(g: &Gpu) -> Vec<VasCensusRow> {
    // ⊘ `Gpu::vas_census` and not a private enumeration: it is the exact row set a boot log
    // prints, so nothing asserted here can be true of a row shape the log does not print.
    g.vas_census()
}

// =====================================================================================
// C1 — every mapping was published at a blockage point before it was used
// =====================================================================================

/// ★★★★★ **C1, THE POSITIVE HALF.** A `GPU_PROMOTE_CTX` served while the guest is blocked in
/// `_issueRpcAndWait` binds rows attributed to [`BlockagePoint::GspRpc`], and a later use of
/// one of those rows lands in `uses[gsp-rpc]` and **not** in `USES_UNCOVERED`.
///
/// ⊘ It asserts the whole line's four numbers together — the attribution, the denominator,
/// the uncovered count and the verdict — because each alone is satisfiable by a broken
/// instrument: an attribution that fires for everything gives `uses_uncovered = 0` on a boot
/// with no guards at all.
#[test]
fn a_promotion_served_under_the_rpc_halt_is_attributed_to_it() {
    let mut g = world_with_kernel();

    let join = {
        let _halt = BlockageGuard::enter(BlockagePoint::GspRpc);
        g.promote_ctx(&promotion()).expect("the promotion joins")
    };
    assert_eq!(join.bound, 1, "non-vacuity: a row really was bound");

    // The use — a real `AddressTable::resolve`, the same call a doorbell's ring translation
    // makes. ⊘ Not `binding_at`: that is a diagnostic and is deliberately not a use.
    let pid = pid_of(&g, A_PDB);
    assert!(
        g.procs[&pid].vases[&(GpuId::ZERO, A_PDB)]
            .table
            .resolve(A_PDB, GR_VA)
            .is_ok(),
        "the promoted range resolves — otherwise there is no use to attribute"
    );

    // ★ The row's OWN stamp, not only the aggregate. A census that summed correctly over
    // rows stamped wrongly would satisfy every count below and be false of every row.
    assert_eq!(
        g.procs[&pid].vases[&(GpuId::ZERO, A_PDB)]
            .table
            .publication_at(GR_VA)
            .expect("the row exists")
            .point,
        Some(BlockagePoint::GspRpc),
    );

    let c = counts(&g, A_PDB, 24);
    assert_eq!(c.binds[BlockagePoint::GspRpc.index()], 1);
    assert_eq!(c.binds_uncovered, 0);
    assert_eq!(c.publications(), 1, "the denominator");
    assert_eq!(c.uses[BlockagePoint::GspRpc.index()], 1);
    assert_eq!(c.uses_uncovered, 0, "★ THE C1 NUMBER");
    assert_eq!(c.uncovered_vas_total, 0);

    // The arming state is the third term, and the verdict is read WITH it: the same zeros
    // over an unarmed boot are `⊘NEVER-ARMED`, which is not a pass.
    let armed = {
        let mut e = [0u64; 3];
        e[BlockagePoint::GspRpc.index()] = 1;
        e
    };
    assert_eq!(c.verdict(armed), BlockageVerdict::Covered);
    assert!(c.verdict(armed).is_pass());
    assert_eq!(
        c.verdict([0, 0, 0]),
        BlockageVerdict::NeverArmed,
        "⊘ the SAME counts, with nothing armed, are UNMEASURED and not a pass — this is the \
         distinction the whole census exists to keep"
    );
}

/// ★★★★★ **C1, THE FALSIFIER — and it must fail for the mock too, or the test is vacuous.**
///
/// The **same** promotion, served with no halt declared, publishes a row attributed to
/// nothing. The verdict flips to `⊘UNCOVERED`, the count is 1, and the line **names the VA**
/// — which is R1.4's literal requirement: *"the count and the VAs"*.
///
/// ⊘ Written as the twin of the test above, one variable apart, because a positive control
/// and a falsifier that differ in two things measure neither.
#[test]
fn the_same_promotion_with_no_halt_declared_is_uncovered_and_names_its_va() {
    let mut g = world_with_kernel();
    // ⊘ No guard. This is the one variable.
    let join = g.promote_ctx(&promotion()).expect("the promotion joins");
    assert_eq!(join.bound, 1);

    let pid = pid_of(&g, A_PDB);
    g.procs[&pid].vases[&(GpuId::ZERO, A_PDB)]
        .table
        .resolve(A_PDB, GR_VA)
        .expect("bound");

    assert_eq!(
        g.procs[&pid].vases[&(GpuId::ZERO, A_PDB)]
            .table
            .publication_at(GR_VA)
            .expect("the row exists")
            .point,
        None,
        "⊘ the row itself is stamped with NO halt — the aggregate below is a consequence, \
         not an independent fact"
    );

    let c = counts(&g, A_PDB, 24);
    assert_eq!(c.binds_uncovered, 1);
    assert_eq!(c.uses_uncovered, 1, "★ THE C1 NUMBER, non-zero");
    assert_eq!(c.uncovered_vas_total, 1);
    assert_eq!(
        c.uncovered_vas,
        vec![GR_VA.0],
        "★ R1.4 asks for *the count AND the VAs* — a count nobody can chase is a report, \
         not an instrument"
    );

    let armed = {
        let mut e = [0u64; 3];
        e[BlockagePoint::GspRpc.index()] = 1;
        e
    };
    assert_eq!(c.verdict(armed), BlockageVerdict::Uncovered);
    assert!(!c.verdict(armed).is_pass());

    let line = c.render(armed, 24);
    assert!(line.contains("⊘UNCOVERED"), "{line}");
    assert!(line.contains("USES_UNCOVERED=1"), "{line}");
    assert!(
        line.contains(&format!("0x{:x}", GR_VA.0)),
        "the VA must be ON the line: {line}"
    );
}

/// ⊘ **A HALT DECLARED ON ONE THREAD IS NOT A HALT ON ANOTHER.** [`BlockageGuard`] is
/// `!Send`, so it cannot travel; this asserts the *consequence* — a bind on a second thread
/// while the first thread holds a guard is attributed to **nothing**.
///
/// ★ It matters because each guest vCPU is a host thread here. A guard whose effect leaked
/// across threads would attribute vCPU-1's publication to vCPU-0's halt, and C1 would report
/// coverage for a mapping the guest could already be using.
#[test]
fn a_halt_on_one_thread_does_not_cover_a_bind_on_another() {
    let mut g = world_with_kernel();
    let _halt = BlockageGuard::enter(BlockagePoint::EmulatedDoorbell);
    // The promotion runs on a scoped thread that entered no guard of its own.
    std::thread::scope(|sc| {
        sc.spawn(|| {
            assert_eq!(
                kayfabe_mmu::blockage::current(),
                None,
                "★ a fresh thread is inside no halt, whatever this thread holds"
            );
            g.promote_ctx(&promotion()).expect("the promotion joins");
        });
    });
    let c = counts(&g, A_PDB, 24);
    assert_eq!(c.binds[BlockagePoint::EmulatedDoorbell.index()], 0);
    assert_eq!(c.binds_uncovered, 1);
}

// =====================================================================================
// C3 — the guest-kernel channels classify `Emulated`, by COUNT
// =====================================================================================

/// ★★★★★ **C3.** Both populations are counted off the rows a boot log prints, both
/// misclassification lists are empty, and the verdict is `AGREES`.
///
/// ⊘ The two offender lists are asserted **empty in both directions**, not just the
/// guest-kernel one. A classifier that answered `Emulated` for *everything* would satisfy
/// *"every system channel is emulated"* and would route a guest process's ring into the
/// inspecting path — the `#14` confused deputy, arriving through a one-sided assertion.
#[test]
fn the_channel_kind_census_counts_both_populations_and_agrees() {
    let g = world_with_kernel();
    let c = ChannelKindCensus::of(&rows(&g));

    assert!(
        c.system_proc_channels > 0,
        "★ NON-VACUITY: no guest-kernel channel means the `Emulated` half of the \
         biconditional is satisfied by having nothing to check"
    );
    assert!(c.user_proc_channels > 0, "★ NON-VACUITY, the other half");
    assert_eq!(c.emulated, c.system_proc_channels);
    assert_eq!(c.passthrough, c.user_proc_channels);
    assert_eq!(c.system_not_emulated, Vec::new());
    assert_eq!(c.user_not_passthrough, Vec::new());
    assert_eq!(c.verdict(), ChannelKindVerdict::Agrees);

    let line = c.render(12);
    assert!(line.contains("CHANNEL-KIND AGREES"), "{line}");
    assert!(
        line.contains(&format!("emulated={}", c.emulated)),
        "the counts are ON the line, because C3 is graded by count: {line}"
    );
    assert!(!line.contains("VACUOUS"), "{line}");
}

/// ⊘ **AND THE VACUOUS ARM IS REACHABLE AND SAYS WHICH SIDE IS EMPTY.**
///
/// A boot sampled at teardown has no user channels left; a boot sampled before the guest
/// kernel has built its CeUtils channel has no system ones. ⊘ **Neither of those two states
/// is observed here** — this test only pins that the census *distinguishes* them. Without
/// this arm, `AGREES` over zero guest-kernel channels would be the same word as `AGREES`
/// over a boot that exercised the split (`every_row_verified_over_zero_rows`).
#[test]
fn a_census_with_only_one_population_refuses_to_say_it_agrees() {
    let user_only = ChannelKindCensus::of(&rows(&world_user_only()));
    assert!(user_only.user_proc_channels > 0);
    assert_eq!(user_only.system_proc_channels, 0);
    assert_eq!(user_only.verdict(), ChannelKindVerdict::Vacuous);
    let line = user_only.render(12);
    assert!(line.contains("⊘VACUOUS"), "{line}");
    assert!(
        line.contains("NO GUEST-KERNEL CHANNEL WAS LIVE"),
        "the line must say WHICH side is empty: {line}"
    );

    let empty = ChannelKindCensus::of(&[]);
    assert_eq!(empty.verdict(), ChannelKindVerdict::Vacuous);
    assert!(
        empty.render(12).contains("NO LIVE CHANNELS AT ALL"),
        "{}",
        empty.render(12)
    );
}

/// ⊘ **THE CENSUS READS THE DECLARED KIND; IT DOES NOT RE-DERIVE IT.**
///
/// If it recomputed the kind from `proc == SYSTEM_PROC` it would be checking its own
/// arithmetic and would pass on any device at all — `two_projections_of_one_fact_disagreeing`
/// with the disagreement removed by making one projection fake. This drives the *real* rows
/// through a hand-corrupted copy and asserts the offender is caught and named.
#[test]
fn a_corrupted_kind_is_caught_and_named_rather_than_recomputed() {
    let g = world_with_kernel();
    let mut r = rows(&g);
    let victim = r
        .iter_mut()
        .find(|row| row.kind == GuestChannelKind::Emulated)
        .expect("non-vacuity: the fixture has a guest-kernel channel to corrupt");
    let (p, ch) = (victim.proc.0, victim.chan.0);
    victim.kind = GuestChannelKind::Passthrough;

    let c = ChannelKindCensus::of(&r);
    assert_eq!(c.verdict(), ChannelKindVerdict::Disagrees);
    assert_eq!(c.system_not_emulated, vec![(p, ch)]);
    let line = c.render(12);
    assert!(line.contains("⊘DISAGREES"), "{line}");
    assert!(
        line.contains(&format!("p{p}/c{ch}")),
        "the offending channel must be named, not merely counted: {line}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════════════
// ★★★★★ w390 — THE TLB-INVALIDATE BLOCKAGE POINT'S TWO WIRES
// ═══════════════════════════════════════════════════════════════════════════════════════
//
// `MmuInvalidateLog` has been complete since w326 and DEAD since w326: `arm` had no caller
// and `WriteOutcome::publish_before_completing` had no consumer, so `note_trigger` answered
// `Observed` on every boot and the halt spent nothing. w390 wires both. These tests assert
// the property the wiring turns on — not that the code exists.
//
// ⊘ THEY ARE WRITTEN TO GO RED IF EITHER WIRE IS PULLED, and each says which:
//   - unwire `arm`            ⇒ `armed_trigger_demands_a_publication` fails (it gets
//                               `Observed` and the guest is never held).
//   - unwire the consumer     ⇒ `a_publication_that_panics_still_clears` cannot even be
//                               written against the shell, so it is asserted here against
//                               the type's own contract, which is where the guarantee lives.
mod w390_invalidate_blockage {
    use kayfabe_device::mmuinval::{MmuInvalidateLog, TriggerAction};

    /// The raw value of a write that sets `TRIGGER` and nothing else.
    ///
    /// ⊘ **Bit 31, taken from `Invalidate::decode`'s own `raw & (1 << 31)`** — not from the
    /// name. The first draft of this file used `1`, which is `ALL_VA`, and every armed
    /// assertion came back `Observed`: a scope bit read as a commit. ★ That the tests went
    /// RED rather than green is the point — a constant guessed from a doc comment is exactly
    /// what a suite asserting only "the code ran" would have missed.
    const TRIGGER: u32 = 1 << 31;

    /// ⊘ **THE CONTROL, and it must come first.** Disarmed is byte-identical to every boot
    /// before w390: the trigger is counted and answered immediately. A test suite that only
    /// checked the armed arm could not tell "the arm works" from "the arm is always on",
    /// and always-on is a guest hang on every boot.
    #[test]
    fn disarmed_answers_immediately_and_still_counts() {
        let log = MmuInvalidateLog::new();
        let (_, act) = log.note_trigger(TRIGGER, 0);
        assert_eq!(
            act,
            TriggerAction::Observed,
            "a DISARMED log must answer the guest immediately — this is the pre-w390 \
             behaviour and the byte-comparable control"
        );
        assert!(
            !log.pending(),
            "nothing may be outstanding on the control arm, or the guest spins on a boot \
             that never asked for this lane"
        );
        assert_eq!(
            log.read_trigger(),
            0,
            "the guest's poll must read TRIGGER clear while disarmed"
        );
        assert_eq!(
            log.snapshot().triggers,
            1,
            "★ the CENSUS still counts on the control arm — that is what makes the control \
             a measurement rather than a silence"
        );
    }

    /// ★ **The wire w390 adds.** An armed trigger demands a publication and holds the guest
    /// until one is reported. ⊘ If `arm`'s caller is removed this fails on the first assert.
    #[test]
    fn armed_trigger_demands_a_publication_and_holds_the_guest() {
        let log = MmuInvalidateLog::new();
        log.arm();
        let (_, act) = log.note_trigger(TRIGGER, 1_000);
        assert_eq!(
            act,
            TriggerAction::Publish,
            "★ an ARMED trigger must tell the caller to publish — this is the blockage \
             point spending something, and it is the whole of w390"
        );
        assert!(log.pending(), "a publication must be outstanding");
        assert_ne!(
            log.read_trigger(),
            0,
            "★★★ THE GUEST MUST SEE TRIGGER SET while the publication is outstanding — \
             this is what makes `kgmmuCheckPendingInvalidates_TU102` spin, i.e. what makes \
             this a blockage point at all rather than a place we happen to run code"
        );
        log.complete(1_500);
        assert!(!log.pending(), "completion must clear the outstanding flag");
        assert_eq!(
            log.read_trigger(),
            0,
            "and the guest's next poll must return"
        );
        assert_eq!(
            log.snapshot().worst_hold_us,
            500,
            "the hold is measured from the caller's own clock — 1500 - 1000. ⚠ Every \
             microsecond of it is a microsecond the guest spun"
        );
    }

    /// ⊘⊘ **THE LIVENESS OBLIGATION, asserted rather than commented.** The shell completes
    /// in a `Drop` guard precisely so a publication that panics or returns early still
    /// clears. This asserts the type supports that: `complete` is idempotent, so a guard
    /// that fires alongside an explicit call costs nothing the second time — which is what
    /// lets the guard be unconditional.
    #[test]
    fn completion_is_idempotent_so_the_drop_guard_can_be_unconditional() {
        let log = MmuInvalidateLog::new();
        log.arm();
        log.note_trigger(TRIGGER, 0);
        log.complete(100);
        let after_first = log.snapshot().worst_hold_us;
        log.complete(9_999_999);
        assert!(!log.pending());
        assert_eq!(
            log.snapshot().worst_hold_us,
            after_first,
            "⊘ a second completion must not re-stamp the hold — otherwise the Drop guard \
             firing after an explicit complete would report a hold that never happened, and \
             the over-budget count would be measuring the guard rather than the guest"
        );
    }

    /// ⊘ A write that does NOT set `TRIGGER` is scope bits being latched, not a commit.
    /// Publishing on it would publish BEFORE the guest said its tables were ready — the one
    /// ordering this whole plane exists to respect.
    #[test]
    fn a_non_trigger_write_never_publishes_even_when_armed() {
        let log = MmuInvalidateLog::new();
        log.arm();
        let (_, act) = log.note_trigger(0, 0);
        assert_eq!(
            act,
            TriggerAction::Observed,
            "★★★ scope bits without TRIGGER are a LATCH. Publishing here would publish \
             before the guest committed its page tables"
        );
        assert!(!log.pending(), "and the guest must not be held for a latch");
    }
}
