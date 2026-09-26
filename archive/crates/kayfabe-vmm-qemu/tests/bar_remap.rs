//! ★★★ The BAR-remap latch — **the C's latent bug, closed and watched firing**.
//!
//! `host_execution_plane.md` §1.5: `C: nvkvm_mmap_host.c:189-193` caches the window base
//! once and there is **no BAR-change hook anywhere** in the C's device. If the guest moves
//! the BAR after the memslot is installed, the hypervisor's own region follows and our
//! memslot does not — so the **old** guest-physical range keeps working, and becomes
//! reassignable to another BAR, while the **new** one reads zeros. Silently, in both
//! directions.
//!
//! It never fired in the C for three evidenced reasons, none of which is a mechanism. This
//! file is the mechanism, in three parts that close different halves:
//!
//! | part | what it is | catches |
//! |---|---|---|
//! | `bar_move_requested` | a **preventer**, for a configuration-space write override | the move before it happens |
//! | `note_bar_mapping` | a **tripwire**, for the mapping-update path | a move that happened anyway |
//! | `check_bars` on every resolve | an **assertion**, re-reading the live base | a move nothing told us about |
//!
//! ★★ **Every one of the three is watched failing in this file**, because a guard that has
//! never been seen to fire is not evidence (`suspect_the_instrument_first`). The negative
//! arms are the ones that establish the positive ones are not vacuous.

mod common;

use common::{BAR0_BASE, BAR1_BASE, MOCK_CEILING, config, machine, page, window_gpa, window_len};

use kayfabe_vmm::{BarId, Vmm, VmmError};
use kayfabe_vmm_qemu::host::QemuHost;
use kayfabe_vmm_qemu::mock_host::{MockPolicy, MockQemuHost, MockSlotPlane};
use kayfabe_vmm_qemu::{
    BAR_MOVED_UNDER_US, BAR_NOT_AT_ITS_DECLARED_BASE, BAR_NOT_PROGRAMMED, QemuMachine,
};
use std::sync::Arc;

/// ★★★ **The whole bug, in one test.** A window is installed, the guest moves the BAR, and
/// every access into that BAR refuses by name instead of silently reading the wrong memory.
///
/// The two halves of the C's failure are asserted **separately**, because they are
/// different bugs that happen to share a cause:
///
/// * the **new** base must not resolve — in the C it reads zeros from the reservation BAR's
///   stub ops, which is a wrong answer rather than an error;
/// * the **old** base must not keep working — in the C it does, and the guest-physical range
///   it is working at has just become available to another BAR.
#[test]
fn a_bar_that_moves_under_a_live_memslot_refuses_both_the_old_base_and_the_new_one() {
    let (m, host, _slots) = machine();
    let mut v = m.vmm();
    let p = page();

    // It works first, or nothing below means anything.
    v.gpa_write(window_gpa(), &[0xAB; 8])
        .expect("the window serves before the move");

    // The guest reprograms BAR1. The hypervisor's own region follows; our memslot does not.
    host.move_bar(BarId::Bar1, Some(BAR1_BASE + 0x1_0000_0000));

    assert_eq!(
        v.gpa_read(window_gpa() + 0x1_0000_0000, &mut [0u8; 8]),
        Err(VmmError::Unsupported(BAR_MOVED_UNDER_US)),
        "the NEW base must refuse; through the hypervisor's flat view it is a stub that \
         returns zeros, which is a wrong answer and not an error"
    );
    assert_eq!(
        v.gpa_read(window_gpa(), &mut [0u8; 8]),
        Err(VmmError::Unsupported(BAR_MOVED_UNDER_US)),
        "and the OLD base must ALSO refuse — this is the half the C gets wrong in the more \
         dangerous direction: the range still works and is now reassignable to another BAR"
    );
    assert_eq!(
        v.gpa_write(window_gpa() + p, &[0u8; 8]),
        Err(VmmError::Unsupported(BAR_MOVED_UNDER_US)),
        "writes too, not only reads"
    );

    let a = m.audit();
    assert_eq!(
        a.bar_moves_detected, 1,
        "the move must be detected ONCE, not once per access — the poison is a latch"
    );
    // ★ Exactly three, and the exact number is the point. The install latches (1), the
    // clean write re-reads (2), the first access after the move re-reads and detects (3) —
    // and the two accesses after that **short-circuit on the poison without a foreign
    // call**. That short circuit is deliberate: once the BAR is known bad there is nothing
    // to learn from asking again, and the hot path should not pay for a fact that cannot
    // change. Asserted by equality rather than by a floor so that losing the short circuit
    // is a red test rather than a silent extra call per access.
    assert_eq!(
        a.bar_base_checks, 3,
        "install + one clean access + the detecting access; the two refused afterwards must \
         cost NO base read"
    );
}

/// ★★ **The poison is sticky.** Moving the BAR back does not make the window work again.
///
/// It cannot: between the two moves the old guest-physical range was live in the kernel
/// with our memslot over it, and the hypervisor was free to hand that range to another
/// device. "It looks right again" is not the same as "it is right again".
#[test]
fn moving_the_bar_back_does_not_un_poison_it() {
    let (m, host, _slots) = machine();
    let mut v = m.vmm();

    host.move_bar(BarId::Bar1, Some(BAR1_BASE + 0x1_0000_0000));
    assert_eq!(
        v.gpa_read(window_gpa(), &mut [0u8; 8]),
        Err(VmmError::Unsupported(BAR_MOVED_UNDER_US))
    );

    host.move_bar(BarId::Bar1, Some(BAR1_BASE));
    assert_eq!(
        v.gpa_read(window_gpa(), &mut [0u8; 8]),
        Err(VmmError::Unsupported(BAR_MOVED_UNDER_US)),
        "a BAR that came back must STAY refused; while it was away the range was ours in \
         the kernel and the hypervisor's to give away"
    );
    // And a new install into the poisoned BAR is refused too, rather than latching a base
    // that disagrees with the memslots already out there.
    assert_eq!(
        m.install_ram_window(window_gpa() + window_len(), page())
            .map(|_| ()),
        Err(VmmError::Unsupported(BAR_MOVED_UNDER_US))
    );
}

/// ★★ **Unmapping the BAR entirely is a move too.** PCI BAR *sizing* passes through exactly
/// this state — the C guards that transient separately, with real code rather than a
/// comment (`C: virtio_nvgpu_pci.c:74-75`), and it is the reason its latent bug never fired.
///
/// Here it is not a special case: `None` is not the latched base, so it poisons like any
/// other disagreement. The conservative answer is the right one, because a device whose
/// BAR is unmapped has no guest-physical window at all.
#[test]
fn an_unmapped_bar_is_treated_as_a_move_and_not_as_a_transient_to_be_tolerated() {
    let (m, host, _slots) = machine();
    let mut v = m.vmm();
    host.move_bar(BarId::Bar1, None);
    assert_eq!(
        v.gpa_read(window_gpa(), &mut [0u8; 8]),
        Err(VmmError::Unsupported(BAR_MOVED_UNDER_US))
    );
    assert_eq!(m.audit().bar_moves_detected, 1);
}

/// ★★★ **The tripwire fires on its own** — without any access having to notice.
///
/// This is the `MemoryListener`-shaped half. `host_execution_plane.md` §1.5 establishes
/// that the accelerator's *slot* handler early-returns for our pure-MMIO reservation, so it
/// creates nothing — but the hypervisor's listener still fires for the region, which is what
/// makes a tripwire possible at all.
#[test]
fn the_mapping_tripwire_poisons_the_bar_before_any_access_looks_at_it() {
    let (m, host, _slots) = machine();
    // Deliberately do NOT move the mock host's own base: this asserts that the tripwire is
    // load-bearing by itself, rather than that `check_bars` happens to catch it as well.
    assert_eq!(m.audit().bar_moves_detected, 0);
    m.note_bar_mapping(BarId::Bar1, Some(BAR1_BASE + 0x2000_0000));
    assert_eq!(
        m.audit().bar_moves_detected,
        1,
        "the tripwire must poison on its own; if this is 0 the callback is decoration"
    );
    assert_eq!(
        m.vmm().gpa_read(window_gpa(), &mut [0u8; 8]),
        Err(VmmError::Unsupported(BAR_MOVED_UNDER_US))
    );
    // ...and the mock host still reports the ORIGINAL base, which is what proves the
    // tripwire — not the re-read — is what fired.
    assert_eq!(host.bar_base(BarId::Bar1), Some(BAR1_BASE));
}

/// ★ And the tripwire does **not** fire for a mapping report that agrees, or for a BAR we
/// never latched. A tripwire that poisons on every callback would break the device the
/// first time the hypervisor re-reported an unchanged topology.
#[test]
fn the_tripwire_is_silent_when_the_report_agrees_and_when_the_bar_is_not_ours() {
    let (m, _host, _slots) = machine();
    m.note_bar_mapping(BarId::Bar1, Some(BAR1_BASE));
    m.note_bar_mapping(BarId::Bar0, Some(0xDEAD_0000));
    m.note_bar_mapping(BarId::Bar2, None);
    assert_eq!(
        m.audit().bar_moves_detected,
        0,
        "an agreeing report, and a report about a BAR with no memslot of ours in it, must \
         both be silent"
    );
    m.vmm()
        .gpa_write(window_gpa(), &[1u8; 4])
        .expect("the window still serves");
}

/// ★★★ **The preventer refuses the move before it happens** — and only once we have
/// something to lose.
///
/// A configuration-space write override calls this before letting a base-address-register
/// write through. Before the first install there is nothing to protect and the answer must
/// be `Ok`, or firmware could never assign the BAR in the first place.
#[test]
fn a_bar_move_is_refused_once_a_memslot_is_in_it_and_permitted_before() {
    let host = common::host_with(MockPolicy::default());
    let slots = common::slot_plane();
    let cfg = kayfabe_vmm_qemu::MachineConfig {
        windows: Vec::new(),
        ..config()
    };
    let m = QemuMachine::realize(cfg, Arc::clone(&host) as Arc<_>, slots).expect("realizes");

    for bar in [BarId::Bar0, BarId::Bar1, BarId::Bar2] {
        assert_eq!(
            m.bar_move_requested(bar),
            Ok(()),
            "with no memslot installed anywhere, firmware must still be able to assign \
             {bar:?} — a preventer that refused from birth would stop enumeration"
        );
    }

    m.install_ram_window(window_gpa(), window_len())
        .expect("a window in BAR1");

    assert_eq!(
        m.bar_move_requested(BarId::Bar1),
        Err(VmmError::Unsupported(BAR_MOVED_UNDER_US)),
        "BAR1 now has a memslot of ours in it and may not move"
    );
    assert_eq!(
        m.bar_move_requested(BarId::Bar0),
        Ok(()),
        "BAR0 has nothing of ours in it and must still be free to move — a preventer that \
         locked every BAR would be a different bug wearing this one's clothes"
    );
}

/// ★★ A window in a BAR the guest has never programmed is refused **by its own name**.
///
/// The C's guard, ported: `C: nvkvm_mmap_host.c:194-207` returns early on a base of zero
/// rather than installing at a fallback, *"or we'd cache the wrong base"*. The near
/// neighbour is a BAR that is programmed **somewhere else**, and the two must not report as
/// each other: one means "wait", the other means "your configuration is stale".
#[test]
fn an_unprogrammed_bar_and_a_displaced_one_are_different_refusals() {
    let host = Arc::new(MockQemuHost::new());
    // Nothing programmed at all.
    let m = QemuMachine::realize(
        kayfabe_vmm_qemu::MachineConfig {
            windows: Vec::new(),
            ..config()
        },
        Arc::clone(&host) as Arc<_>,
        common::slot_plane(),
    )
    .expect("realizes with no windows");
    assert_eq!(
        m.install_ram_window(window_gpa(), window_len()).map(|_| ()),
        Err(VmmError::Unsupported(BAR_NOT_PROGRAMMED))
    );

    // Programmed, but not where this device was realized.
    host.place_bar(BarId::Bar1, BAR1_BASE + page());
    assert_eq!(
        m.install_ram_window(window_gpa(), window_len()).map(|_| ()),
        Err(VmmError::Unsupported(BAR_NOT_AT_ITS_DECLARED_BASE)),
        "a BAR that is programmed elsewhere is a STALE CONFIGURATION, not an unprogrammed \
         BAR; reporting either as the other sends an operator to the wrong place"
    );

    // And once it agrees, it installs — or the two refusals above are just a broken device.
    host.place_bar(BarId::Bar1, BAR1_BASE);
    m.install_ram_window(window_gpa(), window_len())
        .expect("a BAR at its declared base takes the window");
}

/// ★★ Realize itself carries the refusal, so a machine whose BARs are not ready never comes
/// up half-built — and the failed realize gives back the blocker and installs no slot.
#[test]
fn realize_refuses_when_the_reservation_bar_is_not_ready_and_leaves_nothing_behind() {
    let host = Arc::new(MockQemuHost::new());
    let slots = Arc::new(MockSlotPlane::new(MOCK_CEILING, page()));
    assert_eq!(
        QemuMachine::realize(
            config(),
            Arc::clone(&host) as Arc<_>,
            Arc::clone(&slots) as Arc<_>
        )
        .err(),
        Some(VmmError::Unsupported(BAR_NOT_PROGRAMMED))
    );
    assert!(
        host.blockers().is_empty(),
        "a failed realize must give back the migration blocker it took, or the machine is \
         unmigratable forever"
    );
    assert!(!host.discard_disabled(), "and re-enable discard");
    assert_eq!(slots.installs(), 0, "and install no memslot at all");
}

/// ★★★ **THE INSTRUMENT CHECK.** Everything above asserts that the guard fires. This asserts
/// that it is *possible* for it not to — i.e. that the tests are not passing because the
/// device refuses every access for some unrelated reason.
///
/// A guard that never lets anything through and a guard that works are indistinguishable
/// from the positive tests alone, and this project has found its own new gate unable to fail
/// three times in one day.
#[test]
fn without_a_move_nothing_is_poisoned_and_the_window_serves_indefinitely() {
    let (m, host, _slots) = machine();
    let mut v = m.vmm();
    let p = page();
    for i in 0..32u64 {
        let at = window_gpa() + (i % 8) * p;
        v.gpa_write(at, &[i as u8; 4])
            .unwrap_or_else(|e| panic!("write {i} at {at:#x}: {e:?}"));
        let mut back = [0u8; 4];
        v.gpa_read(at, &mut back)
            .unwrap_or_else(|e| panic!("read {i}: {e:?}"));
        assert_eq!(back, [i as u8; 4]);
    }
    let a = m.audit();
    assert_eq!(
        a.bar_moves_detected, 0,
        "nothing moved, so nothing may be detected"
    );
    assert!(
        a.bar_base_checks >= 64,
        "the check must have run on every one of the 64 accesses; {} means it is being \
         skipped and the whole file above is vacuous",
        a.bar_base_checks
    );
    assert_eq!(host.bar_base(BarId::Bar1), Some(BAR1_BASE));
    assert_eq!(BAR0_BASE, common::BAR0_BASE);
}
