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
    assert_eq!(
        inspect_entry(gp_entry((1 << 40) - 4, 4).unwrap(), 64),
        Err(Refusal::Entry)
    );
    assert!(inspect_entry(gp_entry((1 << 40) - 8, 4).unwrap(), 64).is_ok());
}

#[test]
fn every_fragmentation_preserves_bindings_software_calls_and_reference_filtering() {
    use kf_chan::user_stream::feed;
    let mut words = packet(4, 0, &[0xc6b5]);
    words.extend(packet(4, 0x200, &[0, 0x5080, 0x4000002c]));
    words.extend(packet(6, 0, &[0x5080]));
    words.extend(packet(6, 0x200, &[0, 0x4000002c]));
    words.extend([(3 << 29) | (3 << 16) | (0x50 / 4), 0, 12, 13]);
    let mut final_state = State::default();
    let expected = parse(&words, &mut final_state).unwrap();
    for split in 0..=words.len() {
        let mut state = State::default();
        let mut got = feed(
            &Commands::copy_from(&words[..split], 128).unwrap(),
            &mut state,
            object,
        )
        .unwrap();
        got.extend(
            feed(
                &Commands::copy_from(&words[split..], 128).unwrap(),
                &mut state,
                object,
            )
            .unwrap(),
        );
        assert_eq!(got, expected, "split={split}");
        assert_eq!(state, final_state, "split={split}");
        assert!(!state.has_pending());
        state.finish().unwrap();
    }
    let mut state = State::default();
    let mut got = Vec::new();
    for word in &words {
        got.extend(
            feed(
                &Commands::copy_from(&[*word], 1).unwrap(),
                &mut state,
                object,
            )
            .unwrap(),
        );
    }
    assert_eq!(got, expected);
    assert_eq!(state, final_state);
    state.finish().unwrap();
}

#[test]
fn pending_arguments_never_become_headers_and_failed_continuations_are_transactional() {
    use kf_chan::user_stream::feed;
    let mut state = State::default();
    // A split SET_OBJECT argument must still be channel-owned; a value that is
    // itself a valid immediate header cannot escape as a header on the next GP.
    feed(
        &Commands::copy_from(&[method_header_inc(6, 0, 1).unwrap()], 1).unwrap(),
        &mut state,
        object,
    )
    .unwrap();
    let before = state.clone();
    assert!(state.has_pending());
    assert!(matches!(state.finish(), Err(Refusal::Truncated { .. })));
    let raw_header = (4 << 29) | (7 << 16) | (0x50 / 4);
    assert_eq!(
        feed(
            &Commands::copy_from(&[raw_header], 1).unwrap(),
            &mut state,
            object
        ),
        Err(Refusal::Object { value: raw_header })
    );
    assert_eq!(state, before);
    // Retrying a rejected fragment doesn't consume the pending argument.
    assert_eq!(
        feed(
            &Commands::copy_from(&[0x5080], 1).unwrap(),
            &mut state,
            object
        )
        .unwrap(),
        []
    );
    assert!(!state.has_pending());
    assert!(
        matches!(feed(&Commands::copy_from(&packet(6, 0x200, &[31]), 128).unwrap(), &mut state, object).unwrap().as_slice(), [Op::Software(call)] if call.data == 31)
    );
}

#[test]
fn split_one_increment_and_declared_overflow_are_checked_before_publication() {
    use kf_chan::user_stream::feed;
    let mut state = State::default();
    parse(&packet(4, 0, &[0xc6b5]), &mut state).unwrap();
    let words = [(5 << 29) | (3 << 16) | (4 << 13) | (0x200 / 4), 1, 2, 3];
    let expected = parse(&words, &mut state.clone()).unwrap();
    let mut got = Vec::new();
    for word in words {
        got.extend(
            feed(
                &Commands::copy_from(&[word], 1).unwrap(),
                &mut state,
                object,
            )
            .unwrap(),
        );
    }
    assert_eq!(got, expected);
    assert!(matches!(
        got.as_slice(),
        [
            Op::Hardware { method: 0x200, .. },
            Op::Hardware { method: 0x204, .. },
            Op::Hardware { method: 0x204, .. }
        ]
    ));
    let before = state.clone();
    // Even with no argument yet available, reject an increment that would wrap
    // into reference/binding methods rather than deferring validation to later.
    assert_eq!(
        feed(
            &Commands::copy_from(&[method_header_inc(4, 0x3ffc, 2).unwrap()], 1).unwrap(),
            &mut state,
            object
        ),
        Err(Refusal::Method { method: 0x4000 })
    );
    assert_eq!(state, before);
}

#[test]
fn malformed_suffix_restores_an_already_consumed_pending_binding() {
    use kf_chan::user_stream::feed;
    let mut state = State::default();
    feed(
        &Commands::copy_from(&[method_header_inc(6, 0, 1).unwrap()], 1).unwrap(),
        &mut state,
        object,
    )
    .unwrap();
    let before = state.clone();
    // Valid pending binding followed by a forbidden control header: neither the
    // binding nor the consumed-argument progress may escape this failed fragment.
    assert!(matches!(
        feed(
            &Commands::copy_from(&[0x5080, 6 << 29], 2).unwrap(),
            &mut state,
            object
        ),
        Err(Refusal::Header { at: 1 })
    ));
    assert_eq!(state, before);
    feed(
        &Commands::copy_from(&[0x5080], 1).unwrap(),
        &mut state,
        object,
    )
    .unwrap();
    assert!(
        matches!(feed(&Commands::copy_from(&packet(6, 0x200, &[19]), 128).unwrap(), &mut state, object).unwrap().as_slice(), [Op::Software(call)] if call.data == 19)
    );
}

#[test]
fn split_reference_header_shaped_data_is_bookkeeping_only() {
    use kf_chan::user_stream::feed;
    let mut state = State::default();
    feed(
        &Commands::copy_from(
            &[method_header_inc(7, fifo::SET_REFERENCE_C56F, 1).unwrap()],
            1,
        )
        .unwrap(),
        &mut state,
        object,
    )
    .unwrap();
    let nested = (4 << 29) | (31 << 16) | (fifo::SET_REFERENCE_C56F / 4);
    assert_eq!(
        feed(
            &Commands::copy_from(&[nested], 1).unwrap(),
            &mut state,
            object
        )
        .unwrap(),
        [Op::GuestReference(nested)]
    );
    state.finish().unwrap();
}

#[test]
fn user_tlb_invalidate_becomes_an_own_space_software_action() {
    let words = [
        method_header_inc(0, fifo::MEM_OP_A, 4).unwrap(),
        0,
        0,
        0xffff_f020,
        (9 << 27) | 0x7ff_ffff,
    ];
    let commands = Commands::copy_from(&words, 32).unwrap();
    let ops = decode(&commands, &mut State::default(), |_| None).unwrap();
    assert_eq!(ops, [Op::OwnInvalidate]);
    assert_eq!(
        kf_chan::user_batch::plan(&ops).unwrap(),
        [kf_chan::user_batch::Action::OwnInvalidate]
    );
}

#[test]
fn mem_op_fields_persist_across_fragments_but_never_forward_a_pdb() {
    use kf_chan::user_stream::feed;
    let mut state = State::default();
    let a = Commands::copy_from(
        &[
            method_header_inc(0, fifo::MEM_OP_A, 4).unwrap(),
            1 << 11,
            0,
            0x12345020,
        ],
        32,
    )
    .unwrap();
    assert!(feed(&a, &mut state, |_| None).unwrap().is_empty());
    let d = Commands::copy_from(&[(0xa << 27) | 7], 32).unwrap();
    let ops = feed(&d, &mut state, |_| None).unwrap();
    assert_eq!(ops.first(), Some(&Op::OwnInvalidate));
    assert_eq!(
        ops.len(),
        5,
        "own-space completion precedes four authored sysmembar methods"
    );
    assert!(ops[1..].iter().all(|op| matches!(op, Op::Hardware { .. })));
    assert!(!state.has_pending());
}

#[test]
fn unsupported_replay_ack_scope_and_reserved_pdb_shapes_are_refused() {
    let base = [0, 0, 0x20, 9 << 27];
    let mut cases = Vec::new();
    for replay in 1..=7 {
        let mut x = base;
        x[2] |= replay << 2;
        cases.push(x);
    }
    for ack in [0, 0x40, 0x60] {
        let mut x = base;
        x[2] = ack;
        cases.push(x);
    }
    for c in [0x22, 0x420, 0x1021, 0x121] {
        let mut x = base;
        x[2] = c;
        cases.push(x);
    }
    for a in [1, 0x40, 0x80, 0xc0, 0x1000] {
        let mut x = base;
        x[0] = a;
        cases.push(x);
    }
    let mut b = base;
    b[1] = 1;
    cases.push(b);
    let mut targeted_all = base;
    targeted_all[2] |= 1;
    targeted_all[3] = 0xa << 27;
    cases.push(targeted_all);
    for values in cases {
        let commands = Commands::copy_from(&packet(0, fifo::MEM_OP_A, &values), 32).unwrap();
        let mut state = State::default();
        let before = state.clone();
        assert!(
            decode(&commands, &mut state, |_| None).is_err(),
            "{values:x?}"
        );
        assert_eq!(state, before);
    }
}

#[test]
fn mem_op_expansion_ordinary_tail_and_bad_suffix_are_transactional() {
    use kf_chan::user_stream::feed;
    let mut state = State::default();
    feed(
        &Commands::copy_from(&packet(0, fifo::MEM_OP_A, &[0, 0, 0]), 32).unwrap(),
        &mut state,
        |_| None,
    )
    .unwrap();
    let saved = state.clone();
    let mut words = packet(0, fifo::MEM_OP_A + 12, &[5 << 27]);
    for i in 0..100 {
        // Public Immediate form, one input word per output operation.
        words.push((4 << 29) | (i << 16) | (fifo::SET_REFERENCE_C56F / 4));
    }
    let out = feed(
        &Commands::copy_from(&words, 256).unwrap(),
        &mut state,
        |_| None,
    )
    .unwrap();
    assert_eq!(out.len(), 104);
    assert!(
        out.len() > words.len(),
        "the old input-sized output reserve was insufficient"
    );
    words.push(1 << 12); // reserved header bit, after valid expanded work
    let mut state = saved.clone();
    assert!(
        feed(
            &Commands::copy_from(&words, 256).unwrap(),
            &mut state,
            |_| None
        )
        .is_err()
    );
    assert_eq!(state, saved);
}
