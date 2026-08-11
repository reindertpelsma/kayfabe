//! Batch-3 the ONE pushbuffer parser + working-set publication — the #14 fix through
//! the REAL exec path (`execution_plane.md` §2.3/§2.4).
//!
//! - A scripted pushbuffer (via `MockArch`'s `PushbufferAbi`) → the address table is
//!   forward-populated for CE PT-write dsts (#13 capture), the `SemRelease` is
//!   `observe`d on the owning proc's completion queue, a `TlbInvalidate` membar is
//!   recorded, and an `Opaque` method changes no core state.
//! - The 2-proc sim: identical guest VAs in two Procs each publish their working set
//!   into their OWN host VAS, so `gate_working_set` passes for BOTH (the #14 fix); an
//!   unpublished VA at ring time is a loud fault (the proven EXECUTION-fault root).
//! - A soak variant: a sustained submit/complete loop loses no completion.
//! - **The hostile-input boundary, measured** (`core_mutation_gate.md` §L1 baseline):
//!   `total_read_budget_clamps_a_straddling_range_to_what_is_left` puts the
//!   `MAX_PUSH_TOTAL_BYTES` edge *inside* a GPFIFO range so the remaining-budget clamp
//!   is observable at all, and
//!   `sem_release_completion_identities_mix_both_operands_and_never_collide` pins that
//!   a `SemRelease`'s completion identity mixes BOTH operands, so two guest fences can
//!   never fold onto one queue entry. Both close campaign survivors.
//!
//! Invariant/contract tests (decision #15), mock-driven, GPU-free.

#![allow(clippy::unusual_byte_groupings)]

use kayfabe_arch::ids::GpuId;
use kayfabe_arch::ids::{GpuVa, HClient, Pdb, VChid};
use kayfabe_completion::{BatchId, OsEventRef};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_fwd::{FwdFault, gate_working_set, handle_doorbell, parse_pushbuffer, publish_backing};
use kayfabe_mocks::{
    MockArch, MockIsolateFactory, MockPushbuffer, MockVmm, RmVerb, SharedRecorder,
    mock_classes as mc,
};
use kayfabe_tests::{Guarded, Scenario, bind_ring, identical_handles, pb_va, script_ring_via};

const A_PDB: Pdb = Pdb(0x3401_000);
const B_PDB: Pdb = Pdb(0x3405_000);
const SHARED_VA: GpuVa = GpuVa(0x2_0020_0000);

/// Lay out a GPFIFO ring (one entry) + the method words it names, and **bind the range**
/// in the issuing channel's address table — i.e. do what the guest's own driver does: map
/// the pushbuffer, then name the resulting GPU VA in the entry.
///
/// ★★★ The entry names `pb_va(gpa)`, **not** `gpa`, and the bias is load-bearing. A ring
/// whose VA equals the GPA its bytes live at cannot distinguish a translating
/// `read_pushbuffer` from the untranslated one it replaces — which is exactly what
/// `MockPushbuffer`'s GPA-carrying entry baked in, and what hid a wrong-bytes read for the
/// whole life of the seam (`execution_plane_increments.md` §8.2.3).
fn script_pushbuffer(
    gpu: &mut Gpu,
    vmm: &mut MockVmm,
    pid: kayfabe_core::ProcId,
    cid: kayfabe_core::ChanId,
    gpa: u64,
    methods: &[(u32, Vec<u32>)],
) -> Vec<u8> {
    let ring = script_ring_via(vmm, gpa, methods);
    bind_ring(gpu, pid, cid, &ring);
    ring
}

fn one_proc_gpu() -> (Guarded<Gpu>, MockVmm) {
    let arch = Box::new(MockArch::new());
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");
    let mut s = Scenario::new();
    s.compute_process(HClient(0xAA), A_PDB, identical_handles(0x10, 0x11));
    for ev in s.events {
        gpu.apply(ev).expect("applies");
    }
    (
        Guarded::new("pushbuffer_parser::one_proc_gpu", gpu, rec),
        MockVmm::new(),
    )
}

/// A scripted pushbuffer drives ALL four fact kinds: CE-PT-write capture latches
/// `pt_pages`, `SemRelease` is observed, `TlbInvalidate` is recorded, `Opaque` changes
/// nothing.
///
/// ★★★ **#102 — this test was rewritten around the operand split, and the old form was
/// a tautology over the bug.** It used to assert that a physical CE destination is
/// captured *and then resolves as a virtual address to itself* — i.e. it pinned
/// `phys: dst.0`, a binding that publishes nothing, as if it were a fact about the guest.
/// It also asserted that *"a VIRTUAL CE dst is a data copy, NOT a PT write"*, which is
/// false and is #13: the guest kernel's copy-engine utility identity-maps the whole
/// framebuffer into its own address space at 512 MiB pages and issues its page-table
/// writes as **virtual-destination** copies (`C: nvkvm_gpu_emul.c:4936-4952`).
///
/// What decides is what the operands CARRY, read off the **resolved physical**
/// destination: does it land on a tracked page-table page? The physical-destination case
/// is below; the virtual one has its own test.
#[test]
fn scripted_pushbuffer_captures_pt_writes_and_observes_completion() {
    let (mut gpu, mut vmm) = one_proc_gpu();
    let pid = *gpu.spine.by_pdb.get(&(GpuId::ZERO, A_PDB)).unwrap();
    let cid = *gpu.procs[&pid].chan_ids.values().next().unwrap();

    // The VAS's page-directory ROOT — a PDB *is* the physical address of its root page
    // directory, so this page is a declared fact, not a discovered one.
    let pt_page: u64 = A_PDB.0 & !0xfff;
    let sem_addr: u64 = 0x2_0030_0000;
    let ring = script_pushbuffer(
        &mut gpu,
        &mut vmm,
        pid,
        cid,
        0x5000_0000,
        &[
            MockPushbuffer::set_object(mc::DMA_COPY),
            // Physical CE dst ON a page-table page = a PT write (#13 capture).
            MockPushbuffer::ce_launch_dma(pt_page + 0x40, 0x1000, false),
            // A physical CE dst that is NOT a page-table page is a DATA copy: forwarded,
            // never intercepted. (The old test asserted the discriminator was the
            // virtual/physical FORM; it is not.)
            MockPushbuffer::ce_launch_dma(0x1_2340_0000, 0x2000, false),
            // An unresolvable VIRTUAL dst is likewise data — we do not track user data
            // addresses, and not tracking one must never be read as a page-table write.
            MockPushbuffer::ce_launch_dma(0x2_0040_0000, 0x2000, true),
            MockPushbuffer::sem_release(sem_addr, 0xabc),
            MockPushbuffer::tlb_invalidate(A_PDB.0, true),
            // An opaque method (unknown opcode) — passed through, no state change.
            MockPushbuffer::method(0xEE, &[1, 2, 3]),
        ],
    );

    let out = parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring).expect("parses");

    // Exactly ONE of the three copies is a page-table write, and it names its owner.
    assert_eq!(out.pt_writes.len(), 1, "one PT write among three copies");
    assert_eq!(
        (
            out.pt_writes[0].page,
            out.pt_writes[0].owner,
            out.pt_writes[0].owner_pdb
        ),
        (pt_page, pid, A_PDB),
        "captured as a PT page, attributed to the Vas that owns it"
    );
    assert_eq!(
        out.data_copies, 2,
        "the other two are VA-operand work: forwarded, not intercepted"
    );
    assert!(
        gpu.procs[&pid].vases[&(GpuId::ZERO, A_PDB)]
            .pt_pages
            .contains(&pt_page),
        "latched into the OWNING Vas.pt_pages"
    );
    // SemRelease observed.
    assert_eq!(out.sem_releases, vec![(GpuVa(sem_addr), 0xabc)]);
    assert!(
        gpu.procs[&pid].completion.has_outstanding(),
        "completion observed on the proc's queue"
    );
    // TlbInvalidate membar recorded.
    assert_eq!(out.invalidates, vec![(A_PDB, true)]);
    // Opaque method counted, changed nothing else.
    assert_eq!(out.opaque, 1);

    // ★★★ And the captured page is **NOT** bound as a mapping of itself.
    //
    // The old test asserted the opposite — that `pt_page + 0x40` resolves — which was
    // true only because the CE arm bound `phys: dst.0`: a page-table page mapped, as a
    // virtual address, to its own physical address. That is not a fact about the guest,
    // it is the identity function. It publishes nothing, it is the *destination* rather
    // than anything the destination describes, and its aperture was hardcoded `Vidmem`
    // where the C reads it from PTE bits [2:1].
    //
    // A latch is a latch. What the page's CONTENT describes is decoded at the guest's own
    // commit point (the semaphore release) and bound then — the next stage.
    assert!(
        kayfabe_fwd::resolve(&gpu, GpuId::ZERO, A_PDB, GpuVa(pt_page + 0x40)).is_err(),
        "a page-table page does not map to itself — the latch binds NOTHING"
    );
}

/// ★★★ **#102 stage C1 — the C's EXECUTE predicate, row for row** (`C:
/// nvkvm_gpu_emul.c:6310`).
///
/// A literal transcription of the C's conjunction rather than a re-derivation of it:
/// each row states the four inputs and the answer the C gives, so a change to
/// [`kayfabe_fwd::ce_executor_c`]'s *formula* cannot also change what it is being
/// checked against.
///
/// The two rows that matter most, and that stage B could not express at all:
/// - **`(Copy, GuestKernel, virtual, virtual)` ⇒ `Ours`.** Every guest-kernel CE copy is
///   ours in the C — *including* the framebuffer-alias page-table write, which is
///   virtual-destination and passes any purely operand-carried test.
/// - **`(Scrub|Fill, User, virtual, virtual)` ⇒ `Ours`.** A scrub or a fill is never
///   handed to the host copy engine, whoever submitted it.
#[test]
fn ce_execute_predicate_is_the_c_row_for_row() {
    use kayfabe_arch::CeWork::{Copy, Scrub};
    // A non-uniform pattern: a byte-uniform one hides every phase error downstream.
    const FILL: kayfabe_arch::CeWork = kayfabe_arch::CeWork::Fill {
        pattern: 0xdead_beef,
    };
    use kayfabe_fwd::CeExecutor::{HostCe, Ours};
    use kayfabe_fwd::ChannelOrigin::{GuestKernel, User};
    use kayfabe_fwd::{ChannelOrigin, ce_executor_c};

    // (work, origin, src_is_virtual, dst_is_virtual) -> the C's `host_ce`.
    // The FULL cross product: 3 work kinds x 2 origins x 2 x 2 = 24 rows, none omitted.
    let rows = [
        // The ONE combination the C forwards.
        ((Copy, User, true, true), HostCe),
        // …and every one of its 23 neighbours.
        ((Copy, User, true, false), Ours),
        ((Copy, User, false, true), Ours),
        ((Copy, User, false, false), Ours),
        ((Copy, GuestKernel, true, true), Ours),
        ((Copy, GuestKernel, true, false), Ours),
        ((Copy, GuestKernel, false, true), Ours),
        ((Copy, GuestKernel, false, false), Ours),
        ((Scrub, User, true, true), Ours),
        ((Scrub, User, true, false), Ours),
        ((Scrub, User, false, true), Ours),
        ((Scrub, User, false, false), Ours),
        ((Scrub, GuestKernel, true, true), Ours),
        ((Scrub, GuestKernel, true, false), Ours),
        ((Scrub, GuestKernel, false, true), Ours),
        ((Scrub, GuestKernel, false, false), Ours),
        ((FILL, User, true, true), Ours),
        ((FILL, User, true, false), Ours),
        ((FILL, User, false, true), Ours),
        ((FILL, User, false, false), Ours),
        ((FILL, GuestKernel, true, true), Ours),
        ((FILL, GuestKernel, true, false), Ours),
        ((FILL, GuestKernel, false, true), Ours),
        ((FILL, GuestKernel, false, false), Ours),
    ];
    assert_eq!(rows.len(), 24, "the cross product, not a sample of it");
    for ((work, origin, srcv, dstv), want) in rows {
        assert_eq!(
            ce_executor_c(work, origin, srcv, dstv),
            want,
            "C: :6310 row ({work:?}, {origin:?}, src_virt={srcv}, dst_virt={dstv})"
        );
    }

    // The `is_user_ce` conjunct's port: the system proc IS the guest-kernel component.
    assert_eq!(ChannelOrigin::of(Gpu::SYSTEM_PROC), GuestKernel);
    assert_eq!(ChannelOrigin::of(kayfabe_core::ProcId(1)), User);
}

/// ★★★ **#102 stage C1 — EXECUTE and CAPTURE are two decisions, and they DISAGREE on
/// the same pushbuffer** (`eight_blockers_resolved.md` §11.5).
///
/// Stage B folded them: it classified on the resolved physical destination (right, for
/// capture) and answered execute by routing everything non-phys to hardware (wrong —
/// nobody had made that decision). Three commands here, and no function of one tally
/// yields the other:
///
/// | command | CAPTURE | EXECUTE |
/// |---|---|---|
/// | physical copy onto the VAS's page-directory root | PT write | ours (`dst_phys`) |
/// | user scrub of an untracked virtual address | data | ours (`mscrub`) |
/// | user copy, virtual→virtual, untracked | data | **host CE** |
///
/// `pt_writes=1, data_copies=2` and `host_ce=1, ours=2` count the SAME three commands,
/// and the pairing between them is the point: the row that is "not a page-table write"
/// is not therefore hardware's, and the row that is hardware's is not therefore
/// uninteresting to the address plane.
#[test]
fn execute_and_capture_are_two_decisions_and_they_disagree() {
    use kayfabe_arch::CeWork;

    let (mut gpu, mut vmm) = one_proc_gpu();
    let pid = *gpu.spine.by_pdb.get(&(GpuId::ZERO, A_PDB)).unwrap();
    let cid = *gpu.procs[&pid].chan_ids.values().next().unwrap();
    let pt_page: u64 = A_PDB.0 & !0xfff;

    let ring = script_pushbuffer(
        &mut gpu,
        &mut vmm,
        pid,
        cid,
        0x5000_0000,
        &[
            MockPushbuffer::set_object(mc::DMA_COPY),
            // CAPTURE: PT write.  EXECUTE: ours — a physical destination is never the
            // host copy engine's in the C, whoever submitted it.
            MockPushbuffer::ce_launch_dma_full(
                pt_page + 0x40,
                false,
                GpuVa(0x9_0000_0000),
                true,
                0x1000,
                CeWork::Copy,
            ),
            // CAPTURE: data (untracked).  EXECUTE: ours — `mscrub`.
            MockPushbuffer::ce_launch_dma_full(
                0x2_0040_0000,
                true,
                GpuVa(0),
                true,
                0x2000,
                CeWork::Scrub,
            ),
            // CAPTURE: data (untracked).  EXECUTE: host CE — the one forwarded row.
            MockPushbuffer::ce_launch_dma_full(
                0x2_0080_0000,
                true,
                GpuVa(0x2_00c0_0000),
                true,
                0x2000,
                CeWork::Copy,
            ),
        ],
    );

    let out = parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring).expect("parses");

    assert_eq!(
        (out.pt_writes.len(), out.data_copies),
        (1, 2),
        "CAPTURE: one page-table write among three copies"
    );
    assert_eq!(
        (out.c_execute_host_ce, out.c_execute_ours),
        (1, 2),
        "EXECUTE: exactly one of the three is the host copy engine's"
    );
    // ★ The non-vacuity of the split, stated as an equation rather than a hope: the two
    // partitions cover the same three commands and are NOT the same partition.
    assert_eq!(
        out.pt_writes.len() + out.data_copies,
        out.c_execute_host_ce + out.c_execute_ours,
        "both decisions see every LAUNCH_DMA"
    );
    assert_ne!(
        (out.pt_writes.len(), out.data_copies),
        (out.c_execute_ours, out.c_execute_host_ce),
        "…and they are not each other's mirror either"
    );
}

/// A hostile / truncated ring never panics and never desyncs the parser into an
/// unbounded read — and ★★★ **an entry naming a VA the guest never bound FAULTS BY NAME**
/// rather than reading whatever guest RAM shares the number (§8.2.3, the E5 control).
///
/// The two arms are the whole point of the pair: the unbound entry is refused before a
/// byte is fetched, and the *same* nonsense bytes behind a **bound** entry parse fine and
/// decode to nothing actionable. Without the second arm, "it faulted" would be
/// indistinguishable from "the parser refuses everything".
#[test]
fn hostile_ring_never_panics() {
    let (mut gpu, mut vmm) = one_proc_gpu();
    let pid = *gpu.spine.by_pdb.get(&(GpuId::ZERO, A_PDB)).unwrap();
    let cid = *gpu.procs[&pid].chan_ids.values().next().unwrap();

    // ARM 1 — a bogus GPFIFO entry naming an UNBOUND VA + a truncated tail.
    let mut ring = Vec::new();
    ring.extend_from_slice(&0x9000_0000u64.to_le_bytes());
    ring.extend_from_slice(&64u64.to_le_bytes());
    ring.push(0xff); // truncated trailing entry (ignored)
    let err = parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring)
        .expect_err("an unbound pushbuffer VA is a MISS, and a MISS is a FAULT");
    assert_eq!(
        err,
        FwdFault::Address(kayfabe_mmu::AddressFault::Miss {
            pdb: A_PDB,
            va: GpuVa(0x9000_0000),
        }),
        "the refusal names the address space and the exact faulting VA — never a zero, \
         never a read of the guest RAM that happens to share the number"
    );

    // ARM 2 — the same shape, but the guest bound it first. Now the bytes are read, and
    // unwritten RAM decodes to nothing actionable. No panic, no desync, no completion.
    let mut ring = Vec::new();
    ring.extend_from_slice(&pb_va(0x9000_0000).0.to_le_bytes());
    ring.extend_from_slice(&64u64.to_le_bytes());
    kayfabe_tests::bind_ring_at(&mut gpu, pid, cid, pb_va(0x9000_0000), 0x9000_0000, 64);
    ring.push(0xff); // truncated trailing entry (ignored)
    let out = parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring).expect("does not panic");
    assert!(out.sem_releases.is_empty());
}

// ---------------------------------------------------------------------------------
// ★ The #14 fix through the REAL exec path: per-Vas working-set publication.
// ---------------------------------------------------------------------------------

fn two_proc_gpu() -> (Guarded<Gpu>, MockVmm, SharedRecorder) {
    let arch = Box::new(MockArch::new());
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");
    let mut s = Scenario::new();
    s.compute_process(HClient(0xAA), A_PDB, identical_handles(0x10, 0x11));
    s.compute_process(HClient(0xBB), B_PDB, identical_handles(0x20, 0x21));
    for ev in s.events {
        gpu.apply(ev).expect("applies");
    }
    // ★177: the guest always schedules a channel before ringing it; restore that step
    // so the doorbell gate's fault is the one under test, not `NotScheduled`.
    kayfabe_tests::guest_schedules_every_channel(&mut gpu);
    (
        Guarded::new("pushbuffer_parser::two_proc_gpu", gpu, rec.clone()),
        MockVmm::new(),
        rec,
    )
}

/// ★ THE #14 regression through the exec path — now STRUCTURAL: `handle_doorbell` is
/// the ONE ring path and it gates. Two Procs, identical guest VAs: neither can ring
/// its declared working set before publishing (loud fault, ZERO host ops — not even
/// channel materialization); after each publishes into its OWN host VAS, both ring,
/// on distinct host tokens.
#[test]
fn t14_per_vas_publication_gates_the_ring() {
    let (mut gpu, _vmm, rec) = two_proc_gpu();
    let pid_a = *gpu.spine.by_pdb.get(&(GpuId::ZERO, A_PDB)).unwrap();
    let pid_b = *gpu.spine.by_pdb.get(&(GpuId::ZERO, B_PDB)).unwrap();
    let cid_a = *gpu.procs[&pid_a].chan_ids.values().next().unwrap();
    let a_token = MockArch::token_for(VChid(0x10));
    let b_token = MockArch::token_for(VChid(0x20));

    // Before publication: a doorbell declaring the identical working-set VA is a
    // LOUD fault for BOTH — the exact #14 EXECUTION fault, refused by the ONE ring
    // path itself (there is no ungated sibling to reach the host through).
    assert!(matches!(
        handle_doorbell(&mut gpu, GpuId::ZERO, a_token, &[SHARED_VA]),
        Err(FwdFault::Address(_))
    ));
    assert!(matches!(
        handle_doorbell(&mut gpu, GpuId::ZERO, b_token, &[SHARED_VA]),
        Err(FwdFault::Address(_))
    ));
    // The refused rings did NOTHING host-side: no channel, no schedule, no doorbell.
    assert!(
        rec.lock().unwrap().log.is_empty(),
        "a gated-out ring performs ZERO host ops"
    );
    // The query form agrees (and cannot ring anything by construction).
    assert!(gate_working_set(&gpu, pid_a, cid_a, &[SHARED_VA]).is_err());

    // Publish the SAME guest VA in each proc's OWN Vas (distinct host VASes).
    let pub_a = publish_backing(
        gpu.procs.get_mut(&pid_a).unwrap(),
        GpuId::ZERO,
        A_PDB,
        SHARED_VA,
        0x10000,
    )
    .unwrap();
    let pub_b = publish_backing(
        gpu.procs.get_mut(&pid_b).unwrap(),
        GpuId::ZERO,
        B_PDB,
        SHARED_VA,
        0x10000,
    )
    .unwrap();
    // ★★★ #102 — corrected (was `assert_ne!`, the wrong reading of #14 — see
    // `sim_14_two_process::t14_identical_va_disjoint_backing` for the full argument).
    // Identical guest VAs map at the SAME host VA in DIFFERENT host VASes; that is what
    // lets each proc's forwarded ring resolve, and the separation is the VAS.
    assert_eq!(
        (pub_a.host_va, pub_b.host_va),
        (SHARED_VA.0, SHARED_VA.0),
        "address identity: both procs are host-mapped AT the guest VA they named"
    );
    assert_ne!(
        gpu.procs[&pid_a].vases[&(GpuId::ZERO, A_PDB)].host_vas,
        gpu.procs[&pid_b].vases[&(GpuId::ZERO, B_PDB)].host_vas,
        "…in DIFFERENT host VASes — the separation #14 actually rests on"
    );

    // Now the SAME ring path passes for BOTH — each resolves in its OWN host VAS.
    let out_a = handle_doorbell(&mut gpu, GpuId::ZERO, a_token, &[SHARED_VA])
        .expect("A rings after publish");
    let out_b = handle_doorbell(&mut gpu, GpuId::ZERO, b_token, &[SHARED_VA])
        .expect("B rings after publish");
    assert_ne!(
        out_a.host_token, out_b.host_token,
        "each proc rang its OWN host token — no cross-proc content-pick"
    );
}

/// Publish `SHARED_VA` in proc A's Vas, then **restate its backing kind** as `bytes`,
/// holding everything else byte-identical — same phys, same aperture, same host object,
/// same host VA (address identity, which `AddressTable::bind` enforces at its entrance).
///
/// ⊘ Everything but [`kayfabe_mmu::BackingBytes`] is held constant on purpose: it is the
/// variable under test, and a two-arm test whose arms differ in more than one place
/// measures nothing. Same discipline as
/// `ce_representability_split::classify_host_backed`, which is the *classifier's* half of
/// the identical question.
fn published_with_backing(
    bytes: kayfabe_mmu::BackingBytes,
) -> (
    Guarded<Gpu>,
    kayfabe_core::ProcId,
    kayfabe_core::ChanId,
    u64,
) {
    let (mut gpu, _vmm, _rec) = two_proc_gpu();
    let pid = *gpu.spine.by_pdb.get(&(GpuId::ZERO, A_PDB)).unwrap();
    let cid = *gpu.procs[&pid].chan_ids.values().next().unwrap();
    publish_backing(
        gpu.procs.get_mut(&pid).unwrap(),
        GpuId::ZERO,
        A_PDB,
        SHARED_VA,
        0x10000,
    )
    .expect("publishes");

    let vas = gpu
        .procs
        .get_mut(&pid)
        .unwrap()
        .vases
        .get_mut(&(GpuId::ZERO, A_PDB))
        .unwrap();
    let (start, len, b) = vas.table.binding_at(SHARED_VA).expect("published");
    assert_eq!(start, SHARED_VA.0, "the publication is the whole range");
    let h = b.host.expect("`publish_backing` writes a host backing");
    assert_eq!(
        h.host_va(),
        SHARED_VA.0,
        "address identity — the rebind below relies on it and `bind` enforces it"
    );
    vas.table
        .unbind(SHARED_VA)
        .expect("it was there to replace");
    vas.table
        .bind(
            A_PDB,
            SHARED_VA,
            len,
            kayfabe_mmu::Binding {
                host: Some(kayfabe_mmu::HostBacking::whole(
                    h.memory(),
                    h.host_va(),
                    bytes,
                )),
                ..b
            },
        )
        .expect("rebinds at the same identity");
    (gpu, pid, cid, len)
}

/// ★★★★★ **A RING NEVER NAMES A BACKING THE GUEST READS SOMEWHERE ELSE** — the #14 gate
/// and the copy-engine classifier, made to answer one question with one authority.
///
/// # ⊘ What this test is, and what it is NOT
///
/// It is **not** a live-defect regression. `[NOT MEASURED as live, census 2026-08-11]` the
/// ring gate is **vacuous in production**: the only production caller of
/// `kayfabe_rt::device::SharedDevice::doorbell` passes `&[]` (`kayfabe-qemu-raw`'s
/// `SharedDoorbell`, which says so and gives the reason — recovering the touched VAs means
/// parsing the ring, increment E4/E5), `kayfabe_fwd::gate_working_set` has test callers
/// only, and `kayfabe_fwd::arm_fence` has no production caller at all. No boot has ever run
/// this predicate over a VA.
///
/// It **is** the disagreement between two authorities for one question, closed before the
/// working set is populated. On 2026-08-11 `kayfabe_fwd::representability_of` was corrected
/// off the predicate `binding.host.is_some()` — *"does a host object exist here"* — onto
/// `BackingBytes`, which asks *"are the guest's bytes in it"*. The ring gate was **not**
/// corrected with it, so the copy-engine classifier refused a `ShadowsGuestMemory` backing
/// by name while the gate admitted the same backing to a ring. `BackingBytes`'s own words
/// settle which site was wrong: *"⊘ Fatal for anything the guest reads or polls, **which is
/// what a ring is**"*.
///
/// # Why TWO arms, and why they must differ
///
/// The known-positive is the half that makes the falsifier mean anything: a gate that
/// refused everything would pass a single-arm test while having destroyed the plane. That
/// is this campaign's §16.85.3 class — a census whose instrument cannot return the other
/// answer — and it is cheap to close here.
///
/// ★ Both **forms** of the gate are asserted, because they are different code paths onto
/// one predicate: the read-only `gate_working_set` query, and the enforcing form inside
/// `VerbPlan::gated_doorbell` reached through `handle_doorbell`. A fix to one only would
/// leave the ring itself open while the query reported it closed — which is strictly worse
/// than the defect, because it manufactures evidence of a repair.
#[test]
fn a_ring_never_names_a_backing_the_guest_reads_somewhere_else() {
    let a_token = MockArch::token_for(VChid(0x10));

    // ---- known-positive: the gate CAN still say yes -----------------------------------
    let (mut gpu, pid, cid, _len) = published_with_backing(kayfabe_mmu::BackingBytes::SoleBacking);
    let sole_query = gate_working_set(&gpu, pid, cid, &[SHARED_VA]);
    assert_eq!(
        sole_query,
        Ok(()),
        "★ the known-positive: a backing that is the range's ONLY memory still rings. If \
         this fails the gate has become a blanket refusal and the other arm proves nothing."
    );
    assert!(
        handle_doorbell(&mut gpu, GpuId::ZERO, a_token, &[SHARED_VA]).is_ok(),
        "…through the ENFORCING form too — the query is not the thing that rings"
    );
    drop(gpu);

    // ---- the falsifier: identical in every respect but the backing kind ---------------
    let (mut gpu, pid, cid, _len) =
        published_with_backing(kayfabe_mmu::BackingBytes::ShadowsGuestMemory);
    let shadow_query = gate_working_set(&gpu, pid, cid, &[SHARED_VA]);
    assert_eq!(
        shadow_query,
        Err(FwdFault::BackingNotGuestVisible {
            addr: SHARED_VA.0,
            aperture: kayfabe_arch::Aperture::SysmemCoherent,
        }),
        "★★★ FORBIDDEN #2 AT THE RING: a second memory at an address the guest reads \
         through the emulated framebuffer must not be handed to a doorbell. And by NAME — \
         reporting it as `AddressFault::Miss` would send a reader hunting a mapping that \
         is present, which is the wrong-name-refusal class §16.108 paid for."
    );
    assert_eq!(
        handle_doorbell(&mut gpu, GpuId::ZERO, a_token, &[SHARED_VA]),
        Err(FwdFault::BackingNotGuestVisible {
            addr: SHARED_VA.0,
            aperture: kayfabe_arch::Aperture::SysmemCoherent,
        }),
        "…and the ENFORCING form refuses with the SAME name — one authority \
         (`ring_admits`), re-derived across the isolate seam's bare-bool collapse, so the \
         query and the ring can never disagree about why"
    );

    // ---- and the arms must DIFFER, or neither measured anything -----------------------
    assert_ne!(
        sole_query, shadow_query,
        "★ the arms must disagree: a gate that answers the same for both is not reading \
         `BackingBytes` at all"
    );
}

/// ★ The gate is STRUCTURAL, not caller discipline: a doorbell whose working set is
/// bound-but-unpublished (the #14 EXECUTION-fault state — the shadow had the VA, the
/// channel's OWN host VAS did not) is refused BEFORE any host op. Once published, the
/// SAME path rings.
///
/// ★ **corrected 2026-07-27, twice over.** This doc used to say *"`handle_doorbell` is
/// the ONE ring path (the ungated `ring_gated` sibling is gone)"*, which the
/// whitepaper's verification pass showed to be false as stated — the L1 path a real
/// guest MMIO write takes is `kayfabe_rt::SharedDevice::doorbell`, which never enters
/// `handle_doorbell`. The invariant re-anchored one level down, onto
/// `kayfabe_fwd::plan_doorbell` as the sole constructor of `VerbPlan::Doorbell`; and it
/// is now enforced by the **type system** rather than by the call graph
/// (`kayfabe_isolate::VerbPlan::gated_doorbell` is the only constructor and it runs the
/// gate — `crates/kayfabe-isolate/tests/ui/ungated_doorbell.rs` pins the compile error,
/// `l1_verb_seam::an_ungated_working_set_cannot_become_a_ring_plan` the runtime half).
/// This test is the end-to-end composition of that property through the real path.
#[test]
fn t14_ring_gate_is_structural_no_ungated_door() {
    let (mut gpu, _vmm, rec) = two_proc_gpu();
    let pid_a = *gpu.spine.by_pdb.get(&(GpuId::ZERO, A_PDB)).unwrap();
    let a_token = MockArch::token_for(VChid(0x10));
    const MEM: kayfabe_arch::ids::HObject = kayfabe_arch::ids::HObject(0x5c00_0100);

    // Bind the VA through the RPC source ONLY (declared backing, host_va = None): it
    // resolves in the guest-side table but was never published into the channel's own
    // host VAS — exactly the #14 EXECUTION-fault shape.
    let mut s = Scenario::new();
    s.memory(
        HClient(0xAA),
        kayfabe_arch::ids::HObject(0x5c00_0001),
        MEM,
        0x9_0000_0000,
    );
    s.map(
        HClient(0xAA),
        kayfabe_arch::ids::HObject(0x5c00_0010),
        MEM,
        SHARED_VA,
        0x10000,
    );
    for ev in s.events {
        gpu.apply(ev).expect("rpc map applies");
    }

    // The ONE ring path refuses it — bound but not host-published — with ZERO host
    // ops (not even channel materialization). There is no ungated door to try instead.
    let before = rec.lock().unwrap().log.len();
    assert_eq!(
        handle_doorbell(&mut gpu, GpuId::ZERO, a_token, &[SHARED_VA]),
        Err(FwdFault::Address(kayfabe_mmu::AddressFault::Miss {
            pdb: A_PDB,
            va: SHARED_VA
        })),
        "the exact miss, by PDB and VA — `matches!(.., Err(FwdFault::Address(_)))` (what \
         this asserted until 2026-07-27) passes for a miss on the WRONG proc's PDB, which \
         is precisely the #14 confusion under test"
    );
    assert_eq!(
        rec.lock().unwrap().log.len(),
        before,
        "a gated-out ring did no host op"
    );

    // The guest eager-unmaps the RPC binding, then the VA is published into the
    // channel's OWN host VAS (host_va = Some): the SAME path now rings.
    gpu.apply(kayfabe_core::rmgraph::RmEvent::Unmap {
        client: HClient(0xAA),
        vaspace: kayfabe_arch::ids::HObject(0x5c00_0010),
        va: SHARED_VA,
    })
    .expect("unmap applies");
    publish_backing(
        gpu.procs.get_mut(&pid_a).unwrap(),
        GpuId::ZERO,
        A_PDB,
        SHARED_VA,
        0x10000,
    )
    .unwrap();
    handle_doorbell(&mut gpu, GpuId::ZERO, a_token, &[SHARED_VA]).expect("published set rings");
    assert!(
        rec.lock()
            .unwrap()
            .log
            .iter()
            .any(|(_, v)| matches!(v, RmVerb::RingDoorbell { .. })),
        "the successful ring reached the host doorbell"
    );
}

/// A VA outside any binding is a loud MISS at gate time — never a guess, never a
/// cross-proc reach.
#[test]
fn t14_unpublished_va_is_a_loud_fault() {
    let (mut gpu, _vmm, _rec) = two_proc_gpu();
    let pid_a = *gpu.spine.by_pdb.get(&(GpuId::ZERO, A_PDB)).unwrap();
    let cid_a = *gpu.procs[&pid_a].chan_ids.values().next().unwrap();
    publish_backing(
        gpu.procs.get_mut(&pid_a).unwrap(),
        GpuId::ZERO,
        A_PDB,
        SHARED_VA,
        0x10000,
    )
    .unwrap();

    // A VA that was never published: loud MISS.
    assert!(matches!(
        gate_working_set(&gpu, pid_a, cid_a, &[GpuVa(0xdead_0000)]),
        Err(FwdFault::Address(_))
    ));
}

/// Soak: a sustained submit/complete loop through the parser loses no completion and
/// the address table does not grow unbounded (reclaim deferred, but re-submits of the
/// same VA do not double-bind).
#[test]
fn soak_submit_complete_loop_loses_no_completion() {
    let (mut gpu, mut vmm) = one_proc_gpu();
    let pid = *gpu.spine.by_pdb.get(&(GpuId::ZERO, A_PDB)).unwrap();
    let cid = *gpu.procs[&pid].chan_ids.values().next().unwrap();

    let mut expected_completions = 0usize;
    for iter in 0..64u64 {
        let sem_addr = 0x2_0030_0000 + iter * 0x1000;
        let ring = script_pushbuffer(
            &mut gpu,
            &mut vmm,
            pid,
            cid,
            0x5000_0000,
            &[MockPushbuffer::sem_release(sem_addr, iter)],
        );
        let out = parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring).expect("parses");
        assert_eq!(out.sem_releases.len(), 1);
        expected_completions += 1;

        // Drain the proc's queue each iter (post + drain + ack) — no completion lost.
        assert!(gpu.procs[&pid].completion.has_outstanding());
        let batch = gpu.pump_completions(GpuId::ZERO).expect("posts");
        assert!(!batch.events.is_empty());
        gpu.completions_drained(GpuId::ZERO);
    }
    assert_eq!(
        expected_completions, 64,
        "every iteration's completion was observed"
    );
    // The address table has no unbounded growth from sema releases (they observe, not
    // bind). ★ The ONE binding present is the pushbuffer's own — the fixture's stand-in
    // for the guest's `MAP_MEMORY_DMA`, re-bound identically all 64 iterations — so this
    // is `1` and not `0`, and a 65th entry would mean the parse itself bound something.
    assert_eq!(
        gpu.procs[&pid].vases[&(GpuId::ZERO, A_PDB)]
            .table
            .iter()
            .map(|(va, _, _)| va)
            .collect::<Vec<_>>(),
        vec![pb_va(0x5000_0000).0],
        "sema releases do not bind; only the ring's own mapping is in the table"
    );
}

// ---------------------------------------------------------------------------------
// Fuzz: an arbitrary pushbuffer byte stream — hostile methods, truncated rings, bogus
// sema/dst addresses — every path is either a decoded fact or a loud fault, NEVER a
// panic, NEVER a silent guess (boundary-1 posture).
// ---------------------------------------------------------------------------------

mod fuzz {
    use kayfabe_arch::ids::GpuId;
    use kayfabe_fwd::parse_pushbuffer;
    use kayfabe_vmm::Vmm;
    use proptest::collection::vec;
    use proptest::prelude::*;

    /// Any GPFIFO ring bytes + any method-region bytes: the parser returns a
    /// `Result`, never panics, and any bound table entry is consistent (resolve
    /// of a bound VA succeeds — no torn state).
    ///
    /// Gated on `KAYFABE_SLOW=1` — measured 73 s debug, the single largest test
    /// in the workspace (the whole rest of the suite is ~20 s). The cost is not
    /// the parser: hostile GPFIFO entries make it read ~1 MB ranges through
    /// `MockVmm`'s byte-per-node `BTreeMap` RAM, ×128 cases. The nightly `slow`
    /// CI job runs it; the parser's boundary is still covered every push by the
    /// deterministic tests above and the libfuzzer harness (`fuzz/`) stays the
    /// deep-fuzz tier.
    #[test]
    fn arbitrary_pushbuffer_bytes_never_panic() {
        kayfabe_tests::skip_slow!("arbitrary_pushbuffer_bytes_never_panic");
        proptest!(
            ProptestConfig::with_cases(128),
            |(ring in vec(any::<u8>(), 0..80), region in vec(any::<u8>(), 0..256))| {
                let (mut gpu, mut vmm) = super::one_proc_gpu();
                let pid = *gpu.spine.by_pdb.get(&(GpuId::ZERO, super::A_PDB)).unwrap();
                let cid = *gpu.procs[&pid].chan_ids.values().next().unwrap();
                // Back the region GPFIFO entries might point at (0x5000_0000 window).
                vmm.gpa_write(0x5000_0000, &region).unwrap();
                // ★ One WIDE binding over the low 4 GiB of the VAS, at a non-zero bias.
                // Without it every arbitrary GPFIFO entry would refuse as an address-table
                // MISS and the fuzz would never reach the guest-memory read at all — the
                // read arm would go quietly vacant, which is the shape a coverage
                // regression hides in. With it, a VA under 4 GiB translates and reads, and
                // anything above it still misses, so BOTH arms are reachable.
                kayfabe_tests::bind_ring_at(
                    &mut gpu,
                    pid,
                    cid,
                    kayfabe_arch::ids::GpuVa(0),
                    0x1_0000_0000,
                    1 << 32,
                );

                // Never panics — a Result at worst a loud fault.
                let _ = parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring);

                // Whatever got bound resolves cleanly (no torn/partial binding).
                let vas = &gpu.procs[&pid].vases[&(GpuId::ZERO, super::A_PDB)];
                for (va, _len, b) in vas.table.iter() {
                    prop_assert_eq!(
                        vas.table.resolve(super::A_PDB, kayfabe_arch::ids::GpuVa(va)).map(|(x, _)| x.phys),
                        Ok(b.phys),
                        "a bound VA must resolve to its own binding (no torn state)"
                    );
                }
            }
        );
    }
}

/// ★ **boundary-1's TOTAL budget binds on the LAST range too** — the clamp is
/// `remaining budget`, not `this range's own length`.
///
/// `read_pushbuffer` walks a guest-controlled GPFIFO and reads each range's method
/// bytes. Two caps make that bounded: `MAX_PUSH_RANGE_BYTES` per range, and
/// `MAX_PUSH_TOTAL_BYTES` across the whole ring. The per-range cap is trivially
/// exercised by any oversized range; the TOTAL cap is only *visible* when a range
/// straddles the budget's edge — i.e. when what is left is smaller than both the
/// range and the per-range cap. Every other test in this file uses ranges far under
/// budget, so the `MAX_PUSH_TOTAL_BYTES - total` term was never observed at all, and
/// the first L1 mutation campaign duly found `lib.rs:1606:39 replace - with +`
/// surviving: with `+`, the third clamp becomes vacuous and the straddling range is
/// read WHOLE, past the budget the boundary exists to enforce.
///
/// The ring below is sized so the budget edge lands strictly INSIDE a range:
/// 768 KiB × 10 = 7.5 MiB, leaving 512 KiB of an 8 MiB budget for range 11 — which
/// wants 768 KiB. Content is unwritten guest RAM (reads as zero), and the mock arch
/// decodes a zero header as a zero-argument method, so one decoded method == one
/// 32-bit word read: the method count IS the byte count, and the assertion is exact
/// rather than a bound.
#[test]
fn total_read_budget_clamps_a_straddling_range_to_what_is_left() {
    /// Per-range length, deliberately not a divisor of the total budget.
    const RANGE: u64 = 768 << 10;
    /// `kayfabe-fwd`'s `MAX_PUSH_TOTAL_BYTES` (private; mirrored here so a change to
    /// it fails this test loudly instead of silently weakening the assertion).
    const TOTAL_BUDGET: u64 = 8 << 20;

    let (mut gpu, mut vmm) = one_proc_gpu();
    let pid = *gpu.spine.by_pdb.get(&(GpuId::ZERO, A_PDB)).unwrap();
    let cid = *gpu.procs[&pid].chan_ids.values().next().unwrap();
    // 12 ranges — enough that the budget is spent before the ring is, so the "ranges
    // past the budget are skipped" arm runs too. Each range sits in its own 4 GiB
    // window so no two ranges can alias.
    let mut ring = Vec::new();
    for i in 0..12u64 {
        // ★ The entry names the VA; the binding put there by `bind_ring` is what maps it
        // to the GPA the bytes (do not) live at. A regression to reading the entry's
        // number raw lands 0x7000_0000_0000 away from any declared RAM.
        ring.extend_from_slice(&pb_va(0x8000_0000 + i * 0x1_0000_0000).0.to_le_bytes());
        ring.extend_from_slice(&RANGE.to_le_bytes());
    }
    bind_ring(&mut gpu, pid, cid, &ring);

    let ranges = kayfabe_fwd::pushbuffer_ranges(&gpu.spine, &ring);
    let methods =
        kayfabe_fwd::read_pushbuffer(&gpu.spine, &gpu.procs[&pid], cid, &mut vmm, &ranges)
            .expect("unwritten guest RAM reads as zeros, never a fault");

    // 10 full ranges (7.5 MiB) + a final 512 KiB slice == exactly the budget. With the
    // remaining-budget term removed, the 11th range contributes its whole 768 KiB and
    // this count is 65_536 words higher.
    assert_eq!(
        methods.len() as u64,
        TOTAL_BUDGET / 4,
        "the ring's total method bytes must be clamped to EXACTLY the budget — the \
         straddling range is cut to what is left, not read whole"
    );
    assert!(
        methods.iter().all(|(h, args)| *h == 0 && args.is_empty()),
        "sanity: every decoded method came from unwritten (zero) guest RAM"
    );
}

/// ★ A `SemRelease`'s **completion identity mixes BOTH of its operands**, so two
/// distinct releases can never land on one completion.
///
/// `apply_pushbuffer` observes `OsEventRef(addr ^ payload)`. Every existing test
/// asserts only that *a* completion was observed (`has_outstanding`), never *which* —
/// so the mix was free to degrade, and the first ICE-free L1 campaign found both
/// `lib.rs:1676:59 replace ^ with |` and `… with &` surviving. Either turns the mix
/// into a lossy fold, and a lossy fold is a completion COLLISION: two guest fences
/// that must be distinguishable become one queue entry, which is a lost completion —
/// the F2 species, arriving through the untrusted pushbuffer.
///
/// The three releases below are chosen so the property is about collisions, not about
/// a magic constant: under `|` the first two fold to the same value, under `&` the
/// first and third do, and under `^` all three are distinct. The exact values are
/// asserted too, so a *different* lossy mix cannot pass by accident.
#[test]
fn sem_release_completion_identities_mix_both_operands_and_never_collide() {
    // (addr, payload) triples: ^ ⇒ {0x1000, 0x1100, 0x0000}, all distinct;
    //                          | ⇒ {0x1100, 0x1100, …}  — first two COLLIDE;
    //                          & ⇒ {0x0100, 0x0000, 0x0100} — first and third COLLIDE.
    const RELEASES: [(u64, u64); 3] = [(0x1100, 0x0100), (0x1000, 0x0100), (0x0100, 0x0100)];

    let (mut gpu, mut vmm) = one_proc_gpu();
    let pid = *gpu.spine.by_pdb.get(&(GpuId::ZERO, A_PDB)).expect("routed");
    let cid = *gpu.procs[&pid]
        .chan_ids
        .values()
        .next()
        .expect("the scenario's channel");
    let methods: Vec<(u32, Vec<u32>)> = RELEASES
        .iter()
        .map(|&(addr, payload)| MockPushbuffer::sem_release(addr, payload))
        .collect();
    let ring = script_pushbuffer(&mut gpu, &mut vmm, pid, cid, 0x4_0000_0000, &methods);
    let out =
        parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring).expect("the scripted ring parses");
    assert_eq!(out.sem_releases.len(), 3, "all three releases were decoded");

    let observed = gpu
        .procs
        .get_mut(&pid)
        .expect("live")
        .completion
        .compose_into(BatchId(1));
    let expected: Vec<OsEventRef> = RELEASES
        .iter()
        .map(|&(addr, payload)| OsEventRef(addr ^ payload))
        .collect();
    assert_eq!(
        observed, expected,
        "each release's completion identity mixes addr AND payload, in order"
    );
    let distinct: std::collections::BTreeSet<_> = observed.iter().collect();
    assert_eq!(
        distinct.len(),
        3,
        "…and three distinct releases stay three distinct completions — a lossy \
         fold collides them and loses one"
    );
}

// =================================================================================
// ★★★ #102/#13 — THE OPERAND SPLIT: the discriminator is what the operands CARRY,
// read off the RESOLVED PHYSICAL destination, never the operand's FORM.
// =================================================================================

/// Give `pdb`'s VAS the framebuffer alias the guest kernel's copy-engine utility builds:
/// a virtual range that maps straight onto physical framebuffer, at the largest page
/// size, so page-table writes issued through it are **virtual-destination** copies.
///
/// This is the C's `bUseVasForCeCopy` / `channel_utils.c` shape:
/// *"identity-maps the WHOLE FB heap into its own VAS at the largest page size — 512 MiB
/// — and then issues its page-table writes as VIRTUAL-dst CE copies (dstAddr = fbPhys +
/// fbAliasVA - startFbOffset = fbPhys)"* (`C: nvkvm_gpu_emul.c:4936-4952`).
fn declare_fb_alias(gpu: &mut Gpu, pid: kayfabe_core::ProcId, pdb: Pdb, at: GpuVa, phys: u64) {
    gpu.procs
        .get_mut(&pid)
        .expect("live proc")
        .vases
        .get_mut(&(GpuId::ZERO, pdb))
        .expect("the Vas")
        .table
        .bind(
            pdb,
            at,
            0x2000_0000, // 512 MiB, the alias's page size
            kayfabe_mmu::Binding {
                phys,
                aperture: kayfabe_arch::Aperture::Vidmem,
                host: None,
            },
        )
        .expect("the alias binds");
}

/// ★★★ **THE INVERTED GATE, fixed.** A page-table write issued as a **VIRTUAL**
/// destination through the framebuffer alias is captured — and the old gate
/// (`if !dst_is_virtual`) skipped exactly this, which is what #13 is.
///
/// The C hooks on the **resolved physical** destination regardless of the form the
/// operand took (`C: :6353` for the fill path, `:6437` for the copy path — both take the
/// post-resolve address). Our port gated on the form and therefore excluded the only
/// shape the guest's page-table writer actually uses.
///
/// **Non-vacuity, and it is the whole test:** the same page is written twice, once as a
/// physical destination and once as a virtual one through the alias, and the two must
/// produce the SAME capture. A test that only checked the virtual case could pass on a
/// parser that captured everything.
#[test]
fn a_virtual_destination_through_the_fb_alias_is_captured_as_a_pt_write() {
    let (mut gpu, mut vmm) = one_proc_gpu();
    let pid = *gpu.spine.by_pdb.get(&(GpuId::ZERO, A_PDB)).unwrap();
    let cid = *gpu.procs[&pid].chan_ids.values().next().unwrap();

    let root: u64 = A_PDB.0 & !0xfff;
    // The alias covers physical 0 upward, so `ALIAS_BASE + p` resolves to physical `p`.
    // ★ Deliberately under 2^46. This is a GPU VA and never becomes a KVM memslot, but
    // the house rule after task #105 is that no hardcoded address in a test needs 47+
    // bits: the memslot ceiling is the HOST CPU's physical-address width (46 on the
    // Xeon E5-2697A v4 bench box, 48 on the AMD dev box), and a test that encodes the
    // wider assumption passes locally and fails on hardware.
    const ALIAS_BASE: GpuVa = GpuVa(0x3000_0000_0000);
    declare_fb_alias(&mut gpu, pid, A_PDB, ALIAS_BASE, 0);

    // (a) the PHYSICAL form.
    let ring = script_pushbuffer(
        &mut gpu,
        &mut vmm,
        pid,
        cid,
        0x5000_0000,
        &[MockPushbuffer::ce_launch_dma(root + 0x40, 0x40, false)],
    );
    let phys_form = parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring).expect("parses");

    // (b) the VIRTUAL form, through the alias, naming the SAME physical page.
    let ring = script_pushbuffer(
        &mut gpu,
        &mut vmm,
        pid,
        cid,
        0x5001_0000,
        &[MockPushbuffer::ce_launch_dma(
            ALIAS_BASE.0 + root + 0x40,
            0x40,
            true,
        )],
    );
    let virt_form = parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring).expect("parses");

    assert_eq!(
        phys_form.pt_writes, virt_form.pt_writes,
        "★ the SAME page-table write, in two operand forms, must capture identically — \
         the old `if !dst_is_virtual` gate dropped the second one entirely, and the \
         second one is the only form the guest's page-table writer uses"
    );
    assert_eq!(virt_form.pt_writes.len(), 1);
    assert_eq!(
        (
            virt_form.pt_writes[0].page,
            virt_form.pt_writes[0].owner_pdb
        ),
        (root, A_PDB)
    );
    assert_eq!(
        (phys_form.data_copies, virt_form.data_copies),
        (0, 0),
        "neither was misclassified as forwardable data"
    );

    // ★ Non-vacuity in the other direction: a VIRTUAL destination through the SAME alias
    // that lands on a page nothing owns is data — forwarded. So the capture above is
    // about the destination, not about the alias.
    let ring = script_pushbuffer(
        &mut gpu,
        &mut vmm,
        pid,
        cid,
        0x5002_0000,
        &[MockPushbuffer::ce_launch_dma(
            ALIAS_BASE.0 + 0x1_2340_0000,
            0x1000,
            true,
        )],
    );
    let out = parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring).expect("parses");
    assert_eq!((out.pt_writes.len(), out.data_copies), (0, 1));
}

/// ★★★ **The write and the owner are DIFFERENT procs**, which is why the page-table
/// ownership index is device-global and why the latch is a separate pass.
///
/// The guest kernel is what writes a user process's page tables. Here proc B's channel,
/// through B's own framebuffer alias, writes the page table belonging to proc **A** — and
/// the capture must be attributed to A, latched into A's `Vas`, and must NOT appear in
/// B's. Attributing it to the writer is the C's `va_map[]` aliasing class
/// (`eight_blockers_resolved.md` §2: keyed on client, *"not dup-edge aware"*).
#[test]
fn a_pt_write_is_attributed_to_the_vas_that_owns_the_page_not_to_the_writer() {
    let (mut gpu, _vmm, _rec) = two_proc_gpu();
    let mut vmm = MockVmm::new();
    let pid_a = *gpu.spine.by_pdb.get(&(GpuId::ZERO, A_PDB)).unwrap();
    let pid_b = *gpu.spine.by_pdb.get(&(GpuId::ZERO, B_PDB)).unwrap();
    let cid_b = *gpu.procs[&pid_b].chan_ids.values().next().unwrap();

    // B's channel has the FB alias; A's page-directory root is what it writes.
    // Under 2^46, as above (task #105).
    const ALIAS_BASE: GpuVa = GpuVa(0x3000_0000_0000);
    declare_fb_alias(&mut gpu, pid_b, B_PDB, ALIAS_BASE, 0);
    let a_root: u64 = A_PDB.0 & !0xfff;

    let ring = script_pushbuffer(
        &mut gpu,
        &mut vmm,
        pid_b,
        cid_b,
        0x5000_0000,
        &[MockPushbuffer::ce_launch_dma(
            ALIAS_BASE.0 + a_root + 0x80,
            0x40,
            true,
        )],
    );
    let out =
        parse_pushbuffer(&mut gpu, &mut vmm, pid_b, cid_b, &ring).expect("B's channel parses");

    assert_eq!(out.pt_writes.len(), 1, "captured");
    assert_eq!(
        (out.pt_writes[0].owner, out.pt_writes[0].owner_pdb),
        (pid_a, A_PDB),
        "★ attributed to A, whose page table it is — NOT to B, who wrote it"
    );
    assert!(
        gpu.procs[&pid_a].vases[&(GpuId::ZERO, A_PDB)]
            .pt_pages
            .contains(&a_root),
        "latched into A's Vas"
    );
    assert!(
        gpu.procs[&pid_b].vases[&(GpuId::ZERO, B_PDB)]
            .pt_pages
            .is_empty(),
        "…and NOT into the writer's — an alias in B's VAS confers no ownership"
    );
}
