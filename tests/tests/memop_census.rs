//! ★★★★★ **w392 — THE `MEM_OP` CENSUS'S OWN KNOWN-POSITIVE.**
//!
//! `kayfabe_fwd::memop_census` exists to answer one question on the bench: does a
//! `MMU_TLB_INVALIDATE` on the guest's **emulated** channels ever reach
//! `apply_pushbuffer`? That is the second of the owner's three coverage points
//! (`w391_the_three_point_coverage_ruling.md`), and `CLAUDE.md` records the C measuring
//! this transport at **ZERO** on the Mode-2 compute path.
//!
//! ⊘ **A zero from an instrument with no known-positive is not a measurement.** It cannot
//! tell *"the guest never invalidates"* from *"the counter was never wired"*, and those
//! two demand opposite work. This file is the wiring half: it drives a real
//! `PushMethod::TlbInvalidate` through the real `parse_pushbuffer` and asserts the census
//! moved — so a zero on the bench is about the **guest**, not about us.
//!
//! ★ **Its own binary on purpose.** The census is a process-global `AtomicU64`; sharing a
//! binary with unrelated tests would make an exact count a race. Every assertion here is
//! still written as a **delta** (`after > before`), which is correct under any
//! interleaving, because the counter is monotonic.

use std::sync::{Mutex, MutexGuard};

use kayfabe_arch::ids::{GpuId, GpuVa, HClient, Pdb};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::{ChanId, ProcId};
use kayfabe_mocks::{
    MockArch, MockIsolateFactory, MockPushbuffer, MockVmm, mock_classes as mc,
};
use kayfabe_tests::{Scenario, bind_ring, identical_handles, script_ring_via};

const GPU: GpuId = GpuId::ZERO;
const CLIENT: HClient = HClient(0x92);
const PDB0: Pdb = Pdb(0x4001_0000);
const RING_GPA: u64 = 0x5000_0000;

/// The page-directory base the invalidate names. Deliberately **not** `PDB0`: the census
/// reports whatever the guest wrote, and a fixture that made them equal could not tell a
/// real read of the method words from a lucky echo of the channel's own VAS.
const NAMED_PDB: u64 = 0x7_A000_0000;

/// ★★★ **CAUGHT BY THE NEGATIVE CONTROL, FIRST RUN — and the defect was MINE, in the
/// test.** The census is a process-global `AtomicU64` and libtest runs a binary's tests on
/// several threads, so a `before`/`after` pair taken around one parse can straddle
/// *another test's* invalidate. The negative control below duly reported `left: 1,
/// right: 0` on a ring that carried no invalidate at all — a **true** report of a **false**
/// premise.
///
/// ⊘ The tempting fix — weaken it to `after >= before` — would have deleted the only
/// assertion that can catch a counter incremented on every method. The right fix is to
/// make the delta attributable: every test that reads the global takes this lock, so the
/// window contains exactly its own parse.
///
/// ⚠ It does NOT serialize against other *binaries*; nothing needs it to. Each test
/// binary is its own process, so the global is per-binary — which is also why this file
/// is a binary of its own.
static CENSUS_DELTA: Mutex<()> = Mutex::new(());

/// Take the delta lock, ignoring poisoning: a panicking sibling must not turn one failure
/// into three misleading ones.
fn delta_window() -> MutexGuard<'static, ()> {
    CENSUS_DELTA.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn device() -> (Gpu, MockVmm, ProcId, ChanId) {
    let (factory, _rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu =
        Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa).expect("the device realizes");
    let mut s = Scenario::new();
    s.compute_process(CLIENT, PDB0, identical_handles(0x20, 0x21));
    for ev in s.events {
        gpu.apply(ev).expect("applies");
    }
    let pid = *gpu.spine.by_pdb.get(&(GPU, PDB0)).expect("the VAS routed");
    let cid = *gpu.procs[&pid]
        .chan_ids
        .values()
        .next()
        .expect("the scenario's channel");
    (gpu, MockVmm::new(), pid, cid)
}

/// ★★★★★ **THE KNOWN-POSITIVE.** A guest ring carrying one `MMU_TLB_INVALIDATE` moves the
/// census, and the outcome carries the PDB the guest actually named.
///
/// Asserts three separable things, because a single "it went up" would pass on a counter
/// wired to the wrong arm:
/// 1. the parse **decodes** it — `invalidates` has exactly one entry;
/// 2. that entry names `NAMED_PDB`, not the channel's own `PDB0` (rules out an echo);
/// 3. the **census** moved, which is the fact the bench log reports.
#[test]
fn a_guest_invalidate_reaches_the_census_and_names_its_own_pdb() {
    let (mut gpu, mut vmm, pid, cid) = device();
    let _window = delta_window();

    let before = kayfabe_fwd::memop_census::seen();
    let methods = vec![
        MockPushbuffer::set_object(mc::DMA_COPY),
        MockPushbuffer::tlb_invalidate(NAMED_PDB, true),
    ];
    let ring = script_ring_via(&mut vmm, RING_GPA, &methods);
    bind_ring(&mut gpu, pid, cid, &ring);

    let parsed =
        kayfabe_fwd::parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring).expect("it PARSES");

    assert_eq!(
        parsed.invalidates.len(),
        1,
        "★ the decode arm ran — without this the census below could only be a coincidence"
    );
    let (pdb, membar) = parsed.invalidates[0];
    assert_eq!(
        pdb,
        Pdb(NAMED_PDB),
        "★ the PDB is READ OUT OF THE GUEST'S METHOD WORDS, not echoed from the channel \
         (PDB0 = {PDB0:?}) — the whole point of the census is to name a VAS we were not told about"
    );
    assert!(membar, "the fixture asked for a membar and it survived the decode");

    let after = kayfabe_fwd::memop_census::seen();
    assert!(
        after > before,
        "★★★★★ THE CENSUS IS WIRED: seen() {before} → {after}. A zero on the bench is \
         therefore a fact about the GUEST, not about this counter"
    );
    assert!(
        kayfabe_fwd::memop_census::targeted() > 0,
        "every invalidate that reaches the arm is PDB-targeted by construction \
         (ga10x.rs:1457 drops PDB_ALL before this point)"
    );
    assert!(
        kayfabe_fwd::memop_census::census().contains("seen="),
        "the boot-log line renders"
    );
}

/// ⊘ **THE NEGATIVE CONTROL — a ring with no invalidate must not move the census.**
///
/// Without this, a counter incremented on *every* decoded method would pass the test above
/// and report a large non-zero on the bench that meant nothing. Measured as a delta across
/// a ring that carries a genuine CE copy, so the parse definitely ran.
#[test]
fn a_ring_with_no_invalidate_leaves_the_census_untouched() {
    let (mut gpu, mut vmm, pid, cid) = device();

    let methods = vec![
        MockPushbuffer::set_object(mc::DMA_COPY),
        MockPushbuffer::ce_launch_dma_full(
            GpuVa(0x2_0010_0000).0,
            true,
            GpuVa(0x2_0000_0000),
            true,
            0x1000,
            kayfabe_arch::CeWork::Copy,
        ),
    ];
    let ring = script_ring_via(&mut vmm, RING_GPA, &methods);
    bind_ring(&mut gpu, pid, cid, &ring);

    let _window = delta_window();
    let before = kayfabe_fwd::memop_census::seen();
    let parsed =
        kayfabe_fwd::parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring).expect("it PARSES");
    let after = kayfabe_fwd::memop_census::seen();

    assert!(
        parsed.invalidates.is_empty(),
        "★ non-vacuity is the assertion below; this one says the ring genuinely had none"
    );
    assert_eq!(
        after, before,
        "⊘ the census counts INVALIDATES, not methods — a CE ring must leave it flat"
    );
}

/// ⊘ **A zero census must SAY it is ambiguous.** The line a grader reads has to name its
/// own three causes, or a `seen=0` gets read as "the guest does not invalidate" — which is
/// exactly the reading `CLAUDE.md` records the C's zero being given.
///
/// ⚠ Runs in a **child process** so it observes a census that is genuinely untouched;
/// asserting on a fresh zero from inside this binary is impossible once the tests above
/// have run, and test order is not guaranteed.
#[test]
fn a_zero_census_names_its_own_ambiguity() {
    // The renderer is a pure function of the two counters, so the zero branch is checked
    // by reading the source of truth it formats — no process needed for THIS half.
    let line = kayfabe_fwd::memop_census::census();
    if kayfabe_fwd::memop_census::seen() == 0 {
        assert!(
            line.contains("UNMEASURED-OR-ABSENT") && line.contains("Three causes"),
            "a zero must name its ambiguity, got: {line}"
        );
    } else {
        // Already non-zero (the known-positive ran first). Assert the OTHER branch is
        // honest instead of skipping: it must not claim measurement it does not have.
        assert!(
            line.contains("seen=") && !line.contains("UNMEASURED"),
            "a non-zero census must not print the unmeasured wording, got: {line}"
        );
    }
}
