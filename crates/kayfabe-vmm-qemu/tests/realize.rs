//! ★★ Realize, its refusals, its unwinding, and §4.3's latch.
//!
//! `l2_qemu_adapter.md` §8.1 lists ten ordered steps and §10 assigns their *runtime*
//! acceptance to stage Q2 — a booted hypervisor refusing at the monitor. Everything below
//! is the half that needs no hypervisor at all: the **order**, the **unwinding**, and the
//! **latch**, all of which are pure logic and all of which Q2 would otherwise discover
//! against a machine where a failure has two candidate causes.
//!
//! Every assertion here is by exact content. The refusal strings are public constants
//! precisely so a test can name the one it means instead of matching `is_err()`.

mod common;

use std::sync::Arc;

use common::{config, machine, machine_with, page};
use kayfabe_vmm::{BarId, TrapMode, Vmm, VmmError};
use kayfabe_vmm_qemu::host::HostError;
use kayfabe_vmm_qemu::mock_host::{HostCall, MockPolicy, MockQemuHost};
use kayfabe_vmm_qemu::{
    BELOW_FLOOR, DISCARD_REQUIRER_PRESENT, MachineConfig, NOT_ACCELERATED, QemuMachine,
    TOPOLOGY_AFTER_REALIZE, TRAP_OUTSIDE_THE_REALIZED_TABLE, TrapSpec,
};

/// ★★ §8.1's order is the assertion, and a set of counters could not make it.
///
/// The two orderings that are load-bearing rather than tidy: the migration blocker is
/// taken **before anything is mapped** (§8.1 step 3 — a partially realized device that
/// somebody migrates is the silent failure §8.4 is about), and guest-driven discard is
/// refused **before the reservation exists** (§8.1 step 4 — the window it protects is
/// created in step 5).
#[test]
fn realize_takes_the_blocker_and_refuses_discard_before_it_maps_anything() {
    let (_m, host) = machine();
    let log = host.log();

    // The prefix, by exact content. Not `contains`: a step in the wrong place is exactly
    // the defect, and `contains` cannot see an order.
    assert_eq!(
        log[0],
        HostCall::Version,
        "the runtime floor is checked first — everything after it is written against a \
         version whose facilities we have not yet confirmed ({log:?})"
    );
    assert_eq!(
        log[1],
        HostCall::KvmEnabled,
        "and the accelerator second, because without it the whole threading design of \
         section 4 is false and nothing else is worth doing ({log:?})"
    );
    assert!(
        matches!(log[2], HostCall::AddBlocker(_)),
        "the migration blocker comes BEFORE anything is mapped ({log:?})"
    );
    assert_eq!(
        log[3],
        HostCall::DiscardDisable(true),
        "and guest-driven discard is refused before the reservation it protects exists \
         ({log:?})"
    );
    assert!(
        matches!(log[4], HostCall::PublishWindow(_)),
        "only then is the reservation published ({log:?})"
    );
    assert!(
        matches!(log[5], HostCall::PublishRomOverlay(_)),
        "then the read-native overlays ({log:?})"
    );
    assert_eq!(
        log[6],
        HostCall::RegisterListener,
        "and the topology listener LAST, so no callback can arrive naming a range we have \
         not finished declaring ({log:?})"
    );
    assert_eq!(log.len(), 7, "and nothing else at all ({log:?})");
    assert!(host.discard_disabled(), "the refusal is still in force");
    assert!(host.listening(), "and the listener is registered");
}

/// ★★ The runtime floor, swept rather than witnessed — and the refusal leaves the machine
/// **untouched**, which is the half a single `assert!(is_err())` would miss.
///
/// §3.5: the compile-time check is a claim about the headers and this is a claim about
/// the binary; a header-only mismatch or a compatible relink separates them, so neither
/// substitutes for the other.
#[test]
fn every_version_below_the_floor_is_refused_by_name_and_takes_no_blocker() {
    for (version, expected) in [
        ((9, 0), Some(BELOW_FLOOR)),
        ((9, 2), Some(BELOW_FLOOR)),
        ((10, 0), Some(BELOW_FLOOR)),
        ((10, 1), Some(BELOW_FLOOR)),
        ((10, 2), None),
        ((10, 3), None),
        ((11, 0), None),
    ] {
        let host = Arc::new(MockQemuHost::with_policy(MockPolicy {
            version,
            ..MockPolicy::default()
        }));
        let got = QemuMachine::realize(config(), Arc::clone(&host) as Arc<_>);
        match expected {
            Some(reason) => {
                assert_eq!(
                    got.err(),
                    Some(VmmError::Unsupported(reason)),
                    "{version:?} is below the 10.2 floor and must be refused by that name"
                );
                assert_eq!(
                    host.log(),
                    vec![HostCall::Version],
                    "{version:?}: a refused realize asks the host NOTHING else — it must \
                     not take a blocker, disable discard or publish a region it will then \
                     have to unwind"
                );
                assert!(
                    host.blockers().is_empty() && !host.discard_disabled(),
                    "{version:?}: and it leaves the machine exactly as it found it"
                );
            }
            None => {
                assert!(
                    got.is_ok(),
                    "{version:?} is at or above the floor and must realize"
                );
                assert!(
                    host.blockers().len() == 1,
                    "{version:?}: a realized device holds exactly one migration blocker"
                );
            }
        }
    }
}

/// ★ No accelerator is a **refusal**, never a slow mode (§3.4, decision Q6), and it too
/// happens before the blocker.
#[test]
fn a_machine_without_the_accelerator_is_refused_before_the_blocker_is_taken() {
    let host = Arc::new(MockQemuHost::with_policy(MockPolicy {
        kvm_enabled: false,
        ..MockPolicy::default()
    }));
    assert_eq!(
        QemuMachine::realize(config(), Arc::clone(&host) as Arc<_>).err(),
        Some(VmmError::Unsupported(NOT_ACCELERATED)),
        "the lockless-IO opt-out is honoured only on the accelerator's dispatch path, so \
         an interpreted machine runs this device under the VMM's global lock on every \
         access — a measured 5.3x amplification with no way to see it from inside"
    );
    assert_eq!(
        host.log(),
        vec![HostCall::Version, HostCall::KvmEnabled],
        "and it asks for nothing it would then have to give back"
    );
}

/// ★★ §8.5's `-EBUSY` arm: a discard *requirer* is already present, and realize must
/// refuse **and withdraw the blocker it had already taken**.
///
/// This is the first step whose failure has something to unwind, and the unwinding is the
/// assertion — a machine left permanently unmigratable by a device that failed to realize
/// is a worse outcome than the failure.
#[test]
fn a_discard_requirer_makes_realize_refuse_and_give_back_the_blocker_it_took() {
    let host = Arc::new(MockQemuHost::with_policy(MockPolicy {
        discard_refuses: Some(HostError::Busy {
            what: "a device that requires guest-driven discard",
        }),
        ..MockPolicy::default()
    }));
    assert_eq!(
        QemuMachine::realize(config(), Arc::clone(&host) as Arc<_>).err(),
        Some(VmmError::Unsupported(DISCARD_REQUIRER_PRESENT)),
        "the conflict is named, not folded into a generic host refusal — an operator must \
         be able to tell a configuration clash from a bug in us"
    );
    assert!(
        host.blockers().is_empty(),
        "★ the blocker taken one step earlier is withdrawn. Without this the machine is \
         unmigratable forever because of a device that does not exist"
    );
    let log = host.log();
    assert!(
        matches!(log.last(), Some(HostCall::DelBlocker(_))),
        "and the withdrawal is the LAST thing it does ({log:?})"
    );
    assert!(
        !host.discard_disabled(),
        "the refusal never took effect, so nothing must claim it did"
    );
}

/// ★★★ **The partial realize, swept over every step that can fail.**
///
/// The device publishes N regions. For each k, the k-th publication is refused, and the
/// property asserted is the same one every time: **nothing survives**. Not "it returned an
/// error" — the host address space, the blocker and the discard refusal are all measured
/// afterwards, because a ledger that only increments on success cannot see a leak.
///
/// Sweeping k rather than witnessing one is what makes this a test of the unwinding
/// rather than of one arm of it: with a single k, an unwind that handles the first failure
/// and not the second passes.
#[test]
fn a_partial_realize_gives_back_every_region_it_had_already_published() {
    let cfg = config();
    let publications = cfg.windows.len() + cfg.overlays.len();
    assert!(
        publications >= 2,
        "★ NON-VACUITY: the sweep below needs at least two publications for 'unwind the \
         ones that already landed' to differ from 'unwind nothing'"
    );
    for k in 0..publications {
        let host = Arc::new(MockQemuHost::with_policy(MockPolicy {
            publish_refuses_at: Some(k as u64),
            ..MockPolicy::default()
        }));
        let got = QemuMachine::realize(config(), Arc::clone(&host) as Arc<_>);
        let what = if k < cfg.windows.len() {
            "publishing a reservation"
        } else {
            "publishing a read-native overlay"
        };
        assert_eq!(
            got.err(),
            Some(VmmError::HostRefused {
                what,
                errno: Some(12)
            }),
            "publication {k} was refused, and WHICH operation was refused is named — an \
             operator reading a log must be able to tell a reservation from an overlay"
        );
        assert_eq!(
            host.live_regions(),
            Vec::new(),
            "★ publication {k}: every region published BEFORE the failure is given back. \
             This is measured on the host, not inferred from a counter we increment"
        );
        assert!(
            host.blockers().is_empty(),
            "publication {k}: and the migration blocker with it"
        );
        assert!(
            !host.discard_disabled(),
            "publication {k}: and the discard refusal is lifted, or the next device in \
             this machine inherits a restriction from one that never realized"
        );
    }
}

/// ★ The listener registration is the last step, so its failure unwinds the most.
#[test]
fn a_listener_that_cannot_be_registered_unwinds_the_whole_device() {
    let host = Arc::new(MockQemuHost::with_policy(MockPolicy {
        listener_refuses: Some(HostError::Refused {
            what: "registering a listener",
            errno: Some(1),
        }),
        ..MockPolicy::default()
    }));
    assert_eq!(
        QemuMachine::realize(config(), Arc::clone(&host) as Arc<_>).err(),
        Some(VmmError::HostRefused {
            what: "registering the topology listener",
            errno: Some(1),
        }),
    );
    assert_eq!(
        host.live_regions(),
        Vec::new(),
        "the reservation and the overlay were both published before this step, and both \
         must be given back"
    );
    assert!(host.blockers().is_empty() && !host.discard_disabled());
}

/// ★★★ **§4.3 as a mechanism.** The design states *"the adapter contains ZERO calls to
/// `bql_lock`"* as a discipline; here it is a latch with a negative test.
///
/// A topology transaction after realize would have to take the VMM's global lock, because
/// no thread that reaches this method is holding it. So it is refused by name and
/// **counted** — the counter is what makes the refusal string provably reachable rather
/// than a constant nothing evaluates.
#[test]
fn a_topology_transaction_after_realize_is_refused_by_name_and_counted() {
    let (m, host) = machine();
    let published_at_realize = m.audit().regions_published;
    let before = host.log().len();

    let p = page();
    assert_eq!(
        m.install_ram_window(BarId::Bar1, common::BAR1_BASE + 32 * p, 2 * p)
            .err(),
        Some(VmmError::Unsupported(TOPOLOGY_AFTER_REALIZE)),
        "★ a reservation created after realize is a topology transaction on a thread that \
         does not hold the VMM's global lock — the one thing section 4.3 says this adapter \
         must never construct"
    );
    assert_eq!(
        m.audit().topology_ops_refused_after_realize,
        1,
        "★ NON-VACUITY: the refusal is counted, so 'that string is unreachable' is a \
         failing assertion rather than an unexamined possibility"
    );
    assert_eq!(
        host.log().len(),
        before,
        "and the host was not asked for anything at all — a refusal that had already \
         published the region would be the violation with a tidy return value"
    );
    assert_eq!(
        m.audit().regions_published,
        published_at_realize,
        "★ the frequency claim: the count of regions handed to the hypervisor stops moving \
         the instant realize returns, whatever the guest asks for afterwards"
    );
}

/// ★★ Unrealize is the conservation ledger: every region given back, every reference
/// released, the blocker withdrawn and the discard refusal lifted.
#[test]
fn unrealize_gives_back_every_region_and_reference_and_the_blocker() {
    let (m, host) = machine();
    assert_eq!(
        host.live_regions().len(),
        2,
        "★ NON-VACUITY: there is something to give back — one reservation and one overlay"
    );
    m.unrealize();
    assert_eq!(
        host.live_regions(),
        Vec::new(),
        "every region this device published is gone"
    );
    assert!(
        host.blockers().is_empty(),
        "and the machine is migratable again"
    );
    assert!(
        !host.discard_disabled(),
        "and the next device does not inherit our discard refusal"
    );
    assert_eq!(
        m.audit().window_mappings_released,
        1,
        "★ and the reservation's mapping came back. The hypervisor frees nothing of ours \
         — it never took ownership — so this step is not optional and has no backstop"
    );
}

/// ★★ The Rust half of §3.3's coverage clause: a trap the realize-time table never named
/// is refused, because a trapped region nobody enumerated is a region nobody marked.
///
/// Swept over containment, mode and BAR, because the three failure directions are
/// independent and a single case would test whichever one it happened to pick.
#[test]
fn a_trap_outside_the_realize_time_table_is_refused_however_it_misses() {
    let (m, _host) = machine();
    let mut v = m.vmm();
    let p = page();

    // Inside a realized row, right mode: accepted.
    for (what, bar, range, mode) in [
        (
            "the whole read-write row",
            BarId::Bar0,
            0..16 * p,
            TrapMode::ReadWrite,
        ),
        (
            "a sub-range of the read-write row",
            BarId::Bar0,
            p..2 * p,
            TrapMode::ReadWrite,
        ),
        (
            "the write-only row",
            BarId::Bar0,
            16 * p..18 * p,
            TrapMode::WriteOnly,
        ),
    ] {
        assert_eq!(
            v.set_trap(bar, range, mode),
            Ok(()),
            "{what} is in the realized table and must be accepted"
        );
    }

    for (what, bar, range, mode, expected) in [
        (
            "a range past the end of every row",
            BarId::Bar0,
            32 * p..33 * p,
            TrapMode::ReadWrite,
            VmmError::Unsupported(TRAP_OUTSIDE_THE_REALIZED_TABLE),
        ),
        (
            "a range straddling the two rows",
            BarId::Bar0,
            15 * p..17 * p,
            TrapMode::ReadWrite,
            VmmError::Unsupported(TRAP_OUTSIDE_THE_REALIZED_TABLE),
        ),
        (
            "the right range with the WRONG mode",
            BarId::Bar0,
            0..p,
            TrapMode::WriteOnly,
            VmmError::Unsupported(TRAP_OUTSIDE_THE_REALIZED_TABLE),
        ),
        (
            "the right range in the wrong BAR",
            BarId::Bar1,
            0..p,
            TrapMode::ReadWrite,
            VmmError::Unsupported(TRAP_OUTSIDE_THE_REALIZED_TABLE),
        ),
        (
            "a BAR the device was never realized with",
            BarId::Bar2,
            0..p,
            TrapMode::ReadWrite,
            VmmError::Unsupported("a BAR this machine was not realized with"),
        ),
        (
            "a range past the end of the BAR itself",
            BarId::Bar0,
            0..1024 * p,
            TrapMode::ReadWrite,
            VmmError::Unsupported("a trap range outside its BAR"),
        ),
        (
            "an empty range",
            BarId::Bar0,
            p..p,
            TrapMode::ReadWrite,
            VmmError::Unsupported("an empty or inverted trap range"),
        ),
        (
            "an inverted range",
            BarId::Bar0,
            2 * p..p,
            TrapMode::ReadWrite,
            VmmError::Unsupported("an empty or inverted trap range"),
        ),
    ] {
        assert_eq!(
            v.set_trap(bar, range, mode).err(),
            Some(expected),
            "{what}: the four refusals are near neighbours and must never start reporting \
             as one another"
        );
    }
}

/// ★ A device realized with an empty trap table accepts **nothing** — the degenerate case
/// that proves the validation is against the table and not against the BAR.
#[test]
fn a_device_with_no_realized_traps_accepts_no_trap_registration_at_all() {
    let cfg = MachineConfig {
        traps: Vec::new(),
        ..config()
    };
    let (m, _host) = machine_with(MockPolicy::default(), cfg);
    let mut v = m.vmm();
    assert_eq!(
        v.set_trap(BarId::Bar0, 0..page(), TrapMode::ReadWrite)
            .err(),
        Some(VmmError::Unsupported(TRAP_OUTSIDE_THE_REALIZED_TABLE)),
        "with no rows in the table there is no trapped region to register against, and \
         accepting one would be bookkeeping for a trap that does not exist"
    );
}

/// ★ A realize-time trap row that names a BAR the device does not have is a configuration
/// error, and it must not be silently ignorable through `set_trap` either.
#[test]
fn a_realized_trap_row_in_an_absent_bar_cannot_be_registered_against() {
    let mut cfg = config();
    cfg.traps.push(TrapSpec {
        bar: BarId::Bar2,
        range: 0..page(),
        mode: TrapMode::ReadWrite,
    });
    let (m, _host) = machine_with(MockPolicy::default(), cfg);
    let mut v = m.vmm();
    assert_eq!(
        v.set_trap(BarId::Bar2, 0..page(), TrapMode::ReadWrite)
            .err(),
        Some(VmmError::Unsupported(
            "a BAR this machine was not realized with"
        )),
        "the BAR check runs BEFORE the table check, so a row naming an absent BAR reports \
         the absent BAR rather than an apparently-missing table row"
    );
}

/// ★★ A reservation whose geometry the device cannot honour is refused **at realize**, by
/// the exact reason, swept over all five ways it can be wrong.
///
/// Found by a bite-check: without the empty-length and past-the-end rows, deleting either
/// check survived the whole suite. Neither is harmless. A zero-length reservation reaches
/// the mapping layer and comes back with a *different* refusal, so an operator reading the
/// log is told the host refused a mapping when in fact the configuration is nonsense; and
/// a reservation that runs past its BAR is only caught by the hypervisor's own subregion
/// check, which is a check we do not own and must not rely on.
#[test]
fn a_reservation_with_geometry_the_device_cannot_honour_is_refused_at_realize_by_name() {
    let p = page();
    let bad_geometry = "a reservation whose base or length is not a whole number of host pages";
    let outside_bar = "a reservation outside the BAR it names";
    for (what, gpa, len, expected) in [
        (
            "a zero-length reservation",
            common::BAR1_BASE,
            0,
            bad_geometry,
        ),
        (
            "a base that is not a whole number of host pages",
            common::BAR1_BASE + 8,
            p,
            bad_geometry,
        ),
        (
            "a length that is not a whole number of host pages",
            common::BAR1_BASE,
            p + 8,
            bad_geometry,
        ),
        (
            "a reservation that starts inside its BAR and runs past the end",
            common::BAR1_BASE + 60 * p,
            8 * p,
            outside_bar,
        ),
        (
            "a reservation below its BAR's base",
            common::BAR1_BASE - 4 * p,
            2 * p,
            outside_bar,
        ),
    ] {
        let mut cfg = config();
        cfg.windows = vec![kayfabe_vmm_qemu::WindowSpec {
            bar: BarId::Bar1,
            gpa,
            len,
        }];
        let host = Arc::new(MockQemuHost::new());
        assert_eq!(
            QemuMachine::realize(cfg, Arc::clone(&host) as Arc<_>).err(),
            Some(VmmError::Unsupported(expected)),
            "{what}: the refusal must name the CONFIGURATION, not whatever the host \
             happened to say about a request we should never have made"
        );
        assert_eq!(
            host.live_regions(),
            Vec::new(),
            "{what}: and nothing was published"
        );
        assert!(
            host.blockers().is_empty(),
            "{what}: and the blocker was given back"
        );
    }

    // The control: the same shape with legal geometry realizes.
    let host = Arc::new(MockQemuHost::new());
    assert!(
        QemuMachine::realize(config(), Arc::clone(&host) as Arc<_>).is_ok(),
        "★ NON-VACUITY: legal geometry still realizes, so the sweep above is about \
         geometry and not about a device that stopped accepting reservations"
    );
}
