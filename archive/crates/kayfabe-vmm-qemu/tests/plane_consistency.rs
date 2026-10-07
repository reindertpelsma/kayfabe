//! ★★★ The controls that could not fire — and the shapes that make them fire.
//!
//! Three defects of one family, all found by the 2026-07-29 seam audit and all in this
//! adapter:
//!
//! 1. **`registered_traps` was write-only.** [`Vmm::set_trap`] pushed to it and nothing
//!    read it. `set_trap` validates its precondition once, at registration; a reservation
//!    installed afterwards falsifies it and the trap then survives only in that vector.
//!    The sibling KVM adapter had exactly this and fixed it; the fix is ported here as
//!    [`QemuMachine::assert_map_matches_the_kernel`].
//! 2. **`install_window` had no check against an existing reservation**, relying on the
//!    kernel's `EEXIST` — which an observe-tiered span, having no memslot at all, never
//!    reaches.
//! 3. **`remove_window` pruned its published tables by GEOMETRY**, not by ownership. That
//!    is the worst member of the family: it does not break a guard directly, it deletes the
//!    guard's *input*, after which the guard passes and looks exactly like a guard that
//!    works.
//!
//! Every test here is written so that removing the fix makes it red — the file exists to be
//! bitten, not to be read.

mod common;

use common::{
    BAR0_BASE, BAR1_BASE, FOREIGN_RAM, machine, machine_with, overlay_gpa, overlay_len,
    overlay_trap, page, ram_facts, window_gpa, window_len,
};

use kayfabe_vmm::{BarId, HostRegion, TrapMode, Vmm, VmmError};
use kayfabe_vmm_qemu::mock_host::MockPolicy;
use kayfabe_vmm_qemu::slots::Tier;
use kayfabe_vmm_qemu::{
    FOREIGN_OVERLAPS_OURS, MachineConfig, TRAP_OVER_A_LIVE_SLOT, WINDOW_OVER_A_LIVE_RESERVATION,
    WindowSpec,
};

/// The port's "fill it from nothing" sentinel.
const FILL_FROM_NOTHING: HostRegion = HostRegion {
    id: u64::MAX,
    offset: 0,
};

/// Run `f` and return the panic message it produced, or fail saying it did not panic.
fn must_panic(what: &str, f: impl FnOnce()) -> String {
    let e = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .err()
        .unwrap_or_else(|| panic!("{what}: this MUST panic, and it returned normally"));
    e.downcast_ref::<String>()
        .cloned()
        .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_default()
}

// =====================================================================================
// 1. The write-only vector
// =====================================================================================

/// ★★★ **A trap registration that a LATER reservation falsifies.**
///
/// `set_trap` is right at the instant it is called and wrong a moment later, and until
/// `registered_traps` was read by something, *nothing in this device could tell*. Every
/// other instrument stays green: the slot ledger balances, the tiering is exactly what was
/// asked for, `tiers()` reports the truth, the audit conserves. The registration is the
/// only thing that has become a lie, and it lives in a vector nobody consults.
#[test]
fn a_trap_registration_a_later_reservation_falsifies_is_caught_only_by_the_plane_check() {
    let p = page();
    let (m, _host, slots) = machine();
    let mut v = m.vmm();

    // BAR0 0..16p is the realize-time read-write trap row. Nothing is installed over
    // 8p..10p, so the registration is honest when it is made.
    v.set_trap(BarId::Bar0, 8 * p..10 * p, TrapMode::ReadWrite)
        .expect("a read-write trap over an unbacked range is exactly what a trap is");
    assert_eq!(
        m.assert_map_matches_the_kernel(),
        1,
        "★ NON-VACUITY: the check saw the realize-time reservation, so a later green is a \
         green about something"
    );

    // Now put a passthrough reservation over the very range that trap names. Nothing here
    // refuses — and nothing should: a device may legitimately back a range it once
    // trapped. What it may not do is keep claiming the range is trapped.
    let over = m
        .install_ram_window(BAR0_BASE + 8 * p, 2 * p)
        .expect("backing a previously-trapped range is legal");

    // ★ Everything else still says the device is fine.
    assert_eq!(m.audit().live_memslots, 2, "the ledger balances");
    assert_eq!(slots.replaces(), 0, "no slot was silently replaced");
    assert!(
        m.tiers()
            .iter()
            .any(|(g, _, t)| *g == BAR0_BASE + 8 * p && *t == Tier::Passthrough),
        "and the tiering reports exactly what was asked for"
    );

    // ★★★ The one instrument that can see it.
    let msg = must_panic("a falsified trap registration", || {
        let _ = m.assert_map_matches_the_kernel();
    });
    assert!(
        msg.contains("no longer makes it one") && msg.contains("never leaves the guest"),
        "and it must say WHICH registration stopped being true and why, not merely fail: \
         {msg}"
    );

    // ★ Nothing more is asked of `m`, deliberately: the assertion panicked with the view
    // and installer locks held, so both are now poisoned and every further method on this
    // machine would abort on the poison rather than on the plane. That is the correct
    // shape for a check that only ever runs in a test — but it makes "one caught panic per
    // machine" a rule, not a style.
    let _ = over;
}

/// ★★ The same seam in the **write-only** direction: a `WriteOnly` registration is legal
/// only over a read-native tier, and removing that overlay must make the check fire.
#[test]
fn a_write_only_registration_outlives_its_read_only_slot_and_the_check_says_so() {
    let p = page();
    let (m, _host, _slots) = machine();
    let mut v = m.vmm();

    let slot = v
        .map_read_native(
            overlay_gpa(),
            overlay_len(),
            FILL_FROM_NOTHING,
            Some(overlay_trap()),
        )
        .expect("the read-native reservation the trap row is about");
    v.set_trap(BarId::Bar0, 16 * p..17 * p, TrapMode::WriteOnly)
        .expect("and now the registration is honest");
    assert_eq!(
        m.assert_map_matches_the_kernel(),
        2,
        "★ NON-VACUITY: two live reservations were checked"
    );

    v.unmap_guest(slot).expect("the overlay goes");
    let msg = must_panic("a write-only trap with no read-only slot left", || {
        let _ = m.assert_map_matches_the_kernel();
    });
    assert!(
        msg.contains("no read-native tier covers"),
        "and for THAT reason — its writes now land in RAM instead of exiting: {msg}"
    );
}

// =====================================================================================
// 2. The reservation the kernel could not refuse
// =====================================================================================

/// ★★★ **An overlapping reservation, in the shape the kernel cannot see.**
///
/// `install_window` relied on `KVM_SET_USER_MEMORY_REGION`'s `EEXIST`. But the kernel is
/// only ever told about spans that get a **memslot**, and [`Tier::Observe`] is the tier
/// defined by having none. So the collision is invisible to it from either side, and the
/// mock — which models the kernel faithfully, including this — cannot see it either.
#[test]
fn a_reservation_over_a_live_one_is_refused_even_where_no_memslot_could_collide() {
    let p = page();
    let (m, _host, slots) = machine_with(
        MockPolicy::default(),
        MachineConfig {
            // One reservation whose middle two pages are observe-tiered: no memslot there.
            windows: vec![WindowSpec {
                gpa: window_gpa(),
                len: 8 * p,
                observe: core::iter::once(window_gpa() + 2 * p..window_gpa() + 4 * p)
                    .collect::<Vec<_>>(),
            }],
            ..common::config()
        },
    );
    let installs_before = slots.installs();

    // (a) Wholly inside the observe hole: NO memslot exists over this range and none would
    // be installed for a passthrough window either… except one would, which is precisely
    // the danger — two of our reservations claiming one guest-physical range, resolved by
    // `BTreeMap` iteration order.
    assert_eq!(
        m.install_ram_window(window_gpa() + 2 * p, 2 * p),
        Err(VmmError::Unsupported(WINDOW_OVER_A_LIVE_RESERVATION)),
        "★ the collision the kernel is never asked about"
    );
    // (b) An observe-tiered newcomer over a passthrough incumbent — the same blindness from
    // the other side.
    assert_eq!(
        m.install_tiered_window(&WindowSpec {
            gpa: window_gpa() + 6 * p,
            len: 2 * p,
            observe: core::iter::once(window_gpa() + 6 * p..window_gpa() + 8 * p)
                .collect::<Vec<_>>(),
        }),
        Err(VmmError::Unsupported(WINDOW_OVER_A_LIVE_RESERVATION))
    );
    // (c) Strictly containing it, and (d) exactly equal to it.
    assert_eq!(
        m.install_ram_window(window_gpa(), 16 * p),
        Err(VmmError::Unsupported(WINDOW_OVER_A_LIVE_RESERVATION))
    );
    assert_eq!(
        m.install_ram_window(window_gpa(), 8 * p),
        Err(VmmError::Unsupported(WINDOW_OVER_A_LIVE_RESERVATION))
    );
    // (e) Overlapping by exactly one page at the tail — the off-by-one both directions.
    assert_eq!(
        m.install_ram_window(window_gpa() + 7 * p, 2 * p),
        Err(VmmError::Unsupported(WINDOW_OVER_A_LIVE_RESERVATION))
    );

    // ★ NON-VACUITY, and the whole point of refusing in the PLAN phase: not one of those
    // five refusals mapped a reservation or installed a slot first.
    assert_eq!(
        slots.installs(),
        installs_before,
        "a refused reservation must not have installed anything to unwind"
    );
    assert_eq!(m.audit().live_windows, 1);
    assert_eq!(m.audit().host_refusals, 0, "the host was never even asked");

    // ★ And a reservation that merely ABUTS is accepted, so the check is about overlap and
    // not about proximity.
    let next = m
        .install_ram_window(window_gpa() + 8 * p, 2 * p)
        .expect("touching, not overlapping");
    assert_eq!(m.assert_map_matches_the_kernel(), 2);
    m.remove_window(next).expect("remove");
    assert_eq!(
        m.install_ram_window(window_gpa() + 4 * p, 2 * p),
        Err(VmmError::Unsupported(WINDOW_OVER_A_LIVE_RESERVATION)),
        "and removing the neighbour did not un-claim the incumbent's range"
    );
}

/// ★★ A read-native overlay is a reservation like any other, so `map_read_native` over a
/// live one is refused too — and refused **without** leaving a half-built overlay behind.
#[test]
fn map_read_native_over_a_live_reservation_is_refused_and_leaves_nothing() {
    let (m, _host, slots) = machine();
    let mut v = m.vmm();
    let before = (slots.installs(), slots.live().len());
    assert_eq!(
        v.map_read_native(
            window_gpa(),
            window_len(),
            FILL_FROM_NOTHING,
            Some(window_gpa()..window_gpa() + page()),
        ),
        Err(VmmError::Unsupported(WINDOW_OVER_A_LIVE_RESERVATION)),
        "the realize-time reservation is already there"
    );
    assert_eq!(
        (slots.installs(), slots.live().len()),
        before,
        "and nothing was built to unwind"
    );
    assert_eq!(m.assert_map_matches_the_kernel(), 1);
}

// =====================================================================================
// 3. The tables pruned by geometry
// =====================================================================================

/// ★★★ **Removing a reservation that spans its whole BAR must not delete the BAR's own
/// claim.**
///
/// `View::ours` holds one row per realize-time BAR *and* one per reservation, and
/// `remove_window` pruned it by matching `(gpa, len)` exactly. A reservation over a whole
/// BAR — `WindowSpec::passthrough(bar.base, bar.len)`, the ordinary full-BAR shape — has
/// the same `(gpa, len)` as the BAR, so removing it deleted the BAR's row as collateral.
///
/// What that costs is not a stale table. `ours` is the input to
/// [`FOREIGN_OVERLAPS_OURS`], so afterwards a reported topology section can declare over a
/// range this device owns and the guard **passes**, having been handed an empty list. A
/// guard defeated by a bookkeeping bug is indistinguishable from a guard that works.
#[test]
fn removing_a_full_bar_reservation_must_not_delete_the_bars_own_claim() {
    let p = page();
    // A machine with no realize-time reservation, so BAR1 is free for a full-BAR one.
    let (m, host, _slots) = machine_with(
        MockPolicy::default(),
        MachineConfig {
            windows: Vec::new(),
            ..common::config()
        },
    );

    // The guard works before anything is removed.
    let s = host.mint_foreign(BAR1_BASE, 4 * p, ram_facts());
    assert_eq!(
        m.region_add(s),
        Err(VmmError::Unsupported(FOREIGN_OVERLAPS_OURS)),
        "★ NON-VACUITY: the claim is real to begin with"
    );

    // A reservation over the WHOLE of BAR1 — same base, same length as the BAR itself.
    let whole = m
        .install_ram_window(BAR1_BASE, 1024 * p)
        .expect("a full-BAR reservation");
    assert_eq!(m.assert_map_matches_the_kernel(), 1);
    m.remove_window(whole).expect("and it goes away again");

    // ★★★ The BAR is still ours.
    let s = host.mint_foreign(BAR1_BASE, 4 * p, ram_facts());
    assert_eq!(
        m.region_add(s),
        Err(VmmError::Unsupported(FOREIGN_OVERLAPS_OURS)),
        "★ removing the reservation must not have un-claimed the BAR underneath it — a \
         foreign section here would race us to declare the same guest-physical range"
    );
    // …and so is BAR0, which was never involved.
    let s = host.mint_foreign(BAR0_BASE, 4 * p, ram_facts());
    assert_eq!(
        m.region_add(s),
        Err(VmmError::Unsupported(FOREIGN_OVERLAPS_OURS))
    );
    // A section far from both is still accepted, so the guard has not become a blanket no.
    let s = host.mint_foreign(FOREIGN_RAM, 4 * p, ram_facts());
    m.region_add(s)
        .expect("★ NON-VACUITY: ordinary guest RAM is still accepted");
    assert!(
        m.resolve_region(FOREIGN_RAM, p).is_ok(),
        "and it really did become a resolvable region"
    );
}

// =====================================================================================
// The mean run
// =====================================================================================

/// ★★★ **The mean shape**: reservations of three arities appearing and disappearing in an
/// order that is neither stack-like nor queue-like, a live trap registration standing over
/// the whole thing, foreign sections arriving in the gaps — with the plane interrogated
/// after **every** step.
///
/// Written against the pruning bugs specifically: each removal is of a reservation that has
/// live neighbours on both sides, and the survivors' tier rows are re-checked afterwards by
/// asking `set_trap` — the consumer of those rows — rather than by reading them.
#[test]
fn the_plane_agrees_with_itself_across_a_mean_reservation_churn() {
    let p = page();
    let (m, host, slots) = machine_with(
        MockPolicy::default(),
        MachineConfig {
            windows: Vec::new(),
            // BAR1 is where the churn happens, so the realize-time table must have a row
            // for it — `set_trap` refuses anything outside the table before it ever gets to
            // ask about the tiering, and a test that took that refusal would be asserting
            // the wrong guard.
            traps: {
                let mut t = common::config().traps;
                t.push(kayfabe_vmm_qemu::TrapSpec {
                    bar: BarId::Bar1,
                    range: 0..1024 * p,
                    mode: TrapMode::ReadWrite,
                });
                t
            },
            ..common::config()
        },
    );
    let mut v = m.vmm();

    // A read-native overlay first, with a write-only trap standing over its read-only span
    // for the rest of the run.
    v.map_read_native(
        overlay_gpa(),
        overlay_len(),
        FILL_FROM_NOTHING,
        Some(overlay_trap()),
    )
    .expect("the overlay");
    v.set_trap(BarId::Bar0, 16 * p..17 * p, TrapMode::WriteOnly)
        .expect("the write-only registration");
    // …and a read-write trap over a range nothing backs, likewise for the whole run.
    v.set_trap(BarId::Bar0, 8 * p..10 * p, TrapMode::ReadWrite)
        .expect("the read-write registration");

    // Reservations in BAR1, three at a time, retired oldest-first so every removal has a
    // live neighbour on both sides.
    let mut live: Vec<(kayfabe_vmm::RamRegionId, u64)> = Vec::new();
    let mut foreign_at = FOREIGN_RAM;
    for i in 0..40u64 {
        let gpa = BAR1_BASE + (i % 8) * 8 * p;
        // Alternate arity: a plain reservation (one slot) and a tiered one (three).
        let spec = if i % 2 == 0 {
            WindowSpec::passthrough(gpa, 4 * p)
        } else {
            WindowSpec {
                gpa,
                len: 4 * p,
                observe: core::iter::once(gpa + p..gpa + 2 * p).collect::<Vec<_>>(),
            }
        };
        let r = m
            .install_tiered_window(&spec)
            .unwrap_or_else(|e| panic!("reservation {i} must install ({e:?})"));
        live.push((r, gpa));
        if live.len() > 3 {
            m.remove_window(live.remove(0).0).expect("the oldest goes");
        }
        // A foreign section every few steps, in ranges far from both BARs.
        if i % 5 == 0 {
            let s = host.mint_foreign(foreign_at, 2 * p, ram_facts());
            m.region_add(s).expect("ordinary guest RAM");
            foreign_at += 4 * p;
        }
        assert_eq!(
            m.assert_map_matches_the_kernel(),
            live.len() + 1,
            "★ NON-VACUITY: every live reservation was checked at step {i}"
        );
        // ★★ The survivors' tier rows are still there, asked of the guard that consumes
        // them: a read-write trap over a range one of them backs must still be REFUSED.
        // Under a prune that deleted a neighbour's rows this succeeds, and the failure is
        // an accepted registration rather than a red assertion.
        for (_, g) in &live {
            let off = *g - BAR1_BASE;
            assert_eq!(
                v.set_trap(BarId::Bar1, off..off + p, TrapMode::ReadWrite),
                Err(VmmError::Unsupported(TRAP_OVER_A_LIVE_SLOT)),
                "★ step {i}: a live reservation at {g:#x} must still make a read-write trap \
                 over it impossible"
            );
        }
    }

    for (r, _) in live {
        m.remove_window(r).expect("teardown");
    }
    let a = m.audit();
    assert_eq!(a.live_windows, 1, "only the overlay remains");
    assert_eq!(a.live_memslots, 2, "and its two spans");
    assert_eq!(m.assert_map_matches_the_kernel(), 1);
    assert!(
        a.slot_numbers_recycled >= 40,
        "★ NON-VACUITY: the churn really re-issued slot numbers (got {})",
        a.slot_numbers_recycled
    );
    assert_eq!(
        slots.replaces(),
        0,
        "and never silently replaced a live slot"
    );

    m.unrealize();
    assert!(
        slots.live().is_empty(),
        "unrealize leaves the kernel holding nothing: {:?}",
        slots.live()
    );
    assert_eq!(slots.clears(), slots.installs());
}
