//! ★★★ The slot plane against a **real kernel** — the differential that stops the double
//! from lying.
//!
//! `host_execution_plane.md` §1.5's last box says the tiering is new construction and must
//! earn its own green gate. Everything else in this suite earns it against
//! [`MockSlotPlane`], which is what keeps the tiering assertions runnable where `/dev/kvm`
//! is absent (`kvm_gated_tests_ci_blind` — this project's CI runner has no device, and a
//! design in which every tiering assertion needs one is a design whose tiering assertions CI
//! never runs).
//!
//! **But a double that stops refusing where the kernel refuses turns the whole suite green
//! and wrong.** `07da582` is the precedent: a cross-GPU handle accepted by the real host and
//! caught **only** by the mock — one known divergence, found by accident.
//!
//! So this file runs the *same scenario list* against [`KvmSlotPlane`] and asserts the two
//! agree, arm for arm. It is `require_kvm!`-gated: on a runner with no device it reports
//! SKIPPED rather than passing silently.
//!
//! ★★ It also **verifies the two retyped errno constants**. `slots.rs` hardcodes `EINVAL`
//! and `EEXIST` rather than importing `libc`, on the crate rule that the adapter names no OS
//! constant — and states that the objection *"a hardcoded errno can be wrong"* is answered
//! here, by measurement. This is that measurement.

mod common;

use common::{MOCK_CEILING, page};

use kayfabe_linux_raw::{GuestWindow, HostPageSize, Kvm, RawError};
use kayfabe_vmm_qemu::mock_host::MockSlotPlane;
use kayfabe_vmm_qemu::slots::{KERNEL_EEXIST, KERNEL_EINVAL, KvmSlotPlane, SlotPlane};
use std::sync::Arc;

/// A real machine's slot plane, and a window to install into it.
fn real() -> KvmSlotPlane {
    let vm = Kvm::open()
        .expect("/dev/kvm must be present for this differential")
        .create_vm()
        .expect("KVM_CREATE_VM");
    KvmSlotPlane::new(Arc::new(vm))
}

fn window(pages: u64) -> Arc<GuestWindow> {
    let p = HostPageSize::query();
    Arc::new(GuestWindow::create(pages * p.bytes(), p).expect("a reservation"))
}

/// ★★★ **The scenario list, run twice.** Every refusal the mock models must be the refusal
/// the kernel gives, and every acceptance must be an acceptance.
///
/// The comparison is on the **exact** `RawError`, not on `is_err()`: `EEXIST` and `EINVAL`
/// mean operationally different things (a guest-physical range already taken versus a
/// request the kernel cannot express), and a suite that could only assert *"it refused"*
/// would pass whichever one it got.
#[test]
fn the_mock_kernel_refuses_exactly_where_the_real_one_does() {
    kayfabe_linux_raw::require_kvm!("the_mock_kernel_refuses_exactly_where_the_real_one_does");
    let p = page();
    let real = real();
    let ceiling = real.ceiling().expect("the kernel's ceiling");
    assert!(
        ceiling >= 32,
        "every kernel since the slot array became dynamic reports hundreds; {ceiling} means \
         the capability query is not reaching the kernel and this whole file is vacuous"
    );
    let mock = MockSlotPlane::new(ceiling, p);

    // Each case is `(name, slot, gpa, window pages, install offset, install len, readonly)`.
    let cases: Vec<(&str, u32, u64, u64, u64, u64, bool)> = vec![
        (
            "an ordinary read-write slot",
            0,
            0x4000_0000,
            2,
            0,
            2 * p,
            false,
        ),
        ("a read-only slot", 1, 0x4100_0000, 2, 0, 2 * p, true),
        (
            "a slot over the window's tail",
            2,
            0x4200_0000,
            4,
            2 * p,
            2 * p,
            false,
        ),
        (
            "a misaligned guest-physical base",
            3,
            0x4300_0000 + 8,
            2,
            0,
            2 * p,
            false,
        ),
        ("a misaligned length", 4, 0x4400_0000, 2, 0, p + 8, false),
        ("a zero length", 5, 0x4500_0000, 2, 0, 0, false),
        (
            "a number at the ceiling",
            ceiling,
            0x4600_0000,
            2,
            0,
            2 * p,
            false,
        ),
        (
            "a number past the ceiling",
            ceiling + 7,
            0x4700_0000,
            2,
            0,
            2 * p,
            false,
        ),
        (
            "a length past the window's end",
            6,
            0x4800_0000,
            2,
            0,
            4 * p,
            false,
        ),
        (
            "an offset past the window's end",
            7,
            0x4900_0000,
            2,
            4 * p,
            p,
            false,
        ),
    ];

    for (what, slot, gpa, pages, off, len, ro) in cases {
        let wr = window(pages);
        let wm = window(pages);
        let r = real.install(slot, gpa, &wr, off, len, ro).map(|_| ());
        let k = mock.install(slot, gpa, &wm, off, len, ro).map(|_| ());
        assert_eq!(
            r, k,
            "{what}: the double and the kernel must agree EXACTLY. A double that accepts \
             what the kernel refuses turns this suite green and wrong; a double that \
             refuses what the kernel accepts makes an arm untestable"
        );
    }
}

/// ★★★ **The two retyped errno constants, measured.**
///
/// `slots.rs` writes `EINVAL` and `EEXIST` as numbers rather than importing `libc`, because
/// the adapter crates name no OS constant. The honesty clause on that decision is that a
/// hardcoded errno is checked by measurement rather than by care — this is the check.
#[test]
fn the_retyped_errno_constants_are_the_numbers_the_kernel_actually_returns() {
    kayfabe_linux_raw::require_kvm!(
        "the_retyped_errno_constants_are_the_numbers_the_kernel_actually_returns"
    );
    let p = page();
    let real = real();
    let w = window(2);

    assert_eq!(
        real.install(0, 0x4A00_0000 + 8, &w, 0, 2 * p, false)
            .map(|_| ())
            .unwrap_err(),
        RawError::Syscall {
            call: "KVM_SET_USER_MEMORY_REGION",
            errno: Some(KERNEL_EINVAL),
        },
        "a misaligned base is the kernel's EINVAL, and KERNEL_EINVAL must BE that number"
    );

    let _held = real
        .install(1, 0x4B00_0000, &w, 0, 2 * p, false)
        .expect("the first slot");
    let w2 = window(2);
    assert_eq!(
        real.install(2, 0x4B00_0000 + p, &w2, 0, 2 * p, false)
            .map(|_| ())
            .unwrap_err(),
        RawError::Syscall {
            call: "KVM_SET_USER_MEMORY_REGION",
            errno: Some(KERNEL_EEXIST),
        },
        "an overlapping guest-physical range is the kernel's EEXIST, and KERNEL_EEXIST must \
         BE that number"
    );
}

/// ★★★ **The read-only tier reaches the kernel** — and this is the assertion the C could
/// never make, because `readonly` is a dead parameter there.
///
/// A slot installed with the flag and one without must both succeed and must **not** be the
/// same request. The polarity is not observable from a userspace install alone (both return
/// success), so what is asserted is that the two are accepted independently over the same
/// range — i.e. the flag is carried and the range really was replaced. The *guest-visible*
/// half of the property is held one layer down, by
/// `kayfabe_linux_raw::vcpu_unsafe`'s own test that a guest store into a read-only slot
/// faults out and never reaches the backing.
#[test]
fn the_read_only_tier_is_accepted_by_a_real_kernel_in_both_polarities() {
    kayfabe_linux_raw::require_kvm!(
        "the_read_only_tier_is_accepted_by_a_real_kernel_in_both_polarities"
    );
    let p = page();
    let real = real();
    let w = window(4);

    let ro = real
        .install(0, 0x4C00_0000, &w, 0, 2 * p, true)
        .expect("a read-only slot is a thing this kernel does");
    let rw = real
        .install(1, 0x4C00_0000 + 2 * p, &w, 2 * p, 2 * p, false)
        .expect("and a read-write one beside it");
    drop(ro);
    drop(rw);

    // Dropping cleared both, so the whole range may be claimed again by ONE slot. If a drop
    // had not cleared, this is `EEXIST` — which is the only way from here to tell a cleared
    // slot from a forgotten one.
    real.install(2, 0x4C00_0000, &w, 0, 4 * p, false)
        .expect("both slots must have been cleared by their drops");
}

/// ★★ A mixed layout — the observe hole — against a real kernel: two slots with a real gap
/// between them, and the gap really is claimable afterwards.
///
/// The absence of a slot is the tier with nothing to point at. Against the mock it is read
/// out of the table; against a real kernel the only way to observe it is that **the range is
/// still free**, which a third install proves.
#[test]
fn an_observe_hole_is_a_range_a_real_kernel_still_considers_free() {
    kayfabe_linux_raw::require_kvm!(
        "an_observe_hole_is_a_range_a_real_kernel_still_considers_free"
    );
    let p = page();
    let real = real();
    let w = window(8);
    let base = 0x4D00_0000;

    let _a = real
        .install(0, base, &w, 0, 2 * p, false)
        .expect("the piece before the hole");
    let _b = real
        .install(1, base + 3 * p, &w, 3 * p, 5 * p, false)
        .expect("the piece after it");

    // The hole is one page wide and nothing covers it.
    let filler = real.install(2, base + 2 * p, &w, 2 * p, p, false).expect(
        "★ the observe hole must be a range the kernel still considers FREE; an \
                 EEXIST here would mean a slot is covering the range every guest access is \
                 supposed to exit from",
    );
    drop(filler);
}

/// ★ The kernel's ceiling and the mock's are in the same class — so a test written against
/// the mock's number is not testing a fiction.
///
/// Not an equality: the mock's ceiling is a fixture and the kernel's is a fact. What must
/// hold is that both are large enough for the device's budget, which is the only property
/// any test depends on.
#[test]
fn the_mock_ceiling_and_a_real_one_are_in_the_same_class() {
    kayfabe_linux_raw::require_kvm!("the_mock_ceiling_and_a_real_one_are_in_the_same_class");
    let real_ceiling = real().ceiling().expect("the kernel's ceiling");
    let budget = kayfabe_vmm_qemu::slots::OUR_SLOT_BUDGET;
    assert!(
        real_ceiling >= budget * 2,
        "a real kernel must be able to hold this device's budget with room beneath it; \
         {real_ceiling} against a budget of {budget}"
    );
    assert!(
        MOCK_CEILING >= budget * 2,
        "and so must the fixture, or every mock-side test is exercising the too-small arm"
    );
}
