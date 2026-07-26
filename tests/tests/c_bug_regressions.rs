//! ★ THE C-BUG REGRESSION MATRIX — the GAP rows (decision #18B).
//!
//! Companion to `docs/design/c_bug_regression_matrix.md`, which classifies EVERY
//! hard-won C bug as impossible-by-construction / already-tested / gap. This file
//! adds the regressions for the gap rows, so the rewrite can never silently
//! reintroduce a bug the C already paid (days of bench time) to find. Each test's
//! doc comment names the C incident it guards against, with its memory-file ref.
//!
//! Everything here drives the real core through the mock seams — no GPU, no OS,
//! millisecond-fast (decision #15: test invariants/behavior, never internals).

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped handle/VA literals

use kayfabe_arch::ids::GpuId;
use kayfabe_arch::ids::{GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_completion::OsEventRef;
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::{Gpu, GpuError};
use kayfabe_core::rmgraph::{NodeKey, RmEvent};
use kayfabe_fwd::{
    FwdFault, handle_doorbell, parse_pushbuffer, publish_backing, resolve, signal_golden_capture,
};
use kayfabe_mocks::{MockArch, MockIsolateFactory, MockPushbuffer, MockVmm, mock_classes as mc};
use kayfabe_tests::{Guarded, ResidueClaim, Scenario, identical_handles};
use kayfabe_vmm::Vmm;

const A_PDB: Pdb = Pdb(0x3401_000);
const B_PDB: Pdb = Pdb(0x3405_000);
const VA: GpuVa = GpuVa(0x2_0020_0000);

/// Fresh device with a generous window (default geometry for non-lifecycle tests).
fn fresh_gpu() -> (
    Guarded<Gpu>,
    std::sync::Arc<std::sync::Mutex<kayfabe_mocks::RmRecorder>>,
) {
    gpu_with_window(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000)
}

/// Fresh device over an explicit window geometry (the lifecycle tests size the
/// window tightly so exhaustion-vs-recycling is observable).
fn gpu_with_window(
    window: core::ops::Range<u64>,
    arena_len: u64,
) -> (
    Guarded<Gpu>,
    std::sync::Arc<std::sync::Mutex<kayfabe_mocks::RmRecorder>>,
) {
    let arch = Box::new(MockArch::new());
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(window, arena_len);
    (
        Guarded::new(
            "c_bug_regressions",
            Gpu::new(arch, Box::new(factory), gpa).expect("device realizes"),
            rec.clone(),
        ),
        rec,
    )
}

fn apply_all(gpu: &mut Gpu, evs: impl IntoIterator<Item = RmEvent>) {
    for ev in evs {
        gpu.apply(ev).expect("scenario event applies");
    }
}

/// Build one compute process and return its (pid, first ChanId).
fn build_proc(
    gpu: &mut Gpu,
    client: HClient,
    pdb: Pdb,
    vchids: (u16, u16),
) -> (kayfabe_core::ProcId, kayfabe_core::ChanId) {
    let mut s = Scenario::new();
    s.compute_process(client, pdb, identical_handles(vchids.0, vchids.1));
    apply_all(gpu, s.events);
    let pid = *gpu
        .spine
        .by_pdb
        .get(&(GpuId::ZERO, pdb))
        .expect("routed by PDB");
    let cid = *gpu.procs[&pid]
        .chan_ids
        .values()
        .next()
        .expect("has a channel");
    (pid, cid)
}

/// Lay out a one-entry GPFIFO ring pointing at method words written to guest RAM.
fn script_pushbuffer(vmm: &mut MockVmm, gpa: u64, methods: &[(u32, Vec<u32>)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (h, args) in methods {
        bytes.extend_from_slice(&h.to_le_bytes());
        for a in args {
            bytes.extend_from_slice(&a.to_le_bytes());
        }
    }
    vmm.gpa_write(gpa, &bytes).unwrap();
    let mut ring = Vec::new();
    ring.extend_from_slice(&gpa.to_le_bytes());
    ring.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    ring
}

// =================================================================================
// #11 — the USERD-wipe class: a CE fill/copy must never clobber live core state.
// =================================================================================

/// **Guards #11** (`mode2_baremetal_32`: the CE zero-fill that WIPED a live USERD
/// page — PyTorch's channel ring state — fixed in the C by `nvkvm_fb_is_live_userd`).
///
/// The core half of that invariant: an observed CE **physical** write landing on an
/// address the core already tracks as LIVE state must never silently REPLACE that
/// state. The parser's capture path is resolve-first — a dst that already resolves
/// keeps its live binding (recorded as a dirtied PT page only); the live mapping's
/// phys/host publication survive untouched.
///
/// (The *content* half — refusing the actual byte wipe over a live USERD in the FB
/// shadow — is the FB-shadow/regs port's job and is flagged as a deferred-milestone
/// row in the matrix. This pins the state half the pure core owns today.)
#[test]
fn cb11_ce_write_never_clobbers_live_binding() {
    let (mut gpu, _rec) = fresh_gpu();
    let mut vmm = MockVmm::new();
    let (pid, cid) = build_proc(&mut gpu, HClient(0xAA), A_PDB, (0x10, 0x11));

    // Live state: a published backing at VA (host-mapped in this Vas's host VAS).
    let live = publish_backing(
        gpu.procs.get_mut(&pid).unwrap(),
        GpuId::ZERO,
        A_PDB,
        VA,
        0x10000,
    )
    .unwrap();

    // A scrubber-shaped CE physical write lands INSIDE the live range.
    let ring = script_pushbuffer(
        &mut vmm,
        0x5000_0000,
        &[
            MockPushbuffer::set_object(mc::DMA_COPY),
            MockPushbuffer::ce_launch_dma(VA.0 + 0x20, 0x1000, false),
        ],
    );
    let out = parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring).expect("parses");

    // Observed (dirty-latched), but the LIVE binding was not replaced: the VA still
    // resolves to its original backing + host publication — never to the CE write's
    // identity mapping (the wipe-shaped clobber).
    assert_eq!(
        out.pt_writes,
        vec![VA.0 & !0xfff],
        "write observed as a dirtied page"
    );
    let (b, off) = resolve(&gpu, GpuId::ZERO, A_PDB, GpuVa(VA.0 + 0x20)).unwrap();
    assert_eq!(off, 0x20);
    assert_eq!(
        b.phys, live.gpa,
        "live binding's phys survives the CE write"
    );
    assert_eq!(
        b.host_va(),
        Some(live.host_va),
        "live host publication survives the CE write"
    );
}

// =================================================================================
// #12 — the sema-write VAS collapse (UVM MAX_JUMP): completion writes must route by
// the OWNING identity, never through a foreign VAS fallback.
// =================================================================================

/// **Guards #12 cont.32/33** (`mode2_12_layered_status`): the C's `chan_translate`
/// blind fallback resolved a scrubber's sema release under a STALE FOREIGN client's
/// PDB and wrote onto UVM's persistent tracking-sema page → UVM read a BACKWARDS
/// value → `uvm_gpu_semaphore.c:776 MAX_JUMP` fatal → CTX2 dead.
///
/// The rewrite must route a channel's sema release to the OWNING proc's completion
/// queue only — never to any other proc's (there is no fallback resolve to reach a
/// foreign queue through). And once the owner is torn down, the release is a LOUD
/// refusal — never consumed against a surviving proc's state.
#[test]
fn cb12_sema_release_routes_to_owner_never_a_foreign_proc() {
    let (mut gpu, _rec) = fresh_gpu();
    let mut vmm = MockVmm::new();
    let (pid_a, cid_a) = build_proc(&mut gpu, HClient(0xAA), A_PDB, (0x10, 0x11));
    let (pid_b, _cid_b) = build_proc(&mut gpu, HClient(0xBB), B_PDB, (0x20, 0x21));

    // A's channel releases a sema. ONLY A's queue observes it.
    let ring = script_pushbuffer(
        &mut vmm,
        0x5000_0000,
        &[MockPushbuffer::sem_release(0x2_0044_0f00, 0x45)],
    );
    parse_pushbuffer(&mut gpu, &mut vmm, pid_a, cid_a, &ring).expect("parses");
    assert!(
        gpu.procs[&pid_a].completion.has_outstanding(),
        "owner's queue observed"
    );
    assert!(
        !gpu.procs[&pid_b].completion.has_outstanding(),
        "foreign proc's queue NEVER sees another channel's release (the MAX_JUMP write)"
    );
    assert!(
        !gpu.system.completion.has_outstanding(),
        "nor does the system queue"
    );

    // Tear A down; replaying its release is a LOUD refusal — the C's fallback would
    // have resolved it under the survivor's PDB and poisoned B's live state.
    gpu.apply(RmEvent::Free {
        client: HClient(0xAA),
        handle: HObject(0x5c00_0000),
    })
    .unwrap();
    let replay = parse_pushbuffer(&mut gpu, &mut vmm, pid_a, cid_a, &ring);
    assert!(
        matches!(replay, Err(FwdFault::RetiredProc(_))),
        "a torn-down channel's release is refused, never re-routed: {replay:?}"
    );
    assert!(
        !gpu.procs[&pid_b].completion.has_outstanding(),
        "survivor B untouched"
    );
}

/// **Guards the #12 finishPayload-forge scoping rule + the scrubber-timeout class**
/// (`mode2_12_layered_status` cont.33 fix 2: the forge is for KERNEL CeUtils ONLY,
/// user-CE/GR excluded; lesson L5): a fabricated/system completion must land on the
/// SYSTEM proc's queue and can never reach a user proc's completion state. In the
/// rewrite the forge entry point is *typed* to the system proc
/// (`signal_golden_capture`) — this pins the runtime half.
#[test]
fn cb12_system_forge_never_reaches_a_user_proc_queue() {
    let (mut gpu, _rec) = fresh_gpu();
    let (pid_a, _) = build_proc(&mut gpu, HClient(0xAA), A_PDB, (0x10, 0x11));

    let ev = signal_golden_capture(&mut gpu, OsEventRef(0x60)).expect("system-typed forge");
    assert_eq!(ev, OsEventRef(0x60));
    assert!(
        gpu.system.completion.has_outstanding(),
        "forge landed on the SYSTEM queue"
    );
    assert!(
        !gpu.procs[&pid_a].completion.has_outstanding(),
        "a user proc's queue never receives a forged completion (L5)"
    );
    // It still DELIVERS (the guest's 4s golden-capture poll is satisfied).
    let batch = gpu
        .pump_completions(GpuId::ZERO)
        .expect("system completion posts");
    assert_eq!(batch.events, vec![OsEventRef(0x60)]);
}

// =================================================================================
// #13 — CE-PT-write publication: decode the observed write DIRECTLY, never a
// root walk racing the guest's leaf-filled-then-linked build order.
// =================================================================================

/// **Guards #13 rounds 1-3's mystery** (`mode2_13_multiiter_idle_hang`): the guest
/// fills a leaf PT page and links it under the root a SEPARATE push later. The C's
/// root walks read `runs=0` while the host faulted one page past the last-backed
/// leaf; the v6 fix decodes each dirtied page DIRECTLY. The rewrite's capture path
/// must make an observed CE PT-write resolvable IMMEDIATELY — with zero
/// root-reachability, before any "linking" write arrives — and a later linking
/// write must not disturb it. (There is no root-walk resolve path to get this
/// wrong, but this pins the observable contract.)
#[test]
fn cb13_pt_write_capture_is_direct_no_root_reachability_needed() {
    let (mut gpu, _rec) = fresh_gpu();
    let mut vmm = MockVmm::new();
    let (pid, cid) = build_proc(&mut gpu, HClient(0xAA), A_PDB, (0x10, 0x11));

    // Push 1: the CE fills an ORPHAN leaf PT page — nothing references it yet.
    let leaf_page: u64 = 0x1_2340_0000;
    let ring1 = script_pushbuffer(
        &mut vmm,
        0x5000_0000,
        &[MockPushbuffer::ce_launch_dma(leaf_page, 0x1000, false)],
    );
    parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring1).expect("push 1 parses");

    // ★ Resolvable IMMEDIATELY at the release point — no link, no walk, no sweep.
    let (b, _) = resolve(&gpu, GpuId::ZERO, A_PDB, GpuVa(leaf_page))
        .expect("an observed PT-write is resolvable directly — the C's root walk read runs=0 here");
    assert_eq!(b.phys, leaf_page);
    assert!(
        gpu.procs[&pid].vases[&(GpuId::ZERO, A_PDB)]
            .pt_pages
            .contains(&leaf_page)
    );

    // Push 2 (a push LATER): the upper-PD "linking" write. Both captured; the leaf
    // from push 1 is undisturbed.
    let upper_pd: u64 = 0x1_2000_0000;
    let ring2 = script_pushbuffer(
        &mut vmm,
        0x5001_0000,
        &[MockPushbuffer::ce_launch_dma(upper_pd, 0x1000, false)],
    );
    parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring2).expect("push 2 parses");
    assert!(resolve(&gpu, GpuId::ZERO, A_PDB, GpuVa(upper_pd)).is_ok());
    assert!(
        resolve(&gpu, GpuId::ZERO, A_PDB, GpuVa(leaf_page)).is_ok(),
        "leaf capture survives the link"
    );
}

// =================================================================================
// FUZZ-FOUND — the pushbuffer decoder's boundary-1 posture over HOSTILE guest bytes
// (found by the coverage-guided `fuzz/fuzz_targets/parse_pushbuffer.rs` cargo-fuzz
// target; both minimized here as deterministic regressions so the fuzzer never has to
// re-find them). Class: attacker-controlled address arithmetic reaching an unchecked
// `+`/`checked_add().expect()` — a hostile byte string could PANIC the core.
// =================================================================================

/// **Fuzz-found #1** — a hostile CE `LAUNCH_DMA` with a *physical* destination near
/// `u64::MAX` panicked the core: the #13 capture path binds `[dst, dst + len.max(4K))`
/// into the address table, and `IntervalMap::insert` did `start.checked_add(len)
/// .expect("range wraps u64")` — a guest-controlled `dst` made that wrap and abort.
///
/// Fix: `IntervalMap::insert` now returns `IntervalError::Wraps` (and `::Empty` for a
/// zero-length range) instead of panicking, which `AddressTable::bind` maps to the
/// loud `AddressFault::Malformed`. A malformed range from hostile bytes is a clean
/// `Err`, never a panic (boundary-1). The SAME container also backs the RPC-map bind
/// path (`sync_rpc_mappings`), so that untrusted entry is closed by the same fix.
#[test]
fn cbfuzz_ce_physical_dst_near_umax_is_a_loud_fault_never_a_panic() {
    let (mut gpu, _rec) = fresh_gpu();
    let mut vmm = MockVmm::new();
    let (pid, cid) = build_proc(&mut gpu, HClient(0xAA), A_PDB, (0x10, 0x11));

    // The minimized crash: one physical CE dst whose page + len wraps u64.
    let ring = script_pushbuffer(
        &mut vmm,
        0x5000_0000,
        &[MockPushbuffer::ce_launch_dma(
            0xFFFF_FFFF_FFFF_F800,
            0x1000,
            false,
        )],
    );
    let out = parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring);
    // Loud fault (address-table refusal), NOT a panic and NOT a silent accept.
    assert!(
        matches!(out, Err(FwdFault::Address(_))),
        "a u64-wrapping CE dst must be a loud fault, got {out:?}"
    );
    // No torn state: nothing was left half-bound.
    assert_eq!(
        gpu.procs[&pid].vases[&(GpuId::ZERO, A_PDB)]
            .table
            .iter()
            .count(),
        0,
        "a refused malformed bind leaves the table empty (no torn partial state)"
    );
}

/// **Fuzz-found #2** — a hostile GPFIFO entry naming a range base near `u64::MAX`
/// panicked the guest-memory adapter: the parser caps the per-range read to multiple
/// pages (bounded work, correct) and forwards the attacker-controlled `gpa` to
/// `Vmm::gpa_read`, whose contract is to return a `Result` — but the mock adapter did
/// `gpa + i` and overflowed. The core is correct (it handles a `gpa_read` `Err` via
/// `map_err`); the adapter must not panic. Fix: the mock now addresses guest RAM with
/// `checked_add` (an un-formable address is simply absent → reads 0, exactly a real
/// adapter's unbacked-page behavior). Pins that a top-of-space range never panics.
#[test]
fn cbfuzz_gpfifo_range_gpa_near_umax_never_panics() {
    let (mut gpu, _rec) = fresh_gpu();
    let mut vmm = MockVmm::new();
    let (pid, cid) = build_proc(&mut gpu, HClient(0xAA), A_PDB, (0x10, 0x11));

    // A GPFIFO ring with one entry: base near the top of the 64-bit space, a
    // multi-page declared length (the parser caps it, then reads across the wrap).
    let mut ring = Vec::new();
    ring.extend_from_slice(&0xFFFF_FFFF_FFFF_F000u64.to_le_bytes());
    ring.extend_from_slice(&0x20_0000u64.to_le_bytes()); // 2 MiB declared
    let out = parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring)
        .expect("a top-of-space range reads sparse zeros, never panics");
    // Zeroed method words decode to nothing actionable.
    assert!(out.sem_releases.is_empty() && out.pt_writes.is_empty() && out.invalidates.is_empty());
}

// =================================================================================
// #14 — the arming-window class: there is no "multiproc mode" to arm too late.
// =================================================================================

/// **Guards #14 round 3's transition window** (`mode2_14_concurrent_apps`): the C
/// gated its per-process divergences on `multiproc() = n>1`, but n flips only after
/// the 2nd process is DETECTED — proc 1 + shared state ran blind first, and the
/// corruption had landed by arming time. The rewrite has NO gate (single-process is
/// the N=1 case of the only code path, lesson L9) — so a second process arriving
/// AFTER the first is fully active (the exact C order: wo_14 interleaves; this test
/// serializes) must build cleanly AND leave the first's state untouched.
#[test]
fn cb14_second_proc_arrives_after_first_is_active_no_arming_window() {
    let (mut gpu, _rec) = fresh_gpu();
    // Proc A is built AND fully active: published, scheduled, rung, completed.
    let (pid_a, _cid_a) = build_proc(&mut gpu, HClient(0xAA), A_PDB, (0x10, 0x11));
    let pub_a = publish_backing(
        gpu.procs.get_mut(&pid_a).unwrap(),
        GpuId::ZERO,
        A_PDB,
        VA,
        0x10000,
    )
    .unwrap();
    let out_a =
        handle_doorbell(&mut gpu, GpuId::ZERO, MockArch::token_for(VChid(0x10)), &[]).unwrap();
    assert!(out_a.scheduled_now);
    gpu.procs
        .get_mut(&pid_a)
        .unwrap()
        .completion
        .observe(OsEventRef(0xA))
        .unwrap();
    let batch = gpu
        .pump_completions(GpuId::ZERO)
        .expect("A's completion posts");
    gpu.completions_drained(GpuId::ZERO);
    gpu.procs
        .get_mut(&pid_a)
        .unwrap()
        .completion
        .ack(batch.events[0]);

    // NOW proc B arrives — identical handles, identical guest VA.
    let (pid_b, _cid_b) = build_proc(&mut gpu, HClient(0xBB), B_PDB, (0x20, 0x21));
    let pub_b = publish_backing(
        gpu.procs.get_mut(&pid_b).unwrap(),
        GpuId::ZERO,
        B_PDB,
        VA,
        0x10000,
    )
    .unwrap();
    assert_ne!(
        pub_a.gpa, pub_b.gpa,
        "late-arriving B still gets disjoint backing"
    );
    assert_ne!(pub_a.host_va, pub_b.host_va);
    let out_b =
        handle_doorbell(&mut gpu, GpuId::ZERO, MockArch::token_for(VChid(0x20)), &[]).unwrap();
    assert!(
        out_b.scheduled_now,
        "B's channel schedules on ITS OWN plane"
    );
    assert_ne!(out_a.host_token, out_b.host_token);

    // A's pre-existing state is untouched by B's arrival: same resolution, and its
    // already-scheduled channel rings again WITHOUT rescheduling (state preserved).
    let (ba, _) = resolve(&gpu, GpuId::ZERO, A_PDB, VA).unwrap();
    assert_eq!(ba.phys, pub_a.gpa, "A resolves ITS backing after B armed");
    let again =
        handle_doorbell(&mut gpu, GpuId::ZERO, MockArch::token_for(VChid(0x10)), &[]).unwrap();
    assert!(
        !again.scheduled_now,
        "A's exec-plane state survived B's arrival"
    );
}

/// **Guards the silent-state-fold half of the same class** (lesson L9): a dup edge
/// that would MERGE two procs after both touched their data planes is a LOUD
/// `LateMerge` refusal, applied ATOMICALLY — the device rolls back and BOTH procs
/// stay fully functional (the C instead folded/aliased state and corrupted the
/// loser).
#[test]
fn cb14_late_merge_after_touch_is_loud_and_atomic() {
    let (mut gpu, _rec) = fresh_gpu();
    let (pid_a, _) = build_proc(&mut gpu, HClient(0xAA), A_PDB, (0x10, 0x11));
    let (pid_b, _) = build_proc(&mut gpu, HClient(0xBB), B_PDB, (0x20, 0x21));
    let pub_a = publish_backing(
        gpu.procs.get_mut(&pid_a).unwrap(),
        GpuId::ZERO,
        A_PDB,
        VA,
        0x10000,
    )
    .unwrap();
    let pub_b = publish_backing(
        gpu.procs.get_mut(&pid_b).unwrap(),
        GpuId::ZERO,
        B_PDB,
        VA,
        0x10000,
    )
    .unwrap();

    // A hostile/late dup edge that would join the two touched components.
    let late_dup = RmEvent::Dup {
        src: NodeKey::new(HClient(0xAA), HObject(0x5c00_0010)),
        dst: NodeKey::new(HClient(0xBB), HObject(0x9999_0000)),
    };
    let refused = gpu.apply(late_dup);
    assert!(
        matches!(refused, Err(GpuError::LateMerge { .. })),
        "merging two touched procs is a loud refusal, never a silent fold: {refused:?}"
    );

    // ATOMIC: both procs still live, still routed, still resolving their OWN state.
    assert_eq!(gpu.procs.len(), 2, "rollback kept both procs live");
    let (ba, _) = resolve(&gpu, GpuId::ZERO, A_PDB, VA).unwrap();
    let (bb, _) = resolve(&gpu, GpuId::ZERO, B_PDB, VA).unwrap();
    assert_eq!(ba.phys, pub_a.gpa);
    assert_eq!(bb.phys, pub_b.gpa);
    assert!(handle_doorbell(&mut gpu, GpuId::ZERO, MockArch::token_for(VChid(0x10)), &[]).is_ok());
    assert!(handle_doorbell(&mut gpu, GpuId::ZERO, MockArch::token_for(VChid(0x20)), &[]).is_ok());
}

/// ★ Mutation-gate kill (`Proc::is_untouched` `&&`→`||` between the ARENA clause and
/// the host-channel clause): a proc is untouched ONLY if EVERY data-plane clause holds
/// — arena un-carved AND no host channel AND no host VAS. `cb14` above touches all
/// three at once (via `publish_backing`), so it cannot distinguish an `&&`→`||`
/// mutation (which only differs when the clauses DISAGREE). This isolates the arena
/// clause: it carves the arena DIRECTLY (no host channel/VAS materialized), so
/// `is_untouched` must be false on the arena clause ALONE. A late merge that would
/// absorb this arena-touched proc is therefore a loud `LateMerge` — the `||` mutant
/// (which would read `arena_touched || channels_clean = true`) would wrongly permit the
/// silent fold the C corrupted the loser with.
#[test]
fn cb14_arena_touch_alone_blocks_a_late_merge() {
    let (mut gpu, _rec) = fresh_gpu();
    let (pid_a, _) = build_proc(&mut gpu, HClient(0xAA), A_PDB, (0x10, 0x11));
    let (pid_b, _) = build_proc(&mut gpu, HClient(0xBB), B_PDB, (0x20, 0x21));

    // Touch ONLY proc B's arena — carve a GPA directly, materializing NO host channel
    // and NO host VAS (so the arena clause of `is_untouched` is the only false one).
    let _touch = gpu
        .procs
        .get_mut(&pid_b)
        .unwrap()
        .arenas
        .get_mut(&GpuId::ZERO)
        .unwrap()
        .alloc(0x10000, 0x1000)
        .expect("arena carves");
    let _ = pid_a;

    // A late dup edge that would join A's and B's components. B's arena is touched, so
    // the merge must be refused loudly and atomically — never a silent fold.
    let late_dup = RmEvent::Dup {
        src: NodeKey::new(HClient(0xAA), HObject(0x5c00_0010)),
        dst: NodeKey::new(HClient(0xBB), HObject(0x9999_0000)),
    };
    let refused = gpu.apply(late_dup);
    assert!(
        matches!(refused, Err(GpuError::LateMerge { .. })),
        "an arena-touched proc cannot be silently merge-absorbed: {refused:?}"
    );
    assert_eq!(gpu.procs.len(), 2, "rollback kept both procs live");
}

/// ★ Mutation-gate kill (`Proc::is_untouched` `&&`→`||` between the HOST-CHANNEL clause
/// and the host-VAS clause): the twin of the test above, isolating the OTHER `&&`. A
/// materialized host channel alone (arena un-carved, no host VAS) must make a proc
/// touched — so a late merge absorbing it is a loud `LateMerge`. The `||` mutant would
/// read `channels_clean(false) || vases_clean(true) = true` and wrongly permit the fold.
/// We set the host channel DIRECTLY (the only touched clause) rather than via a doorbell
/// (which would also materialize the host VAS and mask the disagreement).
#[test]
fn cb14_host_channel_touch_alone_blocks_a_late_merge() {
    let (mut gpu, _rec) = fresh_gpu();
    let (pid_a, _) = build_proc(&mut gpu, HClient(0xAA), A_PDB, (0x10, 0x11));
    let (pid_b, cid_b) = build_proc(&mut gpu, HClient(0xBB), B_PDB, (0x20, 0x21));
    let _ = pid_a;
    // ★ §12.35 — DECLARED RESIDUE (dangling). This test FABRICATES a `HostHandle` the
    // mock never minted, because the state it needs — "host-VAS/host-channel touched and
    // nothing else" — is not reachable through the protocol: materializing one for real
    // would touch the other clauses too. A fabricated handle is by construction unknown
    // to the ledger, so it reads as dangling. Declared rather than worked around: a
    // harness that reaches past the core should have to say so.
    gpu.declare_residue(
        ResidueClaim::on(
            kayfabe_isolate::IsolateId(pid_b.0),
            "harness bypass: the late-merge clause under test is set by writing a \
             fabricated HostHandle straight into core state, so the ledger has never \
             seen it",
        )
        .dangling(1, 0),
    );

    // Touch ONLY proc B's host-channel clause: give one channel a host channel object,
    // leaving the arena un-carved and every VAS's host VAS unmaterialized.
    gpu.procs
        .get_mut(&pid_b)
        .unwrap()
        .channels
        .get_mut(&cid_b)
        .unwrap()
        .host_channel = Some(kayfabe_isolate::HostHandle::new(
        kayfabe_isolate::IsolateId(0),
        0xC0FFEE,
    ));

    let late_dup = RmEvent::Dup {
        src: NodeKey::new(HClient(0xAA), HObject(0x5c00_0010)),
        dst: NodeKey::new(HClient(0xBB), HObject(0x9999_0000)),
    };
    let refused = gpu.apply(late_dup);
    assert!(
        matches!(refused, Err(GpuError::LateMerge { .. })),
        "a host-channel-touched proc cannot be silently merge-absorbed: {refused:?}"
    );
    assert_eq!(gpu.procs.len(), 2, "rollback kept both procs live");
}

/// ★ Mutation-gate kill (`Proc::is_untouched` `&&`→`||` INSIDE the per-VAS clause,
/// between `host_vas.is_none()` and the "no published binding" check): a VAS is
/// untouched only if it has NEITHER a materialized host VAS NOR any host-published
/// binding — either one alone is host state that blocks an early merge. `publish_backing`
/// sets BOTH at once, so the two sub-conditions always agree there; this isolates the
/// host-VAS sub-condition by materializing ONLY a host VAS (no published binding, arena
/// un-carved, no host channel). The `||` mutant would read `host_vas_absent(false) ||
/// no_published_binding(true) = true` and wrongly call the VAS untouched.
#[test]
fn cb14_host_vas_touch_alone_blocks_a_late_merge() {
    let (mut gpu, _rec) = fresh_gpu();
    let (pid_a, _) = build_proc(&mut gpu, HClient(0xAA), A_PDB, (0x10, 0x11));
    let (pid_b, _) = build_proc(&mut gpu, HClient(0xBB), B_PDB, (0x20, 0x21));
    let _ = pid_a;
    // ★ §12.35 — DECLARED RESIDUE (dangling). This test FABRICATES a `HostHandle` the
    // mock never minted, because the state it needs — "host-VAS/host-channel touched and
    // nothing else" — is not reachable through the protocol: materializing one for real
    // would touch the other clauses too. A fabricated handle is by construction unknown
    // to the ledger, so it reads as dangling. Declared rather than worked around: a
    // harness that reaches past the core should have to say so.
    gpu.declare_residue(
        ResidueClaim::on(
            kayfabe_isolate::IsolateId(pid_b.0),
            "harness bypass: the late-merge clause under test is set by writing a \
             fabricated HostHandle straight into core state, so the ledger has never \
             seen it",
        )
        .dangling(1, 0),
    );

    // Materialize ONLY a host VAS on proc B (no published binding, no host channel,
    // arena un-carved) — the host-VAS sub-condition is the only touched one.
    let vas = gpu
        .procs
        .get_mut(&pid_b)
        .unwrap()
        .vases
        .get_mut(&(GpuId::ZERO, B_PDB))
        .unwrap();
    vas.host_vas = Some(kayfabe_isolate::HostHandle::new(
        kayfabe_isolate::IsolateId(0),
        0xBADA55,
    ));

    let late_dup = RmEvent::Dup {
        src: NodeKey::new(HClient(0xAA), HObject(0x5c00_0010)),
        dst: NodeKey::new(HClient(0xBB), HObject(0x9999_0000)),
    };
    let refused = gpu.apply(late_dup);
    assert!(
        matches!(refused, Err(GpuError::LateMerge { .. })),
        "a host-VAS-touched proc cannot be silently merge-absorbed: {refused:?}"
    );
    assert_eq!(gpu.procs.len(), 2, "rollback kept both procs live");
}

/// ★ Mutation-gate kill (`handle_doorbell` ring-gate match guard
/// `None if working_set.is_empty()` → `true` / → `false`): the no-VAS arm of the #14
/// ring-gate is REACHABLE with a live host channel — a guest can `Free` its VASpace
/// handle AFTER the channel materialized (re-projection nulls `chan.vas_pdb`; the host
/// channel persists). In that state the guard is load-bearing on both sides:
///
/// - a NON-EMPTY working set has no address space to have published it → loud
///   `NoVas`, ZERO host ops (the →`true` mutant would skip the gate and ring the
///   host doorbell UNGATED — exactly the #14 cross-VAS class);
/// - an EMPTY working set touches no tracked VA → nothing to fault on, the ring
///   proceeds (the →`false` mutant would refuse the legitimate GSP-managed-shaped
///   ring).
///
/// (An earlier triage called these mutants equivalent by assuming `vas == None` ⇒
/// `host_channel == None`; the free-after-materialize sequence disproves that.)
#[test]
fn cb14_ring_gate_on_vas_freed_channel_refuses_nonempty_allows_empty() {
    let (mut gpu, rec) = fresh_gpu();
    let (_pid, _cid) = build_proc(&mut gpu, HClient(0xAA), A_PDB, (0x10, 0x11));
    let token = MockArch::token_for(VChid(0x10));

    // Materialize + schedule the GR channel with an empty (trivially gated) ring.
    let first =
        handle_doorbell(&mut gpu, GpuId::ZERO, token, &[]).expect("first ring materializes");
    assert!(first.scheduled_now);

    // The guest frees its VASpace HANDLE: re-projection nulls the channel's declared
    // VAS while the materialized host channel survives.
    gpu.apply(RmEvent::Free {
        client: HClient(0xAA),
        handle: HObject(0x5c00_0010),
    })
    .expect("freeing the VASpace handle applies");

    // Non-empty working set on the VAS-less channel: loud NoVas, zero host ops.
    let ops_before = rec.lock().unwrap().log.len();
    let refused = handle_doorbell(&mut gpu, GpuId::ZERO, token, &[VA]);
    assert!(
        matches!(refused, Err(FwdFault::NoVas(_))),
        "a non-empty submission with no declared VAS is refused loudly: {refused:?}"
    );
    assert_eq!(
        rec.lock().unwrap().log.len(),
        ops_before,
        "the refused ring performed ZERO host ops (no ungated doorbell)",
    );

    // Empty working set: nothing to gate — the ring on the still-materialized host
    // channel proceeds (and needs no re-materialization/re-schedule).
    let ok = handle_doorbell(&mut gpu, GpuId::ZERO, token, &[])
        .expect("an empty submission has nothing to gate and rings");
    assert!(
        !ok.scheduled_now,
        "already scheduled — no re-materialization"
    );
}

// =================================================================================
// ★ The device teardown→restart lifecycle (decision #18B's known-gap probe).
// The GSP-reboot FSM half (WPR2/fn-47/seqNum) is NOT MODELED yet (`kayfabe-gsp` is a
// skeleton) — that is a named MILESTONE row in the matrix, not faked here. The
// teardown→recreate half IS core-coverable today: these two tests pin it.
// =================================================================================

/// **Guards the full-device teardown→rebuild cycle** (the C's "turn down, turn up"
/// probe: #12's fn-47 idle-release path, `mode2_12_layered_status` cont.30-32, minus
/// the unmodeled GSP FSM): tear EVERY process down, reap at the quiesce point, then
/// rebuild with IDENTICAL handles/PDBs/VAs. The rebuilt device must be
/// indistinguishable from fresh: everything routes, backs, schedules and completes,
/// on FRESH isolate sessions, with the old procs' arenas RECYCLED (the tight window
/// makes non-recycling loudly impossible).
#[test]
fn cb_lifecycle_full_teardown_reap_rebuild_identical() {
    // Window fits EXACTLY 3 arenas: system + two procs. A rebuild can only succeed
    // if reap actually recycled the torn-down procs' arenas.
    let (mut gpu, rec) = gpu_with_window(0x1_0000_0000..0x1_3000_0000, 0x1000_0000);

    // ---- Generation 1: two procs, real work. ----
    let (pid_a, _) = build_proc(&mut gpu, HClient(0xAA), A_PDB, (0x10, 0x11));
    let (pid_b, _) = build_proc(&mut gpu, HClient(0xBB), B_PDB, (0x20, 0x21));
    publish_backing(
        gpu.procs.get_mut(&pid_a).unwrap(),
        GpuId::ZERO,
        A_PDB,
        VA,
        0x10000,
    )
    .unwrap();
    publish_backing(
        gpu.procs.get_mut(&pid_b).unwrap(),
        GpuId::ZERO,
        B_PDB,
        VA,
        0x10000,
    )
    .unwrap();
    handle_doorbell(&mut gpu, GpuId::ZERO, MockArch::token_for(VChid(0x10)), &[]).unwrap();
    handle_doorbell(&mut gpu, GpuId::ZERO, MockArch::token_for(VChid(0x20)), &[]).unwrap();
    let gen1_arenas: Vec<_> = gpu
        .procs
        .values()
        .map(|p| p.arenas[&GpuId::ZERO].range.clone())
        .collect();
    let gen1_sessions: std::collections::BTreeSet<u32> =
        rec.lock().unwrap().log.iter().map(|(id, _)| id.0).collect();

    // ---- Full teardown: every client root freed. ----
    gpu.apply(RmEvent::Free {
        client: HClient(0xAA),
        handle: HObject(0x5c00_0000),
    })
    .unwrap();
    gpu.apply(RmEvent::Free {
        client: HClient(0xBB),
        handle: HObject(0x5c00_0000),
    })
    .unwrap();
    assert!(gpu.procs.is_empty(), "all procs retired");
    assert!(
        gpu.spine.by_pdb.is_empty() && gpu.spine.by_vchid.is_empty(),
        "no routing residue"
    );
    assert_eq!(
        gpu.spine.retired.len(),
        2,
        "both staged for the deferred reap (L10)"
    );

    // ---- The quiesce point: reap + recycle. ----
    assert_eq!(gpu.reap_retired().len(), 2);
    assert!(
        gpu.spine.retired.is_empty(),
        "reap drained the staging area"
    );

    // ---- Generation 2: IDENTICAL handles, PDBs, VAs. ----
    let (pid_a2, _) = build_proc(&mut gpu, HClient(0xAA), A_PDB, (0x10, 0x11));
    let (pid_b2, _) = build_proc(&mut gpu, HClient(0xBB), B_PDB, (0x20, 0x21));
    let pa2 = publish_backing(
        gpu.procs.get_mut(&pid_a2).unwrap(),
        GpuId::ZERO,
        A_PDB,
        VA,
        0x10000,
    )
    .expect("gen-2 backs the identical VA cleanly — no stale residue");
    let pb2 = publish_backing(
        gpu.procs.get_mut(&pid_b2).unwrap(),
        GpuId::ZERO,
        B_PDB,
        VA,
        0x10000,
    )
    .unwrap();
    assert_ne!(pa2.gpa, pb2.gpa, "gen-2 procs still disjoint");
    // Recycled arenas: gen-2 procs run inside gen-1's ranges (the tight window
    // leaves nothing else) — and that reuse is CLEAN by construction.
    for p in gpu.procs.values() {
        assert!(
            gen1_arenas.contains(&p.arenas[&GpuId::ZERO].range),
            "gen-2 arena {:x?} recycled from gen-1's {gen1_arenas:x?}",
            p.arenas[&GpuId::ZERO].range
        );
    }
    // Fresh channels schedule on fresh exec planes; completions flow.
    assert!(
        handle_doorbell(&mut gpu, GpuId::ZERO, MockArch::token_for(VChid(0x10)), &[])
            .unwrap()
            .scheduled_now
    );
    assert!(
        handle_doorbell(&mut gpu, GpuId::ZERO, MockArch::token_for(VChid(0x20)), &[])
            .unwrap()
            .scheduled_now
    );
    gpu.procs
        .get_mut(&pid_a2)
        .unwrap()
        .completion
        .observe(OsEventRef(0xA2))
        .unwrap();
    assert!(
        gpu.pump_completions(GpuId::ZERO).is_some(),
        "gen-2 completions deliver"
    );

    // Fresh isolate sessions — gen 2 never ran on a dead proc's host session.
    let all_sessions: std::collections::BTreeSet<u32> =
        rec.lock().unwrap().log.iter().map(|(id, _)| id.0).collect();
    assert!(
        all_sessions.len() > gen1_sessions.len(),
        "gen-2 spawned FRESH isolate sessions (gen1={gen1_sessions:?}, all={all_sessions:?})"
    );
}

/// **Guards sustained process churn** (the C's #80 `teardown_hardening_done`: "host
/// reaper + GPA free-list" — the shared window exhausted under create→destroy
/// churn; and the C-era "fresh boot per GPU run" tax, lesson L12, that the rewrite
/// exists to kill): many sequential process generations through a window sized for
/// FOUR arenas must never exhaust, because every reap recycles.
///
/// ★ This regression EXPOSED a real core gap when written: `GpaSpace::carve` never
/// recycled and `Gpu::retired` was never reaped, so generation ~4 died with
/// `WindowExhausted` — exactly the C's leak, silently reintroduced. Fixed by
/// `Gpu::reap_retired` + `GpaSpace::release` (see the matrix doc).
#[test]
fn cb_lifecycle_process_churn_never_exhausts_the_window() {
    // Room for system + 3 concurrent procs only; 24 generations churn through it.
    let (mut gpu, _rec) = gpu_with_window(0x1_0000_0000..0x1_4000_0000, 0x1000_0000);

    for generation in 0..24u64 {
        // Same client handle + PDB + VA every generation — the #12 identical-reuse
        // shape, at device-lifecycle scale.
        let (pid, _cid) = build_proc(&mut gpu, HClient(0xAA), A_PDB, (0x10, 0x11));
        let p = publish_backing(
            gpu.procs.get_mut(&pid).unwrap(),
            GpuId::ZERO,
            A_PDB,
            VA,
            0x10000,
        )
        .unwrap_or_else(|e| panic!("generation {generation}: publish must succeed: {e:?}"));
        let out = handle_doorbell(&mut gpu, GpuId::ZERO, MockArch::token_for(VChid(0x10)), &[])
            .unwrap_or_else(|e| panic!("generation {generation}: doorbell must ring: {e:?}"));
        assert!(
            out.scheduled_now,
            "generation {generation}: fresh channel schedules"
        );
        let (b, _) = resolve(&gpu, GpuId::ZERO, A_PDB, VA).unwrap();
        assert_eq!(
            b.phys, p.gpa,
            "generation {generation}: resolves its OWN backing"
        );

        // Teardown + reap at the quiesce point.
        gpu.apply(RmEvent::Free {
            client: HClient(0xAA),
            handle: HObject(0x5c00_0000),
        })
        .unwrap();
        assert_eq!(
            gpu.reap_retired().len(),
            1,
            "generation {generation}: reaped"
        );
    }
    assert!(
        gpu.spine.retired.is_empty(),
        "no unbounded staging growth across churn"
    );
    assert!(gpu.procs.is_empty());
}
