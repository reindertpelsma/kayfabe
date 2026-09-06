//! # ★★★★★ **ONE FRAMEBUFFER FRAME, N GPU ADDRESSES** — `w380`, the LLM wall's fix.
//!
//! # ⊘⊘ The defect this file is written against, measured
//!
//! `[measured w376llmd, real GA106]` the framebuffer join was keyed by **frame alone**:
//! `install_join(phys, …)` / `release_join(phys)` / `fb_join_installed_at(phys)`. So one frame
//! could be host-backed at exactly **one** GPU VA, and the guest aliases frames at several.
//! Publishing either alias therefore *revoked the other* — **127 supersedes over 17 frames**,
//! then **28 108 `⊘ SUPERSEDE CAPPED`** once the per-pair cap froze the ping-pong, leaving one
//! live VA of each pair permanently unbacked. The `Xid 31 FAULT_PDE` landed at
//! `0x7480_27604000`, a VA we had unpublished ourselves, and the supersede path's own log line
//! predicted it verbatim.
//!
//! `w377` §9 settled which fix: a supersede **TARGET later becomes a SOURCE**, repeatedly and
//! stably. Staleness is monotone — a row we merely failed to drop can never be re-declared —
//! so only a guest that holds *both* VAs mapped can produce an alternating pair. ⇒ **The guest
//! holds every alias live, and the fix is to support the aliasing, not to arbitrate it.**
//!
//! And the driver agrees: `traces/real_ga106/w379_mapping_plane_real_ga106.txt`, bare metal,
//! 5/5 at two revisions — RM maps ONE allocation at TWO GPU VAs, both stay live, and unmapping
//! one leaves the other. Keying host backing by frame alone was **strictly weaker than the
//! driver we stand in for**.
//!
//! # ★★★ What this suite can and cannot judge
//!
//! Mock-driven and GPU-free, exactly as `fb_leaf_backing.rs`. It judges the **chain and the
//! bookkeeping** — that an alias mints no second memory, that adding VA_B leaves VA_A's row and
//! its host object untouched, that N is not capped, and that the last VA out is what frees the
//! frame. It judges **nothing** about whether RM accepts the second `OS_DESCRIPTOR`; that is
//! the bare-metal trace above, and a boot.

use std::sync::Arc;

use kayfabe_arch::Aperture;
use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_fwd::FbLeafBacking;
use kayfabe_mmu::{Binding, HostBacking};
use kayfabe_mocks::{MockArch, MockIsolateFactory, RmVerb, SharedRecorder};
use kayfabe_rt::device::{LockMode, SharedDevice};
use kayfabe_tests::{Guarded, Scenario, identical_handles};

const GPU: GpuId = GpuId::ZERO;
const CLIENT: HClient = HClient(0xA0);
const PDB: Pdb = Pdb(0x3400_0000);
const GR: VChid = VChid(0x100);
const CE: VChid = VChid(0x200);
const MEM: HObject = HObject(0x6000_0000);

/// The frame every VA in this file names. ⊘ `0x1e00000` is not invented: it is the frame
/// `w377` §9 printed the ping-pong of, the one that alternated between `0x7480ac000000` and
/// `0x748037200000` eight times.
const FRAME: u64 = 0x1e0_0000;
/// The leaf's length — one PD0 leaf on this chip, and a whole number of
/// [`kayfabe_fwd::FB_LEAF_GRANULE`]s, or the plan refuses on granularity before any of this
/// file's subject matter is reached.
const LEN: u64 = 0x20_0000;

/// The three VAs, from the same measured chain. ★ **Three, not two**, and that is the point of
/// the file: 8 of the 17 frames in that boot reached three VAs, and a fix that handled two
/// would be `SUPERSEDE_CAP_PER_FRAME`'s mistake one level up.
const VA_A: GpuVa = GpuVa(0x7480_b000_0000);
const VA_B: GpuVa = GpuVa(0x7480_ac00_0000);
const VA_C: GpuVa = GpuVa(0x7480_3720_0000);

fn leaf(va: GpuVa) -> kayfabe_fwd::FbLeafRange {
    kayfabe_fwd::FbLeafRange {
        va,
        len: LEN,
        phys: FRAME,
    }
}

/// One guest proc on GPU0 — `fb_leaf_backing.rs`'s fixture, unchanged.
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
            "fb_leaf_alias::device",
            Arc::new(SharedDevice::new(gpu, LockMode::Sharded)),
            recorder.clone(),
        ),
        pid,
        recorder,
    )
}

/// Declare, in this proc's address table, that the guest's own page tables bind `va` to
/// [`FRAME`] — the populate pass's output, which is what the production caller reads.
fn guest_binds(device: &SharedDevice, pid: kayfabe_core::ProcId, va: GpuVa) {
    device
        .with_proc_mut(pid, |p| {
            let vas = p.vases.get_mut(&(GPU, PDB)).expect("the compute VAS");
            vas.table
                .bind(
                    PDB,
                    va,
                    LEN,
                    Binding::declared_by_guest(FRAME, Aperture::Vidmem)
                        .expect("the fixture declares a kind the guest can declare"),
                )
                .expect("the fixture's own bind is well-formed");
        })
        .expect("the proc is live");
}

/// The binding the table holds at `va`, if any.
fn tabled(device: &SharedDevice, pid: kayfabe_core::ProcId, va: GpuVa) -> Option<Binding> {
    device
        .with_proc_mut(pid, |p| {
            p.vases
                .get(&(GPU, PDB))
                .expect("the compute VAS")
                .table
                .binding_at(va)
                .map(|(_s, _l, b)| b)
        })
        .expect("the proc is live")
}

/// The host backing at `va` — `None` when the row is absent OR carries no host object. ⊘ The
/// two are collapsed **only** in helpers that then assert on the difference themselves.
fn backing_at(device: &SharedDevice, pid: kayfabe_core::ProcId, va: GpuVa) -> Option<HostBacking> {
    tabled(device, pid, va).and_then(|b| b.host())
}

/// Join [`FRAME`] at `va` and bind the row — the first VA's full chain, both halves.
fn join(device: &SharedDevice, pid: kayfabe_core::ProcId, va: GpuVa) {
    guest_binds(device, pid, va);
    let backed = device
        .back_fb_leaf(GPU, PDB, va, LEN, FRAME, FbLeafBacking::Joined)
        .expect("the leaf joins");
    assert!(
        !backed.alias,
        "⊘ a FIRST join is not an alias, and the two must not be confused: they need \
         opposite actions from the shell"
    );
    assert!(
        backed.backing.is_some(),
        "★ the join's whole point — a backing the VMM can map as the guest's view"
    );
    device
        .adopt_joined_fb_leaf(GPU, PDB, leaf(va), &backed)
        .expect("the adopt binds");
}

/// Alias [`FRAME`] at `va` and bind the row — the second and every later VA's full chain.
fn alias(device: &SharedDevice, pid: kayfabe_core::ProcId, va: GpuVa) {
    guest_binds(device, pid, va);
    let backed = device
        .back_fb_leaf(GPU, PDB, va, LEN, FRAME, FbLeafBacking::Aliased)
        .expect("the frame aliases");
    assert!(
        backed.alias,
        "★★★ the shell must be TOLD this was an alias: `backing == None` also means \
         `already`, and those two need opposite actions — bind, versus do nothing"
    );
    assert!(
        backed.backing.is_none(),
        "⊘ an alias hands up NO backing: the frame's `memfd` crossed once, with the join. A \
         second descriptor would be a second lifetime for one file and a second view of one \
         memory"
    );
    device
        .adopt_joined_fb_leaf(GPU, PDB, leaf(va), &backed)
        .expect("the adopt binds the alias");
}

/// Unbind the row at `va` and give its host half back — what `apply_settlement_as`'s
/// `RevokeWholeJoins` plus the shell's `release_revoked_joins` do, minus the `RegPlane` this
/// suite does not have.
///
/// ⊘ The host object goes **whichever way the store's join goes**, and that asymmetry is the
/// driver's own: `w379` measured *"unmapping VA_A left VA_B live"* on real hardware. What the
/// last-VA rule governs is the STORE's join, not this.
fn revoke(device: &SharedDevice, pid: kayfabe_core::ProcId, va: GpuVa) {
    let h = backing_at(device, pid, va).expect("the row is backed");
    device
        .with_proc_mut(pid, |p| {
            p.vases
                .get_mut(&(GPU, PDB))
                .expect("the compute VAS")
                .table
                .unbind(va);
        })
        .expect("the proc is live");
    device.revoke_published_fb_leaf(GPU, PDB, h.host_va(), h.memory());
    device.drain_pending_releases();
}

/// How many times each verb kind touched [`FRAME`] — `(joins, aliases)`.
fn frame_verbs(rec: &SharedRecorder) -> (usize, usize) {
    let log = rec.lock().expect("recorder");
    let joins = log
        .log
        .iter()
        .filter(|(_, v)| matches!(v, RmVerb::JoinFbLeaf { phys, .. } if *phys == FRAME))
        .count();
    let aliases = log
        .log
        .iter()
        .filter(|(_, v)| matches!(v, RmVerb::AliasFbLeaf { phys, .. } if *phys == FRAME))
        .count();
    (joins, aliases)
}

// ---------------------------------------------------------------------------------
// 1 — ★★★★★ THE DEFECT, AS A TEST THAT WOULD HAVE CAUGHT IT
// ---------------------------------------------------------------------------------

/// ★★★★★ **INSTALLING VA_B MUST NOT UNBIND VA_A.** This is the regression test for the wall:
/// the whole of `w377`'s `Xid 31` is that publishing the second alias revoked the first.
///
/// ⊘ It asserts on the **first** VA, deliberately. Every instrument in that boot reported the
/// second one — the supersede line named the VA it was moving *to*, the census counted the row
/// it had just bound — and the row that had gone silent was the one that faulted. A test that
/// only checked "VA_B is backed" would have passed on the broken build.
#[test]
fn aliasing_a_frame_at_a_second_va_leaves_the_first_bound_and_backed() {
    let (device, pid, rec) = device();
    join(&device, pid, VA_A);
    let a_before = backing_at(&device, pid, VA_A).expect("VA_A is backed by the join");

    alias(&device, pid, VA_B);

    let a_after = backing_at(&device, pid, VA_A).expect(
        "★★★★★ VA_A MUST STILL BE HOST-BACKED. This is the w377 wall: publishing the second \
         alias revoked the first, and the engine still pointed there took a contained fault \
         our own log line had predicted verbatim",
    );
    assert_eq!(
        a_after.host_va(),
        a_before.host_va(),
        "⊘ and it must be the SAME mapping, not a re-placed one — a row that silently moved \
         would pass a mere `is_some` check"
    );
    assert_eq!(
        a_after.memory(),
        a_before.memory(),
        "⊘ and the SAME host object: freeing and re-minting under a live VA is the defect \
         with an extra step"
    );
    let b = backing_at(&device, pid, VA_B).expect("and VA_B is backed too — that is the point");
    assert_ne!(
        b.memory(),
        a_after.memory(),
        "★★★ each VA owns its OWN `OS_DESCRIPTOR`. That is what makes every existing reclaim \
         path — which frees per address-table row, unconditionally — correct without a \
         refcount: releasing one row takes nothing from its siblings. It is also what the \
         driver does (`w379`: unmapping VA_A left VA_B live)"
    );

    // ★★★★★ ONE MEMORY. The counts are the assertion: N addresses cost ONE mint of bytes.
    let (joins, aliases) = frame_verbs(&rec);
    assert_eq!(
        (joins, aliases),
        (1, 1),
        "⊘ exactly one JOIN and one ALIAS for this frame. Two joins would be two `memfd`s — \
         two memories for one frame, which is `w228` and is self-concealing"
    );
}

// ---------------------------------------------------------------------------------
// 2 — ★★★ N IS UNBOUNDED
// ---------------------------------------------------------------------------------

/// ★★★ **THREE VAs OVER ONE FRAME, ALL LIVE.** `[measured w377 §9]` 8 of 17 frames reach
/// three, and nothing in the guest's behaviour caps it.
///
/// ⚠ The `2` case passing is not evidence for the `3` case: `SUPERSEDE_CAP_PER_FRAME = 4` was
/// itself a bound that looked generous and became the wall. A cap does not stop a ping-pong;
/// it freezes it.
#[test]
fn three_vas_over_one_frame_are_all_backed_by_the_one_memory() {
    let (device, pid, rec) = device();
    join(&device, pid, VA_A);
    alias(&device, pid, VA_B);
    alias(&device, pid, VA_C);

    let mut objects = Vec::new();
    for va in [VA_A, VA_B, VA_C] {
        let h = backing_at(&device, pid, va)
            .unwrap_or_else(|| panic!("va=0x{:x} must be host-backed", va.0));
        assert_eq!(
            h.host_va(),
            va.0,
            "address identity: each alias is placed at the guest's OWN VA, or the engine \
             dereferences whatever else lives there"
        );
        assert_eq!(
            h.bytes(),
            kayfabe_mmu::BackingBytes::JoinsGuestWindow,
            "★ an alias declares `JoinsGuestWindow` and it is TRUE of it — the guest's window \
             for this frame was re-pointed at these very pages by the join"
        );
        assert!(
            h.frees_object(),
            "⊘ whole, not a slice: each alias owns its own object outright, which is what \
             makes the existing per-row reclaim correct"
        );
        objects.push(h.memory());
    }
    objects.sort_unstable();
    objects.dedup();
    assert_eq!(objects.len(), 3, "three rows, three distinct host objects");

    let (joins, aliases) = frame_verbs(&rec);
    assert_eq!(
        (joins, aliases),
        (1, 2),
        "★★★★★ ONE mint of bytes, THREE addresses. A trace showing three joins would show \
         three memories, and that is the opposite finding from the same count"
    );
}

// ---------------------------------------------------------------------------------
// 3 — ★★★★★ THE LAST VA OUT, NOT THE FIRST
// ---------------------------------------------------------------------------------

/// ★★★★★ **THE FRAME'S JOIN IS GIVEN BACK WHEN THE *LAST* VA DROPS, NOT THE FIRST.**
///
/// [`SharedDevice::fb_join_namers`] is the predicate the shell's release path gates on
/// (`release_revoked_joins`, `RETIRED-FB-RELEASE`). Before aliasing existed, *"this row is
/// going"* and *"this frame is finished"* were the same fact; they are not any more, and
/// reading them as one gives the store's join back while a sibling still maps those pages —
/// the guest's framebuffer reverts to fabricated pages while the engine reads the `memfd`.
/// Silent in both directions.
///
/// ⊘ The **host object** of the dropped row goes either way, and that asymmetry is the
/// driver's own: `w379` measured *"unmapping VA_A left VA_B live"* on real hardware.
#[test]
fn the_frame_is_still_named_after_the_first_of_two_aliases_goes() {
    let (device, pid, _rec) = device();
    join(&device, pid, VA_A);
    alias(&device, pid, VA_B);
    assert_eq!(
        device.fb_join_namers(FRAME),
        (2, 0),
        "two live rows name the frame"
    );

    // The guest unmaps VA_A. The settlement unbinds the row and hands the host half back; the
    // shell then asks who is left. ⊘ The release is performed, not skipped: this suite's
    // teardown audit refuses a host object nothing can name, and *that* is the discipline the
    // production path owes too — `revoke_published_fb_leaf` is exactly what the shell calls.
    revoke(&device, pid, VA_A);

    assert_eq!(
        device.fb_join_namers(FRAME),
        (1, 0),
        "★★★★★ ONE NAMER LEFT ⇒ THE STORE'S JOIN MUST BE KEPT. A release here is the \
         two-memories bug wearing a correct-looking reclaim"
    );

    revoke(&device, pid, VA_B);

    assert_eq!(
        device.fb_join_namers(FRAME),
        (0, 0),
        "⊘ and only NOW is the frame finished. Getting this backwards leaks or double-frees, \
         and this tree has had both"
    );
}

// ---------------------------------------------------------------------------------
// 4 — ★★★ THE PER-VAS QUESTION THE SHELL ASKS FIRST
// ---------------------------------------------------------------------------------

/// ★★★ **`fb_join_va_in_vas` is what selects `Aliased` over `Joined`**, and choosing wrongly
/// is silent in one direction: `Joined` where `Aliased` was wanted mints a **second memory**.
///
/// ⊘ It must answer `None` before any join exists — otherwise the very first leaf of a frame
/// would be planned as an alias and refused by name against a frame with no pages.
#[test]
fn the_per_vas_lookup_finds_a_sibling_only_once_one_exists() {
    let (device, pid, _rec) = device();
    assert_eq!(
        device.fb_join_va_in_vas(GPU, PDB, FRAME),
        None,
        "⊘ nothing names the frame yet, so the first leaf must plan a JOIN"
    );
    join(&device, pid, VA_A);
    assert_eq!(
        device.fb_join_va_in_vas(GPU, PDB, FRAME),
        Some(VA_A.0),
        "★ now VA_A holds the frame's pages, so a second VA must plan an ALIAS"
    );
    // ⊘ A row the guest declared but nothing has backed is NOT a sibling: it has no pages to
    // alias. Asserting this is what stops the lookup from matching the very leaf being planned.
    guest_binds(&device, pid, VA_C);
    assert_eq!(
        device.fb_join_va_in_vas(GPU, PDB, FRAME),
        Some(VA_A.0),
        "an unbacked declaration of the same frame is not a sibling"
    );
}

// ---------------------------------------------------------------------------------
// 5 — ⊘ THE REFUSAL THAT MAKES THE ASYMMETRY SAFE
// ---------------------------------------------------------------------------------

/// ⊘ **ALIASING A FRAME NOBODY HAS JOINED IS REFUSED BY NAME, AND MINTS NOTHING.**
///
/// The two mistakes are not symmetric and the code is built around that: `Joined` where
/// `Aliased` was wanted is **silent** (two memories), so it is made unrepresentable by asking
/// the store first; `Aliased` where `Joined` was wanted is **loud**, so it is left loud.
///
/// ★ The status is duplicated across a crate boundary ([`kayfabe_mocks::MOCK_FB_ALIAS_NO_JOIN`]
/// versus `kayfabe_isolate_host::rm::FB_ALIAS_NO_JOIN`) because `kayfabe-mocks` does not depend
/// on `kayfabe-isolate-host`. This test is the pin that makes a duplicated constant safe.
#[test]
fn aliasing_a_frame_with_no_join_is_refused_and_allocates_nothing() {
    let (device, pid, rec) = device();
    guest_binds(&device, pid, VA_B);
    let err = device
        .back_fb_leaf(GPU, PDB, VA_B, LEN, FRAME, FbLeafBacking::Aliased)
        .expect_err("a frame with no pages has nothing to alias");
    let text = format!("{err:?}");
    assert!(
        text.contains(&format!("{:x}", kayfabe_mocks::MOCK_FB_ALIAS_NO_JOIN))
            || text.contains(&format!("{}", kayfabe_mocks::MOCK_FB_ALIAS_NO_JOIN)),
        "⊘ refused BY NAME, carrying the alias-specific status rather than a generic \
         NoMemory that cannot be told from exhaustion. Got {text}"
    );
    assert!(
        backing_at(&device, pid, VA_B).is_none(),
        "and nothing was bound"
    );
    let (joins, aliases) = frame_verbs(&rec);
    assert_eq!(
        joins, 0,
        "★★★★★ AND NOTHING WAS MINTED. Falling back to a join here would give the frame a \
         second memory — the fabricated pages the guest still reads, and a blank host object \
         the engine reads — which is exactly what the refusal exists to prevent"
    );
    assert_eq!(aliases, 1, "the attempt IS recorded, with no object");
}

/// ⊘ The two duplicated constants must be equal, checked as a value and not as a comment.
#[test]
fn alias_refusal_status_matches_the_host_backend() {
    assert_eq!(
        kayfabe_mocks::MOCK_FB_ALIAS_NO_JOIN,
        kayfabe_isolate_host::rm::FB_ALIAS_NO_JOIN,
        "⊘ a duplicated constant is safe only while something fails when it drifts"
    );
}
