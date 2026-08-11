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
use kayfabe_fwd::{FbLeafBacking, FwdFault};
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
/// The same three numbers as one value — what [`SharedDevice::adopt_joined_fb_leaf`] takes.
const LEAF: kayfabe_fwd::FbLeafRange = kayfabe_fwd::FbLeafRange {
    va: LEAF_VA,
    len: LEAF_LEN,
    phys: LEAF_PHYS,
};

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
                    match host {
                        // ★ The fixture declares the kind, exactly as production does: a
                        // host object means kind 3, its absence means the guest's own
                        // declaration (kind 2 for `Vidmem`, kind 4 for sysmem).
                        Some(h) => Binding::real_gpu_memory(phys, aperture, h)
                            .expect("the fixture's host backing is kind 3"),
                        None => Binding::declared_by_guest(phys, aperture)
                            .expect("the fixture declares a kind the guest can declare"),
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

/// ★★★★★ **RULING 3 (owner, 2026-08-11): THIS CROSSING IS REFUSED, and the guest's own row
/// survives the refusal.**
///
/// > *"no fake FB ever can be mapped to a real GPU VA of an isolate except the scratchpad."*
///
/// # ⊘ What this test asserted BEFORE, and why the inversion is the rung
///
/// It asserted the crossing SUCCEEDS: *"backing a framebuffer leaf allocates host vidmem and
/// maps it at the guest's own VA"*, and that the binding then *"carries its
/// materialization"*. `[measured 2026-08-11, w228]` that object is `placed_as_asked=true`
/// **and blank** — the guest's bytes stay in `kayfabe_device::SparseFb` and the guest goes on
/// reading and writing them through BAR1/BAR2. The chain minted a SECOND memory at the
/// guest's own address, which is the owner's forbidden #2 and the execution blocker this
/// rung exists for.
///
/// ⇒ [`kayfabe_mmu::Binding::real_gpu_memory`] refuses the state, so `commit_back_fb_leaf`
/// cannot bind it and refuses by name instead.
///
/// # ★★ The three things asserted, and the third is the one that is easy to get wrong
///
/// 1. **Refused by name**, carrying the address plane's own answer — not a generic error.
/// 2. **Nothing is adopted**: the table does not name a host object for this leaf.
/// 3. ★★★ **The guest's own row is UNTOUCHED.** Dropping it would leave the range with *no
///    row at all*, and an absent row is `Representability::Untracked`, which routes to the
///    **real host GPU**. ⊘ The two derived defaults point opposite ways, so a refusal that
///    also unbinds would hand the range to hardware — a worse outcome than the state it
///    refused. This arm is what stops that.
#[test]
fn backing_a_framebuffer_leaf_is_refused_by_name_and_the_guests_own_row_survives() {
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
    let refused = device
        .back_fb_leaf(
            GPU,
            PDB,
            LEAF_VA,
            LEAF_LEN,
            LEAF_PHYS,
            FbLeafBacking::Vidmem,
        )
        .expect_err("★ ruling 3: a fake-FB region may not be mapped to a real GPU VA");
    assert_eq!(
        refused,
        kayfabe_rt::FwdFault::RegionKindRefused {
            va: LEAF_VA,
            fault: kayfabe_mmu::RegionKindFault::FakeFbAtRealGpuVa {
                aperture: Aperture::Vidmem
            },
        },
        "refused BY NAME, carrying the address plane's own answer — a generic error here \
         would leave a reader unable to tell ruling 3 from an allocation failure"
    );

    // ★★★ The guest's own row is exactly as it was: kind 2, no host object.
    let (start, len, b) = tabled(&device, pid).expect(
        "★★★ THE ROW MUST SURVIVE: no row at all is `Untracked`, which routes to the REAL \
         HOST GPU. A refusal that unbinds is worse than the state it refused.",
    );
    assert_eq!((start, len), (LEAF_VA.0, LEAF_LEN), "same range");
    assert_eq!(b.phys(), LEAF_PHYS, "the guest's own framebuffer address");
    assert_eq!(b.aperture(), Aperture::Vidmem, "still vidmem");
    assert_eq!(
        b.kind(),
        kayfabe_mmu::RegionKind::FakeFramebuffer,
        "and it is still DECLARED fake framebuffer — the kind the guest's own leaf PTE named"
    );
    assert!(
        b.host().is_none(),
        "⊘ nothing was adopted: the binding names no host object"
    );

    // ⚠ The host verbs still RAN — the refusal is at the commit, so the execute phase had
    // already allocated. Everything it allocated goes back as orphans; nothing is bound.
    // ⊘ Moving the refusal into `plan_back_fb_leaf` so nothing is allocated at all is a
    // separate change: it makes `commit_back_fb_leaf`'s fresh-publish arm unreachable.
    let v = verbs(&rec);
    assert!(
        !v.contains(&"sysmem"),
        "⊘ this chain must never mint host sysmem — that is `publish_backing`: {v:?}"
    );
}

/// ★★★★★ **THE HOLE (A) DOES NOT CLOSE, pinned as a test rather than left to a reader.**
///
/// ⊘ This test asserted that a leaf the address table has never seen *"is backed and
/// bound"* — the crossing bound a row where none existed. Ruling 3 refuses the crossing, and
/// the commit deliberately does not bind anything on its refusal path (the table is the
/// walker's to populate, not the publish chain's).
///
/// ⇒ **A leaf with no prior row is left with no row**, and an absent row is
/// [`kayfabe_fwd::Representability::Untracked`], which routes to the **real host GPU**.
///
/// ★★ That is the second derived default, and (A) does not remove it: deciding the kind at
/// bind fixes what a BOUND range means and says nothing about a range nobody bound. The
/// thing that closes it is the walker's forward-populate running over this leaf first —
/// which is exactly what the sibling test above has (its `guest_binds` is that populate) and
/// what this one deliberately does not.
///
/// ⚠ This test exists to make the gap **loud and located**. If a future rung closes it, this
/// test is where the change will be seen.
#[test]
fn a_leaf_the_table_has_never_seen_is_refused_and_left_untracked() {
    let _wd = watchdog(
        "fb_leaf_backing::unbound",
        std::time::Duration::from_secs(60),
    );
    let (device, pid, _rec) = device();
    assert!(tabled(&device, pid).is_none(), "nothing bound to start");

    let refused = device
        .back_fb_leaf(
            GPU,
            PDB,
            LEAF_VA,
            LEAF_LEN,
            LEAF_PHYS,
            FbLeafBacking::Vidmem,
        )
        .expect_err("ruling 3 refuses the crossing whether or not a row exists");
    assert_eq!(
        refused,
        kayfabe_rt::FwdFault::RegionKindRefused {
            va: LEAF_VA,
            fault: kayfabe_mmu::RegionKindFault::FakeFbAtRealGpuVa {
                aperture: Aperture::Vidmem
            },
        }
    );
    assert!(
        tabled(&device, pid).is_none(),
        "⚠ THE GAP: still unbound, i.e. `Untracked`, i.e. the host GPU arm. The publish \
         chain does not populate the table — the walker does — so refusing here cannot \
         invent the guest's declaration."
    );
}

/// ★★★ **The refusal is STABLE, and it is the same refusal every time.**
///
/// ⊘ This test asserted idempotence of the *success* path — a second ask replayed the first
/// object and cost no host verb, because a colliding fixed map is answered `0x51
/// NV_ERR_NO_MEMORY`, a status that cannot be told apart from real exhaustion (the C's own
/// R2). Ruling 3 removes the success path, so what has to be pinned instead is that the
/// refusal does not *drift*: a chain that refused once and succeeded on the retry would be
/// worse than one that never refused, because the hazard would be intermittent.
#[test]
fn a_second_ask_is_refused_identically_and_never_succeeds_on_retry() {
    let _wd = watchdog(
        "fb_leaf_backing::replay",
        std::time::Duration::from_secs(60),
    );
    let (device, pid, _rec) = device();
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
        .back_fb_leaf(
            GPU,
            PDB,
            LEAF_VA,
            LEAF_LEN,
            LEAF_PHYS,
            FbLeafBacking::Vidmem,
        )
        .expect_err("refused");
    let second = device
        .back_fb_leaf(
            GPU,
            PDB,
            LEAF_VA,
            LEAF_LEN,
            LEAF_PHYS,
            FbLeafBacking::Vidmem,
        )
        .expect_err("★ and refused AGAIN — a hazard that appears only on the retry is worse");
    assert_eq!(first, second, "the same refusal, by the same name");
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
        .back_fb_leaf(
            GPU,
            PDB,
            LEAF_VA,
            LEAF_LEN,
            LEAF_PHYS,
            FbLeafBacking::Vidmem,
        )
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
        tabled(&device, pid)
            .expect("still bound")
            .2
            .host()
            .is_none(),
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
        .back_fb_leaf(
            GPU,
            PDB,
            LEAF_VA,
            LEAF_LEN,
            LEAF_PHYS,
            FbLeafBacking::Vidmem,
        )
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
        .back_fb_leaf(
            GPU,
            PDB,
            LEAF_VA,
            LEAF_LEN,
            LEAF_PHYS,
            FbLeafBacking::Vidmem,
        )
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
        .back_fb_leaf(GPU, PDB, LEAF_VA, 0x1000, LEAF_PHYS, FbLeafBacking::Vidmem)
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
        .back_fb_leaf(
            GPU,
            PDB,
            GpuVa(LEAF_VA.0 + 0x1000),
            LEAF_LEN,
            LEAF_PHYS,
            FbLeafBacking::Vidmem,
        )
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
        kayfabe_fwd::plan_back_fb_leaf(
            p,
            GPU,
            PDB,
            LEAF_VA,
            LEAF_LEN,
            LEAF_PHYS,
            FbLeafBacking::Vidmem,
        )
        .expect_err("the system proc is refused")
    });
    assert!(
        matches!(refused, Some(FwdFault::SystemDataPlane)),
        "must refuse by name, got {refused:?}"
    );
    assert!(verbs(&rec).is_empty(), "nothing was built");
}

/// ★★★★★ **§5.11 — THE CHAIN THAT REPLACES THE ONE ABOVE.** The same leaf, joined: the
/// object is **not** vidmem, and the reply carries a **backing the VMM can map**.
///
/// ⊘ The two assertions are the whole difference and they are asserted as a pair. A join
/// that minted vidmem would be `w228` wearing a new name; a join that placed the object
/// correctly and handed back no backing would be `w228` exactly — the leaf would be
/// materialized on the host and the guest would still be reading the emulator's own copy,
/// silently, which is the state this rung exists to end.
///
/// ⚠ `[SOURCED, not MEASURED]` — this runs against `MockRmBackend`, which mints no memory.
/// It proves the *plumbing*: that the plan emits the join chain, that the commit adopts it,
/// and that a token comes back. It says **nothing** about whether the two views hold the same
/// bytes; that is `fb_cpu_view.md` §3's hardware measurement and this rung's boot.
#[test]
fn joining_a_framebuffer_leaf_mints_no_vidmem_and_hands_a_backing_to_the_vmm() {
    let _wd = watchdog("fb_leaf_backing::join", std::time::Duration::from_secs(60));
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
    let joined = device
        .back_fb_leaf(
            GPU,
            PDB,
            LEAF_VA,
            LEAF_LEN,
            LEAF_PHYS,
            FbLeafBacking::Joined,
        )
        .expect("the leaf joins");

    assert!(!joined.already, "the first call did the work");
    assert_eq!(
        joined.host_va, LEAF_VA.0,
        "address identity: placed at the GUEST's VA, exactly as the vidmem chain is"
    );
    let backing = joined
        .backing
        .expect("★ the join's whole point: a backing the VMM can map");
    assert_eq!(
        backing.len, LEAF_LEN,
        "the backing covers the whole leaf — a short one would leave part of the leaf in          two memories, which is the defect with a smaller blast radius rather than a fix"
    );

    let v = verbs(&rec);
    assert!(
        !v.contains(&"vidmem"),
        "⊘ the joined chain must mint NO device-local memory: card memory is exactly what          cannot carry a guest-reachable CPU view. {v:?}"
    );
    assert!(
        !v.contains(&"sysmem"),
        "⊘ nor `publish_backing`'s MAPPING_NO_MAP sysmem: {v:?}"
    );

    // ★★ The join was recorded against the leaf's own FRAMEBUFFER address, not only its VA.
    // A join placed at the right guest VA but standing for the wrong framebuffer range would
    // pass every assertion above and answer the instrument about someone else's bytes.
    let phys_seen: Vec<u64> = rec
        .lock()
        .unwrap()
        .log
        .iter()
        .filter_map(|(_, verb)| match verb {
            RmVerb::JoinFbLeaf { phys, at, .. } if *at == LEAF_VA => Some(*phys),
            _ => None,
        })
        .collect();
    assert_eq!(
        phys_seen,
        vec![LEAF_PHYS],
        "exactly one join, carrying the walk's own framebuffer address"
    );

    // ★★★★★ **AND NOTHING IS BOUND YET.** The join call adopts the HOST facts and stops;
    // the row is the guest's own un-backed declaration until the shell has installed the
    // view. ⊘ This is the ordering fix asserted, not described: `fb-join` bound here, three
    // steps before the guest's window was re-pointed.
    let before = tabled(&device, pid).expect("the guest's own row survives");
    assert!(
        before.2.host().is_none(),
        "⊘ the leaf must NOT be host-backed before the install — a row declaring \
         `JoinsGuestWindow` over a window nobody has re-pointed is `w228`'s two memories \
         wearing the one word that says they are one. Got {before:?}"
    );
    assert_eq!(
        before.2.kind(),
        kayfabe_mmu::RegionKind::FakeFramebuffer,
        "★ and it is still kind 2, because it still is: the emulated framebuffer is still \
         this range's store"
    );

    // ---- THE SHELL INSTALLS THE VIEW HERE. This suite has no `RegPlane`, so the install is
    // the thing it cannot perform; what it CAN judge is that the bind is a separate call the
    // shell has to reach, which is the whole content of the ordering fix.
    device
        .adopt_joined_fb_leaf(GPU, PDB, LEAF, &joined)
        .expect("the adopt binds");

    // ★ And now the address table records it exactly as the vidmem chain would — same range,
    // same aperture, same materialization. ⊘ The two chains differ in what backs the leaf and
    // in WHEN the row appears, NOT in what the core believes about it afterwards.
    let (start, len, b) = tabled(&device, pid).expect("still bound");
    assert_eq!((start, len), (LEAF_VA.0, LEAF_LEN), "same range");
    assert_eq!(b.phys(), LEAF_PHYS);
    assert_eq!(
        b.aperture(),
        Aperture::Vidmem,
        "⊘ the aperture is NOT corrected to sysmem by the join. It records the GUEST's own \
         declaration, and `phys` is a framebuffer offset — see `BackingBytes::JoinsGuestWindow`"
    );
    assert_eq!(
        b.kind(),
        kayfabe_mmu::RegionKind::RealGpuMemory,
        "★★★ ruling 4: a joined leaf IS kind 3. The emulated framebuffer is no longer its \
         store, so it is no longer kind 2 — even though the guest still declares it `Vidmem`"
    );
    assert!(
        !b.is_guest_ram(),
        "⊘ and `phys` must stay un-handable to `Vmm::gpa_read`"
    );
    let host = b.host().expect("the binding carries its materialization");
    assert_eq!(
        host.bytes(),
        kayfabe_mmu::BackingBytes::JoinsGuestWindow,
        "★★★★★ ONE memory, declared. ⊘ NOT `SoleBacking` — that would have been admitted by \
         an aperture-blind guard and is the word `w228`'s chain could also have used"
    );
    assert_eq!(host.host_va(), LEAF_VA.0);
    assert_eq!(host.memory(), joined.memory);
}

/// ★★★ **Idempotence hands back NO second backing**, and that is not tidiness.
///
/// A doorbell repeats. A replay that handed the caller a second descriptor for the same
/// pages would be a second lifetime for one file, and a VMM that installed it would map the
/// same memory at the same framebuffer address twice.
#[test]
fn a_joined_leaf_replayed_hands_back_no_second_backing() {
    let _wd = watchdog(
        "fb_leaf_backing::join_replay",
        std::time::Duration::from_secs(60),
    );
    let (device, pid, _rec) = device();
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
        .back_fb_leaf(
            GPU,
            PDB,
            LEAF_VA,
            LEAF_LEN,
            LEAF_PHYS,
            FbLeafBacking::Joined,
        )
        .expect("joins");
    // ⊘ **The adopt is what makes the replay a replay**, and that is a consequence of the
    // ordering fix rather than an incidental. `plan_back_fb_leaf` reads idempotence off the
    // table row's host backing, and the row does not carry one until the view is installed.
    // ⇒ A join whose install never happened is released by the shell and re-asks as a FIRST
    // join, which is the correct answer — the alternative is replaying onto a window that
    // points somewhere else.
    device
        .adopt_joined_fb_leaf(GPU, PDB, LEAF, &first)
        .expect("the adopt binds");
    let second = device
        .back_fb_leaf(
            GPU,
            PDB,
            LEAF_VA,
            LEAF_LEN,
            LEAF_PHYS,
            FbLeafBacking::Joined,
        )
        .expect("replays");
    assert!(first.backing.is_some() && !first.already);
    assert!(
        second.already && second.backing.is_none(),
        "a replay is a replay: no work, no second descriptor — got {second:?}"
    );
    assert_eq!(second.memory, first.memory, "the same host object");
}

/// ★★★★★ **A JOIN WHOSE VIEW NEVER GOT INSTALLED LEAVES NO ROW, AND RE-ASKS AS A FIRST
/// JOIN** — the failure path of the ordering fix, which is the half that matters.
///
/// ⊘ The happy path of *"bind after install"* and the happy path of *"bind before install"*
/// are indistinguishable; the difference is entirely in what a **refused** install leaves
/// behind. `fb-join` left a row asserting `BackingBytes::JoinsGuestWindow` over a
/// framebuffer window still pointing at the emulator's own page — permanently, with no
/// fault and no status, which is `w228`'s *"two memories, silent in both directions"* under
/// the one name that claims they are one.
///
/// ⇒ Here the shell simply never calls the adopt (this suite has no `RegPlane` to refuse an
/// install, and it does not need one — *not reaching* the adopt is exactly what a refused
/// install produces). The assertions are that the guest's own row is untouched, and that the
/// next ask is a FIRST join rather than a replay onto a window that points elsewhere.
#[test]
fn a_join_that_was_never_installed_leaves_the_guests_row_alone_and_re_asks_as_a_first_join() {
    let _wd = watchdog(
        "fb_leaf_backing::join_uninstalled",
        std::time::Duration::from_secs(60),
    );
    let (device, pid, _rec) = device();
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
        .back_fb_leaf(
            GPU,
            PDB,
            LEAF_VA,
            LEAF_LEN,
            LEAF_PHYS,
            FbLeafBacking::Joined,
        )
        .expect("joins");
    assert!(first.backing.is_some(), "the backing crossed");

    // ---- The shell's install fails here, so it releases and never adopts.
    device.release_unadopted_fb_leaf(GPU, PDB, first.host_va, first.memory);

    let (start, len, b) = tabled(&device, pid).expect("the guest's own row survives");
    assert_eq!((start, len), (LEAF_VA.0, LEAF_LEN), "the same range");
    assert!(
        b.host().is_none(),
        "★★★★★ THE WHOLE POINT: no host backing is recorded for a join nobody installed"
    );
    assert_eq!(
        b.kind(),
        kayfabe_mmu::RegionKind::FakeFramebuffer,
        "⊘ and the kind is not quietly promoted either — this range's store really is still \
         the emulated framebuffer"
    );

    // ★ And the re-ask is a FIRST join, not a replay: there is nothing to replay onto.
    let again = device
        .back_fb_leaf(
            GPU,
            PDB,
            LEAF_VA,
            LEAF_LEN,
            LEAF_PHYS,
            FbLeafBacking::Joined,
        )
        .expect("re-joins");
    assert!(
        !again.already && again.backing.is_some(),
        "⊘ a replay here would hand the shell no descriptor and it would install nothing, so \
         the leaf would stay two memories forever — got {again:?}"
    );

    // ★★★★★ **AND THE TEARDOWN AUDIT IS THE ASSERTION THAT THE RELEASE IS REAL.** Both joins
    // are released and neither is ever bound, so at teardown §12.35 must be able to account
    // for **every** host object this test made. ⊘ Watched: with only this second release
    // missing the audit failed naming exactly **one** unaccounted object and one mapping —
    // *one*, not two, which is the first release having genuinely been disposed rather than
    // merely enqueued. A `release_unadopted_fb_leaf` that dropped its orphans on the floor
    // would have named two, and no assertion written by hand in this file would have noticed.
    device.release_unadopted_fb_leaf(GPU, PDB, again.host_va, again.memory);
}
