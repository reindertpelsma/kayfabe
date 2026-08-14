//! # ★★★★★ **w317 — IS THE BQL DISPOSAL BOUNDED, AND DOES THE REMAINDER STILL GET DONE?**
//!
//! ## The number this file exists for
//!
//! `[measured 2026-08-14 (w314), bench vh, real GA106, n=4 per arm, non-overlapping ranges]`
//!
//! | arm | `max_reap_us` | vs `scrubberDestruct`'s 4 000 ms |
//! |---|---|---|
//! | clean master `eb3d99ad` | 2 648 366 · 2 918 210 · 2 666 893 · 2 772 771 | **73.0 %** |
//! | + w310's pin release | 3 336 519 · **3 702 806** · 3 263 826 · 3 250 535 | **92.6 %** |
//!
//! Every guest MMIO write arrives with the QEMU **BQL** held, so that is not "a slow vCPU" —
//! it is **the whole VM frozen**, main loop and timers included
//! (`blocking_and_completion_model.md` §0). `INLINE-SAFE` clause (b) fails, on the standard
//! workload, on a green boot.
//!
//! ## ⊘ Why the property is NOT "the drain is fast"
//!
//! A budget that only bounds is trivially satisfiable by never draining, and *"a drain that
//! defers indefinitely is a leak with extra steps."* The two halves have to be asserted
//! together, so the property this file gates is:
//!
//! > **A bounded drain disposes of AT MOST the budget per turn, and of EXACTLY the same
//! > objects, exactly once, as the unbudgeted one — given enough turns.**
//!
//! Both halves have their own test, plus a third for the ordering the split must not break
//! and a fourth for the reap gate that makes the bound bind at all.
//!
//! ## ⊘ What this file does NOT witness
//!
//! - **It is mock-driven and GPU-free.** It judges reachability, bounding, ordering and
//!   totals. It judges **nothing** about how long a real RM `Free` takes, which is the whole
//!   of the clause-(b) claim — `only_live_boots_are_proof`, and the bench numbers are
//!   pre-registered in this rung's report. What it *can* do that a boot cannot is exercise
//!   the exact control flow of a spent budget deterministically, with no clock: the deadline
//!   is a closure (`SharedDevice::drain_retired_budgeted`), so "the budget ran out on turn
//!   two" is a test input rather than a race.
//! - **It does not measure the granularity's overshoot.** `RETIRED_DRAIN_CHUNK`'s cost is a
//!   property of the host, not of this fixture.

use std::sync::Arc;

use kayfabe_arch::Aperture;
use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::rmgraph::RmEvent;
use kayfabe_isolate::{GuestRamGrant, HostHandle, IsolateId, Orphans};
use kayfabe_mmu::Binding;
use kayfabe_mocks::watchdog;
use kayfabe_mocks::{MockArch, MockIsolateFactory, RmVerb, SharedRecorder};
use kayfabe_rt::device::{LockMode, SharedDevice};
use kayfabe_tests::{Guarded, Scenario, identical_handles};
use kayfabe_vmm::Prot;

const GPU: GpuId = GpuId::ZERO;
const CLIENT: HClient = HClient(0xA0);
const PDB: Pdb = Pdb(0x3400_0000);
const GR: VChid = VChid(0x100);
const CE: VChid = VChid(0x200);
const MEM: HObject = HObject(0x6000_0000);

/// ⊘ The same addresses `guest_ram_pin{,_release}.rs` use, and for their stated reason: a
/// test whose addresses look nothing like the boot's cannot be read beside the boot.
const RING_VA: GpuVa = GpuVa(0x4_2006_4000);
const RING_GPA: u64 = 0x0768_a000;
const RING_FILE_OFFSET: u64 = 0x1_0000_0000 + 0x0768_a000;
const PIN_LEN: u64 = 4096;
const GUEST_RAM_BYTES: u64 = 0x2_0000_0000;

/// How many pages the guest pins before it dies. ★ Chosen so that **several** budgeted turns
/// are needed at the chunk sizes below — a fixture that fits in one turn would exercise the
/// unbudgeted path under a budgeted name and be green for the wrong reason.
const PINS: u64 = 8;

/// One guest proc on GPU0 whose isolates can see guest memory — `guest_ram_pin_release.rs`'s
/// fixture, unchanged, because this file's subject is *when* that one's disposal runs.
fn device() -> (
    Guarded<Arc<SharedDevice>>,
    kayfabe_core::ProcId,
    SharedRecorder,
) {
    let (factory, recorder) = MockIsolateFactory::with_pool_size(2);
    let factory = factory.with_guest_ram(GUEST_RAM_BYTES);
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
            "budgeted_drain::device",
            Arc::new(SharedDevice::new(gpu, LockMode::Sharded)),
            recorder.clone(),
        ),
        pid,
        recorder,
    )
}

/// Declare `PINS` guest-bound pages and pin every one of them, so the proc dies owing a
/// queue big enough to need several budgeted turns.
///
/// Returns the descriptors, which is what the totals below are compared against.
fn guest_pins_pages(device: &SharedDevice, pid: kayfabe_core::ProcId) -> Vec<HostHandle> {
    device
        .with_proc_mut(pid, |p| {
            let vas = p.vases.get_mut(&(GPU, PDB)).expect("the compute VAS");
            for i in 0..PINS {
                vas.table
                    .bind(
                        PDB,
                        GpuVa(RING_VA.0 + i * PIN_LEN),
                        PIN_LEN,
                        Binding::declared_by_guest(
                            RING_GPA + i * PIN_LEN,
                            Aperture::SysmemCoherent,
                        )
                        .expect("the fixture declares a kind the guest can declare"),
                    )
                    .expect("the fixture's own bind is well-formed");
            }
        })
        .expect("the proc is live");
    (0..PINS)
        .map(|i| {
            device
                .pin_guest_ram(
                    GPU,
                    PDB,
                    GpuVa(RING_VA.0 + i * PIN_LEN),
                    GuestRamGrant::originated_by_the_vmm(
                        RING_FILE_OFFSET + i * PIN_LEN,
                        PIN_LEN,
                        Prot::ReadWrite,
                    ),
                )
                .expect("the pin runs")
                .memory
        })
        .collect()
}

/// The guest frees its own client root — the production teardown, naming no pin.
fn guest_tears_the_process_down(device: &SharedDevice) {
    device
        .apply(RmEvent::Free {
            client: CLIENT,
            handle: identical_handles(GR.0, CE.0).client_root,
        })
        .expect("the guest frees its own client root");
}

/// Every handle the mock backend recorded a successful `Free` of.
fn freed(rec: &SharedRecorder) -> Vec<HostHandle> {
    rec.lock()
        .expect("recorder")
        .log
        .iter()
        .filter_map(|(_, v)| match v {
            RmVerb::Free { obj } => Some(*obj),
            _ => None,
        })
        .collect()
}

/// How many isolate-side guest-RAM windows were `munmap`ed.
fn guest_ram_unmaps(rec: &SharedRecorder) -> usize {
    rec.lock()
        .expect("recorder")
        .log
        .iter()
        .filter(|(_, v)| matches!(v, RmVerb::UnmapGuestRam { .. }))
        .count()
}

/// Every disposal verb, in issue order, as a coarse kind — the sequence the split must not
/// reorder.
fn disposal_order(rec: &SharedRecorder) -> Vec<&'static str> {
    rec.lock()
        .expect("recorder")
        .log
        .iter()
        .filter_map(|(_, v)| match v {
            RmVerb::UnmapGpuVa { .. } => Some("unmap"),
            RmVerb::Free { .. } => Some("free"),
            RmVerb::UnmapGuestRam { .. } => Some("munmap"),
            _ => None,
        })
        .collect()
}

// =================================================================================
// ★★★★★ THE GATE
// =================================================================================

/// ★★★★★ **THE GATE — a spent budget disposes of a BOUNDED slice, and the remainder is
/// finished by later turns of the same recurring edge.**
///
/// Read the phases as one sentence: *the guest pins eight pages and dies; one budgeted turn
/// disposes of at most the chunk and the reap refuses to take the proc; more turns finish the
/// job; and only then does the proc reap — having disposed of exactly what an unbudgeted reap
/// would have, exactly once.*
///
/// ★ **Phase 2 is the whole claim and it is the half a "is it fast?" test cannot make.** The
/// bound is asserted as *the reap did not happen and the verb count did not exceed the
/// chunk*, which is a statement about **what ran inside one trap** — the only thing clause
/// (b) is about.
#[test]
fn a_spent_budget_disposes_a_bounded_slice_and_finishes_the_rest_later() {
    let _wd = watchdog(
        "budgeted_drain::the_gate",
        std::time::Duration::from_secs(60),
    );
    let (device, pid, rec) = device();
    let descriptors = guest_pins_pages(&device, pid);

    // ---- phase 1: NON-VACUITY. The queue must be big enough to need several turns.
    guest_tears_the_process_down(&device);
    assert_eq!(
        device.retired_len(),
        1,
        "the teardown really did retire the proc"
    );
    let owed = device
        .with_retired(|procs| procs.iter().map(|p| p.pending_release_len()).sum::<usize>());
    assert!(
        owed >= 3 * PINS as usize,
        "★ NON-VACUITY: the dead proc must owe at least one unmap + one free + one munmap per \
         pin, or every bound below is a statement about an empty queue. owed={owed}"
    );

    // ---- phase 2: ★★★ ONE TURN, and it is BOUNDED and does NOT reap.
    const CHUNK: usize = 5;
    let mut turns = 0usize;
    let stats = device.drain_retired_budgeted(CHUNK, || {
        turns += 1;
        turns >= 1 // the budget is spent after exactly one turn
    });
    assert!(stats.budget_hit, "the fixture must actually spend the budget");
    assert_eq!(
        stats.turns, 1,
        "exactly one plan→execute→check-in turn ran before the deadline was read"
    );
    assert!(
        stats.disposed + stats.residue <= CHUNK,
        "★★★★★ THE BOUND. One turn disposed of {} + {} = more than the {CHUNK}-disposal \
         chunk it was given. This is the assertion the whole rung exists for: what runs \
         inside ONE guest MMIO trap, with the BQL held and every vCPU halted, must be \
         capped. w314 measured the uncapped version at 2.65–3.70 s against a 4 000 ms guest \
         timeout.",
        stats.disposed,
        stats.residue,
    );
    let (reaped, held) = device.reap_retired_held();
    assert_eq!(
        (reaped, held),
        (0, 1),
        "★★★★★ THE GATE THAT MAKES THE BOUND BIND. The proc is quiesced and would have been \
         reaped — and `Proc::drop` would then have issued THE WHOLE REMAINDER in one \
         unbounded burst inside this same trap, which is exactly the stall being fixed. It \
         must be HELD, and held for the drain's reason (`deferred_for_drain`), not silently \
         inside the pre-existing `deferred` count."
    );
    assert!(
        device
            .with_retired(|procs| procs.iter().map(|p| p.pending_release_len()).sum::<usize>())
            > 0,
        "…and the remainder is still queued, still named, still reachable — deferred, not lost"
    );

    // ---- phase 3: the edge RECURS. Nothing here is new machinery: `Regs::write` runs on
    //      every guest MMIO write, so "a later turn" is the guest's next register access.
    for _ in 0..64 {
        let _ = device.drain_retired_budgeted(CHUNK, || false);
        if device.reap_retired_held() == (1, 0) {
            break;
        }
    }
    assert_eq!(
        device.retired_len(),
        0,
        "★★★★★ TERMINATION. A retired proc's queue is CLOSED — it is out of every routing \
         map and refuses every new op — so it strictly decreases and must empty. A drain \
         that defers indefinitely is a leak with extra steps, and this is the assertion that \
         it does not."
    );

    // ---- phase 4: ★★ and EXACTLY THE SAME WORK GOT DONE. Bounding must not lose or
    //      duplicate a disposal; the totals are the same ones `guest_ram_pin_release.rs`
    //      asserts for a single pin, times `PINS`.
    for d in &descriptors {
        assert_eq!(
            freed(&rec).iter().filter(|h| *h == d).count(),
            1,
            "★★ EXACTLY ONCE, per descriptor. A split that re-queued what it had already \
             issued would double-free {d:?}; one that dropped a batch would leak it. \
             Neither is visible from a count of the whole set, which is why this is per \
             handle."
        );
    }
    assert_eq!(
        guest_ram_unmaps(&rec),
        PINS as usize,
        "★★ THE ISOLATE'S OWN WINDOWS — all of them, across the batch boundaries. This is \
         the kind that runs LAST in a `Release` plan, so it is the one a naive split would \
         lose first."
    );
    let t = device.pin_reclaim_gone();
    assert_eq!(
        (t.released, t.refused_no_host_vas),
        (PINS as usize, 0),
        "and the tally the `PIN-RELEASE` boot line prints is unchanged by the budget"
    );
}

/// ★★★★★ **THE UNBUDGETED CONTROL — the same fixture, the same totals, no budget.**
///
/// ⊘ Without this, phase 4 above is a number with nothing to be equal *to*: it would be
/// asserting that the budgeted path disposes of `PINS` descriptors because the test says
/// `PINS`, not because that is what the disposal actually owes. This arm derives the
/// expectation from the production path itself.
///
/// ★ It is also the known-negative for the reap gate: under [`SharedDevice::reap_retired`]
/// (`ReapPolicy::Unbudgeted`) the proc reaps **immediately**, disposal and all — which is
/// master's behaviour, and is why every existing caller of that method is untouched by this
/// rung.
#[test]
fn the_unbudgeted_reap_still_disposes_of_everything_in_one_call() {
    let _wd = watchdog(
        "budgeted_drain::control",
        std::time::Duration::from_secs(60),
    );
    let (device, pid, rec) = device();
    let descriptors = guest_pins_pages(&device, pid);
    guest_tears_the_process_down(&device);

    assert_eq!(
        device.reap_retired(),
        1,
        "★ the UNBUDGETED arm reaps on the first call — unchanged from master, and the \
         reason this rung is additive for every caller that is not `Regs::write`"
    );
    assert_eq!(device.retired_len(), 0, "and nothing is held back");
    for d in &descriptors {
        assert_eq!(
            freed(&rec).iter().filter(|h| *h == d).count(),
            1,
            "the control disposes of {d:?} exactly once too — the budgeted arm's totals are \
             compared against THIS, not against a hand-written constant"
        );
    }
    assert_eq!(guest_ram_unmaps(&rec), PINS as usize);
}

/// ★★★★★ **THE ORDER SURVIVES THE BATCH BOUNDARIES — every `unmap` before every `free`,
/// every `free` before every `munmap`.**
///
/// `Worker::execute`'s `Release` arm runs the three kinds in that order and both orderings
/// are load-bearing: unmap-before-free protects **our mirror** of the mapping, and
/// free-before-`munmap` is `Orphans::guest_ram`'s stated invariant — *"the GPU's access
/// outlives our view of the pages"* is a state no reader should ever have to reason about.
///
/// ⊘ **A split that took `budget/3` from each kind would break both**, and would do it
/// silently: every object would still be disposed of exactly once, so both the per-handle
/// counts and the totals above would stay green. This is the assertion that a passing
/// `EXACTLY ONCE` cannot make.
#[test]
fn the_release_order_is_preserved_across_budget_boundaries() {
    let _wd = watchdog("budgeted_drain::order", std::time::Duration::from_secs(60));
    let (device, pid, rec) = device();
    let _ = guest_pins_pages(&device, pid);
    guest_tears_the_process_down(&device);

    // A chunk of 3 against 8 pins: the boundaries land *inside* every kind.
    for _ in 0..64 {
        let _ = device.drain_retired_budgeted(3, || false);
        if device.reap_retired_held() == (1, 0) {
            break;
        }
    }
    assert_eq!(device.retired_len(), 0, "the fixture completed");

    let order = disposal_order(&rec);
    let last_unmap = order.iter().rposition(|k| *k == "unmap");
    let first_free = order.iter().position(|k| *k == "free");
    let last_free = order.iter().rposition(|k| *k == "free");
    let first_munmap = order.iter().position(|k| *k == "munmap");
    assert!(
        last_unmap.is_some() && first_free.is_some() && first_munmap.is_some(),
        "★ NON-VACUITY: all three kinds must appear, or the orderings below hold vacuously. \
         {order:?}"
    );
    assert!(
        last_unmap < first_free,
        "★★★ EVERY `unmap` BEFORE EVERY `free`, across the batch boundaries. \
         last_unmap={last_unmap:?} first_free={first_free:?} order={order:?}"
    );
    assert!(
        last_free < first_munmap,
        "★★★ EVERY `free` BEFORE EVERY `munmap`. RM's `OS_DESCRIPTOR` pins the pages \
         independently of our mapping, so a `munmap` that overtook a `free` would drop OUR \
         window while the GPU's translation is still live. \
         last_free={last_free:?} first_munmap={first_munmap:?} order={order:?}"
    );
}

/// ★★★ **`Orphans::split_off_budget` in isolation** — the cutting edge, with no device,
/// no proc and no isolate around it.
///
/// The three properties the whole rung rests on: it never takes more than the budget, it
/// never loses or duplicates an entry, and it fills kind by kind rather than proportionally.
#[test]
fn the_split_takes_at_most_the_budget_and_loses_nothing() {
    let iso = IsolateId::new(1, GPU);
    let mut q = Orphans {
        unmap: (0..5)
            .map(|i| (HostHandle::new(iso, 0x100 + i), 0x1000 * i))
            .collect(),
        free: (0..5).map(|i| HostHandle::new(iso, 0x200 + i)).collect(),
        guest_ram: Vec::new(),
    };
    let total = q.len();

    let first = q.split_off_budget(3);
    assert_eq!(first.len(), 3, "at most — and here exactly — the budget");
    assert_eq!(
        (first.unmap.len(), first.free.len()),
        (3, 0),
        "★ KIND BY KIND, not proportionally: `unmap` is filled to exhaustion before `free` \
         is touched at all. That is what makes the global order survive the boundary."
    );
    let second = q.split_off_budget(4);
    assert_eq!(
        (second.unmap.len(), second.free.len()),
        (2, 2),
        "the second batch finishes `unmap` and only then starts `free`"
    );
    let third = q.split_off_budget(99);
    assert_eq!(third.len(), 3, "a budget larger than the remainder takes it all");
    assert_eq!(q.len(), 0, "…and the queue is empty");
    assert_eq!(
        first.len() + second.len() + third.len(),
        total,
        "★★ NOTHING LOST, NOTHING DUPLICATED: the batches partition the queue"
    );

    let mut empty = Orphans::default();
    assert_eq!(empty.split_off_budget(10).len(), 0, "an empty queue splits to nothing");
    let mut q2 = Orphans {
        unmap: vec![(HostHandle(1), 0)],
        free: Vec::new(),
        guest_ram: Vec::new(),
    };
    assert_eq!(
        q2.split_off_budget(0).len(),
        0,
        "⊘ a ZERO budget takes NOTHING — which is what makes a budget-exhausted caller a \
         no-op rather than a spin, and is why the shim asserts its chunk is non-zero at \
         compile time"
    );
    assert_eq!(q2.len(), 1, "…and leaves the queue untouched");
}
