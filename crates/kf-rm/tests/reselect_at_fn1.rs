//! ★★ Re-selection at fn 1 (`docs/design/V3_DRIVER_MATRIX.md` §4.2, owner ruling 6): a device
//! whose guest version was DEFAULTED rebuilds its chain for the guest's own version when the
//! pair's pre-fn-1 surface is identical, and refuses by name otherwise; a DECLARED version is
//! never overridden.

use kf_abi::DriverVersion;
use kf_abi::guestsysinfo::SET_GUEST_SYSTEM_INFO_SIZE;
use kf_abi::versions::{DriverAbiTable, pre_fn1_surface_differs, table_for};
use kf_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};
use kf_rm::{GuestDriverSource, ReselectAtFn1, Reselection};
use std::sync::{Arc, Mutex};

fn v(major: u16, minor: u16, patch: u16) -> DriverVersion {
    DriverVersion {
        major,
        minor,
        patch,
    }
}

fn t(major: u16, minor: u16, patch: u16) -> DriverAbiTable {
    *table_for(v(major, minor, patch)).expect("measured")
}

/// A stub chain that answers every command with the version it was built for, so a test can
/// see which chain answered.
struct Stub(DriverVersion);
impl CommandPolicy for Stub {
    fn respond(&mut self, _cmd: &RpcCommand) -> Option<Reply> {
        let s = self.0.to_string();
        Some(Reply {
            rpc_result_private: None,
            rpc_result: 0,
            body: s.into_bytes(),
        })
    }
}

fn wrapper(
    provisional: DriverAbiTable,
    source: GuestDriverSource,
) -> (ReselectAtFn1, Arc<Mutex<Vec<DriverVersion>>>) {
    let built = Arc::new(Mutex::new(Vec::new()));
    let log = built.clone();
    let w = ReselectAtFn1::new(
        provisional,
        source,
        Box::new(move |table: DriverAbiTable| {
            log.lock().expect("lock").push(table.driver_version());
            Box::new(Stub(table.driver_version())) as Box<dyn CommandPolicy>
        }),
    );
    (w, built)
}

fn fn1(version: &str) -> RpcCommand {
    let mut payload = vec![0u8; SET_GUEST_SYSTEM_INFO_SIZE];
    payload[24..24 + version.len()].copy_from_slice(version.as_bytes());
    RpcCommand {
        function: RpcFunction::SetGuestSystemInfo,
        code: 1,
        sequence: 1,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

fn answered_as(w: &mut ReselectAtFn1, cmd: &RpcCommand) -> String {
    String::from_utf8(w.respond(cmd).expect("the stub answers").body).expect("utf8")
}

/// The measured pairs: every 580.x shares the pre-fn-1 surface; 570/575 do too (same element,
/// four-field init args, same RPC numbers and fn-1 body); 595 declares nine-field init args; 610
/// frames elements with MCTP.
#[test]
fn the_pre_fn1_surface_is_stated_per_pair() {
    let bench = t(580, 159, 4);
    for (a, b, c) in [
        (580u16, 105u16, 8u16),
        (580, 65, 6),
        (580, 178, 4),
        (575, 57, 8),
        (570, 148, 8),
    ] {
        assert_eq!(
            pre_fn1_surface_differs(&bench, &t(a, b, c)),
            None,
            "{a}.{b}.{c}"
        );
    }
    assert_eq!(
        pre_fn1_surface_differs(&bench, &t(595, 84, 0)),
        Some("MESSAGE_QUEUE_INIT_ARGUMENTS")
    );
    assert_eq!(
        pre_fn1_surface_differs(&bench, &t(610, 43, 2)),
        Some("GSP_MSG_QUEUE_ELEMENT")
    );
}

#[test]
fn a_defaulted_device_reselects_to_the_guests_own_version() {
    let (mut w, built) = wrapper(t(580, 159, 4), GuestDriverSource::Defaulted);
    assert_eq!(
        answered_as(&mut w, &fn1("580.105.08")),
        "580.105.08",
        "fn 1 itself is answered by the new chain"
    );
    assert_eq!(w.current(), v(580, 105, 8));
    assert_eq!(
        *built.lock().expect("lock"),
        vec![v(580, 159, 4), v(580, 105, 8)]
    );
    // A later fn 1 of the same version (a GSP re-init) is a no-op.
    assert_eq!(w.on_fn1(&fn1("580.105.08").payload), Reselection::Kept);
    assert_eq!(built.lock().expect("lock").len(), 2);
}

#[test]
fn a_declared_version_is_never_overridden() {
    let (mut w, built) = wrapper(t(580, 159, 4), GuestDriverSource::Declared);
    assert_eq!(
        w.on_fn1(&fn1("580.105.08").payload),
        Reselection::RefusedDeclared {
            reported: v(580, 105, 8)
        }
    );
    assert_eq!(
        answered_as(&mut w, &fn1("580.105.08")),
        "580.159.04",
        "the declared chain answers (and refuses)"
    );
    assert_eq!(built.lock().expect("lock").len(), 1);
}

#[test]
fn a_different_pre_fn1_surface_or_an_unmeasured_version_is_refused_by_name() {
    let (mut w, _) = wrapper(t(580, 159, 4), GuestDriverSource::Defaulted);
    assert_eq!(
        w.on_fn1(&fn1("610.43.02").payload),
        Reselection::RefusedSurface {
            reported: v(610, 43, 2),
            what: "GSP_MSG_QUEUE_ELEMENT"
        }
    );
    match w.on_fn1(&fn1("580.159.03").payload) {
        Reselection::RefusedUnserved { reported, why } => {
            assert_eq!(reported, v(580, 159, 3));
            assert!(why.contains("never measured"), "{why}");
        }
        other => panic!("an unmeasured version must be refused, got {other:?}"),
    }
    assert_eq!(w.current(), v(580, 159, 4));
    // an unserved version cannot be pinned (there is no table for it), so no pin is offered
    let (mut w, _) = wrapper(t(580, 159, 4), GuestDriverSource::Defaulted);
    let outcome = w.on_fn1(&fn1("580.159.03").payload);
    assert!(
        !w.describe(&outcome).expect("said").contains("guest-driver"),
        "{outcome:?}"
    );
}

/// ★ Device reset (`docs/design/V3_DEVICE_RESET.md`): the chain goes back to the provisional
/// version with a FRESH build (no state of the old chain survives), and the next life re-selects
/// on its own fn 1 — to another version, if the guest rebooted into one.
#[test]
fn a_reset_chain_is_a_fresh_provisional_build_and_the_next_life_reselects_again() {
    let (mut w, built) = wrapper(t(580, 159, 4), GuestDriverSource::Defaulted);
    assert_eq!(answered_as(&mut w, &fn1("580.105.08")), "580.105.08");
    w.reset_life();
    assert_eq!(w.current(), v(580, 159, 4));
    assert_eq!(
        answered_as(
            &mut w,
            &RpcCommand {
                function: RpcFunction::GspSetSystemInfo,
                code: 72,
                sequence: 1,
                payload: Vec::new(),
                elements: 1,
                delivered: Vec::new(),
            }
        ),
        "580.159.04",
        "the fresh provisional chain answers"
    );
    assert_eq!(
        *built.lock().expect("lock"),
        vec![v(580, 159, 4), v(580, 105, 8), v(580, 159, 4)],
        "one fresh build per reset"
    );
    assert_eq!(answered_as(&mut w, &fn1("580.65.06")), "580.65.06");
    assert_eq!(w.current(), v(580, 65, 6));
    w.reset_life();
    w.reset_life();
    assert_eq!(w.current(), v(580, 159, 4));
}

/// ★ The refusal says the fix (2026-11-20). A defaulted device that cannot re-select across
/// branches names the EXACT pin the operator must add: `add guest-driver=<what the guest
/// reported>`, and what the device defaulted to. Which pairs are refused is unchanged (the guest
/// never selects the host side), so the outcome is asserted next to the text.
#[test]
fn a_cross_branch_refusal_names_the_exact_pin_to_add() {
    for (host, guest, guest_says, what) in [
        // a 580 guest on a 595 / 610 host, and the reverse
        (
            t(595, 84, 0),
            v(580, 159, 4),
            "580.159.04",
            "MESSAGE_QUEUE_INIT_ARGUMENTS",
        ),
        (
            t(610, 43, 2),
            v(580, 159, 4),
            "580.159.04",
            "GSP_MSG_QUEUE_ELEMENT",
        ),
        (
            t(580, 159, 4),
            v(595, 84, 0),
            "595.84",
            "MESSAGE_QUEUE_INIT_ARGUMENTS",
        ),
        (
            t(580, 159, 4),
            v(610, 43, 2),
            "610.43.02",
            "GSP_MSG_QUEUE_ELEMENT",
        ),
        // 595 against 610
        (
            t(595, 84, 0),
            v(610, 43, 2),
            "610.43.02",
            "GSP_MSG_QUEUE_ELEMENT",
        ),
    ] {
        let defaulted = host.driver_version();
        let (mut w, built) = wrapper(host, GuestDriverSource::Defaulted);
        let outcome = w.on_fn1(&fn1(guest_says).payload);
        let Reselection::RefusedSurface { reported, what: w2 } = outcome.clone() else {
            panic!("{defaulted} host, {guest_says} guest: must stay refused, got {outcome:?}");
        };
        assert_eq!((reported, w2), (guest, what));
        assert_eq!(
            w.current(),
            defaulted,
            "a refusal never moves the host side"
        );
        assert_eq!(built.lock().expect("lock").len(), 1, "no chain was rebuilt");

        let msg = w.describe(&outcome).expect("a refusal is said");
        // the exact pin, spelled the way the property parses it
        assert!(
            msg.contains(&format!("add guest-driver={guest}")),
            "{defaulted} host, {guest_says} guest: the pin is missing: {msg}"
        );
        assert_eq!(
            kf_abi::DriverVersion::parse(&format!("{guest}")),
            Some(guest),
            "the pin round-trips through guest-driver="
        );
        // what the device defaulted to, and the first differing fact, stay in the text
        assert!(
            msg.contains(&defaulted.to_string()),
            "{defaulted} host: the defaulted version is missing: {msg}"
        );
        assert!(msg.contains(what), "{what} is missing: {msg}");
        assert!(
            msg.contains("cannot re-select"),
            "the refusal keeps its name: {msg}"
        );
    }
}

/// The pin is the GUEST's version, never the device's own: a message that told the operator to
/// pin what the device already defaulted to would be a no-op.
#[test]
fn the_pin_is_what_the_guest_reported_not_what_the_device_defaulted_to() {
    let (mut w, _) = wrapper(t(595, 84, 0), GuestDriverSource::Defaulted);
    let outcome = w.on_fn1(&fn1("580.159.04").payload);
    let msg = w.describe(&outcome).expect("said");
    assert!(msg.contains("add guest-driver=580.159.04"), "{msg}");
    assert!(
        !msg.contains("guest-driver=595.84"),
        "must not pin the host's own version: {msg}"
    );
    // the first differing fact is named, so the refusal can be looked up
    assert!(msg.contains("MESSAGE_QUEUE_INIT_ARGUMENTS"), "{msg}");
    // and the pair stays refused after an earlier fn 1 moved the chain within a branch: the
    // text still says what the device DEFAULTED to
    let (mut w, _) = wrapper(t(580, 159, 4), GuestDriverSource::Defaulted);
    assert!(matches!(
        w.on_fn1(&fn1("580.65.06").payload),
        Reselection::Reselected { .. }
    ));
    let outcome = w.on_fn1(&fn1("610.43.02").payload);
    let msg = w.describe(&outcome).expect("said");
    assert!(msg.contains("from 580.159.04 (the version"), "{msg}");
    assert!(msg.contains("add guest-driver=610.43.02"), "{msg}");
}

/// Every other outcome's message is byte-for-byte what it was, and none of them gains the pin
/// advice: a declared version is never overridden, and re-selection or an unserved version has
/// nothing to pin.
#[test]
fn every_other_outcome_message_is_unchanged() {
    let (mut w, _) = wrapper(t(580, 159, 4), GuestDriverSource::Declared);
    let declared = w.on_fn1(&fn1("595.84").payload);
    assert_eq!(
        w.describe(&declared).as_deref(),
        Some(
            "kf-rm: the guest says it is driver 595.84, the device was DECLARED 580.159.04; not \
             re-selecting a declared version"
        )
    );

    let (mut w, _) = wrapper(t(580, 159, 4), GuestDriverSource::Defaulted);
    let r = w.on_fn1(&fn1("580.105.08").payload);
    assert_eq!(
        w.describe(&r).as_deref(),
        Some(
            "kf-rm: guest driver RE-SELECTED at fn 1: 580.159.04 (defaulted) -> 580.105.08 (the \
             guest's own; pre-fn-1 surface identical)"
        )
    );
    let (mut w, _) = wrapper(t(580, 159, 4), GuestDriverSource::Defaulted);
    let r = w.on_fn1(&fn1("580.159.03").payload);
    let msg = w.describe(&r).expect("said");
    assert!(
        msg.starts_with("kf-rm: the guest says it is driver 580.159.03; no table to re-select: ")
            && msg.contains("never measured")
            && !msg.contains("guest-driver="),
        "{msg}"
    );
    // a guest that is the version already served says nothing
    let (mut w, _) = wrapper(t(580, 159, 4), GuestDriverSource::Defaulted);
    let r = w.on_fn1(&fn1("580.159.04").payload);
    assert_eq!(w.describe(&r), None);
}

/// A declared device keeps its refusal text even for a cross-branch guest: the pin advice belongs
/// to the DEFAULTED case only (a declared pin is already there, and is the thing that is wrong).
#[test]
fn a_declared_cross_branch_mismatch_gets_no_defaulted_advice() {
    let (mut w, _) = wrapper(t(595, 84, 0), GuestDriverSource::Declared);
    let r = w.on_fn1(&fn1("580.159.04").payload);
    assert!(matches!(r, Reselection::RefusedDeclared { .. }), "{r:?}");
    let msg = w.describe(&r).expect("said");
    assert!(msg.contains("DECLARED 595.84"), "{msg}");
    assert!(!msg.contains("add guest-driver"), "{msg}");
}

/// "What the device defaulted to" is the realize-time version, even after an earlier fn 1 moved
/// the chain to another release of the same branch: the surface check is made against it.
#[test]
fn the_defaulted_version_in_the_message_is_the_realize_time_one() {
    let (mut w, _) = wrapper(t(580, 159, 4), GuestDriverSource::Defaulted);
    assert!(matches!(
        w.on_fn1(&fn1("580.105.08").payload),
        Reselection::Reselected { .. }
    ));
    assert_eq!(w.current(), v(580, 105, 8));
    let outcome = w.on_fn1(&fn1("595.84").payload);
    let msg = w.describe(&outcome).expect("said");
    assert!(
        msg.contains("cannot re-select from 580.159.04 (the version the device defaulted to)"),
        "{msg}"
    );
    assert!(msg.contains("add guest-driver=595.84"), "{msg}");
}
