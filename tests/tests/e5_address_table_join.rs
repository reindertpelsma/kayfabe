//! # ★★★ E5 — **the address table populated from the guest's OWN bindings, so the CE
//! operands resolve** (`docs/design/execution_plane_increments.md`, the E5 row).
//!
//! > **acceptance:** a guest VA that *was* bound resolves; the copy's operands are found.
//! > **★ control:** a VA that was **never bound** must FAULT.
//!
//! # The two populate sources, named as THIS WIRE has them
//!
//! ⊘ **Not** *"bind-time RPC bindings"*. `MAP_MEMORY_DMA`/`UNMAP_MEMORY_DMA` are HAL stubs
//! on every GSP-client part, so `RmEvent::MapMemoryDma` is never constructed from the wire
//! (`kayfabe_rmrpc` crate docs §2.7; `decode_map_memory_dma` has no caller outside tests),
//! and `Spine::sync_rpc_mappings` runs over an **empty set** — a live code path with no
//! live input, which is exactly how the wrong name survived in two places. The two real
//! sources are:
//!
//! | source | how it gets here | where it binds |
//! |---|---|---|
//! | **`GPU_PROMOTE_CTX`** (`#93`) | an RM control the guest emits | `kayfabe_core::promote::apply_promote_ctx` |
//! | **the observed CE page-table write** (`#102` C2/C3) | `parse_pushbuffer` witnesses the copy → `pt_writes` → `latch_pt_writes` → `plan/run/commit_pt_decode` | `kayfabe_mmu::reach::apply_settlement` |
//!
//! **This file is the join.** Each source has its own suite (`promote_ctx.rs`,
//! `pt_decode.rs`) and each proves its own half; neither proves that the halves land in
//! **one** table that a copy-engine command then resolves against. That composition is
//! E5's whole content, and a composition is precisely what per-source tests cannot assert.
//!
//! # ★★★ THE JOIN WAS ASYMMETRIC, AND E8 CLOSED IT — but read what each test asserts
//!
//! Writing this file measured the asymmetry. `GPU_PROMOTE_CTX` populated the table end to
//! end; the observed CE page-table write **witnessed only a root page directory**, because
//! `Spine::pt_page_owner` read `Spine::pt_roots` alone, and that map is root-only by
//! construction. A guest CE write into a *leaf* table was therefore classified as ordinary
//! data, forwarded, never witnessed, and every leaf under it stayed `unwitnessed` — a MISS,
//! a FAULT.
//!
//! **E8 built the stage `Spine::pt_roots`'s doc called "the next stage".** The decode
//! reports the pages it learned, a fourth PUBLISH phase installs them in
//! `Spine::pt_learned` at rank 0, and the guest's *next* write into one of them is
//! classified. [`a_ce_write_into_a_learned_leaf_table_is_witnessed_and_binds_its_leaf`] is
//! the closed join, and it carries its own non-vacuity: the identical ring parsed *before*
//! the publish must yield zero page-table writes.
//!
//! ## ⚠ And a lesson about the absence test, recorded because it is this file's own
//!
//! [`the_ce_pt_write_source_can_witness_only_a_root_page_today`] was written to go **red**
//! the day that stage landed. **It did not.** Its assertion is
//! `(bound, unwitnessed) == (0, 1)` over a *single* pass from the root — and that is still
//! exactly right after E8, because one pass learns the leaf's level without the guest
//! having written the leaf yet. What E8 falsified was the test's *prose* (*"only a ROOT
//! can be [witnessed]"*), which no assertion ever checked.
//!
//! ⊘ **A test whose message is stronger than its assertion cannot detect the thing its
//! message is about.** The absence is now covered by the *presence* test above instead,
//! which is the direction that actually bites; this one is kept and re-scoped to the pass
//! shape it really measures.
//!
//! # ⚠ What "must FAULT" means here, stated exactly, because it is not uniform
//!
//! MISS = FAULT is the address plane's law, and the places it is enforced are named
//! rather than assumed:
//!
//! - [`kayfabe_mmu::AddressTable::resolve`] — a miss is `AddressFault::Miss`, always.
//! - `kayfabe_fwd::read_pushbuffer` — a GPFIFO entry naming an unbound VA is refused
//!   before a guest byte is fetched (§8.2.3).
//! - `kayfabe_fwd::gate_working_set` — a doorbell whose working set is not host-published
//!   is refused (the #14 ring gate).
//!
//! ⊘ And **one place it is deliberately not**, which this file states rather than hides:
//! `classify_ce` does **not** fault on an unresolvable CE *data* destination. The
//! overwhelming majority of virtual-destination copies are ordinary writes into a user
//! address space this port never had to model, and faulting them would turn *"we are not
//! intercepting this"* into *"the guest did something wrong"*. What must never happen is
//! *guessing* one into a page-table write, and an unresolved destination cannot be
//! classified as one — it has no physical address to look up. The safety net for that arm
//! is the ring gate above, not a fault here.
//!
//! Invariant/contract tests (decision #15), mock-driven, **GPU-free**.

#![allow(clippy::unusual_byte_groupings)]

use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb};
use kayfabe_arch::{Aperture, Arch, CeWork, GmmuFmt, PhysTarget};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::promote::{CtxPromotion, PromoteDeclined, PromotedRange};
use kayfabe_fwd::{
    IsolateFb, PT_DECODE_BUDGET, Representability, commit_pt_decode, gate_working_set,
    partition_ce, plan_pt_decode, publish_backing, run_pt_decode,
};
use kayfabe_isolate::{
    CeExecutor, CeSource, CeSubCopy, HostHandle, IsolateFactory, IsolateId, VerbPlan, VerbReply,
    Worker,
};
use kayfabe_mmu::AddressFault;
use kayfabe_mocks::{
    MOCK_DUAL_LEVEL, MockArch, MockGmmuFmt, MockIsolateFactory, MockPushbuffer, MockVmm,
    SharedRecorder, mock_classes as mc,
};
use kayfabe_tests::{Guarded, Scenario, bind_ring, identical_handles, script_ring_via};

const GPU: GpuId = GpuId::ZERO;

/// The proc's address space. A PDB **is** the physical address of its root page
/// directory, which is why the root is a declared fact and not a discovered one.
const A_PDB: Pdb = Pdb(0x1001_0000);
const ROOT: u64 = A_PDB.0;

/// The fabricated aperture the isolate maps — where the guest's page tables live, because
/// every byte in it was written by us (`#102` §12.2).
const FAB_BASE: u64 = 0x1000_0000;
const FAB_LEN: u64 = 0x0400_0000;
/// Staging inside the aperture, so a page write is a real `CeExecutor::Ours` sub-copy.
const STAGE: u64 = 0x1300_0000;

/// The intermediate directories and the leaf table of the one path this file builds.
const PD_L1: u64 = 0x1002_0000;
const PD_L2: u64 = 0x1003_0000;
const PD_DUAL: u64 = 0x1004_0000;
const PT_SMALL: u64 = 0x1005_0000;

/// Which small-page slot the leaf table fills, and what it maps to.
const LEAF_SLOT: usize = 9;
const LEAF_PHYS: u64 = 0xF000_0000;

/// The GR context buffer `GPU_PROMOTE_CTX` declares — VA, length, backing.
const CTX_VA: GpuVa = GpuVa(0x2_0010_0000);
const CTX_LEN: u64 = 0x2_0000;
const CTX_PHYS: u64 = 0x8_0000_0000;

/// ★ Thin wrapper over [`partition_ce`] defaulting both phys-mode targets to the register
/// reset (`PhysTarget::LocalFb`) — every operand in this file is virtual, so the target is
/// carried-but-unread. E10b's residency-signal decode is exercised in `ce_representability_split`.
#[allow(clippy::too_many_arguments)]
fn pc(
    dst_table: Option<&kayfabe_mmu::AddressTable>,
    dst: GpuVa,
    dst_is_virtual: bool,
    src: GpuVa,
    src_is_virtual: bool,
    len: u64,
    work: CeWork,
) -> Result<Vec<kayfabe_fwd::CeSpan>, kayfabe_fwd::FwdFault> {
    let mut ops = kayfabe_fwd::TableOperands::new(dst_table, Some(kayfabe_arch::ids::Pdb(0)));
    partition_ce(
        &mut ops,
        dst,
        dst_is_virtual,
        PhysTarget::LocalFb,
        src,
        src_is_virtual,
        PhysTarget::LocalFb,
        len,
        work,
    )
}

/// A VA nothing ever binds. **The control.** Deliberately far from every range above and
/// from the pushbuffer's own mapping, so "it faulted" cannot be an accident of adjacency.
const NEVER_BOUND: GpuVa = GpuVa(0x6_0000_0000);

/// The handles the scripted process uses.
const CLIENT: HClient = HClient(0xAA);

// =====================================================================================
// Scaffolding — the same shapes `pt_decode.rs` uses, because they model the same thing
// =====================================================================================

/// One table page's bytes: `entries` slots of `width` bytes, all zero except `set`.
fn image(width: usize, entries: usize, set: &[(usize, u128)]) -> Vec<u8> {
    let mut v = vec![0u8; entries * width];
    for &(i, e) in set {
        let at = i * width;
        v[at..at + width].copy_from_slice(&e.to_le_bytes()[..width]);
    }
    v
}

/// A page at `level`, filled from `set`.
fn page_at(fmt: &dyn GmmuFmt, level: u8, set: &[(usize, u128)]) -> Vec<u8> {
    let g = fmt.level_shift(level).expect("a level the regime has");
    image(usize::from(fmt.entry_size(level)), g.entries as usize, set)
}

fn leaf(phys: u64) -> u128 {
    MockGmmuFmt::encode_leaf(phys, false)
}
fn pde(next: u64) -> u128 {
    MockGmmuFmt::encode_pde(next, false, false)
}

/// The level of the small-page leaf table.
fn small_leaf_level() -> u8 {
    MOCK_DUAL_LEVEL + 1
}

/// Allocate a host VAS on `worker` (the mock's `Publish` chain is the only minting path).
fn fresh_host_vas(worker: &mut Worker) -> HostHandle {
    match worker
        .execute(&VerbPlan::Publish {
            host_vas: None,
            len: 0x1000,
            at: GpuVa(0x4000_0000),
        })
        .expect("a host VAS")
    {
        VerbReply::Published { host_vas, .. } => host_vas.expect("freshly allocated"),
        other => panic!("unexpected reply {other:?}"),
    }
}

/// Put `bytes` at fabricated address `phys` the way production does: stage them, then
/// have the **isolate** perform a `CeExecutor::Ours` sub-copy into fabricated space. The
/// read side and the write side must be the same memory, or the decode below is reading a
/// fixture rather than the guest's page table.
fn write_fabricated(
    worker: &mut Worker,
    rec: &SharedRecorder,
    vas: HostHandle,
    phys: u64,
    bytes: &[u8],
) {
    rec.lock().expect("recorder").ce_seed(STAGE, bytes);
    worker
        .execute(&VerbPlan::CeSplit {
            vas,
            subs: vec![CeSubCopy {
                dst: phys,
                src: CeSource::Address(STAGE),
                len: bytes.len() as u64,
                by: CeExecutor::Ours,
            }],
        })
        .expect("an unrepresentable copy is ours to perform");
}

/// A device with one compute proc, plus a standalone isolate whose aperture is declared.
fn fixture() -> (
    Guarded<Gpu>,
    MockVmm,
    MockIsolateFactory,
    SharedRecorder,
    kayfabe_core::ProcId,
    kayfabe_core::ChanId,
) {
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu =
        Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa).expect("the device realizes");
    let mut s = Scenario::new();
    s.compute_process(CLIENT, A_PDB, identical_handles(0x10, 0x11));
    for ev in s.events {
        gpu.apply(ev).expect("applies");
    }
    let pid = *gpu.spine.by_pdb.get(&(GPU, A_PDB)).expect("the VAS routed");
    let cid = *gpu.procs[&pid]
        .chan_ids
        .values()
        .next()
        .expect("the scenario's channel");
    let (fb_factory, fb_rec) = {
        let (f, r) = MockIsolateFactory::new();
        r.lock().expect("recorder").fb_declare(FAB_BASE, FAB_LEN);
        (f, r)
    };
    (
        Guarded::new("e5_address_table_join", gpu, rec),
        MockVmm::new(),
        fb_factory,
        fb_rec,
        pid,
        cid,
    )
}

/// The VA the leaf table's one entry describes, derived from the geometry rather than
/// written down — a hardcoded number here would be a fact about arithmetic, not about the
/// walk.
fn leaf_va(fmt: &dyn GmmuFmt) -> GpuVa {
    let g = fmt.level_shift(small_leaf_level()).expect("small");
    GpuVa((LEAF_SLOT as u64) << g.shift)
}

/// **SOURCE 1** — the guest promotes a GR context buffer. One control, one range.
fn promote_ctx_buffer(gpu: &mut Gpu) {
    let h = identical_handles(0x10, 0x11);
    let join = gpu
        .promote_ctx(&CtxPromotion {
            client: CLIENT,
            chan_client: CLIENT,
            object: h.gr_channel,
            ranges: vec![PromotedRange {
                va: CTX_VA,
                len: CTX_LEN,
                phys: CTX_PHYS,
                aperture: Aperture::Vidmem,
                buffer_id: 0,
            }],
            declined: PromoteDeclined::default(),
        })
        .expect("the promotion resolves to this proc's address space");
    assert_eq!(
        (join.bound, join.already, join.route.pdb),
        (1, 0, A_PDB),
        "the promotion bound its one range into THIS address space"
    );
}

/// **SOURCE 2** — the guest's copy engine writes its page tables, we witness the writes
/// through the pushbuffer parser, and the decode pass turns the witnessed pages into
/// bindings.
///
/// ★ The witnessing runs through `parse_pushbuffer` rather than by poking `pt_pages`,
/// because *"the observed CE page-table write"* is a claim about the **observation**. A
/// test that inserts the page by hand asserts the decoder works and says nothing about
/// whether anything would ever have seen the write.
fn witness_and_decode_page_tables(
    gpu: &mut Guarded<Gpu>,
    vmm: &mut MockVmm,
    worker: &mut Worker,
    fb_rec: &SharedRecorder,
    pid: kayfabe_core::ProcId,
    cid: kayfabe_core::ChanId,
) -> kayfabe_fwd::PtDecodeOutcome {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let host_vas = fresh_host_vas(worker);

    // The guest's own build order: the leaf table first (nothing points at it yet), then
    // the chain down to it. Each page's bytes land in the fabricated aperture because WE
    // performed the copy — which is the premise the decode reads back through.
    for (phys, bytes) in [
        (
            PT_SMALL,
            page_at(fmt, small_leaf_level(), &[(LEAF_SLOT, leaf(LEAF_PHYS))]),
        ),
        (ROOT, page_at(fmt, 0, &[(0, pde(PD_L1))])),
        (PD_L1, page_at(fmt, 1, &[(0, pde(PD_L2))])),
        (PD_L2, page_at(fmt, 2, &[(0, pde(PD_DUAL))])),
        (
            PD_DUAL,
            page_at(fmt, MOCK_DUAL_LEVEL, &[(0, pde(PT_SMALL))]),
        ),
    ] {
        write_fabricated(worker, fb_rec, host_vas, phys, &bytes);
    }

    // ★★★ The WITNESS. A copy-engine command whose resolved physical destination is the
    // VAS's root page directory: `parse_pushbuffer` classifies it as a page-table write
    // and latches the page onto the owning `Vas`.
    let ring = script_ring_via(
        vmm,
        0x5000_0000,
        &[
            MockPushbuffer::set_object(mc::DMA_COPY),
            MockPushbuffer::ce_launch_dma(ROOT + 0x8, 0x40, false),
        ],
    );
    bind_ring(gpu, pid, cid, &ring);
    let out = kayfabe_fwd::parse_pushbuffer(gpu, vmm, pid, cid, &ring).expect("the ring parses");
    assert_eq!(
        out.pt_writes.len(),
        1,
        "★ the page-table write must be OBSERVED — with nothing witnessed there is no \
         second populate source and the rest of this file would be about promote alone"
    );
    assert_eq!(
        (out.pt_writes[0].page, out.pt_writes[0].owner_pdb),
        (ROOT, A_PDB),
        "…and attributed to the address space that OWNS the page"
    );

    // The decode pass, in the three phases R1 forces.
    let plan = plan_pt_decode(gpu.procs.get_mut(&pid).expect("live"));
    assert_eq!(
        plan.tasks.iter().map(|t| t.page.phys).collect::<Vec<_>>(),
        vec![ROOT],
        "the root's level is a DECLARED fact, so the descent starts there"
    );
    let results = {
        let mut fb = IsolateFb::new(worker);
        run_pt_decode(fmt, &mut fb, &plan.tasks, PT_DECODE_BUDGET)
    };
    let outcome = commit_pt_decode(fmt, gpu.procs.get_mut(&pid).expect("live"), &results);
    assert!(outcome.is_clean(), "{outcome:?}");
    assert_eq!(
        outcome.meta_learned, 4,
        "★ the chain BELOW the root was learned forward — four pages, one observed write, \
         no sweep. This is the half of source 2 that does work today"
    );
    outcome
}

// =====================================================================================
// The acceptance
// =====================================================================================

/// ★★★ **E5's acceptance.** The table is populated from the guest's own bindings, and a
/// copy-engine command's operands resolve against it.
///
/// The operands come from the **promotion** — which is a live source on this wire — and
/// the assertion is E5's row read literally: *a guest VA that was bound resolves; the
/// copy's operands are found*.
///
/// ⊘ The second source's reach is asserted **separately and by name**
/// ([`the_ce_pt_write_source_can_witness_only_a_root_page_today`]), because it is not the
/// same statement and running them together would let one carry the other.
#[test]
fn a_promoted_range_resolves_and_a_ce_copys_operands_are_found() {
    let (mut gpu, _vmm, _fb_factory, _fb_rec, pid, _cid) = fixture();

    // ---- non-vacuity: before the guest binds anything, the operand MISSES.
    assert_eq!(
        gpu.procs[&pid].vases[&(GPU, A_PDB)]
            .table
            .resolve(A_PDB, CTX_VA),
        Err(AddressFault::Miss {
            pdb: A_PDB,
            va: CTX_VA
        }),
        "★ so every resolution below is the populate source's doing, not the fixture's"
    );

    promote_ctx_buffer(&mut gpu);

    // ---- ACCEPTANCE (a): a guest VA that WAS bound resolves, at its own offset.
    let table = &gpu.procs[&pid].vases[&(GPU, A_PDB)].table;
    assert_eq!(
        table
            .resolve(A_PDB, GpuVa(CTX_VA.0 + 0x1000))
            .map(|(b, off)| (b.phys + off, b.aperture)),
        Ok((CTX_PHYS + 0x1000, Aperture::Vidmem)),
        "the promoted range resolves — and the offset is carried, not rounded to the base"
    );

    // ---- ACCEPTANCE (b): the copy's operands are FOUND, both ends.
    let spans = pc(
        Some(table),
        GpuVa(CTX_VA.0 + 0x1000),
        true,
        CTX_VA,
        true,
        0x1000,
        CeWork::Copy,
    )
    .expect("the request partitions");
    assert_eq!(
        spans
            .iter()
            .map(|s| (s.sub.len, s.dst_kind, s.src_kind))
            .collect::<Vec<_>>(),
        vec![(
            0x1000,
            Representability::Fabricated,
            Some(Representability::Fabricated)
        )],
        "★ BOTH operands are found — one span, and neither end is `Untracked`. \
         `Fabricated` rather than `HostBacked` is the correct answer for a range that is \
         DECLARED and not yet host-published; what E5 asks is that they are found at all"
    );
    assert_eq!(
        spans[0].sub.by,
        CeExecutor::Ours,
        "…and a fabricated operand is unrepresentable to a real engine, so the copy is \
         ours to perform (§12's ruling, not a property of who asked)"
    );
}

/// ★★ **ONE PASS FROM THE ROOT LEARNS THE SUBTREE AND BINDS NOTHING** — the witness gate
/// doing its job, and the starting state of the E8 test above.
///
/// ⚠ **RE-SCOPED, 2026-08-04.** This test was written to assert *"the observed CE
/// page-table write can witness ONLY A ROOT PAGE"* and to go **red** the day the next
/// stage landed. E8 landed and it stayed **green** — because its assertion is about a
/// *single* pass and that assertion is still true: pass 1 learns the leaf table's level
/// but the guest has not written the leaf yet, so nothing is witnessed below the root and
/// nothing binds. The claim E8 falsified lived only in the prose.
///
/// ⊘ **The lesson is kept rather than tidied away**: a test whose message is broader than
/// its assertion cannot detect the thing its message is about, and *"it will go red when
/// X lands"* is a prediction the test must actually encode. The presence test above is
/// what encodes it — it fails if the publish stops working, which is the direction that
/// bites. See `should_panic_matches_the_wrong_site` for the same species.
///
/// `[measured]` 2026-08-02 at rev `4e8960f`; still holding at E8. The chain it exercises:
///
/// 1. `classify_ce` produces a `PtWrite` only for a destination `Spine::pt_page_owner`
///    recognises — at this point in the sequence, only the root.
/// 2. `latch_pt_writes` is the only writer of `Vas::pt_pages`.
/// 3. `plan_pt_decode` drains `pt_pages` and is the only caller of `ReachShadow::witness`.
/// 4. `settle` binds a leaf only from a page that is reachable **and** witnessed.
///
/// ⇒ after pass 1, every leaf below the root is `unwitnessed`, which is a MISS, which is a
/// FAULT. **The bytes are decoded correctly and the metadata chain is learned; only the
/// binding is withheld** — and it is withheld until the guest is *seen* to write the leaf
/// table, which is what E8 made possible and the test above demonstrates.
///
/// ⊘ **Deliberately not worked around.** Hand-inserting the leaf page into `pt_pages` —
/// which `pt_decode.rs`'s own fixtures do, correctly, to test the decoder — would make
/// this file report a join the live path cannot perform
/// (`mock_fidelity_both_directions`).
#[test]
fn the_ce_pt_write_source_can_witness_only_a_root_page_today() {
    let (mut gpu, mut vmm, fb_factory, fb_rec, pid, cid) = fixture();
    let mut iso = fb_factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("a fresh pool");
    let arch = MockArch::new();
    let fmt = arch.mmu();

    let outcome =
        witness_and_decode_page_tables(&mut gpu, &mut vmm, &mut worker, &fb_rec, pid, cid);

    // ---- what DOES work: the write was witnessed, the tree was walked, the chain learned.
    assert_eq!(
        outcome.faults,
        vec![],
        "the walk itself is clean — this is not a decode failure"
    );
    // ---- and the wall: the leaf the tree describes is REACHABLE but not WITNESSED.
    assert_eq!(
        (outcome.bound, outcome.unwitnessed),
        (0, 1),
        "★★★ nothing bound, and the reason is named: the leaf's page was never witnessed, \
         because only a ROOT can be. `Spine::pt_roots` is root-only by construction"
    );
    let leaf_va = leaf_va(fmt);
    assert_eq!(
        gpu.procs[&pid].vases[&(GPU, A_PDB)]
            .table
            .resolve(A_PDB, leaf_va),
        Err(AddressFault::Miss {
            pdb: A_PDB,
            va: leaf_va
        }),
        "…so the leaf does not resolve. MISS = FAULT, and here the miss is OURS, not the \
         guest's — which is why it is banked as a wall rather than as a refusal"
    );
    // ---- the metadata IS there, which is what makes this a missing *witness* and not a
    // missing *decode*: everything the next stage needs has already been learned.
    assert!(
        kayfabe_fwd::pt_meta_of(&gpu.procs[&pid], GPU, A_PDB).contains_key(&PT_SMALL),
        "the leaf table's level and vabase were learned forward from the root's decode"
    );
}

/// ★★★ **E8 — THE JOIN CLOSED: a guest CE write into a LEAF page table is witnessed, and
/// the leaf under it BINDS.**
///
/// This is E5's acceptance read literally for source 2 — *a guest VA that was bound
/// resolves* — and it is the statement
/// [`the_ce_pt_write_source_can_witness_only_a_root_page_today`] could not make.
///
/// # ★ The sequence is TWO writes, and that is the whole content
///
/// One decode pass starting at the root learns the entire subtree's metadata (its levels
/// and vabases) but binds nothing, because the leaf table's page was never *witnessed* —
/// the guest's write into it was classified as ordinary data and forwarded. E8 publishes
/// what the pass learned into the device-global ownership index, so the guest's **next**
/// write into that page is recognised. Pass 2 then witnesses it and the leaf binds.
///
/// ⊘ **The non-vacuity half runs first and would fail on its own.** The identical ring is
/// parsed *before* the publish and must yield **zero** page-table writes. Without that,
/// a fixture in which the leaf happened to be recognised anyway would let this test pass
/// while E8 did nothing — which is exactly how the sibling test stayed green through a
/// change that was supposed to make it red.
///
/// ⊘ **The publish is called here explicitly, and that is deliberate.** This file's
/// fixture is a `Gpu`, so it can reach `Spine` directly; whether the *shell* actually
/// calls `publish_pt_pages` is a different claim and is asserted where
/// `SharedDevice::decode_pt_writes` is driven (`pt_decode.rs`,
/// `the_pass_runs_through_the_shell_in_both_lock_modes_with_the_blocking_phase_unlocked`).
/// Proving both here would let one carry the other.
#[test]
fn a_ce_write_into_a_learned_leaf_table_is_witnessed_and_binds_its_leaf() {
    let (mut gpu, mut vmm, fb_factory, fb_rec, pid, cid) = fixture();
    let mut iso = fb_factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("a fresh pool");
    let arch = MockArch::new();
    let fmt = arch.mmu();

    // ---- PASS 1: the guest writes the ROOT. The chain below it is learned, nothing binds.
    let first = witness_and_decode_page_tables(&mut gpu, &mut vmm, &mut worker, &fb_rec, pid, cid);
    assert_eq!(
        (first.bound, first.unwitnessed),
        (0, 1),
        "the starting state is the sibling test's ending state"
    );

    // The ring for the SECOND write — into the leaf TABLE this time, not the root.
    let ring2 = script_ring_via(
        &mut vmm,
        0x5100_0000,
        &[
            MockPushbuffer::set_object(mc::DMA_COPY),
            MockPushbuffer::ce_launch_dma(PT_SMALL + 0x8, 0x40, false),
        ],
    );
    bind_ring(&mut gpu, pid, cid, &ring2);

    // ---- ★ NON-VACUITY: before the publish, that write is NOT a page-table write.
    let before = kayfabe_fwd::parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring2)
        .expect("the ring parses");
    assert_eq!(
        before.pt_writes.len(),
        0,
        "★★★ the index knows roots only, so a write into a LEAF table is forwarded as \
         ordinary data — this is the wall E8 exists to remove, reproduced here so the \
         assertion below cannot be vacuous"
    );

    // ---- PUBLISH — E8's fourth phase, the one that needs rank 0.
    let pages: Vec<u64> = first.learned_pages.iter().map(|&(_, _, p)| p).collect();
    assert!(
        pages.contains(&PT_SMALL),
        "pass 1 learned the leaf table's level — that is what makes it publishable"
    );
    let (published, refused) = gpu.spine.publish_pt_pages(pid, GPU, A_PDB, pages);
    assert_eq!(
        (published, refused),
        (4, 0),
        "the four pages below the root; the root itself is declared and is not re-published"
    );

    // ---- and now the SAME ring, byte for byte, IS a page-table write.
    let after = kayfabe_fwd::parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring2)
        .expect("the ring parses");
    assert_eq!(
        (
            after.pt_writes.len(),
            after.pt_writes[0].page,
            after.pt_writes[0].owner_pdb
        ),
        (1, PT_SMALL, A_PDB),
        "★★★ classified, and attributed to the address space that OWNS the page — which \
         is not the channel's proc in general, and is why the index is device-global"
    );

    // ---- PASS 2: the leaf's page is witnessed now, so the leaf binds.
    let plan = plan_pt_decode(gpu.procs.get_mut(&pid).expect("live"));
    assert_eq!(
        plan.tasks.iter().map(|t| t.page.phys).collect::<Vec<_>>(),
        vec![PT_SMALL],
        "and the descent starts at the leaf TABLE, whose level pass 1 learned — a page \
         whose level is unknown would have been DEFERRED, not decoded"
    );
    let results = {
        let mut fb = IsolateFb::new(&mut worker);
        run_pt_decode(fmt, &mut fb, &plan.tasks, PT_DECODE_BUDGET)
    };
    let second = commit_pt_decode(fmt, gpu.procs.get_mut(&pid).expect("live"), &results);
    assert!(second.is_clean(), "{second:?}");
    assert_eq!(
        (second.bound, second.unwitnessed),
        (1, 0),
        "★★★ BOUND. The second populate source now reaches a leaf, which is the half of \
         E5 that was PARTIAL"
    );

    // ---- E5's acceptance, literally: the guest VA that was bound RESOLVES.
    let leaf_va = leaf_va(fmt);
    assert_eq!(
        gpu.procs[&pid].vases[&(GPU, A_PDB)]
            .table
            .resolve(A_PDB, leaf_va)
            .map(|(b, off)| (b.phys, off)),
        Ok((LEAF_PHYS, 0)),
        "…and it resolves to the physical address the guest's OWN page table entry names, \
         at offset zero — not to a nearest binding and not to a reverse-resolve"
    );
}

/// ★★★ **THE CONTROL: a VA that was never bound FAULTS, by name.**
///
/// Asserted at all three places the law is enforced, because "it faulted" is only
/// evidence if the *variant* is the one the plane means (`testing_doctrine.md` §2 rule 3).
/// The three are different planes and they must not report as each other.
#[test]
fn a_va_that_was_never_bound_faults_at_every_place_the_law_is_enforced() {
    let (mut gpu, mut vmm, fb_factory, fb_rec, pid, cid) = fixture();
    let mut iso = fb_factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("a fresh pool");

    promote_ctx_buffer(&mut gpu);
    witness_and_decode_page_tables(&mut gpu, &mut vmm, &mut worker, &fb_rec, pid, cid);

    // (1) THE ADDRESS TABLE — `mode2_address_table.md`: the table IS the guest's TLB.
    assert_eq!(
        gpu.procs[&pid].vases[&(GPU, A_PDB)]
            .table
            .resolve(A_PDB, NEVER_BOUND),
        Err(AddressFault::Miss {
            pdb: A_PDB,
            va: NEVER_BOUND
        }),
        "MISS = FAULT: never a reverse-resolve, never the nearest binding, never zero"
    );

    // (2) THE PUSHBUFFER READ (§8.2.3) — a GPFIFO entry naming it is refused before a
    //     single guest byte is fetched, even though the VMM would happily serve the
    //     number: `MockVmm::new()` declares the whole 64-bit space RAM.
    let mut ring = Vec::new();
    ring.extend_from_slice(&NEVER_BOUND.0.to_le_bytes());
    ring.extend_from_slice(&0x40u64.to_le_bytes());
    assert_eq!(
        kayfabe_fwd::parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring),
        Err(kayfabe_fwd::FwdFault::Address(AddressFault::Miss {
            pdb: A_PDB,
            va: NEVER_BOUND
        })),
        "★ the read TRANSLATES first — an unbound ring address is a fault, not the bytes \
         of whatever guest RAM shares its number"
    );

    // (3) THE #14 RING GATE — a doorbell whose working set is not host-published is
    //     refused. This is the safety net for the one arm that deliberately does NOT
    //     fault (an untracked CE data operand, see the module docs).
    assert!(
        gate_working_set(&gpu, pid, cid, &[NEVER_BOUND]).is_err(),
        "an unpublished working-set VA never reaches a doorbell"
    );

    // ★ NON-VACUITY, and it is the half that makes the three above mean anything: a VA
    // the guest DID bind and publish passes the same gate.
    // ★ The guest's eager unmap first: the address plane refuses to overwrite a live
    // binding ("unmap eager, map lazy"), and a promotion IS a live binding. Replacing it
    // silently is the `ALREADY-MAPPED` collision class the table exists to make loud.
    assert!(
        gpu.procs
            .get_mut(&pid)
            .expect("live")
            .vases
            .get_mut(&(GPU, A_PDB))
            .expect("the vas")
            .table
            .unbind(CTX_VA)
            .is_some(),
        "the declared binding was there to replace"
    );
    publish_backing(
        gpu.procs.get_mut(&pid).expect("live"),
        GPU,
        A_PDB,
        CTX_VA,
        CTX_LEN,
    )
    .expect("publishes");
    assert!(
        gate_working_set(&gpu, pid, cid, &[CTX_VA]).is_ok(),
        "…and a bound, published VA is served — so the refusals above are about the \
         binding and not about the gate refusing everything"
    );
}

/// ★★ **"…resolve in the isolate's host VAS"** — E5's row says *where*, and this is that
/// clause, which the acceptance above deliberately does not claim.
///
/// A declared binding is `Fabricated`: real to the guest, backed by nothing host-side, so
/// a real copy engine pointed at it resolves nothing. Publishing it moves it to
/// `HostBacked` — **at the identical address**, which is `#102`'s address-identity law and
/// the whole reason the guest's own number can be handed to the host engine.
#[test]
fn publishing_a_populated_range_makes_its_operand_host_representable_at_the_same_va() {
    let (mut gpu, _vmm, _fb_factory, _fb_rec, pid, _cid) = fixture();
    promote_ctx_buffer(&mut gpu);

    let before = {
        let t = &gpu.procs[&pid].vases[&(GPU, A_PDB)].table;
        pc(Some(t), CTX_VA, true, GpuVa(0), true, 0x1000, CeWork::Scrub).expect("partitions")
    };
    assert_eq!(
        before
            .iter()
            .map(|s| (s.dst_kind, s.sub.by))
            .collect::<Vec<_>>(),
        vec![(Representability::Fabricated, CeExecutor::Ours)],
        "declared but unpublished: no real engine can be pointed here"
    );

    // ★ The guest's eager unmap first: the address plane refuses to overwrite a live
    // binding ("unmap eager, map lazy"), and a promotion IS a live binding. Replacing it
    // silently is the `ALREADY-MAPPED` collision class the table exists to make loud.
    assert!(
        gpu.procs
            .get_mut(&pid)
            .expect("live")
            .vases
            .get_mut(&(GPU, A_PDB))
            .expect("the vas")
            .table
            .unbind(CTX_VA)
            .is_some(),
        "the declared binding was there to replace"
    );
    publish_backing(
        gpu.procs.get_mut(&pid).expect("live"),
        GPU,
        A_PDB,
        CTX_VA,
        CTX_LEN,
    )
    .expect("publishes");

    let after = {
        let t = &gpu.procs[&pid].vases[&(GPU, A_PDB)].table;
        pc(Some(t), CTX_VA, true, GpuVa(0), true, 0x1000, CeWork::Scrub).expect("partitions")
    };
    assert_eq!(
        after
            .iter()
            .map(|s| (s.dst_kind, s.sub.by))
            .collect::<Vec<_>>(),
        vec![(Representability::HostBacked, CeExecutor::HostCe)],
        "★ published ⇒ representable ⇒ the real engine, pointed at the GUEST's number"
    );
    assert_eq!(
        gpu.procs[&pid].vases[&(GPU, A_PDB)]
            .table
            .resolve(A_PDB, CTX_VA)
            .map(|(b, _)| b.host_va()),
        Ok(Some(CTX_VA.0)),
        "…and it is published AT THE SAME VA — `#102`'s identity law, which is what makes \
         the guest's own operand a legal host operand"
    );
    // ★ The whole-table audit, not just this binding: a law with only an entry check is
    // one bulk-populate path away from being a law about nothing.
    assert_eq!(
        gpu.procs[&pid].vases[&(GPU, A_PDB)]
            .table
            .audit_identity(A_PDB),
        Ok(()),
        "every host-backed binding in the joined table satisfies the identity law"
    );
}

/// ⊘ **The one place a miss is deliberately NOT a fault, pinned so it cannot drift into
/// one by accident** — and pinned as the *pair* it is, because the dangerous half is the
/// other direction.
///
/// An unresolvable virtual CE destination is ordinary user data this port never modelled:
/// forwarded, counted, never captured. What must be impossible is the inverse — a
/// destination we could not resolve being *guessed* into a page-table write, which would
/// hand the address plane a page the guest never wrote.
#[test]
fn an_unresolvable_ce_destination_is_forwarded_and_can_never_be_guessed_into_a_capture() {
    let (mut gpu, mut vmm, fb_factory, fb_rec, pid, cid) = fixture();
    let mut iso = fb_factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("a fresh pool");
    promote_ctx_buffer(&mut gpu);
    witness_and_decode_page_tables(&mut gpu, &mut vmm, &mut worker, &fb_rec, pid, cid);

    let ring = script_ring_via(
        &mut vmm,
        0x5100_0000,
        &[
            MockPushbuffer::set_object(mc::DMA_COPY),
            MockPushbuffer::ce_launch_dma(NEVER_BOUND.0, 0x1000, true),
        ],
    );
    bind_ring(&mut gpu, pid, cid, &ring);
    let out = kayfabe_fwd::parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring).expect("parses");
    assert_eq!(
        (out.pt_writes.len(), out.data_copies),
        (0, 1),
        "★ an unresolvable destination is DATA — forwarded, never guessed into a capture"
    );
    assert_eq!(
        out.ce_spans.iter().map(|s| s.dst_kind).collect::<Vec<_>>(),
        vec![Representability::Untracked],
        "…and it is named `Untracked`, which is a different statement from `Fabricated`: \
         the ring gate is what stops an untracked working set reaching a doorbell"
    );
}

/// ★ **Two address spaces, one proc: a range bound in ONE must not resolve in the other.**
///
/// The join is per-`Vas`, and the C artifact's #12 collision class is exactly what a
/// cross-VAS fallback produces. Neither populate source may leak across the key.
#[test]
fn a_range_bound_in_one_vas_does_not_resolve_in_another_on_the_same_proc() {
    use kayfabe_core::rmgraph::{AllocFacts, RmEvent};
    const B_PDB: Pdb = Pdb(0x1008_0000);
    const SECOND_VAS: HObject = HObject(0x5c00_0bbb);

    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu =
        Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa).expect("the device realizes");
    let h = identical_handles(0x10, 0x11);
    let mut s = Scenario::new();
    s.compute_process(CLIENT, A_PDB, h);
    s.push(RmEvent::Alloc {
        client: CLIENT,
        parent: h.device,
        handle: SECOND_VAS,
        class: mc::VASPACE,
        facts: AllocFacts::default(),
    });
    s.push(RmEvent::SetPageDir {
        client: CLIENT,
        vaspace: SECOND_VAS,
        pdb: B_PDB,
    });
    for ev in s.events {
        gpu.apply(ev).expect("applies");
    }
    let mut gpu = Guarded::new("e5_address_table_join::two_vases", gpu, rec);
    let pid = *gpu.spine.by_pdb.get(&(GPU, A_PDB)).expect("A routes");
    promote_ctx_buffer(&mut gpu);

    assert!(
        gpu.procs[&pid].vases[&(GPU, A_PDB)]
            .table
            .resolve(A_PDB, CTX_VA)
            .is_ok(),
        "the promotion landed in the address space its context object names"
    );
    assert_eq!(
        gpu.procs[&pid].vases[&(GPU, B_PDB)]
            .table
            .resolve(B_PDB, CTX_VA),
        Err(AddressFault::Miss {
            pdb: B_PDB,
            va: CTX_VA
        }),
        "★ …and NOWHERE else. A second address space on the same proc, at the identical \
         VA, misses — there is no cross-VAS fallback, which is the C's #12 collision class"
    );
}
