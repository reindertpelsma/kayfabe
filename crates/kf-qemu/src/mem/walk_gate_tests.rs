//! Executable questions for a future per-space walk gate. Production policy is unchanged.
//!
//! The adapter deliberately calls the current VM-global predicate, just as PumpIo does.
//! Replace it with the scoped predicate only after scope/alias and wake rules are settled.

use super::{Inbox, SplitTarget};
use kf_mem::vasmgr::VasKey;

const A: VasKey = VasKey(0x100);
const B: VasKey = VasKey(0x200);

fn current_user_gate(inbox: &Inbox, _ring_space: VasKey) -> bool {
    inbox.walk_busy()
}

#[test]
fn a_space_split_holds_its_own_ring_until_successful_completion() {
    let inbox = Inbox::new().unwrap();
    let t = inbox.request_user_split(7, A).ok().unwrap();
    assert!(current_user_gate(&inbox, A));
    assert_eq!(
        inbox.take_split_requests(),
        vec![(t, SplitTarget::Space(A))]
    );
    assert!(current_user_gate(&inbox, A));
    inbox.finish_split(t, Ok(()));
    assert!(!current_user_gate(&inbox, A));
}

#[test]
fn an_all_pdb_split_holds_every_potentially_affected_space() {
    let inbox = Inbox::new().unwrap();
    let t = inbox.request_split(7, None);
    assert!(current_user_gate(&inbox, A));
    assert!(current_user_gate(&inbox, B));
    inbox.take_split_requests();
    assert!(current_user_gate(&inbox, A));
    assert!(current_user_gate(&inbox, B));
    inbox.finish_split(t, Ok(()));
    assert!(!current_user_gate(&inbox, A));
    assert!(!current_user_gate(&inbox, B));
}

#[test]
fn a_queued_walk_of_a_does_not_hold_an_unrelated_ring_in_b() {
    let inbox = Inbox::new().unwrap();
    inbox.request_user_split(7, A).ok().unwrap();
    assert!(current_user_gate(&inbox, A));
    assert!(
        !current_user_gate(&inbox, B),
        "B has no pending mapping change"
    );
}

#[test]
fn a_running_walk_of_a_does_not_hold_an_unrelated_ring_in_b() {
    let inbox = Inbox::new().unwrap();
    inbox.request_user_split(7, A).ok().unwrap();
    inbox.take_split_requests();
    assert!(current_user_gate(&inbox, A));
    assert!(
        !current_user_gate(&inbox, B),
        "taking a split must preserve its scope"
    );
}

#[test]
fn completing_a_releases_a_while_an_independent_walk_of_b_remains_pending() {
    let inbox = Inbox::new().unwrap();
    let a = inbox.request_user_split(7, A).ok().unwrap();
    inbox.request_user_split(9, B).ok().unwrap();
    inbox.take_split_requests();
    inbox.finish_split(a, Ok(()));
    assert!(current_user_gate(&inbox, B));
    assert!(!current_user_gate(&inbox, A), "B must not extend A's hold");
}

#[test]
fn a_scoped_retire_statement_does_not_hold_an_unrelated_ring() {
    let inbox = Inbox::new().unwrap();
    inbox.push(kf_rm::barpde::MemStatement::Retire {
        client: 0,
        vaspace: A.0 as u32,
    });
    assert!(!inbox.all_settled());
    assert!(current_user_gate(&inbox, A));
    assert!(
        !current_user_gate(&inbox, B),
        "unsettled is currently a VM-global gate"
    );
}
