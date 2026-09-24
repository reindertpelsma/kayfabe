//! # ★★★★★ **CONSTRAINT 27 — A REFRESH MAY NOT COMPLETE UNTIL ITS UNMAPS HAVE LANDED**
//!
//! > Owner, 2026-09-15: *"Before a refresh finishes, this kernel channel has unmapped slices
//! > the guest userspace no longer has access to. So the guest kernel knows: okay,
//! > invalidate done, I can reuse this phys for another userspace process safely after a
//! > scrub."*
//!
//! ⇒ **This is a GUEST-INTERNAL isolation invariant and we are the only thing that can break
//! it.** If the refresh reports the TLB invalidate complete before its unmaps land, the guest
//! kernel reuses a physical page a guest **userspace** process can still reach through a
//! stale slice — a cross-process leak *inside* the guest, caused by us, and **invisible to
//! the guest**.
//!
//! ## ⚠ The shape the constraint demands, in its own words
//!
//! > *"**Needs a known-positive**: stall an unmap and assert the invalidate does **not**
//! > complete. A test that only checks unmaps happen cannot tell "before" from
//! > "eventually"."*
//!
//! So the stall here is **real, not simulated**. `SharedDevice::drain_pending_releases`
//! walks the **live** procs only — `core::iter::once(Gpu::SYSTEM_PROC).chain(st.procs.keys())`
//! — so a proc that has already retired owes a queue that call structurally cannot clear.
//! That is a stalled unmap produced by the production code's own control flow, with no mock
//! refusal and no injected failure, and the test then shows:
//!
//! 1. with it outstanding, the invalidate is **withheld** and `TRIGGER` stays set;
//! 2. the ordinary drain does **not** clear it (the stall is real, not a timing artefact);
//! 3. once the retired-proc drain empties the queue, the *same call* completes and `TRIGGER`
//!    clears.
//!
//! ⊘ Step 3 is the control. Without it a green in step 1 would be equally consistent with
//! *"this function never completes anything"*, which is a hang wearing a barrier's clothes —
//! and a hang is the one way a fix for a leak can be worse than the leak.
//!
//! ## ⊘ What this file does NOT witness
//!
//! It is mock-driven and GPU-free. It judges the **barrier**: which verdict comes back for
//! which queue state, and what the guest's poll reads in each. It judges nothing about how
//! long a real RM unmap takes, and nothing about the shim's arm — that half is
//! `the_invalidate_arm_orders_unmaps_before_maps` below, which is a source census and says
//! so.

use std::sync::Arc;

use kayfabe_arch::Aperture;
use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::rmgraph::RmEvent;
use kayfabe_device::mmuinval::{CompletionVerdict, MmuInvalidateLog, TriggerAction};
use kayfabe_isolate::{GuestRamGrant, HostHandle};
use kayfabe_mmu::Binding;
use kayfabe_mocks::watchdog;
use kayfabe_mocks::{MockArch, MockIsolateFactory, SharedRecorder};
use kayfabe_rt::device::{LockMode, SharedDevice};
use kayfabe_tests::{Guarded, Scenario, identical_handles};
use kayfabe_vmm::Prot;

const GPU: GpuId = GpuId::ZERO;
const CLIENT: HClient = HClient(0xA0);
const PDB: Pdb = Pdb(0x3400_0000);
const GR: VChid = VChid(0x100);
const CE: VChid = VChid(0x200);
const MEM: HObject = HObject(0x6000_0000);

/// ⊘ The same addresses `budgeted_drain.rs` and `guest_ram_pin_release.rs` use — a test
/// whose addresses look nothing like the boot's cannot be read beside the boot.
const RING_VA: GpuVa = GpuVa(0x4_2006_4000);
const RING_GPA: u64 = 0x0768_a000;
const RING_FILE_OFFSET: u64 = 0x1_0000_0000 + 0x0768_a000;
const PIN_LEN: u64 = 4096;
const GUEST_RAM_BYTES: u64 = 0x2_0000_0000;
const PINS: u64 = 8;

/// The raw value a guest writes to `MMU_INVALIDATE` to commit: `TRIGGER` (bit 31) set.
const TRIGGER_WRITE: u32 = 1 << 31;

fn device() -> (
    Guarded<Arc<SharedDevice>>,
    kayfabe_core::ProcId,
    SharedRecorder,
) {
    let (factory, recorder) = MockIsolateFactory::with_pool_size(2);
    let factory = factory.with_guest_ram(GUEST_RAM_BYTES);
    let gpa = GpaSpace::new(0x10_0000_0000..0x1000_0000_0000, 0x10_0000_0000);
    let mut gpu =
        Gpu::new(std::sync::Arc::new(MockArch::new()), Box::new(factory), gpa).expect("realizes");
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
            "unmap_before_completion::device",
            Arc::new(SharedDevice::new(gpu, LockMode::Sharded)),
            recorder.clone(),
        ),
        pid,
        recorder,
    )
}

/// Declare `PINS` guest-bound pages and pin every one, so the proc dies owing a real queue
/// of unmap + free + munmap triples.
fn guest_pins_pages(device: &SharedDevice, pid: kayfabe_core::ProcId) -> Vec<HostHandle> {
    device
        .with_proc_mut(pid, |p| {
            let vas = p.vas_by_pdb_mut(GPU, PDB).expect("the compute VAS");
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

/// An armed log with one trigger outstanding, and the `seq` a worker would have snapshotted.
fn armed_with_one_trigger() -> (MmuInvalidateLog, u64) {
    let log = MmuInvalidateLog::new();
    log.arm();
    let (_inv, act) = log.note_trigger(TRIGGER_WRITE, 1_000);
    assert!(
        matches!(act, TriggerAction::Publish),
        "★ NON-VACUITY: the fixture's trigger did not ask for a publication, so every \
         assertion below would be about a log that was never holding the guest. act={act:?}"
    );
    assert_eq!(
        log.read_trigger(),
        1 << 31,
        "★ NON-VACUITY: TRIGGER must be set before we ask whether it clears"
    );
    let seq = log.issued();
    (log, seq)
}

// =====================================================================================
// ★★★★★ THE GATE — a stalled unmap withholds the completion, and a drained one releases it
// =====================================================================================

#[test]
fn a_stalled_unmap_withholds_the_invalidate_and_the_drain_releases_it() {
    let _wd = watchdog(
        "unmap_before_completion::the_gate",
        std::time::Duration::from_secs(60),
    );
    let (device, pid, _rec) = device();
    let descriptors = guest_pins_pages(&device, pid);
    assert_eq!(
        descriptors.len(),
        PINS as usize,
        "★ NON-VACUITY: the fixture pinned nothing"
    );

    // ---- THE STALL. The proc retires owing its queue, and `drain_pending_releases` walks
    //      LIVE procs only, so it structurally cannot clear a corpse's debt. ⊘ No mock
    //      refusal, no injected failure: this is the production control flow's own hole.
    guest_tears_the_process_down(&device);
    assert_eq!(device.retired_len(), 1, "the teardown retired the proc");
    let owed = device.staged_release_len();
    assert!(
        owed >= 3 * PINS as usize,
        "★ NON-VACUITY: the dead proc must owe at least one unmap + one free + one munmap \
         per pin, or every assertion below is a statement about an empty queue. owed={owed}"
    );

    let drained = device.drain_pending_releases();
    assert_eq!(
        device.staged_release_len(),
        owed,
        "★★★ THE STALL IS REAL, not a race. The ordinary drain ran (disposed={drained}) and \
         the queue is unchanged, because the debt belongs to a retired proc. This assertion \
         is what makes the withholding below a measurement rather than a coincidence."
    );

    // ---- 1. ★★★★★ THE BARRIER. TRIGGER must stay set.
    let (log, seq) = armed_with_one_trigger();
    let verdict = log.complete_through_unmaps(seq, 2_000, device.staged_release_len());
    assert_eq!(
        verdict,
        CompletionVerdict::WithheldUnmapsOutstanding { n: owed },
        "★★★★★ CONSTRAINT 27 — {owed} staged disposal(s) had NOT landed and the invalidate \
         was reported complete anyway. That tells the guest kernel it may recycle a physical \
         page a guest USERSPACE process can still reach through a slice we have not taken \
         down: a cross-process leak inside the guest, caused by us, invisible to it."
    );
    assert_eq!(
        log.read_trigger(),
        1 << 31,
        "★★★★★ …and the guest must still SEE it pending. A verdict that says `withheld` over \
         a register that reads `done` is a barrier that reports rather than gates."
    );
    assert!(
        log.pending(),
        "the pending flag and the register must agree"
    );

    // ---- 2. ⊘ THE CONTROL. Drain the corpse, and the SAME call must complete.
    //      Without this, step 1 is equally consistent with "this function never completes".
    for _ in 0..256 {
        let _ = device.drain_retired_budgeted(64, || false);
        if device.staged_release_len() == 0 {
            break;
        }
    }
    assert_eq!(
        device.staged_release_len(),
        0,
        "★ NON-VACUITY: the control could not clear the queue, so the completion below would \
         be withheld for the right reason by accident"
    );
    let verdict = log.complete_through_unmaps(seq, 3_000, device.staged_release_len());
    assert_eq!(
        verdict,
        CompletionVerdict::Completed,
        "⊘ THE CONTROL — with every unmap landed the guest MUST be released. A barrier that \
         never completes is a hang, which is worse than the leak it prevents."
    );
    assert_eq!(
        log.read_trigger(),
        0,
        "…and the guest's poll must see it. This is the edge RM's \
         `kgmmuCheckPendingInvalidates` spins on."
    );

    // ---- 3. the census carries the withholding, so a boot can be read for it.
    let snap = log.snapshot();
    assert_eq!(
        (snap.withheld_unmaps, snap.worst_unmaps_outstanding),
        (1, owed),
        "★ The withholding must be COUNTED. A barrier that fires silently cannot be \
         distinguished at teardown from one that is not wired at all — this tree's \
         `a census zero needs a known-positive` lesson, applied to the census itself."
    );
}

#[test]
fn a_newer_trigger_and_an_outstanding_unmap_are_different_verdicts() {
    // ⊘⊘ THE DISTINCTION THAT IS A HANG IF IT IS LOST. `WithheldNewerTrigger` means somebody
    // else completes this one; `WithheldUnmapsOutstanding` means NOBODY will and the caller
    // owes a retry. The previous `complete_through` returned `bool` for both, which is why
    // this arm exists at all.
    let (log, stale_seq) = armed_with_one_trigger();
    // A second trigger arrives while the first job is still in flight.
    let _ = log.note_trigger(TRIGGER_WRITE, 1_500);
    let verdict = log.complete_through_unmaps(stale_seq, 2_000, 0);
    assert_eq!(
        verdict,
        CompletionVerdict::WithheldNewerTrigger {
            issued: log.issued()
        },
        "★★★ A stale seq must be answered as a NEWER TRIGGER even with zero unmaps \
         outstanding — the two tests are ordered so the arm that owes nothing wins."
    );
    assert_eq!(
        log.snapshot().withheld_unmaps,
        0,
        "⊘ …and it must NOT be counted as constraint 27's withholding. Reading a newer \
         trigger as an unmap debt would make the boot census say the barrier fired when it \
         did not."
    );

    // The converse: a current seq with an outstanding unmap is the OTHER arm.
    let fresh_seq = log.issued();
    assert_eq!(
        log.complete_through_unmaps(fresh_seq, 2_500, 7),
        CompletionVerdict::WithheldUnmapsOutstanding { n: 7 }
    );
}

// =====================================================================================
// The shim's arm — a source census, and it says so
// =====================================================================================

fn shim_rs() -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../crates/kayfabe-qemu-raw/src/shim.rs");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// ★★★ **CONSTRAINT 27's ORDERING HALF.** *"unmaps must be ordered **before** maps within a
/// refresh."*
///
/// ⊘ A source census because the arm is a `loop` inside a spawned publication thread that
/// needs a live `RegPlane`, a `BarMirror` and a QEMU machine — there is no fixture that can
/// enter it, and building one would be building a second publication worker whose ordering
/// could differ from the real one's, which is the failure this would be testing for.
#[test]
fn the_invalidate_arm_orders_unmaps_before_maps() {
    let code = shim_rs();
    let arm = code
        .find("if job.kind() == kayfabe_device::pubqueue::PublicationKind::Invalidate {")
        .expect("★ NON-VACUITY: the invalidate arm is gone — this gate gates nothing");
    let body = &code[arm..];

    let unmap_first = body.find("plane.revalidate_mirror_first();").expect(
        "★★★★★ CONSTRAINT 27 REGRESSED — the invalidate arm no longer retires stale \
             mirror slots before it publishes. `drain_mirror_revalidation` cannot serve this: \
             it runs `drain_fills()` FIRST, so using it here installs new memslots before the \
             stale ones are gone — maps before unmaps.",
    );
    let publish = body
        .find("let published = ctx.publish_vas_rows(token, None, off_vcpu);")
        .expect("★ NON-VACUITY: the publication call moved; this gate is comparing nothing");
    assert!(
        unmap_first < publish,
        "★★★★★ CONSTRAINT 27 REGRESSED — the unmap-first pass now runs AFTER the \
         publication. Within one refresh every unmap must precede every map, or the guest \
         is told its tables are settled while a slice it revoked is still installed."
    );

    let premap = body
        .find("m.premap_bars();")
        .expect("★ NON-VACUITY: `premap_bars` is gone from the invalidate arm");
    assert!(
        unmap_first < premap,
        "★★★★★ CONSTRAINT 27 REGRESSED — the BAR premap (a MAP) now runs before the \
         unmap-first pass."
    );

    let complete = body.find("complete_through_unmaps(").expect(
        "★★★★★ CONSTRAINT 27 REGRESSED — the arm completes through the UNGATED \
             `complete_through`. That function cannot see the staged unmaps and will release \
             the guest over a live mapping.",
    );
    assert!(
        premap < complete && publish < complete,
        "★ every publication step must precede the completion — the pre-existing ordering \
         this change must not have disturbed"
    );
    assert!(
        body[..complete].contains("refresh.unmaps_outstanding"),
        "★★★★★ CONSTRAINT 27 REGRESSED — the completion is no longer fed the refresh's own \
         outstanding count. A literal `0` here is the barrier deleted with its call site \
         left in place, which is the shape nothing goes red for."
    );
}

/// ⊘ The outstanding count must come from the QUEUE, never from the drain's return value.
#[test]
fn the_outstanding_count_is_read_off_the_queue_and_not_off_the_drain() {
    let code = shim_rs();
    let at = code
        .find("fn refresh_page_tables(")
        .expect("★ NON-VACUITY: `refresh_page_tables` is gone");
    let end = code[at..]
        .find("\n    }\n")
        .map(|o| at + o)
        .expect("a closing brace");
    let body = &code[at..end];
    assert!(
        body.contains("self.device.staged_release_len()"),
        "★★★★★ CONSTRAINT 27 REGRESSED — the refresh no longer asks the QUEUE how much is \
         owed. `drain_pending_releases` returns `0` both for \"nothing was owed\" and for \
         \"the pool was full so nothing could be issued\" — its own docs say it SKIPS — so a \
         number derived from it cannot answer the question the barrier asks."
    );
    assert!(
        body.contains("drain_trips < PT_DRAIN_TRIPS_MAX"),
        "★★★ CONSTRAINT 27 — the re-drain must be a FIXED TRIP COUNT. The queue's length is \
         a function of guest activity, so `while staged > 0` is a loop that terminates on \
         guest data."
    );
}
