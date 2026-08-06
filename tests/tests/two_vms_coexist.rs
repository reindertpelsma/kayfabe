//! # ★★★ Two VMs in one process, and NOTHING is shared between them
//!
//! Owner ruling (2026-08-05): *"cross isolate isolation must be strong"*, and on how to get
//! multi-tenancy — *"on 'tenantID' just avoid globals, then you can just instantiate multiple
//! vms easily."*
//!
//! That is a design answer with a **testable** consequence, and until this file the
//! consequence was asserted by construction and never driven. A survey of every
//! interior-mutable `static` in library code (2026-08-06) found none holding per-VM state —
//! the statics are read-only tables (`GA106_BOOT_REGS`, `GA10X_PAGE_SIZES`), caches of *host*
//! facts (`kvm_gate`'s `AVAILABLE`, `page_size`'s `CACHED`, the isolate `IMAGE`), and
//! thread-local witnesses (`lockwitness`, `leafwitness`). ⊘ **But "I grepped and found none"
//! is an argument, not a guard.** The next `static` nobody notices is exactly the one that
//! breaks this, and it would break it *silently*: two VMs would simply start agreeing.
//!
//! ## Why identical identity is the whole test
//!
//! Every value a guest chooses is guest-local. Two VMs booting the same image will pick the
//! **same** `HClient`, the **same** `Pdb`, the same VAs, and touch the same GPGAs — not by
//! collusion, but because they are running the same code against the same emulated hardware.
//! So the fixture deliberately makes VM A and VM B **identical in every guest-visible value**,
//! including the GPA range. If anything at all is process-global, these two collide.
//!
//! ★ This is `#14`'s defect species (*"identical-VA collision"*) lifted one level: `#14` was
//! two processes inside one guest, this is two guests inside one host process. The remedy
//! there was a per-process isolate; the remedy here is that a VM **is** an object, and two
//! objects share nothing. A test is how that stays true.
//!
//! Invariant/contract tests (decision #15), mock-driven, **GPU-free**.

use kayfabe_arch::ids::{GpuId, HClient, Pdb};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_mmu::walker::PtPage;
use kayfabe_mocks::{MockArch, MockIsolateFactory};
use kayfabe_tests::{Scenario, identical_handles};

const GPU: GpuId = GpuId::ZERO;

// ★★★ IDENTICAL IN BOTH VMs — on purpose. Two guests running the same image choose the same
// numbers; a design that needs them to differ has not solved multi-tenancy, it has postponed
// it. ⊘ Do not "fix" a failure here by making these differ — that deletes the test.
const CLIENT: HClient = HClient(0xAA);
const PDB: Pdb = Pdb(0x1001_0000);
const PAGE: u64 = 0x1005_0000;
const OTHER_PAGE: u64 = 0x2005_0000;

fn pt_page(phys: u64) -> PtPage {
    PtPage {
        phys,
        aperture: kayfabe_arch::Aperture::Vidmem,
        level: 1,
        vabase: 0,
    }
}

/// One whole VM: its own arch, its own isolate factory, its own GPA space — and the GPA
/// range is the SAME in every VM, which is the realistic case and the hostile one.
///
/// `decoys` is how many short-lived processes this VM saw *before* the one under test.
///
/// ## ⚠ Why `decoys` exists — the first version of this file DID NOT BITE
///
/// `[measured]` 2026-08-06. The guard was checked by making the page-ownership index
/// process-global (the exact defect) and **only 1 of 4 tests went red**. The reason is worth
/// more than the fix: `ProcId` is a **per-VM counter**, so two freshly built VMs both mint
/// `ProcId(0)` for their first process. The assertion `pt_page_owner(PAGE) == Some((a, PDB))`
/// was therefore satisfied *by a shared index that happened to agree* — `a` and `b` were the
/// same value, so a global map holding one entry answered correctly for both VMs by
/// coincidence.
///
/// ⊘ An assertion that can be satisfied for the wrong reason is not a guard. Giving VM B a
/// prior process makes its `ProcId` genuinely differ, so the shared-index defect produces two
/// claimants and is forced to declare itself.
///
/// ★ Note what did NOT change: every **guest-chosen** value (`CLIENT`, `PDB`, `PAGE`, the GPA
/// range) is still identical across VMs. `ProcId` is ours, not the guest's, and two VMs with
/// different histories naturally differ in it. The hostile case is preserved; only the
/// coincidence is removed.
fn a_vm_after(decoys: u16) -> (Gpu, kayfabe_core::ProcId) {
    let (factory, _rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu =
        Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa).expect("the device realizes");
    let mut s = Scenario::new();
    for i in 0..decoys {
        s.compute_process(
            HClient(0xD0 + u32::from(i)),
            Pdb(0x9001_0000 + u64::from(i) * 0x1_0000),
            identical_handles(0x90 + i, 0x91 + i),
        );
    }
    s.compute_process(CLIENT, PDB, identical_handles(0x10, 0x11));
    for ev in s.events {
        gpu.apply(ev).expect("applies");
    }
    let pid = *gpu.spine.by_pdb.get(&(GPU, PDB)).expect("the VAS routed");
    (gpu, pid)
}

/// Two VMs whose processes carry **different** `ProcId`s, so an owner-identity assertion
/// cannot be satisfied by coincidence. See [`a_vm_after`].
fn two_vms() -> (Gpu, kayfabe_core::ProcId, Gpu, kayfabe_core::ProcId) {
    let (vm_a, a) = a_vm_after(0);
    let (vm_b, b) = a_vm_after(1);
    assert_ne!(
        a, b,
        "★ the fixture must give the two VMs distinct ProcIds, or every owner-identity \
         assertion below can be satisfied by a SHARED index that happens to agree — which is \
         exactly how the first version of this file passed under the defect it guards"
    );
    (vm_a, a, vm_b, b)
}

fn learn(gpu: &mut Gpu, pid: kayfabe_core::ProcId, pages: &[u64]) {
    let vas = gpu
        .procs
        .get_mut(&pid)
        .expect("live")
        .vases
        .get_mut(&(GPU, PDB))
        .expect("the vas");
    for &p in pages {
        vas.pt_meta.insert(p, pt_page(p));
    }
}

// =====================================================================================

/// ★★★ **THE PROPERTY.** Two VMs, byte-identical in every guest-chosen value, both claiming
/// the SAME page. Each must own it **in its own VM** — and the page must NOT become
/// contested, because contested means *two claimants in one address plane* and these are two
/// planes.
///
/// ⊘ This is the assertion a process-global would fail. A shared page-ownership index would
/// see two claimants for `PAGE` and decline it for both (`Spine::pt_contested`) — so the
/// symptom of the bug is not a crash or a leak, it is **both VMs quietly losing a mapping
/// they each legitimately own**. That is why it is worth a test rather than an argument.
#[test]
fn two_vms_claiming_the_same_page_each_own_it_and_neither_is_contested() {
    let (mut vm_a, a, mut vm_b, b) = two_vms();

    learn(&mut vm_a, a, &[PAGE]);
    learn(&mut vm_b, b, &[PAGE]);
    vm_a.spine.publish_pt_pages(a, GPU, PDB, vec![PAGE]);
    vm_b.spine.publish_pt_pages(b, GPU, PDB, vec![PAGE]);

    assert_eq!(
        vm_a.spine.pt_page_owner(GPU, PAGE),
        Some((a, PDB)),
        "VM A must own the page it claimed. If this is None, the ownership index is SHARED \
         between VMs and both claimants declined each other — the failure mode is a silently \
         lost mapping, not an error"
    );
    assert_eq!(
        vm_b.spine.pt_page_owner(GPU, PAGE),
        Some((b, PDB)),
        "VM B must own the SAME page independently — identical addresses in two guests are \
         normal, not a conflict"
    );
}

/// ★★ Work in one VM must be INVISIBLE in the other — including work that never touches a
/// shared key. Publishing a page only A knows about must not appear in B at all.
#[test]
fn a_page_published_in_one_vm_does_not_exist_in_the_other() {
    let (mut vm_a, a, vm_b, _b) = two_vms();

    learn(&mut vm_a, a, &[OTHER_PAGE]);
    vm_a.spine.publish_pt_pages(a, GPU, PDB, vec![OTHER_PAGE]);

    assert!(
        vm_a.spine.pt_page_owner(GPU, OTHER_PAGE).is_some(),
        "the fixture must actually publish something, or the assertion below is vacuous"
    );
    assert_eq!(
        vm_b.spine.pt_page_owner(GPU, OTHER_PAGE),
        None,
        "★ a page published in VM A leaked into VM B — the two share an index"
    );
}

/// ★★ And the identity plane too: the `ProcId` minted for VM B's process must not resolve to
/// anything in VM A, even though both used the same `HClient`/`Pdb`.
///
/// ⚠ ★ **This one is INSENSITIVE to the shared-page-index mutation, and that is correct.** In
/// the 2026-08-06 bite check it was the 1 of 4 that stayed green, because it asserts about the
/// **identity** plane (`procs`, `by_pdb`) and not about page ownership. Recording that here so
/// a later reader does not mistake a principled non-biter for a weak test — it would go red
/// under a *different* mutation: a shared `ProcId` allocator or a shared `by_pdb` map.
///
/// ⚠ The two `ProcId`s here DO differ, but only because [`two_vms`] gives VM B a prior
/// process. Equal ids across VMs would also be legal — they are per-VM counters, and a test
/// demanding they differ would be demanding a global namespace, which is the thing this file
/// exists to say we do not have. What must hold is that A's map answers only about A.
#[test]
fn identical_client_and_pdb_in_two_vms_are_two_different_processes() {
    let (vm_a, a, vm_b, b) = two_vms();

    assert!(
        vm_a.procs.contains_key(&a),
        "A's process lives in A's own table"
    );
    assert!(
        vm_b.procs.contains_key(&b),
        "B's process lives in B's own table"
    );

    // Each VM routes the shared (gpu, pdb) key to ITS OWN process, never the other's table.
    assert_eq!(vm_a.spine.by_pdb.get(&(GPU, PDB)), Some(&a));
    assert_eq!(vm_b.spine.by_pdb.get(&(GPU, PDB)), Some(&b));

    // The load-bearing half: A's VAS is reachable from A and describes A's address plane.
    // ⊘ NOT asserting `a != b` — see the doc comment; equal ids are correct here.
    assert!(
        vm_a.procs
            .get(&a)
            .expect("live")
            .vases
            .contains_key(&(GPU, PDB)),
        "VM A's process must own a VAS under the shared key, in VM A's own table"
    );
}

/// ★ Dropping a whole VM must not disturb the survivor — the shape a global would break at
/// teardown rather than at setup, which is the harder one to notice.
#[test]
fn dropping_one_vm_leaves_the_other_intact() {
    let (mut vm_a, a, mut vm_b, b) = two_vms();

    learn(&mut vm_a, a, &[PAGE]);
    learn(&mut vm_b, b, &[PAGE]);
    vm_a.spine.publish_pt_pages(a, GPU, PDB, vec![PAGE]);
    vm_b.spine.publish_pt_pages(b, GPU, PDB, vec![PAGE]);

    drop(vm_a);

    assert_eq!(
        vm_b.spine.pt_page_owner(GPU, PAGE),
        Some((b, PDB)),
        "★ tearing down one VM revoked the survivor's ownership — teardown reached shared \
         state. A VM's Drop must be a fact about that VM only"
    );
    assert!(
        vm_b.procs.contains_key(&b),
        "the survivor's process table must be untouched by a sibling's teardown"
    );
}
