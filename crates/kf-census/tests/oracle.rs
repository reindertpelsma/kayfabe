//! GPU-free tests with small synthetic fixtures: the oracle must catch a gap, a truncated
//! element, a reordered sequence, an unknown function id, a corrupted message and a loss
//! marker; a stream produced by kayfabe's own transport encoder must round-trip through the
//! capture format and the decoder byte for byte; the format must carry nothing board-specific.

mod common;

use common::{
    ES, Msg, TAG, abi, capture, control_payload, conversation, count, ctl_rep, ctl_req,
    encode, header, natural_seqs, observer_jsonl, run_capture, run_observer, runs_of, tx_header,
};
use kf_abi::DriverVersion;
use kf_census::analyze::{Analyzer, AnomalyKind};
use kf_census::capture::{
    Dir, FILE_FIELDS, FILE_HEADER_BYTES, FileHeader, Kind, RECORD_FIELDS, RECORD_HEADER_BYTES,
    Reader, Writer,
};
use kf_census::{analyzer_for, report, stream};
use std::cell::RefCell;
use std::rc::Rc;

// ---------------------------------------------------------------------------------------------
// valid streams

#[test]
fn valid_capture_passes_and_the_census_is_exact() {
    let msgs = conversation();
    let runs = runs_of(&msgs, &natural_seqs(&msgs));
    let r = run_capture(&capture(&runs, 4, true));
    assert!(r.oracle_ok(), "{:?}", r.anomalies);
    assert_eq!(r.messages, [4, 5]);
    let c = |cmd: u32| r.controls.iter().find(|c| c.cmd == cmd).unwrap();
    let c852a = c(0x2080_852a);
    assert_eq!((c852a.requests, c852a.replies, c852a.paired), (2, 2, 2));
    assert_eq!(c852a.req_sizes.get(&24), Some(&2));
    assert_eq!(c852a.rep_sizes.get(&1784), Some(&2));
    assert_eq!(c852a.distinct_replies, 1);
    assert_eq!(c852a.echo, 0);
    let ca02f = c(0x2080_a02f);
    assert_eq!(ca02f.statuses.get(&0x56), Some(&1));
    let c730128 = c(0x0073_0128);
    assert_eq!(c730128.echo, 1, "reply params equal request params");
    assert_eq!(r.unbalanced_controls(), 0);
    assert_eq!(r.rpc_sequence_nonzero, 0);
}

#[test]
fn valid_observer_stream_passes() {
    let msgs = conversation();
    let runs = runs_of(&msgs, &natural_seqs(&msgs));
    let r = run_observer(&observer_jsonl(&runs, None, None));
    assert!(r.oracle_ok(), "{:?}", r.anomalies);
    assert_eq!(r.messages, [4, 5]);
    assert_eq!(r.sources[0].records, 9);
}

#[test]
fn capture_and_observer_agree_on_the_census() {
    let msgs = conversation();
    let runs = runs_of(&msgs, &natural_seqs(&msgs));
    let a = run_capture(&capture(&runs, 2, true));
    let b = run_observer(&observer_jsonl(&runs, None, None));
    assert_eq!(report::controls_tsv(&a), report::controls_tsv(&b));
    assert_eq!(report::functions_tsv(&a), report::functions_tsv(&b));
}

// ---------------------------------------------------------------------------------------------
// the oracle catches what it must

#[test]
fn a_gap_is_reported_and_counted() {
    let msgs = conversation();
    // Cmd queue seqs 0,1,3,4 (2 missing); status stays natural.
    let mut seqs = natural_seqs(&msgs);
    for (m, s) in msgs.iter().zip(seqs.iter_mut()) {
        if m.dir == Dir::Cmd && *s >= 2 {
            *s += 1;
        }
    }
    let runs = runs_of(&msgs, &seqs);
    for r in [
        run_capture(&capture(&runs, 8, true)),
        run_observer(&observer_jsonl(&runs, None, None)),
    ] {
        assert!(!r.oracle_ok());
        assert_eq!(count(&r, AnomalyKind::SeqGap), 1, "{:?}", r.anomalies);
        assert_eq!(r.missing_messages, 1);
        assert_eq!(
            count(&r, AnomalyKind::ParseFailure),
            0,
            "gaps are not parse failures"
        );
    }
}

#[test]
fn a_missing_prefix_is_reported() {
    let msgs = conversation();
    let seqs: Vec<u32> = natural_seqs(&msgs).into_iter().map(|s| s + 5).collect();
    let r = run_observer(&observer_jsonl(&runs_of(&msgs, &seqs), None, None));
    assert_eq!(
        count(&r, AnomalyKind::PrefixMissing),
        2,
        "one per direction"
    );
    assert_eq!(r.missing_messages, 10);
}

#[test]
fn a_reordered_sequence_is_reported() {
    let msgs = conversation();
    let mut seqs = natural_seqs(&msgs);
    // Swap the 2nd and 3rd Cmd sequence numbers: 0, 2, 1, 3.
    let cmd_ix: Vec<usize> = (0..msgs.len())
        .filter(|&i| msgs[i].dir == Dir::Cmd)
        .collect();
    seqs.swap(cmd_ix[1], cmd_ix[2]);
    let runs = runs_of(&msgs, &seqs);
    for r in [
        run_capture(&capture(&runs, 8, true)),
        run_observer(&observer_jsonl(&runs, None, None)),
    ] {
        assert!(!r.oracle_ok());
        assert_eq!(
            count(&r, AnomalyKind::SeqGap),
            1,
            "2 arrives where 1 was due"
        );
        assert_eq!(
            count(&r, AnomalyKind::SeqRewind),
            1,
            "then 1 arrives after 2"
        );
    }
}

#[test]
fn a_duplicate_is_a_rewind() {
    let msgs = conversation();
    let mut seqs = natural_seqs(&msgs);
    let cmd_ix: Vec<usize> = (0..msgs.len())
        .filter(|&i| msgs[i].dir == Dir::Cmd)
        .collect();
    seqs[cmd_ix[2]] = seqs[cmd_ix[1]];
    let r = run_observer(&observer_jsonl(&runs_of(&msgs, &seqs), None, None));
    assert_eq!(count(&r, AnomalyKind::SeqRewind), 1);
}

#[test]
fn a_truncated_element_is_a_parse_failure_not_a_skip() {
    let msgs = conversation();
    let mut runs = runs_of(&msgs, &natural_seqs(&msgs));
    // Observer-style truncation: the message is cut mid-body but the line is self-consistent.
    let cut = (48usize + 32 + 20).next_multiple_of(8);
    runs[1].1.truncate(ES); // keep the run whole for the encoder-side helper ...
    let text = observer_jsonl(&runs, None, None);
    // ... then shorten the payload of record 2 inside the JSON (hex and payload_bytes together).
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let l = &lines[2];
    let hex_start = l.find("\"payload_hex\":\"").unwrap() + 15;
    let hex_end = l[hex_start..].find('"').unwrap() + hex_start;
    let shortened = format!(
        "{}{}{}",
        &l[..hex_start],
        &l[hex_start..hex_start + cut * 2],
        &l[hex_end..]
    )
    .replace(
        &format!("\"payload_bytes\":{}", (hex_end - hex_start) / 2),
        &format!("\"payload_bytes\":{cut}"),
    );
    lines[2] = shortened;
    let r = run_observer(&(lines.join("\n") + "\n"));
    assert!(
        count(&r, AnomalyKind::ParseFailure) >= 1,
        "{:?}",
        r.anomalies
    );
    assert!(!r.oracle_ok());
}

#[test]
fn a_capture_file_cut_inside_a_record_is_reported() {
    let msgs = conversation();
    let runs = runs_of(&msgs, &natural_seqs(&msgs));
    let mut bytes = capture(&runs, 8, true);
    bytes.truncate(bytes.len() - 5000); // lands inside an Elements payload or loses the End
    let r = run_capture(&bytes);
    assert!(!r.oracle_ok());
    assert!(
        count(&r, AnomalyKind::StreamTruncated) + count(&r, AnomalyKind::EndMissing) >= 1,
        "{:?}",
        r.anomalies
    );
}

#[test]
fn a_message_whose_last_element_never_arrives_is_trailing_bytes() {
    let a = abi();
    // A 2-element message (payload > one element) ...
    let big = Msg {
        dir: Dir::Cmd,
        function: a.rpc.codes.gsp_set_system_info,
        rpc_sequence: 0,
        rpc_result: 0xffff_ffff,
        payload: vec![3u8; 5000],
    };
    let run = encode(std::slice::from_ref(&big), &[0]).remove(0);
    assert_eq!(run.len(), 2 * ES);
    // ... of which the capture holds only the first element.
    let r = run_capture(&capture(&[(Dir::Cmd, run[..ES].to_vec())], 8, true));
    assert_eq!(
        count(&r, AnomalyKind::TrailingBytes),
        1,
        "{:?}",
        r.anomalies
    );
    assert_eq!(r.messages, [0, 0]);
}

#[test]
fn an_unknown_function_id_is_reported() {
    let mut msgs = conversation();
    msgs[2].function = 0x7777; // not an ogkm function at any tag
    let runs = runs_of(&msgs, &natural_seqs(&msgs));
    for r in [
        run_capture(&capture(&runs, 8, true)),
        run_observer(&observer_jsonl(&runs, None, None)),
    ] {
        assert_eq!(
            count(&r, AnomalyKind::UnknownFunction),
            1,
            "{:?}",
            r.anomalies
        );
        assert!(!r.oracle_ok());
        let row = r.functions.get(&(Dir::Cmd, 0x7777)).unwrap();
        assert_eq!(row.name, None);
    }
}

#[test]
fn a_corrupted_message_is_a_parse_failure() {
    let msgs = conversation();
    let mut runs = runs_of(&msgs, &natural_seqs(&msgs));
    runs[4].1[125] ^= 0x10; // inside the declared length (params of the 20-byte control): the checksum no longer folds to zero
    for r in [
        run_capture(&capture(&runs, 8, true)),
        run_observer(&observer_jsonl(&runs, None, None)),
    ] {
        assert_eq!(count(&r, AnomalyKind::ParseFailure), 1, "{:?}", r.anomalies);
    }
}

#[test]
fn observer_metadata_that_disagrees_with_the_payload_is_reported() {
    let msgs = conversation();
    let runs = runs_of(&msgs, &natural_seqs(&msgs));
    let r = run_observer(&observer_jsonl(&runs, Some(0x4d), None));
    assert_eq!(count(&r, AnomalyKind::MetaMismatch), 9);
}

#[test]
fn a_footer_that_disagrees_with_the_file_is_reported() {
    let msgs = conversation();
    let runs = runs_of(&msgs, &natural_seqs(&msgs));
    let r = run_observer(&observer_jsonl(&runs, None, Some(10)));
    assert_eq!(
        count(&r, AnomalyKind::FooterProblem),
        1,
        "{:?}",
        r.anomalies
    );
    // and a file cut before its footer
    let text = observer_jsonl(&runs, None, None);
    let cut: String = text.lines().take(5).map(|l| format!("{l}\n")).collect();
    let r = run_observer(&cut);
    assert_eq!(count(&r, AnomalyKind::FooterProblem), 1);
}

#[test]
fn an_explicit_loss_marker_is_counted_and_marks_the_next_gap_declared() {
    let msgs = conversation();
    let mut seqs = natural_seqs(&msgs);
    for (m, s) in msgs.iter().zip(seqs.iter_mut()) {
        if m.dir == Dir::Cmd && *s >= 2 {
            *s += 1; // one Cmd message lost
        }
    }
    let runs = runs_of(&msgs, &seqs);
    // Build by hand so a Loss record sits between Cmd chunks.
    let mut w = Writer::new(Vec::new(), &header()).unwrap();
    for d in [Dir::Cmd, Dir::Status] {
        w.bind(1, d, 1, &tx_header()).unwrap();
    }
    let cmd: Vec<&Vec<u8>> = runs
        .iter()
        .filter(|r| r.0 == Dir::Cmd)
        .map(|r| &r.1)
        .collect();
    let stat: Vec<&Vec<u8>> = runs
        .iter()
        .filter(|r| r.0 == Dir::Status)
        .map(|r| &r.1)
        .collect();
    w.elements(
        1,
        Dir::Cmd,
        2,
        0,
        2,
        &[cmd[0].as_slice(), cmd[1].as_slice()].concat(),
    )
    .unwrap();
    w.loss(Kind::LossOverflow, 1, Dir::Cmd, 3, 2, 1).unwrap();
    w.elements(
        1,
        Dir::Cmd,
        4,
        3,
        2,
        &[cmd[2].as_slice(), cmd[3].as_slice()].concat(),
    )
    .unwrap();
    let all: Vec<u8> = stat.iter().flat_map(|r| r.iter().copied()).collect();
    for (i, c) in all.chunks(ES).enumerate() {
        w.elements(1, Dir::Status, 5 + i as u64, i as u32, 1, c)
            .unwrap();
    }
    let r = run_capture(&w.end(99).unwrap());
    assert_eq!(count(&r, AnomalyKind::CaptureLoss), 1);
    assert_eq!(
        count(&r, AnomalyKind::SeqGap),
        1,
        "the oracle sees the loss independently"
    );
    assert_eq!(r.declared_loss_followups, 1, "and knows it was announced");
    assert_eq!(
        count(&r, AnomalyKind::DiscontinuousEntries),
        0,
        "the marker explains the jump"
    );
    assert!(!r.oracle_ok());
}

#[test]
fn an_unmarked_jump_in_ring_entries_is_reported() {
    let msgs = conversation();
    let runs = runs_of(&msgs, &natural_seqs(&msgs));
    let mut w = Writer::new(Vec::new(), &header()).unwrap();
    for d in [Dir::Cmd, Dir::Status] {
        w.bind(1, d, 1, &tx_header()).unwrap();
    }
    let cmd: Vec<&Vec<u8>> = runs
        .iter()
        .filter(|r| r.0 == Dir::Cmd)
        .map(|r| &r.1)
        .collect();
    w.elements(1, Dir::Cmd, 2, 0, 1, cmd[0]).unwrap();
    w.elements(1, Dir::Cmd, 3, 5, 1, cmd[1]).unwrap(); // jumps from 1 to 5, no loss record
    let r = run_capture(&w.end(9).unwrap());
    assert_eq!(count(&r, AnomalyKind::DiscontinuousEntries), 1);
    assert_eq!(count(&r, AnomalyKind::CounterGap), 0);
}

#[test]
fn record_counter_gaps_and_clock_regressions_are_reported() {
    let msgs = conversation();
    let runs = runs_of(&msgs, &natural_seqs(&msgs));
    let mut bytes = capture(&runs, 8, true);
    // Records start after the 128-byte header; the first record is a 32-byte header + 32-byte
    // Bind payload. Patch the second record's counter (offset +12 in its header) from 1 to 7.
    let second = FILE_HEADER_BYTES + RECORD_HEADER_BYTES + 32;
    bytes[second + 12..second + 16].copy_from_slice(&7u32.to_le_bytes());
    let r = run_capture(&bytes);
    assert!(count(&r, AnomalyKind::CounterGap) >= 1, "{:?}", r.anomalies);
}

#[test]
fn a_capture_without_end_is_reported() {
    let msgs = conversation();
    let runs = runs_of(&msgs, &natural_seqs(&msgs));
    let r = run_capture(&capture(&runs, 8, false));
    assert_eq!(count(&r, AnomalyKind::EndMissing), 1);
    assert_eq!(r.anomaly_total(), 1, "nothing else is wrong with it");
}

// ---------------------------------------------------------------------------------------------
// large controls (head + continuation records), joined with kf-gsp's own assembler

#[test]
fn a_large_control_is_joined_across_continuation_records() {
    let a = abi();
    let params = vec![0x5au8; 70_000];
    let whole = control_payload(0x2080_a095, 0, &params);
    let rpc_max = a.element_size_max as usize - a.element.hdr_size();
    let head_payload_len = rpc_max - 32;
    let cont = a.rpc.codes.continuation_record;
    let mut msgs = vec![Msg {
        dir: Dir::Cmd,
        function: a.rpc.codes.gsp_rm_control,
        rpc_sequence: 0,
        rpc_result: 0xffff_ffff,
        payload: whole[..head_payload_len].to_vec(),
    }];
    let mut rest = &whole[head_payload_len..];
    while !rest.is_empty() {
        let n = rest.len().min(rpc_max - 32);
        msgs.push(Msg {
            dir: Dir::Cmd,
            function: cont,
            rpc_sequence: 0,
            rpc_result: 0xffff_ffff,
            payload: rest[..n].to_vec(),
        });
        rest = &rest[n..];
    }
    let runs = runs_of(&msgs, &natural_seqs(&msgs));
    let r = run_capture(&capture(&runs, 16, true));
    assert!(r.oracle_ok(), "{:?}", r.anomalies);
    assert_eq!(r.large_joined, 1);
    let c = &r.controls[0];
    assert_eq!((c.cmd, c.requests), (0x2080_a095, 1));
    assert_eq!(c.req_sizes.get(&70_000), Some(&1));
}

// ---------------------------------------------------------------------------------------------
// round trip through the capture format, byte for byte

#[test]
fn transport_encoded_stream_round_trips_through_the_capture_format_byte_for_byte() {
    let mut msgs = conversation();
    // include multi-element messages (an 8 KiB and a 20 KiB payload) so that reassembly across
    // chunk and ring boundaries is exercised
    let a = abi();
    for (i, size) in [8000usize, 20_000, 100].into_iter().enumerate() {
        msgs.push(Msg {
            dir: if i == 1 { Dir::Status } else { Dir::Cmd },
            function: a.rpc.codes.set_registry,
            rpc_sequence: 0,
            rpc_result: 0,
            payload: (0..size).map(|j| (j * 7 + i) as u8).collect(),
        });
    }
    // enough messages to wrap the 63-entry ring more than once (100 single-element + 3 multi)
    for k in 0..100u32 {
        msgs.push(ctl_req(0x2080_0000 + k, &[k as u8; 300]));
    }
    let runs = runs_of(&msgs, &natural_seqs(&msgs));
    let file = capture(&runs, 3, true);

    // 1. the reader hands back exactly what the writer wrote (framing is lossless)
    let mut rd = Reader::new(file.as_slice()).unwrap();
    assert_eq!(*rd.header(), header());
    let mut kinds = Vec::new();
    while let Some(rec) = rd.next_record().unwrap() {
        assert_eq!(
            rec.header.counter as usize,
            kinds.len(),
            "counter is 0,1,2,..."
        );
        kinds.push(rec.header.kind);
    }
    assert_eq!(kinds.last(), Some(&Kind::End));

    // 2. the decoder reassembles the same message runs the transport encoded
    type Seen = Rc<RefCell<Vec<(Dir, Vec<u8>)>>>;
    let seen: Seen = Rc::default();
    let mut an = analyzer_for(TAG, None).unwrap();
    let sink = Rc::clone(&seen);
    an.set_tap(Box::new(move |d, b| {
        sink.borrow_mut().push((d, b.to_vec()))
    }));
    stream::read(&mut an, "roundtrip", file.as_slice()).unwrap();
    let report = an.finish();
    assert!(report.oracle_ok(), "{:?}", report.anomalies);
    for d in [Dir::Cmd, Dir::Status] {
        let sent: Vec<&Vec<u8>> = runs.iter().filter(|r| r.0 == d).map(|r| &r.1).collect();
        let got = seen.borrow();
        let got: Vec<&Vec<u8>> = got.iter().filter(|r| r.0 == d).map(|r| &r.1).collect();
        assert_eq!(sent.len(), got.len(), "{d:?}: message count");
        for (i, (s, g)) in sent.iter().zip(&got).enumerate() {
            assert_eq!(s, g, "{d:?} message {i} differs");
        }
    }
    assert_eq!(report.messages[0] + report.messages[1], msgs.len() as u64);

    // 3. and the decoded content equals what was sent (function, sequence, payload)
    for (m, run) in msgs.iter().zip(runs.iter().map(|r| &r.1)) {
        let len = kf_gsp::peek_len(&a.element, run, ES as u32, a.element_size_max).unwrap();
        let inc = kf_gsp::decode_message(
            &a.element,
            run,
            len,
            kf_gsp::element::peek_seq_num(&a.element, run).unwrap(),
            &a.driver,
        )
        .unwrap();
        assert_eq!(inc.envelope.function, m.function);
        assert_eq!(inc.payload, m.payload);
    }
}

// ---------------------------------------------------------------------------------------------
// nothing board-specific in the format, nothing payload-derived in the census

#[test]
fn the_format_has_no_board_specific_field() {
    // The field tables tile the headers exactly: there is no byte the tables do not name.
    for (fields, total) in [
        (FILE_FIELDS, FILE_HEADER_BYTES),
        (RECORD_FIELDS, RECORD_HEADER_BYTES),
    ] {
        let mut at = 0;
        for f in fields {
            assert_eq!(f.off, at, "gap or overlap before {}", f.name);
            at += f.len;
        }
        assert_eq!(at, total);
        for f in fields {
            for banned in [
                "uuid",
                "gid",
                "serial",
                "vbios",
                "rom",
                "mac",
                "bdf",
                "bus",
                "slot",
                "subsystem",
                "sub_",
                "board",
                "sku",
                "part",
                "eeprom",
                "address",
                "pa",
                "ip",
            ] {
                let n = f.name;
                let hit = n.split('_').any(|w| w == banned) || n == banned;
                assert!(!hit, "field {n} looks board-specific ({banned})");
            }
        }
    }
    // The only free-form text is two printable-ASCII strings; every other byte is a number
    // the header struct states, or reserved zero.
    let h = header().encode().unwrap();
    for f in FILE_FIELDS.iter().filter(|f| f.zero) {
        assert!(
            h[f.off..f.off + f.len].iter().all(|&b| b == 0),
            "{} not zero",
            f.name
        );
    }
    let back = FileHeader::decode(&h).unwrap();
    assert_eq!(back, header());
    // the strings reject anything that is not printable ASCII (no binary board data fits)
    let mut bad = header();
    bad.driver_version = "58\u{0}0".to_string();
    assert!(bad.encode().is_err());
    bad.driver_version = "x".repeat(32);
    assert!(bad.encode().is_err());
    // and a reserved byte that is set makes the file unreadable rather than ignored
    let mut dirty = h;
    dirty[37] = 1;
    assert!(FileHeader::decode(&dirty).is_err());
}

#[test]
fn the_census_outputs_carry_counts_and_sizes_but_no_message_bytes() {
    let mut msgs = conversation();
    // a reply body that looks like a UUID/serial: it must never reach the census files
    let secret = *b"BOARD-SERIAL-0123456789ABCDEF!!";
    msgs.push(ctl_req(0x2080_014a, &[0u8; 16]));
    msgs.push(ctl_rep(0x2080_014a, 0, &secret));
    let runs = runs_of(&msgs, &natural_seqs(&msgs));
    let r = run_capture(&capture(&runs, 8, true));
    assert!(r.oracle_ok());
    let hex: String = secret.iter().map(|b| format!("{b:02x}")).collect();
    for text in [
        report::controls_tsv(&r),
        report::functions_tsv(&r),
        report::anomalies_tsv(&r),
        report::json(&r),
    ] {
        assert!(!text.contains("BOARD-SERIAL"));
        assert!(!text.contains(&hex));
    }
}

#[test]
fn an_unmeasured_ogkm_tag_is_refused_not_mapped_to_a_neighbour() {
    let err = analyzer_for(
        DriverVersion {
            major: 580,
            minor: 88,
            patch: 0,
        },
        None,
    );
    assert!(
        err.is_err(),
        "580.88 is not a measured ogkm tag; the caller must choose one"
    );
}

#[test]
fn ogkm_status_distinguishes_defined_absent_and_unknown() {
    let tier1 = "id\tverdict\nctl-00730128\tNOT_IN_ANY_VERSION\nctl-0073011d\tPARTIAL\n";
    let mut an: Analyzer = analyzer_for(TAG, Some(tier1)).unwrap();
    let msgs = [
        ctl_req(0x0073_0128, &[0; 4]),
        ctl_req(0x0073_011d, &[0; 4]),
        ctl_req(0x2080_852a, &[0; 4]),
    ];
    let seqs = [0, 1, 2];
    let runs: Vec<(Dir, Vec<u8>)> = msgs
        .iter()
        .map(|m| m.dir)
        .zip(encode(&msgs, &seqs))
        .collect();
    an.begin_source("t", "test");
    for (i, (d, run)) in runs.iter().enumerate() {
        an.message(kf_census::analyze::In {
            a: 1,
            b: 0,
            dir: *d,
            ordinal: i as u64,
            ts: 0,
            bytes: run,
            element_size: ES as u32,
            exact: false,
            obs: None,
        });
    }
    let r = an.finish();
    let word = |cmd: u32| {
        r.controls
            .iter()
            .find(|c| c.cmd == cmd)
            .unwrap()
            .ogkm
            .word()
    };
    assert_eq!(word(0x0073_0128), "absent_all_tags");
    assert_eq!(word(0x0073_011d), "defined");
    assert_eq!(
        word(0x2080_852a),
        "unknown",
        "not in the matrix or the tier1 file: unknown, not absent"
    );
}

#[test]
fn an_overlong_or_garbage_observer_line_is_an_anomaly_not_a_crash() {
    let msgs = conversation();
    let runs = runs_of(&msgs, &natural_seqs(&msgs));
    let text = observer_jsonl(&runs, None, None);
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    lines.insert(3, "x".repeat(300_000));
    lines.insert(2, "{\"kind\":\"record\",".to_string());
    let r = run_observer(&(lines.join("\n") + "\n"));
    assert_eq!(count(&r, AnomalyKind::BadLine), 2, "{:?}", r.anomalies);
    assert_eq!(r.messages, [4, 5], "the good lines were still decoded");
}

#[test]
fn the_binary_sniffs_both_formats_and_its_exit_status_is_the_oracle() {
    let dir = std::env::temp_dir().join(format!("kf-census-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let msgs = conversation();
    let good = runs_of(&msgs, &natural_seqs(&msgs));
    let cap = dir.join("good.kfcap");
    let obs = dir.join("good.jsonl");
    std::fs::write(&cap, capture(&good, 4, true)).unwrap();
    std::fs::write(&obs, observer_jsonl(&good, None, None)).unwrap();
    let exe = env!("CARGO_BIN_EXE_kf-census");
    let out = dir.join("out");
    let st = std::process::Command::new(exe)
        .args([
            "--out",
            out.to_str().unwrap(),
            cap.to_str().unwrap(),
            obs.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(
        st.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&st.stdout)
    );
    let tsv = std::fs::read_to_string(dir.join("out.controls.tsv")).unwrap();
    assert!(
        tsv.contains("0x2080852a\t-\t4\t4\t4\t0\t0\t0\t0\t24:4\t1784:4"),
        "{tsv}"
    );
    assert!(
        std::fs::read_to_string(dir.join("out.json"))
            .unwrap()
            .contains("\"oracle_ok\":true")
    );

    let mut seqs = natural_seqs(&msgs);
    seqs[2] += 3;
    let bad = dir.join("bad.kfcap");
    std::fs::write(&bad, capture(&runs_of(&msgs, &seqs), 4, true)).unwrap();
    let st = std::process::Command::new(exe)
        .arg(bad.to_str().unwrap())
        .output()
        .unwrap();
    assert_eq!(st.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&st.stdout).contains("seq_gap"));
    let st = std::process::Command::new(exe)
        .arg("--ogkm-tag")
        .arg("580.88")
        .arg(bad.to_str().unwrap())
        .output()
        .unwrap();
    assert_eq!(
        st.status.code(),
        Some(2),
        "an unmeasured tag is a usage error"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
