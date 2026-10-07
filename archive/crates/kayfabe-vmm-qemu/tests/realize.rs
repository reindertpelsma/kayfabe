//! Realize and its refusals — every acceptance here is a **refusal**, and none of them
//! needs a guest.
//!
//! `l2_qemu_adapter.md` §10 stage Q2. Plus **task #97**, whose whole content is one call and
//! one arm, and whose *reason* changed completely under `host_execution_plane.md` §1 — see
//! [`a_discard_requirer_makes_realize_refuse_NAMING_THE_CONFLICT`].

mod common;

use common::{BAR0_BASE, config, machine, machine_with, page, window_gpa, window_len};

use kayfabe_vmm::{BarId, TrapMode, Vmm, VmmError};
use kayfabe_vmm_qemu::host::HostError;
use kayfabe_vmm_qemu::mock_host::{HostCall, MockPolicy};
use kayfabe_vmm_qemu::slots::KERNEL_EBUSY;
use kayfabe_vmm_qemu::{
    BELOW_FLOOR, MachineConfig, NOT_ACCELERATED, QemuMachine, TRAP_OUTSIDE_THE_REALIZED_TABLE,
    VERSION_FLOOR, WINDOW_IN_A_BACKED_BAR, WindowSpec,
};
use std::sync::Arc;

/// ★★ Realize's order is load-bearing, and an ordered log is the only thing that can see an
/// order at all: the blocker before anything is mapped, the discard refusal before the
/// reservation exists.
#[test]
fn realize_takes_the_blocker_and_refuses_discard_before_it_maps_anything() {
    let (m, host, slots) = machine();
    let log = host.log();
    let blocker = log
        .iter()
        .position(|c| matches!(c, HostCall::AddBlocker(_)))
        .expect("a blocker was taken");
    let discard = log
        .iter()
        .position(|c| *c == HostCall::DiscardDisable(true))
        .expect("discard was disabled");
    let first_bar_read = log
        .iter()
        .position(|c| matches!(c, HostCall::BarBase(_)))
        .expect("the reservation BAR was read");
    assert!(
        blocker < discard && discard < first_bar_read,
        "the blocker and the discard refusal must both precede the first thing that maps \
         memory. Log: {log:?}"
    );
    assert!(host.discard_disabled());
    assert_eq!(host.blockers().len(), 1);
    assert!(host.listening(), "and the listener is registered");
    assert_eq!(
        slots.installs(),
        1,
        "the realize-time reservation's memslot"
    );
    assert_eq!(
        m.audit().regions_published,
        0,
        "★★★ and ZERO regions were handed to the hypervisor to back — that number is \
         `host_execution_plane.md` §1 in one field, and it must never move"
    );
}

/// ★★ Every version below the floor is refused **by name**, and takes no blocker.
#[test]
fn every_version_below_the_floor_is_refused_by_name_and_takes_no_blocker() {
    let (maj, min) = VERSION_FLOOR;
    for v in [(0, 0), (9, 9), (maj - 1, 99), (maj, min - 1)] {
        let host = common::host_with(MockPolicy {
            version: v,
            ..MockPolicy::default()
        });
        let slots = common::slot_plane();
        assert_eq!(
            QemuMachine::realize(
                config(),
                Arc::clone(&host) as Arc<_>,
                Arc::clone(&slots) as Arc<_>
            )
            .err(),
            Some(VmmError::Unsupported(BELOW_FLOOR)),
            "version {v:?} is below the {VERSION_FLOOR:?} floor"
        );
        assert!(host.blockers().is_empty());
        assert_eq!(slots.installs(), 0);
    }
    // ...and the floor itself, plus above it, must realize — or the sweep refuses everything.
    for v in [(maj, min), (maj, min + 1), (maj + 1, 0)] {
        let (_m, host, _s) = machine_with(
            MockPolicy {
                version: v,
                ..MockPolicy::default()
            },
            config(),
        );
        assert_eq!(
            host.blockers().len(),
            1,
            "version {v:?} is at or above the floor"
        );
    }
}

/// ★★ A machine with no hardware accelerator is refused **before** the blocker is taken.
///
/// Under §1 this stopped being only a performance argument: the memory plane **is** the
/// accelerator's memslot table, so without it the device has no data path at all.
#[test]
fn a_machine_without_the_accelerator_is_refused_before_the_blocker_is_taken() {
    let host = common::host_with(MockPolicy {
        kvm_enabled: false,
        ..MockPolicy::default()
    });
    let slots = common::slot_plane();
    assert_eq!(
        QemuMachine::realize(
            config(),
            Arc::clone(&host) as Arc<_>,
            Arc::clone(&slots) as Arc<_>
        )
        .err(),
        Some(VmmError::Unsupported(NOT_ACCELERATED))
    );
    assert!(host.blockers().is_empty());
    assert!(
        !host
            .log()
            .iter()
            .any(|c| matches!(c, HostCall::AddBlocker(_))),
        "not even taken and given back — refused before"
    );
}

/// ★★★ **TASK #97.** A discard *requirer* makes realize refuse, **naming the conflict**.
///
/// ## §8.5's reasoning for this call is VOID; here is the one that replaced it
///
/// §8.5 justified `ram_block_discard_disable` by the balloon reaching *our reservation*.
/// Under `host_execution_plane.md` §1 the reservation is no longer a hypervisor RAM block at
/// all, so the balloon skips it **trivially** — its inflate path only walks the machine's own
/// RAM blocks and ours is not one. The old argument does not survive the decision.
///
/// The real hazard is **guest RAM exported to isolates**: that backing is a shared `memfd`,
/// and the discard helper punches the *file* with a hole-punching `fallocate`, which destroys
/// the file's backing pages and therefore reaches **every** mapping of it — the hypervisor's,
/// the accelerator's second-stage tables, and the isolate's own shared one. The hypervisor's
/// own comment warns the call works *"as long as nobody else uses that file"*; an isolate
/// holding exported guest RAM is exactly somebody else.
///
/// ## Why "naming the conflict" is the assertion and not a nicety
///
/// The two devices cannot coexist. An operator who is told only *"a discard requirer is
/// present"* has to bisect their own command line to find out which one to remove — so the
/// refusal carries the requirer's **name**, with the kernel's own `EBUSY` beside it. The
/// previous shape of this adapter flattened it to a class constant and threw the name away.
#[test]
#[allow(non_snake_case)]
fn a_discard_requirer_makes_realize_refuse_NAMING_THE_CONFLICT() {
    const CONFLICT: &str = "a memory device that requires guest-driven discard (virtio-mem)";
    let host = common::host_with(MockPolicy {
        discard_refuses: Some(HostError::Busy { what: CONFLICT }),
        ..MockPolicy::default()
    });
    let slots = common::slot_plane();
    assert_eq!(
        QemuMachine::realize(
            config(),
            Arc::clone(&host) as Arc<_>,
            Arc::clone(&slots) as Arc<_>
        )
        .err(),
        Some(VmmError::HostRefused {
            what: CONFLICT,
            errno: Some(KERNEL_EBUSY),
        }),
        "the refusal must carry the NAME of the conflicting device; a class constant tells \
         an operator that something is wrong and not what to remove"
    );
    assert!(
        host.blockers().is_empty(),
        "and it must give back the migration blocker it had already taken, or a device \
         that failed to realize leaves the machine unmigratable forever"
    );
    assert_eq!(
        slots.installs(),
        0,
        "and no memslot may be live in a machine with no device to tear it down"
    );
}

/// ★★ The cooperative arm, so the refusal above is not the only behaviour this call has:
/// discard really is disabled at realize and really is re-enabled at unrealize.
///
/// The pairing is what makes the device removable without permanently changing the
/// machine's policy — the same obligation the migration blocker has.
#[test]
fn discard_is_disabled_at_realize_and_re_enabled_at_unrealize() {
    let (m, host, _slots) = machine();
    assert!(host.discard_disabled());
    m.unrealize();
    assert!(
        !host.discard_disabled(),
        "a device that is gone must not still be suppressing the machine's discard policy"
    );
    assert_eq!(
        host.log()
            .iter()
            .filter(|c| matches!(c, HostCall::DiscardDisable(_)))
            .count(),
        2,
        "exactly one disable and one enable"
    );
}

/// ★★ A listener that cannot be registered unwinds the **whole** device: the blocker goes
/// back, discard is re-enabled, and every memslot the reservations installed is cleared.
#[test]
fn a_listener_that_cannot_be_registered_unwinds_the_whole_device() {
    let host = common::host_with(MockPolicy {
        listener_refuses: Some(HostError::Refused {
            what: "registering the topology listener",
            errno: Some(1),
        }),
        ..MockPolicy::default()
    });
    let slots = common::slot_plane();
    assert!(
        QemuMachine::realize(
            config(),
            Arc::clone(&host) as Arc<_>,
            Arc::clone(&slots) as Arc<_>
        )
        .is_err()
    );
    assert!(host.blockers().is_empty(), "the blocker went back");
    assert!(!host.discard_disabled(), "discard was re-enabled");
    assert!(
        slots.live().is_empty(),
        "★ and the kernel is holding NOTHING — the reservation's memslot was installed \
         before the listener failed, and a failed realize that leaves a live slot leaves it \
         over memory nobody will unmap. {:?}",
        slots.live()
    );
    assert_eq!(
        slots.clears(),
        slots.installs(),
        "every install matched by a clear"
    );
}

/// ★★★ A window in a BAR the hypervisor **backs** is refused, and that refusal is the whole
/// safety argument of `host_execution_plane.md` §1.5 turned into a check.
///
/// The reservation BAR is a pure-MMIO region: its constructor never sets the RAM flag, so
/// the accelerator's listener early-returns for it and creates **no slot of its own**, in
/// either direction, even across a BAR remap. That early return is what makes installing a
/// foreign memslot over the range safe. A BAR registered with a RAM-backed constructor
/// instead gets a hypervisor-managed slot over the same guest-physical range — and only one
/// of the two can win.
///
/// The C states the collision as *"proven by the earlier probe"*; the probe writeup says
/// only *"the likely cause is a memslot conflict"*, so the mechanism is a strong prior and
/// not a measurement. This device does not need to know which mechanism it is: it refuses
/// to be in that position at all.
#[test]
fn a_window_in_a_bar_the_hypervisor_backs_is_refused_by_name() {
    let host = common::host_with(MockPolicy {
        // BAR1 is NOT declared a reservation.
        reservation_bars: vec![BarId::Bar0],
        ..MockPolicy::default()
    });
    let slots = common::slot_plane();
    assert_eq!(
        QemuMachine::realize(
            config(),
            Arc::clone(&host) as Arc<_>,
            Arc::clone(&slots) as Arc<_>
        )
        .err(),
        Some(VmmError::Unsupported(WINDOW_IN_A_BACKED_BAR))
    );
    assert_eq!(slots.installs(), 0, "refused before anything was installed");
    assert!(host.blockers().is_empty());

    // ★ And the shape really was asked about — a check nobody performs is a check.
    assert!(
        host.log()
            .contains(&HostCall::ReservationShape(BarId::Bar1)),
        "the adapter must ASK whether the BAR is an unbacked reservation. Log: {:?}",
        host.log()
    );

    // The same device with BAR1 declared a reservation realizes — so the refusal is about
    // the declaration and not about the device.
    let (_m, _h, s) = machine_with(
        MockPolicy {
            reservation_bars: vec![BarId::Bar0, BarId::Bar1],
            ..MockPolicy::default()
        },
        config(),
    );
    assert_eq!(s.installs(), 1);
}

/// ★★ Unrealize gives back every reference, the blocker, and every memslot — the
/// conservation ledger, balanced.
#[test]
fn unrealize_gives_back_every_reference_and_the_blocker_and_every_memslot() {
    let (m, host, slots) = machine();
    let s = host.mint_foreign(common::FOREIGN_RAM, 4 * page(), common::ram_facts());
    m.region_add(s).expect("guest RAM");
    let mut v = m.vmm();
    v.map_read_native(
        common::overlay_gpa(),
        common::overlay_len(),
        kayfabe_vmm::HostRegion {
            id: u64::MAX,
            offset: 0,
        },
        Some(common::overlay_trap()),
    )
    .expect("a read-native window");

    m.unrealize();

    assert!(host.blockers().is_empty(), "the blocker");
    assert!(!host.discard_disabled(), "the discard policy");
    assert!(slots.live().is_empty(), "every memslot");
    assert_eq!(slots.clears(), slots.installs());
    let a = m.audit();
    assert_eq!(a.live_windows, 0);
    assert_eq!(a.live_memslots, 0);
    assert_eq!(a.window_bytes, 0);
    assert!(
        a.peak_memslots >= 3,
        "and the ledger was not vacuously empty"
    );
    assert_eq!(
        a.regions_published, 0,
        "★★★ still zero — the hypervisor never backed a byte of ours"
    );
}

/// ★★ A trap outside the realize-time table is refused, however it misses it.
#[test]
fn a_trap_outside_the_realize_time_table_is_refused_however_it_misses() {
    let p = page();
    let (m, _host, _slots) = machine_with(
        MockPolicy::default(),
        MachineConfig {
            windows: Vec::new(),
            ..config()
        },
    );
    let mut v = m.vmm();
    for (what, bar, range, mode) in [
        (
            "past the row's end",
            BarId::Bar0,
            8 * p..20 * p,
            TrapMode::ReadWrite,
        ),
        ("the wrong mode", BarId::Bar0, 0..2 * p, TrapMode::WriteOnly),
        ("a BAR with no row", BarId::Bar1, 0..p, TrapMode::ReadWrite),
    ] {
        assert_eq!(
            v.set_trap(bar, range.clone(), mode),
            Err(VmmError::Unsupported(TRAP_OUTSIDE_THE_REALIZED_TABLE)),
            "{what}"
        );
    }
    for (what, bar, range, mode) in [
        (
            "empty",
            BarId::Bar0,
            0..0,
            "an empty or inverted trap range",
        ),
        (
            "inverted",
            BarId::Bar0,
            2 * p..p,
            "an empty or inverted trap range",
        ),
        (
            "past the BAR",
            BarId::Bar0,
            0..4096 * p,
            "a trap range outside its BAR",
        ),
    ]
    .map(|(w, b, r, e)| (w, b, r, e))
    {
        assert_eq!(
            v.set_trap(bar, range, TrapMode::ReadWrite),
            Err(VmmError::Unsupported(mode)),
            "{what}: a geometry refusal is NOT the table refusal, and merging them would \
             hide a configuration mistake behind an arithmetic one"
        );
    }
    // ...and a row that IS in the table, over a range with no slot, is accepted.
    v.set_trap(BarId::Bar0, 0..2 * p, TrapMode::ReadWrite)
        .expect("a registration the table covers");
    assert_eq!(BAR0_BASE, common::BAR0_BASE);
}

/// ★ A device with no realized trap table accepts no trap registration at all.
#[test]
fn a_device_with_no_realized_traps_accepts_no_trap_registration_at_all() {
    let (m, _host, _slots) = machine_with(
        MockPolicy::default(),
        MachineConfig {
            traps: Vec::new(),
            windows: Vec::new(),
            ..config()
        },
    );
    let mut v = m.vmm();
    assert_eq!(
        v.set_trap(BarId::Bar0, 0..page(), TrapMode::ReadWrite),
        Err(VmmError::Unsupported(TRAP_OUTSIDE_THE_REALIZED_TABLE))
    );
}

/// ★ A reservation whose geometry the device cannot honour is refused at realize, by name,
/// and each way of being wrong is distinguishable.
#[test]
fn a_reservation_with_geometry_the_device_cannot_honour_is_refused_at_realize_by_name() {
    let p = page();
    for (what, spec, expected) in [
        (
            "misaligned base",
            WindowSpec::passthrough(window_gpa() + 8, p),
            "a reservation whose base or length is not a whole number of host pages",
        ),
        (
            "misaligned length",
            WindowSpec::passthrough(window_gpa(), p + 8),
            "a reservation whose base or length is not a whole number of host pages",
        ),
        (
            "zero length",
            WindowSpec::passthrough(window_gpa(), 0),
            "a reservation whose base or length is not a whole number of host pages",
        ),
        (
            "outside every BAR",
            WindowSpec::passthrough(0x5000_0000, p),
            "a reservation that is not inside any realized BAR",
        ),
        (
            "straddling the end of its BAR",
            WindowSpec::passthrough(window_gpa() + 1023 * p, 2 * p),
            "a reservation that is not inside any realized BAR",
        ),
    ] {
        let host = common::host_with(MockPolicy::default());
        let slots = common::slot_plane();
        assert_eq!(
            QemuMachine::realize(
                MachineConfig {
                    windows: vec![spec.clone()],
                    ..config()
                },
                Arc::clone(&host) as Arc<_>,
                Arc::clone(&slots) as Arc<_>
            )
            .err(),
            Some(VmmError::Unsupported(expected)),
            "{what}"
        );
        assert_eq!(slots.installs(), 0, "{what}: refused before any syscall");
        assert!(host.blockers().is_empty(), "{what}: and unwound");
    }
    assert_eq!(window_len(), 8 * p);
}
