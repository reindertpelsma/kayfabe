//! # ★★★★★ **THE SECOND CROSSING** — a blank host **vidmem** object, mapped FIXED at the
//! guest's own VA, over a framebuffer leaf the guest's own page tables already bind.
//!
//! `C: docs/design/mode2_fb_crossing_question.md` §5 (GEN-2), settled 2026-06-04 and built
//! twice in the C artifact. The first crossing (`guest_ram_pin.rs`) moved the guest's own
//! **sysmem** pages. `[measured 2026-08-10, boot `w227c_537894e_census2`]` **3 of the 4
//! bound operands of `cuCtxCreate`'s GR submission are in the emulated framebuffer** — so
//! this crossing is the majority of that census, not its remainder.
//!
//! # ★★★ What this suite can and cannot judge, stated first
//!
//! Mock-driven and GPU-free. It judges the **chain** — order, idempotence, placement
//! enforcement, refusal names, unwinding, and above all the two-source agreement check —
//! and it judges **nothing at all** about whether RM accepts an `NV01_MEMORY_LOCAL_USER`
//! allocation or places a fixed map over one. ⊘ That is real ioctls on a real driver and it
//! is measured on the bench, in a boot log, and nowhere else.
//!
//! # ★★★★★ The property this file exists for, and it is not the happy path
//!
//! The leaf's `(va, len, phys)` has **two** sources: the guest's own page-table walk, and
//! this proc's address table. [`the_walk_and_the_table_disagreeing_is_refused_by_name`] is
//! the assertion that when they disagree, **neither is used** — no host object is
//! allocated, nothing is bound, and the refusal carries both numbers. Preferring one
//! reading is this campaign's most expensive recurring mistake, and the fix has to be a
//! test rather than a comment, because a comment cannot fail.

use std::sync::Arc;

use kayfabe_arch::Aperture;
use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_fwd::FwdFault;
use kayfabe_mmu::{Binding, HostBacking};
use kayfabe_mocks::watchdog;
use kayfabe_mocks::{MockArch, MockIsolateFactory, RmVerb, SharedRecorder};
use kayfabe_rt::device::{LockMode, SharedDevice};
use kayfabe_tests::{Guarded, Scenario, identical_handles};

const GPU: GpuId = GpuId::ZERO;
const CLIENT: HClient = HClient(0xA0);
const PDB: Pdb = Pdb(0x3400_0000);
const GR: VChid = VChid(0x100);
const CE: VChid = VChid(0x200);
const MEM: HObject = HObject(0x6000_0000);

/// The leaf's guest VA. ★ `[measured 2026-08-10, boot `w227c_537894e_census2`]` —
/// `SET_TEX_SAMPLER_POOL`'s operand, so a reader can lay this file beside the boot log.
const LEAF_VA: GpuVa = GpuVa(0x1_0002_0000_0000);
/// Its framebuffer-physical base, from the same census row (`Framebuffer { phys: 8388608 }`).
const LEAF_PHYS: u64 = 0x80_0000;
/// The leaf's length. ⊘ Not a constant this port chose — the walk's own page size for that
/// entry. 2 MiB is what a PD0 leaf on this chip is, and the three census rows sit exactly
/// 2 MiB apart in the framebuffer (`0x400000`, `0x600000`, `0x800000`), which is the
/// evidence that they are consecutive leaves of that size.
const LEAF_LEN: u64 = 0x20_0000;

/// One guest proc on GPU0.
fn device() -> (
    Guarded<Arc<SharedDevice>>,
    kayfabe_core::ProcId,
    SharedRecorder,
) {
    let (factory, recorder) = MockIsolateFactory::with_pool_size(2);
    let gpa = GpaSpace::new(0x10_0000_0000..0x1000_0000_0000, 0x10_0000_0000);
    let mut gpu = Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa).expect("realizes");
    let mut s = Scenario::new();
    s.compute_process_on_gpu(CLIENT, PDB, identical_handles(GR.0, CE.0), None);
    s.memory(CLIENT, HObject(0x5c00_0001), MEM, 0x9_0000_0000);
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies");
    }
    kayfabe_tests::guest_schedules_every_channel(&mut gpu);
    let pid = gpu.spine.by_pdb[&(GPU, PDB)];
    (
        Guarded::new(
            "fb_leaf_backing::device",
            Arc::new(SharedDevice::new(gpu, LockMode::Sharded)),
            recorder.clone(),
        ),
        pid,
        recorder,
    )
}

/// Declare, in this proc's address table, that the guest's own page tables bind this leaf.
///
/// ⊘ The fixture stands in for the **populate pass**, not for the resolver — the production
/// caller reads exactly this table.
fn guest_binds(
    device: &SharedDevice,
    pid: kayfabe_core::ProcId,
    va: GpuVa,
    len: u64,
    phys: u64,
    aperture: Aperture,
    host: Option<HostBacking>,
) {
    device
        .with_proc_mut(pid, |p| {
            let vas = p.vases.get_mut(&(GPU, PDB)).expect("the compute VAS");
            vas.table
                .bind(
                    PDB,
                    va,
                    len,
                    Binding {
                        phys,
                        aperture,
                        host,
                    },
                )
                .expect("the fixture's own bind is well-formed");
        })
        .expect("the proc is live");
}

fn verbs(rec: &SharedRecorder) -> Vec<&'static str> {
    rec.lock()
        .expect("recorder")
        .log
        .iter()
        .map(|(_, v)| match v {
            RmVerb::AllocVaSpace { .. } => "vas",
            RmVerb::AllocSysmem { .. } => "sysmem",
            RmVerb::AllocVidmem { .. } => "vidmem",
            RmVerb::MapGpuVa { .. } => "map_gpu_va",
            RmVerb::UnmapGpuVa { .. } => "unmap_gpu_va",
            RmVerb::Free { .. } => "free",
            _ => "other",
        })
        .collect()
}

/// The binding the table holds for `LEAF_VA`, if any — `(start, len, binding)`.
fn tabled(device: &SharedDevice, pid: kayfabe_core::ProcId) -> Option<(u64, u64, Binding)> {
    device
        .with_proc_mut(pid, |p| {
            p.vases
                .get(&(GPU, PDB))
                .expect("the compute VAS")
                .table
                .binding_at(LEAF_VA)
        })
        .expect("the proc is live")
}

// ---------------------------------------------------------------------------------
// 1 — ★★★★★ THE CHAIN
// ---------------------------------------------------------------------------------

/// ★★★★★ **The whole rung, in one assertion set.** Backing a framebuffer leaf allocates
/// host **vidmem** and maps it at the **guest's own VA**, and it allocates **no host sysmem
/// at all**.
///
/// ★ That last clause is what distinguishes this chain from
/// [`kayfabe_fwd::publish_backing`], and it is asserted rather than described. `publish`
/// mints sysmem with `MAPPING_NO_MAP`; a leaf backed that way maps fine, passes every check
/// and can never become the CPU-side half of the double mapping. If a future edit folded the
/// two chains together, `sysmem` would appear in this list.
#[test]
fn backing_a_framebuffer_leaf_allocates_vidmem_and_places_it_at_the_guests_own_va() {
    let _wd = watchdog(
        "fb_leaf_backing::the_chain",
        std::time::Duration::from_secs(60),
    );
    let (device, pid, rec) = device();
    guest_binds(
        &device,
        pid,
        LEAF_VA,
        LEAF_LEN,
        LEAF_PHYS,
        Aperture::Vidmem,
        None,
    );
    let backed = device
        .back_fb_leaf(GPU, PDB, LEAF_VA, LEAF_LEN, LEAF_PHYS)
        .expect("the leaf backs");

    assert!(!backed.already, "the first call did the work");
    assert_eq!(
        backed.host_va, LEAF_VA.0,
        "address identity: the object is placed at the GUEST's VA, not wherever RM chose"
    );
    let v = verbs(&rec);
    assert!(
        v.contains(&"vidmem"),
        "the object must come out of DEVICE-LOCAL memory: {v:?}"
    );
    assert!(
        !v.contains(&"sysmem"),
        "⊘ this chain must never mint host sysmem — that is `publish_backing`, and its \
         `MAPPING_NO_MAP` object can never be double-mapped: {v:?}"
    );
    assert_eq!(
        v.iter().filter(|x| **x == "map_gpu_va").count(),
        1,
        "exactly one fixed map: {v:?}"
    );

    // ★ The table now names the host object, and NOTHING ELSE about the leaf changed.
    let (start, len, b) = tabled(&device, pid).expect("still bound");
    assert_eq!((start, len), (LEAF_VA.0, LEAF_LEN), "same range");
    assert_eq!(b.phys, LEAF_PHYS, "the guest's own framebuffer address");
    assert_eq!(b.aperture, Aperture::Vidmem, "still vidmem");
    let host = b.host.expect("the binding now carries its materialization");
    assert_eq!(host.host_va(), LEAF_VA.0);
    assert_eq!(host.memory(), backed.memory);
    assert!(
        host.frees_object(),
        "a fresh object per leaf ⇒ the binding IS the object and its release frees it"
    );
}

/// ★★★ A leaf the address table does **not** bind yet is still backable: the walk is the
/// authority on the guest's own page tables, and the table is a mirror that may not have
/// been populated for this range.
#[test]
fn a_leaf_the_table_has_never_seen_is_backed_and_bound() {
    let _wd = watchdog(
        "fb_leaf_backing::unbound",
        std::time::Duration::from_secs(60),
    );
    let (device, pid, _rec) = device();
    assert!(tabled(&device, pid).is_none(), "nothing bound to start");

    let backed = device
        .back_fb_leaf(GPU, PDB, LEAF_VA, LEAF_LEN, LEAF_PHYS)
        .expect("the leaf backs");
    let (_, _, b) = tabled(&device, pid).expect("now bound");
    assert_eq!(b.phys, LEAF_PHYS);
    assert_eq!(b.aperture, Aperture::Vidmem);
    assert_eq!(
        b.host.expect("materialized").memory(),
        backed.memory,
        "the bind names the object the publish returned"
    );
}

/// ★★★ **Idempotence, and it must cost NO host verb.** A doorbell repeats; a caller that
/// re-asked would demand the same host GPU VA twice, and RM answers a colliding fixed map
/// with `0x51 NV_ERR_NO_MEMORY` — a status that ⊘ cannot be told apart from real
/// exhaustion (the C's own R2). So the replay has to resolve entirely in the plan phase.
#[test]
fn a_second_ask_replays_and_issues_no_host_verb_at_all() {
    let _wd = watchdog(
        "fb_leaf_backing::replay",
        std::time::Duration::from_secs(60),
    );
    let (device, pid, rec) = device();
    guest_binds(
        &device,
        pid,
        LEAF_VA,
        LEAF_LEN,
        LEAF_PHYS,
        Aperture::Vidmem,
        None,
    );
    let first = device
        .back_fb_leaf(GPU, PDB, LEAF_VA, LEAF_LEN, LEAF_PHYS)
        .expect("backs");
    let after_first = verbs(&rec).len();

    let second = device
        .back_fb_leaf(GPU, PDB, LEAF_VA, LEAF_LEN, LEAF_PHYS)
        .expect("replays");
    assert!(second.already, "the second call must report the replay");
    assert_eq!(
        (second.host_va, second.memory),
        (first.host_va, first.memory),
        "the replay reports the SAME object, never a fresh one"
    );
    assert_eq!(
        verbs(&rec).len(),
        after_first,
        "⊘ a replay must issue not one host verb: {:?}",
        verbs(&rec)
    );
}

// ---------------------------------------------------------------------------------
// 2 — ★★★★★ THE TWO SOURCES
// ---------------------------------------------------------------------------------

/// ★★★★★ **The point of this file.** The guest's page-table walk says the leaf is at
/// framebuffer address `LEAF_PHYS`; the address table says something else. **Neither may be
/// used**, nothing is allocated, and the refusal carries both numbers.
///
/// ⊘ A version of this chain that preferred the walk would put a real host GPU object under
/// an address the guest reaches through the other reading. One that preferred the table
/// would back a leaf the guest's own page tables do not have. Both are silent.
#[test]
fn the_walk_and_the_table_disagreeing_is_refused_by_name() {
    let _wd = watchdog(
        "fb_leaf_backing::disagree",
        std::time::Duration::from_secs(60),
    );
    let (device, pid, rec) = device();
    // The table's reading: a DIFFERENT framebuffer page.
    guest_binds(
        &device,
        pid,
        LEAF_VA,
        LEAF_LEN,
        LEAF_PHYS + 0x20_0000,
        Aperture::Vidmem,
        None,
    );
    let e = device
        .back_fb_leaf(GPU, PDB, LEAF_VA, LEAF_LEN, LEAF_PHYS)
        .expect_err("the disagreement is refused");
    match e {
        FwdFault::FbLeafDisagrees { va, walked, tabled } => {
            assert_eq!(va, LEAF_VA);
            assert_eq!(walked, (LEAF_PHYS, Aperture::Vidmem));
            assert_eq!(tabled, (LEAF_PHYS + 0x20_0000, Aperture::Vidmem));
        }
        other => panic!("must refuse by name, got {other:?}"),
    }
    assert!(
        verbs(&rec).is_empty(),
        "⊘ refused in the PLAN phase, before a single host object exists: {:?}",
        verbs(&rec)
    );
    assert!(
        tabled(&device, pid).expect("still bound").2.host.is_none(),
        "nothing was materialized"
    );
}

/// ★★★ The same refusal on the **aperture** term alone. A range the table calls sysmem is
/// not a framebuffer leaf, whatever the walk said, and backing it with vidmem would put the
/// two readings permanently at odds.
#[test]
fn a_table_that_calls_the_leaf_sysmem_is_refused_on_the_aperture_alone() {
    let _wd = watchdog(
        "fb_leaf_backing::aperture",
        std::time::Duration::from_secs(60),
    );
    let (device, pid, rec) = device();
    guest_binds(
        &device,
        pid,
        LEAF_VA,
        LEAF_LEN,
        LEAF_PHYS,
        Aperture::SysmemCoherent,
        None,
    );
    let e = device
        .back_fb_leaf(GPU, PDB, LEAF_VA, LEAF_LEN, LEAF_PHYS)
        .expect_err("refused");
    assert!(
        matches!(
            e,
            FwdFault::FbLeafDisagrees {
                tabled: (_, Aperture::SysmemCoherent),
                ..
            }
        ),
        "must name the aperture disagreement, got {e:?}"
    );
    assert!(verbs(&rec).is_empty(), "nothing was built");
}

/// ★★★ A table binding over a **different range** than the leaf the walk found. Backing it
/// would either overhang a neighbour or leave a hole, and which of the two is not something
/// this site can decide — so it decides neither.
#[test]
fn a_table_range_that_is_not_the_leaf_is_refused_by_extent() {
    let _wd = watchdog(
        "fb_leaf_backing::extent",
        std::time::Duration::from_secs(60),
    );
    let (device, pid, rec) = device();
    guest_binds(
        &device,
        pid,
        LEAF_VA,
        LEAF_LEN * 2,
        LEAF_PHYS,
        Aperture::Vidmem,
        None,
    );
    let e = device
        .back_fb_leaf(GPU, PDB, LEAF_VA, LEAF_LEN, LEAF_PHYS)
        .expect_err("refused");
    match e {
        FwdFault::FbLeafExtent { va, len, tabled } => {
            assert_eq!((va, len), (LEAF_VA, LEAF_LEN));
            assert_eq!(tabled, (LEAF_VA.0, LEAF_LEN * 2));
        }
        other => panic!("must refuse by extent, got {other:?}"),
    }
    assert!(verbs(&rec).is_empty(), "nothing was built");
}

// ---------------------------------------------------------------------------------
// 3 — ★★ GRANULARITY, refused rather than rounded
// ---------------------------------------------------------------------------------

/// ★★ A leaf smaller than RM's 64 KiB fixed-map granule is refused **by name**.
///
/// ⊘ The C rounds up instead and registers the rounded range (`C: :8242-8243`), so its
/// object claims up to 60 KiB of guest framebuffer address space **past the end of the
/// leaf** — an overhang the establishment copy never fills and the local shadow can no
/// longer answer for. This port makes that unrepresentable rather than inheriting it.
#[test]
fn a_leaf_below_the_fixed_map_granule_is_refused_and_not_rounded_up() {
    let _wd = watchdog("fb_leaf_backing::gran", std::time::Duration::from_secs(60));
    let (device, _pid, rec) = device();
    let e = device
        .back_fb_leaf(GPU, PDB, LEAF_VA, 0x1000, LEAF_PHYS)
        .expect_err("refused");
    assert!(
        matches!(e, FwdFault::FbLeafGranularity { len: 0x1000, .. }),
        "must name the granularity, got {e:?}"
    );
    assert!(verbs(&rec).is_empty(), "nothing was built");
}

/// ★ And on the address term: a leaf base that is not granule-aligned cannot be placed
/// exactly either.
#[test]
fn a_leaf_base_that_is_not_granule_aligned_is_refused() {
    let _wd = watchdog("fb_leaf_backing::align", std::time::Duration::from_secs(60));
    let (device, _pid, _rec) = device();
    let e = device
        .back_fb_leaf(GPU, PDB, GpuVa(LEAF_VA.0 + 0x1000), LEAF_LEN, LEAF_PHYS)
        .expect_err("refused");
    assert!(matches!(e, FwdFault::FbLeafGranularity { .. }), "got {e:?}");
}

// ---------------------------------------------------------------------------------
// 4 — ★★★ THE SYSTEM-PLANE RULE, not relaxed here either
// ---------------------------------------------------------------------------------

/// ★★★ `l1_concurrency.md` §12.26 — the system proc has no data plane, so it may hold no
/// host state whose reclaim has no defined point. A framebuffer object is host state.
///
/// ⊘ This is the same wall `pin_ring_guest_ram` hit on `w226c`, and it is asserted here so
/// that a future edit which relaxes it has to delete a test rather than a comment.
#[test]
fn the_system_proc_may_not_back_a_framebuffer_leaf() {
    let _wd = watchdog(
        "fb_leaf_backing::system",
        std::time::Duration::from_secs(60),
    );
    let (device, _pid, rec) = device();
    let sys = kayfabe_core::gpu::Gpu::SYSTEM_PROC;
    let refused = device.with_proc_mut(sys, |p| {
        kayfabe_fwd::plan_back_fb_leaf(p, GPU, PDB, LEAF_VA, LEAF_LEN, LEAF_PHYS)
            .expect_err("the system proc is refused")
    });
    assert!(
        matches!(refused, Some(FwdFault::SystemDataPlane)),
        "must refuse by name, got {refused:?}"
    );
    assert!(verbs(&rec).is_empty(), "nothing was built");
}
