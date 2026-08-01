//! ★★★ **Task #111: a bad application pointer surfaces as a simulated GPU fault.**
//!
//! `docs/design/simulated_gpu_fault.md`. The property under test, in one sentence: *an
//! address a guest application's submission names but never mapped must produce a
//! message the guest's own driver acts on — never a hang, and never a silent success.*
//!
//! ## ★★ What "mean, not happy path" means here, concretely
//!
//! The happy-path version of this file would build one report by hand, encode it, and
//! check nine integers. Every test below instead starts from **a real refusal**: a real
//! `Gpu`, real channels derived from a real RM event stream, a doorbell that is actually
//! rung and actually refused by the #14 ring-gate. The fault is derived from the refusal
//! the way production would derive it, because the failure this task exists to remove is
//! *the refusal going nowhere* — and a test that starts after the refusal cannot see it.
//!
//! Three of the five tests assert an **absence**: no event is built for a guest-kernel
//! channel, no event is forged for an engine with no honest route, and no fault is ever
//! attributed to a channel other than the one that was refused. An emitter is only worth
//! having if it declines, and this project has been bitten four times by a gate that
//! could not fire.
//!
//! ## ⚠ What this file does NOT establish
//!
//! It runs no guest. Nothing here proves that a stock NVIDIA driver, having received
//! these bytes, kills the channel and that CUDA reports an error — the driver hangs
//! before compute today (task #123), so no such run is available. The evidence here is
//! **mock-level**: the bytes match the layout the generator lifted out of the vendored
//! driver, the transport accepts them, and an independent re-implementation of the
//! guest's own receive path (`kayfabe_tests::gspworld::Guest`) reads them back. The live
//! test that would close the gap is named in the design note §9.

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped handle/VA literals

use kayfabe_abi::generated::rpc::{
    NV_VGPU_MSG_EVENT_RC_TRIGGERED, ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT, RpcRcTriggeredV1702,
};
use kayfabe_abi::rc::{EngineRoute, RC_NOTIFIER_SCOPE_TSG, RcTriggered};
use kayfabe_arch::fault::{MmuFaultAccess, MmuFaultCause, MmuFaultCodes};
use kayfabe_arch::ids::{EngineKind, GpuId, GpuVa, HClient, Pdb, VChid};
use kayfabe_core::fault::{FaultVerdict, NotAttributable, verdict};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_device::ga10x::Ga10xFaultCodes;
use kayfabe_fwd::{FwdFault, fault_facts, handle_doorbell, route_doorbell};
use kayfabe_mmu::AddressFault;
use kayfabe_mocks::{MockArch, MockIsolateFactory};
use kayfabe_rmrpc::{FaultEmitRefusal, rc_triggered_for};
use kayfabe_tests::{Guarded, Scenario, identical_handles};

/// The address the application submits and never maps. Its shape is the #14 round-1
/// working-set base, so a reader recognises it as a plausible operand rather than a
/// sentinel.
const BAD_VA: GpuVa = GpuVa(0x2_0020_0000);
const A_PDB: Pdb = Pdb(0x3401_000);
const B_PDB: Pdb = Pdb(0x3405_000);
const A_GR: VChid = VChid(0x10);
const A_CE: VChid = VChid(0x11);
const B_GR: VChid = VChid(0x20);

/// The chip's Axis-B encoding table — the same one a realized device would use.
const CODES: Ga10xFaultCodes = Ga10xFaultCodes;

/// `X(RM, SET_GUEST_SYSTEM_INFO, 1)` — the guest's first post-init RPC, used only to
/// drive the FSM to `Running` so an unsolicited event is legal (§7-G7).
const FN_SET_GUEST_SYSTEM_INFO: u32 = 1;

/// Two application processes with identical handles and distinct PDBs (the #14 shape),
/// so "which channel faulted" is a question with a wrong answer available.
fn two_app_gpu() -> Guarded<Gpu> {
    let arch = Box::new(MockArch::new());
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");
    let mut s = Scenario::new();
    s.compute_process(HClient(0xAA), A_PDB, identical_handles(A_GR.0, A_CE.0));
    s.compute_process(HClient(0xBB), B_PDB, identical_handles(B_GR.0, 0x21));
    for ev in s.events {
        gpu.apply(ev).expect("the two-process bring-up applies");
    }
    Guarded::new("simulated_fault", gpu, rec)
}

/// Ring `vchid` with `va` in its working set and return the refusal — which must be the
/// address-plane miss, by exact variant. A test that accepted `is_err()` here would pass
/// for `NoVas`, `UnknownVchid` or a retired proc, none of which is an application's bad
/// pointer.
fn refuse_ring(gpu: &mut Gpu, vchid: VChid, va: GpuVa, pdb: Pdb) {
    assert_eq!(
        handle_doorbell(gpu, GpuId::ZERO, MockArch::token_for(vchid), &[va]),
        Err(FwdFault::Address(AddressFault::Miss { pdb, va })),
        "the ring-gate must refuse an unmapped working-set VA by exact variant"
    );
}

/// Derive the report the way production would: route the same token, collect the facts
/// beside the refusal, ask the core for a verdict.
fn emit_for(gpu: &Gpu, vchid: VChid, va: GpuVa) -> FaultVerdict {
    let route = route_doorbell(&gpu.spine, GpuId::ZERO, MockArch::token_for(vchid))
        .expect("the refused token still routes — the channel exists, its address did not");
    let proc = gpu
        .procs
        .get(&route.proc)
        .expect("the route names a live proc");
    let facts = fault_facts(
        proc,
        &route,
        va,
        MmuFaultCause::NothingMapped,
        MmuFaultAccess::Write,
    )
    .expect("the channel is still there");
    verdict(facts, Gpu::SYSTEM_PROC)
}

fn read_u32(b: &[u8], c_name: &str) -> u32 {
    let off = RpcRcTriggeredV1702::LAYOUT
        .fields
        .iter()
        .find(|f| f.c_name == c_name)
        .map(|f| f.offset)
        .expect("a field of rpc_rc_triggered_v17_02");
    u32::from_le_bytes(b[off..off + 4].try_into().expect("four bytes"))
}

// =====================================================================================
// 1. THE WHOLE CHAIN — refusal → verdict → bytes → transport → the guest reads it
// =====================================================================================

/// ★★★ An application's unmapped working-set VA becomes an `RC_TRIGGERED` event that an
/// independent re-implementation of the guest's own receive path reads back, carrying
/// **this** channel's id and **this** address.
///
/// Every hop is a real one: the refusal comes out of `handle_doorbell`, the bytes come
/// out of the Axis-A encoder over the generated layout, and the delivery goes through
/// `GspFsm::post_event` onto a queue the guest linked itself.
#[test]
fn an_unmapped_application_va_reaches_the_guest_as_a_channel_fault() {
    let mut gpu = two_app_gpu();

    // ---- The refusal. This is the state that used to end here, and end in a hang.
    refuse_ring(&mut gpu, A_GR, BAD_VA, A_PDB);

    // ---- The verdict: an application's context, so we MAY tell the guest.
    let FaultVerdict::Emit(report) = emit_for(&gpu, A_GR, BAD_VA) else {
        panic!("an application channel's unmapped VA must be emittable");
    };
    assert_eq!(report.vchid, A_GR, "the fault names the channel we refused");
    assert_eq!(report.va, BAD_VA);
    assert_eq!(report.pdb, Some(A_PDB));
    assert_eq!(report.engine, EngineKind::GrCompute);

    // ---- The bytes.
    let rpc = rc_triggered_for(
        &report,
        &CODES,
        &kayfabe_tests::gspworld::FUNCTIONS,
        ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT,
    )
    .expect("a GR channel has an honest engine route");
    assert_eq!(rpc.function, NV_VGPU_MSG_EVENT_RC_TRIGGERED);
    assert_eq!(rpc.payload.len(), RpcRcTriggeredV1702::SIZE);
    assert_eq!(
        read_u32(&rpc.payload, "chid"),
        u32::from(A_GR.0),
        "★ attribution: `chid` is the runlist index of the channel that was refused"
    );
    assert_eq!(
        read_u32(&rpc.payload, "exceptType"),
        ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT,
        "the exception type is the MMU-fault one — `Xid 31` in a kernel log"
    );
    assert_eq!(
        (u64::from(read_u32(&rpc.payload, "mmuFaultAddrHi")) << 32)
            | u64::from(read_u32(&rpc.payload, "mmuFaultAddrLo")),
        BAD_VA.0,
        "★ attribution: the faulting address is the one the submission named"
    );
    assert_eq!(
        read_u32(&rpc.payload, "mmuFaultType"),
        CODES.fault_type(MmuFaultCause::NothingMapped),
        "the fault type comes from the chip's table, not from this test"
    );
    assert_eq!(read_u32(&rpc.payload, "scope"), RC_NOTIFIER_SCOPE_TSG);
    assert_eq!(
        read_u32(&rpc.payload, "rcJournalBufferSize"),
        0,
        "an empty journal — we have no register dump and do not invent one"
    );

    // ---- The transport, and the guest reading it back.
    let mut w = kayfabe_tests::gspworld::GspWorld::new(
        kayfabe_tests::gspworld::P580,
        kayfabe_tests::gspworld::MODEL_A,
    );
    w.boot();
    w.link_and_drain();
    w.guest
        .send(&mut w.ram, FN_SET_GUEST_SYSTEM_INFO, 5, &[])
        .expect("the guest issues its first post-init RPC");
    w.doorbell().expect("we service it");
    w.guest.recv(&mut w.ram).expect("and it drains the reply");

    w.fsm
        .post_event(&mut w.ram, &rpc)
        .expect("★ a Running GSP may post an unsolicited RC event");
    let got = w.guest.recv(&mut w.ram).expect("the guest reads its queue");
    assert_eq!(got.len(), 1, "exactly one event arrived");
    assert_eq!(
        got[0].function, NV_VGPU_MSG_EVENT_RC_TRIGGERED,
        "★★ the guest's own receive path classifies it as RC_TRIGGERED"
    );
    assert_eq!(
        got[0].payload.len(),
        RpcRcTriggeredV1702::SIZE,
        "the whole payload survived the element encoding"
    );
    assert_eq!(
        read_u32(&got[0].payload, "chid"),
        u32::from(A_GR.0),
        "★★★ and the channel id survived the round trip — the guest can attribute it"
    );
}

// =====================================================================================
// 2. THE OTHER FAILURE MODE — the guest kernel's own address
// =====================================================================================

/// ★★★ A miss on the **guest kernel's** channel is escalated, and **no event is built**.
///
/// This is failure mode B (`kayfabe_core::fault`'s module docs). The assertion that
/// matters is the negative one: the verdict is not merely different, it carries no
/// report, so there is nothing an over-eager caller could encode and post. Emitting here
/// would tell the guest kernel that its own correctly-declared address faulted in
/// hardware, and would dress one of *our* defects as one of *its*.
#[test]
fn a_guest_kernel_channels_miss_is_escalated_and_builds_no_event() {
    let arch = Box::new(MockArch::new());
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");
    // A KERNEL-privileged client, built out by hand rather than through
    // `Scenario::compute_process` — that helper declares a USER root, and the declared
    // `ClientKind` is the entire discriminator under test, so borrowing a helper that
    // hard-codes the other value would test nothing.
    const KC: HClient = HClient(0xC1D0_0069);
    const K_ROOT: kayfabe_arch::ids::HObject = kayfabe_arch::ids::HObject(0xC1D0_0069);
    const K_DEV: kayfabe_arch::ids::HObject = kayfabe_arch::ids::HObject(0x5C00_0001);
    const K_VAS: kayfabe_arch::ids::HObject = kayfabe_arch::ids::HObject(0x5C00_0010);
    const K_TSG: kayfabe_arch::ids::HObject = kayfabe_arch::ids::HObject(0x5C00_0012);
    const K_CHAN: kayfabe_arch::ids::HObject = kayfabe_arch::ids::HObject(0x5C00_0019);
    const K_VCHID: VChid = VChid(0x30);
    const K_PDB: Pdb = Pdb(0x3409_000);
    use kayfabe_core::rmgraph::{AllocFacts, RmEvent};
    use kayfabe_mocks::mock_classes as mc;
    for ev in [
        kayfabe_tests::kernel_client_root(KC),
        RmEvent::Alloc {
            client: KC,
            parent: K_ROOT,
            handle: K_DEV,
            class: mc::DEVICE,
            facts: AllocFacts {
                device_instance: Some(0),
                ..Default::default()
            },
        },
        RmEvent::Alloc {
            client: KC,
            parent: K_DEV,
            handle: K_VAS,
            class: mc::VASPACE,
            facts: AllocFacts::default(),
        },
        RmEvent::SetPageDir {
            client: KC,
            vaspace: K_VAS,
            pdb: K_PDB,
        },
        RmEvent::Alloc {
            client: KC,
            parent: K_DEV,
            handle: K_TSG,
            class: mc::TSG,
            facts: AllocFacts {
                h_vaspace: Some(K_VAS),
                ..Default::default()
            },
        },
        RmEvent::Alloc {
            client: KC,
            parent: K_TSG,
            handle: K_CHAN,
            class: mc::CHANNEL_GR,
            facts: AllocFacts {
                h_vaspace: Some(K_VAS),
                userd_flags: MockArch::userd_flags_for(K_VCHID),
                ..Default::default()
            },
        },
    ] {
        gpu.apply(ev).expect("the kernel-client bring-up applies");
    }
    let gpu = Guarded::new("simulated_fault::kernel", gpu, rec);

    let route = route_doorbell(&gpu.spine, GpuId::ZERO, MockArch::token_for(K_VCHID))
        .expect("the kernel channel routes");
    assert_eq!(
        route.proc,
        Gpu::SYSTEM_PROC,
        "a kernel-privileged client's channel belongs to the ONE system component"
    );
    // ★ The system component is NOT in `Gpu::procs` — it is its own field
    // (`Gpu::system`), which is itself a small confirmation of the discriminator: the
    // guest kernel's channels are not one of the application `Proc`s at all.
    let facts = fault_facts(
        &gpu.system,
        &route,
        BAD_VA,
        MmuFaultCause::NothingMapped,
        MmuFaultAccess::Write,
    )
    .expect("the channel exists");

    match verdict(facts, Gpu::SYSTEM_PROC) {
        FaultVerdict::Escalate(NotAttributable::GuestKernelContext { vchid, .. }) => {
            assert_eq!(vchid, K_VCHID, "the escalation names the same channel");
        }
        other => panic!("a guest-kernel miss must never be emittable, got {other:?}"),
    }
}

// =====================================================================================
// 3. THE EMITTER DECLINES RATHER THAN GUESSES
// =====================================================================================

/// ★★ A copy-engine fault is **refused**, not forged as a graphics-engine one.
///
/// The receiver resolves `nv2080EngineType` to a channel-id manager and returns early on
/// failure, silently (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:578-583`).
/// So a guessed engine produces a message that is parsed, dropped and counted by *us* as
/// delivered — a false green on the emitter's own side. Refusing is the only honest
/// outcome while `EngineKind::Ce` carries no instance.
#[test]
fn a_copy_engine_fault_is_refused_rather_than_forged_as_graphics() {
    let mut gpu = two_app_gpu();
    refuse_ring(&mut gpu, A_CE, BAD_VA, A_PDB);

    let FaultVerdict::Emit(report) = emit_for(&gpu, A_CE, BAD_VA) else {
        panic!("a CE channel is still an application's channel — the VERDICT is Emit");
    };
    assert_eq!(report.engine, EngineKind::Ce);
    assert_eq!(
        rc_triggered_for(
            &report,
            &CODES,
            &kayfabe_tests::gspworld::FUNCTIONS,
            ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT,
        ),
        Err(FaultEmitRefusal::NoEngineRoute {
            engine: EngineKind::Ce
        }),
        "★ the ENCODER declines, and it declines by name so a caller must handle it"
    );
}

// =====================================================================================
// 4. ATTRIBUTION UNDER THE #14 SHAPE
// =====================================================================================

/// ★★★ Two processes fault on the **identical** VA and each event names its own channel.
///
/// The #14 shape is the one where attribution is genuinely hard: identical guest VAs,
/// identical guest handles, distinct PDBs. A fault emitter that keyed on the address —
/// or on anything global — would attribute both to whichever proc it found first, and a
/// fault the guest cannot attribute is, as the task puts it, nearly useless to it.
#[test]
fn identical_vas_in_two_processes_produce_two_differently_attributed_faults() {
    let mut gpu = two_app_gpu();
    refuse_ring(&mut gpu, A_GR, BAD_VA, A_PDB);
    refuse_ring(&mut gpu, B_GR, BAD_VA, B_PDB);

    let mut seen = Vec::new();
    for (vchid, pdb) in [(A_GR, A_PDB), (B_GR, B_PDB)] {
        let FaultVerdict::Emit(report) = emit_for(&gpu, vchid, BAD_VA) else {
            panic!("both are application channels");
        };
        assert_eq!(
            report.pdb,
            Some(pdb),
            "each fault names its OWN address space"
        );
        let rpc = rc_triggered_for(
            &report,
            &CODES,
            &kayfabe_tests::gspworld::FUNCTIONS,
            ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT,
        )
        .expect("both are GR");
        seen.push((report.proc, read_u32(&rpc.payload, "chid")));
    }
    assert_eq!(seen[0].1, u32::from(A_GR.0));
    assert_eq!(seen[1].1, u32::from(B_GR.0));
    assert_ne!(
        seen[0].0, seen[1].0,
        "★ and the two faults belong to two different procs — no aliasing"
    );
    assert_ne!(
        seen[0].1, seen[1].1,
        "★★★ the identical VA did NOT collapse the two faults onto one channel"
    );
}

// =====================================================================================
// 5. TOTALITY — there is no third outcome
// =====================================================================================

/// ★★ Over every engine kind and both ownerships, the path ends in **either** a
/// well-formed event **or** a named refusal — never in nothing.
///
/// "Nothing" is the failure this task removes, and it is the one the C had at seven
/// separate sites. Quantified over a list that is derived from `EngineKind`'s own
/// variants rather than hand-picked: a new engine arm added tomorrow is in scope
/// tomorrow, which is the `gates_quantified_over_a_list` lesson applied to a test.
#[test]
fn every_engine_and_ownership_ends_in_an_event_or_a_named_refusal() {
    let all_engines = [
        EngineKind::GrCompute,
        EngineKind::GrGraphics,
        EngineKind::Ce,
        EngineKind::NvEnc,
        EngineKind::NvDec,
        EngineKind::Other,
    ];
    // The universe is pinned against the enum by exhaustive `match`: adding a variant
    // stops this compiling until the list grows, so the quantifier cannot silently shrink.
    for e in all_engines {
        match e {
            EngineKind::GrCompute
            | EngineKind::GrGraphics
            | EngineKind::Ce
            | EngineKind::NvEnc
            | EngineKind::NvDec
            | EngineKind::Other => {}
        }
    }

    let mut emitted = 0usize;
    let mut refused = 0usize;
    let mut escalated = 0usize;
    for engine in all_engines {
        for owner in [kayfabe_core::ProcId(7), Gpu::SYSTEM_PROC] {
            let facts = kayfabe_core::fault::FaultFacts {
                gpu: GpuId::ZERO,
                proc: owner,
                chan: kayfabe_core::ChanId(3),
                vchid: A_GR,
                pdb: Some(A_PDB),
                va: BAD_VA,
                engine,
                cause: MmuFaultCause::NothingMapped,
                access: MmuFaultAccess::Read,
            };
            match verdict(facts, Gpu::SYSTEM_PROC) {
                FaultVerdict::Escalate(_) => escalated += 1,
                FaultVerdict::Emit(report) => match rc_triggered_for(
                    &report,
                    &CODES,
                    &kayfabe_tests::gspworld::FUNCTIONS,
                    ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT,
                ) {
                    Ok(rpc) => {
                        assert_eq!(rpc.payload.len(), RpcRcTriggeredV1702::SIZE);
                        assert!(EngineRoute::for_engine(engine).is_some());
                        emitted += 1;
                    }
                    Err(FaultEmitRefusal::NoEngineRoute { engine: e }) => {
                        assert_eq!(e, engine);
                        refused += 1;
                    }
                    Err(other) => panic!("unexpected refusal {other:?}"),
                },
            }
        }
    }
    // Non-vacuity, as counts rather than as a claim: both arms were REACHED.
    assert_eq!(
        escalated,
        all_engines.len(),
        "the system proc always escalates"
    );
    assert_eq!(emitted, 2, "GrCompute and GrGraphics have honest routes");
    assert_eq!(refused, 4, "the other four decline by name");
}

/// ★ A channel with **no declared VAS** is its own escalation, distinct from the
/// guest-kernel one — because a caller told only "not attributable" cannot tell an
/// ownership question from a modelling gap.
#[test]
fn a_channel_with_no_address_space_escalates_with_its_own_reason() {
    let facts = kayfabe_core::fault::FaultFacts {
        gpu: GpuId::ZERO,
        proc: kayfabe_core::ProcId(7),
        chan: kayfabe_core::ChanId(3),
        vchid: A_GR,
        pdb: None,
        va: BAD_VA,
        engine: EngineKind::GrCompute,
        cause: MmuFaultCause::NothingMapped,
        access: MmuFaultAccess::Read,
    };
    assert_eq!(
        verdict(facts, Gpu::SYSTEM_PROC),
        FaultVerdict::Escalate(NotAttributable::NoAddressSpace {
            chan: kayfabe_core::ChanId(3)
        })
    );
}

/// ★ The Axis-B table is total over the abstract vocabulary and stays inside the range
/// the guest's own decoder accepts.
///
/// `kgmmuGetFaultType_GV100` returns `NV_ERR_INVALID_ARGUMENT` from its `default:` arm
/// (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/arch/volta/kern_gmmu_gv100.c:1699-1780`),
/// and that failure aborts the guest's whole drain loop — so one out-of-range code loses
/// the faults queued behind it too.
#[test]
fn the_chip_fault_codes_stay_inside_the_range_the_guest_decodes() {
    for cause in [
        MmuFaultCause::NothingMapped,
        MmuFaultCause::PermissionViolation,
    ] {
        let t = CODES.fault_type(cause);
        assert!(
            t <= 0xf,
            "NV_PFAULT_FAULT_TYPE is a 5-bit field with 16 defined values; {t:#x} is not one"
        );
    }
    for access in [
        MmuFaultAccess::Read,
        MmuFaultAccess::Write,
        MmuFaultAccess::Atomic,
    ] {
        let a = CODES.access_type(access);
        assert!(
            matches!(a, 0x0..=0x4),
            "a VIRTUAL access type; {a:#x} is a physical one or undefined"
        );
    }
    assert_ne!(
        CODES.fault_type(MmuFaultCause::NothingMapped),
        CODES.fault_type(MmuFaultCause::PermissionViolation),
        "two different causes must not encode to one code"
    );
}

/// ★ The encoder's own refusal is reachable and leaves nothing written — the boundary-1
/// posture applied to a buffer we own rather than to guest input.
#[test]
fn a_truncated_event_buffer_writes_nothing() {
    let ev = RcTriggered {
        engine: EngineRoute::for_engine(EngineKind::GrCompute).expect("GR routes"),
        chid: 9,
        except_type: ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT,
        scope: RC_NOTIFIER_SCOPE_TSG,
        mmu_fault_addr: BAD_VA.0,
        mmu_fault_type: CODES.fault_type(MmuFaultCause::NothingMapped),
    };
    let mut b = [0xAAu8; RpcRcTriggeredV1702::SIZE - 1];
    assert!(ev.encode_into(&mut b).is_err());
    assert!(b.iter().all(|&x| x == 0xAA), "a refusal wrote nothing");
}
