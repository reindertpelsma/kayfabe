//! ★★★ Finding the hypervisor's own VM descriptor — all three arms, for real.
//!
//! `host_execution_plane.md` §1 has us installing memslots into a machine **we did not
//! create**, which means the very first thing the QEMU memory plane needs is a descriptor
//! for someone else's VM. `KvmVm::discover_in_this_process` is that door, ported from
//! `C: src/qemu/virtio_nvgpu.c:1114-1141`.
//!
//! ## ★★ Why this is its own test binary, and why every test takes one lock
//!
//! The discovery's answer is a property of **the whole process**: "how many KVM machines
//! does it hold?". `cargo test` runs a binary's tests on several threads at once, so any
//! sibling test holding a VM open changes this one's answer — the classic
//! `suspect_the_instrument_first` shape, where a green run and a red run differ by nothing
//! the test can see. Two mechanisms, not one:
//!
//! 1. **A separate binary**, so the only VMs in the process are the ones these tests make.
//!    The crate's unit tests create VMs freely and are in a different process.
//! 2. **One process-wide lock**, so within this binary the count is whatever the running
//!    test made it.
//!
//! The lock is `Mutex<()>` and every test takes it **first**, before any descriptor exists.
//! A test that forgets is not merely flaky; it makes a *sibling* flaky, so the discipline is
//! stated here rather than assumed.

use kayfabe_linux_raw::{HostPageSize, Kvm, KvmMemslot, KvmVm, RawError};
use std::sync::{Arc, Mutex, MutexGuard};

/// Serialises every test in this binary — see the module docs.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn serialized() -> MutexGuard<'static, ()> {
    ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn kvm() -> Kvm {
    Kvm::open().expect("/dev/kvm must be present to build the KVM-direct harness")
}

/// ★★★ **One machine, and the discovery finds THAT machine** — proved by the kernel, not
/// by a descriptor number.
///
/// A discovered handle that merely *is a VM* is not enough: the whole hazard the ambiguity
/// arm exists for is installing into the wrong one, which succeeds silently. So the
/// assertion is that the two handles share **one guest-physical address space**: a memslot
/// installed through the discovered handle makes the identical range `EEXIST` through the
/// handle we created. Nothing but the same VM produces that.
#[test]
fn the_discovered_machine_is_the_one_this_process_created_and_shares_its_address_space() {
    let _serial = serialized();
    kayfabe_linux_raw::require_kvm!(
        "the_discovered_machine_is_the_one_this_process_created_and_shares_its_address_space"
    );
    let p = HostPageSize::query();
    let created = Arc::new(kvm().create_vm().expect("KVM_CREATE_VM"));

    let discovered = Arc::new(
        KvmVm::discover_in_this_process()
            .expect("exactly one machine exists in this process, so discovery must succeed"),
    );

    let w = Arc::new(
        kayfabe_linux_raw::GuestWindow::create(p.bytes(), p).expect("a one-page reservation"),
    );
    let _held = KvmMemslot::install(
        Arc::clone(&discovered),
        0,
        0x4000_0000,
        Arc::clone(&w),
        0,
        p.bytes(),
        false,
    )
    .expect("installing through the discovered handle");

    let collision = KvmMemslot::install(
        Arc::clone(&created),
        1,
        0x4000_0000,
        Arc::clone(&w),
        0,
        p.bytes(),
        false,
    );
    assert_eq!(
        collision.unwrap_err(),
        RawError::Syscall {
            call: "KVM_SET_USER_MEMORY_REGION",
            errno: Some(libc::EEXIST),
        },
        "the discovered handle and the created one must be ONE address space; if discovery \
         returned a different machine this install would succeed and the guest would never \
         see the window"
    );
}

/// ★★ **Two machines are a refusal, not a first-match** — the arm the C does not have.
///
/// `C: virtio_nvgpu.c:1136-1141` takes the first `anon_inode:kvm-vm` it reads and stops.
/// That is correct exactly while one VM per process holds, and the day it stops holding the
/// symptom is a *successful* memslot install into a machine no guest is running in.
#[test]
fn two_machines_in_one_process_are_refused_rather_than_guessed_between() {
    let _serial = serialized();
    kayfabe_linux_raw::require_kvm!(
        "two_machines_in_one_process_are_refused_rather_than_guessed_between"
    );
    let k = kvm();
    let _first = k.create_vm().expect("the first machine");
    let _second = k.create_vm().expect("the second machine");
    assert_eq!(
        KvmVm::discover_in_this_process().unwrap_err(),
        RawError::Unsupported {
            what: "an unambiguous KVM VM descriptor",
            detail: "this process holds more than one KVM machine and they are \
                     indistinguishable from outside; installing memslots into the wrong \
                     one succeeds silently and the guest never sees them",
        },
        "with two machines there is no way to tell which one the device is in"
    );
}

/// ★ And with **no** machine the refusal is a *different* one — the near neighbour
/// (`testing_doctrine.md` §2 rule 3). "The hypervisor has no KVM machine" is a
/// configuration fact an operator fixes; "it has two" is a fact about the invocation, and
/// reporting either as the other sends them to the wrong place.
///
/// Needs no `/dev/kvm` at all, which is what makes it the arm a runner without the device
/// still holds: with no device no machine can exist, and the answer is the same one.
#[test]
fn a_process_with_no_machine_reports_that_rather_than_inventing_one() {
    let _serial = serialized();
    assert_eq!(
        KvmVm::discover_in_this_process().unwrap_err(),
        RawError::Unsupported {
            what: "this process's KVM VM descriptor",
            detail: "no descriptor in /proc/self/fd is a KVM machine; the hypervisor is not \
                     running on the KVM accelerator, or this library was loaded into a \
                     process that has no virtual machine",
        },
        "no machine and two machines must be DIFFERENT refusals; they send an operator to \
         different places"
    );
}

/// ★ `adopt` is the other door — the one a hypervisor that *can* hand us its descriptor
/// comes through — and it must reach the same address space that made the descriptor.
///
/// Asserted the same way, because "it did not error" is not evidence for a memslot plane.
#[test]
fn an_adopted_descriptor_reaches_the_address_space_it_was_duplicated_from() {
    let _serial = serialized();
    kayfabe_linux_raw::require_kvm!(
        "an_adopted_descriptor_reaches_the_address_space_it_was_duplicated_from"
    );
    let p = HostPageSize::query();
    let created = Arc::new(kvm().create_vm().expect("KVM_CREATE_VM"));
    let adopted = Arc::new(KvmVm::adopt(
        created
            .try_clone_descriptor()
            .expect("duplicating the VM descriptor"),
    ));

    let w = Arc::new(
        kayfabe_linux_raw::GuestWindow::create(p.bytes(), p).expect("a one-page reservation"),
    );
    let _held = KvmMemslot::install(
        Arc::clone(&adopted),
        0,
        0x5000_0000,
        Arc::clone(&w),
        0,
        p.bytes(),
        false,
    )
    .expect("installing through the adopted handle");
    assert_eq!(
        KvmMemslot::install(created, 1, 0x5000_0000, w, 0, p.bytes(), false).unwrap_err(),
        RawError::Syscall {
            call: "KVM_SET_USER_MEMORY_REGION",
            errno: Some(libc::EEXIST),
        },
        "an adopted descriptor must name the SAME machine, not merely a machine"
    );
}
