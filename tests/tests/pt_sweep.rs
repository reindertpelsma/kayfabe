//! ★★★★★ **THE WHOLE-VAS SWEEP** — the C's `enum_gr_sysmem` (`C: nvkvm_gpu_emul.c:583-591`),
//! ported, and the ONE relaxation of the witness rule this port carries.
//!
//! The rung this file exists for is an **adjudication**, not a feature, so the tests are
//! written against the adjudication's two halves rather than against the code's surface:
//!
//! 1. ★★★★★ **The refusal is real, and the sweep is what moves it.** A root-seeded descent
//!    committed the ordinary way publishes **zero** — every page it reaches is unwitnessed by
//!    construction — and the *same* descent committed as a sweep publishes the leaves. This is
//!    `[measured, w275]` turned into a regression: if someone re-tightens the gate, the first
//!    test still passes and the second fails, which is the only way round that is legible.
//! 2. ★★★★★ **The mitigation is not optional.** A page the guest writes after a sweep re-arms
//!    the sweep (the C's `m2_gr_vas_dirty`); a sweep that outran its budget re-arms it too (the
//!    C's `m2_gr_pt_trunc`) **and contributed no leaves at all**; and a page that falls out of
//!    the tree loses its sweep admission along with its witness.
//!
//! ⊘ **What none of these can show.** They are mock-driven and GPU-free: they bound the
//! *logic* of the gate and the triggers. Whether a real `cuCtxCreate` address space is small
//! enough for [`kayfabe_fwd::PT_SWEEP_BUDGET`], and whether publishing its whole VAS moves the
//! wall, are boot measurements and are not in here.

#![allow(clippy::unusual_byte_groupings)]

use kayfabe_arch::ids::{GpuId, GpuVa, HClient, Pdb};
use kayfabe_arch::{Arch, GmmuFmt};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_fwd::{
    Admit, PT_SWEEP_BUDGET, SweepReason, commit_pt_decode_as, commit_pt_sweep, plan_pt_decode,
    plan_pt_sweep, run_pt_sweep,
};
use kayfabe_isolate::{
    CeExecutor, CeSource, CeSubCopy, HostHandle, IsolateFactory, IsolateId, VerbPlan, VerbReply,
    Worker,
};
use kayfabe_mmu::walker::WalkFault;
use kayfabe_mocks::{
    MOCK_DUAL_LEVEL, MockArch, MockGmmuFmt, MockIsolateFactory, SharedRecorder,
};
use kayfabe_tests::{Guarded, Scenario, identical_handles};

const GPU: GpuId = GpuId::ZERO;
const FAB_BASE: u64 = 0x1000_0000;
const FAB_LEN: u64 = 0x0400_0000;

const A_PDB: Pdb = Pdb(0x1001_0000);
const ROOT: u64 = A_PDB.0;
const PD_L1: u64 = 0x1002_0000;
const PD_L2: u64 = 0x1003_0000;
const PD_DUAL: u64 = 0x1004_0000;
const PT_SMALL: u64 = 0x1005_0000;
const STAGE: u64 = 0x1300_0000;

// =====================================================================================
// Scaffolding — deliberately the same shapes `pt_decode.rs` uses, so a reader comparing
// a sweep pass against a decode pass is comparing two passes and not two fixtures.
// =====================================================================================

fn image(width: usize, entries: usize, set: &[(usize, u128)]) -> Vec<u8> {
    let mut v = vec![0u8; entries * width];
    for &(i, e) in set {
        let at = i * width;
        v[at..at + width].copy_from_slice(&e.to_le_bytes()[..width]);
    }
    v
}

fn page_at(fmt: &dyn GmmuFmt, level: u8, set: &[(usize, u128)]) -> Vec<u8> {
    let g = fmt.level_shift(level).expect("a level the regime has");
    image(usize::from(fmt.entry_size(level)), g.entries as usize, set)
}

fn aperture_worker() -> (MockIsolateFactory, SharedRecorder) {
    let (factory, rec) = MockIsolateFactory::new();
    rec.lock().expect("recorder").fb_declare(FAB_BASE, FAB_LEN);
    (factory, rec)
}

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

fn leaf(phys: u64) -> u128 {
    MockGmmuFmt::encode_leaf(phys, false)
}
fn pde(next: u64) -> u128 {
    MockGmmuFmt::encode_pde(next, false, false)
}

fn small_leaf_level() -> u8 {
    MOCK_DUAL_LEVEL + 1
}

fn with_gpu<R>(gpu: &mut Guarded<Gpu>, f: impl FnOnce(&mut Gpu) -> R) -> R {
    f(&mut *gpu)
}

fn only_proc(gpu: &mut Gpu) -> &mut kayfabe_core::gpu::Proc {
    gpu.procs.values_mut().next().expect("one proc")
}

fn fixture() -> (Guarded<Gpu>, MockIsolateFactory, SharedRecorder) {
    let arch = Box::new(MockArch::new());
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");
    let mut s = Scenario::new();
    s.compute_process(HClient(0xAA), A_PDB, identical_handles(0x10, 0x11));
    for ev in s.events {
        gpu.apply(ev).expect("applies");
    }
    let (fb_factory, fb_rec) = aperture_worker();
    (Guarded::new("pt_sweep", gpu, rec), fb_factory, fb_rec)
}

/// ★★★ Build a complete four-level chain from the root down to one small-page leaf,
/// **without telling the port about any of it**. That last clause is the whole fixture: the
/// dirty set stays empty, so nothing here is witnessed, so a decode pass has no task and the
/// witness gate has nothing to admit.
fn write_a_whole_unwitnessed_tree(worker: &mut Worker, rec: &SharedRecorder, vas: HostHandle) {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    write_fabricated(worker, rec, vas, ROOT, &page_at(fmt, 0, &[(0, pde(PD_L1))]));
    write_fabricated(worker, rec, vas, PD_L1, &page_at(fmt, 1, &[(0, pde(PD_L2))]));
    write_fabricated(
        worker,
        rec,
        vas,
        PD_L2,
        &page_at(fmt, 2, &[(0, pde(PD_DUAL))]),
    );
    write_fabricated(
        worker,
        rec,
        vas,
        PD_DUAL,
        &page_at(fmt, MOCK_DUAL_LEVEL, &[(0, pde(PT_SMALL))]),
    );
    write_fabricated(
        worker,
        rec,
        vas,
        PT_SMALL,
        &page_at(fmt, small_leaf_level(), &[(9, leaf(0xF000_0000))]),
    );
}

// =====================================================================================
// 1. The adjudication itself
// =====================================================================================

/// ★★★★★ **`[measured, w275]` AS A REGRESSION: the root-seeded descent, committed the
/// ordinary way, PUBLISHES NOTHING.**
///
/// This is the finding that made this rung an adjudication rather than a loop, and it must
/// keep being true — because it is the statement of what the witness gate *is*. If this test
/// ever starts binding, the gate has been relaxed somewhere other than in the one named place.
#[test]
fn a_root_seeded_descent_committed_as_witnessed_binds_nothing() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let (mut gpu, factory, rec) = fixture();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);
    write_a_whole_unwitnessed_tree(&mut worker, &rec, vas);

    // The plan is the sweep's — one root task — but the COMMIT is the ordinary one.
    let plan = with_gpu(&mut gpu, |g| plan_pt_sweep(only_proc(g)));
    assert_eq!(
        plan.tasks.iter().map(|t| t.page.phys).collect::<Vec<_>>(),
        vec![ROOT],
        "a never-swept address space is seeded at its own page-directory root"
    );
    assert_eq!(plan.reasons, vec![SweepReason::NeverSwept]);

    let mut fb = kayfabe_fwd::IsolateFb::new(&mut worker);
    let results = run_pt_sweep(fmt, &mut fb, &plan.tasks, PT_SWEEP_BUDGET);
    let out = with_gpu(&mut gpu, |g| {
        commit_pt_decode_as(fmt, only_proc(g), &results, Admit::Witnessed)
    });

    assert_eq!(out.bound, 0, "★★★★★ THE REFUSAL — nothing binds");
    assert_eq!(out.swept_binds, 0, "and none of it was a sweep admission");
    assert!(
        out.unwitnessed > 0,
        "the leaves were REACHED and REFUSED, not missed: unwitnessed={}",
        out.unwitnessed
    );
    assert!(
        out.faults.is_empty() && out.reach_faults.is_empty(),
        "⊘ and it must be the GATE refusing, not the walk failing — {:?} {:?}",
        out.faults,
        out.reach_faults
    );
}

/// ★★★★★ **THE RELAXATION, on the identical descent.** Same tree, same walk, same leaves —
/// and the sweep commit publishes them, with [`kayfabe_fwd::PtDecodeOutcome::swept_binds`]
/// saying that every one of them exists *because* the gate moved.
#[test]
fn the_same_descent_committed_as_a_sweep_binds_and_says_the_relaxation_did_it() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let (mut gpu, factory, rec) = fixture();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);
    write_a_whole_unwitnessed_tree(&mut worker, &rec, vas);

    let plan = with_gpu(&mut gpu, |g| plan_pt_sweep(only_proc(g)));
    let mut fb = kayfabe_fwd::IsolateFb::new(&mut worker);
    let results = run_pt_sweep(fmt, &mut fb, &plan.tasks, PT_SWEEP_BUDGET);
    let out = with_gpu(&mut gpu, |g| commit_pt_sweep(fmt, only_proc(g), &results));

    assert_eq!(out.bound, 1, "the one leaf the tree describes");
    assert_eq!(
        out.swept_binds, 1,
        "★ and the count says the relaxation is the whole reason — a sweep whose \
         swept_binds is 0 published nothing the witness rule would not have"
    );
    assert_eq!(out.unwitnessed, 0);
    assert_eq!(out.sweeps_run, 1);
    assert_eq!(out.sweeps_truncated, 0);
    assert_eq!(
        out.pages_swept, 5,
        "root + three directories + the leaf table — the measurement PT_SWEEP_BUDGET must \
         be re-derived from, at this fixture's scale"
    );

    // ★★ And it landed in THE one authoritative table, not in a shadow of one. A pass that
    // reported `bound=1` while the table answered `Miss` would be a second address plane,
    // which is the thing this port refuses to grow.
    let small = fmt.level_shift(small_leaf_level()).expect("small");
    let va = GpuVa(9 * (1u64 << small.shift));
    let phys = with_gpu(&mut gpu, |g| {
        only_proc(g)
            .vases
            .get(&(GPU, A_PDB))
            .expect("the vas")
            .table
            .binding_at(va)
            .map(|(_, _, b)| b.phys())
    });
    assert_eq!(phys, Some(0xF000_0000));
}

/// ★★★ **A page a sweep never reached is still refused.** The relaxation is scoped to the
/// descent: an orphan table the guest filled but never linked is *not* root-reachable, so the
/// sweep does not admit it and its leaves stay a miss.
///
/// ⊘ This is what stops "sweep" from meaning "read whatever the guest points at".
#[test]
fn a_sweep_admits_only_what_it_descended_to() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let (mut gpu, factory, rec) = fixture();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);
    // A root that points at NOTHING, plus a fully-formed leaf table hanging off nobody.
    write_fabricated(&mut worker, &rec, vas, ROOT, &page_at(fmt, 0, &[]));
    write_fabricated(
        &mut worker,
        &rec,
        vas,
        PT_SMALL,
        &page_at(fmt, small_leaf_level(), &[(9, leaf(0xF000_0000))]),
    );

    let plan = with_gpu(&mut gpu, |g| plan_pt_sweep(only_proc(g)));
    let mut fb = kayfabe_fwd::IsolateFb::new(&mut worker);
    let results = run_pt_sweep(fmt, &mut fb, &plan.tasks, PT_SWEEP_BUDGET);
    let out = with_gpu(&mut gpu, |g| commit_pt_sweep(fmt, only_proc(g), &results));

    assert_eq!(out.bound, 0, "the orphan is not root-reachable");
    assert_eq!(out.swept_binds, 0);
    assert_eq!(out.pages_swept, 1, "only the root was walked");
    let swept_only = with_gpu(&mut gpu, |g| {
        only_proc(g)
            .vases
            .get(&(GPU, A_PDB))
            .expect("the vas")
            .reach
            .is_swept(PT_SMALL)
    });
    assert!(
        !swept_only,
        "★ a page the descent never reached must not carry a sweep admission"
    );
}

// =====================================================================================
// 2. The mitigation — build both halves or neither
// =====================================================================================

/// ★★★★★ **THE C's `m2_gr_vas_dirty`: a guest write to a TRACKED page re-arms the sweep.**
///
/// This is the half that makes the relaxation self-healing, and therefore the half that makes
/// it defensible: a page that was mid-update when the sweep read it was, by definition, being
/// written, so it arrives in the dirty set and the next doorbell re-sweeps it.
#[test]
fn a_write_to_a_swept_page_re_arms_the_sweep_and_a_quiet_address_space_does_not() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let (mut gpu, factory, rec) = fixture();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);
    write_a_whole_unwitnessed_tree(&mut worker, &rec, vas);

    // Sweep 1 — the `NeverSwept` trigger.
    let plan = with_gpu(&mut gpu, |g| plan_pt_sweep(only_proc(g)));
    let mut fb = kayfabe_fwd::IsolateFb::new(&mut worker);
    let results = run_pt_sweep(fmt, &mut fb, &plan.tasks, PT_SWEEP_BUDGET);
    with_gpu(&mut gpu, |g| commit_pt_sweep(fmt, only_proc(g), &results));

    // ⊘ THE CONTROL, and it comes first: with nothing dirty, the next plan walks NOTHING.
    // A sweep that re-armed unconditionally would pass every other assertion in this file.
    let quiet = with_gpu(&mut gpu, |g| plan_pt_sweep(only_proc(g)));
    assert_eq!(quiet.tasks, vec![], "a current picture is not re-walked");
    assert_eq!(quiet.skipped, 1);

    // The guest writes a page the sweep is TRACKING. The dirty signal is set by the DECODE
    // pass's drain — which is exactly why the two passes are one design.
    with_gpu(&mut gpu, |g| {
        only_proc(g)
            .vases
            .get_mut(&(GPU, A_PDB))
            .expect("the vas")
            .pt_pages
            .insert(PT_SMALL);
    });
    let drained = with_gpu(&mut gpu, |g| plan_pt_decode(only_proc(g)));
    assert_eq!(
        drained.tasks.len(),
        1,
        "the sweep taught the port this page's level, so the drain can decode it directly"
    );

    let rearmed = with_gpu(&mut gpu, |g| plan_pt_sweep(only_proc(g)));
    assert_eq!(
        rearmed.reasons,
        vec![SweepReason::Dirty],
        "★★★★★ the write re-armed the whole-VAS walk"
    );
    assert_eq!(
        rearmed.tasks.iter().map(|t| t.page.phys).collect::<Vec<_>>(),
        vec![ROOT]
    );
}

/// ★★★ **An UNTRACKED dirty page does not re-arm the sweep** — the trigger is the C's, not
/// "any write at all".
///
/// A page nothing points at yet is §12.1(i)'s orphan: it was not part of the picture the sweep
/// drew, so it cannot have staled it. Widening the trigger to every dirty page would arm a
/// whole-VAS walk on every doorbell forever, and a trigger that always fires is not a trigger.
#[test]
fn a_write_to_a_page_the_sweep_never_saw_does_not_re_arm_it() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let (mut gpu, factory, rec) = fixture();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);
    write_a_whole_unwitnessed_tree(&mut worker, &rec, vas);
    let plan = with_gpu(&mut gpu, |g| plan_pt_sweep(only_proc(g)));
    let mut fb = kayfabe_fwd::IsolateFb::new(&mut worker);
    let results = run_pt_sweep(fmt, &mut fb, &plan.tasks, PT_SWEEP_BUDGET);
    with_gpu(&mut gpu, |g| commit_pt_sweep(fmt, only_proc(g), &results));

    // A page at an address no decode has ever reached.
    const UNKNOWN: u64 = 0x1009_0000;
    with_gpu(&mut gpu, |g| {
        only_proc(g)
            .vases
            .get_mut(&(GPU, A_PDB))
            .expect("the vas")
            .pt_pages
            .insert(UNKNOWN);
    });
    let drained = with_gpu(&mut gpu, |g| plan_pt_decode(only_proc(g)));
    assert_eq!(
        drained.deferred,
        vec![(GPU, A_PDB, UNKNOWN)],
        "an unlinked page is DEFERRED, exactly as before the sweep existed"
    );
    let after = with_gpu(&mut gpu, |g| plan_pt_sweep(only_proc(g)));
    assert_eq!(after.tasks, vec![], "and the sweep is NOT re-armed by it");
}

/// ★★★★★ **THE C's `m2_gr_pt_trunc`: a budget-cut sweep contributes NOTHING and forces
/// another.**
///
/// ⚠ The first assertion is the one that matters and it is easy to get backwards: a truncated
/// walk is **not a smaller walk**. `decode_subtree` refuses the whole task, so a reader who
/// treats `sweeps_truncated > 0` as "we got most of it" has made the `dlen=0` mistake in a
/// different plane.
#[test]
fn a_truncated_sweep_publishes_nothing_and_re_arms_itself() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let (mut gpu, factory, rec) = fixture();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);
    write_a_whole_unwitnessed_tree(&mut worker, &rec, vas);

    let plan = with_gpu(&mut gpu, |g| plan_pt_sweep(only_proc(g)));
    let mut fb = kayfabe_fwd::IsolateFb::new(&mut worker);
    // A budget that cannot pay for even the root page.
    let results = run_pt_sweep(fmt, &mut fb, &plan.tasks, 1);
    assert!(matches!(
        results[0].decode,
        Err(WalkFault::BudgetExhausted)
    ));
    let out = with_gpu(&mut gpu, |g| commit_pt_sweep(fmt, only_proc(g), &results));
    assert_eq!(out.bound, 0);
    assert_eq!(out.swept_binds, 0);
    assert_eq!(out.sweeps_run, 0, "★ nothing COMPLETED");
    assert_eq!(out.sweeps_truncated, 1);
    assert_eq!(out.pages_swept, 0);

    let again = with_gpu(&mut gpu, |g| plan_pt_sweep(only_proc(g)));
    assert_eq!(
        again.tasks.iter().map(|t| t.page.phys).collect::<Vec<_>>(),
        vec![ROOT],
        "★★★★★ it re-arms — a truncated walk that did not would silently lose every \
         mapping past the cut"
    );
    // ⊘ And the REASON is still `NeverSwept`, because this address space has never completed
    // a walk. Asserted rather than glossed: the two triggers overlap here, and a reader who
    // saw `Truncated` would conclude the trunc bit is what re-armed it — which it is not YET.
    assert_eq!(again.reasons, vec![SweepReason::NeverSwept]);

    // With a real budget, the same address space then completes and publishes.
    let mut fb = kayfabe_fwd::IsolateFb::new(&mut worker);
    let results = run_pt_sweep(fmt, &mut fb, &again.tasks, PT_SWEEP_BUDGET);
    let out = with_gpu(&mut gpu, |g| commit_pt_sweep(fmt, only_proc(g), &results));
    assert_eq!(out.sweeps_run, 1);
    assert_eq!(out.bound, 1);
    let third = with_gpu(&mut gpu, |g| plan_pt_sweep(only_proc(g)));
    assert_eq!(third.tasks, vec![], "and the trunc flag is CLEARED, not sticky");

    // ★★★★★ NOW the trigger stands ALONE. Dirty the address space, truncate the re-sweep, and
    // the next plan must name `Truncated` — with `sweeps > 0`, so `NeverSwept` cannot be what
    // fired. This is the arm that shows the C's `m2_gr_pt_trunc` is a live trigger and not a
    // flag that happens to co-occur with a fresh address space.
    with_gpu(&mut gpu, |g| {
        only_proc(g)
            .vases
            .get_mut(&(GPU, A_PDB))
            .expect("the vas")
            .pt_pages
            .insert(PT_SMALL);
    });
    let dirty = with_gpu(&mut gpu, |g| plan_pt_sweep(only_proc(g)));
    assert_eq!(dirty.reasons, vec![SweepReason::Dirty]);
    let mut fb = kayfabe_fwd::IsolateFb::new(&mut worker);
    let results = run_pt_sweep(fmt, &mut fb, &dirty.tasks, 1);
    let out = with_gpu(&mut gpu, |g| commit_pt_sweep(fmt, only_proc(g), &results));
    assert_eq!(out.sweeps_truncated, 1);
    let fourth = with_gpu(&mut gpu, |g| plan_pt_sweep(only_proc(g)));
    assert_eq!(
        fourth.reasons,
        vec![SweepReason::Truncated],
        "★★★★★ and here it is the TRUNCATION doing the re-arming, on an address space that \
         has completed a sweep before"
    );
    // ⊘ And the bindings the completed sweep published are NOT torn down by the truncated one:
    // a truncated walk is no picture, and no picture must not be read as an empty one.
    assert_eq!(out.unbound, 0);
}

/// ★★★ **A retired page loses its sweep admission**, for the same reason it loses its witness:
/// `_mmuWalkPdeRelease` frees the backing store right after clearing the parent, so the next
/// thing written there is not a page table at all. A surviving admission would let those bytes
/// bind as if a root descent had just reached them.
#[test]
fn a_page_that_falls_out_of_the_tree_loses_its_sweep_admission() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let (mut gpu, factory, rec) = fixture();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);
    write_a_whole_unwitnessed_tree(&mut worker, &rec, vas);

    let plan = with_gpu(&mut gpu, |g| plan_pt_sweep(only_proc(g)));
    let mut fb = kayfabe_fwd::IsolateFb::new(&mut worker);
    let results = run_pt_sweep(fmt, &mut fb, &plan.tasks, PT_SWEEP_BUDGET);
    let out = with_gpu(&mut gpu, |g| commit_pt_sweep(fmt, only_proc(g), &results));
    assert_eq!(out.bound, 1);
    assert!(with_gpu(&mut gpu, |g| {
        only_proc(g)
            .vases
            .get(&(GPU, A_PDB))
            .expect("the vas")
            .reach
            .is_swept(PT_SMALL)
    }));

    // The guest tears the subtree down by clearing ONE parent entry — hole 3's shape, in which
    // no leaf is ever written invalid.
    write_fabricated(
        &mut worker,
        &rec,
        vas,
        PD_DUAL,
        &page_at(fmt, MOCK_DUAL_LEVEL, &[]),
    );
    with_gpu(&mut gpu, |g| {
        only_proc(g)
            .vases
            .get_mut(&(GPU, A_PDB))
            .expect("the vas")
            .pt_pages
            .insert(PD_DUAL);
    });
    let re = with_gpu(&mut gpu, |g| plan_pt_sweep(only_proc(g)));
    assert_eq!(re.reasons, vec![SweepReason::Dirty]);
    let mut fb = kayfabe_fwd::IsolateFb::new(&mut worker);
    let results = run_pt_sweep(fmt, &mut fb, &re.tasks, PT_SWEEP_BUDGET);
    let out = with_gpu(&mut gpu, |g| commit_pt_sweep(fmt, only_proc(g), &results));

    assert!(
        out.retired.contains(&PT_SMALL),
        "the leaf table fell out of the tree: {:?}",
        out.retired
    );
    assert_eq!(out.unbound, 1, "and the mapping left the table");
    assert!(
        !with_gpu(&mut gpu, |g| {
            only_proc(g)
                .vases
                .get(&(GPU, A_PDB))
                .expect("the vas")
                .reach
                .is_swept(PT_SMALL)
        }),
        "★ and the sweep admission went with it"
    );
}

// =====================================================================================
// 3. Arm 2.1's instrument — the question a bind census cannot answer
// =====================================================================================

/// ★★★★★ **`leaf_covering` answers about THE GUEST, and it can say both things.**
///
/// The census exists to separate *"the guest describes this VA and our mirror missed it"* from
/// *"the guest never described it"* — two readings of one `Xid` that demand opposite work. A
/// predicate that could only ever answer `None` would look identical on the run that matters,
/// so both answers are asserted from one swept tree.
#[test]
fn the_guest_leaf_census_answers_present_and_absent_from_the_same_swept_tree() {
    let arch = MockArch::new();
    let fmt = arch.mmu();
    let (mut gpu, factory, rec) = fixture();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);
    write_a_whole_unwitnessed_tree(&mut worker, &rec, vas);
    let plan = with_gpu(&mut gpu, |g| plan_pt_sweep(only_proc(g)));
    let mut fb = kayfabe_fwd::IsolateFb::new(&mut worker);
    let results = run_pt_sweep(fmt, &mut fb, &plan.tasks, PT_SWEEP_BUDGET);
    with_gpu(&mut gpu, |g| commit_pt_sweep(fmt, only_proc(g), &results));

    let small = fmt.level_shift(small_leaf_level()).expect("small");
    let mapped = GpuVa(9 * (1u64 << small.shift));
    let (hit, miss, runs) = with_gpu(&mut gpu, |g| {
        let r = &only_proc(g)
            .vases
            .get(&(GPU, A_PDB))
            .expect("the vas")
            .reach;
        (
            r.leaf_covering(mapped),
            r.leaf_covering(GpuVa(0xDEAD_0000_0000)),
            r.reachable_ranges(),
        )
    });
    let hit = hit.expect("★ LEAF-PRESENT: the guest's own tables describe this address");
    assert_eq!(hit.phys, 0xF000_0000);
    assert!(
        miss.is_none(),
        "★ LEAF-ABSENT: and the same predicate says so about an address the guest never \
         described — without which a `None` would be unreadable"
    );
    assert_eq!(
        runs.len(),
        1,
        "one contiguous run, coalesced: {runs:?} — a per-leaf dump is a log nobody reads"
    );
    assert_eq!(runs[0].0, mapped.0);
}
