// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! Sequence metadata, completeness and output-failure invariants.
mod common;

use common::*;
use kf_census::analyze::In;
use kf_census::capture::Dir;
use kf_census::{analyzer_for, observer, sequence::Sequence, stream};
use std::cell::RefCell;
use std::io::{self, Write};
use std::rc::Rc;

fn message<'a>(bytes: &'a [u8], dir: Dir) -> In<'a> {
    In {
        a: 0x1234_5678_9abc_def0,
        b: 0x9876_5432_1122_3344,
        dir,
        ordinal: 1,
        ts: 0,
        bytes,
        element_size: ES as u32,
        exact: false,
        obs: None,
    }
}

#[test]
fn native_and_observer_use_the_same_decode_and_keep_per_direction_order() {
    let msgs = conversation();
    let runs = runs_of(&msgs, &natural_seqs(&msgs));
    let mut exports = Vec::new();
    for is_observer in [false, true] {
        let mut an = analyzer_for(TAG, &[]).unwrap();
        let out = Rc::new(RefCell::new(Sequence::new(abi(), Vec::new(), 100).unwrap()));
        let tap = out.clone();
        an.set_record_tap(Box::new(move |source, m| {
            tap.borrow_mut().message(source, m).unwrap()
        }));
        if is_observer {
            observer::read(
                &mut an,
                "test",
                observer_jsonl(&runs, None, None).as_bytes(),
            )
            .unwrap();
        } else {
            stream::read(&mut an, "test", capture(&runs, 1, true).as_slice()).unwrap();
        }
        let report = an.finish();
        assert!(report.oracle_ok());
        assert_eq!(report.unbalanced_controls(), 0);
        let text =
            String::from_utf8(Rc::try_unwrap(out).ok().unwrap().into_inner().into_inner()).unwrap();
        // The producer polls queues separately; global observation order can
        // differ. Compare each direction, omitting source record ordinals.
        let normalize = |dir: &str| {
            text.lines()
                .skip(1)
                .filter_map(|l| {
                    let mut f: Vec<_> = l.split('\t').collect();
                    if f[3] != dir {
                        return None;
                    }
                    f.remove(2);
                    Some(f.join("\t"))
                })
                .collect::<Vec<_>>()
        };
        exports.push((normalize("cmd"), normalize("status")));
        assert!(!text.contains("c1d00001"), "client handle must never leave");
        assert!(text.contains("control\t0x2080a02f\t0x56"));
    }
    assert_eq!(exports[0], exports[1]);
}

#[test]
fn queue_instances_and_sources_stay_distinct_without_exporting_addresses() {
    let bytes = encode(&[ctl_req(0x2080_852a, &[0; 8])], &[0]).remove(0);
    let m = message(&bytes, Dir::Cmd);
    let mut out = Sequence::new(abi(), Vec::new(), 10).unwrap();
    out.message(0, &m).unwrap();
    out.message(
        0,
        &In {
            dir: Dir::Status,
            ..m
        },
    )
    .unwrap();
    out.message(0, &In { a: m.a + 1, ..m }).unwrap();
    out.message(1, &m).unwrap();
    let text = String::from_utf8(out.into_inner()).unwrap();
    let ids: Vec<_> = text
        .lines()
        .skip(1)
        .map(|l| l.split('\t').take(2).collect::<Vec<_>>())
        .collect();
    assert_eq!(
        ids,
        vec![
            vec!["1", "1"],
            vec!["1", "1"],
            vec!["1", "2"],
            vec!["2", "3"]
        ]
    );
    assert!(!text.contains("123456789abcdef0"));
    assert!(!text.contains("9876543211223344"));
}

#[test]
fn truncated_corrupt_and_excess_rows_are_not_silent_success() {
    let mut out = Sequence::new(abi(), Vec::new(), 1).unwrap();
    let m = message(&[0u8; 8], Dir::Cmd);
    out.message(0, &m).unwrap();
    assert!(
        out.message(0, &m)
            .unwrap_err()
            .to_string()
            .contains("limit")
    );
    assert!(
        String::from_utf8(out.into_inner())
            .unwrap()
            .contains("bad_frame")
    );
    assert!(Sequence::new(abi(), Vec::new(), 0).is_err());
    assert!(Sequence::new(abi(), Vec::new(), usize::MAX).is_err());
    let mut bad = encode(&[ctl_req(0x2080_852a, &[0; 8])], &[0]).remove(0);
    bad[50] ^= 1;
    let mut out = Sequence::new(abi(), Vec::new(), 10).unwrap();
    out.message(0, &message(&bad, Dir::Cmd)).unwrap();
    assert!(
        String::from_utf8(out.into_inner())
            .unwrap()
            .contains("bad_frame")
    );
}

#[test]
fn parses_both_transport_layouts_and_preserves_alloc_and_continuations() {
    for tag in [
        TAG,
        kf_abi::DriverVersion {
            major: 610,
            minor: 43,
            patch: 2,
        },
    ] {
        let a = kf_rm::abi::gsp_abi_for(tag).unwrap();
        let mut out = Sequence::new(a, Vec::new(), 10).unwrap();
        for (seq, f) in [a.rpc.codes.gsp_rm_alloc, a.rpc.codes.continuation_record]
            .into_iter()
            .enumerate()
        {
            let mut payload = vec![0; kf_abi::view::RpcAllocReq::HEADER];
            payload[12..16].copy_from_slice(&0xcbcbu32.to_le_bytes());
            let run = kf_gsp::encode_message(
                &a.element,
                a.rpc.header_version,
                ES as u32,
                a.element_size_max,
                seq as u32,
                &kf_gsp::OutgoingRpc {
                    function: f,
                    sequence: 0,
                    rpc_result: 0x56,
                    rpc_result_private: 0x1f,
                    payload,
                },
            )
            .unwrap();
            out.message(0, &message(&run, Dir::Status)).unwrap();
        }
        let text = String::from_utf8(out.into_inner()).unwrap();
        assert!(text.contains("alloc\t0xcbcb\t-\t0\tok"));
        assert!(text.contains("\t0x56\t0x1f\t"));
        assert!(text.contains("continuation\t-\t-\t-\tok"));
    }
}

#[test]
fn broken_output_is_an_export_failure() {
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("broken output"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::other("broken flush"))
        }
    }
    assert!(Sequence::new(abi(), Broken, 10).is_err());
    struct FlushFailure(Vec<u8>);
    impl Write for FlushFailure {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.write(b)
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::other("broken flush"))
        }
    }
    let mut out = Sequence::new(abi(), FlushFailure(Vec::new()), 10).unwrap();
    assert!(out.finish().is_err());
}
