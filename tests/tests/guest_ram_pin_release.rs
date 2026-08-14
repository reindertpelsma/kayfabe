//! # ★★★★★ **w310 — ARE A DEAD PROC'S GUEST-RAM PINS ACTUALLY RELEASED?**
//!
//! ## ⊘ The gate that was missing, stated as the thing it is not
//!
//! `docs/audits/w301_cancellation_error_leaks.md` §3.2, verified still true at master
//! `74200b2b`: [`kayfabe_core::gpu::Vas::guest_ram_pins`] had **no `remove`, `retain`,
//! `clear` or `drain` anywhere in the tree**, and `Spine::stage_dropped_vases` *"walks
//! `vas.table` and `vas.blocks` only — it never consults `guest_ram_pins`."* Dropping the
//! `Vas` dropped the map and lost the handles, so:
//!
//! > **the host GPU kept a live, RM-pinned translation into guest pages the guest had
//! > freed** — and w307 measured that the fault everyone assumed would announce it
//! > **cannot occur**: for published rows we refuse the guest's unbind, so the translation
//! > we keep is precisely the one the engine would otherwise have faulted on. **No fault,
//! > no `Xid`, no notifier.** A silent cross-process write inside the guest.
//!
//! `tests/tests/guest_ram_pin.rs` was green throughout. It asserts the pin **chain** —
//! order, idempotence, placement, refusal names — which is a claim about *making* a pin.
//! Nothing in this workspace asserted anything about *unmaking* one, because there was
//! nothing to assert.
//!
//! ⇒ The property this file exists for is deliberately not *"the release works"*:
//!
//! > **A guest that tears its own process down releases that process's guest-RAM pins,
//! > through the production path, with nothing the test does itself.**
//!
//! ## ★★★ The known-positive, watched failing before this file was committed
//!
//! Deleting the pin block from `Spine::stage_dropped_vases`
//! (`crates/kayfabe-core/src/gpu.rs`, the `for (_va, pin) in vas.take_guest_ram_pins()`
//! loop) makes [`a_dead_procs_guest_ram_pins_are_released_from_the_production_path`] fail
//! on
//!
//! ```text
//! ★★★ THE LEAK. The pin's `OS_DESCRIPTOR` was never freed …
//!   left: 1
//!  right: 0
//! ```
//!
//! and leaves every other test in this workspace green — which is precisely the state
//! master was in. ⊘ **A gate nobody has seen fail is not a gate**; this one was severed and
//! restored.
//!
//! ## ⊘ What this file does NOT witness
//!
//! - **It is mock-driven and GPU-free.** It judges that the release is *reached and
//!   ordered*, and judges nothing about whether a real `nvidia.ko` unpins the pages on
//!   `NV01_FREE` of an `OS_DESCRIPTOR`. `only_live_boots_are_proof` still applies; the
//!   bench criteria are pre-registered in this rung's report.
//! - **It does not witness the whole leak class.** `FbJoinTable::remove`,
//!   `SparseFb::remove_join` and the `ChildExports`/`ExportRegistry` inverse are still
//!   NOT BUILT (w301 §3.2's other three rows) and are out of this rung's scope.

use std::sync::Arc;

use kayfabe_arch::Aperture;
use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::{Gpu, PinReleaseVerdict, classify_pin_release};
use kayfabe_core::rmgraph::RmEvent;
use kayfabe_isolate::{GuestRamGrant, HostHandle, IsolateId};
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

/// ⊘ The same addresses `guest_ram_pin.rs` uses, and for its stated reason: a test whose
/// addresses look nothing like the ones the boot uses cannot be read beside the boot.
const RING_VA: GpuVa = GpuVa(0x4_2006_4000);
const RING_GPA: u64 = 0x0768_a000;
const RING_FILE_OFFSET: u64 = 0x1_0000_0000 + 0x0768_a000;
const PIN_LEN: u64 = 4096;
const GUEST_RAM_BYTES: u64 = 0x2_0000_0000;

fn grant() -> GuestRamGrant {
    GuestRamGrant::originated_by_the_vmm(RING_FILE_OFFSET, PIN_LEN, Prot::ReadWrite)
}

/// One guest proc on GPU0 whose isolates can see guest memory — `guest_ram_pin.rs`'s
/// fixture, unchanged, because this file's subject is what happens *after* that one's.
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
            "guest_ram_pin_release::device",
            Arc::new(SharedDevice::new(gpu, LockMode::Sharded)),
            recorder.clone(),
        ),
        pid,
        recorder,
    )
}

/// Declare, in this proc's address table, that the guest's own page tables bind `RING_VA`.
/// The pin refuses an unbound VA (MISS = FAULT), so without this there is nothing to pin.
fn guest_binds(device: &SharedDevice, pid: kayfabe_core::ProcId) {
    device
        .with_proc_mut(pid, |p| {
            let vas = p.vases.get_mut(&(GPU, PDB)).expect("the compute VAS");
            vas.table
                .bind(
                    PDB,
                    RING_VA,
                    PIN_LEN,
                    Binding::declared_by_guest(RING_GPA, Aperture::SysmemCoherent)
                        .expect("the fixture declares a kind the guest can declare"),
                )
                .expect("the fixture's own bind is well-formed");
        })
        .expect("the proc is live");
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

/// `(host VAS, host GPU VA)` pairs the backend was asked to unmap.
fn gpu_va_unmaps(rec: &SharedRecorder) -> Vec<(HostHandle, u64)> {
    rec.lock()
        .expect("recorder")
        .log
        .iter()
        .filter_map(|(_, v)| match v {
            RmVerb::UnmapGpuVa { vas, va } => Some((*vas, *va)),
            _ => None,
        })
        .collect()
}

// =================================================================================
// ★★★★★ THE GATE
// =================================================================================

/// ★★★★★ **THE GATE.** A guest process's own teardown releases its guest-RAM pins —
/// descriptor, GPU mapping and isolate window — and this test issues no release itself.
///
/// Read the phases as one sentence: *the guest pins a page and the record proves it; the
/// guest frees its client root; one reap, and all three halves of the pin have been given
/// back.*
///
/// ★ **The non-vacuity is phase 1 and it is load-bearing.** If the pin had not landed,
/// phase 3's counts would be claims about an empty set — the shape this tree names
/// `a_census_zero_needs_a_known_positive`. Phase 1 asserts the **1** that phase 3 turns
/// into a release.
#[test]
fn a_dead_procs_guest_ram_pins_are_released_from_the_production_path() {
    let _wd = watchdog(
        "guest_ram_pin_release::the_gate",
        std::time::Duration::from_secs(60),
    );
    let (device, pid, rec) = device();
    guest_binds(&device, pid);

    // ---- phase 1: the guest pins one of its own pages, through the production verb.
    let pinned = device
        .pin_guest_ram(GPU, PDB, RING_VA, grant())
        .expect("the pin runs");
    let descriptor = pinned.memory;
    assert_eq!(
        device
            .with_proc_mut(pid, |p| p.vases[&(GPU, PDB)].guest_ram_pins.len())
            .expect("the proc is live"),
        1,
        "★ NON-VACUITY: the pin must actually be recorded, or every count below is a \
         statement about nothing"
    );
    assert_eq!(
        guest_ram_unmaps(&rec),
        0,
        "nothing has been released yet — the fixture is not the subject"
    );

    // ---- phase 2: the guest tears its own process down. Nothing here names a pin.
    device
        .apply(RmEvent::Free {
            client: CLIENT,
            handle: identical_handles(GR.0, CE.0).client_root,
        })
        .expect("the guest frees its own client root");
    assert_eq!(
        device.retired_len(),
        1,
        "the teardown really did retire the proc"
    );

    // ---- phase 3: ★★★ THE WITNESS. The reap, and nothing else.
    device.reap_retired();

    assert_eq!(
        freed(&rec).iter().filter(|h| **h == descriptor).count(),
        1,
        "★★★ THE LEAK. The pin's `OS_DESCRIPTOR` {descriptor:?} was freed {} times, not \
         exactly once. ZERO is w301 §3.2's state — `Vas::guest_ram_pins` has no removal, so \
         the handle is lost with the `Vas` and the host GPU keeps a live translation into \
         guest pages the guest has taken back. TWO is the double free w291's merge bound \
         itself to avoid: the same handle staged once by its pin and once by its \
         exact-extent row, which is what `PinReclaim::rows_deduped` exists to prevent.",
        freed(&rec).iter().filter(|h| **h == descriptor).count(),
    );
    assert_eq!(
        guest_ram_unmaps(&rec),
        1,
        "★★ THE ISOLATE'S OWN WINDOW. `GuestRamPin::mapped` had one write and zero reads \
         in the whole tree (w301 §3.2); if this is 0 it has none again, and every pin \
         leaves a never-`munmap`ed VMA in the isolate for the isolate's whole life — the \
         tightest exhaustible resource in the audit, against `vm.max_map_count` = 65530"
    );
    assert!(
        gpu_va_unmaps(&rec).iter().any(|&(_, va)| va == RING_VA.0),
        "the pin's GPU mapping at the guest's own VA must be unmapped: {:?}",
        gpu_va_unmaps(&rec)
    );

    // ---- and the tally says so, which is what a boot log can read.
    let t = device.pin_reclaim_gone();
    assert_eq!(
        (t.released, t.refused_no_host_vas),
        (1, 0),
        "the reclaim tally is what `kayfabe: PIN-RELEASE …` prints; a release that ran \
         with no number beside it is unreadable from a boot"
    );
}

/// ★★ **The double-free door, asserted as a number rather than as an absence.**
///
/// w291's merge writes the pin's own `memory` handle into an **exact-extent** address-table
/// row as well, and states the hazard verbatim: *"one handle written into N rows would be
/// freed N times — a DOUBLE FREE of a host object, strictly worse than the leak this
/// closes."* The pin above is exact-extent (`PIN_LEN` at `RING_VA`, matching the row
/// `guest_binds` made), so it is exactly the shape that would double-free, and the gate
/// above already asserts `== 1` rather than `>= 1` for that reason.
///
/// This asserts the mechanism directly: the row walk **skipped** a row because its pin had
/// already staged it. ⊘ `>= 1` here and not `== 1`: the count is over every dying `Vas` of
/// the proc, and pinning down how many rows a fixture happens to produce would be testing
/// the fixture.
#[test]
fn the_row_walk_skips_a_row_whose_object_its_pin_already_staged() {
    let _wd = watchdog(
        "guest_ram_pin_release::dedupe",
        std::time::Duration::from_secs(60),
    );
    let (device, pid, _rec) = device();
    guest_binds(&device, pid);
    let pinned = device
        .pin_guest_ram(GPU, PDB, RING_VA, grant())
        .expect("the pin runs");
    assert!(
        pinned.bound_into_table,
        "★ NON-VACUITY: this pin must be the EXACT-EXTENT shape, or the dedupe below has \
         nothing to dedupe and its number is vacuous"
    );

    device
        .apply(RmEvent::Free {
            client: CLIENT,
            handle: identical_handles(GR.0, CE.0).client_root,
        })
        .expect("the guest frees its own client root");
    device.reap_retired();

    let t = device.pin_reclaim_gone();
    assert!(
        t.rows_deduped >= 1,
        "the exact-extent row carrying the pin's own handle must be SKIPPED by the row \
         walk; {t:?} says it was not, which means that handle was staged twice"
    );
}

/// ★★★ **The refusal is a value, and it is exercised.**
///
/// `PinReleaseVerdict::RefusedVasLive` is the release this rung deliberately does **not**
/// build: reclaiming a pin whose `Vas` survives needs a GPU quiescence fence, and
/// `docs/audits/w301_cancellation_error_leaks.md` §3.3 established there is none anywhere
/// in this tree (`Isolate::is_quiesced` counts *our own* in-flight ioctls and says so in
/// its own doc title).
///
/// ⊘ **An unpin you cannot justify is worse than the leak**: the leak is silent about
/// memory the guest is done with; a premature unpin corrupts live work. So the absence is
/// asserted as a **named refusal** rather than left as prose nobody can run — the
/// difference between `refuse_by_name_means_the_name_is_true` and a paragraph.
#[test]
fn releasing_a_pin_whose_vas_is_still_live_is_refused_by_name() {
    let vas = HostHandle::new(IsolateId::new(7, GPU), 0x1234);
    assert_eq!(
        classify_pin_release(false, Some(vas)),
        PinReleaseVerdict::RefusedVasLive,
        "a live `Vas` must refuse, EVEN when the host VAS is perfectly nameable — the \
         refusal is about the missing fence, not about missing information"
    );
    assert_eq!(
        classify_pin_release(true, None),
        PinReleaseVerdict::RefusedNoHostVas,
        "a dying `Vas` with no host VAS cannot name the mapping it would be undoing"
    );
    assert_eq!(
        classify_pin_release(true, Some(vas)),
        PinReleaseVerdict::Release(vas),
        "★ THE POSITIVE. Without this arm the two refusals above would pass on a predicate \
         that refuses everything"
    );
}
