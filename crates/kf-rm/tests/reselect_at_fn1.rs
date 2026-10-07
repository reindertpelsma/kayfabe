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
}
