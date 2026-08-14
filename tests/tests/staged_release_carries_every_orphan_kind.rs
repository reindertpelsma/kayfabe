//! ★★★★★ **w323 — THE KNOWN-POSITIVE FOR "A RELEASE PATH MUST NOT DROP A KIND".**
//!
//! `w317` found this defect while reading the disposal path and **deliberately did not fix
//! it** (`docs/design/budgeted_bql_disposal.md` §6): `Proc::stage_release` extended
//! `unmap` and `free` and **silently dropped `orphans.guest_ram`** — the isolate process's
//! own `mmap` windows onto guest RAM. Its stated reason was good and has expired: staging
//! them makes `munmap`s happen that did not happen before, on the live verb path, and an
//! unmeasured behaviour change inside a rung whose whole output was a **timing**
//! measurement would have made that measurement unattributable.
//!
//! §6 also specified the known-positive this rung owes it, in as many words: *"a
//! `VerbFailure` carrying `guest_ram` residue, watched surviving the round trip"*. This
//! file is that, plus the two coverage checks a single-case test could not make.
//!
//! # ⊘⊘ BE EXACT ABOUT WHAT LEAKED — the obvious reading is WORSE than the truth
//!
//! [`Orphans`] has **three** kinds and exactly one was dropped:
//!
//! | kind | what it names | was it staged before this rung? |
//! |---|---|---|
//! | `unmap` | the **host GPU's** translation `(host VAS, host GPU VA)` | ✔ yes |
//! | `free` | RM objects, incl. the `OS_DESCRIPTOR` that pins the guest pages | ✔ yes |
//! | `guest_ram` | the **isolate process's `mmap` window** onto guest RAM | ⊘ **DROPPED** |
//!
//! ⇒ ⊘ It is **not** true that this left a live host-GPU translation into freed guest
//! pages. That is `unmap`, and `unmap` was staged. What it left live is an **unprivileged
//! host process's CPU-visible mapping of guest RAM**, outliving the verb, the proc, and the
//! guest's own release of those pages. Same family, different aperture — and stating the
//! wrong one would put a future reader's attention on the wrong plane.
//!
//! # ★★ Why the sibling code was already right, and why that matters
//!
//! `Orphans::is_empty`, `Orphans::len`, `Orphans::release_plan` and `Orphans::split_off_budget`
//! all quantify over **all three** kinds; `split_off_budget` even preserves the
//! `unmap → free → guest_ram` order across batch boundaries. ⇒ the defect was **one
//! function**, not a design, and the type had already been shaped (w310, *"put the kind ON
//! the value"*) so that the omission was a two-line body disagreeing with its own argument.
//! That is why a test that only asserted `len() > 0` somewhere would have passed: the count
//! was right everywhere except at the one hop where the value was consumed.
//!
//! # ⚠ THIS IS A BEHAVIOUR CHANGE — grade it as one
//!
//! `munmap`s now happen on the live verb path that did not happen before. `w317`'s caution
//! is carried, not dissolved. The offline evidence is here; the boot evidence a follow-up
//! lane must collect is named in `docs/design/publication_off_the_bql.md` §8.

use kayfabe_arch::ids::{GpuId, HClient, HObject, Pdb, VChid};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::{Gpu, Proc};
use kayfabe_core::ProcId;
use kayfabe_isolate::{GuestRamMapped, HostHandle, IsolateId, Orphans};
use kayfabe_mocks::{MockArch, MockIsolateFactory};
use kayfabe_tests::{Scenario, identical_handles};

const GPU: GpuId = GpuId(0);
const CLIENT: HClient = HClient(0xc1e0_0001);
const PDB: Pdb = Pdb(0x1_0000);
const GR: VChid = VChid(8);
const CE: VChid = VChid(9);
const MEM: HObject = HObject(0x5c00_0002);
/// ⊘ A handle's isolate is part of its identity — `Worker::execute` refuses a foreign one
/// by name — so the fixture names one rather than inventing a bare integer.
const ISO: IsolateId = IsolateId::new(7, GPU);

/// One live `Proc` on `GPU`, built exactly as `teardown_reclaim.rs` builds one.
fn one_proc() -> (Gpu, ProcId) {
    let (factory, _rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x10_0000_0000..0x1000_0000_0000, 0x10_0000_0000);
    let mut gpu = Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa).expect("realizes");
    let mut s = Scenario::new();
    s.compute_process_on_gpu(CLIENT, PDB, identical_handles(GR.0, CE.0), None);
    s.memory(CLIENT, HObject(0x5c00_0001), MEM, 0x9_0000_0000);
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies");
    }
    let pid = gpu.spine.by_pdb[&(GPU, PDB)];
    (gpu, pid)
}

/// A residue naming **one of each kind** — the shape a partially-unwound `VerbFailure`
/// leaves behind.
fn one_of_each() -> Orphans {
    Orphans {
        unmap: vec![(HostHandle::new(ISO, 0xdead_0001), 0x7f00_0000)],
        free: vec![HostHandle::new(ISO, 0xdead_0002)],
        guest_ram: vec![GuestRamMapped {
            region: HostHandle::new(ISO, 0xdead_0003),
            len: 0x2000,
        }],
    }
}

/// Sum the three kinds as they actually sit on the queue, per kind rather than as a total:
/// a total can be right while a kind is missing, which is the exact substitution this file
/// exists to catch (*"a count cannot see a substitution"*).
fn staged_by_kind(proc: &Proc) -> (usize, usize, usize) {
    proc.staged_releases()
        .fold((0, 0, 0), |(u, f, g), (_gpu, o)| {
            (u + o.unmap.len(), f + o.free.len(), g + o.guest_ram.len())
        })
}

/// ★★★★★ **THE KNOWN-POSITIVE.** A residue carrying all three kinds must arrive on the
/// queue carrying all three kinds.
///
/// ⚠ Asserted **per kind**, never as `len() == 3`: the pre-fix body staged 2 of 3, and a
/// total-only assertion is exactly the instrument that reports a healthy number while a
/// kind is gone. Against the pre-fix body this is RED on the `guest_ram` clause and GREEN
/// on the other two — which is the discrimination, not decoration.
#[test]
fn a_staged_residue_carries_every_kind_the_failure_named() {
    let (mut gpu, pid) = one_proc();
    let proc = gpu.procs.get_mut(&pid).expect("proc");
    let residue = one_of_each();
    assert_eq!(residue.len(), 3, "the fixture must name one of each kind");

    proc.stage_release(GPU, residue);

    let (unmap, free, guest_ram) = staged_by_kind(proc);
    assert_eq!(unmap, 1, "the GPU translation must be staged");
    assert_eq!(free, 1, "the RM objects must be staged");
    assert_eq!(
        guest_ram, 1,
        "★ THE DEFECT: the isolate's own mmap window onto guest RAM was silently dropped \
         by `Proc::stage_release`, so a failed verb's guest-RAM residue outlived the verb, \
         the proc and the guest's release of those pages. `budgeted_bql_disposal.md` §6."
    );
    assert_eq!(
        proc.pending_release_len(),
        3,
        "and the queue's own count must agree with the per-kind sum"
    );
}

/// ★★★ **The case the early return made INVISIBLE, and it is the worst one.**
///
/// `Orphans::is_empty` quantifies over all three kinds, so a residue of **only** guest-RAM
/// windows passed `stage_release`'s early return and was then dropped by a body that named
/// two kinds. ⇒ the function returned having *accepted* a non-empty argument and staged
/// **nothing** — a silent total loss, not a partial one, and the polarity a
/// one-of-each test cannot reach.
#[test]
fn a_residue_of_only_guest_ram_windows_is_not_swallowed_whole() {
    let (mut gpu, pid) = one_proc();
    let proc = gpu.procs.get_mut(&pid).expect("proc");
    proc.stage_release(
        GPU,
        Orphans {
            guest_ram: vec![
                GuestRamMapped {
                    region: HostHandle::new(ISO, 0xbeef_0001),
                    len: 0x1000,
                },
                GuestRamMapped {
                    region: HostHandle::new(ISO, 0xbeef_0002),
                    len: 0x1000,
                },
            ],
            ..Orphans::default()
        },
    );
    assert_eq!(
        staged_by_kind(proc),
        (0, 0, 2),
        "a residue with no RM objects and no GPU mappings is still a residue; before w323 \
         it was accepted and discarded in the same call"
    );
}

/// ⊘ **The negative control the two above need.** An empty residue must stage nothing and
/// must not create a queue entry — otherwise `has_drainable_releases` would arm on a proc
/// with no work and the reap would defer it forever ("defers indefinitely is a leak with
/// extra steps").
#[test]
fn an_empty_residue_creates_no_queue_entry() {
    let (mut gpu, pid) = one_proc();
    let proc = gpu.procs.get_mut(&pid).expect("proc");
    proc.stage_release(GPU, Orphans::default());
    assert_eq!(proc.pending_release_len(), 0);
    assert_eq!(proc.staged_releases().count(), 0);
    assert!(!proc.has_drainable_releases());
}

/// ★★ **The ordering the fix must not break**, checked where it is observable: the batch
/// splitter fills `unmap` to exhaustion, then `free`, then `guest_ram`. Staging a third
/// kind must not let a `munmap` precede the `free` of the `OS_DESCRIPTOR` that pins the
/// same pages — [`Orphans::guest_ram`]'s own stated invariant.
///
/// ⊘ This is a property of `split_off_budget`, which was already correct; it is asserted
/// **here** because this rung is the first that can put a `guest_ram` entry on the queue at
/// all, so before w323 the property was untested by construction rather than by omission.
#[test]
fn a_budgeted_split_still_issues_every_unmap_before_any_munmap() {
    let mut q = Orphans {
        unmap: vec![(HostHandle::new(ISO, 1), 0x1000), (HostHandle::new(ISO, 1), 0x2000)],
        free: vec![HostHandle::new(ISO, 2)],
        guest_ram: vec![GuestRamMapped {
            region: HostHandle::new(ISO, 3),
            len: 0x1000,
        }],
    };
    // A budget that lands *inside* the queue: the first batch may not reach `guest_ram`.
    let first = q.split_off_budget(2);
    assert_eq!((first.unmap.len(), first.free.len(), first.guest_ram.len()), (2, 0, 0));
    let second = q.split_off_budget(2);
    assert_eq!((second.unmap.len(), second.free.len(), second.guest_ram.len()), (0, 1, 1));
    assert_eq!(q.len(), 0, "nothing may be discarded or duplicated by a split");
}
