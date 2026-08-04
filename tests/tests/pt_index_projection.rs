//! # ★★★ The page-table ownership index: PUBLISH must equal the PROJECTION
//!
//! `Spine::pt_learned` is written two ways — incrementally by
//! [`Spine::publish_pt_pages`] at the decode's commit point (E8's fourth phase), and from
//! scratch by `Spine::refresh` as a projection of every live `Vas::pt_meta`. The design
//! rests on those two producing the **same map**: *"publishing cannot disagree with the
//! projection: it is the projection, run sooner."*
//!
//! ## ⚠ Why this file exists — E8 shipped that claim CITED TO A TEST THAT DID NOT EXIST
//!
//! `gpu.rs` said the invariant was *"stated as a test"* named
//! `pt_index_publish_equals_projection`. That string occurred exactly once in the tree —
//! in the comment itself. It is written here now, and it **fails** against E8's first cut:
//!
//! | path | conflict rule | visible? |
//! |---|---|---|
//! | `publish_pt_pages` (E8 v1) | refuse the **second arrival**, keep the incumbent | counted |
//! | `refresh` (E8 v1) | `entry().or_insert()` — keep the **lowest `ProcId`** | silent |
//!
//! So `pt_page_owner(P)` answered one proc before a refresh and another after, and the
//! "REFUSED, not re-homed" property — sold as the fix for the C's last-writer-wins
//! attribution — was quietly undone by the projection.
//!
//! ⊘ Neither "first" rule can be implemented on both paths: publish cannot know a future
//! claimant, and the projection has no arrival order. So both **decline** — a contested
//! page is indexed for nobody (`Spine::pt_contested`).
//!
//! ## ★ And every refusal here is driven, not asserted at zero
//!
//! E8's tests asserted `(published, refused) == (4, 0)` — happy path only, `refused`
//! pinned at zero — so an adversarial reviewer's five mutations (re-home on conflict,
//! disable the R5 re-resolve, disable the ceiling, flip the projection to last-writer,
//! delete the projection rebuild) were **all uncaught**. Each is driven below.
//!
//! Invariant/contract tests (decision #15), mock-driven, **GPU-free**.

use kayfabe_arch::ids::{GpuId, HClient, Pdb};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::{Gpu, MAX_PT_LEARNED};
use kayfabe_mmu::walker::PtPage;
use kayfabe_mocks::{MockArch, MockIsolateFactory};
use kayfabe_tests::{Scenario, identical_handles};

const GPU: GpuId = GpuId::ZERO;
const A_PDB: Pdb = Pdb(0x1001_0000);
const B_PDB: Pdb = Pdb(0x2001_0000);
const A_CLIENT: HClient = HClient(0xAA);
const B_CLIENT: HClient = HClient(0xBB);

/// A page only A's tables reach.
const A_ONLY: u64 = 0x1005_0000;
/// A page only B's tables reach.
const B_ONLY: u64 = 0x2005_0000;
/// ★ The page BOTH claim — guest aliasing, or a wrong decode. Neither may own it.
const SHARED: u64 = 0x3005_0000;

fn pt_page(phys: u64) -> PtPage {
    PtPage {
        phys,
        aperture: kayfabe_arch::Aperture::Vidmem,
        level: 1,
        vabase: 0,
    }
}

/// Two live compute processes, each with its own VAS.
fn two_procs() -> (Gpu, kayfabe_core::ProcId, kayfabe_core::ProcId) {
    let (factory, _rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu =
        Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa).expect("the device realizes");
    let mut s = Scenario::new();
    s.compute_process(A_CLIENT, A_PDB, identical_handles(0x10, 0x11));
    s.compute_process(B_CLIENT, B_PDB, identical_handles(0x20, 0x21));
    for ev in s.events {
        gpu.apply(ev).expect("applies");
    }
    let a = *gpu.spine.by_pdb.get(&(GPU, A_PDB)).expect("A's VAS routed");
    let b = *gpu.spine.by_pdb.get(&(GPU, B_PDB)).expect("B's VAS routed");
    assert_ne!(a, b, "the fixture must model TWO procs, not one");
    (gpu, a, b)
}

/// Put `pages` into `(pid, pdb)`'s learned metadata — what `commit_pt_decode` does.
fn learn(gpu: &mut Gpu, pid: kayfabe_core::ProcId, pdb: Pdb, pages: &[u64]) {
    let vas = gpu
        .procs
        .get_mut(&pid)
        .expect("live")
        .vases
        .get_mut(&(GPU, pdb))
        .expect("the vas");
    for &p in pages {
        vas.pt_meta.insert(p, pt_page(p));
    }
}

/// Force a projection rebuild the way the guest does — any RM graph event.
fn reproject(gpu: &mut Gpu) {
    let mut s = Scenario::new();
    s.compute_process(
        HClient(0xCC),
        Pdb(0x3001_0000),
        identical_handles(0x30, 0x31),
    );
    for ev in s.events {
        gpu.apply(ev).expect("applies");
    }
}

// =====================================================================================

/// ★★★ **THE INVARIANT E8 CITED AND DID NOT WRITE.** Publish incrementally, then force the
/// projection to rebuild, and the index must be **bit-identical** — including which pages
/// are contested.
///
/// ⊘ The `SHARED` page is what makes this bite. Without a contested page both rules agree
/// trivially and the test is vacuous; the assertion below therefore also checks that the
/// fixture really did produce a contested page.
#[test]
fn pt_index_publish_equals_projection() {
    let (mut gpu, a, b) = two_procs();
    learn(&mut gpu, a, A_PDB, &[A_ONLY, SHARED]);
    learn(&mut gpu, b, B_PDB, &[B_ONLY, SHARED]);

    // INCREMENTAL: A publishes first, then B — so a first-by-arrival rule would keep A,
    // and (because A's ProcId may be the lower) a first-by-ProcId rule could agree by
    // accident. The contested outcome is the only one both paths can reach.
    gpu.spine
        .publish_pt_pages(a, GPU, A_PDB, vec![A_ONLY, SHARED]);
    gpu.spine
        .publish_pt_pages(b, GPU, B_PDB, vec![B_ONLY, SHARED]);

    let learned_after_publish = gpu.spine.pt_learned.clone();
    let contested_after_publish = gpu.spine.pt_contested.clone();

    // ★ non-vacuity: the fixture MUST have produced a contest, or this proves nothing.
    assert!(
        contested_after_publish.contains(&(GPU, SHARED)),
        "the shared page must be contested after publish — without that the two conflict \
         rules agree trivially and this test cannot detect a disagreement"
    );

    reproject(&mut gpu);

    assert_eq!(
        gpu.spine.pt_learned, learned_after_publish,
        "★★★ the projection must rebuild EXACTLY what publish built. E8 v1 failed here: \
         publish refused the second arrival while refresh kept the lowest ProcId"
    );
    assert_eq!(
        gpu.spine.pt_contested, contested_after_publish,
        "…and it must agree about which pages are contested"
    );
}

/// ★★★ A page two address spaces claim is owned by **NEITHER**, and `pt_page_owner`
/// declines — so a guest CE write into it is forwarded as data rather than attributed.
#[test]
fn a_page_two_procs_claim_is_indexed_for_nobody() {
    let (mut gpu, a, b) = two_procs();
    learn(&mut gpu, a, A_PDB, &[A_ONLY, SHARED]);
    learn(&mut gpu, b, B_PDB, &[B_ONLY, SHARED]);

    let (pub_a, ref_a) = gpu
        .spine
        .publish_pt_pages(a, GPU, A_PDB, vec![A_ONLY, SHARED]);
    assert_eq!((pub_a, ref_a), (2, 0), "A's claim lands first, uncontested");
    assert_eq!(
        gpu.spine.pt_page_owner(GPU, SHARED),
        Some((a, A_PDB)),
        "…and until B claims it, A owns it"
    );

    let (pub_b, ref_b) = gpu
        .spine
        .publish_pt_pages(b, GPU, B_PDB, vec![B_ONLY, SHARED]);
    assert_eq!(
        (pub_b, ref_b),
        (1, 1),
        "B's own page publishes; the shared one is REFUSED and counted"
    );

    assert_eq!(
        gpu.spine.pt_page_owner(GPU, SHARED),
        None,
        "★★★ NEITHER owns it — the incumbent is evicted too. Keeping A would be \
         first-by-arrival, which the projection cannot reproduce"
    );
    assert_eq!(gpu.spine.pt_page_owner(GPU, A_ONLY), Some((a, A_PDB)));
    assert_eq!(gpu.spine.pt_page_owner(GPU, B_ONLY), Some((b, B_PDB)));

    // ★ Sticky: re-offering it does not resurrect it, so the path is idempotent.
    let (again, refused_again) = gpu.spine.publish_pt_pages(a, GPU, A_PDB, vec![SHARED]);
    assert_eq!((again, refused_again), (0, 1));
    assert_eq!(gpu.spine.pt_page_owner(GPU, SHARED), None);
}

/// ★★ **R5 drives.** Publishing for an address space that does not route to `pid`
/// publishes nothing — the guard E8 shipped untested.
#[test]
fn publishing_for_an_address_space_that_is_not_this_procs_publishes_nothing() {
    let (mut gpu, a, b) = two_procs();

    // A's pid with B's PDB: `by_pdb` maps (GPU, B_PDB) → b, not a.
    let (published, refused) = gpu.spine.publish_pt_pages(a, GPU, B_PDB, vec![B_ONLY]);
    assert_eq!(
        (published, refused),
        (0, 1),
        "★ the R5 re-resolve must refuse the whole batch — a recycled PDB naming another \
         proc is exactly the aliasing class the index exists to prevent"
    );
    assert_eq!(gpu.spine.pt_page_owner(GPU, B_ONLY), None);

    // A PDB that routes nowhere at all.
    let (p2, r2) = gpu
        .spine
        .publish_pt_pages(b, GPU, Pdb(0xDEAD_0000), vec![A_ONLY]);
    assert_eq!((p2, r2), (0, 1));
    assert_eq!(gpu.spine.pt_page_owner(GPU, A_ONLY), None);
}

/// ★★ **The ceiling drives.** Fill the device-global index and the next page is refused
/// and counted — not silently dropped, and never evicting a live entry.
#[test]
fn the_device_global_ceiling_refuses_and_counts_rather_than_evicting() {
    let (mut gpu, a, _b) = two_procs();
    assert_eq!(gpu.spine.pt_learned_refused, 0);

    // Fill exactly to the ceiling. Page addresses are guest-chosen, which is why the
    // bound exists at all (boundary-1).
    let fill: Vec<u64> = (0..MAX_PT_LEARNED as u64)
        .map(|i| 0x1_0000_0000 + i * 0x1000)
        .collect();
    let (published, refused) = gpu.spine.publish_pt_pages(a, GPU, A_PDB, fill);
    assert_eq!(
        (published, refused),
        (MAX_PT_LEARNED, 0),
        "the ceiling is not yet reached"
    );
    assert_eq!(gpu.spine.pt_learned.len(), MAX_PT_LEARNED);

    let (p2, r2) = gpu.spine.publish_pt_pages(a, GPU, A_PDB, vec![A_ONLY]);
    assert_eq!((p2, r2), (0, 1), "★ one more is REFUSED");
    assert_eq!(
        gpu.spine.pt_learned_refused, 1,
        "…and counted, so an under-provisioned address plane is visible rather than silent"
    );
    assert_eq!(
        gpu.spine.pt_learned.len(),
        MAX_PT_LEARNED,
        "★★ nothing was evicted — evicting a page the guest is still writing would return \
         it to 'ordinary data' and unbind its leaves"
    );
}

/// ★★★ **The projection PRUNES.** A page whose owning proc is gone must not survive a
/// rebuild — the "a Vas that dies takes its pages with it" property, which E8 left with
/// zero coverage (deleting the entire rebuild loop failed no test).
#[test]
fn the_projection_drops_pages_whose_metadata_is_gone() {
    let (mut gpu, a, _b) = two_procs();
    learn(&mut gpu, a, A_PDB, &[A_ONLY]);
    gpu.spine.publish_pt_pages(a, GPU, A_PDB, vec![A_ONLY]);
    assert_eq!(gpu.spine.pt_page_owner(GPU, A_ONLY), Some((a, A_PDB)));

    // The decode's knowledge goes away — the page is no longer a page table to us.
    gpu.procs
        .get_mut(&a)
        .expect("live")
        .vases
        .get_mut(&(GPU, A_PDB))
        .expect("the vas")
        .pt_meta
        .remove(&A_ONLY);

    reproject(&mut gpu);

    assert_eq!(
        gpu.spine.pt_page_owner(GPU, A_ONLY),
        None,
        "★★★ the projection is REBUILT, not accreted — a page no live Vas still names is \
         gone. The C's table was 'never pruned on handle free' and this is that defect's \
         absence, asserted"
    );
}
