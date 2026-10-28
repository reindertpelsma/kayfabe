//! ENS12 bab3c088 Inbox cancellation/idempotence regressions, GPU-free.
//! These tests deliberately expose RED behavior; no product fix is included.
use kf_chan::host::Gate;
use kf_mem::vasmgr::VasKey;
use kf_qemu::mem::Inbox;

fn submitted(gated: bool) -> (Inbox, u64, Option<Gate>) {
    let inbox = Inbox::new().expect("test eventfd");
    let gate = gated.then(|| Gate::scratch().expect("private test page"));
    let ticket = match &gate {
        Some(gate) => inbox.request_gated_split(7, VasKey(0x100), gate.clone(), 11),
        None => inbox.request_split(7, Some(0x1000)),
    };
    assert_eq!(inbox.take_split_requests().len(), 1);
    assert_eq!(inbox.split_result(ticket), None);
    (inbox, ticket, gate)
}

fn duplicate_is_not_a_second_completion(gated: bool) {
    let (inbox, ticket, gate) = submitted(gated);
    // Test-only caller supplies an apply acknowledgement, never a real GPU receipt.
    assert_eq!(inbox.finish_split(ticket, Ok(())), Some(7));
    assert_eq!(inbox.split_result(ticket), Some(Ok(())));
    assert_eq!(inbox.split_result(ticket), None);
    if let Some(gate) = gate {
        assert_eq!(gate.value(), Some(11));
    }
    assert_eq!(inbox.finish_split(ticket, Ok(())), None);
    assert_eq!(
        inbox.split_result(ticket), None,
        "a duplicate apply ACK must not recreate a consumed completion"
    );
}

fn canceled_is_not_resurrected(gated: bool) {
    let (inbox, ticket, gate) = submitted(gated);
    assert_eq!(inbox.reset_life(), 1);
    assert_eq!(inbox.split_result(ticket), None);
    assert_eq!(inbox.finish_split(ticket, Ok(())), None);
    if let Some(gate) = gate {
        assert_eq!(gate.value(), Some(0), "reset must drop its unreleased gate");
    }
    assert_eq!(
        inbox.split_result(ticket), None,
        "a late ACK from an ended life must not create a completion"
    );
}

// Falsifier: replaying a terminal ACK produces a fresh consumable result.
#[test]
fn split_duplicate_ack_has_no_second_completion() {
    duplicate_is_not_a_second_completion(false);
}
#[test]
fn gated_duplicate_ack_has_no_second_completion() {
    duplicate_is_not_a_second_completion(true);
}

// Falsifier: a late ACK produces a result after the request was canceled by reset.
#[test]
fn split_reset_rejects_late_completion() {
    canceled_is_not_resurrected(false);
}
#[test]
fn gated_reset_rejects_late_completion() {
    canceled_is_not_resurrected(true);
}
