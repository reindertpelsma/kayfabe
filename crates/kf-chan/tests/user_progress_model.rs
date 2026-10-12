use kf_chan::tmode::SwCall;
use kf_chan::user_batch::{self, Action};
use kf_chan::user_consumption::ConsumedSource;
use kf_chan::user_progress::{Channel, Refusal, Work};
use kf_chan::user_stream::{self, Commands, Op, State};

fn source() -> ConsumedSource {
    ConsumedSource {
        generation: 91,
        guest_get: 7,
    }
}
fn software() -> Action {
    Action::Software(SwCall {
        sub: 6,
        value: 9,
        method: 0x200,
        data: 27,
    })
}
fn put(c: &mut Channel) -> Option<u32> {
    match c.next().unwrap() {
        Some(Work::Commands { publication, words }) => {
            assert!(!words.is_empty());
            Some(publication.put)
        }
        None => None,
        _ => panic!("expected a command cell"),
    }
}

#[test]
fn single_active_plan_blocks_cross_plan_software_and_requires_once_only_take() {
    let state = State::default();
    let commands = vec![Action::Commands(vec![0, 0])];
    let mut c = Channel::new(8, 1).unwrap();
    c.start(source(), vec![software()], &state).unwrap();
    let ticket = match c.next().unwrap().unwrap() {
        Work::Software { ticket, .. } => ticket,
        _ => panic!("software"),
    };
    assert!(matches!(
        c.start(source(), commands.clone(), &state),
        Err(Refusal::Order)
    ));
    assert!(matches!(
        c.start_noop(source(), &state),
        Err(Refusal::Order)
    ));
    assert!(c.next().unwrap().is_none());
    assert_eq!(c.source_progress(), None);
    c.software_done(ticket).unwrap();
    assert_eq!(c.source_progress(), Some(source()));
    assert!(
        c.start(source(), commands.clone(), &state).is_err(),
        "completed but not taken is still active"
    );
    assert!(c.take_source_progress(90).is_err());
    assert!(c.start(source(), commands.clone(), &state).is_err());
    assert_eq!(c.take_source_progress(91).unwrap(), source());
    assert!(c.take_source_progress(91).is_err());
    c.start(source(), commands, &state).unwrap();
    assert!(c.software_done(ticket).is_err(), "old plan ticket is stale");
    assert_eq!(put(&mut c), Some(3));
    assert_eq!(c.source_progress(), None);
    c.observe_get(1, 3).unwrap();
    assert_eq!(c.take_source_progress(91).unwrap(), source());
}

#[test]
fn header_only_empty_snapshot_is_not_a_noop_or_retired_source() {
    let mut state = State::default();
    let header =
        kf_abi::submit::method_header_inc(0, kf_abi::submit::fifo::SET_REFERENCE_C56F, 1).unwrap();
    let words = Commands::copy_from(&[header], 8).unwrap();
    let ops = user_stream::feed(&words, &mut state, |_| None).unwrap();
    assert!(ops.is_empty());
    assert!(state.has_pending());
    let mut c = Channel::new(8, 1).unwrap();
    assert!(matches!(
        c.start(source(), user_batch::plan(&ops).unwrap(), &state),
        Err(Refusal::Plan)
    ));
    assert!(matches!(c.start_noop(source(), &state), Err(Refusal::Plan)));
    assert_eq!(c.source_progress(), None);
    // Aggregate the contributing source pins through the complete continuation.
    let words = Commands::copy_from(&[42], 8).unwrap();
    let ops = user_stream::feed(&words, &mut state, |_| None).unwrap();
    assert!(!state.has_pending());
    c.start(source(), user_batch::plan(&ops).unwrap(), &state)
        .unwrap();
    assert_eq!(put(&mut c), Some(3));
    c.observe_get(1, 3).unwrap();
    assert_eq!(c.take_source_progress(91).unwrap(), source());
    // Explicit fully inspected no-op has no GPU work to wait for.
    assert!(c.start(source(), vec![], &state).is_err());
    c.start_noop(source(), &state).unwrap();
    assert_eq!(c.take_source_progress(91).unwrap(), source());
}

#[test]
fn backpressure_wrap_software_and_final_source_progress_share_one_ledger() {
    let mut ops = vec![Op::GuestReference(1); user_batch::WORDS + 1];
    ops.push(Op::Software(SwCall {
        sub: 6,
        value: 9,
        method: 0x200,
        data: 27,
    }));
    ops.push(Op::GuestReference(2));
    let mut c = Channel::new(8, 1).unwrap();
    c.start(source(), user_batch::plan(&ops).unwrap(), &State::default())
        .unwrap();
    assert_eq!(put(&mut c), Some(3));
    assert_eq!(put(&mut c), Some(6));
    assert_eq!(put(&mut c), None, "Full leaves the third cell unissued");
    assert!(c.observe_get(2, 3).is_err());
    assert!(c.observe_get(1, 7).is_err());
    c.observe_get(1, 3).unwrap();
    assert_eq!(
        put(&mut c),
        Some(1),
        "same third cell retries after real credit"
    );
    assert!(c.next().unwrap().is_none(), "SW awaits command receipts");
    c.observe_get(1, 1).unwrap();
    let ticket = match c.next().unwrap().unwrap() {
        Work::Software { ticket, .. } => ticket,
        _ => panic!("SW"),
    };
    assert!(c.next().unwrap().is_none());
    assert_eq!(c.source_progress(), None);
    c.software_done(ticket).unwrap();
    assert_eq!(put(&mut c), Some(4));
    assert_eq!(c.source_progress(), None);
    c.observe_get(1, 4).unwrap();
    assert_eq!(c.take_source_progress(91).unwrap(), source());
}
