//! ★★★★★ **G1 + G2 + G3 — the CPU transport's page-table writes, end to end**
//! (`execution_plane_increments.md` §16.73.8).
//!
//! # The measurement this file is written against
//!
//! `[measured 2026-08-10, boot `w208_797a6bc_real`]` §16.73.5. The walling channel's ring
//! `0x420064000` resolves through a five-level tree, and the tracer named the writer of
//! **every level**:
//!
//! ```text
//! walk: L0@0x2efa9c000/byBAR2#39  =PDE→0x2efa9b000/Vidmem
//!       L1@0x2efa9b000/byBAR2#40  =PDE→0x2efa9a000/Vidmem
//!       L2@0x2efa9a000/byBAR2#41  =PDE@0x420000000→0x2efa80000/Vidmem
//!       L3@0x2efa80000/byBAR2#67  =PDE@0x420000000→0x2efa7f000/Vidmem/dual1of2
//!       L5@0x2efa7f000/byBAR2#68  =LEAF@0x420064000→0x237fe000/SysmemCoherent/sz0x1000
//! ```
//!
//! with the same boot's first-writer census reading `PRAMIN 21 / BAR1 0 / BAR2 50 /
//! **EXEC 0**`. ⇒ the transport is the **guest's own CPU** through a framebuffer window,
//! and `Vas::pt_pages` — the only thing that turns a witnessed write into a mapping — had
//! exactly one feed, the `CeOperands::PhysOperand` arm of a **CE pushbuffer parse**. So
//! everything that transport published stayed a miss, four rungs running.
//!
//! # What each family here is for
//!
//! 1. **G1** — the witness fires at the one statement that stamps `/byBAR2`, through
//!    **both** windows, deduped by page, and drained.
//! 2. **The attribution** — a page the index cannot name an owner for comes **back**, and
//!    a later round picks it up. This is the mechanism that makes a five-level tree
//!    reachable from a root that is the only declared page.
//! 3. **G2 + G3, joined** — the guest writes a tree the way a guest writes one (dword by
//!    dword through a window), and the leaf VA becomes **bound** in the core's address
//!    table, read out of the **device's** `FbStore` and not the isolate's.
//!
//! # ⊘ What this file does NOT establish
//!
//! - ⊘ **It is not a boot.** `only_live_boots_are_proof` stands. It is the same statement
//!   made against the same ports with no guest and no hardware.
//! - ⊘ It says nothing about `GP_GET` moving or a release semaphore retiring — R26's
//!   two-fact bar is a fact about a **host** channel and there is none here.
//! - ⊘ It does not test the `IsolateFb` source, which remains right for §12.2's fabricated
//!   aperture and is unchanged.

use kayfabe_abi::versions::BENCH_DRIVER;
use kayfabe_arch::ids::{GpuId, GpuVa, HClient, Pdb};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_device::plane::{NanoClock, RegPlane, SteppingClock};
use kayfabe_device::{FbWriter, SparseFb, abi, ga10x::GA106};
use kayfabe_mocks::MockIsolateFactory;
use kayfabe_rt::device::{LockMode, SharedDevice};
use kayfabe_tests::{Guarded, Scenario, identical_handles};

const GPU: GpuId = GpuId::ZERO;

// ───────────────────── the boot's own addresses, transcribed ─────────────────────

/// `L0` — the page-directory base, which **is** the address space's root page.
const ROOT: u64 = 0x2_EFA9_C000;
const L1: u64 = 0x2_EFA9_B000;
const L2: u64 = 0x2_EFA9_A000;
/// `L3` — the **dual** directory: 256 slots of 16 bytes.
const L3: u64 = 0x2_EFA8_0000;
/// `L5` — the **small-page** leaf table, the `dual1of2` half.
const L5: u64 = 0x2_EFA7_F000;
/// Every page-table page of the tree, root first — the order the guest wrote them in.
const TREE: [u64; 5] = [ROOT, L1, L2, L3, L5];

/// The walling channel's `gpFifoOffset`.
const RING_VA: u64 = 0x4_2006_4000;
/// What its leaf points at: a **system-memory** page, which is what a GPFIFO ring is.
const RING_PHYS: u64 = 0x237F_E000;

const PDB: Pdb = Pdb(ROOT);

// ───────────────────────────── VER2 entry encodings ─────────────────────────────
//
// ⊘ The same four encoders `tests/tests/bar2_translation.rs` uses, deliberately not a
// second set: that file's tree is differentialled against the guest's own `kbusVerifyBar2`
// and against the compiled GMMU oracle, so a private encoder here could drift from the
// format the port actually walks with while every assertion below still passed.

/// A page-directory entry naming a vidmem sub-table.
fn pde_vid(next: u64) -> u64 {
    ((next >> 12) << 8) | (1 << 1)
}

/// The **high** qword of a dual entry: the SMALL-page sub-table, in vidmem.
fn dual_small_vid(next: u64) -> u64 {
    ((next >> 12) << 8) | (1 << 1)
}

/// A page-table entry mapping a **system-memory** page — aperture 2, valid.
fn pte_sys(phys: u64) -> u64 {
    ((phys >> 12) << 8) | (2 << 1) | 1
}

// ───────────────────────────── the plane, and the guest's hands ─────────────────────

fn plane() -> RegPlane {
    let p = RegPlane::new(
        &GA106,
        abi::gsp_abi_for(BENCH_DRIVER).expect("the bench driver has a wire table"),
        Box::new(SteppingClock::new(1)) as Box<dyn NanoClock>,
    )
    .expect("GA106 is servable");
    p.set_fb(Box::new(SparseFb::new(GA106.fb_length)));
    p.set_mmu(Box::new(kayfabe_chips::Ga10xGmmu::new()));
    p
}

/// Point the BAR0 moving window at `phys`, exactly as `GPU_FLD_WR_DRF_NUM(_BASE)` does.
fn point_window(p: &RegPlane, phys: u64) {
    let bar = kayfabe_abi::pcibars::bus_bar::REGS as u8;
    let cur = p.read(bar, GA106.bar0_window_reg, 4).value() as u32;
    let base = u32::try_from(phys >> 16).expect("a 24-bit window base");
    p.write(
        bar,
        GA106.bar0_window_reg,
        4,
        u64::from((cur & !0x00FF_FFFF) | (base & 0x00FF_FFFF)),
    );
}

/// ★★★ Write one dword **through the BAR0 moving window** — the guest's own transport, and
/// the only way this file ever puts a byte in the framebuffer.
///
/// ⊘ Not a store poke. The whole subject of this file is *which statement records the
/// write*, and a fixture that seeded `SparseFb` directly would be green against a witness
/// that never fired — which is precisely the state the tree was in for four rungs.
fn win_wr32(p: &RegPlane, phys: u64, val: u32) {
    point_window(p, phys);
    let w = p.write(
        kayfabe_abi::pcibars::bus_bar::REGS as u8,
        GA106.pramin_window.base + (phys & 0xFFFF),
        4,
        u64::from(val),
    );
    assert_eq!(w.fb_landed, Some(phys), "the window write must land");
}

/// One 64-bit entry, low half first — how a 32-bit register interface has to do it.
fn win_wr_entry(p: &RegPlane, phys: u64, entry: u64) {
    win_wr32(p, phys, entry as u32);
    win_wr32(p, phys + 4, (entry >> 32) as u32);
}

/// Build the boot's own five-level tree mapping [`RING_VA`] → [`RING_PHYS`], through the
/// window, in the guest's write order (root first, leaf last).
fn build_tree(p: &RegPlane) {
    // L0[(va>>47) & 3] → L1. The GA10x root has FOUR entries, not 512
    // (`ogkm-580: kern_gmmu_fmt_gp10x.c:59-60`).
    win_wr_entry(p, ROOT + ((RING_VA >> 47) & 3) * 8, pde_vid(L1));
    // L1[(va>>38) & 511] → L2
    win_wr_entry(p, L1 + ((RING_VA >> 38) & 511) * 8, pde_vid(L2));
    // L2[(va>>29) & 511] → L3
    win_wr_entry(p, L2 + ((RING_VA >> 29) & 511) * 8, pde_vid(L3));
    // L3[(va>>21) & 255] is a 16-byte DUAL slot: the low qword is the big-page sub-table
    // (left zero — there is none) and the high qword is the small-page one. ★ This is the
    // fork §16.73's tracer printed as `/dual1of2`.
    let slot = L3 + ((RING_VA >> 21) & 255) * 16;
    win_wr_entry(p, slot, 0);
    win_wr_entry(p, slot + 8, dual_small_vid(L5));
    // L5[(va>>12) & 511] → the ring's own system-memory page.
    win_wr_entry(p, L5 + ((RING_VA >> 12) & 511) * 8, pte_sys(RING_PHYS));
}

/// A device with one compute proc whose address space is rooted at [`PDB`].
fn device(mode: LockMode) -> Guarded<SharedDevice> {
    let arch = Box::new(kayfabe_mocks::MockArch::new());
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");
    let mut s = Scenario::new();
    s.compute_process(HClient(0xAA), PDB, identical_handles(0x10, 0x11));
    for ev in s.events {
        gpu.apply(ev).expect("applies");
    }
    Guarded::new("cpu_pt_transport", gpu, rec).map(|g| SharedDevice::new(g, mode))
}

/// The format the tree was written in — the same value the composition root installs.
fn fmt() -> kayfabe_chips::Ga10xGmmu {
    kayfabe_chips::Ga10xGmmu::new()
}

/// The proc that owns `(GPU, PDB)`.
///
/// ⊘ Not `live_pids()[0]`: a device has a **system proc** as well as the user one, and
/// indexing the first live id is a coin flip that reads as a fixture detail. The address
/// space is the identity that matters here, so it is what the lookup is keyed on.
fn owner(d: &SharedDevice) -> kayfabe_core::ProcId {
    d.live_pids()
        .into_iter()
        .find(|&pid| {
            d.with_proc(pid, |p| p.vases.contains_key(&(GPU, PDB)))
                .unwrap_or(false)
        })
        .expect("some proc owns the address space")
}

/// ★★★ The rung's loop, transcribed from `SharedDoorbell::decode_cpu_pt_writes`: attribute
/// what the index can name, decode it, and try the leftovers again.
///
/// Returns `(rounds, latched, still_unattributed, bound)`.
fn attribute_and_decode(
    p: &RegPlane,
    d: &SharedDevice,
    rounds_max: usize,
) -> (usize, usize, Vec<u64>, usize) {
    let f = fmt();
    let mut pending = p.drain_pt_witness();
    let (mut rounds, mut latched, mut bound) = (0, 0, 0);
    while rounds < rounds_max && !pending.is_empty() {
        let w = d.witness_cpu_pt_pages(GPU, &pending);
        latched += w.latched;
        pending = w.unattributed;
        if w.procs.is_empty() {
            break;
        }
        rounds += 1;
        for pid in w.procs {
            let mut fb = p.pt_bytes();
            let out = d
                .decode_pt_writes_from(pid, &f, &mut fb)
                .expect("the proc is live");
            assert_eq!(out.transport, None, "this source has no socket to break");
            bound += out.bound;
        }
    }
    p.requeue_pt_witness(pending.iter().copied());
    (rounds, latched, pending, bound)
}

// =====================================================================================
// 1. ★★★★ G1 — THE WITNESS, at the statement that stamps the transport's name
// =====================================================================================

/// ★★★★ **A window write is witnessed, and it is witnessed as the same page the
/// first-writer census attributes it to.**
///
/// ⊘ The two facts are read off **one** access: `page_writer` says `PRAMIN` and
/// `drain_pt_witness` yields that page, and both come from the same
/// `FbStore::write_tagged` call. A fixture that asserted them from two writes would be
/// asserting that the witness fires *somewhere*, which is not the claim.
#[test]
fn a_window_write_is_witnessed_at_the_same_statement_that_names_its_transport() {
    let p = plane();
    assert_eq!(
        p.pt_witness_stats(),
        kayfabe_device::plane::PtWitnessStats::default(),
        "⊘ nothing before the guest writes — the set is not seeded"
    );

    win_wr32(&p, ROOT + 0x40, 0xdead_beef);

    let st = p.pt_witness_stats();
    assert_eq!((st.pending, st.writes, st.refused), (1, 1, 0));
    assert_eq!(
        p.fb_page_origin(ROOT).map(|o| o.by),
        Some(FbWriter::Window(kayfabe_device::FbWindow::Pramin)),
        "★ the census's own answer for this page — the tag the witness is taken beside"
    );
    assert_eq!(p.drain_pt_witness(), vec![ROOT]);
    // ★★ It is a DRAIN. A page written again must be witnessed again, and leaving it set
    // would make the second write indistinguishable from the first — `plan_pt_decode`'s
    // own argument, one layer up.
    assert!(p.drain_pt_witness().is_empty());
}

/// ★★★ **Deduped by PAGE, and both pages of a straddling access.**
///
/// `[measured 2026-08-10, boot `w208_797a6bc_real`]` one boot made **384 807** framebuffer
/// window writes and created **50** BAR2 pages. A witness that recorded accesses rather
/// than pages would hand the doorbell 384 807 rows to attribute, each one a rank-0
/// acquisition.
#[test]
fn the_witness_records_pages_not_accesses_and_never_loses_a_straddle() {
    let p = plane();
    // 1 024 dwords over one page.
    for i in 0..1024u64 {
        win_wr32(&p, L5 + i * 4, i as u32);
    }
    // …and one 8-byte access that ends in the NEXT page.
    point_window(&p, L5 + 0xffc);
    let w = p.write(
        kayfabe_abi::pcibars::bus_bar::REGS as u8,
        GA106.pramin_window.base + ((L5 + 0xffc) & 0xFFFF),
        8,
        0x1122_3344_5566_7788,
    );
    assert_eq!(w.fb_landed, Some(L5 + 0xffc));

    let st = p.pt_witness_stats();
    assert_eq!(st.writes, 1025, "every access counted");
    assert_eq!(
        p.drain_pt_witness(),
        vec![L5, L5 + 0x1000],
        "★ two PAGES from 1 025 accesses — and the straddle's second page is one of them, \
         because `FbStore::write` really does land bytes in both"
    );
}

/// ★★★ **A REFUSED write is not witnessed.**
///
/// A write the store did not take changed no byte. Witnessing it would put a page into the
/// drain that nothing backs, and `unwitnessed` would then be under-reported for a page
/// nobody wrote — the witness gate lying in the one direction that binds.
#[test]
fn a_refused_window_write_leaves_no_witness() {
    let p = plane();
    // Past the framebuffer this chip advertises: `SparseFb` refuses by name.
    let beyond = GA106.fb_length + 0x1000;
    point_window(&p, beyond);
    let w = p.write(
        kayfabe_abi::pcibars::bus_bar::REGS as u8,
        GA106.pramin_window.base,
        4,
        0xabcd_abcd,
    );
    assert!(
        w.fb_landed.is_none() && w.fb_refusal.is_some(),
        "the store must refuse this, or the test proves nothing: {w:?}"
    );
    assert_eq!(p.pt_witness_stats().pending, 0);
    assert!(p.drain_pt_witness().is_empty());
}

/// ★★★★ **A device reset clears it** — `bar_pdes`' cross-life argument, sharpened.
///
/// An undrained page here is handed to `Spine::pt_page_owner` and then **decoded as
/// page-table bytes** into whichever address space claims it. Carried across a device life
/// that publishes the *previous* guest's page tables as this guest's mappings.
#[test]
fn a_device_reset_clears_the_witness_because_the_next_guest_would_decode_it() {
    let p = plane();
    win_wr32(&p, ROOT, 1);
    assert_eq!(p.pt_witness_stats().pending, 1);
    p.device_reset();
    assert_eq!(
        p.pt_witness_stats(),
        kayfabe_device::plane::PtWitnessStats::default(),
        "⊘ and the totals with it: they are life-scoped diagnostics"
    );
    assert!(p.drain_pt_witness().is_empty());
    assert_eq!(p.residue().pt_witness.pending, 0);
}

// =====================================================================================
// 2. ★★★★ THE ATTRIBUTION — "not yet" is carried, never dropped
// =====================================================================================

/// ★★★★ **The index knows the ROOT and nothing else, so the first round attributes exactly
/// one page and hands the other four back.**
///
/// This is the shape that makes the loop necessary and it is asserted as a *count*, not as
/// a hope: `Spine::pt_page_owner` answers from `pt_roots` (declared) or `pt_learned`
/// (discovered), and at the first drain nothing has been discovered.
#[test]
fn the_first_attribution_round_claims_only_the_declared_root() {
    for mode in [LockMode::Degenerate, LockMode::Sharded] {
        let p = plane();
        let d = device(mode);
        build_tree(&p);
        let pages = p.drain_pt_witness();
        assert_eq!(pages.len(), 5, "{mode:?}: five pages, one per level");

        let w = d.witness_cpu_pt_pages(GPU, &pages);
        assert_eq!((w.latched, w.vas_gone), (1, 0), "{mode:?}");
        assert_eq!(
            w.unattributed,
            vec![L5, L3, L2, L1],
            "{mode:?}: ⊘ CARRIED BACK, ascending — a page the index cannot name an owner \
             for is not a page that was not written, and dropping it destroys the only \
             record that the guest wrote it"
        );
        assert_eq!(w.procs.len(), 1, "{mode:?}");
    }
}

/// ★★★ **A page nobody owns is requeued and survives to the next drain.**
///
/// ⊘ The BITE for the previous test: the leftovers are only useful if they come *back*.
#[test]
fn requeued_pages_survive_to_the_next_drain() {
    let p = plane();
    build_tree(&p);
    let pages = p.drain_pt_witness();
    // ⊘ The non-vacuity clause. `[]` requeued gives `[]` drained and the equality below is
    // satisfied by a witness that never fired — which is exactly what a bite on the insert
    // site produced when this test was written without this line.
    assert_eq!(pages.len(), 5, "there must be something to requeue");
    assert_eq!(p.requeue_pt_witness(pages.iter().copied()), 0);
    assert_eq!(p.drain_pt_witness(), pages, "byte for byte, and deduped");
}

// =====================================================================================
// 3. ★★★★★ THE RUNG — G1 + G2 + G3 joined: the ring's VA becomes BOUND
// =====================================================================================

/// ★★★★★ **The guest writes its tree through a window and the core's address table gains
/// the ring's mapping.**
///
/// The whole of §16.73.8, joined. ⊘ It is verified by the **binding**, which is what
/// `kayfabe_fwd::read_gpfifo_ring` consults — `AddressTable::binding_at(gpFifoOffset)`,
/// whose miss is the `RING-VA-UNBOUND` this rung exists to remove — and by the aperture
/// and physical address it carries, not by "the table has an entry".
#[test]
fn the_cpu_written_tree_binds_the_rings_va_in_the_cores_address_table() {
    for mode in [LockMode::Degenerate, LockMode::Sharded] {
        let p = plane();
        let d = device(mode);
        build_tree(&p);

        let (rounds, latched, left, bound) = attribute_and_decode(&p, &d, 8);
        assert_eq!(latched, 5, "{mode:?}: every level reached its `Vas`");
        assert!(
            rounds >= 2,
            "{mode:?}: a five-level tree cannot close in one"
        );
        assert!(left.is_empty(), "{mode:?}: nothing unattributable was left");
        assert_eq!(
            bound, 1,
            "{mode:?}: exactly the one leaf this tree declares"
        );

        let pid = owner(&d);
        let seen = d
            .with_proc(pid, |proc| {
                proc.vases[&(GPU, PDB)]
                    .table
                    .binding_at(GpuVa(RING_VA))
                    .map(|(start, len, b)| (start, len, b.phys(), b.aperture()))
            })
            .expect("the proc is live");
        assert_eq!(
            seen,
            Some((
                RING_VA,
                0x1000,
                RING_PHYS,
                kayfabe_arch::Aperture::SysmemCoherent
            )),
            "{mode:?}: ★★★★★ `LEAF@0x420064000→0x237fe000/SysmemCoherent/sz0x1000`, the \
             boot's own line, now in the table `read_gpfifo_ring` asks"
        );
        // ★★★ And the pages reached the DEVICE-GLOBAL index, which is the half that makes
        // the *next* write into one of them classifiable rather than forwarded as data.
        for page in TREE {
            assert_eq!(
                d.pt_page_owner(GPU, page),
                Some((pid, PDB)),
                "{mode:?}: page {page:#x} is not owned"
            );
        }
    }
}

/// ★★★★★ **THE BITE — the pass RUNS, reads the identical bytes, LEARNS the whole tree, and
/// binds nothing, because only the ROOT was witnessed.**
///
/// ⊘⊘ **My first draft of this test was REFUTED BY ITS OWN RUN, and the refutation is the
/// finding.** It transplanted the tree's bytes into a fresh store so no window write ever
/// happened, then latched all five pages by hand to make the pass execute — and `bound`
/// came back **1**. Of course it did: `plan_pt_decode` takes the witness *at the drain*
/// (`vas.reach.witness(page)`), so handing a page to the latch **is** witnessing it. The
/// test could not fail for the reason it was named for, which is the
/// `injection_measures_necessity_never_sufficiency` shape in a test written to catch a
/// defect already diagnosed.
///
/// What discriminates is not *where the bytes came from* but **which pages the witness
/// carries**. So this latches the ROOT alone — the one page `Spine::pt_page_owner` knows
/// before anything is discovered — and asserts the exact state the tree was in for four
/// rungs: the descent reaches the leaf, learns every level, and refuses to bind because the
/// leaf's page was never seen to be written (`reachability_on_transition.md` §2.2). The
/// second half then witnesses the remaining four and the same leaf binds.
///
/// ⇒ ★★★ **The mutation is run in both directions, on one fixture**, which is the only
/// thing that shows the assertion is about the witness and not about the walk.
#[test]
fn only_the_root_witnessed_learns_the_whole_tree_and_binds_nothing() {
    for mode in [LockMode::Degenerate, LockMode::Sharded] {
        let p = plane();
        let d = device(mode);
        build_tree(&p);
        let pages = p.drain_pt_witness();
        assert_eq!(pages.len(), 5, "{mode:?}");
        let f = fmt();

        // ---- ARM 1: the root, alone.
        let w = d.witness_cpu_pt_pages(GPU, &[ROOT]);
        assert_eq!(w.latched, 1, "{mode:?}");
        let pid = w.procs[0];
        let mut fb = p.pt_bytes();
        let out = d
            .decode_pt_writes_from(pid, &f, &mut fb)
            .expect("the proc is live");
        assert_eq!(
            out.meta_learned, 4,
            "{mode:?}: ⊘ THE PASS RAN AND SAW EVERYTHING — four deeper pages learned out \
             of the device's own store, so nothing here is a walk that failed: {out:?}"
        );
        assert_eq!(
            (out.bound, out.repointed),
            (0, 0),
            "{mode:?}: ★★★★★ and it bound NOTHING. This is the four-rung state, reproduced \
             in a unit test: the table is INCOMPLETE, not the wrong mechanism"
        );
        assert!(
            out.unwitnessed > 0,
            "{mode:?}: and it says so BY NAME rather than by an absence: {out:?}"
        );
        assert_eq!(
            d.with_proc(pid, |proc| proc.vases[&(GPU, PDB)]
                .table
                .binding_at(GpuVa(RING_VA))
                .is_some()),
            Some(false),
            "{mode:?}: `RING-VA-UNBOUND`, in one process and no guest"
        );

        // ---- ARM 2: the same fixture, the same bytes, the other four pages witnessed.
        let rest: Vec<u64> = pages.into_iter().filter(|&x| x != ROOT).collect();
        let w2 = d.witness_cpu_pt_pages(GPU, &rest);
        assert_eq!(
            w2.latched, 4,
            "{mode:?}: ⊘ and they are attributable NOW only because arm 1 published them"
        );
        let mut fb2 = p.pt_bytes();
        let out2 = d
            .decode_pt_writes_from(pid, &f, &mut fb2)
            .expect("the proc is live");
        assert_eq!(
            out2.bound, 1,
            "{mode:?}: ★★★★★ the identical tree, the identical bytes, the identical walk — \
             and the ONLY thing that changed is which pages the witness carried: {out2:?}"
        );
    }
}

/// ★★★ **THE SECOND BITE — the witness fires and the OWNER is wrong: still nothing binds.**
///
/// The tree is written through the window, so the witness is full; the address space is
/// rooted somewhere else, so `Spine::pt_page_owner` answers for none of it. ⊘ This is the
/// arm that would be green if the pass bound leaves into whatever `Vas` it happened to be
/// visiting — the C's never-pruned-table aliasing class, which R5 and the index exist to
/// refuse.
#[test]
fn a_tree_whose_root_belongs_to_no_address_space_binds_nothing_and_is_carried() {
    let p = plane();
    build_tree(&p);

    // A device whose one proc is rooted at a DIFFERENT page.
    let arch = Box::new(kayfabe_mocks::MockArch::new());
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");
    let mut s = Scenario::new();
    s.compute_process(
        HClient(0xAA),
        Pdb(0x1_0000_0000),
        identical_handles(0x10, 0x11),
    );
    for ev in s.events {
        gpu.apply(ev).expect("applies");
    }
    let d = Guarded::new("cpu_pt_transport::foreign", gpu, rec)
        .map(|g| SharedDevice::new(g, LockMode::Sharded));

    let (rounds, latched, left, bound) = attribute_and_decode(&p, &d, 8);
    assert_eq!((rounds, latched, bound), (0, 0, 0));
    assert_eq!(
        left.len(),
        5,
        "⊘ and all five come BACK — an unattributable page is 'not yet', not 'not a page'"
    );
    assert_eq!(
        p.pt_witness_stats().pending,
        5,
        "…and they are in the plane again, for a later doorbell to try"
    );
}

// =====================================================================================
// 4. ⊘ THE SOURCE SEAM — stated as a test, because it is the one open design point
// =====================================================================================

/// ★★★★ **The decode reads the DEVICE's store, and reading the wrong one is visible.**
///
/// §16.73.7 flagged this and declined to take it: `run_pt_decode` reached the isolate
/// (`kayfabe_fwd::IsolateFb` → `RmBackend::fb_read` = `Err(NOT_ON_THIS_RUNG)`), while the
/// guest's page tables are demonstrably in the device's own `FbStore` — the store `/byBAR2`
/// was answered out of.
///
/// ⊘ **A self-consistent wrong store is the failure this guards**: a writer and a reader
/// that agree and are both wrong. So the assertion is not *"the right store answers"* — it
/// is that an **empty** store makes the same pass fault, by name, at the first read, rather
/// than decoding zeros into a plausible table.
#[test]
fn a_byte_source_that_holds_none_of_the_tree_faults_rather_than_decoding_zeros() {
    let p = plane();
    build_tree(&p);
    let pages = p.drain_pt_witness();

    // The same pass, over a plane whose framebuffer never saw those bytes.
    let empty = plane();
    let d = device(LockMode::Sharded);
    let f = fmt();
    let w = d.witness_cpu_pt_pages(GPU, &pages);
    assert_eq!(w.latched, 1, "the root is attributable either way");
    let pid = w.procs[0];
    let mut fb = empty.pt_bytes();
    let out = d
        .decode_pt_writes_from(pid, &f, &mut fb)
        .expect("the proc is live");
    // ⊘ `SparseFb` answers zeros for an unwritten address INSIDE the framebuffer — which is
    // a true statement about memory this device owns — so the honest outcome is a root full
    // of invalid entries, and therefore NO leaf and NO learned page. ★ The discriminator is
    // that it binds nothing and learns nothing, where the populated store learns four.
    assert_eq!(
        (out.bound, out.meta_learned, out.repointed),
        (0, 0, 0),
        "an empty store must produce an empty tree, never a plausible one: {out:?}"
    );
}
