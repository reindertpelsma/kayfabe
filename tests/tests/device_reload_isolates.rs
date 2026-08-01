//! ★★★ **`#130`, the half the recovery test could not reach: DOES THE ISOLATE DIE WITH
//! THE DEVICE?**
//!
//! The owner's requirement, 2026-07-31: *"if the guest bricks kayfabe emulator state at any
//! point, it should be possible to unload and reload device to restore, restarting emulated
//! device, like similar for real gpu"*.
//!
//! `crates/kayfabe-qemu-raw/tests/device_recycle.rs` measured that cycle at the
//! **hypervisor** seam and said, in its own module docs, exactly what it could not say:
//!
//! > *"Isolate lifetime is not observable here, and it is not covered. At stage Q5 the
//! > device seam has no isolate in it … the question "does the isolate die with the
//! > device?" has no answer at this seam — not because it was checked and found fine, but
//! > because there is nothing yet to check."*
//!
//! That is still true of `kayfabe-qemu-raw`. It is **not** true of the core: an isolate is
//! owned by a [`Proc`], a `Proc` is owned by the [`Spine`], and the `Spine` is owned by the
//! device (`kayfabe_core::gpu::Gpu`). So the question has a seam after all, and this file
//! is it.
//!
//! # ★★★ Why this is the load-bearing half
//!
//! An isolate is a **sandboxed host process holding real host RM objects**. The C artifact
//! measured that RM's kernel clients hold refcounted references into *user* memory and that
//! RM serialises every ioctl-reachable path per client, waiting uninterruptibly — so an
//! isolate that outlives its device is not an untidy allocation. It is a live process whose
//! host handles a *reloaded* device could be handed, or whose wedged client blocks its
//! siblings in D state with nobody left who knows why. A reload that reattaches to one has
//! not recovered; it has aliased two device lifetimes.
//!
//! # ★★ The distinction this file exists to draw: RETIRE IS NOT DEATH
//!
//! [`kayfabe_isolate::Isolate::retire`] sets a `bool`. It refuses new checkouts and leaves
//! the sandbox running — correct for draining a process, and **not** what an unload needs.
//! The only thing that kills a real isolate is `Drop`: `HostIsolate::drop` clears the
//! worker sockets, then field-drop glue reaches `SandboxChild::drop`, which is
//! `kill(SIGKILL)` followed by a blocking `waitpid`
//! (`crates/kayfabe-isolate-host/src/isolate.rs:810`,
//! `crates/kayfabe-linux-raw/src/spawn_unsafe.rs:980`). A recovery path built on `retire`
//! would leave every sandbox of the bricked life running, and every existing test would
//! stay green — so [`the_reload_a_retire_would_give_you_is_not_a_reload`] plants that
//! mistake and watches it fail.
//!
//! # ★★★ MEAN: the state that matters is the one the ORDINARY path cannot clean up
//!
//! A device that only ever unloads cleanly does not need this requirement. `#130` exists
//! for the *bricked* device, so the fixture puts a proc into the state from which normal
//! reclamation **provably never escapes**: a checked-out worker means
//! [`kayfabe_core::gpu::Proc::is_quiesced`] is false, and `Spine::reap_retired` puts a
//! non-quiesced proc **back** on the retired list, every time, forever
//! (`crates/kayfabe-core/src/gpu.rs:2653-2657`). [`a_reap_can_never_free_a_wedged_proc`]
//! measures that this is a real trap and not a story, by reaping repeatedly and watching
//! nothing move. Unload is then the *only* exit — which is the whole argument for the
//! requirement.
//!
//! # What is NOT measured here — stated, not implied
//!
//! ⊘ **No hardware.** There is no GPU, no QEMU and no real sandbox in this file: the
//! isolates are [`MockIsolateFactory`]'s, and "the child was SIGKILLed" is modelled by
//! "the value was dropped". That is the right model — for the real type, *being dropped IS
//! the kill*, with no method a caller could forget — but it is a model. The guest-visible
//! cycle (`device_del`/`device_add`, or `rmmod nvidia; modprobe nvidia`) is unmeasured.

use std::collections::BTreeSet;

use kayfabe_arch::ids::{GpuId, HClient, HObject, Pdb};
use kayfabe_core::ProcId;
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::rmgraph::RmEvent;
use kayfabe_isolate::{IsolateId, Worker};
use kayfabe_mocks::{MockArch, MockIsolateFactory, SharedRecorder};
use kayfabe_tests::{Scenario, identical_handles};

use kayfabe_mocks::watchdog::watchdog;
use std::time::Duration;

const GPU: GpuId = GpuId::ZERO;
const MEM: HObject = HObject(0x6000_0000);

/// Three guest processes, so the three fates below are three *different* procs and a fix
/// that happens to work for one is not mistaken for the property.
const LIVE: HClient = HClient(0xA0);
const RETIRED: HClient = HClient(0xB0);
const WEDGED: HClient = HClient(0xC0);
const LIVE_PDB: Pdb = Pdb(0x3400_0000);
const RETIRED_PDB: Pdb = Pdb(0x3500_0000);
const WEDGED_PDB: Pdb = Pdb(0x3600_0000);

// =====================================================================================
// The instrument, and the two lists it keeps
// =====================================================================================

/// Every isolate the device ever **spawned**, from the recorder rather than the factory.
///
/// ★ The factory is moved into `Gpu::realize` and is unreachable afterwards, so
/// `MockIsolateFactory::spawned` cannot be read once a device exists. `spawn` files one
/// [`kayfabe_mocks::ClientLock`] per isolate on the recorder — one RM client per isolate is
/// the model's whole point — so the key set of that map **is** the birth register, and it
/// is reachable from the handle a test keeps.
fn born(rec: &SharedRecorder) -> BTreeSet<IsolateId> {
    rec.lock()
        .expect("recorder")
        .client_locks
        .keys()
        .copied()
        .collect()
}

/// Every isolate that has been **dropped** — i.e. every sandbox that was killed.
fn died(rec: &SharedRecorder) -> Vec<IsolateId> {
    rec.lock().expect("recorder").isolates_dropped.clone()
}

/// A device with the system proc plus three guest procs, none of them yet disturbed.
fn device() -> (Gpu, SharedRecorder, ProcId, ProcId, ProcId) {
    let (factory, recorder) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x10_0000_0000..0x1000_0000_0000, 0x10_0000_0000);
    let mut gpu = Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa).expect("realizes");

    let mut s = Scenario::new();
    for (i, (client, pdb)) in [
        (LIVE, LIVE_PDB),
        (RETIRED, RETIRED_PDB),
        (WEDGED, WEDGED_PDB),
    ]
    .into_iter()
    .enumerate()
    {
        let i = i as u16;
        // ★ Distinct channel ids per proc: `identical_handles` mints the same object
        // handles for each, which is correct — an RM handle namespace is per client — but
        // a vChid is device-wide and two procs may not share one.
        let h = identical_handles(0x100 + i * 0x10, 0x200 + i * 0x10);
        s.compute_process_on_gpu(client, pdb, h, None);
        s.memory(client, h.device, MEM, 0x9_0000_0000);
    }
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies");
    }
    let live = gpu.spine.by_pdb[&(GPU, LIVE_PDB)];
    let retired = gpu.spine.by_pdb[&(GPU, RETIRED_PDB)];
    let wedged = gpu.spine.by_pdb[&(GPU, WEDGED_PDB)];
    (gpu, recorder, live, retired, wedged)
}

/// ★★ **What a device life leaves in the core, as a comparable value — and this one is
/// ENUMERATED, which is a weaker guarantee than the register plane's and is said so
/// plainly.**
///
/// `kayfabe_core::gpu::Gpu` cannot derive equality: it owns `Box<dyn Arch>` and
/// `Box<dyn IsolateFactory>`, and neither has any. So unlike
/// [`kayfabe_device::RegPlane::residue`] — whose exhaustive destructuring makes the next
/// field a compile error — this is a hand-written list, and a routing map added to `Spine`
/// will not appear here until someone adds it.
///
/// ⊘ Recording that rather than implying otherwise is the point. It covers every **public**
/// map the spine keys guest traffic on, plus both proc collections and the condemned
/// routing; it does not cover the private ones (`pending_cancels`, `geom`, `next_proc`).
/// If a residual-state defect is ever found in one of those, this is the list that was
/// short.
fn core_state(g: &Gpu) -> (usize, usize, usize, usize, usize, usize, usize, usize) {
    (
        g.procs.len(),
        g.retired_len(),
        g.spine.condemned_len(),
        g.spine.by_pdb.len(),
        g.spine.by_vchid.len(),
        g.spine.pt_roots.len(),
        g.spine.ctx_vas.len(),
        g.spine.retired.len(),
    )
}

/// Put `pid` into the state ordinary reclamation can never leave: one of its workers is
/// checked out and never comes back, so the proc is permanently non-quiesced.
///
/// ★ Returned rather than dropped, because dropping the [`Worker`] here would quiesce
/// nothing (the pool slot stays `Busy`) but *would* make the fixture's intent unreadable.
/// The caller holds it for the life of the test, which is what "a verb is still in flight"
/// means.
#[must_use]
fn wedge(gpu: &mut Gpu, pid: ProcId) -> Worker {
    let w = gpu
        .procs
        .get_mut(&pid)
        .expect("proc")
        .isolate_mut(GPU)
        .expect("isolate")
        .checkout()
        .expect("a free worker");
    assert!(
        !gpu.procs[&pid].is_quiesced(),
        "precondition: a checked-out worker is what NOT QUIESCED means",
    );
    w
}

/// The guest tearing a process down, by freeing its client root.
fn guest_frees(gpu: &mut Gpu, client: HClient) {
    gpu.apply(RmEvent::Free {
        client,
        handle: identical_handles(0, 0).client_root,
    })
    .expect("teardown applies");
}

// =====================================================================================
// ★★★ THE NON-VACUITY GATES — the instrument before the property
// =====================================================================================

/// ★★★ **Does the death witness exist, and does it discriminate?**
///
/// Before `#130` the harness recorded births and nothing else, so *"the isolate dies with
/// the device"* was unfalsifiable — and an unfalsifiable property reads exactly like one
/// that holds. This asserts the witness is silent while the device is alive and speaks when
/// it is not, which is the minimum for anything below to mean something.
#[test]
fn the_death_witness_is_silent_until_something_actually_dies() {
    let _w = watchdog("device_reload_isolates::witness", Duration::from_secs(60));
    let (gpu, rec, _l, _r, _wd) = device();

    let alive = born(&rec);
    assert!(
        alive.len() >= 4,
        "precondition: the system proc's isolate plus one per guest proc — got {alive:?}",
    );
    assert_eq!(
        died(&rec),
        Vec::new(),
        "★ nothing has died: a witness that already fired cannot prove a later death",
    );

    drop(gpu);

    let dead: BTreeSet<IsolateId> = died(&rec).into_iter().collect();
    assert_eq!(dead, alive, "and dropping the device is what makes it fire");
}

/// ★★★ **The trap the requirement exists for: a reap can never free a wedged proc — and
/// this test is the measurement of it, at every revision it runs
/// (`tests/tests/device_reload_isolates.rs`).**
///
/// If ordinary reclamation could get out of this state, `#130` would be a convenience. It
/// cannot: `Spine::reap_retired` puts a non-quiesced proc straight back, so the loop below
/// can run any number of times and nothing moves. Unload is the only exit, and that is the
/// whole argument.
#[test]
fn a_reap_can_never_free_a_wedged_proc() {
    let _w = watchdog("device_reload_isolates::wedged", Duration::from_secs(60));
    let (mut gpu, rec, _live, _retired, wedged) = device();
    let held = wedge(&mut gpu, wedged);
    guest_frees(&mut gpu, WEDGED);
    assert_eq!(gpu.retired_len(), 1, "the guest's free retired it");

    for round in 0..8 {
        let r = gpu.reap_retired();
        assert_eq!(
            (r.len(), r.deferred()),
            (0, 1),
            "★ reap {round} took NOTHING and deferred the wedged proc — again",
        );
        drop(r);
        assert_eq!(gpu.retired_len(), 1);
    }
    assert_eq!(
        died(&rec),
        Vec::new(),
        "★★ and after eight reaps not one sandbox has been killed: this proc's isolate is \
         unreachable by the ordinary path, which is exactly why unload must reach it",
    );

    drop(held);
    drop(gpu);
}

// =====================================================================================
// ★★★ THE PROPERTY
// =====================================================================================

/// ★★★ **Unloading the device kills EVERY isolate it ever spawned — including the ones
/// reclamation cannot reach.**
///
/// The three guest procs are in three different fates on purpose, because a teardown that
/// walks one collection frees one of them:
///
/// 1. **live** — still in `Gpu::procs`, never retired.
/// 2. **retired, unreaped** — off the live set, sitting on `Spine::retired`.
/// 3. **wedged** — retired, and every reap defers it forever (see
///    [`a_reap_can_never_free_a_wedged_proc`]).
///
/// …plus the **system proc**, which is neither a guest process nor in `Gpu::procs` at all
/// (`Gpu { spine, system, procs }`) and would be missed by any walk of the guest set.
///
/// ★ The assertion compares the death list against the **birth register**, not against a
/// number someone wrote down. A fourth fate added to the core is covered here on the day it
/// is added: it spawns an isolate, the isolate is in `born`, and it must be in `died`.
#[test]
fn unloading_the_device_kills_every_isolate_including_the_unreachable_ones() {
    let _w = watchdog("device_reload_isolates::unload", Duration::from_secs(60));
    let (mut gpu, rec, _live, retired, wedged) = device();

    // (3) the wedged one, first: its worker must stay out across everything below.
    let held = wedge(&mut gpu, wedged);
    guest_frees(&mut gpu, WEDGED);
    // (2) the retired-but-reapable one — retired out of band, and NOT reaped.
    assert!(gpu.retire_proc(retired), "the proc retires");
    // (1) `live` is left alone.

    assert_eq!(gpu.retired_len(), 2, "two retired, neither reaped");
    assert_eq!(gpu.procs.len(), 1, "one guest proc still live");

    let alive = born(&rec);
    assert!(
        alive.len() >= 4,
        "precondition: four isolates were spawned — {alive:?}",
    );
    assert_eq!(died(&rec), Vec::new(), "precondition: none has died yet");

    // ★★★ THE UNLOAD.
    drop(gpu);

    let dead = died(&rec);
    let unique: BTreeSet<IsolateId> = dead.iter().copied().collect();
    assert_eq!(
        unique.len(),
        dead.len(),
        "★ no isolate was killed twice — a double drop of a real sandbox is a `waitpid` on \
         a reaped pid, i.e. a reaped-child race, not a harmless repeat: {dead:?}",
    );
    assert_eq!(
        unique, alive,
        "★★★ EVERY isolate the device spawned died with it. A survivor here is a live \
         sandboxed process holding host RM objects that a reloaded device could be handed \
         — the cross-lifetime hazard, not an untidy allocation",
    );

    drop(held);
}

/// ★★★ **A "reload" built on `retire` is not a reload.**
///
/// This plants the mistake the vocabulary invites — `retire()` is the method that *sounds*
/// like teardown — and measures what it actually leaves: every sandbox still running.
/// Without this the difference is a doc comment, and a doc comment does not go red.
#[test]
fn the_reload_a_retire_would_give_you_is_not_a_reload() {
    let _w = watchdog("device_reload_isolates::retire", Duration::from_secs(60));
    let (mut gpu, rec, live, retired, wedged) = device();

    for pid in [live, retired, wedged] {
        let cancels = gpu.procs.get_mut(&pid).expect("proc").retire();
        drop(cancels);
        assert!(
            gpu.procs[&pid].is_retired(),
            "the proc reports itself retired"
        );
    }

    assert_eq!(
        died(&rec),
        Vec::new(),
        "★★★ THE POINT: three procs retired, every isolate still ALIVE. `retire` refuses \
         new checkouts and returns the outstanding cancels; it does not close a socket, \
         signal a child or reap one. A recovery path that stopped here would leave the \
         bricked life's sandboxes running and every other test in this workspace green",
    );

    drop(gpu);
    assert_eq!(
        died(&rec).into_iter().collect::<BTreeSet<_>>(),
        born(&rec),
        "and the drop that follows is what actually kills them",
    );
}

// =====================================================================================
// ★★★ THE CYCLE: unload -> reload
// =====================================================================================

/// ★★★ **After a bricked life is unloaded, the reloaded device is a first boot — and the
/// first life is GONE, not draining.**
///
/// Two halves, because they fail differently. A device that reloads into a clean state but
/// leaves the old sandboxes up has aliased two lifetimes; a device that kills everything
/// but reloads carrying the first life's routing tables has not restarted. Both are
/// asserted, and the second is asserted against a **control device that never had a first
/// life** rather than against a list of fields someone chose.
#[test]
fn a_reloaded_device_is_a_first_boot_and_the_bricked_life_is_gone() {
    let _w = watchdog("device_reload_isolates::cycle", Duration::from_secs(60));

    // ---- Life one: brick it. -----------------------------------------------------
    let (mut first, rec1, _live, retired, wedged) = device();
    let held = wedge(&mut first, wedged);
    guest_frees(&mut first, WEDGED);
    assert!(first.retire_proc(retired));
    assert_eq!(first.reap_retired().deferred(), 1, "wedged, and stuck");
    let life_one = born(&rec1);

    // ---- The unload. -------------------------------------------------------------
    drop(first);
    drop(held);
    assert_eq!(
        died(&rec1).into_iter().collect::<BTreeSet<_>>(),
        life_one,
        "★ the bricked life left nothing running",
    );

    // ---- The reload, and a control that never had a first life. -------------------
    let (reloaded, rec2, _l2, _r2, _w2) = device();
    let (control, rec3, _l3, _r3, _w3) = device();

    assert_eq!(
        core_state(&reloaded),
        core_state(&control),
        "★★★ the reloaded device's live set, retired list, condemned routing and every \
         routing table are the control's — no cursor, latch or key survived the cycle",
    );
    assert_eq!(
        (reloaded.spine.condemned_len(), reloaded.retired_len()),
        (0, 0),
        "and the control is not itself dirty — an equality against a dirty control is \
         vacuous",
    );

    // ★★ The second life's isolates are NEW OBJECTS, not the first life's re-adopted:
    // a fresh recorder saw a fresh set of births and no deaths at all.
    assert_eq!(
        died(&rec2),
        Vec::new(),
        "the reloaded life has killed nothing"
    );
    assert_eq!(
        born(&rec2),
        born(&rec3),
        "and it spawned exactly what a first boot spawns",
    );

    drop(reloaded);
    drop(control);
}
