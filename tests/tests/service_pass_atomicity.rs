//! ★★★ **GSP-D1 and GSP-D2 — what a service pass owes the guest when it cannot finish.**
//!
//! `GspFsm::service_command_queue` drains the command ring, answers each message, and
//! publishes how far it got. Two orderings inside that had to be wrong before anything went
//! visibly wrong, and both were invisible on this bench for the same reason: **the 580 guest
//! is synchronous under the GPU lock**, so it has one command in flight and cannot fill our
//! status ring against itself. That is a property of one guest, not of the protocol.
//!
//! | | what it was | what the guest saw |
//! |---|---|---|
//! | **GSP-D1** | the read pointer and expected sequence were committed **before** `answer` | `post` can return `QueueFull` — *retryable back-pressure*, by its own doc — and on this path there was no retry. The command was already consumed, so no reply was ever posted; the guest blocked in `_issueRpcAndWait` for the whole RPC timeout. |
//! | **GSP-D2** | the consumption acknowledgement was written **after** the drain's `?`s | a pass that consumed three commands and faulted on the fourth left our published `readPtr` at the previous pass's value, so the guest's `msgqTxGetFreeSpace` saw less room than existed. Self-healing on the next clean pass; a *persistently* failing pass reproduces the C's measured *"buffer is full"* (`C:3352-3358`). |
//!
//! ## ★★ The fix is an ORDERING, and the mirror failure has to be impossible
//!
//! D1 is answered by **answer-then-commit**: the cursor *is* the record of what has been
//! answered, so a refusal leaves the message still owed and the next doorbell re-reads it.
//! The alternative — hold the reply and re-post next pass — needs a queue of deferred
//! replies, and a deferred reply is state that can be lost, reordered or double-sent.
//!
//! Answer-then-commit has its own failure mode and it is the mirror image:
//! **double-service**, one command answered twice because a retry re-reads it after a
//! partially-effective post. Two replies at one `rpc.sequence` desynchronise the guest
//! exactly as badly as none. That is made *impossible* rather than unlikely by the shape of
//! `post`, and this file is what fails if that shape changes:
//!
//! - the flow-control test returns **before any write**;
//! - the elements are written first and `stat_write_ptr` advanced **last**;
//! - `stat_seq` and the cached free count advance only after the pointer.
//!
//! ⇒ a post either becomes visible to the guest in full, or not at all.

use kayfabe_gsp::{EchoOk, GspFault, OutgoingRpc, QueueState};
use kayfabe_tests::gspworld::{GspWorld, MODEL_A, P580};

type World = GspWorld;

const POST_EVENT: u32 = 0x1003;
const FN_RM_CONTROL: u32 = 76;

/// Post one single-element event, or say why not.
fn post(w: &mut World, sequence: u32) -> Result<(), GspFault> {
    let rpc = OutgoingRpc {
        function: POST_EVENT,
        sequence,
        rpc_result: 0,
        rpc_result_private: 0,
        payload: vec![0; 16],
    };
    w.fsm.post(&mut w.ram, &rpc)
}

/// Fill our status ring to within `keep` elements of full, so the next service pass runs
/// out of room in a controlled place. Returns how many are outstanding.
///
/// ★ The capacity is **measured**, not computed: fill the ring until it refuses, count,
/// drain it through the guest, then re-fill to `capacity - keep`. Deriving it from
/// `msgCount - 1` would put a second copy of `msgqTxGetFreeSpace`'s arithmetic in the test,
/// and a test that shares an off-by-one with the code under test cannot see it. The
/// measurement also *is* the non-vacuity check: a ring that never refused would make every
/// assertion below empty.
fn fill_status_ring(w: &mut World, keep: u32) -> u32 {
    let mut capacity = 0;
    while post(w, 9000 + capacity).is_ok() {
        capacity += 1;
        assert!(capacity < 1000, "the status ring never filled");
    }
    assert!(capacity > keep, "the ring is too small to leave {keep} free");
    let drained = w.guest.recv(&mut w.ram).expect("a clean stream");
    assert_eq!(drained.len(), capacity as usize, "the fill was observable");

    let n = capacity - keep;
    for i in 0..n {
        post(w, 1000 + i).expect("space was just measured");
    }
    n
}

/// `(read_ptr, seq)` — the command cursor, which is the whole of what a pass may commit.
fn cursor(w: &World) -> (u32, u32) {
    match w.fsm.queue() {
        QueueState::Bound(b) => (b.command_cursor().read_ptr, w.fsm.command_seq()),
        QueueState::Unbound => panic!("bind first"),
    }
}

#[test]
fn a_command_whose_reply_cannot_be_posted_is_not_consumed() {
    // ★★★ GSP-D1, stated as the guest experiences it: the command must still be there
    // afterwards, because a command that has been consumed will never be answered.
    let mut w = World::new(P580, MODEL_A);
    w.boot();
    w.link_and_drain();

    let filled = fill_status_ring(&mut w, 0);
    assert!(filled > 0, "non-vacuity: the ring was actually filled");

    let before = cursor(&w);
    w.guest
        .send(&mut w.ram, FN_RM_CONTROL, 7, &[0xAB; 32])
        .expect("the guest has command-ring room");
    let err = w
        .fsm
        .service_command_queue(&mut w.ram, &mut EchoOk)
        .expect_err("the status ring is full, so the reply cannot be posted");
    assert!(
        matches!(err, GspFault::QueueFull { .. }),
        "the refusal must be the retryable one, not something that lost the command: {err:?}"
    );
    assert_eq!(
        cursor(&w),
        before,
        "GSP-D1: the cursor moved past a command we never answered"
    );

    // ★ And the retry actually works, which is the other half of the claim: a refusal that
    // wedges the queue would satisfy the assertion above and still hang the guest. Drain
    // the status ring, ring the doorbell again, and the SAME command is answered.
    let drained = w.guest.recv(&mut w.ram).expect("a clean stream");
    assert_eq!(drained.len(), filled as usize);
    let report = w
        .fsm
        .service_command_queue(&mut w.ram, &mut EchoOk)
        .expect("with room, the pass completes");
    assert_eq!(report.commands.len(), 1);
    assert_eq!(report.commands[0].sequence, 7);
    assert_ne!(cursor(&w), before, "and NOW it is consumed");

    // ⊘ Exactly ONE reply — the mirror failure. A retry that re-posted an
    // already-published reply would show up here as two elements at sequence 7, which
    // desynchronises the guest as badly as none.
    let replies = w.guest.recv(&mut w.ram).expect("a clean stream");
    assert_eq!(replies.len(), 1, "double-service, not under-service");
    assert_eq!(replies[0].sequence, 7);
    assert_eq!(replies[0].function, FN_RM_CONTROL);
}

#[test]
fn a_failed_post_leaves_the_status_ring_byte_for_byte_untouched() {
    // ★★ The property answer-then-commit RESTS ON, asserted directly rather than reasoned
    // about: if a refused post could publish part of a message, a retry would publish it
    // twice. Everything the guest can see of the status queue must be unchanged.
    let mut w = World::new(P580, MODEL_A);
    w.boot();
    w.link_and_drain();
    fill_status_ring(&mut w, 0);

    let snapshot = |w: &World| match w.fsm.queue() {
        QueueState::Bound(b) => (b.status_cursor().write_ptr, b.status_cursor().free_cache),
        QueueState::Unbound => panic!("bind first"),
    };
    let before = snapshot(&w);
    let guest_free_before = w.guest.free_space(&mut w.ram);

    let err = post(&mut w, 4242).expect_err("the ring is full");
    assert!(matches!(err, GspFault::QueueFull { .. }));
    assert_eq!(snapshot(&w), before, "a refused post moved our own cursor");
    assert_eq!(
        w.guest.free_space(&mut w.ram),
        guest_free_before,
        "and the guest's view of the ring moved too"
    );

    // The guest sees exactly what was posted before the refusal — no fragment of 4242.
    let msgs = w.guest.recv(&mut w.ram).expect("a clean stream");
    assert!(
        msgs.iter().all(|m| m.sequence != 4242),
        "a refused post left a visible fragment"
    );
}

#[test]
fn a_pass_that_faults_part_way_still_publishes_what_it_consumed() {
    // ★★★ GSP-D2. Three commands, room for two replies: the pass answers two, refuses the
    // third, and must still tell the guest it consumed two command elements. Leaving the
    // published `readPtr` stale makes the guest compute less free space than exists, and a
    // pass that keeps failing never publishes again.
    let mut w = World::new(P580, MODEL_A);
    w.boot();
    w.link_and_drain();

    let filled = fill_status_ring(&mut w, 2);
    assert!(filled > 0, "non-vacuity: the ring was actually filled");

    let guest_free_before = w.guest.free_space(&mut w.ram);
    for seq in 0..3u32 {
        w.guest
            .send(&mut w.ram, FN_RM_CONTROL, seq, &[0x11; 16])
            .expect("the guest has command-ring room");
    }
    let err = w
        .fsm
        .service_command_queue(&mut w.ram, &mut EchoOk)
        .expect_err("the third reply has nowhere to go");
    assert!(matches!(err, GspFault::QueueFull { .. }), "{err:?}");

    // Two consumed, one still owed.
    assert_eq!(cursor(&w).1, 2, "two command sequences were committed");

    // ★ The assertion that matters, and it is made from the GUEST's side — the number our
    // FSM believes is not evidence that the guest can see it. `free_space` reads the
    // published acknowledgement through `msgqTxGetFreeSpace`'s own arithmetic.
    assert_eq!(
        w.guest.free_space(&mut w.ram),
        guest_free_before - 1,
        "GSP-D2: the guest must see two of its three command elements returned, so only \
         the un-answered one is still outstanding"
    );

    // And the pass is resumable: drain, re-service, and the third command is answered once.
    let _ = w.guest.recv(&mut w.ram).expect("a clean stream");
    let report = w
        .fsm
        .service_command_queue(&mut w.ram, &mut EchoOk)
        .expect("with room, the pass completes");
    assert_eq!(report.commands.len(), 1, "only the message still owed");
    assert_eq!(report.commands[0].sequence, 2);
}
