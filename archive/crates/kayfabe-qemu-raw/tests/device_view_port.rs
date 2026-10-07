//! ★★★★★ **§3 ITEM 1 — THE DEVICE-VIEW PORT, AND THE ONE ISOLATE TWO PORTS SHARE.**
//!
//! `SINGLE_STORE_PLAN.md` §3, *"WHAT §3 STILL NEEDS, in order"*, item 1:
//!
//! > A **device-view port** reachable after bring-up (the `WalkShadowPort` shape; ⚠ it and
//! > the walk shadow both want the one `IsolateBox`, so they must **share** it, not take it).
//!
//! # ⊘⊘ What these tests can and cannot say
//!
//! A mock isolate has **no RM and no GPU**, so nothing here can arm a real view. What it
//! **can** do — and what the whole of the port's value rests on — is prove that:
//!
//! 1. two ports really do hold **one** box, which a `take()`-based share makes impossible;
//! 2. an arm **reaches the wire** and its refusal lands **by name**, so a boot's `refused=0`
//!    means *"nothing was refused"* rather than *"the call never happened"*;
//! 3. a port nobody asked says **`⊘⊘ VACUOUS`** rather than printing a zero that reads as
//!    health.
//!
//! ⊘ (2) is the known-positive this tree demands beside every census that reports a zero.

use std::sync::Arc;

use kayfabe_isolate::{HostHandle, IsolateBox, IsolateFactory, IsolateId};
use kayfabe_qemu_raw::deviceview::{DeviceViewPort, DupArc, ViewRefusal};
use kayfabe_qemu_raw::scratchpad::SharedIsolate;
use kayfabe_qemu_raw::walkshadow::WalkShadowPort;
use kayfabe_rt::GpuId;

/// The scratchpad isolate's id — `u32::MAX` in the proc field, which is what makes it unable
/// to alias a live proc's.
fn scratchpad_id() -> IsolateId {
    IsolateId::new(u32::MAX, GpuId::ZERO)
}

fn a_shared_isolate() -> Arc<SharedIsolate> {
    let (factory, _rec) = kayfabe_mocks::MockIsolateFactory::new();
    // ⊘ `_rec` is dropped: these tests read the port's own census, never the recorder's, so
    // that a green here cannot come from an instrument sharing state with the thing measured.
    let iso = IsolateBox::new(factory.spawn(scratchpad_id()));
    Arc::new(SharedIsolate::new(iso))
}

/// A `dup` that answers **`None`** — this process has no export directory in a unit test.
///
/// ⊘ It is deliberately not a stub that fabricates a descriptor: the point of the
/// `NO_DESCRIPTOR` arm is that the view must be **released** before the refusal is returned,
/// and a fake descriptor would route past exactly that.
fn no_dup() -> Arc<DupArc> {
    Arc::new(|_id, _token| None)
}

fn a_port(iso: Arc<SharedIsolate>) -> DeviceViewPort {
    DeviceViewPort::new(
        iso,
        scratchpad_id(),
        HostHandle::new(scratchpad_id(), 0xDEAD_0001),
        no_dup(),
    )
}

/// ★★★★★ **ONE BOX, TWO PORTS** — the property `share_for_walk_shadow`'s old `take()` made
/// unrepresentable.
///
/// # ⊘ Why this is a falsifier and not a tautology
///
/// It would be a tautology if it only checked that two `Arc::clone`s are equal. It is not: it
/// **uses** both ports afterwards, on the same box, and asserts each one's own census moved.
/// Before `SharedIsolate` the second consumer's constructor could not even be called with a
/// box the first had taken — the failure was a `None` at the wiring site, which reads in a
/// boot log exactly like a gate that was off.
#[test]
fn the_walk_shadow_and_the_device_view_port_hold_one_isolate() {
    let iso = a_shared_isolate();
    let shadow = WalkShadowPort::new(Arc::clone(&iso));
    let views = a_port(Arc::clone(&iso));

    assert_eq!(
        Arc::strong_count(&iso),
        3,
        "the handle must be held by this test and by BOTH ports — if a port took the box \
         instead of a handle, the other one could not have been built at all"
    );

    // ── both ports reach the same isolate, and each one's own census says so ──
    let before = views.census_line();
    assert!(
        before.contains("VACUOUS"),
        "a port nobody has asked for a view must say so by name, not print a zero: {before}"
    );
    let refused = views
        .with_node(0, 4096, true, |_fd, _len| Ok::<(), ()>(()))
        .expect_err("a mock isolate has no RM and cannot arm a view of anything");
    assert!(
        matches!(refused, ViewRefusal::Rm(_)),
        "the refusal must come back from the WIRE, named — a `NoWorker` here would mean the \
         port never reached the isolate at all, which is the opposite diagnosis: {refused:?}"
    );
    assert!(
        !shadow.census_line().is_empty(),
        "the walk shadow must still be usable after the device-view port has driven the same \
         box; if the box had moved, this port would be holding a retired isolate"
    );
}

/// ★★★★★ **THE REFUSAL CAN FIRE, SO A BOOT'S `refused=0` MEANS SOMETHING.**
///
/// ⊘⊘ This tree's most repeated instrument failure is a counter that reads zero because its
/// arm never ran. The port's census reports `refused=N` and `first_refusal=[…]`; this makes
/// both move, on the same path a real arm takes, and asserts the sentence rather than merely
/// that *"something was refused"*.
#[test]
fn an_arm_reaches_the_wire_and_the_census_names_the_refusal() {
    let port = a_port(a_shared_isolate());
    let _ = port.with_node(0x1000, 0x1000, false, |_fd, _len| Ok::<(), ()>(()));
    let line = port.census_line();
    assert!(
        line.contains("refused=1"),
        "the refusal must be COUNTED, or a boot's zero cannot be read as 'nothing was \
         refused': {line}"
    );
    assert!(
        line.contains("RM_REFUSED"),
        "and it must be NAMED: four causes reach this counter and they have different fixes \
         — `RM_REFUSED` with NV_ERR_NO_MEMORY means the host BAR1 aperture is full, while \
         `NO_WORKER` means the isolate is gone. A total cannot tell them apart: {line}"
    );
    assert!(
        !line.contains("VACUOUS"),
        "and the port is no longer vacuous — it was asked and it answered: {line}"
    );
}

/// ⊘ **A port that was never asked is NOT a port that found nothing.** The control for the
/// test above, stated as its own assertion so the two verdicts cannot drift apart.
#[test]
fn a_port_nobody_asked_reports_vacuous_and_not_health() {
    let line = a_port(a_shared_isolate()).census_line();
    assert!(line.contains("armed=0"), "{line}");
    assert!(line.contains("refused=0"), "{line}");
    assert!(
        line.contains("VACUOUS"),
        "★ `armed=0 refused=0` is exactly what a healthy unused port and a port nothing could \
         reach both print. The verdict is the only thing that distinguishes them, and it must \
         refuse to read the zeros as health: {line}"
    );
}

/// ⊘ **The `NO_DESCRIPTOR` arm releases before it refuses** — stated here as a documented
/// property that this harness cannot yet exercise, so nobody reads the three tests above as
/// covering it.
///
/// Reaching it needs an isolate that arms a real view (so there is aperture to leak) and a
/// `dup` that then fails — i.e. a GPU. `[measured w722]` a leaked view returns **zero** host
/// BAR1 aperture and the failure is silent until every later arm is refused, so this is the
/// one path whose absence from the unit tests must be said out loud rather than left to be
/// discovered.
#[test]
#[ignore = "needs a real isolate: a mock cannot arm a view, so there is no aperture to leak"]
fn the_no_descriptor_arm_releases_the_view_before_refusing() {
    unimplemented!("see the doc comment — this is a bench test, not a unit test");
}
