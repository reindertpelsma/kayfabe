//! ★★★ Memslot numbers — **down from the kernel's ceiling, and back only after a clear**.
//!
//! `host_execution_plane.md` §1.5: the C allocates upward from a hardcoded base of 64
//! (`C: nvkvm_mmap_host.c:390-435`) on the stated convention that *"below the base is
//! reserved for QEMU's static regions"* — **a convention enforced by nothing**. The
//! hypervisor allocates densely from zero, starting at 16 slots and doubling, so
//! disjointness holds by arithmetic for one device set and breaks under memory hotplug, a
//! virtio memory device, or several pass-through devices with RAM BARs.
//!
//! ★★ **And the collision is not an error.** `KVM_SET_USER_MEMORY_REGION` on a number that
//! is already live is a *replace*: the hypervisor's mapping for that range goes away, ours
//! takes its place, the ioctl succeeds and nothing is reported. That silence is reproduced
//! by the mock kernel on purpose (`MockSlotPlane::replaces`) — a double that refused a
//! duplicate number would make this whole file untestable by making the failure impossible.
//!
//! This file asserts the numbers, the recycling and its ordering rule, and the exhaustion
//! refusal — the last one **watched firing**, because a budget that can never be reached is
//! a budget nobody has tested.

mod common;

use common::{MOCK_CEILING, config, machine, page, window_gpa, window_len};

use kayfabe_vmm::VmmError;
use kayfabe_vmm_qemu::mock_host::{MockPolicy, MockSlotPlane};
use kayfabe_vmm_qemu::slots::{CEILING_TOO_SMALL, OUR_SLOT_BUDGET, SLOT_BUDGET_EXHAUSTED};
use kayfabe_vmm_qemu::{MachineConfig, QemuMachine, WindowSpec};
use std::sync::Arc;

/// ★★★ **The numbers are the top of the kernel's range**, and the device says so.
#[test]
fn the_first_numbers_handed_out_are_the_highest_the_kernel_allows() {
    let (m, _host, slots) = machine();
    assert_eq!(
        m.slot_range(),
        (MOCK_CEILING - OUR_SLOT_BUDGET, MOCK_CEILING),
        "the device's numbers live in the top {OUR_SLOT_BUDGET} of the kernel's ceiling"
    );
    assert_eq!(
        slots.live().iter().map(|r| r.slot).collect::<Vec<_>>(),
        vec![MOCK_CEILING - 1],
        "the realize-time window must take the HIGHEST number, not the lowest — the C's \
         base-64-upward convention is exactly what this inverts"
    );

    let p = page();
    m.install_ram_window(window_gpa() + window_len(), 2 * p)
        .expect("a second window");
    let mut n: Vec<u32> = slots.live().iter().map(|r| r.slot).collect();
    n.sort_unstable();
    assert_eq!(n, vec![MOCK_CEILING - 2, MOCK_CEILING - 1], "descending");
    assert!(
        n.iter().all(|s| *s >= MOCK_CEILING - OUR_SLOT_BUDGET),
        "every number must stay above the floor; below it is where the hypervisor grows"
    );
}

/// ★★★ **Numbers come back — and only after the kernel has been told the slot is gone.**
///
/// The ordering is the whole safety property. A number returned before its clear turns the
/// next install from an ADD into a silent REPLACE, and `replaces()` is the only thing that
/// would ever see it. It is asserted at zero after a full cycle of teardown and reuse.
#[test]
fn a_number_returns_to_the_pool_only_after_its_slot_was_cleared() {
    let p = page();
    let (m, _host, slots) = machine();
    let mut taken = Vec::new();
    let mut regions = Vec::new();

    for i in 0..8u64 {
        let gpa = window_gpa() + window_len() + i * 2 * p;
        regions.push(m.install_ram_window(gpa, 2 * p).expect("a window"));
        taken.push(slots.live().iter().map(|r| r.slot).max().expect("a slot"));
    }
    assert_eq!(slots.installs(), 9, "the realize-time window plus eight");
    assert_eq!(slots.replaces(), 0);

    m.unrealize();
    assert!(slots.live().is_empty());
    assert_eq!(slots.clears(), 9, "every slot cleared");

    // A second machine over the same kernel: the numbers the first one used are free, and
    // re-using them must still be an ADD, because they were cleared first.
    let (m2, _h2, _s2) = machine();
    assert_eq!(
        slots.replaces(),
        0,
        "no install may EVER have replaced a live slot; the kernel would not have said so"
    );
    drop(m2);
}

/// ★★ Recycling is really recycling — the audit's own counter says so, and it is not
/// vacuous because it is zero first.
#[test]
fn released_numbers_are_reissued_rather_than_burning_the_budget() {
    let p = page();
    let (m, _host, _slots) = machine();
    assert_eq!(m.audit().slot_numbers_recycled, 0, "nothing recycled yet");

    // Install and tear down repeatedly, far more times than the budget. Without recycling
    // this exhausts — which is exactly what the C's first allocator did after "a few CUDA
    // processes" (`C: nvkvm_mmap_host.c:382-389`).
    for i in 0..(u64::from(OUR_SLOT_BUDGET) * 3) {
        let gpa = window_gpa() + window_len() + (i % 4) * 2 * p;
        let region = m
            .install_ram_window(gpa, 2 * p)
            .unwrap_or_else(|e| panic!("iteration {i}: {e:?}"));
        m.remove_window(region).expect("teardown");
    }
    assert!(
        m.audit().slot_numbers_recycled >= u64::from(OUR_SLOT_BUDGET),
        "after {n} install/teardown cycles the free list must have been used; {r} means \
         numbers are being burned",
        n = OUR_SLOT_BUDGET * 3,
        r = m.audit().slot_numbers_recycled
    );
}

/// ★★★ **The floor is a refusal, and here it is firing.**
///
/// A budget nothing can exhaust is a budget nobody has tested. This installs windows until
/// the descending allocator reaches its floor and asserts the exact refusal — and then
/// asserts that a teardown makes room again, so the refusal is a boundary and not a wall.
#[test]
fn exhausting_the_budget_is_a_named_refusal_and_a_teardown_makes_room_again() {
    let p = page();
    let (m, _host, slots) = machine();
    let mut ok = Vec::new();
    let mut refusal = None;
    for i in 0..u64::from(OUR_SLOT_BUDGET) + 4 {
        let gpa = window_gpa() + window_len() + i * 2 * p;
        match m.install_ram_window(gpa, 2 * p) {
            Ok(r) => ok.push((r, gpa)),
            Err(e) => {
                refusal = Some(e);
                break;
            }
        }
    }
    assert_eq!(
        refusal,
        Some(VmmError::Unsupported(SLOT_BUDGET_EXHAUSTED)),
        "the descending allocator must REFUSE at its floor rather than walk into the \
         hypervisor's numbers, where a collision is a silent replace"
    );
    assert_eq!(
        ok.len(),
        usize::try_from(OUR_SLOT_BUDGET).expect("budget") - 1,
        "the realize-time window took one of the {OUR_SLOT_BUDGET}"
    );
    assert_eq!(slots.replaces(), 0);

    // A teardown returns a number, and the very next install succeeds. It is deliberately
    // installed at **the same guest-physical range** the torn-down window had: that isolates
    // the slot-number question from the address-space question, because a different range
    // would also need the kernel not to report `EEXIST`.
    let (region, gpa) = *ok.last().expect("at least one");
    m.remove_window(region).expect("teardown");
    m.install_ram_window(gpa, 2 * p)
        .expect("the number that just came back must be usable");
}

/// ★★ A kernel whose ceiling cannot hold the budget is refused **at realize**, by name.
///
/// The near neighbour is [`SLOT_BUDGET_EXHAUSTED`], and they mean opposite things: one is
/// "this kernel is too small for this device", the other is "this device asked for too
/// much of a kernel that is big enough".
#[test]
fn a_kernel_too_small_to_carve_a_budget_from_is_refused_at_realize() {
    for ceiling in [0, 16, OUR_SLOT_BUDGET, OUR_SLOT_BUDGET * 2 - 1] {
        let host = common::host_with(MockPolicy::default());
        let slots = Arc::new(MockSlotPlane::new(ceiling, page()));
        assert_eq!(
            QemuMachine::realize(
                config(),
                Arc::clone(&host) as Arc<_>,
                Arc::clone(&slots) as Arc<_>
            )
            .err(),
            Some(VmmError::Unsupported(CEILING_TOO_SMALL)),
            "a ceiling of {ceiling} leaves no disjoint range"
        );
        assert!(
            host.blockers().is_empty(),
            "and the refusal must not leave a migration blocker behind"
        );
        assert_eq!(slots.installs(), 0);
    }
    // The first ceiling that DOES fit must realize, or the bound above refuses everything.
    let host = common::host_with(MockPolicy::default());
    let slots = Arc::new(MockSlotPlane::new(OUR_SLOT_BUDGET * 2, page()));
    QemuMachine::realize(config(), host as Arc<_>, slots as Arc<_>)
        .expect("the smallest ceiling that fits must be accepted");
}

/// ★★ A window whose spans cannot all be allocated takes **none** of them, and installs
/// nothing — the all-or-nothing rule, at the level the device sees it.
#[test]
fn a_window_that_cannot_get_all_its_numbers_installs_no_slot_at_all() {
    let p = page();
    // A ceiling that leaves exactly two numbers after realize's own window.
    let ceiling = OUR_SLOT_BUDGET * 2;
    let host = common::host_with(MockPolicy::default());
    let slots = Arc::new(MockSlotPlane::new(ceiling, p));
    let m = QemuMachine::realize(
        MachineConfig {
            windows: Vec::new(),
            ..config()
        },
        host as Arc<_>,
        Arc::clone(&slots) as Arc<_>,
    )
    .expect("realizes");

    // Fill the budget to within two numbers.
    for i in 0..u64::from(OUR_SLOT_BUDGET) - 2 {
        m.install_ram_window(window_gpa() + i * 2 * p, 2 * p)
            .unwrap_or_else(|e| panic!("filling: {e:?}"));
    }
    let before = slots.installs();
    // A window needing THREE spans (passthrough, observe hole, passthrough → 2 slots) plus
    // one more observe hole → 3 slots. Only two numbers are left.
    let spec = WindowSpec {
        gpa: window_gpa() + u64::from(OUR_SLOT_BUDGET) * 2 * p,
        len: 8 * p,
        observe: vec![
            window_gpa() + u64::from(OUR_SLOT_BUDGET) * 2 * p + 2 * p
                ..window_gpa() + u64::from(OUR_SLOT_BUDGET) * 2 * p + 3 * p,
            window_gpa() + u64::from(OUR_SLOT_BUDGET) * 2 * p + 5 * p
                ..window_gpa() + u64::from(OUR_SLOT_BUDGET) * 2 * p + 6 * p,
        ],
    };
    assert_eq!(
        m.install_tiered_window(&spec).err(),
        Some(VmmError::Unsupported(SLOT_BUDGET_EXHAUSTED))
    );
    assert_eq!(
        slots.installs(),
        before,
        "a window that could not get all its numbers must have installed NONE of its slots"
    );
    // ...and the two numbers that were available are still available.
    m.install_ram_window(window_gpa() + u64::from(OUR_SLOT_BUDGET) * 4 * p, 2 * p)
        .expect("a refusal must not have consumed the numbers it did not use");
}
