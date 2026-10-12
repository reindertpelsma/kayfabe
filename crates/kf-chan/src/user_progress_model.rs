use crate::tmode::SwCall;
use crate::user_batch::{self, Action};
use crate::user_consumption::ConsumedSource;
use crate::user_progress::{Commands, Ordered};
use crate::user_stream::Op;

fn source() -> ConsumedSource {
    ConsumedSource {
        generation: 91,
        guest_get: 7,
    }
}
fn actions() -> Vec<Action> {
    let mut ops = vec![Op::GuestReference(1); user_batch::WORDS + 1];
    ops.push(Op::Software(SwCall {
        sub: 6,
        value: 0x5080,
        method: 0x200,
        data: 27,
    }));
    ops.push(Op::GuestReference(2));
    user_batch::plan(&ops).unwrap()
}

#[test]
fn multiple_cells_and_software_never_publish_early_source_progress() {
    let mut plan = Ordered::new(1, 10, source(), actions()).unwrap();
    let mut native = Commands::new(16, 1).unwrap();
    for put in [3, 6, 9] {
        let step = plan.issue_next().unwrap();
        assert!(matches!(step.action, Action::Commands(_)));
        assert_eq!(native.publish(step.ticket).unwrap().put, put);
    }
    assert!(
        plan.issue_next().is_none(),
        "software waits for preceding command cells"
    );
    for get in [3, 6, 9] {
        let receipt = native.observe_get(1, get).unwrap();
        assert_eq!(receipt.len(), 1);
        plan.command_consumed(receipt[0]).unwrap();
        assert_eq!(plan.source_progress(), None);
        if get != 9 {
            assert!(plan.issue_next().is_none());
        }
    }
    let software = plan.issue_next().unwrap();
    assert!(matches!(software.action, Action::Software(_)));
    let ticket = software.ticket;
    assert!(
        native.publish(ticket).is_err(),
        "software never becomes GPU work"
    );
    assert!(
        plan.issue_next().is_none(),
        "later commands wait for software outcome"
    );
    plan.software_done(ticket).unwrap();
    assert_eq!(
        plan.source_progress(),
        None,
        "one later command cell remains"
    );
    let last = plan.issue_next().unwrap();
    assert_eq!(native.publish(last.ticket).unwrap().put, 12);
    assert_eq!(plan.source_progress(), None);
    plan.command_consumed(native.observe_get(1, 12).unwrap()[0])
        .unwrap();
    assert_eq!(plan.source_progress(), Some(source()));
}

#[test]
fn wrong_order_replay_and_native_lifetime_cannot_advance_a_source() {
    let commands = user_batch::plan(&vec![Op::GuestReference(1); user_batch::WORDS]).unwrap();
    let mut plan = Ordered::new(1, 10, source(), commands.clone()).unwrap();
    let mut native = Commands::new(16, 1).unwrap();
    let first = plan.issue_next().unwrap().ticket;
    native.publish(first).unwrap();
    assert!(
        native.publish(first).is_err(),
        "duplicate publication is rejected"
    );
    native.publish(plan.issue_next().unwrap().ticket).unwrap();
    let receipts = native.observe_get(1, 6).unwrap();
    assert!(plan.command_consumed(receipts[1]).is_err());
    assert_eq!(plan.source_progress(), None);
    plan.command_consumed(receipts[0]).unwrap();
    assert!(plan.command_consumed(receipts[0]).is_err());
    plan.command_consumed(receipts[1]).unwrap();
    assert_eq!(plan.source_progress(), Some(source()));
    let mut replaced = Ordered::new(1, 11, source(), commands.clone()).unwrap();
    replaced.issue_next().unwrap();
    assert!(replaced.command_consumed(receipts[0]).is_err());
    let mut other_native = Ordered::new(2, 10, source(), commands).unwrap();
    assert!(
        native
            .publish(other_native.issue_next().unwrap().ticket)
            .is_err()
    );
    assert_eq!(other_native.source_progress(), None);
}

#[test]
fn backpressure_preserves_the_exact_pending_ticket_without_cancellation() {
    let mut plan = Ordered::new(1, 10, source(), actions()).unwrap();
    let mut native = Commands::new(8, 1).unwrap();
    native.publish(plan.issue_next().unwrap().ticket).unwrap();
    native.publish(plan.issue_next().unwrap().ticket).unwrap();
    let pending = plan.issue_next().unwrap().ticket;
    assert!(native.publish(pending).is_err());
    assert!(plan.issue_next().is_none());
    assert_eq!(plan.source_progress(), None);
    assert!(native.observe_get(1, 7).is_err());
    for receipt in native.observe_get(1, 3).unwrap() {
        plan.command_consumed(receipt).unwrap();
    }
    assert_eq!(native.publish(pending).unwrap().put, 1);
    assert_eq!(plan.source_progress(), None);
}

#[test]
fn a_new_plan_cannot_replace_pending_native_cells_and_wrap_remains_ordered() {
    let words = vec![Action::Commands(vec![0, 0]); 3];
    let mut first = Ordered::new(1, 10, source(), words).unwrap();
    let mut native = Commands::new(8, 1).unwrap();
    for put in [3, 6] {
        assert_eq!(
            native
                .publish(first.issue_next().unwrap().ticket)
                .unwrap()
                .put,
            put
        );
    }
    let mut next = Ordered::new(1, 11, source(), vec![Action::Commands(vec![0, 0])]).unwrap();
    let next_ticket = next.issue_next().unwrap().ticket;
    assert!(native.publish(next_ticket).is_err());
    for receipt in native.observe_get(1, 6).unwrap() {
        first.command_consumed(receipt).unwrap();
    }
    let third = first.issue_next().unwrap().ticket;
    assert_eq!(native.publish(third).unwrap().put, 1);
    assert!(native.publish(next_ticket).is_err());
    assert!(native.observe_get(2, 1).is_err());
    assert_eq!(first.source_progress(), None);
    first
        .command_consumed(native.observe_get(1, 1).unwrap()[0])
        .unwrap();
    assert_eq!(first.source_progress(), Some(source()));
    assert_eq!(native.publish(next_ticket).unwrap().put, 4);
    next.command_consumed(native.observe_get(1, 4).unwrap()[0])
        .unwrap();
    assert_eq!(next.source_progress(), Some(source()));
    assert!(
        native.publish(third).is_err(),
        "old plan cannot return after replacement"
    );
}

#[test]
fn software_only_plans_require_the_actual_ordered_action_result() {
    let software = Action::Software(SwCall {
        sub: 6,
        value: 9,
        method: 0x200,
        data: 27,
    });
    let mut first =
        Ordered::new(1, 10, source(), vec![software.clone(), software.clone()]).unwrap();
    let mut other = Ordered::new(1, 11, source(), vec![software]).unwrap();
    let wrong = other.issue_next().unwrap().ticket;
    let ticket = first.issue_next().unwrap().ticket;
    assert!(first.issue_next().is_none());
    assert!(first.software_done(wrong).is_err());
    assert_eq!(first.source_progress(), None);
    first.software_done(ticket).unwrap();
    assert!(first.software_done(ticket).is_err());
    let second = first.issue_next().unwrap().ticket;
    assert_eq!(first.source_progress(), None);
    first.software_done(second).unwrap();
    assert_eq!(first.source_progress(), Some(source()));
}

#[test]
fn invalid_host_plan_identity_and_cell_sizes_refuse_before_any_progress() {
    use crate::user_progress::Refusal;
    for (native, plan, generation) in [(0, 1, 1), (1, 0, 1), (1, 1, 0)] {
        let result = Ordered::new(
            native,
            plan,
            ConsumedSource {
                generation,
                guest_get: 0,
            },
            vec![],
        );
        assert!(matches!(result, Err(Refusal::Lifetime)));
    }
    for words in [vec![], vec![0], vec![0; user_batch::WORDS + 2]] {
        assert!(matches!(
            Ordered::new(1, 1, source(), vec![Action::Commands(words)]),
            Err(Refusal::Plan)
        ));
    }
    assert!(matches!(
        Ordered::new(
            1,
            1,
            source(),
            vec![Action::Commands(vec![0, 0]); user_batch::MAX_OPS + 1]
        ),
        Err(Refusal::Plan)
    ));
    // A bounded parser can consume only canonical zero NOPs, with no GPU work
    // or software action in the plan. Source backing/epoch validation remains
    // mandatory at the adapter even for this immediate CPU parsing result.
    let empty = Ordered::new(1, 1, source(), vec![]).unwrap();
    assert_eq!(empty.source_progress(), Some(source()));
}

#[test]
fn coordinator_plan_identity_exhaustion_never_restarts_sequence() {
    use crate::user_progress::{Channel, Refusal};
    let mut channel = Channel::new(8, 1).unwrap();
    channel.next_plan = u64::MAX - 1;
    channel
        .start_noop(source(), &crate::user_stream::State::default())
        .unwrap();
    assert_eq!(channel.take_source_progress(91).unwrap(), source());
    assert!(matches!(
        channel.start_noop(source(), &crate::user_stream::State::default()),
        Err(Refusal::Lifetime)
    ));
    assert_eq!(channel.next_plan, u64::MAX);
    assert_eq!(channel.source_progress(), None);
}

#[test]
fn native_pre_io_refusal_never_skips_an_unpublished_action() {
    use crate::user_progress::{Channel, Refusal, Work};
    let mut channel = Channel::new(8, 1).unwrap();
    channel
        .start(
            source(),
            vec![Action::Commands(vec![0, 0])],
            &crate::user_stream::State::default(),
        )
        .unwrap();
    // Inject a private-ledger consistency refusal at the same pre-I/O boundary
    // as Full/Sequence. No issued action may be skipped on any publish error.
    channel.commands.last = Some((2, 0));
    assert!(matches!(channel.next(), Err(Refusal::Order)));
    assert_eq!(channel.active.as_ref().unwrap().issued, 0);
    assert_eq!(channel.source_progress(), None);
    channel.commands.last = None;
    assert!(matches!(
        channel.next().unwrap(),
        Some(Work::Commands { .. })
    ));
    channel.observe_get(1, 3).unwrap();
    assert_eq!(channel.take_source_progress(91).unwrap(), source());
}
