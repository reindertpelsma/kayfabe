//! Restricted user-stream snapshot model; no mapping or VM adapter.
use kf_abi::submit::{fifo, method_header_inc};
use kf_chan::user_stream::{Binding, Commands, Op, Refusal, State, decode};

fn packet(sub: u32, method: u32, args: &[u32]) -> Vec<u32> {
    let mut out = vec![method_header_inc(sub, method, args.len() as u32).unwrap()];
    out.extend_from_slice(args);
    out
}
fn object(value: u32) -> Option<Binding> {
    match value {
        0xc6b5 => Some(Binding::Hardware { class: 0xc6b5 }),
        0x5080 => Some(Binding::Software { value }),
        _ => None,
    }
}
fn parse(words: &[u32], state: &mut State) -> Result<Vec<Op>, Refusal> {
    decode(&Commands::copy_from(words, 128)?, state, object)
}

#[test]
fn software_calls_are_split_by_binding_not_by_payload_pattern() {
    let mut words = packet(4, 0, &[0xc6b5]);
    words.extend(packet(4, 0x200, &[0x4000002c]));
    words.extend(packet(6, 0, &[0x5080]));
    words.extend(packet(6, 0x200, &[0x4000002c]));
    let ops = parse(&words, &mut State::default()).unwrap();
    assert_eq!(ops.len(), 3); // Software binding does not reach hardware.
    assert_eq!(
        ops[1],
        Op::Hardware {
            sub: 4,
            method: 0x200,
            data: 0x4000002c
        }
    );
    assert!(
        matches!(ops[2], Op::Software(call) if call.sub == 6 && call.value == 0x5080 && call.method == 0x200 && call.data == 0x4000002c)
    );
}

#[test]
fn copied_commands_do_not_follow_later_source_mutations() {
    let mut words = packet(6, 0, &[0x5080]);
    words.extend(packet(6, 0x200, &[27]));
    let snapshot = Commands::copy_from(&words, 128).unwrap();
    words.fill(0);
    let ops = decode(&snapshot, &mut State::default(), object).unwrap();
    assert!(matches!(ops.as_slice(), [Op::Software(call)] if call.data == 27));
}

#[test]
fn bindings_persist_but_unknown_objects_and_unbound_methods_are_refused() {
    let mut state = State::default();
    parse(&packet(6, 0, &[0x5080]), &mut state).unwrap();
    assert!(matches!(
        parse(&packet(6, 0x200, &[19]), &mut state)
            .unwrap()
            .as_slice(),
        [Op::Software(_)]
    ));
    assert!(matches!(
        parse(&packet(5, 0x200, &[19]), &mut state),
        Err(Refusal::Unbound { .. })
    ));
    let before = state.clone();
    assert!(matches!(
        parse(&packet(6, 0, &[0x9999]), &mut state),
        Err(Refusal::Object { .. })
    ));
    assert_eq!(state, before);
}

#[test]
fn guest_reference_writes_never_become_hardware_receipts() {
    let mut words = packet(0, fifo::SET_REFERENCE_C56F, &[11]);
    // Non-incrementing repeated references.
    words.extend([(3 << 29) | (2 << 16) | (0x50 / 4), 12, 13]);
    // Immediate reference on another subchannel.
    words.push((4 << 29) | (14 << 16) | (3 << 13) | (0x50 / 4));
    assert_eq!(
        parse(&words, &mut State::default()).unwrap(),
        vec![
            Op::GuestReference(11),
            Op::GuestReference(12),
            Op::GuestReference(13),
            Op::GuestReference(14),
        ]
    );
}

#[test]
fn malformed_suffix_does_not_commit_an_earlier_binding() {
    let mut words = packet(6, 0, &[0x5080]);
    words.extend([method_header_inc(6, 0x200, 2).unwrap(), 1]);
    let mut state = State::default();
    assert!(matches!(
        parse(&words, &mut state),
        Err(Refusal::Truncated { .. })
    ));
    assert_eq!(state, State::default());
}

#[test]
fn unsupported_control_headers_and_method_overflow_fail_closed() {
    for word in [6 << 29, 7 << 29, 2 << 29, 1 << 16, (1 << 29) | (1 << 12)] {
        assert!(parse(&[word], &mut State::default()).is_err(), "{word:x}");
    }
    let mut words = packet(4, 0, &[0xc6b5]);
    words.extend(packet(4, 0x3ffc, &[1, 2]));
    assert!(matches!(
        parse(&words, &mut State::default()),
        Err(Refusal::Method { .. })
    ));
}

#[test]
fn snapshot_and_plan_stay_within_trusted_budget() {
    assert_eq!(
        Commands::copy_from(&[0; 9], 8).unwrap_err(),
        Refusal::Budget
    );
    let ops = parse(&[0; 128], &mut State::default()).unwrap();
    assert!(ops.is_empty());
    assert!(matches!(
        parse(&packet(0, 4, &[1]), &mut State::default()),
        Err(Refusal::Method { .. })
    ));
}

#[test]
fn gp_entry_flags_cannot_escape_copying_or_discard_wait_ordering() {
    use kf_abi::submit::gp_entry;
    use kf_chan::user_stream::{Source, inspect_entry};
    let raw = gp_entry(0x200000000, 64).unwrap();
    assert!(
        matches!(inspect_entry(raw, 64), Ok(Source::Segment(e)) if e.gpu_va == 0x200000000 && !e.sync_wait)
    );
    assert!(matches!(inspect_entry(raw | (1 << 63), 64), Ok(Source::Segment(e)) if e.sync_wait));
    for flag in [1 << 0, 1 << 1, 1 << 40, 1 << 41] {
        assert_eq!(inspect_entry(raw | flag, 64), Err(Refusal::Entry));
    }
    assert_eq!(inspect_entry(0, 0), Ok(Source::Nop));
    // CRC/extended-base/unknown controls do not become empty segment successes.
    for opcode in 1..=255u64 {
        assert_eq!(inspect_entry(opcode << 32, 4096), Err(Refusal::Entry));
    }
}

#[test]
fn gp_source_full_span_and_trusted_budget_are_checked_before_copy() {
    use kf_abi::submit::gp_entry;
    use kf_chan::user_stream::inspect_entry;
    assert_eq!(
        inspect_entry(gp_entry(0x200000000, 68).unwrap(), 64),
        Err(Refusal::Budget)
    );
    assert_eq!(
        inspect_entry(gp_entry((1 << 40) - 4, 8).unwrap(), 64),
        Err(Refusal::Entry)
    );
    assert!(inspect_entry(gp_entry((1 << 40) - 4, 4).unwrap(), 64).is_ok());
}
